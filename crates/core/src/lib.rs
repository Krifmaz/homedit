//! Shared data model for homedit.
//!
//! This crate holds the types every other crate agrees on: the [`Dashboard`]
//! document (settings, service groups, bookmarks, info widgets, Docker hosts),
//! how a service's status check is derived ([`StatusCheck`]), how icon names
//! resolve to URLs ([`icons`]), and how secrets are masked before data leaves
//! the server ([`secrets`]).
//!
//! It has no I/O and no async code, so it can be used from the database
//! layer, the HTTP server, the YAML importer and tests alike.

pub mod icons;
pub mod model;
pub mod secrets;
pub mod status;

pub use model::*;
pub use status::{StatusCheck, StatusSample};
