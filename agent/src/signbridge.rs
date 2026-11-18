//! Loopback document-signing bridge: browser (e-Management ANSC) → this helper → MoldSign.
//!
//! Why this exists
//! ---------------
//! MoldSign (STISC) runs a local REST server on `127.0.0.1:18480` that only
//! answers browsers whose `Origin` is in its allow-list (`http://localhost` is,
//! the ANSC server address is not). e-Management therefore cannot call MoldSign
//! directly from its own page. This bridge is a thin, tightly-scoped localhost
//! proxy that forwards exactly the three MoldSign calls the signing flow needs,
//! presenting `Origin: http://localhost` to MoldSign so the allow-list is
//! satisfied. The PIN is still entered by the user in MoldSign's own window —
//! nothing here can sign on its own.
//!
//! Security model (this is new local attack surface)
//! -------------------------------------------------
//!   * binds `127.0.0.1` only, on a fixed port separate from the master/TLS and
//!     IPC channels — no `MasterMsg` path reaches here;
//!   * only three request shapes are accepted (list certificates, start a PDF
//!     signature, poll its result) — never arbitrary files, URLs or commands;
//!   * every request must carry `Origin: <pinned e-Management origin>` and a
//!     `Bearer` token that this bridge validates by calling back to the PINNED
//!     e-Management URL (taken from agent config, never from the request — no
//!     SSRF). The signing token is minted by e-Management for the logged-in
//!     user, so a signature can only be initiated by someone with a live session;
//!   * request and response bodies are capped and kept in memory (no temp files);
//!   * MoldSign is auto-started if it is closed, then we wait for it to answer.
//!
//! The listener runs on its own thread with a private current-thread Tokio
//! runtime driving reqwest; requests are served sequentially, which matches the
//! browser's strictly-sequential signing flow (list → sign → poll).

use std::io::Read;
use std::net::{SocketAddr, TcpStream};
use std::time::{Duration, Instant};

use tiny_http::{Header, Method, Request, Response, Server};

/// Fixed loopback port for the sign bridge. Kept clear of the FleetView control
/// (48620) and discovery (48621) ports and of MoldSign's own ports (18480/18443).
pub const PORT: u16 = 48625;

const MOLDSIGN_BASE: &str = "http://127.0.0.1:18480";
/// Origin MoldSign's allow-list accepts (see `allowedURL.properties`).
const MOLDSIGN_ORIGIN: &str = "http://localhost";
const MOLDSIGN_ADDR: &str = "127.0.0.1:18480";

/// PDFs are capped at 50 MB by e-Management; base64 inflates that by ~4/3, so
/// 80 MB comfortably covers a request or response body while bounding memory.
const MAX_BODY: usize = 80 * 1024 * 1024;

/// A started signature pops MoldSign's PIN dialog and the upstream call can
/// block until the user responds, so the per-request budget is generous.
const UPSTREAM_TIMEOUT: Duration = Duration::from_secs(360);
const UPSTREAM_CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

/// How long a successful token validation is trusted before we call e-Management
/// again. The poll loop issues many requests per signature; this keeps the
/// callback from running on every one without meaningfully widening the window.
const TOKEN_CACHE_TTL: Duration = Duration::from_secs(60);

/// Settings handed to the bridge by the service (via the helper command line).
#[derive(Clone, Debug)]
pub struct BridgeConfig {
    /// Exact `Origin` the browser must present (the pinned e-Management origin),
    /// e.g. `https://docs.ansc.md`.
    pub emanagement_origin: String,
    /// Pinned e-Management endpoint used to validate a signing token, e.g.
    /// `https://docs.ansc.md/api/sign/validate`. Never derived from a request.
    pub validate_url: String,
    /// Per-run secret shared service→helper; forwarded to e-Management on the
    /// validate callback so a deployment that pins it can tell genuine bridge
    /// traffic apart. Not the user's signing token.
    pub bridge_token: String,
    /// Optional override for `SignDesktop.exe`; otherwise located under
    /// `Program Files (x86)\STISC\MoldSign`.
    pub moldsign_exe: Option<String>,
}

pub fn spawn(cfg: BridgeConfig) {
    std::thread::spawn(move || {
        if let Err(e) = run(cfg) {
            tracing::error!("sign bridge stopped: {e:#}");
        }
    });
}

fn run(cfg: BridgeConfig) -> anyhow::Result<()> {
    let addr: SocketAddr = ([127, 0, 0, 1], PORT).into();
    let server =
        Server::http(addr).map_err(|e| anyhow::anyhow!("sign bridge cannot bind {addr}: {e}"))?;
    tracing::info!(
        "sign bridge listening on http://{addr} (origin {}, validate {})",
        cfg.emanagement_origin,
        cfg.validate_url
    );

    // Private runtime + client for the outbound (MoldSign / e-Management) calls.
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    let client = reqwest::Client::builder()
        .connect_timeout(UPSTREAM_CONNECT_TIMEOUT)
        .timeout(UPSTREAM_TIMEOUT)
        // Loopback + an internal e-Management host; no redirects to chase.
        .redirect(reqwest::redirect::Policy::none())
        .build()?;

    let mut state = Bridge {
        cfg,
        rt,
        client,
        token_ok_until: None,
        last_token: None,
    };

    for request in server.incoming_requests() {
        state.serve(request);
    }
    Ok(())
}

struct Bridge {
    cfg: BridgeConfig,
    rt: tokio::runtime::Runtime,
    client: reqwest::Client,
    /// The last token that validated (held only in memory), and when that trust
    /// expires. Avoids a callback on every poll of the signing loop.
    last_token: Option<String>,
    token_ok_until: Option<Instant>,
}

impl Bridge {
    fn serve(&mut self, mut request: Request) {
        let method = request.method().clone();
        let url = request.url().to_string();
        let origin = header_value(&request, "origin");

        // The browser only ever reaches us cross-origin, so a missing/foreign
        // Origin is never a legitimate signing request.
        let origin_ok = origin
            .as_deref()
            .map(|o| o == self.cfg.emanagement_origin)
            .unwrap_or(false);

        // CORS preflight carries no Authorization header; answer it before auth.
        if method == Method::Options {
            let resp = if origin_ok {
                cors(Response::empty(204u16), &self.cfg.emanagement_origin)
            } else {
                Response::empty(403u16)
            };
            let _ = request.respond(resp);
            return;
        }

        if !origin_ok {
            let _ = request.respond(json_error(403, "origine refuzată"));
            return;
        }

        // Bearer token, validated against the pinned e-Management URL.
        let token = match bearer(&request) {
            Some(t) => t,
            None => {
                let _ = respond_cors(request, json_error(401, "token lipsă"), &self.cfg);
                return;
            }
        };
        if !self.token_valid(&token) {
            // 401 (not 403) so the page knows to fetch a fresh token and retry.
            let _ = respond_cors(request, json_error(401, "token invalid"), &self.cfg);
            return;
        }

        // Route: only the three MoldSign shapes the signing flow needs, rebuilt
        // from validated parts — never the raw URL (no `..`, no extra query).
        let Some(target_path) = route(&method, &url) else {
            let _ = respond_cors(request, json_error(404, "cerere neacceptată"), &self.cfg);
            return;
        };

        // Make sure MoldSign is running before we forward.
        if let Err(e) = self.ensure_moldsign() {
            tracing::warn!("MoldSign unavailable: {e:#}");
            let _ = respond_cors(
                request,
                json_error(502, "MoldSign nu a putut fi pornit"),
                &self.cfg,
            );
            return;
        }

        // Read the (capped) request body for POST.
        let body = if method == Method::Post {
            match read_body(&mut request) {
                Ok(b) => b,
                Err(code) => {
                    let _ = respond_cors(request, json_error(code, "corp invalid"), &self.cfg);
                    return;
                }
            }
        } else {
            Vec::new()
        };

        match self.forward(&method, &target_path, body) {
            Ok(resp) => {
                let _ = respond_cors(request, resp, &self.cfg);
            }
            Err(e) => {
                tracing::warn!("forward to MoldSign failed: {e:#}");
                let _ = respond_cors(request, json_error(502, "MoldSign nu răspunde"), &self.cfg);
            }
        }
    }

    /// Validate a signing token (with a short cache) by calling the pinned
    /// e-Management endpoint. The URL is never taken from the request.
    fn token_valid(&mut self, token: &str) -> bool {
        if self.last_token.as_deref() == Some(token) {
            if let Some(until) = self.token_ok_until {
                if Instant::now() < until {
                    return true;
                }
            }
        }

        let ok = self.rt.block_on(async {
            let body = serde_json::json!({ "token": token }).to_string();
            let resp = self
                .client
                .post(&self.cfg.validate_url)
                .header("content-type", "application/json")
                .header("accept", "application/json")
                .header("x-fleetview-bridge-token", &self.cfg.bridge_token)
                .body(body)
                .send()
                .await;
            match resp {
                Ok(r) if r.status().is_success() => {
                    let bytes = r.bytes().await.unwrap_or_default();
                    let v: serde_json::Value =
                        serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null);
                    v.get("ok").and_then(|b| b.as_bool()).unwrap_or(false)
                }
                Ok(r) => {
                    tracing::warn!("token validation refused by e-Management: {}", r.status());
                    false
                }
                Err(e) => {
                    tracing::warn!("token validation call failed: {e:#}");
                    false
                }
            }
        });

        if ok {
            self.last_token = Some(token.to_string());
            self.token_ok_until = Some(Instant::now() + TOKEN_CACHE_TTL);
        } else if self.last_token.as_deref() == Some(token) {
            // Previously-trusted token no longer valid; forget it.
            self.last_token = None;
            self.token_ok_until = None;
        }
        ok
    }

    /// Forward one request to MoldSign with the allow-listed Origin and relay the
    /// status, body and the few headers the browser needs back.
    fn forward(
        &self,
        method: &Method,
        url: &str,
        body: Vec<u8>,
    ) -> anyhow::Result<Response<std::io::Cursor<Vec<u8>>>> {
        let target = format!("{MOLDSIGN_BASE}{url}");
        self.rt.block_on(async {
            let mut req = match method {
                Method::Post => self.client.post(&target).body(body),
                _ => self.client.get(&target),
            };
            req = req
                .header("origin", MOLDSIGN_ORIGIN)
                .header("accept", "application/json")
                .header("content-type", "application/json");

            let resp = req.send().await?;
            let status = resp.status().as_u16();
            let location = resp
                .headers()
                .get("location")
                .and_then(|v| v.to_str().ok())
                .map(str::to_owned);
            let moldsign_err = resp
                .headers()
                .get("error")
                .and_then(|v| v.to_str().ok())
                .map(str::to_owned);
            let content_type = resp
                .headers()
                .get("content-type")
                .and_then(|v| v.to_str().ok())
                .unwrap_or("application/json")
                .to_owned();

            // Cap the response body (base64 signed PDF) in memory.
            let bytes = resp.bytes().await?;
            if bytes.len() > MAX_BODY {
                anyhow::bail!("MoldSign response too large");
            }

            let mut out = Response::from_data(bytes.to_vec()).with_status_code(status);
            if let Ok(ct) = Header::from_bytes(b"Content-Type", content_type.as_bytes()) {
                out = out.with_header(ct);
            }
            if let Some(loc) = location {
                if let Ok(h) = Header::from_bytes(b"Location", loc.as_bytes()) {
                    out = out.with_header(h);
                }
            }
            if let Some(err) = moldsign_err {
                if let Ok(h) = Header::from_bytes(b"error", err.as_bytes()) {
                    out = out.with_header(h);
                }
            }
            Ok(out)
        })
    }

    /// If MoldSign isn't answering, launch it and wait (bounded) for it to come up.
    fn ensure_moldsign(&self) -> anyhow::Result<()> {
        if moldsign_up() {
            return Ok(());
        }
        let exe = locate_moldsign(self.cfg.moldsign_exe.as_deref()).ok_or_else(|| {
            anyhow::anyhow!("SignDesktop.exe not found under Program Files (x86)")
        })?;
        tracing::info!("MoldSign not running; launching {}", exe);
        std::process::Command::new(&exe)
            .spawn()
            .map_err(|e| anyhow::anyhow!("cannot launch {exe}: {e}"))?;

        let deadline = Instant::now() + Duration::from_secs(30);
        while Instant::now() < deadline {
            if moldsign_up() {
                tracing::info!("MoldSign is up");
                return Ok(());
            }
            std::thread::sleep(Duration::from_millis(500));
        }
        anyhow::bail!("MoldSign did not start within 30s")
    }
}

fn moldsign_up() -> bool {
    MOLDSIGN_ADDR
        .parse::<SocketAddr>()
        .ok()
        .and_then(|addr| TcpStream::connect_timeout(&addr, Duration::from_millis(500)).ok())
        .is_some()
}

fn locate_moldsign(override_path: Option<&str>) -> Option<String> {
    if let Some(p) = override_path {
        if !p.trim().is_empty() && std::path::Path::new(p).exists() {
            return Some(p.to_string());
        }
    }
    let mut roots = Vec::new();
    if let Some(v) = std::env::var_os("ProgramFiles(x86)") {
        roots.push(std::path::PathBuf::from(v));
    }
    if let Some(v) = std::env::var_os("ProgramFiles") {
        roots.push(std::path::PathBuf::from(v));
    }
    for root in roots {
        let p = root.join("STISC").join("MoldSign").join("SignDesktop.exe");
        if p.exists() {
            return p.to_str().map(str::to_owned);
        }
    }
    None
}

/// Map a browser request to the exact MoldSign path to forward, or `None` if it
/// isn't one of the three allowed shapes:
///
/// * `GET  /certificates[?private_only=true]`
/// * `POST /sign/data`
/// * `GET  /sign/data/{provider}/{session}[/{type}]` (the result to poll)
///
/// Path segments are restricted to `[A-Za-z0-9._-]` (and not `.`/`..`), so a
/// crafted URL can't walk to other MoldSign endpoints; percent-encoding is
/// rejected outright. Any other query string is dropped.
fn route(method: &Method, url: &str) -> Option<String> {
    let (path, query) = url.split_once('?').unwrap_or((url, ""));
    match method {
        Method::Get if path == "/certificates" => Some(if query == "private_only=true" {
            "/certificates?private_only=true".into()
        } else {
            "/certificates".into()
        }),
        Method::Post if path == "/sign/data" && query.is_empty() => Some("/sign/data".into()),
        Method::Get if query.is_empty() => {
            let rest = path.strip_prefix("/sign/data/")?;
            let segs: Vec<&str> = rest.split('/').collect();
            let ok = (2..=3).contains(&segs.len())
                && segs.iter().all(|s| {
                    !s.is_empty()
                        && s.len() <= 128
                        && *s != "."
                        && *s != ".."
                        && s.chars()
                            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_'))
                });
            ok.then(|| format!("/sign/data/{}", segs.join("/")))
        }
        _ => None,
    }
}

// --------------------------------------------------------------------------
// small HTTP helpers
// --------------------------------------------------------------------------

fn header_value(req: &Request, name: &str) -> Option<String> {
    // `HeaderField::equiv` needs a `'static` str, so compare the field directly.
    req.headers()
        .iter()
        .find(|h| h.field.as_str().as_str().eq_ignore_ascii_case(name))
        .map(|h| h.value.as_str().to_string())
}

fn bearer(req: &Request) -> Option<String> {
    let v = header_value(req, "authorization")?;
    let t = v
        .strip_prefix("Bearer ")
        .or_else(|| v.strip_prefix("bearer "))?;
    let t = t.trim();
    if t.is_empty() {
        None
    } else {
        Some(t.to_string())
    }
}

fn read_body(req: &mut Request) -> Result<Vec<u8>, u16> {
    if let Some(len) = req.body_length() {
        if len > MAX_BODY {
            return Err(413);
        }
    }
    let mut buf = Vec::new();
    // Cap regardless of a (possibly absent/lying) Content-Length.
    req.as_reader()
        .take((MAX_BODY as u64) + 1)
        .read_to_end(&mut buf)
        .map_err(|_| 400u16)?;
    if buf.len() > MAX_BODY {
        return Err(413);
    }
    Ok(buf)
}

fn cors<R: std::io::Read>(resp: Response<R>, origin: &str) -> Response<R> {
    let add = |resp: Response<R>, k: &[u8], v: &str| -> Response<R> {
        match Header::from_bytes(k, v.as_bytes()) {
            Ok(h) => resp.with_header(h),
            Err(_) => resp,
        }
    };
    let resp = add(resp, b"Access-Control-Allow-Origin", origin);
    let resp = add(resp, b"Vary", "Origin");
    let resp = add(resp, b"Access-Control-Allow-Methods", "GET, POST, OPTIONS");
    let resp = add(
        resp,
        b"Access-Control-Allow-Headers",
        "authorization, content-type, accept",
    );
    let resp = add(resp, b"Access-Control-Expose-Headers", "Location, error");
    add(resp, b"Access-Control-Max-Age", "600")
}

fn respond_cors<R: std::io::Read>(
    request: Request,
    resp: Response<R>,
    cfg: &BridgeConfig,
) -> std::io::Result<()> {
    request.respond(cors(resp, &cfg.emanagement_origin))
}

fn json_error(status: u16, msg: &str) -> Response<std::io::Cursor<Vec<u8>>> {
    let body = serde_json::json!({ "error": msg }).to_string();
    let mut resp = Response::from_data(body.into_bytes()).with_status_code(status);
    if let Ok(h) = Header::from_bytes(b"Content-Type", b"application/json") {
        resp = resp.with_header(h);
    }
    // Echo the MoldSign-style `error` header so the page's existing error
    // handling (which reads it) shows a meaningful message.
    if let Ok(h) = Header::from_bytes(b"error", msg.as_bytes()) {
        resp = resp.with_header(h);
    }
    resp
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn route_allows_only_the_signing_flow() {
        let get = Method::Get;
        let post = Method::Post;
        assert_eq!(
            route(&get, "/certificates?private_only=true").as_deref(),
            Some("/certificates?private_only=true")
        );
        assert_eq!(
            route(&get, "/certificates?x=1").as_deref(),
            Some("/certificates")
        );
        assert_eq!(route(&post, "/sign/data").as_deref(), Some("/sign/data"));
        assert_eq!(
            route(&get, "/sign/data/moldsign/ab-12_3/pdf").as_deref(),
            Some("/sign/data/moldsign/ab-12_3/pdf")
        );
        // Traversal, encoding, extra query, wrong method/shape.
        assert!(route(&get, "/sign/data/../../admin").is_none());
        assert!(route(&get, "/sign/data/a/%2e%2e").is_none());
        assert!(route(&get, "/sign/data/a/b?c=1").is_none());
        assert!(route(&get, "/sign/data/a").is_none());
        assert!(route(&get, "/sign/data/a/b/c/d").is_none());
        assert!(route(&post, "/sign/data?x").is_none());
        assert!(route(&post, "/certificates").is_none());
        assert!(route(&get, "/settings").is_none());
    }
}
