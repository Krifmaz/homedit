//! Icon names as people write them (gethomepage style) and the URLs they load from, matching the server.

/// The dashboard-icons CDN.
const DASHBOARD_ICONS: &str = "https://cdn.jsdelivr.net/gh/homarr-labs/dashboard-icons";
/// The selfh.st icons CDN.
const SELFHST: &str = "https://cdn.jsdelivr.net/gh/selfhst/icons";
/// Material Design Icons.
const MDI: &str = "https://cdn.jsdelivr.net/npm/@mdi/svg@7.4.47/svg";
/// Simple Icons.
const SIMPLE: &str = "https://cdn.jsdelivr.net/npm/simple-icons@13.21.0/icons";

fn split_ext(name: &str) -> (&str, &str) {
    for ext in ["png", "svg", "webp"] {
        if let Some(stem) = name.strip_suffix(&format!(".{ext}")[..])
            && !stem.is_empty()
        {
            return (stem, ext);
        }
    }
    (name, "png")
}

fn valid(n: &str) -> bool {
    let mut b = n.bytes();
    matches!(b.next(), Some(c) if c.is_ascii_alphanumeric() || c == b'_' || c == b'-')
        && b.all(|c| c.is_ascii_alphanumeric() || matches!(c, b'_' | b'.' | b'-'))
}

/// The URL of an icon name, and whether it is a monochrome mask tinted with the text colour.
pub fn resolve_name(name: &str) -> (String, bool) {
    let n = name.trim();
    if let Some(rest) = n.strip_prefix("custom/") {
        return (format!("/icons/custom/{rest}"), false);
    }
    if !valid(n) {
        return (String::new(), false);
    }
    if let Some(rest) = n.strip_prefix("mdi-") {
        return (format!("{MDI}/{}.svg", rest.trim_end_matches(".svg")), true);
    }
    if let Some(rest) = n.strip_prefix("si-") {
        return (
            format!("{SIMPLE}/{}.svg", rest.trim_end_matches(".svg")),
            true,
        );
    }
    if let Some(rest) = n.strip_prefix("sh-") {
        let (stem, ext) = split_ext(rest);
        return (format!("{SELFHST}/{ext}/{stem}.{ext}"), false);
    }
    let (stem, ext) = split_ext(n);
    (format!("{DASHBOARD_ICONS}/{ext}/{stem}.{ext}"), false)
}

/// Whether `r` is already a URL (absolute, from the root, `data:` or `blob:`).
pub fn is_url(r: &str) -> bool {
    r.starts_with("http://")
        || r.starts_with("https://")
        || r.starts_with("//")
        || r.starts_with('/')
        || r.starts_with("data:")
        || r.starts_with("blob:")
}

/// The URL to load for an icon reference (a URL as is, a name resolved).
pub fn url(r: &str) -> Option<String> {
    if r.is_empty() {
        None
    } else if is_url(r) {
        Some(r.to_string())
    } else {
        Some(resolve_name(r).0).filter(|u| !u.is_empty())
    }
}

/// Whether the reference names a mask icon (`mdi-…`, `si-…`).
pub fn is_mask(r: &str) -> bool {
    !is_url(r) && resolve_name(r).1
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_like_the_server() {
        assert_eq!(
            resolve_name("sonarr").0,
            "https://cdn.jsdelivr.net/gh/homarr-labs/dashboard-icons/png/sonarr.png"
        );
        assert!(resolve_name("sonarr.svg").0.ends_with("/svg/sonarr.svg"));
        assert_eq!(
            resolve_name("sh-immich.webp").0,
            "https://cdn.jsdelivr.net/gh/selfhst/icons/webp/immich.webp"
        );
        assert_eq!(
            resolve_name("mdi-server"),
            (
                "https://cdn.jsdelivr.net/npm/@mdi/svg@7.4.47/svg/server.svg".into(),
                true
            )
        );
        assert_eq!(resolve_name("custom/me.png").0, "/icons/custom/me.png");
        assert_eq!(resolve_name("../etc").0, "");
        assert_eq!(url("/x.png").as_deref(), Some("/x.png"));
        assert_eq!(url(""), None);
        assert!(is_mask("si-github"));
    }
}
