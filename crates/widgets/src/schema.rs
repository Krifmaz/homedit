//! Widget configuration schemas.
//!
//! Every widget describes its settings as a [`WidgetSchema`]. The editor
//! renders forms from it (labels, types, required markers, help text), and
//! the server uses it to validate configs and to know which fields are
//! secrets that must be encrypted at rest and masked in API responses.

use serde::{Deserialize, Serialize};

/// The settings a widget accepts, in display order.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WidgetSchema {
    /// Stable identifier, e.g. `sonarr`. Matches [`crate::Widget::kind`].
    pub kind: String,
    /// Human readable name, e.g. `Sonarr`.
    pub display_name: String,
    /// One sentence describing what the widget shows.
    pub description: String,
    /// The fields of the form, in display order. The service URL is always
    /// the first field and always has the key `url`.
    pub fields: Vec<FieldSpec>,
}

impl WidgetSchema {
    /// Starts a schema with the mandatory `url` field.
    pub fn new(kind: &str, display_name: &str, description: &str) -> Self {
        Self {
            kind: kind.into(),
            display_name: display_name.into(),
            description: description.into(),
            fields: vec![
                FieldSpec::new("url", "URL", FieldType::Url)
                    .required()
                    .help("Base URL of the service as reachable from the homedit server."),
            ],
        }
    }

    /// Appends a field.
    #[must_use]
    pub fn field(mut self, field: FieldSpec) -> Self {
        self.fields.push(field);
        self
    }

    /// Looks up a field by key.
    pub fn get(&self, key: &str) -> Option<&FieldSpec> {
        self.fields.iter().find(|f| f.key == key)
    }

    /// Keys of all fields of type [`FieldType::Secret`].
    pub fn secret_keys(&self) -> impl Iterator<Item = &str> {
        self.fields
            .iter()
            .filter(|f| f.field_type == FieldType::Secret)
            .map(|f| f.key.as_str())
    }
}

/// One form field.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FieldSpec {
    /// Key under which the value is stored in [`crate::WidgetConfig`].
    pub key: String,
    /// Label shown next to the input.
    pub label: String,
    /// Input type and validation rules.
    #[serde(rename = "type")]
    pub field_type: FieldType,
    /// Whether the form must not be saved without a value.
    pub required: bool,
    /// Value used when none is given.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default: Option<serde_json::Value>,
    /// Help text shown under the input.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub help: String,
}

impl FieldSpec {
    /// Creates an optional field with no default and no help text.
    pub fn new(key: &str, label: &str, field_type: FieldType) -> Self {
        Self {
            key: key.into(),
            label: label.into(),
            field_type,
            required: false,
            default: None,
            help: String::new(),
        }
    }

    /// Marks the field as required.
    #[must_use]
    pub fn required(mut self) -> Self {
        self.required = true;
        self
    }

    /// Sets the default value.
    #[must_use]
    pub fn default_value(mut self, value: impl Into<serde_json::Value>) -> Self {
        self.default = Some(value.into());
        self
    }

    /// Sets the help text.
    #[must_use]
    pub fn help(mut self, help: &str) -> Self {
        self.help = help.into();
        self
    }
}

/// The type of a form field.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum FieldType {
    /// An `http` or `https` URL.
    Url,
    /// Free text.
    Text,
    /// A credential (API key, token, password). Encrypted at rest, never sent
    /// back to the browser in full and left out of exported YAML by default.
    Secret,
    /// A number, optionally bounded.
    Number {
        /// Inclusive lower bound.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        min: Option<f64>,
        /// Inclusive upper bound.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        max: Option<f64>,
    },
    /// A checkbox.
    Bool,
    /// One value out of a fixed list.
    Select {
        /// The allowed values.
        options: Vec<SelectOption>,
    },
    /// Arbitrary JSON (used by the generic API widget for headers and field
    /// mappings).
    Json,
}

/// An entry of a [`FieldType::Select`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SelectOption {
    /// The stored value.
    pub value: String,
    /// The label shown in the dropdown.
    pub label: String,
}

impl FieldType {
    /// Shorthand for a [`FieldType::Select`] whose values double as labels.
    pub fn select(values: &[&str]) -> Self {
        Self::Select {
            options: values
                .iter()
                .map(|v| SelectOption {
                    value: (*v).into(),
                    label: (*v).into(),
                })
                .collect(),
        }
    }

    /// Shorthand for an unbounded [`FieldType::Number`].
    pub fn number() -> Self {
        Self::Number {
            min: None,
            max: None,
        }
    }
}
