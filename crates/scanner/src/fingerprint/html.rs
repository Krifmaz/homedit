//! Just enough HTML reading to get a page title and its favicon link.
//!
//! This is deliberately not a full HTML parser: it scans for `<title>` and
//! `<link>` tags case-insensitively and reads their attributes. That is all
//! fingerprinting needs, and it copes with the broken markup some apps serve.

/// Longest title kept, in characters.
const MAX_TITLE: usize = 200;

/// Returns the text of the first `<title>` element, with entities decoded and
/// whitespace collapsed. `None` when there is no title or it is empty.
pub fn title(html: &str) -> Option<String> {
    let lower = html.to_ascii_lowercase();
    let open = lower.find("<title")?;
    let start = open + lower[open..].find('>')? + 1;
    let end = start + lower[start..].find("</title")?;
    let text = collapse_whitespace(&decode_entities(&html[start..end]));
    if text.is_empty() {
        None
    } else {
        Some(text.chars().take(MAX_TITLE).collect())
    }
}

/// Returns the `href` of the best `<link rel="icon">`-style tag.
///
/// Prefers `rel="icon"` or `rel="shortcut icon"` over `apple-touch-icon`,
/// because the small icon is the one hashed by the usual favicon databases.
pub fn favicon_href(html: &str) -> Option<String> {
    let lower = html.to_ascii_lowercase();
    let mut best: Option<(u8, String)> = None;
    let mut pos = 0;
    while let Some(found) = lower[pos..].find("<link") {
        let start = pos + found + "<link".len();
        let Some(len) = lower[start..].find('>') else {
            break;
        };
        let attrs = parse_attributes(&html[start..start + len]);
        pos = start + len;

        let rel = attr(&attrs, "rel").unwrap_or_default().to_ascii_lowercase();
        let Some(href) = attr(&attrs, "href") else {
            continue;
        };
        let tokens: Vec<&str> = rel.split_ascii_whitespace().collect();
        let rank = if tokens.contains(&"icon") {
            2
        } else if tokens.iter().any(|t| t.ends_with("icon")) {
            1
        } else {
            continue;
        };
        if best.as_ref().is_none_or(|(r, _)| rank > *r) {
            best = Some((rank, decode_entities(href.trim())));
        }
    }
    best.map(|(_, href)| href)
}

fn attr<'a>(attrs: &'a [(String, String)], name: &str) -> Option<&'a str> {
    attrs
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case(name))
        .map(|(_, v)| v.as_str())
}

/// Splits the inside of a tag into `(name, value)` pairs. Handles double,
/// single and unquoted values and bare attributes.
fn parse_attributes(s: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut chars = s.chars().peekable();
    loop {
        while chars.next_if(|c| c.is_whitespace() || *c == '/').is_some() {}
        let mut name = String::new();
        while let Some(c) = chars.next_if(|c| !c.is_whitespace() && *c != '=' && *c != '/') {
            name.push(c);
        }
        if name.is_empty() {
            break;
        }
        while chars.next_if(|c| c.is_whitespace()).is_some() {}
        let mut value = String::new();
        if chars.next_if_eq(&'=').is_some() {
            while chars.next_if(|c| c.is_whitespace()).is_some() {}
            match chars.peek().copied() {
                Some(q @ ('"' | '\'')) => {
                    chars.next();
                    for c in chars.by_ref() {
                        if c == q {
                            break;
                        }
                        value.push(c);
                    }
                }
                _ => {
                    while let Some(c) = chars.next_if(|c| !c.is_whitespace()) {
                        value.push(c);
                    }
                }
            }
        }
        out.push((name, value));
    }
    out
}

/// Decodes the handful of entities that show up in titles and URLs.
fn decode_entities(s: &str) -> String {
    if !s.contains('&') {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(amp) = rest.find('&') {
        out.push_str(&rest[..amp]);
        rest = &rest[amp..];
        let Some(semi) = rest[..rest.len().min(10)].find(';') else {
            out.push('&');
            rest = &rest[1..];
            continue;
        };
        let entity = &rest[1..semi];
        let decoded = match entity {
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            "nbsp" => Some(' '),
            _ => entity
                .strip_prefix("#x")
                .or_else(|| entity.strip_prefix("#X"))
                .and_then(|hex| u32::from_str_radix(hex, 16).ok())
                .or_else(|| entity.strip_prefix('#').and_then(|d| d.parse().ok()))
                .and_then(char::from_u32),
        };
        match decoded {
            Some(c) => {
                out.push(c);
                rest = &rest[semi + 1..];
            }
            None => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

fn collapse_whitespace(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_titles() {
        assert_eq!(
            title("<html><head><TITLE>Sonarr</TITLE>").as_deref(),
            Some("Sonarr")
        );
        assert_eq!(
            title("<title data-x=1>\n  Tom &amp; Jerry&#39;s   NAS </title>").as_deref(),
            Some("Tom & Jerry's NAS")
        );
        assert_eq!(title("<title></title>"), None);
        assert_eq!(title("<p>no title</p>"), None);
        assert_eq!(title("<title>unterminated"), None);
        assert_eq!(
            title(&format!("<title>{}</title>", "x".repeat(500)))
                .unwrap()
                .len(),
            MAX_TITLE
        );
    }

    #[test]
    fn finds_favicons() {
        let html = r#"<link rel="apple-touch-icon" href="/apple.png">
            <LINK href='/static/fav.ico?v=2&amp;x=1' REL="shortcut icon" />
            <link rel=stylesheet href=/app.css>"#;
        assert_eq!(
            favicon_href(html).as_deref(),
            Some("/static/fav.ico?v=2&x=1")
        );
        assert_eq!(
            favicon_href(r#"<link rel="apple-touch-icon" href="/apple.png">"#).as_deref(),
            Some("/apple.png")
        );
        assert_eq!(favicon_href("<link rel=stylesheet href=/a.css>"), None);
    }

    #[test]
    fn entities() {
        assert_eq!(
            decode_entities("a &lt;b&gt; &#x41; &unknown; & c"),
            "a <b> A &unknown; & c"
        );
    }
}
