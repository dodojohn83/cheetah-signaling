//! Custom rustls server certificate verifier that, after chain validation,
//! insists the end-entity certificate identifies the expected plugin instance.
//!
//! The plugin's server certificate must contain a Subject Alternative Name
//! (`SubjectAltName`) `URI` entry equal to either the expected plugin name or
//! `plugin:<expected_plugin_name>`. This binds the TLS peer identity to the
//! specific plugin/node being launched, satisfying AGENTS.md section 11.

use rustls::{
    RootCertStore,
    client::{WebPkiServerVerifier, danger::ServerCertVerifier},
    pki_types::{CertificateDer, pem::PemObject},
};
use std::sync::Arc;
use x509_parser::prelude::*;

use cheetah_plugin_sdk::PluginError;

/// Builds a verifier that trusts `ca_pem` and requires the server's leaf
/// certificate to identify itself as `expected_plugin_name`.
pub fn build_plugin_identity_verifier(
    ca_pem: &[u8],
    expected_plugin_name: &str,
) -> Result<Arc<dyn ServerCertVerifier>, PluginError> {
    let mut roots = RootCertStore::empty();
    let mut added = false;
    for cert in CertificateDer::pem_slice_iter(ca_pem) {
        let cert =
            cert.map_err(|e| PluginError::Driver(format!("invalid CA certificate PEM: {e}")))?;
        roots.add(cert).map_err(|e| {
            PluginError::Driver(format!("failed to add CA certificate to trust store: {e}"))
        })?;
        added = true;
    }
    if !added {
        return Err(PluginError::Driver(
            "no CA certificates found in PEM".to_string(),
        ));
    }

    let inner = WebPkiServerVerifier::builder(Arc::new(roots))
        .build()
        .map_err(|e| PluginError::Driver(format!("failed to build server verifier: {e}")))?;

    Ok(Arc::new(PluginIdentityVerifier {
        inner,
        expected_plugin_name: expected_plugin_name.to_string(),
    }))
}

#[derive(Debug)]
struct PluginIdentityVerifier {
    inner: Arc<WebPkiServerVerifier>,
    expected_plugin_name: String,
}

impl ServerCertVerifier for PluginIdentityVerifier {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        intermediates: &[CertificateDer<'_>],
        server_name: &rustls::pki_types::ServerName<'_>,
        ocsp_response: &[u8],
        now: rustls::pki_types::UnixTime,
    ) -> Result<rustls::client::danger::ServerCertVerified, rustls::Error> {
        self.inner.verify_server_cert(
            end_entity,
            intermediates,
            server_name,
            ocsp_response,
            now,
        )?;

        if verify_plugin_identity(end_entity, &self.expected_plugin_name) {
            Ok(rustls::client::danger::ServerCertVerified::assertion())
        } else {
            Err(rustls::Error::InvalidCertificate(
                rustls::CertificateError::NotValidForName,
            ))
        }
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        self.inner.verify_tls12_signature(message, cert, dss)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        self.inner.verify_tls13_signature(message, cert, dss)
    }

    fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
        self.inner.supported_verify_schemes()
    }
}

fn verify_plugin_identity(end_entity: &CertificateDer<'_>, expected_plugin_name: &str) -> bool {
    let der = end_entity.as_ref();
    let (rem, cert) = match X509Certificate::from_der(der) {
        Ok(v) => v,
        Err(_) => return false,
    };
    if !rem.is_empty() {
        return false;
    }

    let san = match cert.subject_alternative_name() {
        Ok(Some(san)) => san,
        _ => return false,
    };

    let expected_uri = format!("plugin:{expected_plugin_name}");
    for name in &san.value.general_names {
        if let x509_parser::extensions::GeneralName::URI(uri) = name
            && (*uri == expected_plugin_name || *uri == expected_uri.as_str())
        {
            return true;
        }
    }

    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use rcgen::{CertificateParams, KeyPair, SanType, string::Ia5String};

    type TestResult = Result<(), Box<dyn std::error::Error>>;

    fn cert_with_sans(
        sans: Vec<SanType>,
    ) -> Result<rcgen::Certificate, Box<dyn std::error::Error>> {
        let mut params = CertificateParams::default();
        params.subject_alt_names = sans;
        let key_pair = KeyPair::generate()?;
        Ok(params.self_signed(&key_pair)?)
    }

    #[test]
    fn accepts_plugin_uri_identity() -> TestResult {
        let uri: Ia5String = "plugin:cheetah/fake".try_into()?;
        let cert = cert_with_sans(vec![SanType::URI(uri)])?;
        assert!(verify_plugin_identity(cert.der(), "cheetah/fake"));
        Ok(())
    }

    #[test]
    fn accepts_exact_uri_identity() -> TestResult {
        let uri: Ia5String = "cheetah/fake".try_into()?;
        let cert = cert_with_sans(vec![SanType::URI(uri)])?;
        assert!(verify_plugin_identity(cert.der(), "cheetah/fake"));
        Ok(())
    }

    #[test]
    fn rejects_dns_only_identity() -> TestResult {
        let dns: Ia5String = "localhost".try_into()?;
        let cert = cert_with_sans(vec![SanType::DnsName(dns)])?;
        assert!(!verify_plugin_identity(cert.der(), "cheetah/fake"));
        Ok(())
    }
}
