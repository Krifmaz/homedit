//! Service widgets for homedit.
//!
//! A service widget shows live stats from a self-hosted app (Plex, Sonarr,
//! qBittorrent, Proxmox, ...) on a dashboard card. Each widget is a small
//! module behind the [`Widget`] trait:
//!
//! - [`Widget::schema`] describes its settings, so the editor can generate
//!   the form (types, required fields, help text) and the server knows which
//!   fields are secrets.
//! - [`Widget::fetch`] calls the app's API and returns [`WidgetData`], a
//!   list of stats and detail rows the frontend renders generically.
//! - [`Widget::test_connection`] backs the editor's "test connection"
//!   button.
//!
//! All requests are made by the server through [`WidgetContext`], with
//! timeouts, no redirects and a host allow-list (only the configured URL's
//! host), so API keys never reach the browser and widgets cannot be used to
//! reach arbitrary hosts. [`WidgetRegistry`] holds the catalogue and caches
//! results per dashboard instance.
//!
//! ```no_run
//! # async fn demo() -> Result<(), homedit_widgets::WidgetError> {
//! use homedit_widgets::{WidgetConfig, WidgetContext, WidgetRegistry};
//!
//! let registry = WidgetRegistry::builtin();
//! let ctx = WidgetContext::with_defaults()?;
//! let cfg = WidgetConfig::default()
//!     .with("url", "http://sonarr.lan:8989")
//!     .with("key", "0123456789abcdef");
//! let data = registry.fetch_cached(&ctx, "card-42", "sonarr", cfg).await?;
//! println!("{}", serde_json::to_string(&*data).unwrap());
//! # Ok(()) }
//! ```

#![warn(missing_docs)]
#![forbid(unsafe_code)]

/// This build's version: `HOMEDIT_VERSION` at build time (the release sets it), else Cargo.toml's.
pub const VERSION: &str = match option_env!("HOMEDIT_VERSION") {
    Some(v) => v,
    None => env!("CARGO_PKG_VERSION"),
};

pub mod config;
pub mod context;
pub mod data;
pub mod error;
pub mod registry;
pub mod schema;
pub mod widget;
pub mod widgets;

pub use config::WidgetConfig;
pub use context::{ContextOptions, WidgetContext};
pub use data::{ListItem, Stat, StatValue, TestReport, WidgetData};
pub use error::WidgetError;
pub use registry::WidgetRegistry;
pub use schema::{FieldSpec, FieldType, SelectOption, WidgetSchema};
pub use widget::Widget;
