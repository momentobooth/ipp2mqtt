use std::sync::Arc;

use log::{debug, warn};
use rumqttc::tokio_rustls::rustls::{
    ClientConfig, DigitallySignedStruct, RootCertStore, SignatureScheme,
    client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier},
    crypto::{CryptoProvider, aws_lc_rs},
    pki_types::{CertificateDer, ServerName, UnixTime, pem::PemObject},
};
use snafu::{ResultExt, Snafu};

use crate::config::models::Mqtt;

/// Builds the TLS configuration for the broker connection.
///
/// rustls types are reached through rumqttc's re-export so they can only ever be
/// the version rumqttc itself links against.
pub fn client_config(settings: &Mqtt) -> Result<ClientConfig, TlsError> {
    let provider = Arc::new(aws_lc_rs::default_provider());

    if settings.ignore_tls_errors {
        warn!("MQTT TLS certificate verification is disabled");
        return Ok(ClientConfig::builder()
            .dangerous()
            .with_custom_certificate_verifier(Arc::new(NoopServerCertVerifier { provider }))
            .with_no_client_auth());
    }

    let mut roots = RootCertStore::empty();
    load_system_roots(&mut roots);

    if let Some(path) = &settings.ca_certificate {
        let certificates = CertificateDer::pem_file_iter(path)
            .context(CaFileSnafu { path: path.clone() })?
            .collect::<Result<Vec<_>, _>>()
            .context(CaFileSnafu { path: path.clone() })?;

        let (added, ignored) = roots.add_parsable_certificates(certificates);
        debug!("Loaded {added} certificate(s) from {path}, ignored {ignored}");
    }

    if roots.is_empty() {
        return Err(TlsError::NoRoots);
    }

    Ok(ClientConfig::builder().with_root_certificates(roots).with_no_client_auth())
}

/// Loads the platform certificate store.
///
/// A partial failure is normal on some systems, so the certificates that did
/// load are kept and the rest reported rather than panicking.
fn load_system_roots(roots: &mut RootCertStore) {
    let result = rustls_native_certs::load_native_certs();

    for error in &result.errors {
        warn!("Could not read part of the system certificate store: {error}");
    }

    let (added, ignored) = roots.add_parsable_certificates(result.certs);
    debug!("Loaded {added} system certificate(s), ignored {ignored}");
}

/// Accepts any certificate.
///
/// Only used when `ignoretlserrors` is set, which is the escape hatch for
/// brokers with a self-signed certificate and no CA file to point at.
#[derive(Debug)]
struct NoopServerCertVerifier {
    provider: Arc<CryptoProvider>,
}

impl ServerCertVerifier for NoopServerCertVerifier {
    fn verify_server_cert(
        &self,
        _end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, rumqttc::tokio_rustls::rustls::Error> {
        Ok(ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rumqttc::tokio_rustls::rustls::Error> {
        Ok(HandshakeSignatureValid::assertion())
    }

    fn verify_tls13_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rumqttc::tokio_rustls::rustls::Error> {
        Ok(HandshakeSignatureValid::assertion())
    }

    /// Taken from the active provider rather than hardcoded, so this never
    /// advertises a scheme the crypto backend cannot actually verify.
    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.provider.signature_verification_algorithms.supported_schemes()
    }
}

// ////// //
// Errors //
// ////// //

#[derive(Debug, Snafu)]
pub enum TlsError {
    #[snafu(display("Could not read CA certificate {path}: {source}"))]
    CaFile { path: String, source: rumqttc::tokio_rustls::rustls::pki_types::pem::Error },

    #[snafu(display("No trusted certificates available; set mqtt.cacertificate or mqtt.ignoretlserrors"))]
    NoRoots,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_permissive_verifier_advertises_only_schemes_the_provider_supports() {
        let provider = Arc::new(aws_lc_rs::default_provider());
        let verifier = NoopServerCertVerifier { provider: provider.clone() };
        let schemes = verifier.supported_verify_schemes();

        assert!(!schemes.is_empty());
        // aws-lc-rs has no SHA1 signature verification, so advertising it would
        // promise something the handshake cannot deliver.
        assert!(!schemes.contains(&SignatureScheme::RSA_PKCS1_SHA1));
        assert!(!schemes.contains(&SignatureScheme::ECDSA_SHA1_Legacy));
    }

    #[test]
    fn the_system_store_yields_certificates() {
        let mut roots = RootCertStore::empty();
        load_system_roots(&mut roots);
        assert!(!roots.is_empty(), "expected the platform store to provide roots");
    }
}
