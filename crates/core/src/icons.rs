//! Icon name resolution.
//!
//! Services and bookmarks name icons the way gethomepage does:
//!
//! | Value                      | Source                                   |
//! |----------------------------|------------------------------------------|
//! | `sonarr` / `sonarr.png`    | dashboard-icons (png, svg or webp)       |
//! | `sh-sonarr` / `sh-x.svg`   | selfh.st icons                           |
//! | `mdi-server`               | Material Design Icons                    |
//! | `si-github`                | Simple Icons                             |
//! | `custom/my.png`            | an upload served from the data folder    |
//! | `https://...` or `/...`    | used as is                               |

use serde::Serialize;

/// Base URL for the dashboard-icons set.
pub const DASHBOARD_ICONS_CDN: &str = "https://cdn.jsdelivr.net/gh/homarr-labs/dashboard-icons";
/// Base URL for the selfh.st icon set.
pub const SELFHST_ICONS_CDN: &str = "https://cdn.jsdelivr.net/gh/selfhst/icons";
/// Base URL for Material Design Icons SVGs.
pub const MDI_CDN: &str = "https://cdn.jsdelivr.net/npm/@mdi/svg@7.4.47/svg";
/// Base URL for Simple Icons SVGs.
pub const SIMPLE_ICONS_CDN: &str = "https://cdn.jsdelivr.net/npm/simple-icons@13.21.0/icons";
/// URL prefix under which custom uploads are served.
pub const CUSTOM_ICON_PREFIX: &str = "/icons/custom/";

/// What kind of icon a name resolved to. Monochrome icons (MDI, Simple
/// Icons) are drawn with a CSS mask so they follow the theme colour.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum IconKind {
    /// A full-colour image.
    Image,
    /// A single-colour glyph to be tinted.
    Mask,
}

/// A resolved icon.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ResolvedIcon {
    /// URL the browser loads.
    pub url: String,
    /// How to draw it.
    pub kind: IconKind,
}

/// Resolves an icon name to a URL. Returns `None` for an empty name or a
/// name with characters that do not belong in an icon name.
pub fn resolve(name: &str) -> Option<ResolvedIcon> {
    let name = name.trim();
    if name.is_empty() {
        return None;
    }
    let image = |url: String| {
        Some(ResolvedIcon {
            url,
            kind: IconKind::Image,
        })
    };
    let mask = |url: String| {
        Some(ResolvedIcon {
            url,
            kind: IconKind::Mask,
        })
    };

    if name.starts_with("http://") || name.starts_with("https://") {
        return image(name.to_owned());
    }
    if name.starts_with('/') && !name.starts_with("//") {
        return image(name.to_owned());
    }
    if let Some(file) = name.strip_prefix("custom/") {
        return valid_slug(file).then(|| ResolvedIcon {
            url: format!("{CUSTOM_ICON_PREFIX}{file}"),
            kind: IconKind::Image,
        });
    }
    if !valid_slug(name) {
        return None;
    }
    if let Some(rest) = name.strip_prefix("mdi-") {
        return mask(format!("{MDI_CDN}/{}.svg", strip_ext(rest)));
    }
    if let Some(rest) = name.strip_prefix("si-") {
        return mask(format!("{SIMPLE_ICONS_CDN}/{}.svg", strip_ext(rest)));
    }
    if let Some(rest) = name.strip_prefix("sh-") {
        let (stem, ext) = split_ext(rest);
        return image(format!("{SELFHST_ICONS_CDN}/{ext}/{stem}.{ext}"));
    }
    let (stem, ext) = split_ext(name);
    image(format!("{DASHBOARD_ICONS_CDN}/{ext}/{stem}.{ext}"))
}

/// Icon names may only hold ASCII letters, digits, `-`, `_` and `.`.
fn valid_slug(s: &str) -> bool {
    !s.is_empty()
        && !s.starts_with('.')
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
}

/// Splits `name.ext` into stem and a supported extension (png by default).
fn split_ext(name: &str) -> (&str, &str) {
    match name.rsplit_once('.') {
        Some((stem, ext @ ("png" | "svg" | "webp"))) => (stem, ext),
        _ => (name, "png"),
    }
}

fn strip_ext(name: &str) -> &str {
    name.strip_suffix(".svg").unwrap_or(name)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn url(name: &str) -> String {
        resolve(name).unwrap().url
    }

    #[test]
    fn dashboard_icons() {
        assert_eq!(
            url("sonarr"),
            format!("{DASHBOARD_ICONS_CDN}/png/sonarr.png")
        );
        assert_eq!(
            url("sonarr.svg"),
            format!("{DASHBOARD_ICONS_CDN}/svg/sonarr.svg")
        );
        assert_eq!(
            url("sonarr.webp"),
            format!("{DASHBOARD_ICONS_CDN}/webp/sonarr.webp")
        );
    }

    #[test]
    fn prefixed_sets() {
        assert_eq!(
            url("sh-immich"),
            format!("{SELFHST_ICONS_CDN}/png/immich.png")
        );
        assert_eq!(
            url("sh-immich.svg"),
            format!("{SELFHST_ICONS_CDN}/svg/immich.svg")
        );
        let m = resolve("mdi-server").unwrap();
        assert_eq!(m.url, format!("{MDI_CDN}/server.svg"));
        assert_eq!(m.kind, IconKind::Mask);
        assert_eq!(url("si-github"), format!("{SIMPLE_ICONS_CDN}/github.svg"));
    }

    #[test]
    fn urls_and_custom() {
        assert_eq!(url("https://x.lan/i.png"), "https://x.lan/i.png");
        assert_eq!(url("/images/a.png"), "/images/a.png");
        assert_eq!(url("custom/nas.png"), "/icons/custom/nas.png");
    }

    #[test]
    fn rejects_junk() {
        assert!(resolve("").is_none());
        assert!(resolve("custom/../etc/passwd").is_none());
        assert!(resolve("a b").is_none());
        assert!(resolve("//evil.example/x.png").is_none());
        assert!(resolve("javascript:alert(1)").is_none());
    }
}
