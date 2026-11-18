//! TLS client config that pins the master's self-signed certificate by its
//! SHA-256 fingerprint.
//!
//! We intentionally do NOT use the public CA trust store: the master uses a
//! self-signed certificate generated on first run. Trust is established out of
//! band — the master's cert fingerprint is baked into the agent config at
//! install time. This both avoids PKI setup on a closed LAN and prevents a
//! rogue host from impersonating the master after an IP change.

use std::sync::Arc;

use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::crypto::ring as provider;
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::{ClientConfig, DigitallySignedStruct, SignatureScheme};

#[derive(Debug)]
struct PinnedCertVerifier {
    /// Lowercase hex SHA-256 of the pinned certificate DER.
    fingerprint: String,
}

impl ServerCertVerifier for PinnedCertVerifier {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        let got = fleetview_shared::crypto::cert_fingerprint(end_entity.as_ref());
        if fleetview_shared::crypto::secrets_match(&got, &self.fingerprint) {
            Ok(ServerCertVerified::assertion())
        } else {
            Err(rustls::Error::General(format!(
                "certificate fingerprint mismatch (pinned {}, got {})",
                self.fingerprint, got
            )))
        }
    }

    // Once the leaf certificate is pinned, the handshake signature is produced
    // by that very certificate's key, so we accept it.
    fn verify_tls12_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        Ok(HandshakeSignatureValid::assertion())
    }

    fn verify_tls13_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        Ok(HandshakeSignatureValid::assertion())
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        provider::default_provider()
            .signature_verification_algorithms
            .supported_schemes()
    }
}

pub fn client_config(cert_fingerprint: &str) -> Arc<ClientConfig> {
    let verifier = Arc::new(PinnedCertVerifier {
        fingerprint: cert_fingerprint.to_lowercase(),
    });
    let config = ClientConfig::builder_with_provider(Arc::new(provider::default_provider()))
        .with_safe_default_protocol_versions()
        .expect("default protocol versions")
        .dangerous()
        .with_custom_certificate_verifier(verifier)
        .with_no_client_auth();
    Arc::new(config)
}
