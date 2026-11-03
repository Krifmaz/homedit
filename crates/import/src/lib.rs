//! Import a [gethomepage](https://gethomepage.dev) configuration into homedit.
//!
//! gethomepage keeps its configuration in a folder of YAML files:
//! `services.yaml`, `bookmarks.yaml`, `widgets.yaml`, `settings.yaml` and
//! `docker.yaml`. This crate reads those files (from disk with
//! [`import_dir`], or from strings with [`import_sources`]) into the plain
//! types in [`model`], which the server then stores in its database.
//!
//! The importer is lenient: an entry with an unexpected shape is skipped and
//! reported in [`ImportedConfig::warnings`], so one typo does not block an
//! import. Only a file that is not valid YAML, or whose top level has the
//! wrong shape, fails the import.
//!
//! `{{HOMEPAGE_VAR_*}}` and `{{HOMEPAGE_FILE_*}}` placeholders are resolved
//! through a [`Resolver`]; unresolved placeholders are kept as written and
//! reported as warnings.
//!
//! ```
//! use homedit_import::{import_sources, ImportSources, NoResolver};
//!
//! let sources = ImportSources {
//!     services: Some("- Media:\n    - Plex:\n        href: http://plex:32400\n".into()),
//!     ..ImportSources::default()
//! };
//! let config = import_sources(&sources, &NoResolver).unwrap();
//! assert_eq!(config.services[0].services[0].name, "Plex");
//! ```

pub mod convert;
pub mod model;
mod parse;
pub mod subst;

use std::path::{Path, PathBuf};

pub use convert::{ImportReport, to_core_service};
pub use model::*;
pub use subst::{EnvResolver, MapResolver, NoResolver, Resolver};

/// Errors that stop an import.
#[derive(Debug, thiserror::Error)]
pub enum ImportError {
    /// A file could not be read.
    #[error("cannot read {path}: {source}")]
    Io {
        /// The file that failed.
        path: PathBuf,
        /// The underlying error.
        #[source]
        source: std::io::Error,
    },
    /// A file is not valid YAML.
    #[error("{file} is not valid YAML: {source}")]
    Yaml {
        /// The file name, such as `services.yaml`.
        file: String,
        /// The parser error, with line and column.
        #[source]
        source: serde_yaml_ng::Error,
    },
    /// A file is valid YAML but its top level has the wrong shape.
    #[error("{file}: {message}")]
    Shape {
        /// The file name.
        file: String,
        /// What was expected.
        message: String,
    },
    /// The folder has none of the known files.
    #[error("no gethomepage config files found in {0}")]
    NothingFound(PathBuf),
}

/// The text of each gethomepage file. Missing files are `None`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ImportSources {
    /// Contents of `services.yaml`.
    pub services: Option<String>,
    /// Contents of `bookmarks.yaml`.
    pub bookmarks: Option<String>,
    /// Contents of `widgets.yaml`.
    pub widgets: Option<String>,
    /// Contents of `settings.yaml`.
    pub settings: Option<String>,
    /// Contents of `docker.yaml`.
    pub docker: Option<String>,
    /// Contents of `custom.css`.
    pub custom_css: Option<String>,
}

impl ImportSources {
    /// Reads the known files from `dir`. Both `.yaml` and `.yml` extensions
    /// are accepted (`.yaml` wins if both exist). Other files are ignored.
    ///
    /// Only regular files directly inside `dir` are read; nothing is
    /// executed.
    pub fn from_dir(dir: &Path) -> Result<Self, ImportError> {
        let read_plain = |path: &Path| -> Result<Option<String>, ImportError> {
            match std::fs::read_to_string(path) {
                Ok(text) => Ok(Some(text)),
                Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(None),
                Err(source) => Err(ImportError::Io {
                    path: path.to_path_buf(),
                    source,
                }),
            }
        };
        let read = |stem: &str| -> Result<Option<String>, ImportError> {
            for ext in ["yaml", "yml"] {
                let path = dir.join(format!("{stem}.{ext}"));
                match std::fs::read_to_string(&path) {
                    Ok(text) => return Ok(Some(text)),
                    Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
                    Err(source) => return Err(ImportError::Io { path, source }),
                }
            }
            Ok(None)
        };
        let sources = Self {
            services: read("services")?,
            bookmarks: read("bookmarks")?,
            widgets: read("widgets")?,
            settings: read("settings")?,
            docker: read("docker")?,
            custom_css: read_plain(&dir.join("custom.css"))?,
        };
        if sources == Self::default() {
            return Err(ImportError::NothingFound(dir.to_path_buf()));
        }
        Ok(sources)
    }

    /// Sets the contents for a file by its name (`services.yaml`,
    /// `bookmarks.yml`, ...). Returns false if the name is not one of the
    /// known files. Useful when files arrive as uploads.
    pub fn set_by_name(&mut self, file_name: &str, contents: String) -> bool {
        if file_name == "custom.css" {
            self.custom_css = Some(contents);
            return true;
        }
        let stem = file_name
            .strip_suffix(".yaml")
            .or_else(|| file_name.strip_suffix(".yml"))
            .unwrap_or(file_name);
        let slot = match stem {
            "services" => &mut self.services,
            "bookmarks" => &mut self.bookmarks,
            "widgets" => &mut self.widgets,
            "settings" => &mut self.settings,
            "docker" => &mut self.docker,
            _ => return false,
        };
        *slot = Some(contents);
        true
    }
}

/// Reads and imports a gethomepage config folder, resolving placeholders
/// from the process environment.
pub fn import_dir(dir: &Path) -> Result<ImportedConfig, ImportError> {
    import_sources(&ImportSources::from_dir(dir)?, &EnvResolver)
}

/// Reads a gethomepage config folder and converts it into a
/// [`homedit_core::Dashboard`] ready for `Db::save_dashboard`.
pub fn import_dashboard_dir(dir: &Path) -> Result<ImportReport, ImportError> {
    import_dashboard(&ImportSources::from_dir(dir)?, &EnvResolver)
}

/// Imports gethomepage config from in-memory file contents and converts it
/// into a [`homedit_core::Dashboard`]. `custom.css`, if present, becomes the
/// dashboard's custom CSS.
pub fn import_dashboard(
    sources: &ImportSources,
    resolver: &dyn Resolver,
) -> Result<ImportReport, ImportError> {
    let mut report = import_sources(sources, resolver)?.into_report();
    if let Some(css) = &sources.custom_css {
        if !css.trim().is_empty() {
            report.dashboard.settings.custom_css = Some(css.clone());
        }
    }
    Ok(report)
}

/// Imports gethomepage config from in-memory file contents.
pub fn import_sources(
    sources: &ImportSources,
    resolver: &dyn Resolver,
) -> Result<ImportedConfig, ImportError> {
    let mut config = ImportedConfig::default();

    macro_rules! run {
        ($field:ident, $file:literal, $parser:path) => {
            if let Some(text) = &sources.$field {
                let mut ctx = parse::Ctx::new($file, resolver);
                config.$field = $parser(&mut ctx, text)?;
                config.warnings.append(&mut ctx.warnings);
            }
        };
    }
    run!(settings, "settings.yaml", parse::settings);
    run!(services, "services.yaml", parse::services);
    run!(bookmarks, "bookmarks.yaml", parse::bookmarks);
    run!(widgets, "widgets.yaml", parse::widgets);
    run!(docker, "docker.yaml", parse::docker);
    Ok(config)
}

/// Builds a [`Service`] from a gethomepage-style field map (`href`, `icon`,
/// `widget`, ...). Docker discovery uses this to turn container labels into
/// the same type the YAML importer produces.
///
/// Returns the service and any warnings about fields that were ignored.
pub fn service_from_fields(name: String, fields: Object) -> (Service, Vec<ImportWarning>) {
    let mut ctx = parse::Ctx::new("docker labels", &NoResolver);
    let path = name.clone();
    let service = parse::service(&mut ctx, &path, name, fields);
    (service, ctx.warnings)
}
