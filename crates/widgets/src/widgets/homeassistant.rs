//! Home Assistant: people at home, lights and switches on, or custom
//! entity states and templates.
//!
//! Authenticates with a long-lived access token sent as
//! `Authorization: Bearer <key>`. Without `custom` entries the widget
//! renders one template through `POST /api/template` that counts people at
//! home and lights and switches that are on. With `custom` entries
//! (gethomepage's format) it shows those instead:
//!
//! - `{"state": "sensor.outside_temperature", "label": "Outside",
//!   "value": "{state} {attributes.unit_of_measurement}"}` reads
//!   `GET /api/states/<entity_id>` and fills the placeholders.
//! - `{"template": "{{ states.light | list | count }}", "label": "Lights"}`
//!   renders the template through `POST /api/template`.

use async_trait::async_trait;
use reqwest::Method;
use serde::Deserialize;
use serde_json::Value;

use crate::config::WidgetConfig;
use crate::context::WidgetContext;
use crate::data::{Stat, StatValue, WidgetData};
use crate::error::WidgetError;
use crate::schema::{FieldSpec, FieldType, WidgetSchema};
use crate::widget::Widget;
use crate::widgets::api_key_field;

/// Most custom entries a widget shows.
pub const MAX_CUSTOM: usize = 8;

/// Separator between the default counts in one rendered template.
const SEP: &str = "\u{1f}";

/// Default value format of a `state` entry.
const DEFAULT_VALUE: &str = "{state} {attributes.unit_of_measurement}";

/// Template that renders `home/total` for people and `on/total` for lights
/// and switches, separated by [`SEP`].
fn default_template() -> String {
    let count = |domain: &str, state: &str| {
        format!(
            "{{{{ states.{domain} | selectattr('state', 'eq', '{state}') | list | count }}}}/\
             {{{{ states.{domain} | list | count }}}}"
        )
    };
    [
        count("person", "home"),
        count("light", "on"),
        count("switch", "on"),
    ]
    .join(SEP)
}

/// Home Assistant widget.
#[derive(Debug, Default, Clone, Copy)]
pub struct HomeAssistant;

/// One `custom` entry.
#[derive(Debug, Clone, PartialEq)]
pub enum CustomEntry {
    /// Show an entity's state.
    State {
        /// Entity id, e.g. `sensor.outside_temperature`.
        entity_id: String,
        /// Label shown above the value.
        label: String,
        /// Format with `{state}` and `{attributes.<name>}` placeholders.
        value: String,
    },
    /// Show a rendered template.
    Template {
        /// Jinja template evaluated by Home Assistant.
        template: String,
        /// Label shown above the value.
        label: String,
    },
}

/// `GET /api/states/<entity_id>` (only the fields used).
#[derive(Debug, Default, Deserialize)]
pub struct EntityState {
    /// Entity id.
    #[serde(default)]
    pub entity_id: String,
    /// Current state, e.g. `21.5` or `on`.
    #[serde(default)]
    pub state: String,
    /// Entity attributes, e.g. `unit_of_measurement`, `friendly_name`.
    #[serde(default)]
    pub attributes: serde_json::Map<String, Value>,
}

/// Whether `id` looks like a Home Assistant entity id (`domain.object_id`
/// in lowercase letters, digits and underscores), so it is safe in a path.
pub fn valid_entity_id(id: &str) -> bool {
    let ok = |s: &str| {
        !s.is_empty()
            && s.chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
    };
    match id.split_once('.') {
        Some((domain, object)) => ok(domain) && ok(object),
        None => false,
    }
}

/// A trimmed string member of a JSON object.
fn text<'a>(v: &'a Value, key: &str) -> Option<&'a str> {
    v.get(key).and_then(Value::as_str).map(str::trim)
}

/// Parses the `custom` setting (an array, or a string holding one).
pub fn parse_custom(value: Option<&Value>) -> Result<Vec<CustomEntry>, WidgetError> {
    let invalid = |m: &str| WidgetError::InvalidConfig(format!("custom: {m}"));
    let parsed;
    let list = match value {
        None => return Ok(Vec::new()),
        Some(Value::String(s)) => {
            parsed = serde_json::from_str::<Value>(s).map_err(|e| invalid(&e.to_string()))?;
            &parsed
        }
        Some(v) => v,
    };
    let Value::Array(items) = list else {
        return Err(invalid("must be a list"));
    };
    items
        .iter()
        .take(MAX_CUSTOM)
        .map(|item| {
            if let Some(entity_id) = text(item, "state") {
                if !valid_entity_id(entity_id) {
                    return Err(invalid(&format!("`{entity_id}` is not an entity id")));
                }
                Ok(CustomEntry::State {
                    entity_id: entity_id.into(),
                    label: text(item, "label").unwrap_or(entity_id).into(),
                    value: text(item, "value").unwrap_or(DEFAULT_VALUE).into(),
                })
            } else if let Some(template) = text(item, "template") {
                Ok(CustomEntry::Template {
                    template: template.into(),
                    label: text(item, "label").unwrap_or("Template").into(),
                })
            } else {
                Err(invalid("each entry needs `state` or `template`"))
            }
        })
        .collect()
}

/// Fills `{state}` and `{attributes.<name>}` placeholders. Unknown
/// attributes become empty, and the result is trimmed.
pub fn format_state(format: &str, entity: &EntityState) -> String {
    let mut out = String::new();
    let mut rest = format;
    while let Some(start) = rest.find('{') {
        out.push_str(&rest[..start]);
        let after = &rest[start + 1..];
        let Some(end) = after.find('}') else {
            out.push_str(&rest[start..]);
            rest = "";
            break;
        };
        let name = after[..end].trim();
        if name == "state" {
            out.push_str(&entity.state);
        } else if let Some(attr) = name.strip_prefix("attributes.") {
            match entity.attributes.get(attr) {
                Some(Value::String(s)) => out.push_str(s),
                Some(Value::Null) | None => {}
                Some(v) => out.push_str(&v.to_string()),
            }
        } else {
            out.push('{');
            out.push_str(&after[..=end]);
        }
        rest = &after[end + 1..];
    }
    out.push_str(rest);
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Parses the rendered default template into widget data.
pub fn parse_defaults(rendered: &str) -> WidgetData {
    let mut parts = rendered.split(SEP).map(str::trim);
    let mut next = || StatValue::Text(parts.next().unwrap_or("-").to_owned());
    WidgetData::new(vec![
        Stat::new("people_home", "People home", next()),
        Stat::new("lights_on", "Lights on", next()),
        Stat::new("switches_on", "Switches on", next()),
    ])
}

async fn render_template(
    ctx: &WidgetContext,
    cfg: &WidgetConfig,
    auth: &str,
    template: &str,
) -> Result<String, WidgetError> {
    let req = ctx
        .request(cfg, Method::POST, "/api/template")?
        .header("Authorization", auth)
        .header("Content-Type", "application/json")
        .body(serde_json::json!({ "template": template }).to_string());
    let body = ctx.send_bytes(req).await?;
    String::from_utf8(body).map_err(WidgetError::parse)
}

#[async_trait]
impl Widget for HomeAssistant {
    fn kind(&self) -> &'static str {
        "homeassistant"
    }

    fn schema(&self) -> WidgetSchema {
        WidgetSchema::new(
            "homeassistant",
            "Home Assistant",
            "People at home, lights and switches on, or your own entities and templates.",
        )
        .field(api_key_field(
            "Profile → Security → Long-lived access tokens → Create token.",
        ))
        .field(FieldSpec::new("custom", "Custom entries", FieldType::Json).help(
            "Optional list replacing the defaults: {\"state\": \"sensor.x\", \"label\": \"..\", \
             \"value\": \"{state} {attributes.unit_of_measurement}\"} or {\"template\": \"..\", \
             \"label\": \"..\"}.",
        ))
    }

    async fn fetch(
        &self,
        ctx: &WidgetContext,
        cfg: &WidgetConfig,
    ) -> Result<WidgetData, WidgetError> {
        let auth = format!("Bearer {}", cfg.require_str("key")?);
        let custom = parse_custom(cfg.get("custom"))?;
        if custom.is_empty() {
            let rendered = render_template(ctx, cfg, &auth, &default_template()).await?;
            return Ok(parse_defaults(&rendered));
        }
        let mut stats = Vec::with_capacity(custom.len());
        for (i, entry) in custom.iter().enumerate() {
            let key = format!("custom_{i}");
            let (label, value) = match entry {
                CustomEntry::State {
                    entity_id,
                    label,
                    value,
                } => {
                    let entity: EntityState = ctx
                        .get_json(
                            cfg,
                            &format!("/api/states/{entity_id}"),
                            &[("Authorization", auth.as_str())],
                        )
                        .await?;
                    (label, format_state(value, &entity))
                }
                CustomEntry::Template { template, label } => {
                    let rendered = render_template(ctx, cfg, &auth, template).await?;
                    (label, rendered.trim().to_owned())
                }
            };
            stats.push(Stat::new(&key, label, StatValue::Text(value)));
        }
        Ok(WidgetData::new(stats))
    }

    async fn test_connection(
        &self,
        ctx: &WidgetContext,
        cfg: &WidgetConfig,
    ) -> Result<String, WidgetError> {
        let auth = format!("Bearer {}", cfg.require_str("key")?);
        let v: Value = ctx
            .get_json(cfg, "/api/", &[("Authorization", auth.as_str())])
            .await?;
        Ok(v.get("message")
            .and_then(Value::as_str)
            .unwrap_or("Connected")
            .to_owned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::widgets::fixture;
    use serde_json::json;
    use wiremock::matchers::{body_json, header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn text(d: &WidgetData, key: &str) -> String {
        match &d.stat(key).unwrap().value {
            StatValue::Text(s) => s.clone(),
            v => panic!("{key}: {v:?}"),
        }
    }

    #[test]
    fn default_template_is_valid_jinja_shape() {
        let t = default_template();
        assert_eq!(t.matches("{{").count(), 6);
        assert_eq!(t.matches("}}").count(), 6);
        assert!(t.contains("states.person | selectattr('state', 'eq', 'home')"));
    }

    #[test]
    fn parses_rendered_defaults() {
        let d = parse_defaults(&format!("2/3{SEP}4/17{SEP}1/6\n"));
        assert_eq!(text(&d, "people_home"), "2/3");
        assert_eq!(text(&d, "lights_on"), "4/17");
        assert_eq!(text(&d, "switches_on"), "1/6");
    }

    #[test]
    fn formats_recorded_entity_state() {
        let e: EntityState =
            serde_json::from_value(fixture("homeassistant/state_temperature.json")).unwrap();
        assert_eq!(format_state(DEFAULT_VALUE, &e), "7.4 °C");
        assert_eq!(
            format_state(
                "{attributes.friendly_name}: {state}{attributes.missing}",
                &e
            ),
            "Outside temperature: 7.4"
        );
        assert_eq!(format_state("{other} {state", &e), "{other} {state");
    }

    #[test]
    fn validates_custom_entries() {
        let entries = parse_custom(Some(&json!([
            {"state": "sensor.outside_temperature", "label": "Outside"},
            {"template": "{{ 1 + 1 }}"}
        ])))
        .unwrap();
        assert_eq!(entries.len(), 2);
        assert!(matches!(&entries[0], CustomEntry::State { value, .. } if value == DEFAULT_VALUE));
        for bad in ["../../api/config", "sensor", "Sensor.X", "sensor./x"] {
            assert!(
                parse_custom(Some(&json!([{ "state": bad }]))).is_err(),
                "{bad}"
            );
        }
        assert!(parse_custom(Some(&json!([{ "label": "x" }]))).is_err());
        assert!(parse_custom(Some(&json!("[]"))).unwrap().is_empty());
    }

    #[tokio::test]
    async fn fetches_defaults_with_bearer_token() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/template"))
            .and(header("Authorization", "Bearer ha-token"))
            .and(body_json(json!({ "template": default_template() })))
            .respond_with(
                ResponseTemplate::new(200).set_body_string(format!("1/2{SEP}3/12{SEP}0/4")),
            )
            .mount(&server)
            .await;
        let ctx = WidgetContext::with_defaults().unwrap();
        let cfg = WidgetConfig::default()
            .with("url", server.uri())
            .with("key", "ha-token");
        let d = HomeAssistant.fetch(&ctx, &cfg).await.unwrap();
        assert_eq!(text(&d, "people_home"), "1/2");
        assert_eq!(text(&d, "switches_on"), "0/4");
        let bad = cfg.clone().with("key", "wrong");
        assert!(HomeAssistant.fetch(&ctx, &bad).await.is_err());
    }

    #[tokio::test]
    async fn fetches_custom_entries() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/states/sensor.outside_temperature"))
            .and(header("Authorization", "Bearer ha-token"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(fixture("homeassistant/state_temperature.json")),
            )
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/api/template"))
            .and(body_json(
                json!({ "template": "{{ states.media_player | list | count }}" }),
            ))
            .respond_with(ResponseTemplate::new(200).set_body_string("5"))
            .mount(&server)
            .await;
        let ctx = WidgetContext::with_defaults().unwrap();
        let cfg = WidgetConfig::default()
            .with("url", server.uri())
            .with("key", "ha-token")
            .with(
                "custom",
                json!([
                    {"state": "sensor.outside_temperature", "label": "Outside"},
                    {"template": "{{ states.media_player | list | count }}", "label": "Players"}
                ]),
            );
        let d = HomeAssistant.fetch(&ctx, &cfg).await.unwrap();
        assert_eq!(d.stats[0].label, "Outside");
        assert_eq!(text(&d, "custom_0"), "7.4 °C");
        assert_eq!(text(&d, "custom_1"), "5");
    }
}
