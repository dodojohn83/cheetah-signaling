//! Contract version and rolling-upgrade window for `cheetah.media.v1`.
//!
//! These constants are the source of truth for the minimum and maximum contract
//! version that this signaling build supports, and the grace window during which
//! media nodes running the previous contract version may remain registered.

/// Minimum `cheetah.media.v1` contract version supported by this build.
pub const MINIMUM_SUPPORTED_CONTRACT_VERSION: u64 = 1;

/// Maximum `cheetah.media.v1` contract version supported by this build.
///
/// Backward-compatible v1 extensions may be added without bumping this value.
/// Breaking changes require a new major contract version.
pub const MAXIMUM_SUPPORTED_CONTRACT_VERSION: u64 = 1;

/// Grace period, in seconds, for media nodes running a contract version older
/// than the latest supported version during a rolling upgrade.
pub const ROLLING_UPGRADE_WINDOW_SECONDS: u64 = 24 * 60 * 60;
