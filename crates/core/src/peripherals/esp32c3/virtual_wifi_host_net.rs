//! Browser host-network bridge for the virtual WiFi AP.
//!
//! Wasm has no sockets. The playground JS layer:
//!   1. polls [`poll_dns_requests`] / [`poll_http_requests`] after each sim step
//!   2. resolves DNS (DoH) / fetches HTTP(S)
//!   3. calls [`fulfill_dns`] / [`fulfill_http`]
//!
//! Core then injects the bytes into the station on the next [`take_dns_replies`]
//! / HTTP egress poll. Native builds leave this queue idle and use real sockets.

use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Mutex, OnceLock};

static BRIDGE_ON: AtomicBool = AtomicBool::new(false);
static NEXT_ID: AtomicU32 = AtomicU32::new(1);

fn state() -> &'static Mutex<HostNetState> {
    static S: OnceLock<Mutex<HostNetState>> = OnceLock::new();
    S.get_or_init(|| Mutex::new(HostNetState::default()))
}

#[derive(Default)]
struct HostNetState {
    /// DNS queries waiting for the host.
    dns_pending: VecDeque<PendingDns>,
    /// Filled DNS answers (query id → A records).
    dns_answers: HashMap<u32, Vec<[u8; 4]>>,
    /// Context needed to build a DNS reply frame once answers arrive.
    dns_ctx: HashMap<u32, DnsCtx>,
    /// Ready DNS reply UDP payloads + routing (for inject into STA inbox).
    dns_replies: VecDeque<DnsReplyOut>,
    /// HTTP proxy requests waiting for the host.
    http_pending: VecDeque<PendingHttp>,
    /// Filled HTTP responses (id → raw HTTP/1.1 response bytes).
    http_answers: HashMap<u32, Vec<u8>>,
    /// Reverse map: resolved A → hostname (client DNS used the real name;
    /// later TCP to the IP must reconstruct `https://name/...` for host fetch).
    ip_to_name: HashMap<[u8; 4], String>,
}

#[derive(Clone, Debug)]
pub struct PendingDns {
    pub id: u32,
    pub name: String,
}

#[derive(Clone, Debug)]
struct DnsCtx {
    sta_mac: [u8; 6],
    client_ip: [u8; 4],
    ap_ip: [u8; 4],
    client_port: u16,
    /// Original DNS query payload (for building the response).
    query: Vec<u8>,
}

#[derive(Clone, Debug)]
pub struct DnsReplyOut {
    pub sta_mac: [u8; 6],
    pub client_ip: [u8; 4],
    pub ap_ip: [u8; 4],
    pub client_port: u16,
    pub udp_payload: Vec<u8>,
}

#[derive(Clone, Debug)]
pub struct PendingHttp {
    pub id: u32,
    /// Absolute URL the host should fetch (http or https).
    pub url: String,
    pub method: String,
    /// Raw HTTP/1.x request (headers + body) for host-side parsing if needed.
    pub raw_request: Vec<u8>,
}

/// Enable/disable the browser host-net bridge. When on, [`super::virtual_wifi_inet::internet_enabled`]
/// returns true on wasm.
pub fn set_bridge_active(active: bool) {
    BRIDGE_ON.store(active, Ordering::SeqCst);
    if !active {
        let mut s = state().lock().unwrap();
        *s = HostNetState::default();
    }
}

pub fn bridge_active() -> bool {
    BRIDGE_ON.load(Ordering::SeqCst)
}

/// Register a DNS query from a station. Returns the request id.
pub fn enqueue_dns(
    name: String,
    query: Vec<u8>,
    sta_mac: [u8; 6],
    client_ip: [u8; 4],
    ap_ip: [u8; 4],
    client_port: u16,
) -> u32 {
    let id = NEXT_ID.fetch_add(1, Ordering::SeqCst);
    let mut s = state().lock().unwrap();
    s.dns_pending.push_back(PendingDns {
        id,
        name: name.clone(),
    });
    s.dns_ctx.insert(
        id,
        DnsCtx {
            sta_mac,
            client_ip,
            ap_ip,
            client_port,
            query,
        },
    );
    id
}

/// Drain pending DNS names for the host to resolve.
pub fn poll_dns_requests() -> Vec<PendingDns> {
    let mut s = state().lock().unwrap();
    s.dns_pending.drain(..).collect()
}

/// Host supplies A records for a previously polled DNS request.
pub fn fulfill_dns(id: u32, ips: Vec<[u8; 4]>) {
    let mut s = state().lock().unwrap();
    if let Some(ctx) = s.dns_ctx.remove(&id) {
        // Remember IP → name so TCP NAT to that A can build a proper URL
        // (client's own browser then fetches that host — any host, not one API).
        let name = dns_qname(&ctx.query).unwrap_or_default();
        if !name.is_empty() {
            for ip in &ips {
                s.ip_to_name.insert(*ip, name.clone());
            }
        }
        let udp_payload = build_dns_response_from_query(&ctx.query, &ips);
        s.dns_replies.push_back(DnsReplyOut {
            sta_mac: ctx.sta_mac,
            client_ip: ctx.client_ip,
            ap_ip: ctx.ap_ip,
            client_port: ctx.client_port,
            udp_payload,
        });
    } else {
        s.dns_answers.insert(id, ips);
    }
}

/// Look up a hostname previously resolved for this IP (browser client DNS).
pub fn name_for_ip(ip: [u8; 4]) -> Option<String> {
    state().lock().unwrap().ip_to_name.get(&ip).cloned()
}

/// Record a name→IP binding (also used when the station uses Host: name).
pub fn remember_ip_name(ip: [u8; 4], name: &str) {
    if name.is_empty() {
        return;
    }
    state()
        .lock()
        .unwrap()
        .ip_to_name
        .insert(ip, name.to_string());
}

/// DNS replies ready to inject into station inboxes.
pub fn take_dns_replies() -> Vec<DnsReplyOut> {
    let mut s = state().lock().unwrap();
    s.dns_replies.drain(..).collect()
}

/// Register an HTTP proxy request (browser NAT for cleartext HTTP on 80/443
/// rewritten to fetch). Returns request id.
pub fn enqueue_http(url: String, method: String, raw_request: Vec<u8>) -> u32 {
    let id = NEXT_ID.fetch_add(1, Ordering::SeqCst);
    let mut s = state().lock().unwrap();
    s.http_pending.push_back(PendingHttp {
        id,
        url,
        method,
        raw_request,
    });
    id
}

pub fn poll_http_requests() -> Vec<PendingHttp> {
    let mut s = state().lock().unwrap();
    s.http_pending.drain(..).collect()
}

pub fn fulfill_http(id: u32, response: Vec<u8>) {
    state().lock().unwrap().http_answers.insert(id, response);
}

pub fn take_http_answer(id: u32) -> Option<Vec<u8>> {
    state().lock().unwrap().http_answers.remove(&id)
}

/// Build a DNS response UDP payload from the original query + A records.
fn build_dns_response_from_query(query: &[u8], ips: &[[u8; 4]]) -> Vec<u8> {
    if query.len() < 12 {
        return Vec::new();
    }
    let mut out = Vec::new();
    out.extend_from_slice(&query[0..2]); // id
    let flags = u16::from_be_bytes([query[2], query[3]]);
    let rd = flags & 0x0100;
    let rcode: u16 = if ips.is_empty() { 3 } else { 0 };
    let rflags = 0x8000u16 | 0x0400 | rd | 0x0080 | rcode;
    out.extend_from_slice(&rflags.to_be_bytes());
    out.extend_from_slice(&1u16.to_be_bytes());
    out.extend_from_slice(&(ips.len() as u16).to_be_bytes());
    out.extend_from_slice(&0u16.to_be_bytes());
    out.extend_from_slice(&0u16.to_be_bytes());
    // Copy question section from original (skip header 12 bytes to end of Q)
    let mut i = 12usize;
    while i < query.len() {
        let len = query[i] as usize;
        if len == 0 {
            i += 1;
            break;
        }
        if len & 0xC0 == 0xC0 {
            i += 2;
            break;
        }
        i += 1 + len;
    }
    i = (i + 4).min(query.len());
    out.extend_from_slice(&query[12..i]);
    for ip in ips {
        out.extend_from_slice(&[0xC0, 0x0C]);
        out.extend_from_slice(&1u16.to_be_bytes());
        out.extend_from_slice(&1u16.to_be_bytes());
        out.extend_from_slice(&60u32.to_be_bytes());
        out.extend_from_slice(&4u16.to_be_bytes());
        out.extend_from_slice(ip);
    }
    out
}

/// Parse QNAME from a DNS query payload.
pub fn dns_qname(query: &[u8]) -> Option<String> {
    if query.len() < 13 {
        return None;
    }
    let mut i = 12usize;
    let mut labels = Vec::new();
    while i < query.len() {
        let len = query[i] as usize;
        if len == 0 {
            break;
        }
        if len & 0xC0 == 0xC0 {
            return None;
        }
        i += 1;
        if i + len > query.len() {
            return None;
        }
        labels.push(std::str::from_utf8(&query[i..i + len]).ok()?.to_string());
        i += len;
    }
    Some(labels.join("."))
}

/// Extract method + URL path + Host from a raw HTTP/1.x request.
pub fn parse_http_request_line_host(raw: &[u8]) -> Option<(String, String, String)> {
    let text = std::str::from_utf8(raw).ok()?;
    let mut lines = text.split("\r\n");
    let req = lines.next()?;
    let mut parts = req.split_whitespace();
    let method = parts.next()?.to_string();
    let path = parts.next()?.to_string();
    let mut host = None;
    for line in lines {
        if line.is_empty() {
            break;
        }
        if let Some(rest) = line
            .strip_prefix("Host:")
            .or_else(|| line.strip_prefix("host:"))
        {
            host = Some(rest.trim().to_string());
        }
    }
    Some((method, path, host?))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dns_roundtrip_queue() {
        set_bridge_active(true);
        let id = enqueue_dns(
            "example.com".into(),
            {
                let mut q = vec![
                    0x12, 0x34, 0x01, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
                ];
                for lab in ["example", "com"] {
                    q.push(lab.len() as u8);
                    q.extend_from_slice(lab.as_bytes());
                }
                q.push(0);
                q.extend_from_slice(&[0x00, 0x01, 0x00, 0x01]);
                q
            },
            [2, 0, 0, 0, 0, 2],
            [192, 168, 4, 2],
            [192, 168, 4, 1],
            12345,
        );
        let pending = poll_dns_requests();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].id, id);
        fulfill_dns(id, vec![[93, 184, 216, 34]]);
        let replies = take_dns_replies();
        assert_eq!(replies.len(), 1);
        assert!(replies[0].udp_payload.len() > 12);
        assert_eq!(
            name_for_ip([93, 184, 216, 34]).as_deref(),
            Some("example.com")
        );
        set_bridge_active(false);
    }

    #[test]
    fn parse_http_host() {
        let raw = b"GET /v1/public-stats HTTP/1.1\r\nHost: api.labwired.com\r\n\r\n";
        let (m, p, h) = parse_http_request_line_host(raw).unwrap();
        assert_eq!(m, "GET");
        assert_eq!(p, "/v1/public-stats");
        assert_eq!(h, "api.labwired.com");
    }
}
