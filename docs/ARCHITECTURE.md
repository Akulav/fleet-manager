# FleetView — Architecture

## Overview

```
                         ┌─────────────────────────────┐
                         │          MASTER PC          │
                         │  fleetview-master.exe (egui)│
                         │  ┌───────────────────────┐  │
   UDP 48621 (signed)    │  │ discovery responder   │  │
 ◄───────────────────────┼──┤ TLS server :48620     │  │
                         │  │ per-agent tasks        │  │
                         │  └───────────────────────┘  │
                         └──────────▲──────────────────┘
          TLS 48620 (agent dials out, cert-pinned)
                                    │
             ┌──────────────────────┼───────────────────────┐
             │                      │                        │
   ┌─────────┴─────────┐  ┌─────────┴─────────┐    ┌─────────┴─────────┐
   │     AGENT PC       │  │     AGENT PC       │    │     AGENT PC       │
   │  service (SYSTEM)  │  │  service (SYSTEM)  │ …  │  service (SYSTEM)  │
   │   └─ loopback IPC  │  │                    │    │                    │
   │  helper (user)     │  │  helper (user)     │    │  helper (user)     │
   │   tray · splash ·  │  │                    │    │                    │
   │   screen capture   │  │                    │    │                    │
   └────────────────────┘  └────────────────────┘    └────────────────────┘
```

Agents **dial out** to the master, so agent machines need no inbound firewall
rules; only the master exposes a port.

## Crates

- **`shared`** — no Windows/async code; unit-testable.
  - `protocol` — `AgentMsg` / `MasterMsg` enums, telemetry & inventory types.
    Frames are length-delimited **MessagePack**.
  - `crypto` — master identity (Ed25519), beacon signing/verification, TLS cert
    fingerprinting, enrollment-secret comparison.
  - `discovery` — the `DiscoveryQuery` / signed `DiscoveryBeacon` format.
- **`agent`** — see below.
- **`master`** — `egui`/`eframe` UI + a background `tokio` runtime hosting the TLS
  server and discovery responder; shared state behind a `Mutex`.

## Agent process model

One executable, several modes (first arg):

| Mode | Runs as | Role |
|------|---------|------|
| `run-service` | LocalSystem (SCM) | background worker |
| `install` / `uninstall` | admin (once) | service management |
| `console` | current user | foreground debug run |
| `helper` | interactive user | tray, splash, screen capture |

### Why a split service + helper?

A service lives in **session 0** and cannot draw UI to, or capture the screen of,
the interactive desktop. So the LocalSystem service launches a **helper** into the
active session with `CreateProcessAsUserW` (grabbing the console session's token
via `WTSQueryUserToken` + `DuplicateTokenEx`). The helper owns the tray icon,
shows the splash, and does screen capture. It is relaunched whenever it exits or
the active session changes (logoff / fast user switch / RDP).

### Service ↔ helper IPC

The service binds an authenticated **loopback** TCP channel (`127.0.0.1:<random>`)
and passes the port + a random token to the helper on its command line. The helper
authenticates, then:

- service → helper: capture-once, start/stop stream, notify user, show splash,
  connection status;
- helper → service: screen frames (JPEG), the logged-in user, log lines.

Screen frames are relayed by the service to the master over TLS. Nothing touches
the network except through the pinned TLS channel.

### Async task layout (service)

`worker_main` supervises three tasks:

1. **IPC server** for the helper.
2. **Helper supervisor** (session watcher + relaunch).
3. **Connection loop** — discover master → TLS connect → enroll → message loop,
   with exponential backoff and automatic reconnect on IP change.

## Discovery flow

1. Agent broadcasts a `DiscoveryQuery` (UDP 48621) to the broadcast address and
   any configured subnet targets.
2. Master replies (unicast) with a `DiscoveryBeacon` signed by its identity key,
   carrying the control port and TLS fingerprint. It also broadcasts a beacon
   every 15 s so fresh agents converge quickly.
3. Agent verifies the signature (pinned public key) and the fingerprint, then
   connects to `source-ip:control_port` over TLS and enrolls.

A fixed `master_address` in the agent config skips discovery entirely.

## Reliability

- The service is `AutoStart` and has `sc failure` recovery actions (restart after
  5 s, repeatedly), so a crash or kill brings it back.
- The helper is restarted by the service; the tray has no *Exit* for users.
- A local administrator can still stop the service via `services.msc` — this is
  intentional and not worked around.

## Data & persistence

| Location | Contents |
|----------|----------|
| Master `%APPDATA%\FleetView\master.settings.json` | identity key, TLS cert/key, enrollment secret |
| Master `%APPDATA%\FleetView\known_agents.json` | every agent ever seen (for the ever-seen metric) |
| Agent `%ProgramData%\FleetView\agent.config.json` | pinned master identity (ACL: SYSTEM+Admins) |
| Agent `%ProgramData%\FleetView\audit.log` | every remotely-run command |
| Agent `%ProgramData%\FleetView\logs\` | service logs |
