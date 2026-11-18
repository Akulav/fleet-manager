//! Agent configuration: how to find and trust the master.
//!
//! This file is written by the installer (values provided by the admin running
//! the master's "Generate agent installer config" action) and read by the
//! service at startup. It pins the master's identity so a rogue host on the LAN
//! cannot impersonate the master after an IP change.

use serde::{Deserialize, Serialize};

use crate::paths;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentConfig {
    /// Stable per-machine id. Generated on first run if absent.
    #[serde(default)]
    pub agent_id: String,

    /// Base64 Ed25519 public key of the master — used to verify discovery beacons.
    pub master_public_key: String,

    /// SHA-256 hex fingerprint of the master's TLS certificate (pinned).
    pub master_cert_fingerprint: String,

    /// Shared enrollment secret proving this agent is authorised.
    pub enrollment_secret: String,

    /// Optional fixed master address `host:port`. When set, discovery is skipped
    /// and we connect here directly (useful across subnets / static master).
    #[serde(default)]
    pub master_address: Option<String>,

    /// Extra subnet broadcast addresses to probe (e.g. "192.168.10.255:48621")
    /// for multi-subnet LANs where the default broadcast won't reach the master.
    #[serde(default)]
    pub discovery_targets: Vec<String>,

    /// Seconds between telemetry reports.
    #[serde(default = "default_telemetry_interval")]
    pub telemetry_interval_secs: u64,
}

fn default_telemetry_interval() -> u64 {
    30
}

impl AgentConfig {
    pub fn load() -> anyhow::Result<Self> {
        let path = paths::config_path();
        let text = std::fs::read_to_string(&path)
            .map_err(|e| anyhow::anyhow!("cannot read {}: {e}", path.display()))?;
        let mut cfg: AgentConfig = serde_json::from_str(&text)?;
        if cfg.agent_id.trim().is_empty() {
            cfg.agent_id = new_agent_id();
            let _ = cfg.save();
        }
        Ok(cfg)
    }

    pub fn save(&self) -> anyhow::Result<()> {
        paths::ensure_data_dir()?;
        let text = serde_json::to_string_pretty(self)?;
        std::fs::write(paths::config_path(), text)?;
        Ok(())
    }
}

fn new_agent_id() -> String {
    use rand::RngCore;
    let mut b = [0u8; 16];
    rand::rngs::OsRng.fill_bytes(&mut b);
    hex::encode(b)
}
