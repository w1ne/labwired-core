//! Universal internet egress for the virtual AP — **any host**, not one URL.
//!
//! * **DNS (any name):** host resolver (native) or DoH via host-net (browser)
//! * **TCP off-LAN (any IP:port):** NAT via `TcpStream` (native); browser uses
//!   host `fetch` when the station speaks cleartext HTTP
//! * **TCP to AP:80 with any `Host:`:** reverse-proxied in `virtual_wifi`
//!
//! Set `LABWIRED_WIFI_NO_INTERNET=1` to force offline on native.

use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::{Shutdown, SocketAddr, TcpStream, ToSocketAddrs};
use std::time::Duration;

#[cfg(target_arch = "wasm32")]
use crate::peripherals::esp32c3::virtual_wifi_host_net;
#[cfg(target_arch = "wasm32")]
use crate::peripherals::esp32c3::virtual_wifi_host_net::parse_http_request_line_host;

/// When false, external destinations are dropped.
pub fn internet_enabled() -> bool {
    if std::env::var_os("LABWIRED_WIFI_NO_INTERNET").is_some()
        || std::env::var_os("LABWIRED_WIFI_STATS_OFFLINE").is_some()
    {
        return false;
    }
    #[cfg(target_arch = "wasm32")]
    {
        virtual_wifi_host_net::bridge_active()
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        true
    }
}

enum EgressBackend {
    #[cfg(not(target_arch = "wasm32"))]
    Native(TcpStream),
    /// Browser: accumulate HTTP request, host `fetch`es, deliver response.
    #[cfg(target_arch = "wasm32")]
    HostHttp {
        req_buf: Vec<u8>,
        pending_id: Option<u32>,
        /// Remaining response bytes to send to the station.
        resp_queue: Vec<u8>,
        done: bool,
    },
}

/// Per-station TCP connection through the NAT (key = station source port).
pub struct EgressTcp {
    pub rcv_nxt: u32,
    pub snd_nxt: u32,
    pub fin_sent: bool,
    pub client_ip: [u8; 4],
    pub remote_ip: [u8; 4],
    pub remote_port: u16,
    backend: EgressBackend,
    /// Remote peer closed its write half.
    pub peer_fin: bool,
}

impl std::fmt::Debug for EgressTcp {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EgressTcp")
            .field("client_ip", &self.client_ip)
            .field("remote_ip", &self.remote_ip)
            .field("remote_port", &self.remote_port)
            .field("fin_sent", &self.fin_sent)
            .finish()
    }
}

impl EgressTcp {
    /// Open a connection: native TCP, or browser host-HTTP proxy.
    pub fn connect(
        client_ip: [u8; 4],
        remote_ip: [u8; 4],
        remote_port: u16,
        rcv_nxt: u32,
        snd_nxt: u32,
    ) -> Option<Self> {
        #[cfg(not(target_arch = "wasm32"))]
        {
            let addr = SocketAddr::from((remote_ip, remote_port));
            let stream = TcpStream::connect_timeout(&addr, Duration::from_secs(3)).ok()?;
            let _ = stream.set_nonblocking(true);
            let _ = stream.set_nodelay(true);
            Some(Self {
                rcv_nxt,
                snd_nxt,
                fin_sent: false,
                client_ip,
                remote_ip,
                remote_port,
                backend: EgressBackend::Native(stream),
                peer_fin: false,
            })
        }
        #[cfg(target_arch = "wasm32")]
        {
            if !virtual_wifi_host_net::bridge_active() {
                return None;
            }
            // Browser: accept any port; HTTP is proxied via fetch when the
            // request is complete. Non-HTTP traffic will stall (no raw TCP).
            Some(Self {
                rcv_nxt,
                snd_nxt,
                fin_sent: false,
                client_ip,
                remote_ip,
                remote_port,
                backend: EgressBackend::HostHttp {
                    req_buf: Vec::new(),
                    pending_id: None,
                    resp_queue: Vec::new(),
                    done: false,
                },
                peer_fin: false,
            })
        }
    }

    pub fn write_all(&mut self, data: &[u8]) -> bool {
        match &mut self.backend {
            #[cfg(not(target_arch = "wasm32"))]
            EgressBackend::Native(stream) => match stream.write_all(data) {
                Ok(()) => true,
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    let _ = stream.write(data);
                    true
                }
                Err(_) => false,
            },
            #[cfg(target_arch = "wasm32")]
            EgressBackend::HostHttp {
                req_buf,
                pending_id,
                ..
            } => {
                req_buf.extend_from_slice(data);
                if pending_id.is_none() && http_req_complete(req_buf) {
                    // Build a fetch URL for **whatever host** the station is
                    // talking to — using Host header, then DNS reverse map,
                    // then dotted IP. Client browser performs the fetch on the
                    // user's own network (no LabWired-only allowlist).
                    if let Some((method, path, url)) =
                        browser_fetch_target(req_buf, self.remote_ip, self.remote_port)
                    {
                        *pending_id = Some(virtual_wifi_host_net::enqueue_http(
                            url,
                            method,
                            req_buf.clone(),
                        ));
                        let _ = path;
                    }
                }
                true
            }
        }
    }

    /// Read available bytes from the real peer (up to `cap`).
    pub fn read_available(&mut self, cap: usize) -> Vec<u8> {
        match &mut self.backend {
            #[cfg(not(target_arch = "wasm32"))]
            EgressBackend::Native(stream) => {
                let mut buf = vec![0u8; cap.min(4096)];
                match stream.read(&mut buf) {
                    Ok(0) => {
                        self.peer_fin = true;
                        Vec::new()
                    }
                    Ok(n) => {
                        buf.truncate(n);
                        buf
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => Vec::new(),
                    Err(_) => {
                        self.peer_fin = true;
                        Vec::new()
                    }
                }
            }
            #[cfg(target_arch = "wasm32")]
            EgressBackend::HostHttp {
                pending_id,
                resp_queue,
                done,
                ..
            } => {
                if let Some(id) = *pending_id {
                    if let Some(resp) = virtual_wifi_host_net::take_http_answer(id) {
                        resp_queue.extend_from_slice(&resp);
                        *pending_id = None;
                        *done = true;
                    }
                }
                if resp_queue.is_empty() {
                    if *done {
                        self.peer_fin = true;
                    }
                    return Vec::new();
                }
                let n = resp_queue.len().min(cap.min(4096));
                let out: Vec<u8> = resp_queue.drain(..n).collect();
                if resp_queue.is_empty() && *done {
                    self.peer_fin = true;
                }
                out
            }
        }
    }

    pub fn shutdown_write(&mut self) {
        match &mut self.backend {
            #[cfg(not(target_arch = "wasm32"))]
            EgressBackend::Native(stream) => {
                let _ = stream.shutdown(Shutdown::Write);
            }
            #[cfg(target_arch = "wasm32")]
            EgressBackend::HostHttp { .. } => {}
        }
    }
}

#[cfg(target_arch = "wasm32")]
fn http_req_complete(req: &[u8]) -> bool {
    // Headers done; for POST/PUT also wait for Content-Length body if present.
    let Some(sep) = req.windows(4).position(|w| w == b"\r\n\r\n") else {
        return false;
    };
    let head = &req[..sep];
    let body = &req[sep + 4..];
    let head_txt = std::str::from_utf8(head).unwrap_or("");
    let mut content_len: Option<usize> = None;
    for line in head_txt.split("\r\n") {
        if let Some(rest) = line
            .strip_prefix("Content-Length:")
            .or_else(|| line.strip_prefix("content-length:"))
        {
            content_len = rest.trim().parse().ok();
            break;
        }
    }
    match content_len {
        Some(n) => body.len() >= n,
        None => true, // GET/HEAD or unknown length
    }
}

/// Resolve method + path + absolute URL for browser `fetch` from a raw request
/// and the TCP peer the station connected to.
#[cfg(target_arch = "wasm32")]
fn browser_fetch_target(
    req: &[u8],
    remote_ip: [u8; 4],
    remote_port: u16,
) -> Option<(String, String, String)> {
    let (method, path, host_hdr) = match parse_http_request_line_host(req) {
        Some(t) => t,
        None => {
            // No Host header — still proxy using DNS reverse map / IP.
            let text = std::str::from_utf8(req).ok()?;
            let mut parts = text.lines().next()?.split_whitespace();
            let method = parts.next()?.to_string();
            let path = parts.next()?.to_string();
            (method, path, String::new())
        }
    };
    let scheme = if remote_port == 443 { "https" } else { "http" };
    if path.starts_with("http://") || path.starts_with("https://") {
        return Some((method, path.clone(), path));
    }
    let host_from_hdr = host_hdr.split(':').next().unwrap_or("").trim();
    let host = if !host_from_hdr.is_empty()
        && !host_from_hdr
            .chars()
            .all(|c| c.is_ascii_digit() || c == '.')
    {
        // Prefer hostname from Host: so the browser uses SNI/CORS origin of that host.
        virtual_wifi_host_net::remember_ip_name(remote_ip, host_from_hdr);
        host_from_hdr.to_string()
    } else if let Some(name) = virtual_wifi_host_net::name_for_ip(remote_ip) {
        name
    } else if !host_from_hdr.is_empty() {
        host_from_hdr.to_string()
    } else {
        format!(
            "{}.{}.{}.{}",
            remote_ip[0], remote_ip[1], remote_ip[2], remote_ip[3]
        )
    };
    let authority = if remote_port == 80 || remote_port == 443 {
        host
    } else {
        format!("{host}:{remote_port}")
    };
    let url = format!("{scheme}://{authority}{path}");
    Some((method, path, url))
}

/// Resolve a hostname (or dotted-quad) to IPv4 addresses via the host resolver.
pub fn resolve_a(name: &str) -> Vec<[u8; 4]> {
    if !internet_enabled() {
        return Vec::new();
    }
    // Dotted quad short-circuit.
    if let Ok(ip) = name.parse::<std::net::Ipv4Addr>() {
        return vec![ip.octets()];
    }
    let host_port = format!("{name}:0");
    match host_port.to_socket_addrs() {
        Ok(iter) => iter
            .filter_map(|a| match a {
                SocketAddr::V4(v4) => Some(v4.ip().octets()),
                _ => None,
            })
            .collect(),
        Err(_) => Vec::new(),
    }
}

/// If `udp_payload` is a DNS query asking for A records, build a response
/// using the host resolver. Returns `None` if not a parseable A query.
pub fn dns_respond(udp_payload: &[u8]) -> Option<Vec<u8>> {
    if udp_payload.len() < 12 {
        return None;
    }
    let id = &udp_payload[0..2];
    let flags = u16::from_be_bytes([udp_payload[2], udp_payload[3]]);
    // QR must be 0 (query).
    if flags & 0x8000 != 0 {
        return None;
    }
    let qdcount = u16::from_be_bytes([udp_payload[4], udp_payload[5]]);
    if qdcount != 1 {
        return None;
    }
    // Parse QNAME
    let mut i = 12usize;
    let mut labels = Vec::new();
    while i < udp_payload.len() {
        let len = udp_payload[i] as usize;
        if len == 0 {
            i += 1;
            break;
        }
        if len & 0xC0 == 0xC0 {
            return None; // compression in query — rare, skip
        }
        i += 1;
        if i + len > udp_payload.len() {
            return None;
        }
        labels.push(
            std::str::from_utf8(&udp_payload[i..i + len])
                .ok()?
                .to_string(),
        );
        i += len;
    }
    if i + 4 > udp_payload.len() {
        return None;
    }
    let qtype = u16::from_be_bytes([udp_payload[i], udp_payload[i + 1]]);
    let qclass = u16::from_be_bytes([udp_payload[i + 2], udp_payload[i + 3]]);
    i += 4;
    if qclass != 1 {
        return None; // IN
    }
    // Only A (1). AAAA (28) → empty NOERROR so clients fall back.
    let name = labels.join(".");
    let answers: Vec<[u8; 4]> = if qtype == 1 {
        resolve_a(&name)
    } else {
        Vec::new()
    };

    let mut out = Vec::new();
    out.extend_from_slice(id);
    // QR=1, AA=1, RD copied, RA=1
    let rd = flags & 0x0100;
    let rcode: u16 = if answers.is_empty() && qtype == 1 {
        3
    } else {
        0
    }; // NXDOMAIN if no A
    let rflags = 0x8000 | 0x0400 | rd | 0x0080 | rcode;
    out.extend_from_slice(&rflags.to_be_bytes());
    out.extend_from_slice(&1u16.to_be_bytes()); // QDCOUNT
    out.extend_from_slice(&(answers.len() as u16).to_be_bytes()); // ANCOUNT
    out.extend_from_slice(&0u16.to_be_bytes()); // NSCOUNT
    out.extend_from_slice(&0u16.to_be_bytes()); // ARCOUNT
                                                // Question section copy
    out.extend_from_slice(&udp_payload[12..i]);
    // Answers: pointer to name at offset 12
    for ip in answers {
        out.extend_from_slice(&[0xC0, 0x0C]); // compression pointer
        out.extend_from_slice(&1u16.to_be_bytes()); // TYPE A
        out.extend_from_slice(&1u16.to_be_bytes()); // CLASS IN
        out.extend_from_slice(&60u32.to_be_bytes()); // TTL 60s
        out.extend_from_slice(&4u16.to_be_bytes()); // RDLENGTH
        out.extend_from_slice(&ip);
    }
    Some(out)
}

/// Key for egress map: station MAC + client TCP port.
pub type EgressKey = ([u8; 6], u16);

/// Table of open NAT connections.
#[derive(Debug, Default)]
pub struct EgressTable {
    pub conns: HashMap<EgressKey, EgressTcp>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_dotted_quad() {
        let ips = resolve_a("127.0.0.1");
        if internet_enabled() {
            assert_eq!(ips, vec![[127, 0, 0, 1]]);
        }
    }

    #[test]
    fn dns_respond_builds_a_record() {
        // Minimal DNS query for example.com A (pre-encoded).
        // We'll build programmatically:
        let mut q = Vec::new();
        q.extend_from_slice(&[0x12, 0x34]); // id
        q.extend_from_slice(&[0x01, 0x00]); // RD
        q.extend_from_slice(&[0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00]);
        for label in ["example", "com"] {
            q.push(label.len() as u8);
            q.extend_from_slice(label.as_bytes());
        }
        q.push(0);
        q.extend_from_slice(&[0x00, 0x01, 0x00, 0x01]); // A IN
        let resp = dns_respond(&q);
        if !internet_enabled() {
            return;
        }
        let resp = resp.expect("dns response");
        assert_eq!(&resp[0..2], &[0x12, 0x34]);
        assert!(resp[2] & 0x80 != 0, "QR set");
    }
}
