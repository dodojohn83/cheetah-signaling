//! Cluster node version and contract compatibility matrix.

use std::collections::HashMap;
use std::fmt;

/// A compatibility matrix that decides whether a node with a given binary
/// version and contract versions is allowed to join the cluster.
#[derive(Clone, Debug)]
pub struct CompatibilityMatrix {
    /// Range of binary versions that are allowed to participate.
    binary_version_range: semver::VersionReq,
    /// Required contract names and the version ranges they must satisfy.
    contract_versions: HashMap<String, semver::VersionReq>,
}

/// Errors returned when a node fails the compatibility check.
#[derive(Clone, Debug, thiserror::Error)]
pub enum CompatibilityError {
    /// The node's binary version could not be parsed.
    #[error("binary version is not a valid semantic version: {0}")]
    InvalidBinaryVersion(String),
    /// The node's binary version is outside the allowed range.
    #[error("binary version {version} is outside the allowed range {range}")]
    BinaryVersionOutOfRange {
        /// Version reported by the node.
        version: String,
        /// Allowed range.
        range: String,
    },
    /// A required contract is missing.
    #[error("required contract {contract} is missing")]
    MissingContract {
        /// Contract name.
        contract: String,
    },
    /// A contract version reported by the node could not be parsed.
    #[error("contract {0} version is not a valid semantic version: {1}")]
    InvalidContractVersion(String, String),
    /// A contract version does not satisfy the required range.
    #[error("contract {contract} version {version} does not satisfy {range}")]
    ContractVersionOutOfRange {
        /// Contract name.
        contract: String,
        /// Version reported by the node.
        version: String,
        /// Allowed range.
        range: String,
    },
}

impl CompatibilityMatrix {
    /// Creates a new compatibility matrix.
    ///
    /// `binary_version_range` is a semver requirement such as `>=0.1.0, <0.3.0`.
    /// `contract_versions` maps contract names to semver requirements.
    pub fn new(
        binary_version_range: &str,
        contract_versions: HashMap<String, String>,
    ) -> Result<Self, CompatibilityError> {
        let binary_version_range = parse_req(binary_version_range).map_err(|e| {
            CompatibilityError::InvalidBinaryVersion(format!(
                "binary version range {binary_version_range:?}: {e}"
            ))
        })?;
        let mut parsed_contracts = HashMap::with_capacity(contract_versions.len());
        for (name, req) in contract_versions {
            let req = parse_req(&req).map_err(|e| {
                CompatibilityError::InvalidContractVersion(
                    name.clone(),
                    format!("range {req:?}: {e}"),
                )
            })?;
            parsed_contracts.insert(name, req);
        }
        Ok(Self {
            binary_version_range,
            contract_versions: parsed_contracts,
        })
    }

    /// A permissive matrix that accepts any binary version and ignores
    /// contract versions. Useful for single-node or test deployments.
    pub fn permissive() -> Self {
        Self {
            binary_version_range: semver::VersionReq::STAR,
            contract_versions: HashMap::new(),
        }
    }

    /// Checks whether the given version and contract versions are compatible.
    pub fn check(
        &self,
        binary_version: &str,
        contract_versions: &HashMap<String, String>,
    ) -> Result<(), CompatibilityError> {
        // A STAR range imposes no binary-version constraint. Skip parsing so that
        // non-semver version strings (e.g., git hashes or custom labels) are still
        // accepted when the matrix is fully permissive.
        if self.binary_version_range != semver::VersionReq::STAR {
            let version = semver::Version::parse(binary_version).map_err(|_| {
                CompatibilityError::InvalidBinaryVersion(binary_version.to_string())
            })?;
            if !self.binary_version_range.matches(&version) {
                return Err(CompatibilityError::BinaryVersionOutOfRange {
                    version: binary_version.to_string(),
                    range: self.binary_version_range.to_string(),
                });
            }
        }

        for (contract, req) in &self.contract_versions {
            let node_version = contract_versions.get(contract).ok_or_else(|| {
                CompatibilityError::MissingContract {
                    contract: contract.clone(),
                }
            })?;
            let node_version = semver::Version::parse(node_version).map_err(|_| {
                CompatibilityError::InvalidContractVersion(contract.clone(), node_version.clone())
            })?;
            if !req.matches(&node_version) {
                return Err(CompatibilityError::ContractVersionOutOfRange {
                    contract: contract.clone(),
                    version: node_version.to_string(),
                    range: req.to_string(),
                });
            }
        }

        Ok(())
    }
}

impl Default for CompatibilityMatrix {
    fn default() -> Self {
        Self::permissive()
    }
}

fn parse_req(s: &str) -> Result<semver::VersionReq, semver::Error> {
    // `VersionReq::parse` accepts caret/tilde/ranges; an exact version is
    // interpreted as a caret requirement, which is usually too loose for our
    // matrix. Normalize bare versions to exact matches.
    if s.chars().all(|c| c.is_ascii_digit() || c == '.') {
        semver::VersionReq::parse(&format!("={s}"))
    } else {
        semver::VersionReq::parse(s)
    }
}

impl fmt::Display for CompatibilityMatrix {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "binary {}, contracts: {:?}",
            self.binary_version_range, self.contract_versions
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn permissive_accepts_any_version() {
        let matrix = CompatibilityMatrix::permissive();
        assert!(matrix.check("v0.1.0-g1234567", &HashMap::new()).is_ok());
    }

    #[test]
    fn rejects_invalid_binary_version() -> Result<(), CompatibilityError> {
        let matrix = CompatibilityMatrix::new(">=1.0.0, <2.0.0", HashMap::new())?;
        assert!(matches!(
            matrix.check("not-a-version", &HashMap::new()),
            Err(CompatibilityError::InvalidBinaryVersion(_))
        ));
        Ok(())
    }

    #[test]
    fn rejects_binary_version_out_of_range() -> Result<(), CompatibilityError> {
        let matrix = CompatibilityMatrix::new(">=1.0.0, <2.0.0", HashMap::new())?;
        assert!(matrix.check("0.9.0", &HashMap::new()).is_err());
        assert!(matrix.check("2.0.0", &HashMap::new()).is_err());
        assert!(matrix.check("1.5.0", &HashMap::new()).is_ok());
        Ok(())
    }

    #[test]
    fn checks_contract_versions() -> Result<(), CompatibilityError> {
        let mut contracts = HashMap::new();
        contracts.insert(
            "cheetah.media.v1".to_string(),
            ">=1.0.0, <2.0.0".to_string(),
        );
        let matrix = CompatibilityMatrix::new(">=1.0.0, <2.0.0", contracts)?;

        let mut node = HashMap::new();
        node.insert("cheetah.media.v1".to_string(), "1.5.0".to_string());
        assert!(matrix.check("1.2.0", &node).is_ok());

        node.insert("cheetah.media.v1".to_string(), "2.0.0".to_string());
        assert!(matrix.check("1.2.0", &node).is_err());
        Ok(())
    }

    #[test]
    fn exact_contract_version_is_exact() -> Result<(), CompatibilityError> {
        let mut contracts = HashMap::new();
        contracts.insert("x".to_string(), "1.0.0".to_string());
        let matrix = CompatibilityMatrix::new(">=0.0.0", contracts)?;

        let mut node = HashMap::new();
        node.insert("x".to_string(), "1.0.0".to_string());
        assert!(matrix.check("1.0.0", &node).is_ok());

        node.insert("x".to_string(), "1.0.1".to_string());
        assert!(matrix.check("1.0.0", &node).is_err());
        Ok(())
    }
}
