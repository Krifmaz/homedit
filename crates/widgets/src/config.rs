//! Per-instance widget configuration.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use url::Url;

use crate::error::WidgetError;
use crate::schema::{FieldType, WidgetSchema};

/// The settings of one widget instance on the dashboard.
///
/// Values are keyed by [`crate::FieldSpec::key`]. The server decrypts secret
/// fields before handing the config to a widget, so widgets always see
/// plain values.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct WidgetConfig {
    /// Field values, including `url`.
    #[serde(flatten)]
    pub values: Map<String, Value>,
}

impl WidgetConfig {
    /// Creates a config from a JSON object. Non-object values give an empty
    /// config.
    pub fn from_json(value: Value) -> Self {
        match value {
            Value::Object(values) => Self { values },
            _ => Self::default(),
        }
    }

    /// Splits a gethomepage-shaped widget block, as stored in
    /// `homedit_core::Service::widget` (`{"type": "sonarr", "url": ..,
    /// "key": ..}`), into the widget kind and its config.
    pub fn from_homepage(value: &Value) -> Result<(String, Self), WidgetError> {
        let Value::Object(map) = value else {
            return Err(WidgetError::InvalidConfig(
                "widget must be an object".into(),
            ));
        };
        let kind = map
            .get("type")
            .and_then(Value::as_str)
            .map(|s| s.trim().to_ascii_lowercase())
            .filter(|s| !s.is_empty())
            .ok_or_else(|| WidgetError::InvalidConfig("widget `type` is required".into()))?;
        let mut values = map.clone();
        values.remove("type");
        Ok((kind, Self { values }))
    }

    /// Sets a value, returning `self` for chaining (mostly useful in tests).
    #[must_use]
    pub fn with(mut self, key: &str, value: impl Into<Value>) -> Self {
        self.values.insert(key.into(), value.into());
        self
    }

    /// The raw value of a field, treating JSON `null` and empty strings as
    /// unset.
    pub fn get(&self, key: &str) -> Option<&Value> {
        match self.values.get(key) {
            None | Some(Value::Null) => None,
            Some(Value::String(s)) if s.trim().is_empty() => None,
            Some(v) => Some(v),
        }
    }

    /// A string field, if set.
    pub fn str(&self, key: &str) -> Option<&str> {
        self.get(key).and_then(Value::as_str).map(str::trim)
    }

    /// A required string field.
    pub fn require_str(&self, key: &str) -> Result<&str, WidgetError> {
        self.str(key)
            .ok_or_else(|| WidgetError::InvalidConfig(format!("`{key}` is required")))
    }

    /// A boolean field, accepting JSON booleans and the strings `true`/`false`.
    pub fn bool(&self, key: &str) -> Option<bool> {
        match self.get(key)? {
            Value::Bool(b) => Some(*b),
            Value::String(s) => s.parse().ok(),
            _ => None,
        }
    }

    /// A numeric field, accepting JSON numbers and numeric strings.
    pub fn f64(&self, key: &str) -> Option<f64> {
        match self.get(key)? {
            Value::Number(n) => n.as_f64(),
            Value::String(s) => s.trim().parse().ok(),
            _ => None,
        }
    }

    /// The base URL of the service, without a trailing slash.
    pub fn base_url(&self) -> Result<Url, WidgetError> {
        let raw = self.require_str("url")?;
        let mut url = Url::parse(raw)
            .map_err(|e| WidgetError::InvalidConfig(format!("`url` is not a valid URL: {e}")))?;
        if !matches!(url.scheme(), "http" | "https") {
            return Err(WidgetError::InvalidConfig(
                "`url` must start with http:// or https://".into(),
            ));
        }
        if url.host_str().is_none() {
            return Err(WidgetError::InvalidConfig("`url` has no host".into()));
        }
        let trimmed = url.path().trim_end_matches('/').to_owned();
        url.set_path(&trimmed);
        url.set_query(None);
        url.set_fragment(None);
        Ok(url)
    }

    /// Joins a path (with optional query string) onto the base URL, keeping
    /// any path prefix of the base URL (for services behind a sub-path).
    pub fn endpoint(&self, path_and_query: &str) -> Result<Url, WidgetError> {
        let base = self.base_url()?;
        let joined = format!(
            "{}/{}",
            base.as_str().trim_end_matches('/'),
            path_and_query.trim_start_matches('/')
        );
        Url::parse(&joined).map_err(|e| WidgetError::InvalidConfig(e.to_string()))
    }

    /// Fills in schema defaults for unset fields.
    pub fn with_defaults(mut self, schema: &WidgetSchema) -> Self {
        for field in &schema.fields {
            if self.get(&field.key).is_none()
                && let Some(default) = &field.default
            {
                self.values.insert(field.key.clone(), default.clone());
            }
        }
        self
    }

    /// Checks the config against a schema: required fields are present and
    /// every set value has the right type.
    pub fn validate(&self, schema: &WidgetSchema) -> Result<(), WidgetError> {
        let invalid = |msg: String| Err(WidgetError::InvalidConfig(msg));
        for field in &schema.fields {
            let Some(value) = self.get(&field.key) else {
                if field.required && field.default.is_none() {
                    return invalid(format!("`{}` is required", field.label));
                }
                continue;
            };
            match &field.field_type {
                FieldType::Url => {
                    let Some(s) = value.as_str() else {
                        return invalid(format!("`{}` must be a URL", field.label));
                    };
                    match Url::parse(s.trim()) {
                        Ok(u) if matches!(u.scheme(), "http" | "https") => {}
                        _ => return invalid(format!("`{}` must be an http(s) URL", field.label)),
                    }
                }
                FieldType::Text | FieldType::Secret => {
                    if !value.is_string() {
                        return invalid(format!("`{}` must be text", field.label));
                    }
                }
                FieldType::Number { min, max } => {
                    let Some(n) = self.f64(&field.key) else {
                        return invalid(format!("`{}` must be a number", field.label));
                    };
                    if min.is_some_and(|m| n < m) || max.is_some_and(|m| n > m) {
                        return invalid(format!("`{}` is out of range", field.label));
                    }
                }
                FieldType::Bool => {
                    if self.bool(&field.key).is_none() {
                        return invalid(format!("`{}` must be true or false", field.label));
                    }
                }
                FieldType::Select { options } => {
                    let ok = value
                        .as_str()
                        .is_some_and(|s| options.iter().any(|o| o.value == s));
                    if !ok {
                        return invalid(format!("`{}` has an unknown value", field.label));
                    }
                }
                FieldType::Json => {}
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::FieldSpec;
    use serde_json::json;

    fn schema() -> WidgetSchema {
        WidgetSchema::new("t", "T", "test")
            .field(FieldSpec::new("key", "API key", FieldType::Secret).required())
            .field(
                FieldSpec::new(
                    "n",
                    "N",
                    FieldType::Number {
                        min: Some(1.0),
                        max: Some(5.0),
                    },
                )
                .default_value(2),
            )
            .field(FieldSpec::new(
                "mode",
                "Mode",
                FieldType::select(&["a", "b"]),
            ))
    }

    #[test]
    fn validates_required_and_types() {
        let s = schema();
        let ok = WidgetConfig::from_json(json!({"url": "http://x:1", "key": "k", "mode": "a"}));
        ok.validate(&s).unwrap();
        let missing = WidgetConfig::from_json(json!({"url": "http://x"}));
        assert!(missing.validate(&s).is_err());
        let bad_url = WidgetConfig::from_json(json!({"url": "ftp://x", "key": "k"}));
        assert!(bad_url.validate(&s).is_err());
        let range = WidgetConfig::from_json(json!({"url": "http://x", "key": "k", "n": 9}));
        assert!(range.validate(&s).is_err());
        let select = WidgetConfig::from_json(json!({"url": "http://x", "key": "k", "mode": "z"}));
        assert!(select.validate(&s).is_err());
    }

    #[test]
    fn splits_homepage_block() {
        let (kind, cfg) =
            WidgetConfig::from_homepage(&json!({"type": "Sonarr", "url": "http://s", "key": "k"}))
                .unwrap();
        assert_eq!(kind, "sonarr");
        assert_eq!(cfg.str("key"), Some("k"));
        assert!(cfg.get("type").is_none());
        assert!(WidgetConfig::from_homepage(&json!({"url": "http://s"})).is_err());
    }

    #[test]
    fn endpoint_keeps_subpath() {
        let c = WidgetConfig::default().with("url", "https://host/sonarr/");
        assert_eq!(
            c.endpoint("/api/v3/queue?page=1").unwrap().as_str(),
            "https://host/sonarr/api/v3/queue?page=1"
        );
    }

    #[test]
    fn defaults_fill_unset() {
        let c = WidgetConfig::default().with_defaults(&schema());
        assert_eq!(c.f64("n"), Some(2.0));
    }
}
