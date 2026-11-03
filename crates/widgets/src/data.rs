//! The data a widget returns for display.

use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

/// What a widget shows: a row of stats and an optional list of items.
///
/// The frontend renders this generically, so adding a widget never needs
/// frontend changes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WidgetData {
    /// Headline numbers, in display order.
    pub stats: Vec<Stat>,
    /// Detail rows such as active streams or download queue entries.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub items: Vec<ListItem>,
    /// When the data was fetched, in milliseconds since the Unix epoch.
    pub fetched_at: u64,
}

impl WidgetData {
    /// Creates data stamped with the current time.
    pub fn new(stats: Vec<Stat>) -> Self {
        Self {
            stats,
            items: Vec::new(),
            fetched_at: now_ms(),
        }
    }

    /// Attaches list items.
    #[must_use]
    pub fn with_items(mut self, items: Vec<ListItem>) -> Self {
        self.items = items;
        self
    }

    /// Looks up a stat by key.
    pub fn stat(&self, key: &str) -> Option<&Stat> {
        self.stats.iter().find(|s| s.key == key)
    }
}

/// One labelled value.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Stat {
    /// Stable key, e.g. `queued`.
    pub key: String,
    /// Display label, e.g. `Queued`.
    pub label: String,
    /// The value with its unit.
    pub value: StatValue,
}

impl Stat {
    /// Creates a stat.
    pub fn new(key: &str, label: &str, value: StatValue) -> Self {
        Self {
            key: key.into(),
            label: label.into(),
            value,
        }
    }

    /// Creates a plain count.
    pub fn count(key: &str, label: &str, n: u64) -> Self {
        Self::new(key, label, StatValue::Number(n as f64))
    }
}

/// A value and how to format it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", content = "value", rename_all = "snake_case")]
pub enum StatValue {
    /// A plain number.
    Number(f64),
    /// Free text, e.g. a status word.
    Text(String),
    /// A size in bytes.
    Bytes(f64),
    /// A transfer rate in bytes per second.
    Rate(f64),
    /// A percentage from 0 to 100.
    Percent(f64),
    /// A duration in seconds.
    Duration(f64),
    /// A duration in milliseconds (e.g. latency).
    Millis(f64),
}

/// A detail row.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ListItem {
    /// Main text, e.g. the title being played.
    pub title: String,
    /// Secondary text, e.g. the user or the state.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subtitle: Option<String>,
    /// Progress from 0 to 100, if meaningful.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub progress: Option<f64>,
}

impl ListItem {
    /// Creates an item with only a title.
    pub fn new(title: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            subtitle: None,
            progress: None,
        }
    }

    /// Sets the subtitle.
    #[must_use]
    pub fn subtitle(mut self, subtitle: impl Into<String>) -> Self {
        self.subtitle = Some(subtitle.into());
        self
    }

    /// Sets the progress, clamped to 0..=100.
    #[must_use]
    pub fn progress(mut self, progress: f64) -> Self {
        self.progress = Some(progress.clamp(0.0, 100.0));
        self
    }
}

/// Result of a "test connection" button press.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TestReport {
    /// Whether the widget could fetch its data.
    pub ok: bool,
    /// A short message for the admin.
    pub message: String,
    /// Round trip time of the test in milliseconds.
    pub latency_ms: u64,
}

pub(crate) fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or_default()
}
