//! Custom API: show fields of any JSON API as stats.
//!
//! The admin gives the full API URL (path and query included), an optional
//! method, headers, body and secret key, and a list of mappings. Each
//! mapping picks a value out of the JSON response with a dot path such as
//! `data.system.load.0` and says how to format it.
//!
//! The request goes to exactly the configured URL, through
//! [`WidgetContext::request_url`], so the host allow-list still applies:
//! the widget can only reach the host of its own `url`.
//!
//! Mappings use this shape:
//!
//! ```json
//! [
//!   {"field": "data.memory.percent", "label": "Memory", "format": "percent"},
//!   {"field": {"data": {"hostname": "name"}}, "label": "Host"}
//! ]
//! ```
//!
//! `field` may also be given in the nested object form (`{"a": {"b":
//! "c"}}` means `a.b.c`), for compatibility with gethomepage configs.

use async_trait::async_trait;
use reqwest::Method;
use reqwest::header::{HeaderMap, HeaderName, HeaderValue};
use serde_json::Value;
use url::Url;

use crate::config::WidgetConfig;
use crate::context::WidgetContext;
use crate::data::{Stat, StatValue, WidgetData};
use crate::error::WidgetError;
use crate::schema::{FieldSpec, FieldType, WidgetSchema};
use crate::widget::Widget;

/// Most mappings a widget shows; extra mappings are ignored.
pub const MAX_MAPPINGS: usize = 20;

/// Default header carrying the secret `key`.
pub const DEFAULT_KEY_HEADER: &str = "Authorization";

/// Generic JSON API widget with a field mapping.
#[derive(Debug, Default, Clone, Copy)]
pub struct CustomApi;

/// How a mapped value is displayed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Format {
    /// Plain number.
    Number,
    /// Text, shown as is.
    #[default]
    Text,
    /// Size in bytes.
    Bytes,
    /// Bytes per second.
    Rate,
    /// Percentage, 0 to 100.
    Percent,
    /// Duration in seconds.
    Duration,
    /// Duration in milliseconds.
    Millis,
}

impl Format {
    /// Parses a format name. A few gethomepage spellings are accepted as
    /// aliases (`float` and `integer` for `number`, `size` for `bytes`,
    /// `string` for `text`).
    pub fn parse(name: &str) -> Option<Self> {
        Some(match name.trim().to_ascii_lowercase().as_str() {
            "number" | "float" | "integer" | "int" => Self::Number,
            "text" | "string" => Self::Text,
            "bytes" | "size" => Self::Bytes,
            "rate" => Self::Rate,
            "percent" => Self::Percent,
            "duration" => Self::Duration,
            "millis" | "ms" => Self::Millis,
            _ => return None,
        })
    }
}

/// One configured mapping from a response field to a stat.
#[derive(Debug, Clone, PartialEq)]
pub struct Mapping {
    /// Dot path into the response, e.g. `data.items.0.name`.
    pub field: String,
    /// Display label (defaults to the path).
    pub label: String,
    /// Display format.
    pub format: Format,
}

fn invalid(msg: impl Into<String>) -> WidgetError {
    WidgetError::InvalidConfig(msg.into())
}

/// Accepts a JSON value, or a string holding JSON (as a text input may
/// store it).
fn json_value(v: &Value) -> Result<Value, WidgetError> {
    match v {
        Value::String(s) => {
            serde_json::from_str(s).map_err(|e| invalid(format!("not valid JSON: {e}")))
        }
        other => Ok(other.clone()),
    }
}

/// Turns a `field` value into a dot path.
///
/// Strings are used as is, numbers become an index, and single-key objects
/// are flattened recursively: `{"a": {"b": "c"}}` gives `a.b.c`.
pub fn flatten_field(field: &Value) -> Result<String, WidgetError> {
    match field {
        Value::String(s) if !s.trim().is_empty() => Ok(s.trim().to_owned()),
        Value::Number(n) => Ok(n.to_string()),
        Value::Object(map) if map.len() == 1 => {
            let (key, rest) = map.iter().next().expect("one entry");
            Ok(format!("{key}.{}", flatten_field(rest)?))
        }
        _ => Err(invalid(
            "mapping `field` must be a dot path or a nested object with one key per level",
        )),
    }
}

/// Parses the `mappings` setting, keeping at most [`MAX_MAPPINGS`].
pub fn parse_mappings(value: Option<&Value>) -> Result<Vec<Mapping>, WidgetError> {
    let value = json_value(value.ok_or_else(|| invalid("`mappings` is required"))?)?;
    let Value::Array(list) = value else {
        return Err(invalid("`mappings` must be a list"));
    };
    if list.is_empty() {
        return Err(invalid("`mappings` must not be empty"));
    }
    list.iter()
        .take(MAX_MAPPINGS)
        .map(|m| {
            let Value::Object(m) = m else {
                return Err(invalid("each mapping must be an object"));
            };
            let field = flatten_field(m.get("field").unwrap_or(&Value::Null))?;
            let label = m
                .get("label")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .unwrap_or(&field)
                .to_owned();
            let format = match m.get("format") {
                None | Some(Value::Null) => Format::default(),
                Some(Value::String(s)) => Format::parse(s)
                    .ok_or_else(|| invalid(format!("unknown mapping format `{s}`")))?,
                Some(_) => return Err(invalid("mapping `format` must be text")),
            };
            Ok(Mapping {
                field,
                label,
                format,
            })
        })
        .collect()
}

/// Parses the `headers` setting (an object of header name to value) into a
/// validated header map.
pub fn parse_headers(value: Option<&Value>) -> Result<HeaderMap, WidgetError> {
    let mut headers = HeaderMap::new();
    let Some(value) = value else {
        return Ok(headers);
    };
    let Value::Object(map) = json_value(value)? else {
        return Err(invalid("`headers` must be an object of name → value"));
    };
    for (name, v) in &map {
        let text = match v {
            Value::String(s) => s.clone(),
            Value::Number(_) | Value::Bool(_) => v.to_string(),
            _ => return Err(invalid(format!("header `{name}` must have a text value"))),
        };
        let (name, value) = header_pair(name, &text)?;
        headers.insert(name, value);
    }
    Ok(headers)
}

fn header_pair(name: &str, value: &str) -> Result<(HeaderName, HeaderValue), WidgetError> {
    let n = HeaderName::from_bytes(name.trim().as_bytes())
        .map_err(|_| invalid(format!("`{name}` is not a valid header name")))?;
    // The value is not echoed: it may be a credential.
    let v = HeaderValue::from_str(value)
        .map_err(|_| invalid(format!("header `{name}` has an invalid value")))?;
    Ok((n, v))
}

/// Looks up a dot path in a JSON value. Numeric segments index arrays;
/// other segments are object keys.
pub fn lookup<'a>(root: &'a Value, path: &str) -> Option<&'a Value> {
    path.split('.').try_fold(root, |cur, seg| match cur {
        Value::Object(map) => map.get(seg),
        Value::Array(list) => seg.parse::<usize>().ok().and_then(|i| list.get(i)),
        _ => None,
    })
}

/// Formats one looked-up value. Missing values, and values that are not
/// numbers or numeric strings for a numeric format, give `-`.
pub fn to_stat_value(value: Option<&Value>, format: Format) -> StatValue {
    let missing = || StatValue::Text("-".into());
    let Some(value) = value else {
        return missing();
    };
    if format == Format::Text {
        return match value {
            Value::Null => missing(),
            Value::String(s) => StatValue::Text(s.clone()),
            other => StatValue::Text(other.to_string()),
        };
    }
    let n = match value {
        Value::Number(n) => n.as_f64(),
        Value::String(s) => s.trim().parse::<f64>().ok().filter(|n| n.is_finite()),
        _ => None,
    };
    let Some(n) = n else {
        return missing();
    };
    match format {
        Format::Number => StatValue::Number(n),
        Format::Bytes => StatValue::Bytes(n),
        Format::Rate => StatValue::Rate(n),
        Format::Percent => StatValue::Percent(n),
        Format::Duration => StatValue::Duration(n),
        Format::Millis => StatValue::Millis(n),
        Format::Text => unreachable!("handled above"),
    }
}

/// Applies the mappings to a response.
///
/// Stat keys are the field paths, made unique by suffixing `_2`, `_3`, ...
/// when the same path is mapped more than once.
pub fn parse_response(root: &Value, mappings: &[Mapping]) -> WidgetData {
    let mut stats: Vec<Stat> = Vec::with_capacity(mappings.len());
    for m in mappings {
        let mut key = m.field.clone();
        let mut n = 1;
        while stats.iter().any(|s| s.key == key) {
            n += 1;
            key = format!("{}_{n}", m.field);
        }
        let value = to_stat_value(lookup(root, &m.field), m.format);
        stats.push(Stat::new(&key, &m.label, value));
    }
    WidgetData::new(stats)
}

#[async_trait]
impl Widget for CustomApi {
    fn kind(&self) -> &'static str {
        "customapi"
    }

    fn schema(&self) -> WidgetSchema {
        let mut schema = WidgetSchema::new(
            "customapi",
            "Custom API",
            "Fields of any JSON API, picked with a mapping.",
        );
        schema.fields[0].help =
            "Full API URL, including path and query, as reachable from the homedit server.".into();
        schema
            .field(
                FieldSpec::new("method", "Method", FieldType::select(&["GET", "POST"]))
                    .default_value("GET"),
            )
            .field(
                FieldSpec::new("headers", "Headers", FieldType::Json)
                    .help(r#"Extra request headers, e.g. {"Accept": "application/json"}."#),
            )
            .field(
                FieldSpec::new("body", "Body", FieldType::Json)
                    .help("JSON body sent with POST requests (a string is sent as is)."),
            )
            .field(
                FieldSpec::new("key", "Secret", FieldType::Secret)
                    .help("Optional API key or token, sent in the header below."),
            )
            .field(
                FieldSpec::new("key_header", "Secret header", FieldType::Text)
                    .default_value(DEFAULT_KEY_HEADER)
                    .help("Header carrying the secret, e.g. Authorization or X-Api-Key."),
            )
            .field(
                FieldSpec::new("mappings", "Mappings", FieldType::Json)
                    .required()
                    .help(
                        r#"Up to 20 entries like {"field": "data.items.0.count", "label": "Items", "format": "number"}. Formats: number, text, bytes, rate, percent, duration, millis."#,
                    ),
            )
    }

    async fn fetch(
        &self,
        ctx: &WidgetContext,
        cfg: &WidgetConfig,
    ) -> Result<WidgetData, WidgetError> {
        let mappings = parse_mappings(cfg.get("mappings"))?;
        let mut headers = parse_headers(cfg.get("headers"))?;
        if let Some(key) = cfg.str("key") {
            let name = cfg.str("key_header").unwrap_or(DEFAULT_KEY_HEADER);
            let (name, mut value) = header_pair(name, key)?;
            value.set_sensitive(true);
            headers.insert(name, value);
        }
        let method = match cfg.str("method").unwrap_or("GET") {
            m if m.eq_ignore_ascii_case("POST") => Method::POST,
            m if m.eq_ignore_ascii_case("GET") => Method::GET,
            _ => return Err(invalid("`method` must be GET or POST")),
        };
        let url = Url::parse(cfg.require_str("url")?)
            .map_err(|e| invalid(format!("`url` is not a valid URL: {e}")))?;

        let is_post = method == Method::POST;
        let mut req = ctx.request_url(cfg, method, url)?.headers(headers);
        if is_post && let Some(body) = cfg.get("body") {
            req = match body {
                Value::String(s) => req.body(s.clone()),
                other => req.json(other),
            };
        }
        let root: Value = ctx.send_json(req).await?;
        Ok(parse_response(&root, &mappings))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::widgets::fixture;
    use serde_json::json;
    use wiremock::matchers::{body_json, header, method, path, query_param};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn mappings(v: Value) -> Vec<Mapping> {
        parse_mappings(Some(&v)).unwrap()
    }

    #[test]
    fn maps_recorded_object_response() {
        let root = fixture("customapi/response.json");
        let m = mappings(json!([
            {"field": "data.system.uptime", "label": "Uptime", "format": "duration"},
            {"field": "data.system.load.0", "label": "Load", "format": "number"},
            {"field": {"data": {"system": {"memory": "used_bytes"}}}, "label": "Memory", "format": "bytes"},
            {"field": "data.system.memory.percent", "label": "Memory %", "format": "percent"},
            {"field": "data.network.rx_rate", "label": "Down", "format": "rate"},
            {"field": "data.hostname", "label": "Host"},
            {"field": "data.latency_ms", "label": "Latency", "format": "millis"},
            {"field": "data.missing.path", "label": "Gone", "format": "number"},
            {"field": "data.hostname", "label": "Host as number", "format": "number"}
        ]));
        let d = parse_response(&root, &m);
        let v = |k: &str| d.stat(k).unwrap().value.clone();
        assert_eq!(v("data.system.uptime"), StatValue::Duration(86400.0));
        assert_eq!(v("data.system.load.0"), StatValue::Number(0.42));
        assert_eq!(
            v("data.system.memory.used_bytes"),
            StatValue::Bytes(3221225472.0)
        );
        assert_eq!(v("data.system.memory.percent"), StatValue::Percent(41.5));
        assert_eq!(v("data.network.rx_rate"), StatValue::Rate(125000.5));
        assert_eq!(v("data.hostname"), StatValue::Text("nas01".into()));
        assert_eq!(v("data.latency_ms"), StatValue::Millis(12.0));
        assert_eq!(v("data.missing.path"), StatValue::Text("-".into()));
        assert_eq!(v("data.hostname_2"), StatValue::Text("-".into()));
        assert_eq!(d.stat("data.system.uptime").unwrap().label, "Uptime");
    }

    #[test]
    fn maps_recorded_array_response() {
        let root = fixture("customapi/array.json");
        let m = mappings(json!([
            {"field": "0.free", "label": "Pool 0", "format": "bytes"},
            {"field": "1.free", "label": "Pool 1", "format": "bytes"},
            {"field": "1.healthy"},
            {"field": "5.free", "format": "bytes"}
        ]));
        let d = parse_response(&root, &m);
        assert_eq!(
            d.stat("0.free").unwrap().value,
            StatValue::Bytes(1099511627776.0)
        );
        assert_eq!(
            d.stat("1.free").unwrap().value,
            StatValue::Bytes(549755813888.0)
        );
        assert_eq!(
            d.stat("1.healthy").unwrap().value,
            StatValue::Text("false".into())
        );
        assert_eq!(d.stat("1.healthy").unwrap().label, "1.healthy");
        assert_eq!(d.stat("5.free").unwrap().value, StatValue::Text("-".into()));
    }

    #[test]
    fn rejects_bad_mappings_and_caps_count() {
        assert!(parse_mappings(None).is_err());
        assert!(parse_mappings(Some(&json!({}))).is_err());
        assert!(parse_mappings(Some(&json!([]))).is_err());
        assert!(parse_mappings(Some(&json!([{"label": "x"}]))).is_err());
        assert!(parse_mappings(Some(&json!([{"field": {"a": 1, "b": 2}}]))).is_err());
        assert!(parse_mappings(Some(&json!([{"field": "a", "format": "weird"}]))).is_err());
        let many: Vec<Value> = (0..30).map(|i| json!({"field": format!("f{i}")})).collect();
        assert_eq!(mappings(Value::Array(many)).len(), MAX_MAPPINGS);
        // A string holding JSON is accepted too.
        let from_text = mappings(json!(r#"[{"field": "a.b", "format": "size"}]"#));
        assert_eq!(from_text[0].format, Format::Bytes);
    }

    #[test]
    fn validates_headers() {
        let h = parse_headers(Some(&json!({"Accept": "application/json", "X-N": 5}))).unwrap();
        assert_eq!(h["accept"], "application/json");
        assert_eq!(h["x-n"], "5");
        assert!(parse_headers(Some(&json!({"Bad Header": "x"}))).is_err());
        assert!(parse_headers(Some(&json!({"X-Ok": "line\nbreak"}))).is_err());
        assert!(parse_headers(Some(&json!({"X-Ok": {"nested": 1}}))).is_err());
        assert!(parse_headers(Some(&json!(["a"]))).is_err());
    }

    #[tokio::test]
    async fn posts_body_with_headers_and_key() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/stats"))
            .and(query_param("scope", "all"))
            .and(header("X-Api-Key", "s3cret"))
            .and(header("X-Client", "homedit"))
            .and(body_json(json!({"query": "summary", "limit": 5})))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(fixture("customapi/response.json")),
            )
            .expect(1)
            .mount(&server)
            .await;
        let ctx = WidgetContext::with_defaults().unwrap();
        let cfg = WidgetConfig::default()
            .with("url", format!("{}/api/stats?scope=all", server.uri()))
            .with("method", "POST")
            .with("headers", json!({"X-Client": "homedit"}))
            .with("body", json!({"query": "summary", "limit": 5}))
            .with("key", "s3cret")
            .with("key_header", "X-Api-Key")
            .with(
                "mappings",
                json!([{"field": "data.hostname", "label": "Host"}]),
            )
            .with_defaults(&CustomApi.schema());
        let d = CustomApi.fetch(&ctx, &cfg).await.unwrap();
        assert_eq!(
            d.stat("data.hostname").unwrap().value,
            StatValue::Text("nas01".into())
        );
    }

    #[tokio::test]
    async fn get_uses_default_key_header() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/pools"))
            .and(header("Authorization", "Bearer t0ken"))
            .respond_with(ResponseTemplate::new(200).set_body_json(fixture("customapi/array.json")))
            .expect(1)
            .mount(&server)
            .await;
        let ctx = WidgetContext::with_defaults().unwrap();
        let cfg = WidgetConfig::default()
            .with("url", format!("{}/pools", server.uri()))
            .with("key", "Bearer t0ken")
            .with("mappings", json!([{"field": "0.name"}]))
            .with_defaults(&CustomApi.schema());
        let d = CustomApi.fetch(&ctx, &cfg).await.unwrap();
        assert_eq!(
            d.stat("0.name").unwrap().value,
            StatValue::Text("pool0".into())
        );
    }

    #[tokio::test]
    async fn bad_header_is_invalid_config() {
        let ctx = WidgetContext::with_defaults().unwrap();
        let cfg = WidgetConfig::default()
            .with("url", "http://127.0.0.1:9/x")
            .with("headers", json!({"Bad Header": "x"}))
            .with("mappings", json!([{"field": "a"}]));
        assert!(matches!(
            CustomApi.fetch(&ctx, &cfg).await,
            Err(WidgetError::InvalidConfig(_))
        ));
    }
}
