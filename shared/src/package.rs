//! Versioning and the agent package format, shared by agent and master.
//!
//! The agent ships as ONE executable that installs, updates and runs itself.
//! Two small conventions make that work without any installer framework:
//!
//! * **Embedded version marker.** The agent binary contains the bytes
//!   `FLEETVIEW_AGENT_VERSION=<x.y.z>\0`. The master reads it to know which
//!   version a file on disk is (without running it), and the agent's updater
//!   reads it to verify that a pushed file really is a FleetView agent.
//! * **Config trailer.** The master's "Save agent setup" appends the agent
//!   config (JSON) to a copy of the agent exe:
//!   `[exe bytes][config json][u32 LE json length][CONFIG_TRAILER_MAGIC]`.
//!   Windows ignores data after the PE image, so the file still runs; on
//!   install the trailer is stripped again (the installed exe is readable by
//!   ordinary users, the config holds the enrollment secret).

use std::fmt;

/// Prefix of the version marker embedded in the agent executable.
pub const AGENT_VERSION_MARKER: &[u8] = b"FLEETVIEW_AGENT_VERSION=";

/// Magic at the very end of an agent exe that carries a config trailer.
pub const CONFIG_TRAILER_MAGIC: &[u8; 8] = b"FVCFG001";

/// Upper bound for an embedded config (a real one is well under 2 KiB).
const MAX_TRAILER_CONFIG: usize = 64 * 1024;

/// Semantic version `major.minor.patch`. Pre-release / build suffixes
/// (`-rc.1`, `+abc`) are accepted when parsing and ignored for ordering.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Version {
    pub major: u32,
    pub minor: u32,
    pub patch: u32,
}

impl Version {
    pub fn parse(s: &str) -> Option<Self> {
        let s = s.trim();
        let s = s.strip_prefix(['v', 'V']).unwrap_or(s);
        let core = s.split(['-', '+']).next()?;
        let mut it = core.split('.');
        let major = it.next()?.parse().ok()?;
        let minor = it.next()?.parse().ok()?;
        let patch = it.next()?.parse().ok()?;
        if it.next().is_some() {
            return None;
        }
        Some(Self {
            major,
            minor,
            patch,
        })
    }
}

impl fmt::Display for Version {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)
    }
}

/// Find the version marker in an agent executable. Returns `None` when the
/// bytes are not a FleetView agent (or one older than the marker, < 0.2.0).
pub fn find_agent_version(exe: &[u8]) -> Option<Version> {
    let m = AGENT_VERSION_MARKER;
    let mut from = 0;
    while let Some(pos) = find(&exe[from..], m) {
        let start = from + pos + m.len();
        // The marker literal itself also appears (as the search needle) in
        // binaries that link this crate; only accept "<version>\0".
        let tail = &exe[start..exe.len().min(start + 32)];
        if let Some(nul) = tail.iter().position(|&b| b == 0) {
            if let Some(v) = std::str::from_utf8(&tail[..nul])
                .ok()
                .and_then(Version::parse)
            {
                return Some(v);
            }
        }
        from = start;
    }
    None
}

fn find(hay: &[u8], needle: &[u8]) -> Option<usize> {
    hay.windows(needle.len()).position(|w| w == needle)
}

/// Split an agent exe into the runnable image and the embedded config (if any).
pub fn split_config_trailer(exe: &[u8]) -> (&[u8], Option<&[u8]>) {
    let magic = CONFIG_TRAILER_MAGIC.len();
    if exe.len() < magic + 4 || &exe[exe.len() - magic..] != CONFIG_TRAILER_MAGIC {
        return (exe, None);
    }
    let len_at = exe.len() - magic - 4;
    let len = u32::from_le_bytes(exe[len_at..len_at + 4].try_into().unwrap()) as usize;
    if len > MAX_TRAILER_CONFIG || len > len_at {
        return (exe, None);
    }
    let cfg_at = len_at - len;
    (&exe[..cfg_at], Some(&exe[cfg_at..len_at]))
}

/// Build a self-provisioning agent setup: the plain agent image plus `config`.
/// Any trailer already present on `exe` is replaced.
pub fn with_config_trailer(exe: &[u8], config: &[u8]) -> anyhow::Result<Vec<u8>> {
    if config.len() > MAX_TRAILER_CONFIG {
        anyhow::bail!("agent config is too large to embed");
    }
    let (image, _) = split_config_trailer(exe);
    let mut out = Vec::with_capacity(image.len() + config.len() + 12);
    out.extend_from_slice(image);
    out.extend_from_slice(config);
    out.extend_from_slice(&(config.len() as u32).to_le_bytes());
    out.extend_from_slice(CONFIG_TRAILER_MAGIC);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_and_order() {
        let v = |s| Version::parse(s).unwrap();
        assert_eq!(v("0.2.0").to_string(), "0.2.0");
        assert_eq!(v("v1.10.3"), v("1.10.3"));
        assert_eq!(v("1.2.3-rc.1"), v("1.2.3"));
        assert!(v("0.10.0") > v("0.9.9"));
        assert!(v("1.0.0") > v("0.99.99"));
        assert!(Version::parse("1.2").is_none());
        assert!(Version::parse("1.2.3.4").is_none());
        assert!(Version::parse("abc").is_none());
    }

    #[test]
    fn finds_marker_but_not_bare_needle() {
        let mut exe = b"MZ....".to_vec();
        exe.extend_from_slice(AGENT_VERSION_MARKER); // bare needle, junk after
        exe.extend_from_slice(b"\x01\x02junk");
        exe.extend_from_slice(AGENT_VERSION_MARKER);
        exe.extend_from_slice(b"0.3.1\0rest");
        assert_eq!(find_agent_version(&exe), Version::parse("0.3.1"));
        assert_eq!(find_agent_version(b"MZ no marker"), None);
    }

    #[test]
    fn trailer_roundtrip() {
        let exe = b"MZ fake agent image".to_vec();
        assert_eq!(split_config_trailer(&exe), (&exe[..], None));
        let cfg = br#"{"enrollment_secret":"x"}"#;
        let setup = with_config_trailer(&exe, cfg).unwrap();
        let (image, got) = split_config_trailer(&setup);
        assert_eq!(image, &exe[..]);
        assert_eq!(got, Some(&cfg[..]));
        // Re-embedding replaces instead of stacking.
        let again = with_config_trailer(&setup, b"{}").unwrap();
        let (image, got) = split_config_trailer(&again);
        assert_eq!(image, &exe[..]);
        assert_eq!(got, Some(&b"{}"[..]));
    }
}
