//! Substitution of gethomepage `{{HOMEPAGE_VAR_*}}` and `{{HOMEPAGE_FILE_*}}`
//! placeholders.
//!
//! gethomepage lets users keep secrets out of their YAML by writing
//! `{{HOMEPAGE_VAR_SONARR_KEY}}` (replaced with that environment variable) or
//! `{{HOMEPAGE_FILE_SONARR_KEY}}` (replaced with the contents of the file
//! whose path is in that environment variable). The importer resolves these
//! through the [`Resolver`] trait so tests and callers can control where
//! values come from.

use std::collections::HashMap;

/// Looks up values for placeholders.
pub trait Resolver {
    /// Returns the value of an environment variable, if set.
    fn var(&self, name: &str) -> Option<String>;

    /// Returns the contents of a file, if it can be read.
    fn read_file(&self, path: &str) -> Option<String>;
}

/// Resolves placeholders from the process environment and the filesystem.
#[derive(Debug, Clone, Copy, Default)]
pub struct EnvResolver;

impl Resolver for EnvResolver {
    fn var(&self, name: &str) -> Option<String> {
        std::env::var(name).ok()
    }

    fn read_file(&self, path: &str) -> Option<String> {
        std::fs::read_to_string(path).ok()
    }
}

/// Resolves nothing: every placeholder is left as written.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoResolver;

impl Resolver for NoResolver {
    fn var(&self, _name: &str) -> Option<String> {
        None
    }

    fn read_file(&self, _path: &str) -> Option<String> {
        None
    }
}

/// Resolves placeholders from an in-memory map. Files are looked up in the
/// same map by path. Useful for tests and for uploads from the browser.
#[derive(Debug, Clone, Default)]
pub struct MapResolver(pub HashMap<String, String>);

impl Resolver for MapResolver {
    fn var(&self, name: &str) -> Option<String> {
        self.0.get(name).cloned()
    }

    fn read_file(&self, path: &str) -> Option<String> {
        self.0.get(path).cloned()
    }
}

/// Replaces every `{{HOMEPAGE_VAR_X}}` and `{{HOMEPAGE_FILE_X}}` in `input`.
///
/// Returns the new text and the names of placeholders that could not be
/// resolved (those are left in the text unchanged). File contents are
/// trimmed of trailing newlines, as most secret files end with one.
pub fn substitute(input: &str, resolver: &dyn Resolver) -> (String, Vec<String>) {
    let mut out = String::with_capacity(input.len());
    let mut unresolved = Vec::new();
    let mut rest = input;
    while let Some(start) = rest.find("{{") {
        out.push_str(&rest[..start]);
        let after = &rest[start + 2..];
        let Some(end) = after.find("}}") else {
            out.push_str(&rest[start..]);
            return (out, unresolved);
        };
        let name = after[..end].trim();
        let value = if name.starts_with("HOMEPAGE_VAR_") {
            resolver.var(name)
        } else if name.starts_with("HOMEPAGE_FILE_") {
            resolver
                .var(name)
                .and_then(|path| resolver.read_file(&path))
                .map(|text| text.trim_end_matches(['\r', '\n']).to_string())
        } else {
            // Not ours: keep it verbatim (it may be a template for a widget).
            out.push_str(&rest[start..start + 2 + end + 2]);
            rest = &after[end + 2..];
            continue;
        };
        match value {
            Some(value) => out.push_str(&value),
            None => {
                unresolved.push(name.to_string());
                out.push_str(&rest[start..start + 2 + end + 2]);
            }
        }
        rest = &after[end + 2..];
    }
    out.push_str(rest);
    (out, unresolved)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn resolver() -> MapResolver {
        MapResolver(HashMap::from([
            ("HOMEPAGE_VAR_KEY".to_string(), "abc123".to_string()),
            (
                "HOMEPAGE_FILE_PW".to_string(),
                "/run/secrets/pw".to_string(),
            ),
            ("/run/secrets/pw".to_string(), "hunter2\n".to_string()),
        ]))
    }

    #[test]
    fn replaces_vars_and_files() {
        let (out, missing) = substitute(
            "key: {{HOMEPAGE_VAR_KEY}}\npw: {{ HOMEPAGE_FILE_PW }}\n",
            &resolver(),
        );
        assert_eq!(out, "key: abc123\npw: hunter2\n");
        assert!(missing.is_empty());
    }

    #[test]
    fn keeps_unknown_and_unresolved() {
        let (out, missing) = substitute("{{HOMEPAGE_VAR_NOPE}} {{other}} {{open", &resolver());
        assert_eq!(out, "{{HOMEPAGE_VAR_NOPE}} {{other}} {{open");
        assert_eq!(missing, vec!["HOMEPAGE_VAR_NOPE".to_string()]);
    }
}
