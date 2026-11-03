//! Request identity, provided by `homedit-auth`.
//!
//! The auth middleware runs on every request and stores a
//! [`CurrentUser`] in the request; these extractors read it. With login
//! turned off (the default, like gethomepage) every visitor is an admin and
//! a warning is logged at startup.

pub use homedit_auth::{CurrentUser, RequireAdmin, RequireViewer, Role};
