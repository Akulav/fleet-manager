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
- **Remote install / uninstall** — push one or **several installers at once**
  (`.exe`/`.msi`) from the master and run them **silently as admin**, or install
  from a URL. Silent switches are **detected per file** (MSI, Inno Setup, NSIS,
  WiX bundle, InstallShield, Advanced Installer); installs run one after another
  on each PC so MSIs never collide, and "restart required" (3010) counts as
  success. Failed MSIs report the tail of their log. URL installs are
  **https-only**; runs time out after 60 minutes.
- **Standards** — one local admin account (same name + password on every PC,
  optionally demoting every other local admin), one desktop background (for all
  profiles and new users, optionally locked) and one app package. Each can be
  kept applied **automatically**, so PCs that enroll later get it too. PCs can be
  renamed one by one or from a pattern such as `ANSC-###`.
- **PowerShell as admin** — run a script on one machine or **all online machines**
  at once (with a confirmation); output comes back to the console.
- **Screenshots & live view** — capture the screen on demand or stream it. The
  agent notifies the user for every screenshot and when a live view starts.
- **Hardened transport** — TLS with certificate pinning + a shared enrollment
  secret, so agents trust only *your* master.
- **Always-on agent** — runs as a LocalSystem Windows service, auto-starts at
  boot, and restarts automatically if it stops.
- **One exe, remote updates** — the agent exe installs and updates itself (no
  installer); the master shows every agent's version and pushes updates to
  outdated PCs, with automatic rollback if a new version doesn't start.
- **MoldSign signing bridge** — optional: lets e-Management ANSC sign documents
  with the user's token through the agent (set the e-Management URL in the master).
- **Fleet actions & remote config** — push one installer or URL to every online
  PC at once, and change a few agent settings (e-Management URL, master address,
  discovery, telemetry interval) on running PCs without reinstalling.
- **Visible remote viewing** — while the screen is captured or streamed, the
  agent shows a red frame + watermark across the desktop and a toast; nothing is
  hidden from the user.
- **Localized** — the master app is **English**; the agent tray/splash default to
  **Romanian**.

### Master app

- **Dashboard** — online/known PCs, PCs needing attention, outdated agents and
  running jobs at a glance, plus fleet CPU/memory/disk averages, the busiest PCs,
  OS breakdown, recent jobs and activity.
- **Devices table** — search (Ctrl+F), status chips (online, offline, needs
  attention, outdated), tag filter, sortable columns with CPU/memory/disk meters,
  and CSV export.
- **Bulk actions** — tick any set of PCs and run a script, install, message
  users, update the agent, restart/shut down (with a 1-minute warning, and
  cancel), Wake-on-LAN, tag, push remote config, or remove offline PCs. Risky
  actions always show a confirmation listing every target.
- **Jobs** — every action becomes a job with a per-PC result (succeeded,
  failed + exit code, skipped, running) and its output; re-run the failures.
- **Alerts** — disk, memory, sustained CPU, long-offline and failed-update
  alerts with adjustable thresholds.
- **Device page** — live CPU/memory charts, disks, system facts (click to copy),
  tags and notes, software with uninstall, a PowerShell console with a snippet
  library, saved snippets and history (Ctrl+Enter / Ctrl+↑↓), screen capture with
  fit/actual size and save-as-PNG, and the PC's job history.
- **Fleet software** — every program across the fleet, how many PCs have it,
  which versions, where it is missing, and uninstall from all of them.
- **Dark and light themes.**
- **Demo mode** — `fleetview-master.exe --demo` opens a simulated fleet (no
  network, separate data folder) to try the app without agents.

## Repository layout

| Path | What it is |
|------|------------|
| `shared/` | Wire protocol, crypto/identity, discovery message format. |
| `agent/`  | The agent: Windows service + user-session tray/splash/capture helper. |
| `master/` | The `egui` desktop control app. |
| `installer/` | Master Inno Setup script + branding assets (icons, splash). |
| `.github/workflows/` | CI build and tagged `.exe` auto-releases. |
| `docs/` | Setup, security and architecture notes. |

## Quick start

1. **Install the master** (`FleetViewMaster-Setup`). On first launch it generates
   its identity, TLS certificate and an enrollment secret.
2. In the master, click **“Agent installer”** → **“Save agent setup (.exe)…”**.
   That one file is the agent: it carries the config that pins your master.
3. **Double-click it on each PC** (UAC prompt; `/S` for silent). It installs the
   service, shows the logon splash, and appears in the tray. Running it again,
   or a newer one, updates the agent in place.
4. Agents discover the master and appear in the list. Select one to see telemetry,
   push software, run PowerShell, or view the screen.
5. **Updates:** after upgrading the master, *Fleet overview* → **“⬆ Update N
   outdated online PC(s)”** pushes the new agent to every PC remotely.

See [docs/SETUP.md](docs/SETUP.md) for the full walkthrough and
[docs/SECURITY.md](docs/SECURITY.md) for the trust model and hardening steps.

## Building from source

Requires the Rust MSVC toolchain (and Inno Setup 6 for the master installer).

```bash
cargo build --workspace --release
```

Binaries land in `target/release/` (`fleetview-master.exe`, `fleetview-agent.exe`).
The agent exe needs no installer; build the master's with `installer/master.iss`
(see [docs/SETUP.md](docs/SETUP.md)).

## Releases

One version for everything: `[workspace.package] version` in `Cargo.toml`. Bump
it, commit, then push the matching tag; `.github/workflows/release.yml` checks
the tag against `Cargo.toml`, builds and tests, and publishes
`FleetViewMaster-Setup-x.y.z.exe` (with the agent package),
`FleetViewAgent-x.y.z.exe` and `fleetview-master.exe`.

```bash
git tag v0.2.0
git push origin v0.2.0
```

## License

MIT © 2026 Akulav. See [LICENSE](LICENSE).
