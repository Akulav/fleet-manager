//! Async backend: TLS control server, signed discovery responder, and the
//! per-agent connection tasks that keep [`crate::state::Shared`] up to date.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use chrono::Local;
use futures_util::{SinkExt, StreamExt};
use tokio::net::{TcpListener, UdpSocket};
use tokio::sync::mpsc;
use tokio_rustls::TlsAcceptor;
use tokio_util::codec::{Framed, LengthDelimitedCodec};

use fleetview_shared::discovery::{DiscoveryBeacon, DiscoveryQuery};
use fleetview_shared::protocol::{AgentMsg, MasterMsg};
use fleetview_shared::{DISCOVERY_MAGIC, DISCOVERY_PORT, PROTOCOL_VERSION};

use crate::settings::MasterSettings;
use crate::state::{save_known, AgentView, ScreenFrame, SharedState};

pub fn spawn_backend(settings: MasterSettings, state: SharedState) {
    std::thread::spawn(move || {
        let rt = match tokio::runtime::Runtime::new() {
            Ok(rt) => rt,
            Err(e) => {
                if let Ok(mut s) = state.lock() {
                    s.listen_error = Some(format!("runtime: {e}"));
                }
                return;
            }
        };
        rt.block_on(backend_main(settings, state));
    });
}

async fn backend_main(settings: MasterSettings, state: SharedState) {
    let settings = Arc::new(settings);

    // Discovery responder runs regardless of the control listener.
    {
        let settings = settings.clone();
        let state = state.clone();
        tokio::spawn(async move {
            if let Err(e) = discovery_responder(settings, state.clone()).await {
                if let Ok(mut s) = state.lock() {
                    s.log(format!("discovery responder stopped: {e:#}"));
                }
            }
        });
    }

    if let Err(e) = control_server(settings, state.clone()).await {
        if let Ok(mut s) = state.lock() {
            s.listen_error = Some(format!("{e:#}"));
            s.listening = false;
            s.log(format!("control server failed: {e:#}"));
        }
    }
}

fn build_tls_acceptor(settings: &MasterSettings) -> Result<TlsAcceptor> {
    let certs = rustls_pemfile::certs(&mut settings.cert_pem.as_bytes())
        .collect::<std::result::Result<Vec<_>, _>>()
        .context("parse cert pem")?;
    let key = rustls_pemfile::private_key(&mut settings.key_pem.as_bytes())
        .context("parse key pem")?
        .context("no private key in pem")?;

    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let config = rustls::ServerConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .context("protocol versions")?
        .with_no_client_auth()
        .with_single_cert(certs, key)
        .context("server cert")?;
    Ok(TlsAcceptor::from(Arc::new(config)))
}

async fn control_server(settings: Arc<MasterSettings>, state: SharedState) -> Result<()> {
    let acceptor = build_tls_acceptor(&settings)?;
    let listener = TcpListener::bind(("0.0.0.0", settings.control_port))
        .await
        .with_context(|| format!("bind control port {}", settings.control_port))?;

    if let Ok(mut s) = state.lock() {
        s.listening = true;
        s.listen_error = None;
        s.log(format!("listening on 0.0.0.0:{}", settings.control_port));
    }

    loop {
        let (tcp, peer) = listener.accept().await?;
        let acceptor = acceptor.clone();
        let settings = settings.clone();
        let state = state.clone();
        tokio::spawn(async move {
            match acceptor.accept(tcp).await {
                Ok(tls) => {
                    if let Err(e) = handle_agent(tls, peer, settings, state.clone()).await {
                        if let Ok(mut s) = state.lock() {
                            s.log(format!("agent {peer} disconnected: {e:#}"));
                        }
                    }
                }
                Err(e) => {
                    if let Ok(mut s) = state.lock() {
                        s.log(format!("TLS handshake from {peer} failed: {e}"));
                    }
                }
            }
        });
    }
}

async fn handle_agent<S>(
    tls: S,
    peer: SocketAddr,
    settings: Arc<MasterSettings>,
    state: SharedState,
) -> Result<()>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    let mut framed = Framed::new(
        tls,
        LengthDelimitedCodec::builder()
            .max_frame_length(fleetview_shared::MAX_FRAME_BYTES)
            .new_codec(),
    );

    // First message must be Hello.
    let first = framed.next().await.context("no hello")??;
    let hello = match rmp_serde::from_slice::<AgentMsg>(&first)? {
        AgentMsg::Hello(h) => h,
        _ => anyhow::bail!("expected hello"),
    };

    if hello.protocol != PROTOCOL_VERSION {
        let _ = framed
            .send(encode(&MasterMsg::HelloAck {
                accepted: false,
                reason: "protocol version mismatch".into(),
            })?)
            .await;
        anyhow::bail!("protocol mismatch");
    }
    if !fleetview_shared::crypto::secrets_match(
        &hello.enrollment_secret,
        &settings.enrollment_secret,
    ) {
        let _ = framed
            .send(encode(&MasterMsg::HelloAck {
                accepted: false,
                reason: "enrollment secret rejected".into(),
            })?)
            .await;
        anyhow::bail!("bad enrollment secret from {peer}");
    }

    framed
        .send(encode(&MasterMsg::HelloAck {
            accepted: true,
            reason: String::new(),
        })?)
        .await?;

    // Register the agent with a command channel.
    let agent_id = if hello.agent_id.trim().is_empty() {
        format!("host:{}", hello.hostname)
    } else {
        hello.agent_id.clone()
    };
    let (tx, mut rx) = mpsc::unbounded_channel::<MasterMsg>();

    {
        let mut s = state.lock().unwrap();
        let now = Local::now();
        let entry = s
            .agents
            .entry(agent_id.clone())
            .or_insert_with(|| AgentView {
                agent_id: agent_id.clone(),
                hostname: hello.hostname.clone(),
                online: false,
                first_seen: now,
                last_seen: now,
                addr: None,
                telemetry: None,
                software: Vec::new(),
                console_output: String::new(),
                last_frame: None,
                frame_seq: 0,
                streaming: false,
                tx: None,
            });
        entry.online = true;
        entry.hostname = hello.hostname.clone();
        entry.addr = Some(peer);
        entry.last_seen = now;
        entry.tx = Some(tx.clone());
        s.log(format!("{} ({}) connected from {peer}", hello.hostname, agent_id));
        save_known(&s);
    }

    // Ask for an initial snapshot.
    let _ = tx.send(MasterMsg::RequestTelemetry);
    let _ = tx.send(MasterMsg::RequestSoftware);

    let result = agent_loop(&mut framed, &mut rx, &agent_id, &state).await;

    // Mark offline.
    {
        let mut s = state.lock().unwrap();
        if let Some(a) = s.agents.get_mut(&agent_id) {
            a.online = false;
            a.streaming = false;
            a.tx = None;
            a.last_seen = Local::now();
        }
        s.log(format!("{agent_id} went offline"));
        save_known(&s);
    }

    result
}

async fn agent_loop<S>(
    framed: &mut Framed<S, LengthDelimitedCodec>,
    rx: &mut mpsc::UnboundedReceiver<MasterMsg>,
    agent_id: &str,
    state: &SharedState,
) -> Result<()>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    loop {
        tokio::select! {
            outgoing = rx.recv() => {
                match outgoing {
                    Some(msg) => framed.send(encode(&msg)?).await?,
                    None => break,
                }
            }
            incoming = framed.next() => {
                let Some(frame) = incoming else { break };
                let frame = frame?;
                let msg: AgentMsg = rmp_serde::from_slice(&frame)?;
                apply_agent_msg(msg, agent_id, state);
            }
        }
    }
    Ok(())
}

fn apply_agent_msg(msg: AgentMsg, agent_id: &str, state: &SharedState) {
    let mut s = state.lock().unwrap();
    let now = Local::now();
    let Some(a) = s.agents.get_mut(agent_id) else { return };
    a.last_seen = now;
    match msg {
        AgentMsg::Hello(_) => {}
        AgentMsg::Telemetry(t) => {
            a.hostname = t.hostname.clone();
            a.telemetry = Some(t);
        }
        AgentMsg::Software(list) => {
            a.software = list;
        }
        AgentMsg::CommandResult(r) => {
            let header = format!(
                "\n$ [{}] exit={:?} ok={}\n",
                r.command_id, r.exit_code, r.success
            );
            a.console_output.push_str(&header);
            if !r.stdout.is_empty() {
                a.console_output.push_str(&r.stdout);
            }
            if !r.stderr.is_empty() {
                a.console_output.push_str("\n[stderr]\n");
                a.console_output.push_str(&r.stderr);
            }
            // Keep the buffer bounded.
            if a.console_output.len() > 200_000 {
                let cut = a.console_output.len() - 200_000;
                a.console_output = a.console_output.split_off(cut);
            }
        }
        AgentMsg::ScreenFrame { width, height, jpeg, .. } => {
            a.last_frame = Some(ScreenFrame { width, height, jpeg });
            a.frame_seq = a.frame_seq.wrapping_add(1);
        }
        AgentMsg::FileAck { transfer_id, received } => {
            let host = a.hostname.clone();
            s.log(format!("{host}: transfer {transfer_id} {received} bytes"));
        }
        AgentMsg::Heartbeat => {}
        AgentMsg::Log { level, message } => {
            let host = a.hostname.clone();
            s.log(format!("{host} [{level}]: {message}"));
        }
    }
}

// ---------------------------------------------------------------------------
// Discovery responder
// ---------------------------------------------------------------------------

async fn discovery_responder(settings: Arc<MasterSettings>, state: SharedState) -> Result<()> {
    let sock = UdpSocket::bind(("0.0.0.0", DISCOVERY_PORT))
        .await
        .with_context(|| format!("bind discovery port {DISCOVERY_PORT}"))?;
    sock.set_broadcast(true)?;

    let mut buf = vec![0u8; 8192];
    let mut announce = tokio::time::interval(Duration::from_secs(15));

    loop {
        tokio::select! {
            // Periodic unsolicited broadcast so fresh agents converge fast.
            _ = announce.tick() => {
                if let Ok(beacon) = build_beacon(&settings) {
                    let mut dg = DISCOVERY_MAGIC.to_vec();
                    dg.push(0);
                    if let Ok(body) = rmp_serde::to_vec_named(&beacon) {
                        dg.extend_from_slice(&body);
                        let target = format!("255.255.255.255:{DISCOVERY_PORT}");
                        let _ = sock.send_to(&dg, &target).await;
                    }
                }
            }
            res = sock.recv_from(&mut buf) => {
                let (n, src) = match res { Ok(v) => v, Err(_) => continue };
                if n <= DISCOVERY_MAGIC.len() + 1 { continue; }
                if &buf[..DISCOVERY_MAGIC.len()] != DISCOVERY_MAGIC { continue; }
                let body = &buf[DISCOVERY_MAGIC.len() + 1..n];
                // Only answer genuine queries (not our own beacons).
                let Ok(query) = rmp_serde::from_slice::<DiscoveryQuery>(body) else { continue };
                if query.protocol != PROTOCOL_VERSION { continue; }
                if let Ok(beacon) = build_beacon(&settings) {
                    let mut dg = DISCOVERY_MAGIC.to_vec();
                    dg.push(0);
                    if let Ok(bbody) = rmp_serde::to_vec_named(&beacon) {
                        dg.extend_from_slice(&bbody);
                        let _ = sock.send_to(&dg, src).await;
                    }
                }
                if let Ok(mut s) = state.lock() {
                    s.log(format!("discovery query from {} ({src})", query.hostname));
                }
            }
        }
    }
}

fn build_beacon(settings: &MasterSettings) -> Result<DiscoveryBeacon> {
    let issued_at_ms = Local::now().timestamp_millis();
    let bytes = DiscoveryBeacon::signing_bytes(
        PROTOCOL_VERSION,
        settings.control_port,
        &settings.cert_fingerprint,
        &settings.master_name,
        issued_at_ms,
    );
    let signature = settings.identity.sign(&bytes)?;
    Ok(DiscoveryBeacon {
        protocol: PROTOCOL_VERSION,
        control_port: settings.control_port,
        cert_fingerprint: settings.cert_fingerprint.clone(),
        master_name: settings.master_name.clone(),
        issued_at_ms,
        signature,
    })
}

fn encode(msg: &MasterMsg) -> Result<bytes::Bytes> {
    Ok(rmp_serde::to_vec_named(msg)?.into())
}
