# FleetView — Setup & Operations

## 1. Prerequisites

- **Master PC:** any Windows 10/11 machine you keep on while managing the fleet.
- **Agent PCs:** Windows 10/11, you have local admin to install once.
- **Network:** a single LAN/subnet works out of the box (UDP broadcast). For
  multiple subnets, see [Multi-subnet](#multi-subnet).
- **To build from source:** Rust (MSVC toolchain); [Inno Setup 6](https://jrsoftware.org/isdl.php) only for the master installer.

## 2. Build the binaries

```bash
cargo build --workspace --release
```

Produces `target/release/fleetview-master.exe` and `fleetview-agent.exe`.

> On a fresh Windows box, install the toolchain with
> `winget install Rustlang.Rustup` and the **“Desktop development with C++”**
> workload (or VS Build Tools) so the MSVC linker is present.

## 3. First run of the master

Run `fleetview-master.exe`. On first launch it creates, under
`%APPDATA%\FleetView`:

- an Ed25519 **identity key** (signs discovery beacons),
- a self-signed **TLS certificate** for the control channel,
- a random **enrollment secret**.

The window shows the listening status and an **Online / ever-seen** counter.

## 4. Make the agent setup file

The agent is **one exe** that installs itself, updates itself and runs as the
service — there is no separate installer.

In the master, click **“Agent installer”**:

- *Fixed master address* — leave empty for **auto-discovery**, or set
  `192.168.1.50:48620` to skip discovery and connect to a fixed host.
- *e-Management URL* — optional; set it (e.g. `http://docs.ansc.local:3080`) to
  turn on the **MoldSign signing bridge** for e-Management ANSC.

Click **“Save agent setup (.exe)…”**. You get `FleetViewAgent-Setup-<version>.exe`:
the agent package (the `fleetview-agent.exe` installed next to the master, or
the one picked with *Choose agent exe…*) with this config embedded:

```json
{
  "master_public_key": "…",
  "master_cert_fingerprint": "…",
  "enrollment_secret": "…",
  "master_address": null,
  "discovery_targets": [],
  "telemetry_interval_secs": 30,
  "emanagement_url": "http://docs.ansc.local:3080"
}
```

Keep that file private: it contains the enrollment secret. (The plain JSON can
still be copied or saved; a plain `fleetview-agent.exe` with
`agent.config.json` next to it works too.)

## 5. Build from source / release

```powershell
cargo build --workspace --release
cd installer
& "${env:ProgramFiles(x86)}\Inno Setup 6\ISCC.exe" master.iss   # master installer, bundles the agent exe
```

CI does this on a `v*` tag (see [Versioning](#versioning)).

## 6. Install on an agent PC

Double-click `FleetViewAgent-Setup-*.exe` and accept the UAC prompt (or run
`FleetViewAgent-Setup-*.exe /S` from an admin script for a silent install).
It:

1. copies itself to `Program Files\FleetView Agent\fleetview-agent.exe`
   (without the embedded config),
2. writes the config to `C:\ProgramData\FleetView\agent.config.json`, merged
   with any config already there (an existing `agent_id` and local keys are
   kept),
3. registers the **`FleetViewAgent`** service (LocalSystem, auto-start,
   auto-restart on failure) and starts it,
4. locks down `C:\ProgramData\FleetView` to SYSTEM + Administrators,
5. adds **FleetView Agent** to *Apps & features*.

Running it on a PC that already has the agent **updates** it in place (the
service is stopped, the exe swapped, the service started; if the new version
doesn't start, the previous exe is put back). PCs installed with the old
Inno Setup installer are upgraded the same way and its leftovers removed.

The agent discovers the master and appears in the master's list within a few
seconds.
## 7. Daily use

- **Telemetry / disks** — select a PC → *Overview*.
- **Install software** — *Software* tab → *Install file…*, pick the installer,
  set the **silent args** (e.g. `/S` for NSIS, `/qn` for MSI, `/quiet` for many
  others), and it runs as SYSTEM on the agent. *Install from URL* downloads then
  runs it.
- **Uninstall** — *Software* tab → *Uninstall* next to an entry (best-effort
  silent; some apps still show UI in session 0).
- **PowerShell** — *PowerShell* tab, optionally *Run on ALL online agents*.
- **Screen** — *Screen* tab → *Capture once* or *Start live view*. The user is
  notified for every screenshot and when live view begins, and while the screen
  is being viewed a **red frame and a watermark bar** are shown across their
  desktop (click-through, so it doesn't get in the way) — remote viewing is
  deliberately impossible to miss.
- **Fleet actions** (Fleet overview) — push one installer or an https URL to
  **every online PC** at once (same silent args), and open **Remote config**.
- **Remote config** — change a few agent settings on running PCs without
  reinstalling: `emanagement_url` (the MoldSign bridge), `master_address`,
  `discovery_targets`, `telemetry_interval_secs`, `moldsign_path`. Tick a key,
  set its value (empty clears it), choose *All online* or *Selected PC*, Apply.
  The agent applies only these keys — never its master identity or enrollment
  secret — and restarts to pick them up. If a `master_address` stops working
  (master moved, or a typo), the agent starts alternating it with UDP discovery
  after 3 failed attempts, so a PC on the master's subnet finds it again; one on
  another subnet needs a correct address or `discovery_targets` (see
  [Multi-subnet](#multi-subnet)).

User notifications are shown as FleetView toasts in the bottom-right corner (not
system message boxes), so they appear above other windows without stealing focus.

### Silent-install switches (cheat sheet)

| Installer type | Silent switch |
|----------------|---------------|
| NSIS | `/S` |
| Inno Setup | `/VERYSILENT /SUPPRESSMSGBOXES /NORESTART` |
| MSI (check **MSI**) | `/qn /norestart` |
| InstallShield | `/s /v"/qn"` |
| Many others | `/quiet` or `/silent` |

If a vendor installer has **no** silent mode (the MSign case), wrap it: extract it
with `7z`, deploy the payload with a PowerShell script, or repackage it as an MSI
with a tool like *MSI Wrapper* / *PSADT*, then push that.

## 8. Updating agents remotely

The master compares each agent's version with its **agent package** (the
`fleetview-agent.exe` next to the master — the master installer puts the
matching one there — or any exe picked with *Choose agent exe…*):

- the *Fleet overview* shows an **Agent** column; older agents are marked ⬆;
- **“⬆ Update N outdated online PC(s)”** (with a confirmation) updates them all,
  4 at a time; a PC's *Overview* tab has **“⬆ Update to x.y.z”** for one PC.

The update is an ordinary file push run with `update`, so it also works for
agents still on 0.1.0. On the PC the pushed exe hands off to a detached worker
and returns at once; the worker stops the service, swaps the exe and starts
the new version, rolling back to the previous exe if it doesn't come up. The
master shows the progress per PC and logs “agent updated a → b” when the PC
reconnects with the new version. If an *e-Management URL* is set, the update
also writes it to the agent config (turning on the MoldSign bridge).
Downgrades are never offered. The worker logs to
`C:\ProgramData\FleetView\logs\update.log*`.

## <a name="versioning"></a>9. Versioning

- There is **one version** for master and agent: `[workspace.package] version`
  in the root `Cargo.toml` (semver `major.minor.patch`).
- It is compiled into both exes, shown in the master's title bar, reported by
  every agent, and embedded in the agent exe as `FLEETVIEW_AGENT_VERSION=x.y.z`
  so the master can read a file's version without running it.
- To release: bump the version in `Cargo.toml`, commit, then tag the same
  version. The release workflow refuses a tag that doesn't match.

```bash
git tag v0.2.0
git push origin v0.2.0
```

  The release contains `FleetViewMaster-Setup-x.y.z.exe` (with the agent
  package), `FleetViewAgent-x.y.z.exe` and `fleetview-master.exe`.
- Upgrade the master first; its new agent package then flags every older agent
  as outdated.

## <a name="multi-subnet"></a>Multi-subnet networks

UDP broadcasts don't cross routers. If agents are on different subnets than the
master, either:

- set a fixed `"master_address"` in `agent.config.json`, **or**
- list the master's subnet broadcast address in `"discovery_targets"`, e.g.
  `["192.168.20.255:48621"]`.

## Ports

| Port | Proto | Purpose |
|------|-------|---------|
| 48620 | TCP | Master control channel (TLS). Open inbound on the master. |
| 48621 | UDP | Discovery. |

Agents dial **out** to the master, so agent PCs need no inbound rules.

## Uninstall

- Agent: *Apps & features* → *FleetView Agent*, or `fleetview-agent.exe uninstall` (`--silent` for scripts). Removes the service, the program and the config; `audit.log`, `logs` and `agent.id` stay.
- Master: *Apps & features* → *FleetView Master*.
