//! LAN discovery message format.
//!
//! Flow:
//!   1. Agent broadcasts a [`DiscoveryQuery`] (UDP, port [`crate::DISCOVERY_PORT`]).
//!   2. Master replies (unicast) with a [`DiscoveryBeacon`] signed by its
//!      Ed25519 identity key.
//!   3. Agent verifies the signature against the pinned master public key and
//!      then connects to `beacon.control_port` over TLS.
//!
//! The master may also periodically *broadcast* an unsolicited beacon so that
//! freshly-booted agents converge quickly without waiting for their own query.

use serde::{Deserialize, Serialize};

/// Sent by an agent to locate the master.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiscoveryQuery {
    pub protocol: u32,
    /// Agent hostname — purely informational, lets the master log who is looking.
    pub hostname: String,
}

/// Signed reply from the master describing where/how to connect.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiscoveryBeacon {
    pub protocol: u32,
    /// TCP port the master's TLS control channel listens on.
    pub control_port: u16,
    /// SHA-256 fingerprint (hex) of the master's TLS certificate, for pinning.
    pub cert_fingerprint: String,
    /// Human-readable master name, shown in agent logs/tray tooltip.
    pub master_name: String,
    /// Unix-millis timestamp, limits replay windows.
    pub issued_at_ms: i64,
    /// Base64 Ed25519 signature over the canonical bytes of the fields above.
    pub signature: String,
}

impl DiscoveryBeacon {
    /// Canonical byte representation that gets signed/verified. Must be stable.
    pub fn signing_bytes(
        protocol: u32,
        control_port: u16,
        cert_fingerprint: &str,
        master_name: &str,
        issued_at_ms: i64,
    ) -> Vec<u8> {
        format!(
            "{protocol}|{control_port}|{cert_fingerprint}|{master_name}|{issued_at_ms}"
        )
        .into_bytes()
    }

    pub fn canonical_bytes(&self) -> Vec<u8> {
        Self::signing_bytes(
            self.protocol,
            self.control_port,
            &self.cert_fingerprint,
            &self.master_name,
            self.issued_at_ms,
        )
    }
}
