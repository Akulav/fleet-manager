# Changelog

All notable changes to FleetView are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and the project uses [Semantic Versioning](https://semver.org/).
Older versions are described on the [releases page](https://github.com/Akulav/fleet-manager/releases).

## [0.4.1] - 2026-11-20

### Changed
- **License:** FleetView is now source-available under the Akulav Source-Available License, like Akulav's other
  apps (it was MIT). The programs' file details say so too.
- README in the style of Akulav's other apps, with links to the setup, security and architecture docs.

### Fixed
- Pushing a file named like a Windows device (`nul.exe`, `COM1`, `LPT2.msi`…) no longer fails in a confusing way:
  the agent saves it under a safe name.