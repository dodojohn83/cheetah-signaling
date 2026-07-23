//! Cluster node version and contract compatibility matrix.

use std::collections::HashMap;
use std::fmt;

/// Maximum byte length of a binary version requirement string.
const MAX_BINARY_VERSION_RANGE_BYTES: usize = 128;
/// Maximum byte length of a binary version reported by a node.
const MAX_BINARY_VERSION_BYTES: usize = 128;
/// Maximum number of contract versions tracked in the matrix.
const MAX_CONTRACT_VERSIONS: usize = 64;
/// Maximum byte length of a contract name.
const MAX_CONTRACT_NAME_BYTES: usize = 128;
/// Maximum byte length of a contract version requirement string.
const MAX_CONTRACT_VERSION_REQ_BYTES: usize = 128;
/// Maximum byte length of a contract version string reported by a node.
const MAX_CONTRACT_VERSION_STRING_BYTES: usize = 128;

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
    /// An input exceeded the allowed length or count.
    #[error("{0}")]
    InvalidArgument(String),
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
        if binary_version_range.len() > MAX_BINARY_VERSION_RANGE_BYTES {
            return Err(CompatibilityError::InvalidArgument(format!(
                "binary version range must not exceed {MAX_BINARY_VERSION_RANGE_BYTES} bytes"
            )));
        }
        if contract_versions.len() > MAX_CONTRACT_VERSIONS {
            return Err(CompatibilityError::InvalidArgument(format!(
                "contract_versions must not exceed {MAX_CONTRACT_VERSIONS} entries"
            )));
        }
        let binary_version_range = parse_req(binary_version_range).map_err(|e| {
            CompatibilityError::InvalidBinaryVersion(format!(
                "binary version range {binary_version_range:?}: {e}"
            ))
        })?;
        let mut parsed_contracts = HashMap::with_capacity(contract_versions.len());
        for (name, req) in contract_versions {
            if name.len() > MAX_CONTRACT_NAME_BYTES {
                return Err(CompatibilityError::InvalidArgument(format!(
                    "contract name must not exceed {MAX_CONTRACT_NAME_BYTES} bytes"
                )));
            }
            if req.len() > MAX_CONTRACT_VERSION_REQ_BYTES {
                return Err(CompatibilityError::InvalidArgument(format!(
                    "contract {name:?} version requirement must not exceed {MAX_CONTRACT_VERSION_REQ_BYTES} bytes"
                )));
            }
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
        if binary_version.len() > MAX_BINARY_VERSION_BYTES {
            return Err(CompatibilityError::InvalidArgument(format!(
                "binary version must not exceed {MAX_BINARY_VERSION_BYTES} bytes"
            )));
        }
        if contract_versions.len() > MAX_CONTRACT_VERSIONS {
            return Err(CompatibilityError::InvalidArgument(format!(
                "contract_versions must not exceed {MAX_CONTRACT_VERSIONS} entries"
            )));
        }

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
            if node_version.len() > MAX_CONTRACT_VERSION_STRING_BYTES {
                return Err(CompatibilityError::InvalidArgument(format!(
                    "contract {contract:?} version must not exceed {MAX_CONTRACT_VERSION_STRING_BYTES} bytes"
                )));
            }
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
        let normalized = format!("={s}");
        semver::VersionReq::parse(&normalized)
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

    #[test]
    fn new_rejects_oversized_binary_version_range() {
        let range = ">=".repeat(65);
        assert!(matches!(
            CompatibilityMatrix::new(&range, HashMap::new()),
            Err(CompatibilityError::InvalidArgument(_))
        ));
    }

    #[test]
    fn new_rejects_too_many_contracts() {
        let contracts = (0..65)
            .map(|i| (format!("contract-{i}"), ">=1.0.0".to_string()))
            .collect();
        assert!(matches!(
            CompatibilityMatrix::new(">=1.0.0", contracts),
            Err(CompatibilityError::InvalidArgument(_))
        ));
    }

    #[test]
    fn new_rejects_oversized_contract_name() {
        let contracts = HashMap::from([("x".repeat(129), ">=1.0.0".to_string())]);
        assert!(matches!(
            CompatibilityMatrix::new(">=1.0.0", contracts),
            Err(CompatibilityError::InvalidArgument(_))
        ));
    }

    #[test]
    fn new_rejects_oversized_contract_req() {
        let contracts = HashMap::from([("cheetah.media.v1".to_string(), ">=".repeat(65))]);
        assert!(matches!(
            CompatibilityMatrix::new(">=1.0.0", contracts),
            Err(CompatibilityError::InvalidArgument(_))
        ));
    }

    #[test]
    fn check_rejects_oversized_binary_version() -> Result<(), CompatibilityError> {
        let matrix = CompatibilityMatrix::new(">=1.0.0, <2.0.0", HashMap::new())?;
        let version = "1.".repeat(65);
        assert!(matches!(
            matrix.check(&version, &HashMap::new()),
            Err(CompatibilityError::InvalidArgument(_))
        ));
        Ok(())
    }

    #[test]
    fn check_rejects_too_many_contract_versions() -> Result<(), CompatibilityError> {
        let matrix = CompatibilityMatrix::new(">=1.0.0, <2.0.0", HashMap::new())?;
        let node = (0..65)
            .map(|i| (format!("contract-{i}"), "1.0.0".to_string()))
            .collect();
        assert!(matches!(
            matrix.check("1.2.0", &node),
            Err(CompatibilityError::InvalidArgument(_))
        ));
        Ok(())
    }

    #[test]
    fn check_rejects_oversized_node_contract_version() -> Result<(), CompatibilityError> {
        let mut contracts = HashMap::new();
        contracts.insert(
            "cheetah.media.v1".to_string(),
            ">=1.0.0, <2.0.0".to_string(),
        );
        let matrix = CompatibilityMatrix::new(">=1.0.0, <2.0.0", contracts)?;
        let node = HashMap::from([("cheetah.media.v1".to_string(), "x".repeat(129))]);
        assert!(matches!(
            matrix.check("1.2.0", &node),
            Err(CompatibilityError::InvalidArgument(_))
        ));
        Ok(())
    }
}
