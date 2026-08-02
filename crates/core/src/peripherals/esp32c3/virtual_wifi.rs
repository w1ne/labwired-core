//! Virtual WiFi medium + infrastructure **internet gateway** for simulated
//! stations (ESP32-C3 WiFi MAC).
//!
//! # Universal bridge (not a single URL)
//!
//! Once a station associates, the AP behaves like a real SoftAP router:
//!
//! 1. **DHCP** — lease + gateway + DNS = AP IP (`192.168.4.1`)
//! 2. **DNS (UDP/53)** — resolve **any** hostname (native resolver / browser DoH)
//! 3. **TCP off-LAN** — **NAT** to **any** destination IP:port (native sockets;
//!    browser: HTTP(S) via host `fetch` for cleartext HTTP clients)
//! 4. **TCP to AP:80** — **HTTP reverse proxy for any `Host:`** (and a small
//!    optional local `/v1/public-stats` convenience for the LBC3.1 demo sketch)
//! 5. **STA↔STA** — L2/L3 forward between associated stations
//!
//! Internet is not limited to `api.labwired.com`. That host is only one of many
//! possible destinations, plus an optional local demo origin when firmware
//! still talks to `192.168.4.1` with an empty Host.
//!
//! # Medium model
//!
//! Each `wifi_mac` submits TX frames via [`VirtualWifiBus::submit`] and pulls
//! its inbox each tick. The air-gap (no RF) is the intentional cut; L3/L4 to
//! the host network is real.

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex, OnceLock};

use crate::network::sim::{HttpResponse, HttpServer, SimServer};
use crate::peripherals::esp32c3::virtual_wifi_inet::{
    dns_respond, internet_enabled, EgressTable, EgressTcp,
};

/// AP identity.
const AP_BSSID: [u8; 6] = [0x02, 0x00, 0x00, 0x00, 0x00, 0x01];
const AP_MAC_L2: [u8; 6] = AP_BSSID;
const AP_IP: [u8; 4] = [192, 168, 4, 1];
const NETMASK: [u8; 4] = [255, 255, 255, 0];
const AP_SSID: &str = "labwired-ap";
/// First DHCP-assignable host octet (192.168.4.2, .3, …).
const FIRST_HOST: u8 = 2;
/// UDP echo-server port the AP hosts (matches the firmware probe).
const UDP_ECHO_PORT: u16 = 9999;

/// TCP flag bits.
const TCP_FIN: u8 = 0x01;
const TCP_SYN: u8 = 0x02;
const TCP_RST: u8 = 0x04;
const TCP_PSH: u8 = 0x08;
const TCP_ACK: u8 = 0x10;
/// Receive window the AP advertises to the STA (ample for a small HTTP GET).
const TCP_WINDOW: u16 = 0x2000;

/// Optional local convenience origin only (LBC3.1 demo still GETs
/// `192.168.4.1/v1/public-stats`). General internet is DNS + NAT / Host proxy
/// for **any** host — not this constant.
const STATS_SNAPSHOT: &str = concat!(
    "{\"generated_at\":\"2026-07-24T19:39:15.804Z\",\"window_days\":90,",
    "\"boards_supported\":9,\"parts_supported\":82,",
    "\"labs_opened\":69,\"simulations_run\":3200,\"active_sessions\":4900}"
);

/// Used only when resolving the optional local `/v1/public-stats` convenience.
const PUBLIC_STATS_HOST: &str = "api.labwired.com";
const PUBLIC_STATS_PATH: &str = "/v1/public-stats";

/// Result of handling an HTTP request on the AP (:80).
enum HttpServeResult {
    /// Response bytes ready immediately (local origin or native proxy).
    Ready(Vec<u8>),
    /// Browser host-net is fetching; `poll_egress` will finish later.
    Pending(u32),
}

/// True when `Host` refers to the AP itself (or is missing) — local origin
/// path. Any other Host is reverse-proxied to the internet (universal).
fn is_local_http_host(host: &str, ap_ip: [u8; 4]) -> bool {
    let h = host.trim().to_ascii_lowercase();
    if h.is_empty() {
        return true;
    }
    let host_only = h.split(':').next().unwrap_or(&h);
    if host_only == "localhost" || host_only == "labwired-ap" {
        return true;
    }
    let ap = format!("{}.{}.{}.{}", ap_ip[0], ap_ip[1], ap_ip[2], ap_ip[3]);
    host_only == ap
}

/// Universal HTTP on the AP: reverse-proxy **any** non-local `Host` to the
/// real network; keep optional local `/v1/public-stats` for the demo sketch.
fn serve_or_proxy_http(local: &Arc<dyn SimServer>, req: &[u8]) -> HttpServeResult {
    use crate::peripherals::esp32c3::virtual_wifi_host_net::{
        enqueue_http, parse_http_request_line_host,
    };

    let parsed = parse_http_request_line_host(req);
    if let Some((method, path, host)) = parsed {
        if !is_local_http_host(&host, AP_IP) {
            // Any Host on the internet — not limited to one API.
            let host_only = host.split(':').next().unwrap_or(&host);
            let port: u16 = host
                .split(':')
                .nth(1)
                .and_then(|p| p.parse().ok())
                .unwrap_or(80);
            let scheme = if port == 443 { "https" } else { "http" };
            let url = if path.starts_with("http://") || path.starts_with("https://") {
                path
            } else {
                format!("{scheme}://{host_only}{path}")
            };
            // Native: synchronous TCP proxy to that host (any host).
            #[cfg(not(target_arch = "wasm32"))]
            {
                if let Some(resp) = native_http_proxy_raw(req, host_only, port) {
                    return HttpServeResult::Ready(resp);
                }
            }
            // Browser host-net: async fetch of any URL.
            if crate::peripherals::esp32c3::virtual_wifi_host_net::bridge_active() {
                let id = enqueue_http(url, method, req.to_vec());
                return HttpServeResult::Pending(id);
            }
            // No upstream: 502 (offline / bridge off).
            let _ = (url, method, port); // silence unused on native-no-bridge path
            return HttpServeResult::Ready(
                HttpResponse {
                    status: 502,
                    reason: "Bad Gateway".into(),
                    content_type: "text/plain".into(),
                    body: b"upstream fetch failed".to_vec(),
                }
                .encode(),
            );
        }
    }
    // Local Host / empty: optional demo origin (LabwiredStats or empty).
    HttpServeResult::Ready(local.on_data(0, req))
}

/// Native: open TCP to host:port, write the raw request, read the response.
#[cfg(not(target_arch = "wasm32"))]
fn native_http_proxy_raw(req: &[u8], host: &str, port: u16) -> Option<Vec<u8>> {
    use std::io::{Read, Write};
    use std::net::TcpStream;
    use std::time::Duration;

    if !internet_enabled() {
        return None;
    }
    let addr = format!("{host}:{port}");
    let mut stream =
        TcpStream::connect_timeout(&addr.parse().ok()?, Duration::from_secs(5)).ok()?;
    let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
    let _ = stream.set_write_timeout(Some(Duration::from_secs(5)));
    stream.write_all(req).ok()?;
    let _ = stream.shutdown(std::net::Shutdown::Write);
    let mut buf = Vec::new();
    stream.read_to_end(&mut buf).ok()?;
    if buf.is_empty() {
        return None;
    }
    Some(buf)
}

/// Optional host-injected body (browser playground fetches the live API via
/// `fetch()` then calls [`set_public_stats_body`]). When set, the AP serves
/// this instead of attempting a native socket fetch. Cleared with
/// [`set_public_stats_body`]`(None)`.
fn public_stats_override() -> &'static Mutex<Option<Vec<u8>>> {
    static BODY: OnceLock<Mutex<Option<Vec<u8>>>> = OnceLock::new();
    BODY.get_or_init(|| Mutex::new(None))
}

/// Inject (or clear) the JSON body the virtual AP serves for
/// `GET /v1/public-stats`. Used by the browser playground so wasm can deliver
/// live API data without sockets; tests use it for determinism.
pub fn set_public_stats_body(body: Option<Vec<u8>>) {
    *public_stats_override().lock().unwrap() = body;
}

/// Snapshot of the current override (if any). Test helper.
#[cfg(test)]
pub fn public_stats_body_override() -> Option<Vec<u8>> {
    public_stats_override().lock().unwrap().clone()
}

/// Resolve the body the AP will serve for `/v1/public-stats`:
/// 1. host/test override (if set),
/// 2. live HTTP GET to `api.labwired.com` (native only),
/// 3. baked [`STATS_SNAPSHOT`] fallback.
pub fn resolve_public_stats_body() -> Vec<u8> {
    if let Some(body) = public_stats_override().lock().unwrap().clone() {
        return body;
    }
    if let Some(body) = fetch_live_public_stats() {
        return body;
    }
    STATS_SNAPSHOT.as_bytes().to_vec()
}

/// Native host: open a short TCP connection to the real LabWired API and return
/// the response body. Wasm always returns `None` (no sockets) — the playground
/// must inject via [`set_public_stats_body`].
#[cfg(not(target_arch = "wasm32"))]
fn fetch_live_public_stats() -> Option<Vec<u8>> {
    use std::io::{Read, Write};
    use std::net::TcpStream;
    use std::time::Duration;

    // Offline / hermetic CI: force the baked snapshot (no network).
    if std::env::var_os("LABWIRED_WIFI_STATS_OFFLINE").is_some() {
        return None;
    }

    let addr = format!("{PUBLIC_STATS_HOST}:80");
    let mut stream =
        TcpStream::connect_timeout(&addr.parse().ok()?, Duration::from_secs(3)).ok()?;
    let _ = stream.set_read_timeout(Some(Duration::from_secs(3)));
    let _ = stream.set_write_timeout(Some(Duration::from_secs(3)));

    let req = format!(
        "GET {PUBLIC_STATS_PATH} HTTP/1.1\r\n\
         Host: {PUBLIC_STATS_HOST}\r\n\
         Connection: close\r\n\
         Accept: application/json\r\n\
         User-Agent: labwired-virtual-ap/1\r\n\
         \r\n"
    );
    stream.write_all(req.as_bytes()).ok()?;

    let mut buf = Vec::new();
    stream.read_to_end(&mut buf).ok()?;
    parse_http_200_body(&buf)
}

#[cfg(target_arch = "wasm32")]
fn fetch_live_public_stats() -> Option<Vec<u8>> {
    None
}

/// Extract the body from a raw HTTP/1.x response. Requires status 200 and a
/// body that looks like our public-stats JSON.
#[cfg_attr(target_arch = "wasm32", allow(dead_code))] // used by native live fetch only
fn parse_http_200_body(raw: &[u8]) -> Option<Vec<u8>> {
    let sep = raw.windows(4).position(|w| w == b"\r\n\r\n")?;
    let head = std::str::from_utf8(&raw[..sep]).ok()?;
    let status_ok = head
        .lines()
        .next()
        .map(|line| line.contains(" 200"))
        .unwrap_or(false);
    if !status_ok {
        return None;
    }
    let body = raw[sep + 4..].to_vec();
    // Strip optional chunked/trailer noise: require the public-stats marker.
    if !body
        .windows(b"boards_supported".len())
        .any(|w| w == b"boards_supported")
    {
        return None;
    }
    Some(body)
}

/// HTTP origin that reverse-proxies `/v1/public-stats` to the live LabWired API
/// (or override / baked fallback). Implements [`SimServer`] so the AP's TCP
/// terminator stays unchanged.
#[derive(Debug, Default)]
struct LabwiredStatsServer {
    /// Per-AP cache: first request resolves live/override/baked; later requests
    /// reuse so a lab run doesn't thrash the origin on every poll.
    cached: Mutex<Option<Vec<u8>>>,
}

impl LabwiredStatsServer {
    fn body(&self) -> Vec<u8> {
        let mut guard = self.cached.lock().unwrap();
        if let Some(ref b) = *guard {
            return b.clone();
        }
        let body = resolve_public_stats_body();
        *guard = Some(body.clone());
        body
    }

    /// Test / inject helper: pre-fill the per-AP cache so the first request
    /// does not race on the process-global override.
    #[cfg(test)]
    fn with_cached_body(body: Vec<u8>) -> Self {
        Self {
            cached: Mutex::new(Some(body)),
        }
    }
}

impl SimServer for LabwiredStatsServer {
    fn on_data(&self, _conn: u32, data: &[u8]) -> Vec<u8> {
        let path_ok = HttpServer::parse_request_line(data)
            .map(|(method, path)| method == "GET" && path == PUBLIC_STATS_PATH)
            .unwrap_or(false);
        if path_ok {
            HttpResponse::json(self.body()).encode()
        } else {
            HttpResponse {
                status: 404,
                reason: "Not Found".into(),
                content_type: "text/plain".into(),
                body: b"not found".to_vec(),
            }
            .encode()
        }
    }
}

/// What the AP's HTTP origin serves. Keeps the AP's L4 surface a small, explicit
/// choice so a lab can host the live LabWired stats origin (the default demo)
/// or an empty origin (every path 404s).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ApServes {
    /// Reverse-proxy `GET /v1/public-stats` to the live LabWired API (with
    /// host-inject / baked fallback). Default demo for the LBC3.1 stats lab.
    #[default]
    LabwiredStats,
    /// Serve nothing; the HTTP origin has no routes (every request 404s).
    None,
}

impl ApServes {
    /// Parse a manifest `serves` string. Unknown values fall back to the default
    /// (`LabwiredStats`) so a typo degrades to the demo rather than a dead AP.
    pub fn parse(s: &str) -> Self {
        match s {
            "labwired-stats" | "stats" | "public-stats" => ApServes::LabwiredStats,
            "none" | "" => ApServes::None,
            _ => ApServes::LabwiredStats,
        }
    }
}

/// Per-lab AP configuration. The former process-global consts (`AP_SSID`,
/// `AP_IP`, and the stats origin) are the [`Default`], so an unconfigured AP is
/// byte-identical to the old hardcoded one; a manifest `wifi_ap` overrides SSID,
/// IP (its /24 is the DHCP pool), and what the HTTP origin serves. BSSID and
/// netmask stay const (the driver never keys on them).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApConfig {
    pub ssid: String,
    pub ip: [u8; 4],
    pub serves: ApServes,
}

impl Default for ApConfig {
    fn default() -> Self {
        Self {
            ssid: AP_SSID.to_string(),
            ip: AP_IP,
            serves: ApServes::LabwiredStats,
        }
    }
}

impl ApConfig {
    /// Build a config from optional parts, filling any missing field from
    /// [`Default`]. `serves` is parsed via [`ApServes::parse`].
    pub fn from_parts(ssid: Option<String>, ip: Option<[u8; 4]>, serves: Option<&str>) -> Self {
        let d = ApConfig::default();
        Self {
            ssid: ssid.unwrap_or(d.ssid),
            ip: ip.unwrap_or(d.ip),
            serves: serves.map(ApServes::parse).unwrap_or(d.serves),
        }
    }

    /// The AP's HTTP origin for this config. `LabwiredStats` reverse-proxies
    /// `/v1/public-stats` to the live LabWired API (override / baked fallback);
    /// `None` is an empty origin (every path 404s). One source of truth for the
    /// AP's L4 routing.
    fn http_origin(&self) -> Arc<dyn SimServer> {
        match self.serves {
            ApServes::LabwiredStats => Arc::new(LabwiredStatsServer::default()),
            ApServes::None => Arc::new(HttpServer::new()),
        }
    }
}

/// Minimal per-connection TCP state for the AP's HTTP server. Enough for lwIP's
/// `esp_http_client` to complete a short in-order GET (no reordering, no SACK).
#[derive(Debug, Default)]
struct TcpConn {
    /// Next sequence number expected from the station.
    rcv_nxt: u32,
    /// Next sequence number the AP will send.
    snd_nxt: u32,
    /// Whether the AP has already sent its FIN.
    fin_sent: bool,
    /// Request bytes accumulated until a full HTTP request head is seen.
    req: Vec<u8>,
    /// Browser/async reverse-proxy: host-net request id waiting for a response.
    proxy_pending: Option<u32>,
    /// Client IP captured at SYN (needed when completing a proxy later).
    client_ip: [u8; 4],
    /// Server port (usually 80) for replies.
    server_port: u16,
}

/// Per-station state the AP tracks.
#[derive(Debug, Default)]
struct StaState {
    /// Frames queued for delivery to this station's RX.
    inbox: VecDeque<Vec<u8>>,
    /// DHCP-assigned IPv4 (0.0.0.0 until offered).
    ip: [u8; 4],
    /// 802.11 sequence counter for AP→this-STA frames (receiver dedups by it).
    ap_seq: u16,
    /// Open TCP connections to the AP's HTTP port, keyed by the station's port.
    conns: HashMap<u16, TcpConn>,
}

#[derive(Debug)]
struct VirtualWifi {
    stas: HashMap<[u8; 6], StaState>,
    next_host: u8,
    /// Deterministic seed for TCP initial sequence numbers.
    next_isn: u32,
    /// This AP's identity + L4 config (SSID, IP, what it serves).
    cfg: ApConfig,
    /// The AP's HTTP origin. Reuses the L4 [`HttpServer`] router + HTTP/1.1
    /// encoder so the TCP layer only moves bytes — one source of truth for HTTP.
    /// Derived from `cfg.serves`.
    http: Arc<dyn SimServer>,
    /// NAT table: TCP connections to off-LAN destinations (real internet).
    egress: EgressTable,
}

impl VirtualWifi {
    /// Build an AP from an explicit config; its HTTP origin is derived from
    /// `cfg.serves`.
    fn with_config(cfg: ApConfig) -> Self {
        let http = cfg.http_origin();
        Self {
            stas: HashMap::new(),
            next_host: 0,
            next_isn: 0,
            cfg,
            http,
            egress: EgressTable::default(),
        }
    }
}

impl Default for VirtualWifi {
    fn default() -> Self {
        // Default cfg = the former hardcoded AP (SSID "labwired-ap",
        // 192.168.4.1, serving the public-stats snapshot).
        Self::with_config(ApConfig::default())
    }
}

/// A shared WiFi medium — the infrastructure AP plus every associated station.
/// WiFi MACs minted from the same bus associate to the same virtual AP, get
/// distinct DHCP leases, and route to each other; MACs on different buses are
/// fully isolated — the behaviour the former process-static `MEDIUM` could not
/// offer, so two WiFi labs (or two workers) can coexist. `Arc<Mutex<…>>` keeps
/// the MAC `Send` inside a `Machine` (native requires `MachineTrait: Send`); the
/// browser is single-threaded so it never contends.
#[derive(Debug, Clone, Default)]
pub struct VirtualWifiBus {
    inner: Arc<Mutex<VirtualWifi>>,
}

impl VirtualWifiBus {
    pub fn new() -> Self {
        Self::default()
    }

    /// Build a medium whose AP uses an explicit [`ApConfig`] (SSID, IP, what it
    /// serves). `new()`/`Default` use [`ApConfig::default`] — the former
    /// hardcoded AP — so existing behaviour is byte-identical.
    pub fn with_config(cfg: ApConfig) -> Self {
        Self {
            inner: Arc::new(Mutex::new(VirtualWifi::with_config(cfg))),
        }
    }

    fn with<R>(&self, f: impl FnOnce(&mut VirtualWifi) -> R) -> R {
        f(&mut self.inner.lock().unwrap())
    }

    /// Reset the medium (tests / fresh runs).
    pub fn reset(&self) {
        self.with(|m| {
            m.stas.clear();
            m.next_host = 0;
            m.egress.conns.clear();
        });
    }

    /// Submit a frame a station transmitted. The AP processes it: management →
    /// response to the sender; DHCP → DORA; ARP → reply (gateway or another STA's
    /// MAC); IP destined to another station → routed into that station's inbox.
    pub fn submit(&self, src_mac: [u8; 6], frame: &[u8]) {
        self.with(|m| m.handle_tx(src_mac, frame));
    }

    /// Drain frames the medium has queued for `mac` (delivered to its RX ring).
    pub fn take_inbox(&self, mac: [u8; 6]) -> Vec<Vec<u8>> {
        self.with(|m| {
            m.stas
                .get_mut(&mac)
                .map(|s| s.inbox.drain(..).collect())
                .unwrap_or_default()
        })
    }

    /// Queue a beacon for `mac` (the AP beacons so a scanning STA finds it).
    /// Called periodically by the MAC while not yet associated.
    pub fn queue_beacon(&self, mac: [u8; 6], channel: u8) {
        self.with(|m| {
            let frame = build_beacon(&m.cfg.ssid, channel);
            m.enqueue(mac, frame);
        });
    }

    /// Poll NAT sockets and inject any pending AP→STA TCP segments. Call once
    /// per WiFi MAC bus tick so real-internet downloads make progress.
    pub fn poll(&self) {
        self.with(|m| m.poll_egress());
    }
}

// --- Transitional process-global medium (browser back-compat) ----------------
//
// WiFi MACs built via `Esp32c3WifiMac::new()` share this one module-global bus.
// One wasm module = one worker = one lab, so this is byte-identical to the former
// `static MEDIUM`. The follow-up threads a per-lab-group `VirtualWifiBus` through
// the MAC's construction, after which this global is deleted.
fn default_wifi_bus() -> &'static VirtualWifiBus {
    static BUS: OnceLock<VirtualWifiBus> = OnceLock::new();
    BUS.get_or_init(VirtualWifiBus::new)
}

/// The process-global WiFi medium every `Esp32c3WifiMac::new()` binds to.
/// Transitional; prefer an explicitly owned [`VirtualWifiBus`].
pub fn default_medium() -> VirtualWifiBus {
    default_wifi_bus().clone()
}

/// Reset the process-global medium (the CLI bridge calls this for a fresh run).
/// Transitional; prefer [`VirtualWifiBus::reset`] on an owned bus.
pub fn reset() {
    default_wifi_bus().reset();
}

impl VirtualWifi {
    fn sta(&mut self, mac: [u8; 6]) -> &mut StaState {
        self.stas.entry(mac).or_default()
    }

    /// Assign (or look up) this station's DHCP IP.
    fn assign_ip(&mut self, mac: [u8; 6]) -> [u8; 4] {
        if let Some(s) = self.stas.get(&mac) {
            if s.ip != [0, 0, 0, 0] {
                return s.ip;
            }
        }
        let host = FIRST_HOST + self.next_host;
        self.next_host += 1;
        let ip = [self.cfg.ip[0], self.cfg.ip[1], self.cfg.ip[2], host];
        self.sta(mac).ip = ip;
        ip
    }

    /// Reverse lookup: which station owns this IP?
    fn mac_for_ip(&self, ip: [u8; 4]) -> Option<[u8; 6]> {
        self.stas
            .iter()
            .find(|(_, s)| s.ip == ip)
            .map(|(mac, _)| *mac)
    }

    /// Enqueue an AP→STA frame, stamping the per-STA 802.11 sequence number so
    /// the receiver does not drop it as a retransmission.
    fn enqueue(&mut self, mac: [u8; 6], mut frame: Vec<u8>) {
        if frame.len() >= 24 {
            let s = self.sta(mac);
            let sc = (s.ap_seq & 0xFFF) << 4;
            frame[22] = sc as u8;
            frame[23] = (sc >> 8) as u8;
            s.ap_seq = s.ap_seq.wrapping_add(1);
        }
        self.sta(mac).inbox.push_back(frame);
    }

    fn handle_tx(&mut self, src: [u8; 6], frame: &[u8]) {
        if frame.len() < 2 {
            return;
        }
        let ftype = (frame[0] >> 2) & 3;
        let subtype = frame[0] >> 4;
        if ftype == 0 {
            // Management: respond to the transmitting station.
            match subtype {
                0x4 => self.enqueue(src, build_probe_resp(&self.cfg.ssid, src, 1)),
                0xB => self.enqueue(src, build_auth_resp(src)),
                0x0 | 0x2 => self.enqueue(src, build_assoc_resp(src)),
                _ => {}
            }
            return;
        }
        if ftype != 2 {
            return; // control frames: ignored
        }
        // Data frame.
        if let Some((xid, mtype)) = parse_dhcp(frame) {
            let ip = self.assign_ip(src);
            let reply = build_dhcp_reply(self.cfg.ip, src, ip, xid, mtype == 3);
            self.enqueue(src, reply);
            return;
        }
        if let Some((oper, _spa, tpa)) = parse_arp(frame) {
            // Answer ARP requests, EXCEPT the DHCP CHECKING self-probe (target ==
            // sender's own offered IP) which must time out to let the bind
            // complete. Resolve the gateway to the AP, and another station's IP
            // to that station's MAC (so unicast STA↔STA routing works).
            if oper == 1 {
                let own = self.stas.get(&src).map(|s| s.ip).unwrap_or_default();
                if tpa != own {
                    let who = if tpa == self.cfg.ip {
                        AP_MAC_L2
                    } else if let Some(m) = self.mac_for_ip(tpa) {
                        m
                    } else {
                        return;
                    };
                    let reply = build_arp_reply(src, who, tpa, own);
                    self.enqueue(src, reply);
                }
            }
            return;
        }
        // IPv4: AP services, STA↔STA route, or internet NAT.
        if let Some((dst_ip, proto)) = parse_ipv4_dst(frame) {
            if dst_ip == self.cfg.ip {
                // Local AP services: HTTP origin (TCP/80), DNS (UDP/53), UDP echo.
                if proto == 6 {
                    self.handle_tcp(src, frame);
                } else if proto == 17 {
                    if let Some(dns) = build_dns_reply(self.cfg.ip, src, frame) {
                        self.enqueue(src, dns);
                    } else if let Some(echo) = build_udp_echo(self.cfg.ip, src, frame) {
                        self.enqueue(src, echo);
                    }
                }
                return;
            }
            if let Some(dst_mac) = self.mac_for_ip(dst_ip) {
                // Re-frame as a from-DS data frame to the destination station,
                // preserving the LLC/SNAP + IP payload.
                if let Some(routed) = reframe_to_sta(frame, dst_mac) {
                    self.enqueue(dst_mac, routed);
                }
                return;
            }
            // Off-LAN: NAT through the host network (when internet is enabled).
            if internet_enabled() && proto == 6 {
                self.handle_tcp_egress(src, frame, dst_ip);
            }
        }
    }

    /// Terminate a TCP connection from a station to the AP's HTTP port: drive the
    /// handshake, reassemble the HTTP request, hand it to the L4 HTTP server, and
    /// stream the response back, then FIN. Minimal and in-order — enough for
    /// lwIP's `esp_http_client` to complete a short GET. The station runs the
    /// real TCP stack; the AP is the peer that terminates it (no thunks).
    fn handle_tcp(&mut self, src_mac: [u8; 6], frame: &[u8]) {
        let ap_ip = self.cfg.ip;
        let ipoff = snap_off(frame);
        if frame.len() < ipoff + 20 {
            return;
        }
        let ihl = (frame[ipoff] & 0x0F) as usize * 4;
        let total_len = u16::from_be_bytes([frame[ipoff + 2], frame[ipoff + 3]]) as usize;
        let mut src_ip = [0u8; 4];
        src_ip.copy_from_slice(&frame[ipoff + 12..ipoff + 16]);
        let tcpoff = ipoff + ihl;
        let seg_end = (ipoff + total_len).min(frame.len());
        if ihl < 20 || tcpoff + 20 > seg_end {
            return;
        }
        let seg = &frame[tcpoff..seg_end];
        let client_port = u16::from_be_bytes([seg[0], seg[1]]);
        let server_port = u16::from_be_bytes([seg[2], seg[3]]);
        let seq = u32::from_be_bytes([seg[4], seg[5], seg[6], seg[7]]);
        let data_off = (seg[12] >> 4) as usize * 4;
        if data_off < 20 || data_off > seg.len() {
            return;
        }
        let flags = seg[13];
        let payload = seg[data_off..].to_vec();

        let dbg = std::env::var("LABWIRED_TCP_DEBUG").is_ok();
        if dbg {
            eprintln!(
                "[tcp] rx sport={client_port} dport={server_port} flags={flags:#04x} seq={seq} paylen={}",
                payload.len()
            );
        }

        // RST tears the connection down.
        if flags & TCP_RST != 0 {
            if let Some(s) = self.stas.get_mut(&src_mac) {
                s.conns.remove(&client_port);
            }
            return;
        }

        // SYN (no ACK) opens a connection; reply SYN-ACK.
        if flags & TCP_SYN != 0 && flags & TCP_ACK == 0 {
            let isn = 0x0001_0000u32.wrapping_add(self.next_isn);
            self.next_isn = self.next_isn.wrapping_add(0x0001_0000);
            let rcv_nxt = seq.wrapping_add(1);
            *self.conn_mut(src_mac, client_port) = TcpConn {
                rcv_nxt,
                snd_nxt: isn.wrapping_add(1),
                fin_sent: false,
                req: Vec::new(),
                proxy_pending: None,
                client_ip: src_ip,
                server_port,
            };
            let synack = build_tcp_to_sta(
                ap_ip,
                src_mac,
                src_ip,
                server_port,
                client_port,
                isn,
                rcv_nxt,
                TCP_SYN | TCP_ACK,
                &[],
            );
            if dbg {
                eprintln!("[tcp] SYN -> SYN-ACK isn={isn} ack={rcv_nxt}");
            }
            self.enqueue(src_mac, synack);
            return;
        }

        // Established: process in-order data (→ HTTP response) and FIN.
        let http = Arc::clone(&self.http);
        let mut out: Vec<Vec<u8>> = Vec::new();
        let mut close = false;
        {
            let conn = match self
                .stas
                .get_mut(&src_mac)
                .and_then(|s| s.conns.get_mut(&client_port))
            {
                Some(c) => c,
                None => {
                    if dbg {
                        eprintln!("[tcp] no conn for port {client_port} (seq={seq})");
                    }
                    return;
                }
            };
            if dbg {
                eprintln!(
                    "[tcp] conn port={client_port} rcv_nxt={} snd_nxt={} (seq={seq} paylen={})",
                    conn.rcv_nxt,
                    conn.snd_nxt,
                    payload.len()
                );
            }
            if !payload.is_empty() && seq == conn.rcv_nxt {
                conn.req.extend_from_slice(&payload);
                conn.rcv_nxt = conn.rcv_nxt.wrapping_add(payload.len() as u32);
                if dbg {
                    eprintln!(
                        "[tcp] data += {} (req now {} bytes, complete={})",
                        payload.len(),
                        conn.req.len(),
                        http_req_complete(&conn.req)
                    );
                }
                if http_req_complete(&conn.req) && !conn.fin_sent && conn.proxy_pending.is_none() {
                    // Universal HTTP reverse proxy on the AP:
                    //   Host: example.com  → fetch any origin on the internet
                    //   Host: empty / AP IP + /v1/public-stats → local demo origin
                    match serve_or_proxy_http(&http, &conn.req) {
                        HttpServeResult::Ready(resp) => {
                            if dbg {
                                eprintln!("[tcp] -> HTTP response {} bytes + FIN", resp.len());
                            }
                            out.push(build_tcp_to_sta(
                                ap_ip,
                                src_mac,
                                src_ip,
                                server_port,
                                client_port,
                                conn.snd_nxt,
                                conn.rcv_nxt,
                                TCP_PSH | TCP_ACK,
                                &resp,
                            ));
                            conn.snd_nxt = conn.snd_nxt.wrapping_add(resp.len() as u32);
                            out.push(build_tcp_to_sta(
                                ap_ip,
                                src_mac,
                                src_ip,
                                server_port,
                                client_port,
                                conn.snd_nxt,
                                conn.rcv_nxt,
                                TCP_FIN | TCP_ACK,
                                &[],
                            ));
                            conn.snd_nxt = conn.snd_nxt.wrapping_add(1);
                            conn.fin_sent = true;
                        }
                        HttpServeResult::Pending(id) => {
                            // Browser host-net will fulfill; poll_egress finishes.
                            conn.proxy_pending = Some(id);
                            conn.client_ip = src_ip;
                            conn.server_port = server_port;
                            out.push(build_tcp_to_sta(
                                ap_ip,
                                src_mac,
                                src_ip,
                                server_port,
                                client_port,
                                conn.snd_nxt,
                                conn.rcv_nxt,
                                TCP_ACK,
                                &[],
                            ));
                        }
                    }
                } else if !http_req_complete(&conn.req) && !conn.fin_sent {
                    // Partial request: acknowledge what we have so far.
                    out.push(build_tcp_to_sta(
                        ap_ip,
                        src_mac,
                        src_ip,
                        server_port,
                        client_port,
                        conn.snd_nxt,
                        conn.rcv_nxt,
                        TCP_ACK,
                        &[],
                    ));
                }
            }
            // A station FIN consumes one sequence number; acknowledge it. Once our
            // own FIN has also been sent, the connection is fully closed.
            if flags & TCP_FIN != 0 && seq.wrapping_add(payload.len() as u32) == conn.rcv_nxt {
                conn.rcv_nxt = conn.rcv_nxt.wrapping_add(1);
                out.push(build_tcp_to_sta(
                    ap_ip,
                    src_mac,
                    src_ip,
                    server_port,
                    client_port,
                    conn.snd_nxt,
                    conn.rcv_nxt,
                    TCP_ACK,
                    &[],
                ));
                if conn.fin_sent {
                    close = true;
                }
            }
        }
        for f in out {
            self.enqueue(src_mac, f);
        }
        if close {
            if let Some(s) = self.stas.get_mut(&src_mac) {
                s.conns.remove(&client_port);
            }
        }
    }

    fn conn_mut(&mut self, mac: [u8; 6], port: u16) -> &mut TcpConn {
        self.stas
            .entry(mac)
            .or_default()
            .conns
            .entry(port)
            .or_default()
    }

    /// TCP NAT: station talks to a real off-LAN peer via a host `TcpStream`.
    /// Replies are sourced from the remote IP so the station's lwIP stack sees
    /// a normal internet path (gateway ARPs the AP; TCP peer is remote).
    fn handle_tcp_egress(&mut self, src_mac: [u8; 6], frame: &[u8], remote_ip: [u8; 4]) {
        let ipoff = snap_off(frame);
        if frame.len() < ipoff + 20 {
            return;
        }
        let ihl = (frame[ipoff] & 0x0F) as usize * 4;
        let total_len = u16::from_be_bytes([frame[ipoff + 2], frame[ipoff + 3]]) as usize;
        let mut client_ip = [0u8; 4];
        client_ip.copy_from_slice(&frame[ipoff + 12..ipoff + 16]);
        let tcpoff = ipoff + ihl;
        let seg_end = (ipoff + total_len).min(frame.len());
        if ihl < 20 || tcpoff + 20 > seg_end {
            return;
        }
        let seg = &frame[tcpoff..seg_end];
        let client_port = u16::from_be_bytes([seg[0], seg[1]]);
        let remote_port = u16::from_be_bytes([seg[2], seg[3]]);
        let seq = u32::from_be_bytes([seg[4], seg[5], seg[6], seg[7]]);
        let data_off = (seg[12] >> 4) as usize * 4;
        if data_off < 20 || data_off > seg.len() {
            return;
        }
        let flags = seg[13];
        let payload = &seg[data_off..];
        let key = (src_mac, client_port);

        if flags & TCP_RST != 0 {
            self.egress.conns.remove(&key);
            return;
        }

        // SYN → open real socket + SYN-ACK (or RST if connect fails).
        if flags & TCP_SYN != 0 && flags & TCP_ACK == 0 {
            let isn = 0x0002_0000u32.wrapping_add(self.next_isn);
            self.next_isn = self.next_isn.wrapping_add(0x0001_0000);
            let rcv_nxt = seq.wrapping_add(1);
            let snd_nxt = isn.wrapping_add(1);
            match EgressTcp::connect(client_ip, remote_ip, remote_port, rcv_nxt, snd_nxt) {
                Some(conn) => {
                    self.egress.conns.insert(key, conn);
                    let synack = build_tcp_to_sta(
                        remote_ip,
                        src_mac,
                        client_ip,
                        remote_port,
                        client_port,
                        isn,
                        rcv_nxt,
                        TCP_SYN | TCP_ACK,
                        &[],
                    );
                    self.enqueue(src_mac, synack);
                }
                None => {
                    let rst = build_tcp_to_sta(
                        remote_ip,
                        src_mac,
                        client_ip,
                        remote_port,
                        client_port,
                        0,
                        seq.wrapping_add(1),
                        TCP_RST | TCP_ACK,
                        &[],
                    );
                    self.enqueue(src_mac, rst);
                }
            }
            return;
        }

        let Some(conn) = self.egress.conns.get_mut(&key) else {
            return;
        };
        let mut out: Vec<Vec<u8>> = Vec::new();
        let mut close = false;

        if !payload.is_empty() && seq == conn.rcv_nxt {
            let _ = conn.write_all(payload);
            conn.rcv_nxt = conn.rcv_nxt.wrapping_add(payload.len() as u32);
            out.push(build_tcp_to_sta(
                remote_ip,
                src_mac,
                client_ip,
                remote_port,
                client_port,
                conn.snd_nxt,
                conn.rcv_nxt,
                TCP_ACK,
                &[],
            ));
        }

        if flags & TCP_FIN != 0 && seq.wrapping_add(payload.len() as u32) == conn.rcv_nxt {
            conn.rcv_nxt = conn.rcv_nxt.wrapping_add(1);
            conn.shutdown_write();
            out.push(build_tcp_to_sta(
                remote_ip,
                src_mac,
                client_ip,
                remote_port,
                client_port,
                conn.snd_nxt,
                conn.rcv_nxt,
                TCP_ACK,
                &[],
            ));
            if conn.fin_sent {
                close = true;
            }
        }

        // Pull any immediately available remote data (short responses).
        let data = conn.read_available(4096);
        if !data.is_empty() {
            out.push(build_tcp_to_sta(
                remote_ip,
                src_mac,
                client_ip,
                remote_port,
                client_port,
                conn.snd_nxt,
                conn.rcv_nxt,
                TCP_PSH | TCP_ACK,
                &data,
            ));
            conn.snd_nxt = conn.snd_nxt.wrapping_add(data.len() as u32);
        }
        if conn.peer_fin && !conn.fin_sent {
            out.push(build_tcp_to_sta(
                remote_ip,
                src_mac,
                client_ip,
                remote_port,
                client_port,
                conn.snd_nxt,
                conn.rcv_nxt,
                TCP_FIN | TCP_ACK,
                &[],
            ));
            conn.snd_nxt = conn.snd_nxt.wrapping_add(1);
            conn.fin_sent = true;
            close = true;
        }

        for f in out {
            self.enqueue(src_mac, f);
        }
        if close {
            self.egress.conns.remove(&key);
        }
    }

    /// Drain NAT sockets / browser host-net answers → station inboxes.
    fn poll_egress(&mut self) {
        // DNS answers fulfilled by the browser host bridge.
        for rep in crate::peripherals::esp32c3::virtual_wifi_host_net::take_dns_replies() {
            let frame = build_udp_to_sta(
                rep.ap_ip,
                rep.sta_mac,
                rep.client_ip,
                53,
                rep.client_port,
                &rep.udp_payload,
            );
            self.enqueue(rep.sta_mac, frame);
        }

        // Local AP:80 reverse-proxy (any Host) pending browser fetches.
        let ap_ip = self.cfg.ip;
        let mut proxy_done: Vec<([u8; 6], u16, Vec<u8>)> = Vec::new();
        for (mac, sta) in self.stas.iter_mut() {
            for (port, conn) in sta.conns.iter_mut() {
                if let Some(id) = conn.proxy_pending {
                    if let Some(resp) =
                        crate::peripherals::esp32c3::virtual_wifi_host_net::take_http_answer(id)
                    {
                        conn.proxy_pending = None;
                        proxy_done.push((*mac, *port, resp));
                    }
                }
            }
        }
        for (mac, client_port, resp) in proxy_done {
            if let Some(conn) = self
                .stas
                .get_mut(&mac)
                .and_then(|s| s.conns.get_mut(&client_port))
            {
                if conn.fin_sent {
                    continue;
                }
                let client_ip = conn.client_ip;
                let server_port = conn.server_port;
                let data = build_tcp_to_sta(
                    ap_ip,
                    mac,
                    client_ip,
                    server_port,
                    client_port,
                    conn.snd_nxt,
                    conn.rcv_nxt,
                    TCP_PSH | TCP_ACK,
                    &resp,
                );
                conn.snd_nxt = conn.snd_nxt.wrapping_add(resp.len() as u32);
                let fin = build_tcp_to_sta(
                    ap_ip,
                    mac,
                    client_ip,
                    server_port,
                    client_port,
                    conn.snd_nxt,
                    conn.rcv_nxt,
                    TCP_FIN | TCP_ACK,
                    &[],
                );
                conn.snd_nxt = conn.snd_nxt.wrapping_add(1);
                conn.fin_sent = true;
                self.enqueue(mac, data);
                self.enqueue(mac, fin);
            }
        }

        if self.egress.conns.is_empty() {
            return;
        }
        let keys: Vec<_> = self.egress.conns.keys().copied().collect();
        for key in keys {
            let (src_mac, client_port) = key;
            let Some(conn) = self.egress.conns.get_mut(&key) else {
                continue;
            };
            let remote_ip = conn.remote_ip;
            let remote_port = conn.remote_port;
            let client_ip = conn.client_ip;
            let data = conn.read_available(4096);
            let mut frames = Vec::new();
            if !data.is_empty() {
                let seq = conn.snd_nxt;
                let ack = conn.rcv_nxt;
                conn.snd_nxt = conn.snd_nxt.wrapping_add(data.len() as u32);
                frames.push(build_tcp_to_sta(
                    remote_ip,
                    src_mac,
                    client_ip,
                    remote_port,
                    client_port,
                    seq,
                    ack,
                    TCP_PSH | TCP_ACK,
                    &data,
                ));
            }
            if conn.peer_fin && !conn.fin_sent {
                let seq = conn.snd_nxt;
                let ack = conn.rcv_nxt;
                conn.snd_nxt = conn.snd_nxt.wrapping_add(1);
                conn.fin_sent = true;
                frames.push(build_tcp_to_sta(
                    remote_ip,
                    src_mac,
                    client_ip,
                    remote_port,
                    client_port,
                    seq,
                    ack,
                    TCP_FIN | TCP_ACK,
                    &[],
                ));
            }
            let drop = conn.fin_sent && conn.peer_fin;
            for f in frames {
                self.enqueue(src_mac, f);
            }
            if drop {
                self.egress.conns.remove(&key);
            }
        }
    }
}

// ─────────────────────────── frame parsing ───────────────────────────

/// Offset of the LLC/SNAP header in a data frame (24, or 26 with QoS).
fn snap_off(frame: &[u8]) -> usize {
    (if frame[0] & 0x80 != 0 { 26 } else { 24 }) + 8
}

/// (xid, dhcp-message-type) if this is a DHCP client→server datagram.
fn parse_dhcp(frame: &[u8]) -> Option<([u8; 4], u8)> {
    if (frame[0] >> 2) & 3 != 2 {
        return None;
    }
    let ip = snap_off(frame);
    if frame.len() < ip + 20 || frame[ip] >> 4 != 4 || frame[ip + 9] != 17 {
        return None;
    }
    let udp = ip + (frame[ip] & 0xF) as usize * 4;
    if frame.len() < udp + 8 || u16::from_be_bytes([frame[udp + 2], frame[udp + 3]]) != 67 {
        return None;
    }
    let dhcp = udp + 8;
    if frame.len() < dhcp + 240 {
        return None;
    }
    let xid = [
        frame[dhcp + 4],
        frame[dhcp + 5],
        frame[dhcp + 6],
        frame[dhcp + 7],
    ];
    let mut o = dhcp + 240;
    let mut mt = 0u8;
    while o + 1 < frame.len() {
        match frame[o] {
            255 => break,
            0 => {
                o += 1;
                continue;
            }
            53 if frame[o + 1] >= 1 => mt = frame[o + 2],
            _ => {}
        }
        o += 2 + frame[o + 1] as usize;
    }
    Some((xid, mt))
}

/// (oper, sender-ip, target-ip) if this data frame carries ARP.
fn parse_arp(frame: &[u8]) -> Option<(u16, [u8; 4], [u8; 4])> {
    if (frame[0] >> 2) & 3 != 2 {
        return None;
    }
    let snap = snap_off(frame);
    if frame.len() < snap + 28 || u16::from_be_bytes([frame[snap - 2], frame[snap - 1]]) != 0x0806 {
        return None;
    }
    let oper = u16::from_be_bytes([frame[snap + 6], frame[snap + 7]]);
    let mut spa = [0u8; 4];
    spa.copy_from_slice(&frame[snap + 14..snap + 18]);
    let mut tpa = [0u8; 4];
    tpa.copy_from_slice(&frame[snap + 24..snap + 28]);
    Some((oper, spa, tpa))
}

/// (dst-ip, proto) if this data frame carries IPv4.
fn parse_ipv4_dst(frame: &[u8]) -> Option<([u8; 4], u8)> {
    if (frame[0] >> 2) & 3 != 2 {
        return None;
    }
    let ip = snap_off(frame);
    if frame.len() < ip + 20 || frame[ip] >> 4 != 4 {
        return None;
    }
    let mut dst = [0u8; 4];
    dst.copy_from_slice(&frame[ip + 16..ip + 20]);
    Some((dst, frame[ip + 9]))
}

// ─────────────────────────── frame builders ───────────────────────────

fn mgmt_hdr(subtype_fc0: u8, da: [u8; 6]) -> Vec<u8> {
    let mut f = Vec::new();
    f.extend_from_slice(&[subtype_fc0, 0x00, 0x00, 0x00]); // FC + duration
    f.extend_from_slice(&da); // addr1 = DA (the STA)
    f.extend_from_slice(&AP_BSSID); // addr2 = SA (AP)
    f.extend_from_slice(&AP_BSSID); // addr3 = BSSID
    f.extend_from_slice(&[0x00, 0x00]); // seq/frag
    f
}

fn build_beacon(ssid: &str, channel: u8) -> Vec<u8> {
    let mut f = mgmt_hdr(0x80, [0xFF; 6]);
    f.extend_from_slice(&[0u8; 8]); // timestamp
    f.extend_from_slice(&[0x64, 0x00]); // beacon interval
    f.extend_from_slice(&[0x01, 0x00]); // capability: ESS, OPEN
    f.push(0x00);
    f.push(ssid.len() as u8);
    f.extend_from_slice(ssid.as_bytes());
    f.extend_from_slice(&[0x01, 0x08, 0x82, 0x84, 0x8b, 0x96, 0x0c, 0x12, 0x18, 0x24]);
    f.extend_from_slice(&[0x03, 0x01, channel]);
    f
}

fn build_probe_resp(ssid: &str, da: [u8; 6], channel: u8) -> Vec<u8> {
    let mut f = build_beacon(ssid, channel);
    f[0] = 0x50; // probe response
    f[4..10].copy_from_slice(&da);
    f
}

fn build_auth_resp(da: [u8; 6]) -> Vec<u8> {
    let mut f = mgmt_hdr(0xB0, da);
    f.extend_from_slice(&[0x00, 0x00, 0x02, 0x00, 0x00, 0x00]); // open, seq2, success
    f
}

fn build_assoc_resp(da: [u8; 6]) -> Vec<u8> {
    let mut f = mgmt_hdr(0x10, da);
    f.extend_from_slice(&[0x01, 0x00, 0x00, 0x00, 0x01, 0xC0]); // cap, status0, AID1
    f.extend_from_slice(&[0x01, 0x08, 0x82, 0x84, 0x8b, 0x96, 0x0c, 0x12, 0x18, 0x24]);
    f
}

fn inet_checksum(data: &[u8]) -> u16 {
    let mut sum = 0u32;
    let mut i = 0;
    while i + 1 < data.len() {
        sum += u16::from_be_bytes([data[i], data[i + 1]]) as u32;
        i += 2;
    }
    if i < data.len() {
        sum += (data[i] as u32) << 8;
    }
    while sum >> 16 != 0 {
        sum = (sum & 0xFFFF) + (sum >> 16);
    }
    !(sum as u16)
}

/// AP→STA from-DS data frame wrapping an IPv4 (or ARP) payload.
fn data_frame(da: [u8; 6], ethertype: u16, l3: &[u8]) -> Vec<u8> {
    let mut f = Vec::new();
    f.extend_from_slice(&[0x08, 0x02, 0x00, 0x00]); // data, from-DS, duration
    f.extend_from_slice(&da); // addr1 = DA (STA)
    f.extend_from_slice(&AP_BSSID); // addr2 = BSSID
    f.extend_from_slice(&AP_BSSID); // addr3 = SA
    f.extend_from_slice(&[0x00, 0x00]); // seq/frag
    f.extend_from_slice(&[0xAA, 0xAA, 0x03, 0x00, 0x00, 0x00]);
    f.extend_from_slice(&ethertype.to_be_bytes());
    f.extend_from_slice(l3);
    f
}

fn build_dhcp_reply(
    ap_ip: [u8; 4],
    da: [u8; 6],
    yiaddr: [u8; 4],
    xid: [u8; 4],
    ack: bool,
) -> Vec<u8> {
    let mut dhcp = vec![0x02, 0x01, 0x06, 0x00];
    dhcp.extend_from_slice(&xid);
    dhcp.extend_from_slice(&[0x00, 0x00, 0x80, 0x00]); // secs, broadcast flag
    dhcp.extend_from_slice(&[0, 0, 0, 0]); // ciaddr
    dhcp.extend_from_slice(&yiaddr);
    dhcp.extend_from_slice(&ap_ip); // siaddr
    dhcp.extend_from_slice(&[0, 0, 0, 0]); // giaddr
    dhcp.extend_from_slice(&da); // chaddr (6)
    dhcp.extend_from_slice(&[0u8; 10]);
    dhcp.extend_from_slice(&[0u8; 64]); // sname
    dhcp.extend_from_slice(&[0u8; 128]); // file
    dhcp.extend_from_slice(&[0x63, 0x82, 0x53, 0x63]); // magic
    dhcp.extend_from_slice(&[53, 1, if ack { 5 } else { 2 }]);
    dhcp.extend_from_slice(&[54, 4, ap_ip[0], ap_ip[1], ap_ip[2], ap_ip[3]]);
    dhcp.extend_from_slice(&[51, 4, 0x00, 0x01, 0x51, 0x80]); // lease
    dhcp.extend_from_slice(&[1, 4, NETMASK[0], NETMASK[1], NETMASK[2], NETMASK[3]]);
    dhcp.extend_from_slice(&[3, 4, ap_ip[0], ap_ip[1], ap_ip[2], ap_ip[3]]);
    dhcp.extend_from_slice(&[6, 4, ap_ip[0], ap_ip[1], ap_ip[2], ap_ip[3]]);
    dhcp.push(255);

    let udp_len = (8 + dhcp.len()) as u16;
    let mut udp = Vec::new();
    udp.extend_from_slice(&67u16.to_be_bytes());
    udp.extend_from_slice(&68u16.to_be_bytes());
    udp.extend_from_slice(&udp_len.to_be_bytes());
    udp.extend_from_slice(&[0, 0]);
    udp.extend_from_slice(&dhcp);

    let ip_total = (20 + udp.len()) as u16;
    let mut ip = vec![
        0x45,
        0x00,
        (ip_total >> 8) as u8,
        ip_total as u8,
        0,
        0,
        0,
        0,
        0x40,
        0x11,
        0,
        0,
    ];
    ip.extend_from_slice(&ap_ip);
    ip.extend_from_slice(&[255, 255, 255, 255]);
    let cks = inet_checksum(&ip);
    ip[10] = (cks >> 8) as u8;
    ip[11] = cks as u8;
    ip.extend_from_slice(&udp);
    data_frame(da, 0x0800, &ip)
}

fn build_arp_reply(da: [u8; 6], who_mac: [u8; 6], who_ip: [u8; 4], target_ip: [u8; 4]) -> Vec<u8> {
    let mut arp = Vec::new();
    arp.extend_from_slice(&[0x00, 0x01, 0x08, 0x00, 0x06, 0x04, 0x00, 0x02]);
    arp.extend_from_slice(&who_mac);
    arp.extend_from_slice(&who_ip);
    arp.extend_from_slice(&da);
    arp.extend_from_slice(&target_ip);
    data_frame(da, 0x0806, &arp)
}

/// If `frame` is a DNS query (UDP/53) to the AP, resolve via the host and
/// return a from-DS reply frame. DHCP already advertises the AP as DNS.
/// On browser (host-net bridge), queues async DNS and returns `None` until
/// [`VirtualWifi::poll_egress`] injects the reply.
fn build_dns_reply(ap_ip: [u8; 4], da: [u8; 6], frame: &[u8]) -> Option<Vec<u8>> {
    let ip = snap_off(frame);
    if frame.len() < ip + 20 || frame[ip] >> 4 != 4 || frame[ip + 9] != 17 {
        return None;
    }
    let udp = ip + (frame[ip] & 0xF) as usize * 4;
    if frame.len() < udp + 8 {
        return None;
    }
    let sport = u16::from_be_bytes([frame[udp], frame[udp + 1]]);
    let dport = u16::from_be_bytes([frame[udp + 2], frame[udp + 3]]);
    if dport != 53 {
        return None;
    }
    let ulen = u16::from_be_bytes([frame[udp + 4], frame[udp + 5]]) as usize;
    if ulen < 8 || frame.len() < udp + ulen {
        return None;
    }
    let mut src_ip = [0u8; 4];
    src_ip.copy_from_slice(&frame[ip + 12..ip + 16]);
    let q = &frame[udp + 8..udp + ulen];
    // Browser host-net: never use native resolver (none in wasm) — queue DoH.
    if crate::peripherals::esp32c3::virtual_wifi_host_net::bridge_active() {
        if let Some(name) = crate::peripherals::esp32c3::virtual_wifi_host_net::dns_qname(q) {
            crate::peripherals::esp32c3::virtual_wifi_host_net::enqueue_dns(
                name,
                q.to_vec(),
                da,
                src_ip,
                ap_ip,
                sport,
            );
        }
        return None;
    }
    // Native host resolver (instant).
    let resp = dns_respond(q)?;
    Some(build_udp_to_sta(ap_ip, da, src_ip, 53, sport, &resp))
}

/// Build a from-DS IPv4/UDP frame AP→STA.
fn build_udp_to_sta(
    src_ip: [u8; 4],
    da: [u8; 6],
    dst_ip: [u8; 4],
    sport: u16,
    dport: u16,
    payload: &[u8],
) -> Vec<u8> {
    let udp_len = (8 + payload.len()) as u16;
    let mut u = Vec::new();
    u.extend_from_slice(&sport.to_be_bytes());
    u.extend_from_slice(&dport.to_be_bytes());
    u.extend_from_slice(&udp_len.to_be_bytes());
    u.extend_from_slice(&[0, 0]); // checksum optional for IPv4
    u.extend_from_slice(payload);

    let ip_total = (20 + u.len()) as u16;
    let mut iph = vec![
        0x45,
        0x00,
        (ip_total >> 8) as u8,
        ip_total as u8,
        0,
        0,
        0,
        0,
        0x40,
        0x11,
        0,
        0,
    ];
    iph.extend_from_slice(&src_ip);
    iph.extend_from_slice(&dst_ip);
    let cks = inet_checksum(&iph);
    iph[10] = (cks >> 8) as u8;
    iph[11] = cks as u8;
    iph.extend_from_slice(&u);
    data_frame(da, 0x0800, &iph)
}

/// If `frame` is a UDP datagram to the AP's echo port, build the echoed reply
/// (same payload, src/dst swapped) as a from-DS data frame to the sender `da`.
fn build_udp_echo(ap_ip: [u8; 4], da: [u8; 6], frame: &[u8]) -> Option<Vec<u8>> {
    let ip = snap_off(frame);
    if frame.len() < ip + 20 || frame[ip] >> 4 != 4 || frame[ip + 9] != 17 {
        return None;
    }
    let udp = ip + (frame[ip] & 0xF) as usize * 4;
    if frame.len() < udp + 8 {
        return None;
    }
    let sport = u16::from_be_bytes([frame[udp], frame[udp + 1]]);
    let dport = u16::from_be_bytes([frame[udp + 2], frame[udp + 3]]);
    if dport != UDP_ECHO_PORT {
        return None;
    }
    let ulen = u16::from_be_bytes([frame[udp + 4], frame[udp + 5]]) as usize;
    if ulen < 8 || frame.len() < udp + ulen {
        return None;
    }
    // Sender's source IP (reply destination).
    let mut src_ip = [0u8; 4];
    src_ip.copy_from_slice(&frame[ip + 12..ip + 16]);
    let payload = &frame[udp + 8..udp + ulen];

    let udp_len = (8 + payload.len()) as u16;
    let mut u = Vec::new();
    u.extend_from_slice(&UDP_ECHO_PORT.to_be_bytes());
    u.extend_from_slice(&sport.to_be_bytes());
    u.extend_from_slice(&udp_len.to_be_bytes());
    u.extend_from_slice(&[0, 0]);
    u.extend_from_slice(payload);

    let ip_total = (20 + u.len()) as u16;
    let mut iph = vec![
        0x45,
        0x00,
        (ip_total >> 8) as u8,
        ip_total as u8,
        0,
        0,
        0,
        0,
        0x40,
        0x11,
        0,
        0,
    ];
    iph.extend_from_slice(&ap_ip);
    iph.extend_from_slice(&src_ip);
    let cks = inet_checksum(&iph);
    iph[10] = (cks >> 8) as u8;
    iph[11] = cks as u8;
    iph.extend_from_slice(&u);
    Some(data_frame(da, 0x0800, &iph))
}

/// Build an AP→STA from-DS data frame carrying a TCP segment over IPv4 (both
/// IPv4 and TCP checksums computed) from the AP's HTTP port to the station.
#[allow(clippy::too_many_arguments)]
fn build_tcp_to_sta(
    ap_ip: [u8; 4],
    da: [u8; 6],
    client_ip: [u8; 4],
    sport: u16,
    dport: u16,
    seq: u32,
    ack: u32,
    flags: u8,
    payload: &[u8],
) -> Vec<u8> {
    let mut tcp = Vec::with_capacity(20 + payload.len());
    tcp.extend_from_slice(&sport.to_be_bytes());
    tcp.extend_from_slice(&dport.to_be_bytes());
    tcp.extend_from_slice(&seq.to_be_bytes());
    tcp.extend_from_slice(&ack.to_be_bytes());
    tcp.push(0x50); // data offset = 5 words (20-byte header), reserved 0
    tcp.push(flags);
    tcp.extend_from_slice(&TCP_WINDOW.to_be_bytes());
    tcp.extend_from_slice(&[0, 0]); // checksum (filled below)
    tcp.extend_from_slice(&[0, 0]); // urgent pointer
    tcp.extend_from_slice(payload);
    let cks = tcp_checksum(&ap_ip, &client_ip, &tcp);
    tcp[16] = (cks >> 8) as u8;
    tcp[17] = cks as u8;

    let ip_total = (20 + tcp.len()) as u16;
    let mut ip = vec![
        0x45,
        0x00,
        (ip_total >> 8) as u8,
        ip_total as u8,
        0x00,
        0x00, // identification
        0x40,
        0x00, // flags: don't-fragment
        0x40, // TTL 64
        0x06, // protocol: TCP
        0x00,
        0x00, // header checksum (filled below)
    ];
    ip.extend_from_slice(&ap_ip);
    ip.extend_from_slice(&client_ip);
    let ipck = inet_checksum(&ip);
    ip[10] = (ipck >> 8) as u8;
    ip[11] = ipck as u8;
    ip.extend_from_slice(&tcp);
    data_frame(da, 0x0800, &ip)
}

/// TCP checksum over the IPv4 pseudo-header + segment. The segment's checksum
/// field must be zero on entry.
fn tcp_checksum(src_ip: &[u8; 4], dst_ip: &[u8; 4], tcp: &[u8]) -> u16 {
    let mut buf = Vec::with_capacity(12 + tcp.len());
    buf.extend_from_slice(src_ip);
    buf.extend_from_slice(dst_ip);
    buf.push(0);
    buf.push(6); // protocol = TCP
    buf.extend_from_slice(&(tcp.len() as u16).to_be_bytes());
    buf.extend_from_slice(tcp);
    inet_checksum(&buf)
}

/// Whether the accumulated bytes contain a complete HTTP request head (headers
/// terminated by CRLFCRLF). Sufficient for GET (no body).
fn http_req_complete(req: &[u8]) -> bool {
    req.windows(4).any(|w| w == b"\r\n\r\n")
}

/// Re-wrap a station-transmitted IPv4 data frame as a from-DS frame to the
/// destination station (the AP forwarding STA↔STA traffic). Copies the LLC/SNAP
/// + IP bytes verbatim.
fn reframe_to_sta(frame: &[u8], dst_mac: [u8; 6]) -> Option<Vec<u8>> {
    let snap = snap_off(frame);
    if frame.len() < snap {
        return None;
    }
    let ethertype = u16::from_be_bytes([frame[snap - 2], frame[snap - 1]]);
    Some(data_frame(dst_mac, ethertype, &frame[snap..]))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sta_mac(n: u8) -> [u8; 6] {
        [0x02, 0, 0, 0, 0, n]
    }

    // Each test owns its VirtualWifiBus, so they no longer race on a shared
    // process-global (the old `static MEDIUM` forced everything into one
    // sequential test).
    #[test]
    fn medium_assoc_dhcp_and_routing() {
        let bus = VirtualWifiBus::new();
        let (a, b) = (sta_mac(2), sta_mac(3));

        // ── Association handshake responds to the sender ──
        bus.submit(a, &[0x40, 0, 0, 0]); // probe-req (mgmt subtype 4)
        let inbox = bus.take_inbox(a);
        assert_eq!(inbox.len(), 1);
        assert_eq!(inbox[0][0], 0x50); // probe response
        assert_eq!(&inbox[0][4..10], &a); // addressed to the sender

        // ── Distinct, idempotent DHCP IPs per station ──
        let ip_a = bus.with(|m| m.assign_ip(a));
        let ip_b = bus.with(|m| m.assign_ip(b));
        assert_eq!(ip_a, [192, 168, 4, 2]);
        assert_eq!(ip_b, [192, 168, 4, 3]);
        assert_eq!(bus.with(|m| m.assign_ip(a)), ip_a);

        // ── IPv4 routed station-to-station ──
        // Station A sends an IPv4/UDP datagram to B's IP (to-DS data frame).
        let payload = b"hi-b";
        let mut udp = Vec::new();
        udp.extend_from_slice(&1111u16.to_be_bytes());
        udp.extend_from_slice(&2222u16.to_be_bytes());
        udp.extend_from_slice(&((8 + payload.len()) as u16).to_be_bytes());
        udp.extend_from_slice(&[0, 0]);
        udp.extend_from_slice(payload);
        let ip_total = (20 + udp.len()) as u16;
        let mut ip = vec![
            0x45,
            0x00,
            (ip_total >> 8) as u8,
            ip_total as u8,
            0,
            0,
            0,
            0,
            0x40,
            0x11,
            0,
            0,
        ];
        ip.extend_from_slice(&[192, 168, 4, 2]); // src A
        ip.extend_from_slice(&ip_b); // dst B
        ip.extend_from_slice(&udp);
        // to-DS data frame from A.
        let mut tx = vec![0x08, 0x01, 0x00, 0x00];
        tx.extend_from_slice(&AP_BSSID); // addr1 = BSSID
        tx.extend_from_slice(&a); // addr2 = SA
        tx.extend_from_slice(&b); // addr3 = DA
        tx.extend_from_slice(&[0x00, 0x00]);
        tx.extend_from_slice(&[0xAA, 0xAA, 0x03, 0x00, 0x00, 0x00, 0x08, 0x00]);
        tx.extend_from_slice(&ip);
        bus.submit(a, &tx);
        // B should receive a from-DS frame carrying the same payload.
        let inbox_b = bus.take_inbox(b);
        assert_eq!(inbox_b.len(), 1, "B should receive the routed frame");
        let f = &inbox_b[0];
        assert_eq!(f[1] & 0x02, 0x02, "from-DS");
        assert_eq!(&f[4..10], &b, "addressed to B");
        assert!(
            f.windows(payload.len()).any(|w| w == payload),
            "payload preserved"
        );
        assert!(bus.take_inbox(a).is_empty(), "A gets nothing back");

        // ── Isolation: a station on a DIFFERENT bus hears nothing ──
        // Re-send A→B's routed datagram; a second, independent medium must not
        // deliver it to B. This is what the process-static MEDIUM could not do.
        let other = VirtualWifiBus::new();
        other.with(|m| {
            m.assign_ip(a);
            m.assign_ip(b);
        });
        bus.submit(a, &tx);
        assert!(
            other.take_inbox(b).is_empty(),
            "frame leaked across independent WiFi buses"
        );
    }

    // ───────────────────── TCP / HTTP bridge ─────────────────────

    const CLIENT_IP: [u8; 4] = [192, 168, 4, 2];

    /// Build a station→AP to-DS 802.11 data frame carrying IPv4/TCP. RX-side
    /// checksums are not validated by the AP, so they are left zero here.
    #[allow(clippy::too_many_arguments)]
    fn sta_tcp(
        sta: [u8; 6],
        sport: u16,
        dport: u16,
        seq: u32,
        ack: u32,
        flags: u8,
        payload: &[u8],
    ) -> Vec<u8> {
        let mut tcp = Vec::new();
        tcp.extend_from_slice(&sport.to_be_bytes());
        tcp.extend_from_slice(&dport.to_be_bytes());
        tcp.extend_from_slice(&seq.to_be_bytes());
        tcp.extend_from_slice(&ack.to_be_bytes());
        tcp.push(0x50);
        tcp.push(flags);
        tcp.extend_from_slice(&0x2000u16.to_be_bytes());
        tcp.extend_from_slice(&[0, 0, 0, 0]); // checksum + urgent
        tcp.extend_from_slice(payload);

        let ip_total = (20 + tcp.len()) as u16;
        let mut ip = vec![
            0x45,
            0x00,
            (ip_total >> 8) as u8,
            ip_total as u8,
            0,
            0,
            0x40,
            0x00,
            0x40,
            0x06,
            0,
            0,
        ];
        ip.extend_from_slice(&CLIENT_IP);
        ip.extend_from_slice(&AP_IP);
        ip.extend_from_slice(&tcp);

        let mut f = vec![0x08, 0x01, 0x00, 0x00]; // data, to-DS
        f.extend_from_slice(&AP_BSSID); // addr1 = BSSID
        f.extend_from_slice(&sta); // addr2 = SA
        f.extend_from_slice(&AP_BSSID); // addr3 = DA (the AP)
        f.extend_from_slice(&[0x00, 0x00]);
        f.extend_from_slice(&[0xAA, 0xAA, 0x03, 0x00, 0x00, 0x00, 0x08, 0x00]);
        f.extend_from_slice(&ip);
        f
    }

    /// (flags, seq, ack, payload) from an AP→STA TCP reply frame.
    fn reply_tcp(frame: &[u8]) -> (u8, u32, u32, Vec<u8>) {
        let ip = snap_off(frame);
        let ihl = (frame[ip] & 0x0F) as usize * 4;
        let t = ip + ihl;
        let seq = u32::from_be_bytes([frame[t + 4], frame[t + 5], frame[t + 6], frame[t + 7]]);
        let ack = u32::from_be_bytes([frame[t + 8], frame[t + 9], frame[t + 10], frame[t + 11]]);
        let doff = (frame[t + 12] >> 4) as usize * 4;
        (frame[t + 13], seq, ack, frame[t + doff..].to_vec())
    }

    /// Verify a reply's TCP checksum (0 == valid) and its IPv4 header checksum.
    fn checksums_ok(frame: &[u8]) -> bool {
        let ip = snap_off(frame);
        let ihl = (frame[ip] & 0x0F) as usize * 4;
        if inet_checksum(&frame[ip..ip + ihl]) != 0 {
            return false;
        }
        let tcp = &frame[ip + ihl..];
        // Pseudo-header uses AP_IP as source, CLIENT_IP as dest.
        tcp_checksum(&AP_IP, &CLIENT_IP, tcp) == 0
    }

    #[test]
    fn tcp_http_get_roundtrip() {
        // Live API numbers move; only require a public-stats-shaped JSON body.
        let bus = VirtualWifiBus::new();
        let sta = sta_mac(2);
        let (sport, dport) = (50000u16, 80u16);

        // ── SYN → SYN-ACK ──
        bus.submit(sta, &sta_tcp(sta, sport, dport, 1000, 0, TCP_SYN, &[]));
        let rx = bus.take_inbox(sta);
        assert_eq!(rx.len(), 1, "one SYN-ACK");
        let (flags, srv_isn, ack, _) = reply_tcp(&rx[0]);
        assert_eq!(flags, TCP_SYN | TCP_ACK, "SYN-ACK flags");
        assert_eq!(ack, 1001, "acks client ISN+1");
        assert!(checksums_ok(&rx[0]), "SYN-ACK checksums valid");

        // ── ACK + GET (combined) → response + FIN ──
        let get = b"GET /v1/public-stats HTTP/1.1\r\nHost: 192.168.4.1\r\n\r\n";
        bus.submit(
            sta,
            &sta_tcp(sta, sport, dport, 1001, srv_isn + 1, TCP_PSH | TCP_ACK, get),
        );
        let rx = bus.take_inbox(sta);
        assert_eq!(rx.len(), 2, "response segment + FIN");

        let (f1, seq1, ack1, body) = reply_tcp(&rx[0]);
        assert_eq!(f1 & TCP_PSH, TCP_PSH, "response is PSH");
        assert_eq!(seq1, srv_isn + 1, "response seq follows SYN");
        assert_eq!(ack1, 1001 + get.len() as u32, "acks the full request");
        assert!(checksums_ok(&rx[0]), "response checksums valid");

        let text = String::from_utf8_lossy(&body);
        assert!(text.starts_with("HTTP/1.1 200"), "200 OK: {text}");
        assert!(text.contains("application/json"), "json content-type");
        assert!(
            text.contains("boards_supported"),
            "carries public-stats JSON (live or baked): {text}"
        );

        let (f2, seq2, _, _) = reply_tcp(&rx[1]);
        assert_eq!(f2 & TCP_FIN, TCP_FIN, "then FIN");
        assert_eq!(seq2, srv_isn + 1 + body.len() as u32, "FIN seq after body");

        // ── client FIN → ACK, connection closed ──
        let client_fin_seq = 1001 + get.len() as u32;
        bus.submit(
            sta,
            &sta_tcp(
                sta,
                sport,
                dport,
                client_fin_seq,
                seq2 + 1,
                TCP_FIN | TCP_ACK,
                &[],
            ),
        );
        let rx = bus.take_inbox(sta);
        assert_eq!(rx.len(), 1, "ACK of client FIN");
        let (f3, _, ack3, _) = reply_tcp(&rx[0]);
        assert_eq!(f3 & TCP_ACK, TCP_ACK);
        assert_eq!(ack3, client_fin_seq + 1, "acks client FIN");
        // A further stray segment on the closed connection is ignored.
        bus.submit(
            sta,
            &sta_tcp(sta, sport, dport, client_fin_seq + 1, 0, TCP_ACK, &[]),
        );
        assert!(
            bus.take_inbox(sta).is_empty(),
            "closed connection is silent"
        );
    }

    #[test]
    fn tcp_http_unknown_path_404() {
        let bus = VirtualWifiBus::new();
        let sta = sta_mac(2);
        let (sport, dport) = (40001u16, 80u16);
        bus.submit(sta, &sta_tcp(sta, sport, dport, 500, 0, TCP_SYN, &[]));
        let (_, srv_isn, _, _) = reply_tcp(&bus.take_inbox(sta)[0]);

        let get = b"GET /nope HTTP/1.1\r\n\r\n";
        bus.submit(
            sta,
            &sta_tcp(sta, sport, dport, 501, srv_isn + 1, TCP_PSH | TCP_ACK, get),
        );
        let rx = bus.take_inbox(sta);
        let (_, _, _, body) = reply_tcp(&rx[0]);
        let text = String::from_utf8_lossy(&body);
        assert!(
            text.starts_with("HTTP/1.1 404"),
            "unknown path → 404: {text}"
        );
    }

    #[test]
    fn with_config_ssid_carried_in_beacon() {
        // A configured SSID must appear in the beacon the AP queues. Default cfg
        // (SSID "labwired-ap") is byte-identical to the old hardcoded AP; here we
        // override it and confirm the override propagates through the builder.
        let cfg = ApConfig::from_parts(Some("myap".to_string()), None, None);
        let bus = VirtualWifiBus::with_config(cfg);
        let sta = sta_mac(2);
        bus.queue_beacon(sta, 6);
        let rx = bus.take_inbox(sta);
        assert_eq!(rx.len(), 1, "one beacon queued");
        let f = &rx[0];
        assert_eq!(f[0], 0x80, "beacon subtype");
        assert!(
            f.windows(4).any(|w| w == b"myap"),
            "beacon carries the configured SSID"
        );
    }

    #[test]
    fn serves_none_returns_404() {
        // serves = None → the HTTP origin has no routes, so even the demo path
        // 404s (nothing is served). Proves the L4 config is honored.
        let cfg = ApConfig::from_parts(None, None, Some("none"));
        assert_eq!(cfg.serves, ApServes::None);
        let bus = VirtualWifiBus::with_config(cfg);
        let sta = sta_mac(2);
        let (sport, dport) = (45000u16, 80u16);
        bus.submit(sta, &sta_tcp(sta, sport, dport, 100, 0, TCP_SYN, &[]));
        let (_, srv_isn, _, _) = reply_tcp(&bus.take_inbox(sta)[0]);
        let get = b"GET /v1/public-stats HTTP/1.1\r\n\r\n";
        bus.submit(
            sta,
            &sta_tcp(sta, sport, dport, 101, srv_isn + 1, TCP_PSH | TCP_ACK, get),
        );
        let rx = bus.take_inbox(sta);
        let (_, _, _, body) = reply_tcp(&rx[0]);
        let text = String::from_utf8_lossy(&body);
        assert!(
            text.starts_with("HTTP/1.1 404"),
            "serves=none → no /v1/public-stats route → 404: {text}"
        );
    }

    #[test]
    fn parse_http_200_body_extracts_json() {
        let raw =
            b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\n\r\n{\"boards_supported\":11}";
        let body = parse_http_200_body(raw).expect("200 body");
        assert_eq!(body, br#"{"boards_supported":11}"#);
        assert!(parse_http_200_body(b"HTTP/1.1 500 err\r\n\r\nnope").is_none());
        assert!(parse_http_200_body(b"HTTP/1.1 200 OK\r\n\r\nnot-json").is_none());
    }

    #[test]
    fn resolve_public_stats_body_honors_override() {
        // Serialize against other tests that touch the process-global override.
        let _gate = public_stats_override().lock().unwrap();
        // Hold the lock only for the set/get dance — re-enter via set_ which
        // also locks, so do the set without nested lock by writing directly.
        drop(_gate);
        set_public_stats_body(Some(b"{\"boards_supported\":42}".to_vec()));
        let body = resolve_public_stats_body();
        assert_eq!(body, b"{\"boards_supported\":42}");
        set_public_stats_body(None);
        // Without override: live (if online) or baked fallback — always has marker.
        let body = resolve_public_stats_body();
        assert!(
            body.windows(b"boards_supported".len())
                .any(|w| w == b"boards_supported"),
            "live or baked body must carry boards_supported: {}",
            String::from_utf8_lossy(&body)
        );
    }

    #[test]
    fn labwired_stats_server_serves_cached_body() {
        let srv = LabwiredStatsServer::with_cached_body(
            br#"{"boards_supported":7,"parts_supported":1,"labs_opened":1,"simulations_run":1,"active_sessions":1}"#
                .to_vec(),
        );
        let resp = srv.on_data(0, b"GET /v1/public-stats HTTP/1.1\r\n\r\n");
        let text = String::from_utf8_lossy(&resp);
        assert!(text.starts_with("HTTP/1.1 200"), "{text}");
        assert!(text.contains("\"boards_supported\":7"), "{text}");
    }

    #[test]
    fn tcp_egress_http_get_public_stats() {
        if !internet_enabled() {
            return;
        }
        // Resolve api.labwired.com and GET /v1/public-stats through the NAT.
        let ips = crate::peripherals::esp32c3::virtual_wifi_inet::resolve_a("api.labwired.com");
        let Some(remote) = ips.into_iter().next() else {
            return; // DNS failed offline
        };
        let bus = VirtualWifiBus::new();
        let sta = sta_mac(9);
        // Give the STA a lease so client IP is known (DHCP not strictly required
        // for the NAT path which reads src IP from the frame).
        let client_ip = [192, 168, 4, 9];
        let (sport, dport) = (51000u16, 80u16);

        // SYN → SYN-ACK (real connect).
        bus.submit(
            sta,
            &sta_tcp_to(sta, client_ip, remote, sport, dport, 1000, 0, TCP_SYN, &[]),
        );
        let rx = bus.take_inbox(sta);
        assert_eq!(rx.len(), 1, "SYN-ACK or RST");
        let (flags, srv_isn, ack, _) = reply_tcp(&rx[0]);
        if flags & TCP_RST != 0 {
            return; // network blocked
        }
        assert_eq!(flags & (TCP_SYN | TCP_ACK), TCP_SYN | TCP_ACK);
        assert_eq!(ack, 1001);

        let get =
            b"GET /v1/public-stats HTTP/1.1\r\nHost: api.labwired.com\r\nConnection: close\r\n\r\n";
        bus.submit(
            sta,
            &sta_tcp_to(
                sta,
                client_ip,
                remote,
                sport,
                dport,
                1001,
                srv_isn + 1,
                TCP_PSH | TCP_ACK,
                get,
            ),
        );
        // Drain NAT (poll may be needed for delayed body).
        let mut body = Vec::new();
        for _ in 0..50 {
            bus.poll();
            for f in bus.take_inbox(sta) {
                let (_, _, _, pay) = reply_tcp(&f);
                body.extend_from_slice(&pay);
            }
            if body
                .windows(b"boards_supported".len())
                .any(|w| w == b"boards_supported")
            {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        let text = String::from_utf8_lossy(&body);
        // Soft-skip when CI blocks outbound HTTP (common). Local/dev with
        // network still proves the path when the body arrives.
        if !text.contains("boards_supported") {
            eprintln!("skip: no live public-stats over NAT (network restricted?): {text}");
        }
    }

    /// Like `sta_tcp` but to an off-LAN peer (not the AP IP).
    #[allow(clippy::too_many_arguments)]
    fn sta_tcp_to(
        sa: [u8; 6],
        client_ip: [u8; 4],
        remote_ip: [u8; 4],
        sport: u16,
        dport: u16,
        seq: u32,
        ack: u32,
        flags: u8,
        payload: &[u8],
    ) -> Vec<u8> {
        let mut tcp = Vec::new();
        tcp.extend_from_slice(&sport.to_be_bytes());
        tcp.extend_from_slice(&dport.to_be_bytes());
        tcp.extend_from_slice(&seq.to_be_bytes());
        tcp.extend_from_slice(&ack.to_be_bytes());
        tcp.push(0x50);
        tcp.push(flags);
        tcp.extend_from_slice(&TCP_WINDOW.to_be_bytes());
        tcp.extend_from_slice(&[0, 0, 0, 0]);
        tcp.extend_from_slice(payload);
        let cks = tcp_checksum(&client_ip, &remote_ip, &tcp);
        tcp[16] = (cks >> 8) as u8;
        tcp[17] = cks as u8;
        let ip_total = (20 + tcp.len()) as u16;
        let mut ip = vec![
            0x45,
            0x00,
            (ip_total >> 8) as u8,
            ip_total as u8,
            0,
            0,
            0,
            0,
            0x40,
            0x06,
            0,
            0,
        ];
        ip.extend_from_slice(&client_ip);
        ip.extend_from_slice(&remote_ip);
        let c = inet_checksum(&ip);
        ip[10] = (c >> 8) as u8;
        ip[11] = c as u8;
        ip.extend_from_slice(&tcp);
        // to-DS data frame STA→AP
        let mut f = Vec::new();
        f.extend_from_slice(&[0x08, 0x01, 0x00, 0x00]); // FC to-DS
        f.extend_from_slice(&AP_BSSID); // addr1 BSSID
        f.extend_from_slice(&sa); // addr2 SA
        f.extend_from_slice(&AP_BSSID); // addr3
        f.extend_from_slice(&[0x00, 0x00]);
        f.extend_from_slice(&[0xAA, 0xAA, 0x03, 0x00, 0x00, 0x00, 0x08, 0x00]);
        f.extend_from_slice(&ip);
        f
    }
}
