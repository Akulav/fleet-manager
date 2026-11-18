<div align="center">

# FleetView

**Self-hosted fleet management for Windows LANs — no cloud, no server required.**

Made by **Akulav** · Licensed under the [MIT License](LICENSE)

</div>

FleetView is a lightweight master/agent system for managing a fleet of Windows
PCs on a local network. It was built for the case where you administer dozens of
machines by hand and are **not allowed to run your own servers** — the master is
just a desktop app you run on your own PC, and agents find it automatically over
the LAN even when IP addresses change.

> ⚠️ **Authorized use only.** FleetView runs commands as SYSTEM and can view
> screens. Deploy it only on machines you are authorized to administer, and tell
> your users it is installed. The agent is intentionally **visible** (tray icon +
> logon splash) and every remote command is written to a local audit log.

---

## Features

- **Auto-discovery** — agents locate the master via signed UDP broadcasts, so the
  master's IP can change freely. A fixed address and extra subnets are also
  configurable.
- **Live telemetry** — CPU, memory, **disk usage**, uptime, OS, logged-in user,
  IP/MAC addresses, refreshed on a schedule.
- **Online vs. ever-seen** — the dashboard shows how many PCs are online now
  against the total number ever enrolled.
- **Software inventory** — the installed-programs list from each machine.
- **Remote install / uninstall** — push an installer (`.exe`/`.msi`) from the
  master and run it **silently as admin**, or install from a URL. No more walking
  to every desk to type your admin password for the next MSign update.
- **PowerShell as admin** — run a script on one machine or **all online machines**
  at once; output comes back to the console.
- **Screenshots & live view** — capture the screen on demand or stream it. The
  user is notified when a live session starts.
- **Hardened transport** — TLS with certificate pinning + a shared enrollment
  secret, so agents trust only *your* master.
- **Always-on agent** — runs as a LocalSystem Windows service, auto-starts at
  boot, and restarts automatically if it stops.
- **Localized** — the master app is **English**; the agent tray/splash default to
  **Romanian**.

## Repository layout

| Path | What it is |
|------|------------|
| `shared/` | Wire protocol, crypto/identity, discovery message format. |
| `agent/`  | The agent: Windows service + user-session tray/splash/capture helper. |
| `master/` | The `egui` desktop control app. |
| `installer/` | Inno Setup scripts + branding assets (icons, splash). |
| `.github/workflows/` | CI build and tagged `.exe` auto-releases. |
| `docs/` | Setup, security and architecture notes. |

## Quick start

1. **Build or download** the two installers (see [docs/SETUP.md](docs/SETUP.md)).
2. **Run the master** (`FleetViewMaster-Setup`). On first launch it generates its
   identity, TLS certificate and an enrollment secret.
3. In the master, click **“Agent installer config”** and save `agent.config.json`
   next to `agent.iss` (or into the agent install folder). This pins your master.
4. **Install the agent** on each PC (`FleetViewAgent-Setup`, run as admin). It
   registers the service, shows the logon splash, and appears in the tray.
5. Agents discover the master and appear in the list. Select one to see telemetry,
   push software, run PowerShell, or view the screen.

See [docs/SETUP.md](docs/SETUP.md) for the full walkthrough and
[docs/SECURITY.md](docs/SECURITY.md) for the trust model and hardening steps.

## Building from source

Requires the Rust MSVC toolchain and (for installers) Inno Setup 6.

```bash
cargo build --workspace --release
```

Binaries land in `target/release/` (`fleetview-master.exe`, `fleetview-agent.exe`).
Build the installers with the scripts in `installer/` (see
[docs/SETUP.md](docs/SETUP.md)).

## Releases

Pushing a tag like `v0.1.0` triggers `.github/workflows/release.yml`, which builds
the release binaries, compiles both Inno Setup installers, and attaches the
`.exe` files to a GitHub Release.

```bash
git tag v0.1.0
git push origin v0.1.0
```

## License

MIT © 2026 Akulav. See [LICENSE](LICENSE).
