//! Parsing of gethomepage-compatible container labels.
//!
//! gethomepage reads labels such as:
//!
//! ```text
//! homepage.group=Media
//! homepage.name=Sonarr
//! homepage.icon=sonarr.png
//! homepage.href=http://sonarr.lan:8989
//! homepage.widget.type=sonarr
//! homepage.widget.url=http://sonarr:8989
//! homepage.widget.key=abc123
//! homepage.widget.fields=["queued","wanted"]
//! homepage.widgets[1].type=customapi
//! homepage.instance.internal.href=http://sonarr:8989
//! ```
//!
//! Each label after the prefix is a dotted path; `[N]` selects a list
//! element. Values that look like JSON arrays or objects are parsed as JSON.
//! `homepage.instance.<name>.<path>` labels override the plain ones when
//! discovery runs with that instance name.

use serde_json::{Map, Value};

/// The label prefix gethomepage uses.
pub const PREFIX: &str = "homepage.";

/// The highest list index accepted in a label path. Prevents a label such
/// as `homepage.widgets[999999999].type` from allocating a huge list.
pub const MAX_INDEX: usize = 32;

/// One step in a label path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Step {
    /// A map key.
    Key(String),
    /// A list index.
    Index(usize),
}

/// Splits `widget.mappings[0].field` into steps. Returns an error message
/// for malformed paths.
pub(crate) fn parse_path(path: &str) -> Result<Vec<Step>, String> {
    let mut steps = Vec::new();
    for segment in path.split('.') {
        let (key, mut rest) = match segment.find('[') {
            Some(at) => (&segment[..at], &segment[at..]),
            None => (segment, ""),
        };
        if key.is_empty() {
            return Err(format!("empty key in `{path}`"));
        }
        steps.push(Step::Key(key.to_string()));
        while !rest.is_empty() {
            let close = rest
                .find(']')
                .filter(|_| rest.starts_with('['))
                .ok_or_else(|| format!("bad index in `{path}`"))?;
            let index: usize = rest[1..close]
                .parse()
                .map_err(|_| format!("bad index in `{path}`"))?;
            if index > MAX_INDEX {
                return Err(format!("index {index} in `{path}` is above {MAX_INDEX}"));
            }
            steps.push(Step::Index(index));
            rest = &rest[close + 1..];
        }
    }
    Ok(steps)
}

/// Turns a label value into JSON: JSON arrays and objects are parsed, and
/// everything else stays a string (the importer converts numeric and
/// boolean strings where it expects them).
pub(crate) fn parse_value(raw: &str) -> Value {
    let trimmed = raw.trim();
    if ((trimmed.starts_with('[') && trimmed.ends_with(']'))
        || (trimmed.starts_with('{') && trimmed.ends_with('}')))
        && let Ok(value) = serde_json::from_str(trimmed)
    {
        return value;
    }
    Value::String(raw.to_string())
}

/// Sets `value` at `steps` inside `root`, creating maps and lists as
/// needed. Fails when the path crosses a value of another shape (for
/// example `widget=x` and `widget.type=y` on the same container).
pub(crate) fn set_path(
    root: &mut Map<String, Value>,
    steps: &[Step],
    value: Value,
) -> Result<(), String> {
    let Some((Step::Key(first), rest)) = steps.split_first() else {
        return Err("path must start with a key".into());
    };
    if rest.is_empty() {
        root.insert(first.clone(), value);
        return Ok(());
    }
    let slot = root.entry(first.clone()).or_insert(Value::Null);
    set_in(slot, rest, value)
}

fn set_in(slot: &mut Value, steps: &[Step], value: Value) -> Result<(), String> {
    let Some((step, rest)) = steps.split_first() else {
        *slot = value;
        return Ok(());
    };
    match step {
        Step::Key(key) => {
            if slot.is_null() {
                *slot = Value::Object(Map::new());
            }
            let Value::Object(map) = slot else {
                return Err(format!("`{key}` is set on a value that is not a map"));
            };
            set_in(map.entry(key.clone()).or_insert(Value::Null), rest, value)
        }
        Step::Index(index) => {
            if slot.is_null() {
                *slot = Value::Array(Vec::new());
            }
            let Value::Array(list) = slot else {
                return Err(format!(
                    "index [{index}] is used on a value that is not a list"
                ));
            };
            if list.len() <= *index {
                list.resize(*index + 1, Value::Null);
            }
            set_in(&mut list[*index], rest, value)
        }
    }
}

/// Builds the gethomepage field map for one container from its labels.
///
/// Plain `homepage.*` labels are applied first, then
/// `homepage.instance.<instance>.*` labels for the given instance, so the
/// instance-specific values win. Labels for other instances are ignored.
/// Returns the field map and one message per label that could not be used.
pub(crate) fn fields_from_labels<'a>(
    labels: impl IntoIterator<Item = (&'a String, &'a String)>,
    instance: Option<&str>,
) -> (Map<String, Value>, Vec<String>) {
    let mut base = Vec::new();
    let mut overrides = Vec::new();
    for (key, value) in labels {
        let Some(path) = key.strip_prefix(PREFIX) else {
            continue;
        };
        if let Some(rest) = path.strip_prefix("instance.") {
            let Some((name, path)) = rest.split_once('.') else {
                continue;
            };
            if Some(name) == instance {
                overrides.push((key, path, value));
            }
        } else {
            base.push((key, path, value));
        }
    }
    let mut fields = Map::new();
    let mut problems = Vec::new();
    for (label, path, value) in base.into_iter().chain(overrides) {
        let result =
            parse_path(path).and_then(|steps| set_path(&mut fields, &steps, parse_value(value)));
        if let Err(message) = result {
            problems.push(format!("label `{label}`: {message}"));
        }
    }
    // Lists built from sparse indexes may contain gaps; drop them.
    if let Some(Value::Array(widgets)) = fields.get_mut("widgets") {
        widgets.retain(|w| !w.is_null());
    }
    (fields, problems)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::collections::BTreeMap;

    #[test]
    fn parses_paths() {
        assert_eq!(
            parse_path("widget.mappings[0].field").unwrap(),
            vec![
                Step::Key("widget".into()),
                Step::Key("mappings".into()),
                Step::Index(0),
                Step::Key("field".into()),
            ]
        );
        assert!(parse_path("a..b").is_err());
        assert!(parse_path("a[x]").is_err());
        assert!(parse_path("a[1").is_err());
        assert!(parse_path("widgets[33]").is_err());
    }

    #[test]
    fn builds_nested_fields_and_applies_instances() {
        let labels: BTreeMap<String, String> = [
            ("homepage.name", "Sonarr"),
            ("homepage.href", "http://public"),
            ("homepage.widget.type", "sonarr"),
            ("homepage.widget.fields", r#"["queued","wanted"]"#),
            ("homepage.widgets[1].type", "customapi"),
            ("homepage.widgets[1].mappings[0].field", "x"),
            ("homepage.instance.internal.href", "http://internal"),
            ("homepage.instance.other.href", "http://other"),
            ("traefik.enable", "true"),
        ]
        .into_iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();

        let (fields, problems) = fields_from_labels(&labels, Some("internal"));
        assert!(problems.is_empty(), "{problems:?}");
        assert_eq!(fields["href"], json!("http://internal"));
        assert_eq!(fields["widget"]["fields"], json!(["queued", "wanted"]));
        assert_eq!(
            fields["widgets"],
            json!([{"type": "customapi", "mappings": [{"field": "x"}]}])
        );
        assert!(!fields.contains_key("traefik"));

        let (fields, _) = fields_from_labels(&labels, None);
        assert_eq!(fields["href"], json!("http://public"));
    }

    #[test]
    fn reports_conflicting_labels() {
        let labels: BTreeMap<String, String> = [
            ("homepage.widget", "plain"),
            ("homepage.widget.type", "sonarr"),
        ]
        .into_iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
        let (_, problems) = fields_from_labels(&labels, None);
        assert_eq!(problems.len(), 1);
    }
}
