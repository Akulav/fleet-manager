//! Installer type detection, so each pushed file gets the right silent
//! switches instead of one `/S` for everything.
//!
//! A wrong switch is the usual reason a "silent" install fails: the installer
//! opens its wizard (or an "invalid argument" box) on the hidden session-0
//! desktop of the service, where nobody can click it, and the run hangs until
//! the agent's timeout. Detection reads the start of the file and looks for
//! the markers each installer framework leaves in its stub.

use std::io::Read;
use std::path::Path;

/// How much of an installer is scanned. The framework markers live in the
/// stub / version resources near the start of the file.
const SCAN_BYTES: u64 = 8 * 1024 * 1024;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum InstallerKind {
    Msi,
    Inno,
    Nsis,
    WixBurn,
    InstallShield,
    AdvancedInstaller,
    Squirrel,
    Unknown,
}

impl InstallerKind {
    pub fn label(self) -> &'static str {
        match self {
            InstallerKind::Msi => "MSI",
            InstallerKind::Inno => "Inno Setup",
            InstallerKind::Nsis => "NSIS",
            InstallerKind::WixBurn => "WiX bundle",
            InstallerKind::InstallShield => "InstallShield",
            InstallerKind::AdvancedInstaller => "Advanced Installer",
            InstallerKind::Squirrel => "Squirrel (per-user)",
            InstallerKind::Unknown => "unknown type",
        }
    }

    /// Silent arguments for this installer type.
    pub fn silent_args(self) -> &'static str {
        match self {
            InstallerKind::Msi => "/qn /norestart",
            InstallerKind::Inno => "/VERYSILENT /SUPPRESSMSGBOXES /NORESTART /SP-",
            InstallerKind::Nsis => "/S",
            InstallerKind::WixBurn => "/quiet /norestart",
            // `/v/qn` (no space, no quotes) passes /qn to the inner MSI.
            InstallerKind::InstallShield => "/s /v/qn",
            InstallerKind::AdvancedInstaller => "/exenoui /qn /norestart",
            InstallerKind::Squirrel => "--silent",
            InstallerKind::Unknown => "/S",
        }
    }

    /// A warning to show next to the file, if any.
    pub fn warning(self) -> Option<&'static str> {
        match self {
            InstallerKind::Unknown => Some(
                "Unknown installer type — check the vendor's silent switch (try /? on the exe). \
                 A wrong switch makes the install hang until the 60-minute timeout.",
            ),
            InstallerKind::Squirrel => Some(
                "Squirrel installers install per user; run as SYSTEM they land in the system \
                 profile. Look for the vendor's MSI / machine-wide installer instead.",
            ),
            _ => None,
        }
    }
}

/// Detect the installer type of a file.
pub fn detect(path: &Path) -> InstallerKind {
    if path
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("msi"))
    {
        return InstallerKind::Msi;
    }
    let mut buf = Vec::new();
    match std::fs::File::open(path) {
        Ok(f) => {
            let _ = f.take(SCAN_BYTES).read_to_end(&mut buf);
        }
        Err(_) => return InstallerKind::Unknown,
    }
    detect_bytes(&buf)
}

pub fn detect_bytes(b: &[u8]) -> InstallerKind {
    // Order matters: a WiX bundle or Advanced Installer exe may embed strings
    // of the frameworks it wraps, so the most specific markers go first.
    if contains(b, b".wixburn") {
        InstallerKind::WixBurn
    } else if contains(b, b"Inno Setup") {
        InstallerKind::Inno
    } else if contains(b, b"Nullsoft") || contains(b, b"NullsoftInst") {
        InstallerKind::Nsis
    } else if contains(b, b"Advanced Installer") || contains(b, &utf16(b"Advanced Installer")) {
        InstallerKind::AdvancedInstaller
    } else if contains(b, b"InstallShield") || contains(b, &utf16(b"InstallShield")) {
        InstallerKind::InstallShield
    } else if contains(b, b"Squirrel") && contains(b, b"Update.exe") {
        InstallerKind::Squirrel
    } else if contains(b, &utf16(b"Inno Setup")) {
        InstallerKind::Inno
    } else {
        InstallerKind::Unknown
    }
}

fn utf16(ascii: &[u8]) -> Vec<u8> {
    ascii.iter().flat_map(|&c| [c, 0]).collect()
}

fn contains(hay: &[u8], needle: &[u8]) -> bool {
    if needle.is_empty() || hay.len() < needle.len() {
        return false;
    }
    let first = needle[0];
    let last = hay.len() - needle.len();
    let mut i = 0;
    while i <= last {
        match hay[i..=last].iter().position(|&c| c == first) {
            None => return false,
            Some(p) => {
                i += p;
                if &hay[i..i + needle.len()] == needle {
                    return true;
                }
                i += 1;
            }
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_frameworks() {
        assert_eq!(
            detect_bytes(b"MZ....This installation was built with Inno Setup."),
            InstallerKind::Inno
        );
        assert_eq!(
            detect_bytes(b"MZ..Nullsoft.NSIS.exehead"),
            InstallerKind::Nsis
        );
        assert_eq!(
            detect_bytes(b"MZ..PE..\x00.wixburn\x00 Inno Setup"),
            InstallerKind::WixBurn
        );
        assert_eq!(
            detect_bytes(&[b"MZ".as_slice(), &utf16(b"InstallShield")].concat()),
            InstallerKind::InstallShield
        );
        assert_eq!(detect_bytes(b"MZ plain exe"), InstallerKind::Unknown);
    }

    #[test]
    fn msi_by_extension() {
        assert_eq!(detect(Path::new("C:/nope/x.MSI")), InstallerKind::Msi);
    }

    #[test]
    fn contains_edges() {
        assert!(contains(b"abc", b"abc"));
        assert!(contains(b"xxabc", b"abc"));
        assert!(!contains(b"ab", b"abc"));
        assert!(!contains(b"aab", b"abb"));
    }
}
