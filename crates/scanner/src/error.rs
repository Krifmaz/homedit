//! Error type shared by every part of the scanner.

use std::net::IpAddr;

/// Everything that can make a scan, a target or a Docker query fail.
///
/// Individual closed or filtered ports are *not* errors: they are simply left
/// out of the [`ScanReport`](crate::ScanReport). An error means the scan as a
/// whole was refused or could not run.
#[derive(Debug, thiserror::Error)]
pub enum ScanError {
    /// A target string is neither a CIDR range, an IP address nor a valid
    /// hostname.
    #[error("invalid target {input:?}: {reason}")]
    InvalidTarget {
        /// The string the admin typed.
        input: String,
        /// Why it was rejected.
        reason: String,
    },

    /// A port list such as `"80,443,8000-8100"` could not be parsed.
    #[error("invalid port list {input:?}: {reason}")]
    InvalidPorts {
        /// The string that was parsed.
        input: String,
        /// Why it was rejected.
        reason: String,
    },

    /// A hostname did not resolve to any address.
    #[error("could not resolve {host:?}: {reason}")]
    Resolve {
        /// The hostname that was looked up.
        host: String,
        /// The resolver's error message.
        reason: String,
    },

    /// An address is outside the ranges the [`ScanPolicy`](crate::ScanPolicy)
    /// allows (for example a public address while `allow_public` is off).
    #[error("{ip} is not in an allowed range ({reason})")]
    AddressNotAllowed {
        /// The refused address.
        ip: IpAddr,
        /// Which rule refused it.
        reason: &'static str,
    },

    /// The scan would exceed one of the policy's size limits.
    #[error("scan too large: {0}")]
    TooLarge(String),

    /// The request had no targets or no ports.
    #[error("nothing to scan: {0}")]
    Empty(&'static str),

    /// The scan request or policy is inconsistent.
    #[error("invalid configuration: {0}")]
    Config(String),

    /// Talking to the Docker API failed.
    #[error("docker: {0}")]
    Docker(String),

    /// An I/O error outside of individual port probes.
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

/// Shorthand for `Result<T, ScanError>`.
pub type Result<T, E = ScanError> = std::result::Result<T, E>;
