# FleetView — Security Model

FleetView lets a master run commands as SYSTEM and view screens across a fleet.
That power demands a clear trust model. This document describes it and the
hardening the project applies.

## Threat model

- **In scope:** another host on the LAN trying to impersonate the master; a rogue
  agent trying to enroll; passive eavesdropping on the wire.
- **Partly in scope:** a local non-admin user on an agent trying to read secrets
  or stop the agent.
- **Out of scope:** a local **administrator** on an agent (they own the machine
  and can stop any service); a compromised master host (it is the trust root).

## Trust establishment

The master is the trust anchor. On first run it generates:

1. an **Ed25519 identity keypair** — signs discovery beacons;
2. a **self-signed TLS certificate** — secures the control channel;
3. a random **enrollment secret**.

The agent is provisioned (via `agent.config.json`) with the **public** half:
the master's public key, the TLS certificate **SHA-256 fingerprint**, and the
enrollment secret.

### Discovery can't be spoofed

A discovery beacon is signed with the master's identity key and names the TLS
fingerprint. An agent **verifies the signature** against its pinned public key and
checks the fingerprint before connecting. A rogue host without the private key
cannot forge a beacon, so agents won't follow it — even after a legitimate IP
change.

### The channel is pinned, not CA-trusted

The agent's TLS verifier ignores the public CA store and instead requires the
server certificate's SHA-256 to equal the **pinned** fingerprint. This both
removes the need for PKI on a closed LAN and blocks man-in-the-middle hosts.

### Agents must prove enrollment

After the TLS handshake the agent presents the **enrollment secret**; the master
rejects anything else. So joining the fleet requires possessing the secret.

## What this means for secrets

- **Keep `agent.config.json` and the master settings private.** Anyone with the
  enrollment secret can enroll an agent and receive commands as that master.
- The agent config lives in `C:\ProgramData\FleetView`, which the installer
  restricts to **SYSTEM + Administrators**. The IPC token for the service↔helper
  channel is passed to the helper on its command line, not stored world-readable.
- The master's identity key and TLS private key live in `%APPDATA%\FleetView` on
  the master host. Back them up; losing them means re-provisioning all agents.

## Transparency & accountability (by design)

FleetView is **not** covert surveillance software and should not be deployed as
such:

- the agent shows a **tray icon** whenever it runs and a **splash at logon**;
- starting a **live screen view** notifies the user;
- every remote command (PowerShell, install, file run) is appended to a local
  **audit log** at `C:\ProgramData\FleetView\audit.log` on the agent;
- the service name, publisher and description are plainly labelled.

Tell your users the agent is installed and what it does. At an organization,
get sign-off from IT security before deployment.

## Residual risks & recommendations

- **Unsigned binaries** may be flagged by Defender/SmartScreen. Prefer
  **code-signing** the executables (and the installers) with your organization's
  certificate. Do **not** try to evade detection; use an exclusion via policy only
  with approval.
- **Session-0 command execution** runs as SYSTEM — scripts and installers you push
  have full control of the machine. Review what you push.
- The pinned-cert verifier accepts the handshake signature once the leaf cert
  matches; this is standard for certificate pinning but means the cert is the sole
  trust factor — protect the master's key material accordingly.
- Consider network segmentation so the master's control port (48620/TCP) is only
  reachable from managed subnets.

## Reporting

Found a security issue? Open a private report / security advisory on the project's
GitHub repository rather than a public issue.
