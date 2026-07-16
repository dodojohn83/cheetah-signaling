//! SDK version negotiation between a plugin and the host.

use crate::error::PluginError;
use crate::manifest::SdkVersionReq;

/// Negotiates a common SDK version.
///
/// Returns the provided host version if it satisfies the plugin's requirement.
/// The host must therefore support every version it advertises.
pub fn negotiate_sdk_version(
    plugin_requirement: &SdkVersionReq,
    host_version: &semver::Version,
) -> Result<semver::Version, PluginError> {
    if plugin_requirement.matches(host_version)? {
        Ok(host_version.clone())
    } else {
        Err(PluginError::IncompatibleSdk {
            plugin: plugin_requirement.to_string(),
            host: host_version.to_string(),
        })
    }
}
