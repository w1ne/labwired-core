// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Simulated network endpoints — a deterministic, in-process TCP/server
//! substrate for the ESP32 WiFi functional model.
//!
//! This is the **"simulated endpoints"** networking backend: firmware that
//! connects through (eventually) thunked `esp_wifi` + lwIP socket calls
//! reaches virtual servers hosted entirely inside the simulator — an
//! in-sim HTTP responder, an echo server, (later) an MQTT broker — with no
//! traffic ever touching the host's real network. That keeps runs
//! deterministic, sandboxed, and CI-friendly.
//!
//! Layering:
//!   * [`VirtualAp`] models the WiFi association step — a station "joins"
//!     an SSID and is handed an IPv4 lease. No 802.11/radio fidelity; this
//!     is the *functional* outcome firmware observes (got-IP).
//!   * [`SimNet`] is the socket/transport layer: virtual servers keyed by
//!     `SocketAddrV4`, simple DNS, and a `connect`/`send`/`recv`/`close`
//!     API the firmware-facing socket thunks will drive.
//!   * [`SimServer`] is how an endpoint responds. [`EchoServer`] and
//!     [`HttpServer`] are built in.
//!
//! Connections are synchronous and request/response: `send` runs the
//! server's handler immediately and buffers the reply for the next `recv`.
//! That matches how blocking-socket firmware (and our future thunks) drive
//! it, and keeps the model free of threads or real I/O.

use std::collections::HashMap;
use std::net::{Ipv4Addr, SocketAddrV4};
use std::sync::Arc;

/// A virtual server endpoint. Handlers are pure: given the connection id and
/// the bytes a client sent, return the bytes to send back (possibly empty).
pub trait SimServer: Send + Sync + std::fmt::Debug {
    /// Bytes to emit immediately on connect (a banner/greeting). Default none.
    fn on_connect(&self, _conn: u32) -> Vec<u8> {
        Vec::new()
    }
    /// Handle a chunk of client→server bytes; return server→client bytes.
    fn on_data(&self, conn: u32, data: &[u8]) -> Vec<u8>;
}

/// Echoes whatever it receives — the simplest reachable endpoint.
#[derive(Debug, Default)]
pub struct EchoServer;

impl SimServer for EchoServer {
    fn on_data(&self, _conn: u32, data: &[u8]) -> Vec<u8> {
        data.to_vec()
    }
}

/// A canned HTTP/1.1 response (status code + reason + body).
#[derive(Debug, Clone)]
pub struct HttpResponse {
    pub status: u16,
    pub reason: String,
    pub content_type: String,
    pub body: Vec<u8>,
}

impl HttpResponse {
    /// 200 OK with a `text/plain` body.
    pub fn ok(body: impl Into<Vec<u8>>) -> Self {
        Self {
            status: 200,
            reason: "OK".into(),
            content_type: "text/plain".into(),
            body: body.into(),
        }
    }

    /// 200 OK with an `application/json` body.
    pub fn json(body: impl Into<Vec<u8>>) -> Self {
        Self {
            status: 200,
            reason: "OK".into(),
            content_type: "application/json".into(),
            body: body.into(),
        }
    }

    /// Serialize to an HTTP/1.1 response with a `Content-Length` header and
    /// `Connection: close`.
    fn encode(&self) -> Vec<u8> {
        let head = format!(
            "HTTP/1.1 {} {}\r\nContent-Type: {}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            self.status,
            self.reason,
            self.content_type,
            self.body.len()
        );
        let mut out = head.into_bytes();
        out.extend_from_slice(&self.body);
        out
    }
}

/// Minimal in-sim HTTP/1.1 responder. Routes are keyed by `(METHOD, path)`;
/// an unmatched request gets a 404. Enough for firmware doing
/// `GET`/`POST` against a known device backend.
#[derive(Debug, Default)]
pub struct HttpServer {
    routes: HashMap<(String, String), HttpResponse>,
    not_found: Option<HttpResponse>,
}

impl HttpServer {
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a response for `METHOD path` (method is upper-cased).
    pub fn route(mut self, method: &str, path: &str, resp: HttpResponse) -> Self {
        self.routes
            .insert((method.to_ascii_uppercase(), path.to_string()), resp);
        self
    }

    /// Convenience for a `GET path` route.
    pub fn get(self, path: &str, resp: HttpResponse) -> Self {
        self.route("GET", path, resp)
    }

    /// Parse the request line (`METHOD SP path SP HTTP/x`) from the first
    /// line of `data`. Returns `(method, path)` upper-cased method.
    fn parse_request_line(data: &[u8]) -> Option<(String, String)> {
        let text = std::str::from_utf8(data).ok()?;
        let first = text.lines().next()?;
        let mut parts = first.split_whitespace();
        let method = parts.next()?.to_ascii_uppercase();
        let target = parts.next()?;
        // Strip any query string for routing.
        let path = target.split('?').next().unwrap_or(target).to_string();
        Some((method, path))
    }
}

impl SimServer for HttpServer {
    fn on_data(&self, _conn: u32, data: &[u8]) -> Vec<u8> {
        let resp = match Self::parse_request_line(data) {
            Some((method, path)) => self.routes.get(&(method, path)).cloned(),
            None => None,
        };
        let resp = resp.unwrap_or_else(|| {
            self.not_found.clone().unwrap_or_else(|| HttpResponse {
                status: 404,
                reason: "Not Found".into(),
                content_type: "text/plain".into(),
                body: b"not found".to_vec(),
            })
        });
        resp.encode()
    }
}

/// Models the WiFi association step: a station joins an SSID and gets an
/// IPv4 lease from a simple sequential pool. Functional only — no radio,
/// no 4-way handshake; the outcome firmware sees (associated + got-IP).
#[derive(Debug)]
pub struct VirtualAp {
    ssid: String,
    /// Optional pre-shared key; `None` = open network. When set, a join
    /// must present the matching key.
    psk: Option<String>,
    gateway: Ipv4Addr,
    /// Next host octet to lease (gateway is `.1`, leases start at `.2`).
    next_host: u8,
    leases: HashMap<[u8; 6], Ipv4Addr>,
}

impl VirtualAp {
    /// Open AP with gateway `192.168.4.1` (the ESP32 SoftAP default).
    pub fn open(ssid: &str) -> Self {
        Self {
            ssid: ssid.to_string(),
            psk: None,
            gateway: Ipv4Addr::new(192, 168, 4, 1),
            next_host: 2,
            leases: HashMap::new(),
        }
    }

    /// WPA2-style AP requiring `psk`.
    pub fn wpa2(ssid: &str, psk: &str) -> Self {
        let mut ap = Self::open(ssid);
        ap.psk = Some(psk.to_string());
        ap
    }

    pub fn ssid(&self) -> &str {
        &self.ssid
    }

    pub fn gateway(&self) -> Ipv4Addr {
        self.gateway
    }

    /// Attempt to associate `mac` to `ssid` with optional `key`. Returns the
    /// leased station IP on success, or `None` on SSID/key mismatch.
    pub fn associate(&mut self, ssid: &str, key: Option<&str>, mac: [u8; 6]) -> Option<Ipv4Addr> {
        if ssid != self.ssid {
            return None;
        }
        if let Some(expected) = &self.psk {
            if key != Some(expected.as_str()) {
                return None;
            }
        }
        if let Some(ip) = self.leases.get(&mac) {
            return Some(*ip);
        }
        let octets = self.gateway.octets();
        let ip = Ipv4Addr::new(octets[0], octets[1], octets[2], self.next_host);
        self.next_host = self.next_host.saturating_add(1);
        self.leases.insert(mac, ip);
        Some(ip)
    }
}

#[derive(Debug)]
struct Conn {
    server: Arc<dyn SimServer>,
    /// Buffered server→client bytes awaiting `recv`.
    rx: Vec<u8>,
    open: bool,
}

/// The simulated network: virtual servers + DNS + a synchronous socket API.
#[derive(Debug, Default)]
pub struct SimNet {
    servers: HashMap<SocketAddrV4, Arc<dyn SimServer>>,
    dns: HashMap<String, Ipv4Addr>,
    conns: HashMap<u32, Conn>,
    next_conn: u32,
}

impl SimNet {
    pub fn new() -> Self {
        Self::default()
    }

    /// Host a `server` at `addr`. Replaces any existing server there.
    pub fn listen(&mut self, addr: SocketAddrV4, server: Arc<dyn SimServer>) {
        self.servers.insert(addr, server);
    }

    /// Register a DNS A-record so firmware can connect by hostname.
    pub fn add_host(&mut self, name: &str, ip: Ipv4Addr) {
        self.dns.insert(name.to_ascii_lowercase(), ip);
    }

    /// Resolve a hostname to an IPv4 address (case-insensitive). Also accepts
    /// a literal dotted-quad.
    pub fn resolve(&self, name: &str) -> Option<Ipv4Addr> {
        if let Some(ip) = self.dns.get(&name.to_ascii_lowercase()) {
            return Some(*ip);
        }
        name.parse::<Ipv4Addr>().ok()
    }

    /// Open a connection to `addr`. Returns a connection id, or `None` if no
    /// server is listening there (connection refused). Any connect banner is
    /// buffered for the first `recv`.
    pub fn connect(&mut self, addr: SocketAddrV4) -> Option<u32> {
        let server = Arc::clone(self.servers.get(&addr)?);
        let id = self.next_conn;
        self.next_conn = self.next_conn.wrapping_add(1);
        let banner = server.on_connect(id);
        self.conns.insert(
            id,
            Conn {
                server,
                rx: banner,
                open: true,
            },
        );
        Some(id)
    }

    /// Resolve `host` then connect to `host:port`.
    pub fn connect_host(&mut self, host: &str, port: u16) -> Option<u32> {
        let ip = self.resolve(host)?;
        self.connect(SocketAddrV4::new(ip, port))
    }

    /// Send `data` on `conn`; the server's response is buffered for `recv`.
    /// Returns the number of bytes accepted, or `None` if the connection is
    /// closed/unknown.
    pub fn send(&mut self, conn: u32, data: &[u8]) -> Option<usize> {
        let c = self.conns.get_mut(&conn)?;
        if !c.open {
            return None;
        }
        let reply = c.server.on_data(conn, data);
        c.rx.extend_from_slice(&reply);
        Some(data.len())
    }

    /// Drain and return any buffered server→client bytes for `conn` (empty
    /// if none / unknown connection).
    pub fn recv(&mut self, conn: u32) -> Vec<u8> {
        match self.conns.get_mut(&conn) {
            Some(c) => std::mem::take(&mut c.rx),
            None => Vec::new(),
        }
    }

    /// True if `conn` is open with no pending bytes to read.
    pub fn is_drained(&self, conn: u32) -> bool {
        self.conns
            .get(&conn)
            .map(|c| c.rx.is_empty())
            .unwrap_or(true)
    }

    /// Close and forget `conn`.
    pub fn close(&mut self, conn: u32) {
        self.conns.remove(&conn);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn echo_server_round_trips() {
        let mut net = SimNet::new();
        let addr = SocketAddrV4::new(Ipv4Addr::new(10, 0, 0, 5), 7);
        net.listen(addr, Arc::new(EchoServer));

        let conn = net.connect(addr).expect("connect");
        net.send(conn, b"hello").unwrap();
        assert_eq!(net.recv(conn), b"hello");
        // Drained after read.
        assert!(net.is_drained(conn));
    }

    #[test]
    fn connect_to_unlistened_addr_is_refused() {
        let mut net = SimNet::new();
        let addr = SocketAddrV4::new(Ipv4Addr::new(10, 0, 0, 9), 80);
        assert!(net.connect(addr).is_none());
    }

    #[test]
    fn dns_resolves_then_connects() {
        let mut net = SimNet::new();
        let ip = Ipv4Addr::new(93, 184, 216, 34);
        net.add_host("example.com", ip);
        net.listen(
            SocketAddrV4::new(ip, 80),
            Arc::new(HttpServer::new().get("/", HttpResponse::ok("hi"))),
        );
        assert_eq!(net.resolve("EXAMPLE.com"), Some(ip));
        let conn = net.connect_host("example.com", 80).expect("connect");
        net.send(conn, b"GET / HTTP/1.1\r\nHost: example.com\r\n\r\n")
            .unwrap();
        let resp = net.recv(conn);
        let text = String::from_utf8_lossy(&resp);
        assert!(text.starts_with("HTTP/1.1 200 OK"), "{text}");
        assert!(text.ends_with("hi"), "{text}");
        assert!(text.contains("Content-Length: 2"), "{text}");
    }

    #[test]
    fn http_routes_and_404() {
        let server = Arc::new(
            HttpServer::new()
                .get("/status", HttpResponse::json(r#"{"ok":true}"#))
                .route("POST", "/cmd", HttpResponse::ok("done")),
        );
        let mut net = SimNet::new();
        let addr = SocketAddrV4::new(Ipv4Addr::new(192, 168, 4, 1), 80);
        net.listen(addr, server);

        let c = net.connect(addr).unwrap();
        net.send(c, b"GET /status?x=1 HTTP/1.1\r\n\r\n").unwrap();
        let r = String::from_utf8(net.recv(c)).unwrap();
        assert!(r.contains("200 OK") && r.contains("application/json") && r.ends_with("true}"));

        net.send(c, b"POST /cmd HTTP/1.1\r\n\r\n").unwrap();
        assert!(String::from_utf8(net.recv(c)).unwrap().ends_with("done"));

        net.send(c, b"GET /missing HTTP/1.1\r\n\r\n").unwrap();
        assert!(String::from_utf8(net.recv(c))
            .unwrap()
            .contains("404 Not Found"));
    }

    #[test]
    fn virtual_ap_leases_distinct_ips() {
        let mut ap = VirtualAp::open("labwired");
        let a = ap.associate("labwired", None, [0, 0, 0, 0, 0, 1]).unwrap();
        let b = ap.associate("labwired", None, [0, 0, 0, 0, 0, 2]).unwrap();
        assert_eq!(a, Ipv4Addr::new(192, 168, 4, 2));
        assert_eq!(b, Ipv4Addr::new(192, 168, 4, 3));
        // Same MAC re-associating keeps its lease.
        assert_eq!(ap.associate("labwired", None, [0, 0, 0, 0, 0, 1]), Some(a));
        // Wrong SSID is rejected.
        assert!(ap.associate("other", None, [0, 0, 0, 0, 0, 9]).is_none());
    }

    #[test]
    fn virtual_ap_enforces_psk() {
        let mut ap = VirtualAp::wpa2("secure", "hunter2");
        assert!(ap.associate("secure", None, [1; 6]).is_none());
        assert!(ap.associate("secure", Some("wrong"), [1; 6]).is_none());
        assert!(ap.associate("secure", Some("hunter2"), [1; 6]).is_some());
    }
}
