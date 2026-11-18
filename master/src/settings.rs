//! Master identity + TLS material + enrollment secret.
//!
//! Generated on first run and persisted. The agent installer is provisioned
//! with the public half of all of this (see [`MasterSettings::agent_config_json`]).

use serde::{Deserialize, Serialize};

use fleetview_shared::crypto::MasterIdentity;
use fleetview_shared::DEFAULT_CONTROL_PORT;

use crate::paths;

#[derive(Clone, Serialize, Deserialize)]
pub struct MasterSettings {
    pub identity: MasterIdentity,
    pub cert_pem: String,
    pub key_pem: String,
    pub cert_fingerprint: String,
    pub enrollment_secret: String,
    pub master_name: String,
    pub control_port: u16,
}

impl MasterSettings {
    pub fn load_or_create() -> anyhow::Result<Self> {
        let path = paths::settings_path();
        if let Ok(text) = std::fs::read_to_string(&path) {
            if let Ok(s) = serde_json::from_str::<MasterSettings>(&text) {
                return Ok(s);
            }
        }
        let s = Self::generate()?;
        s.save()?;
        Ok(s)
    }

    fn generate() -> anyhow::Result<Self> {
        let identity = MasterIdentity::generate();

        // Self-signed TLS certificate for the control channel.
        let certified =
            rcgen::generate_simple_self_signed(vec!["fleetview-master".to_string()])?;
        let cert_pem = certified.cert.pem();
        let key_pem = certified.key_pair.serialize_pem();
        let fingerprint =
            fleetview_shared::crypto::cert_fingerprint(certified.cert.der().as_ref());

        let host = std::env::var("COMPUTERNAME").unwrap_or_else(|_| "FleetView".into());

        Ok(Self {
            identity,
            cert_pem,
            key_pem,
            cert_fingerprint: fingerprint,
            enrollment_secret: fleetview_shared::crypto::random_secret(),
            master_name: format!("FleetView @ {host}"),
            control_port: DEFAULT_CONTROL_PORT,
        })
    }

    pub fn save(&self) -> anyhow::Result<()> {
        paths::ensure_data_dir()?;
        std::fs::write(paths::settings_path(), serde_json::to_string_pretty(self)?)?;
        Ok(())
    }

    /// The JSON an agent needs (`agent.config.json`). The installer bundles this.
    pub fn agent_config_json(&self, master_address: Option<String>) -> anyhow::Result<String> {
        let value = serde_json::json!({
            "agent_id": "",
            "master_public_key": self.identity.public_key_b64()?,
            "master_cert_fingerprint": self.cert_fingerprint,
            "enrollment_secret": self.enrollment_secret,
            "master_address": master_address,
            "discovery_targets": [],
            "telemetry_interval_secs": 30
        });
        Ok(serde_json::to_string_pretty(&value)?)
    }
}
