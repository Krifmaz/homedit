//! Card names from page titles.
//!
//! Login and start pages put boilerplate in their `<title>`: "Login |
//! OPNsense", "Sign in · Gitea", "Home - Grafana", "Welcome to nginx!".
//! [`clean_title`] keeps the part that names the app.

use crate::catalogue::Catalogue;

/// Separators between the parts of a title. Plain hyphens and slashes only
/// count with spaces around them, so "Pi-hole" and "AC/DC" stay whole.
const SEPARATORS: &[&str] = &[
    " | ", "|", " - ", " – ", " — ", "–", "—", " · ", "·", " • ", "•", " :: ", " » ", "»", " « ",
    "«", " / ", ": ",
];

/// Phrases that are boilerplate at the start of a part ("Login to Gitea").
const LEADING: &[&[&str]] = &[
    &["welcome", "to"],
    &["log", "in", "to"],
    &["login", "to"],
    &["sign", "in", "to"],
    &["signin", "to"],
    &["sign-in", "to"],
    &["log", "in"],
    &["login"],
    &["log-in"],
    &["logon"],
    &["sign", "in"],
    &["sign-in"],
    &["signin"],
    &["welcome"],
];

/// Phrases that are boilerplate at the end of a part ("Traefik Dashboard").
const TRAILING: &[&[&str]] = &[
    &["log", "in"],
    &["login"],
    &["log-in"],
    &["logon"],
    &["sign", "in"],
    &["sign-in"],
    &["signin"],
    &["dashboard"],
    &["page"],
];

/// Words that mean nothing when a part is made only of them ("Home",
/// "Dashboard", "Login Page"). "Home" is only dropped on its own, so "Home
/// Assistant" stays.
const FILLER: &[&str] = &[
    "login",
    "log",
    "in",
    "log-in",
    "logon",
    "sign",
    "sign-in",
    "signin",
    "dashboard",
    "home",
    "homepage",
    "welcome",
    "index",
    "main",
    "start",
    "page",
    "portal",
    "web",
    "ui",
    "webui",
    "interface",
    "overview",
    "untitled",
    "loading",
    "loading...",
    "please",
    "wait",
];

/// A card name from a page title, or `None` when the title is only
/// boilerplate ("Login", "Dashboard").
///
/// Splits the title on separators (`|`, ` - `, `·`, ...), drops "Login",
/// "Sign in", "Welcome to", "Dashboard", "Home" and the like, and keeps:
/// a part that is a known app's name (from the built-in catalogue, in its
/// usual spelling), else the remaining parts joined with " - ".
///
/// ```
/// use scanner::naming::clean_title;
/// assert_eq!(clean_title("Login | OPNsense").as_deref(), Some("OPNsense"));
/// assert_eq!(clean_title("Home - grafana").as_deref(), Some("Grafana"));
/// assert_eq!(clean_title("Welcome to My NAS!").as_deref(), Some("My NAS"));
/// assert_eq!(clean_title("Dashboard"), None);
/// ```
pub fn clean_title(raw: &str) -> Option<String> {
    clean_title_with(raw, Catalogue::builtin())
}

/// [`clean_title`] against a given catalogue.
pub fn clean_title_with(raw: &str, catalogue: &Catalogue) -> Option<String> {
    let parts: Vec<String> = split(raw)
        .into_iter()
        .filter_map(|p| clean_part(&p))
        .collect();
    for part in &parts {
        if let Some(app) = catalogue
            .apps()
            .iter()
            .find(|a| a.name.eq_ignore_ascii_case(part))
        {
            return Some(app.name.to_string());
        }
    }
    let mut kept: Vec<String> = Vec::new();
    for p in parts {
        if !kept.iter().any(|k| k.eq_ignore_ascii_case(&p)) {
            kept.push(p);
        }
    }
    (!kept.is_empty()).then(|| kept.join(" - "))
}

fn split(raw: &str) -> Vec<String> {
    let mut parts = vec![raw.to_string()];
    for sep in SEPARATORS {
        parts = parts
            .iter()
            .flat_map(|p| p.split(sep).map(str::to_string).collect::<Vec<_>>())
            .collect();
    }
    parts
}

/// Characters trimmed from the ends of a part.
fn is_edge(c: char) -> bool {
    c.is_whitespace()
        || matches!(
            c,
            '-' | '|' | ':' | ',' | '.' | '!' | '·' | '•' | '»' | '«' | '/' | '–' | '—'
        )
}

fn norm(word: &str) -> String {
    word.trim_matches(|c: char| !c.is_alphanumeric() && c != '-')
        .to_lowercase()
}

fn starts_with(words: &[&str], phrase: &[&str]) -> bool {
    words.len() >= phrase.len() && words.iter().zip(phrase).all(|(w, p)| norm(w) == *p)
}

fn ends_with(words: &[&str], phrase: &[&str]) -> bool {
    words.len() >= phrase.len()
        && words[words.len() - phrase.len()..]
            .iter()
            .zip(phrase)
            .all(|(w, p)| norm(w) == *p)
}

fn clean_part(part: &str) -> Option<String> {
    let mut words: Vec<&str> = part.split_whitespace().collect();
    loop {
        if words.iter().all(|w| {
            let n = norm(w);
            n.is_empty() || FILLER.contains(&n.as_str())
        }) {
            return None;
        }
        if let Some(p) = LEADING.iter().find(|p| starts_with(&words, p)) {
            words.drain(..p.len());
            continue;
        }
        if let Some(p) = TRAILING.iter().find(|p| ends_with(&words, p)) {
            words.truncate(words.len() - p.len());
            continue;
        }
        break;
    }
    let text = words.join(" ");
    let text = text.trim_matches(is_edge);
    (!text.is_empty()).then(|| text.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn c(s: &str) -> Option<String> {
        clean_title(s)
    }

    #[test]
    fn known_apps_win() {
        assert_eq!(c("Login | OPNsense").as_deref(), Some("OPNsense"));
        assert_eq!(c("pfSense - Login").as_deref(), Some("pfSense"));
        assert_eq!(
            c("router.lan - pfSense - Login").as_deref(),
            Some("pfSense")
        );
        assert_eq!(c("Sign in · Gitea").as_deref(), Some("Gitea"));
        assert_eq!(c("Home - grafana").as_deref(), Some("Grafana"));
        assert_eq!(c("Grafana").as_deref(), Some("Grafana"));
        assert_eq!(c("Home Assistant").as_deref(), Some("Home Assistant"));
        assert_eq!(c("Pi-hole - raspberrypi").as_deref(), Some("Pi-hole"));
    }

    #[test]
    fn strips_boilerplate() {
        assert_eq!(c("Welcome to My NAS!").as_deref(), Some("My NAS"));
        assert_eq!(c("Login to Foo").as_deref(), Some("Foo"));
        assert_eq!(c("Sign in to Acme Wiki").as_deref(), Some("Acme Wiki"));
        assert_eq!(c("Foo Dashboard").as_deref(), Some("Foo"));
        assert_eq!(c("Foo :: Login Page").as_deref(), Some("Foo"));
        assert_eq!(c("Log in | Foo | Foo").as_deref(), Some("Foo"));
        assert_eq!(c("Foo - Bar").as_deref(), Some("Foo - Bar"));
        assert_eq!(c("Homebox").as_deref(), Some("Homebox"));
        assert_eq!(c("Home Lab").as_deref(), Some("Home Lab"));
    }

    #[test]
    fn only_boilerplate_is_none() {
        for t in [
            "Login",
            "Sign In",
            "Dashboard",
            "Home",
            "Home | Dashboard",
            " - ",
            "Login Page",
            "Loading...",
        ] {
            assert_eq!(c(t), None, "{t:?}");
        }
    }
}
