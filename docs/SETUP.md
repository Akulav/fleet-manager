# FleetView — Setup & Operations

## 1. Prerequisites

- **Master PC:** any Windows 10/11 machine you keep on while managing the fleet.
- **Agent PCs:** Windows 10/11, you have local admin to install once.
- **Network:** a single LAN/subnet works out of the box (UDP broadcast). For
  multiple subnets, see [Multi-subnet](#multi-subnet).
- **To build from source:** Rust (MSVC toolchain) + [Inno Setup 6](https://jrsoftware.org/isdl.php).

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

## 4. Provision the agent config

In the master, click **“Agent installer config”**. You get an `agent.config.json`
containing the master's public key, TLS fingerprint and enrollment secret:

```json
{
  "master_public_key": "…",
  "master_cert_fingerprint": "…",
  "enrollment_secret": "…",
  "master_address": null,
  "discovery_targets": [],
  "telemetry_interval_secs": 30
}
```

- Leave `master_address` null to use **auto-discovery**.
- Set it to `"192.168.1.50:48620"` to skip discovery and connect to a fixed host.

**Copy** or **Save to file** this JSON.

## 5. Build the installers

Put `agent.config.json` next to `installer/agent.iss` (optional — it gets baked
into `%ProgramData%\FleetView`), then:

```powershell
cd installer
$env:FLEETVIEW_VERSION = "0.1.0"
& "${env:ProgramFiles(x86)}\Inno Setup 6\ISCC.exe" agent.iss
& "${env:ProgramFiles(x86)}\Inno Setup 6\ISCC.exe" master.iss
```

Installers appear in `dist/`. (CI does all of this automatically on a `v*` tag.)

If you did **not** bundle `agent.config.json`, copy it manually to
`C:\ProgramData\FleetView\agent.config.json` on each agent after install.

## 6. Install on an agent PC

Run `FleetViewAgent-Setup-*.exe` **as administrator**. It:

1. installs the exe to `Program Files\FleetView Agent`,
2. registers the **`FleetViewAgent`** service (LocalSystem, auto-start),
3. sets service recovery to auto-restart on failure,
4. locks down `C:\ProgramData\FleetView` to SYSTEM + Administrators,
5. starts the service, which shows the **logon splash** and the **tray icon**.

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
  notified when live view begins.

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

- Agent: *Apps & features* → *FleetView Agent* (removes the service).
- Master: *Apps & features* → *FleetView Master*.
