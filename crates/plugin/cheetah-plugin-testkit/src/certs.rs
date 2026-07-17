//! Self-signed test certificate generation for plugin integration tests.
//!
//! The generated CA issues a server certificate that includes the plugin
//! identity as a `URI` subject alternative name (`plugin:<plugin_name>`) so it
//! satisfies the host's out-of-process mTLS verifier.

use rcgen::string::Ia5String;
use rcgen::{
    BasicConstraints, CertificateParams, DistinguishedName, DnType, IsCa, Issuer, KeyPair,
    KeyUsagePurpose, SanType,
};
use std::path::{Path, PathBuf};

/// PEM-encoded certificates and keys used to stand up a fake plugin server.
#[derive(Clone, Debug)]
pub struct TestCerts {
    /// PEM-encoded CA certificate.
    pub ca_pem: String,
    /// PEM-encoded server certificate signed by the CA.
    pub server_cert_pem: String,
    /// PEM-encoded server private key.
    pub server_key_pem: String,
    /// PEM-encoded client certificate signed by the CA.
    pub client_cert_pem: String,
    /// PEM-encoded client private key.
    pub client_key_pem: String,
}

/// File paths for certificates written into a temporary directory.
#[derive(Clone, Debug)]
pub struct CertPaths {
    /// Path to the PEM-encoded CA certificate.
    pub ca_path: PathBuf,
    /// Path to the PEM-encoded server certificate.
    pub server_cert_path: PathBuf,
    /// Path to the PEM-encoded server private key.
    pub server_key_path: PathBuf,
    /// Path to the PEM-encoded client certificate.
    pub client_cert_path: PathBuf,
    /// Path to the PEM-encoded client private key.
    pub client_key_path: PathBuf,
}

impl TestCerts {
    /// Generate a CA and a server/client key pair for `plugin_name`.
    pub fn generate(plugin_name: &str) -> Result<Self, rcgen::Error> {
        let ca_key_pair = KeyPair::generate()?;
        let mut ca_params = CertificateParams::new(vec![])?;
        ca_params.distinguished_name = DistinguishedName::new();
        ca_params
            .distinguished_name
            .push(DnType::CommonName, "Cheetah Test CA");
        ca_params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
        ca_params.key_usages = vec![KeyUsagePurpose::KeyCertSign, KeyUsagePurpose::CrlSign];

        let ca_cert = ca_params.self_signed(&ca_key_pair)?;
        let ca_issuer = Issuer::from_params(&ca_params, ca_key_pair);

        let server_key_pair = KeyPair::generate()?;
        let mut server_params = CertificateParams::new(vec!["localhost".to_string()])?;
        server_params.distinguished_name = DistinguishedName::new();
        server_params
            .distinguished_name
            .push(DnType::CommonName, "Cheetah Test Plugin");

        let plugin_uri: Ia5String = format!("plugin:{plugin_name}").try_into()?;
        server_params
            .subject_alt_names
            .push(SanType::URI(plugin_uri));

        let server_cert = server_params.signed_by(&server_key_pair, &ca_issuer)?;

        let client_key_pair = KeyPair::generate()?;
        let mut client_params = CertificateParams::new(vec![])?;
        client_params.distinguished_name = DistinguishedName::new();
        client_params
            .distinguished_name
            .push(DnType::CommonName, "Cheetah Test Client");
        let client_cert = client_params.signed_by(&client_key_pair, &ca_issuer)?;

        Ok(Self {
            ca_pem: ca_cert.pem(),
            server_cert_pem: server_cert.pem(),
            server_key_pem: server_key_pair.serialize_pem(),
            client_cert_pem: client_cert.pem(),
            client_key_pem: client_key_pair.serialize_pem(),
        })
    }

    /// Write all PEM files into `dir` and return their paths.
    pub fn write_to_dir(&self, dir: &Path) -> Result<CertPaths, std::io::Error> {
        std::fs::create_dir_all(dir)?;
        let paths = CertPaths {
            ca_path: dir.join("ca.pem"),
            server_cert_path: dir.join("server.pem"),
            server_key_path: dir.join("server.key.pem"),
            client_cert_path: dir.join("client.pem"),
            client_key_path: dir.join("client.key.pem"),
        };
        std::fs::write(&paths.ca_path, &self.ca_pem)?;
        std::fs::write(&paths.server_cert_path, &self.server_cert_pem)?;
        std::fs::write(&paths.server_key_path, &self.server_key_pem)?;
        std::fs::write(&paths.client_cert_path, &self.client_cert_pem)?;
        std::fs::write(&paths.client_key_path, &self.client_key_pem)?;
        Ok(paths)
    }
}
