//! # homedit scanner
//!
//! Finds the self-hosted apps on your own network so they can be added to the
//! dashboard in one click.
//!
//! * **Targets**: CIDR ranges, single addresses and hostnames ([`Target`]).
//! * **Safety**: only private, link-local and loopback addresses unless the
//!   admin allows more ([`ScanPolicy`]); plain TCP connect checks with no raw
//!   sockets or extra privileges ([`connect`]); a concurrency cap and an
//!   optional per-second rate limit ([`RateLimiter`]); size limits per scan; an audit record
//!   for every scan on the [`AUDIT_TARGET`] tracing target.
//! * **Ports**: 80, 443, 3000–9999 and every catalogue port by default, the
//!   catalogue ports only, an explicit list, or the full range on request
//!   ([`PortSet`]).
//! * **Fingerprints**: page title, favicon hash, `Server` header, redirects
//!   and known unauthenticated API paths, matched against a built-in
//!   [`Catalogue`] of 44 apps ([`fingerprint`]). Unknown web apps are still
//!   reported with their title and favicon.
//! * **Live hosts first**: addresses from a range are checked with a quick
//!   connect to a few common ports, reverse DNS and the ARP table, and only
//!   hosts that answer are port-scanned. Open ports are identified as soon
//!   as they are found.
//! * **Docker**: published ports read from the Docker API, so apps on a known
//!   Docker host appear without scanning ([`docker`]), and hosts on Docker
//!   networks are named after their container.
//! * **Default targets**: [`local_networks`] lists this machine's private
//!   networks (each interface's /24) and its own addresses.
//! * **Rescans**: [`diff()`] compares two reports and words the changes
//!   ("Jellyfin found on 10.0.1.7:8096").
//!
//! This crate has no database or web framework dependency: the app stores
//! [`ScanReport`]s, enforces admin-only access and exposes the routes.
//!
//! ## Example
//!
//! ```no_run
//! use scanner::{PortSet, ScanPolicy, ScanRequest, Scanner};
//!
//! # async fn run() -> Result<(), scanner::ScanError> {
//! let scanner = Scanner::new(ScanPolicy::default())?;
//! let mut req = ScanRequest::new(vec!["192.168.1.0/24".parse()?, "nas.lan".parse()?]);
//! req.ports = PortSet::Common;
//! req.requested_by = Some("admin".into());
//!
//! let mut report = scanner.scan(req, None).await?;
//! report.mark_on_dashboard(["http://192.168.1.20:8096"]);
//! for (host, service) in report.services() {
//!     let s = service.suggestion(&host.host_label());
//!     println!("{} at {} (on dashboard: {})", s.name, s.href, service.on_dashboard);
//! }
//! # Ok(()) }
//! ```

#![warn(missing_docs)]

/// This build's version: `HOMEDIT_VERSION` at build time (the release sets it), else Cargo.toml's.
pub const VERSION: &str = match option_env!("HOMEDIT_VERSION") {
    Some(v) => v,
    None => env!("CARGO_PKG_VERSION"),
};

pub mod catalogue;
pub mod connect;
pub mod diff;
pub mod docker;
mod error;
pub mod fingerprint;
pub mod net;
pub mod policy;
pub mod ports;
pub mod rate;
pub mod report;
mod scanner;
pub mod target;

pub use catalogue::{AppDef, Catalogue};
pub use diff::{ScanDiff, ServiceChange, diff};
pub use docker::{ContainerAddress, DockerEndpoint, DockerInventory, PublishedPort};
pub use error::{Result, ScanError};
pub use fingerprint::{AppMatch, Favicon, HttpInfo, Scheme};
pub use net::{LocalNetwork, Network, SystemNetwork, local_networks};
pub use policy::{AddressClass, ScanPolicy, classify};
pub use ports::{PortList, PortSet};
pub use rate::RateLimiter;
pub use report::{
    ContainerRef, DiscoveredService, HostResult, ScanReport, ServiceSuggestion, Source,
};
pub use scanner::{AUDIT_TARGET, DISCOVERY_PORTS, ScanEvent, ScanPhase, ScanRequest, Scanner};
pub use target::{ResolvedHost, Target};
