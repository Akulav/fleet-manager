//! TLS client config that pins the master's self-signed certificate by its
//! SHA-256 fingerprint.
//!
//! We intentionally do NOT use the public CA trust store: the master uses a
//! self-signed certificate generated on first run. Trust is established out of
//! band — the master's cert fingerprint is baked into the agent config at
//! install time. This both avoids PKI setup on a closed LAN and prevents a
//! rogue host from impersonating the master after an IP change.
//!
//! IMPORTANT: pinning the certificate alone is not enough. The certificate is
//! public (the master sends it in every handshake), so anyone could replay it.
//! What proves possession of the matching private key is the handshake
//! signature, which we therefore verify with the crypto provider below.

use std::sync::Arc;

use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::crypto::{ring as provider, WebPkiSupportedAlgorithms};
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::{ClientConfig, DigitallySignedStruct, SignatureScheme};

#[derive(Debug)]
struct PinnedCertVerifier {
    /// Lowercase hex SHA-256 of the pinned certificate DER.
    fingerprint: String,
    algs: WebPkiSupportedAlgorithms,
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

    // The handshake signature MUST be verified against the (pinned) leaf
    // certificate's public key; otherwise a host that merely replays the
    // public certificate would be accepted as the master.
    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls12_signature(message, cert, dss, &self.algs)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(message, cert, dss, &self.algs)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.algs.supported_schemes()
    }
}

pub fn client_config(cert_fingerprint: &str) -> Arc<ClientConfig> {
    let provider = Arc::new(provider::default_provider());
    let verifier = Arc::new(PinnedCertVerifier {
        fingerprint: cert_fingerprint.trim().to_lowercase(),
        algs: provider.signature_verification_algorithms,
    });
    let config = ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .expect("default protocol versions")
        .dangerous()
        .with_custom_certificate_verifier(verifier)
        .with_no_client_auth();
    Arc::new(config)
}
