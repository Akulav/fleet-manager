//! Shared types and helpers used by both the FleetView agent and master.
//!
//! This crate contains the wire protocol, the cryptographic identity /
//! enrollment primitives, and the LAN discovery message format. It is kept
//! free of Windows-specific and async-runtime code so it can be unit-tested
//! on any platform.

pub mod crypto;
pub mod discovery;
pub mod protocol;

/// UDP port agents broadcast on to discover the master.
pub const DISCOVERY_PORT: u16 = 48621;

/// Default TCP port the master listens on for agent connections (TLS).
pub const DEFAULT_CONTROL_PORT: u16 = 48620;

/// Magic string prefixing every discovery datagram so we ignore unrelated
/// broadcast traffic quickly.
pub const DISCOVERY_MAGIC: &[u8] = b"FLEETVIEW/1";

/// Maximum size of a single protocol frame (64 MiB). Large enough to stream
/// installer binaries in chunks, small enough to bound memory per message.
pub const MAX_FRAME_BYTES: usize = 64 * 1024 * 1024;

/// Current protocol version. Bumped on breaking wire changes.
pub const PROTOCOL_VERSION: u32 = 1;
