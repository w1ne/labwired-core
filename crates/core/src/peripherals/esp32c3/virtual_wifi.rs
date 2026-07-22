//! Virtual WiFi medium + infrastructure AP — the shared "air" that lets two (or
//! more) ESP32-C3 stations communicate over simulated 802.11, the WiFi analog of
//! the BLE `VirtualAir` (`nrf52::radio`).
//!
//! A process-global [`VirtualWifi`] holds, per associated station (keyed by its
//! MAC), an inbox of 802.11 frames awaiting delivery, plus the infrastructure
//! AP's state (assigned IPs). Each `wifi_mac` model:
//!   * submits every transmitted frame via [`submit`] (the AP processes it and
//!     queues any reply / routes data to the destination station), and
//!   * polls [`take_inbox`] each tick for frames addressed to its own MAC.
//!
//! Because the medium is a global shared across `Machine`/`WasmSimulator`
//! instances in the same process (exactly like the BLE virtual air), two C3
//! firmwares — each its own simulator instance with a distinct eFuse MAC — can
//! associate to the same virtual AP, get distinct DHCP leases, and exchange real
//! IP traffic, with the AP forwarding station-to-station data frames.
//!
//! The AP is OPEN (no WPA): beacon → probe/auth/assoc → DHCP DORA → ARP → routed
//! data. It models only what the real driver requires; the air-gap (radio) is
//! the single intentional cut. This is the same behaviour the single-device CLI
//! bridge proved, lifted into core and made MAC-aware so it scales to N stations.

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex, OnceLock};

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

/// Per-station state the AP tracks.
#[derive(Debug, Default)]
struct StaState {
    /// Frames queued for delivery to this station's RX.
    inbox: VecDeque<Vec<u8>>,
    /// DHCP-assigned IPv4 (0.0.0.0 until offered).
    ip: [u8; 4],
    /// 802.11 sequence counter for AP→this-STA frames (receiver dedups by it).
    ap_seq: u16,
}

#[derive(Debug, Default)]
struct VirtualWifi {
    stas: HashMap<[u8; 6], StaState>,
    next_host: u8,
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

    fn with<R>(&self, f: impl FnOnce(&mut VirtualWifi) -> R) -> R {
        f(&mut self.inner.lock().unwrap())
    }

    /// Reset the medium (tests / fresh runs).
    pub fn reset(&self) {
        self.with(|m| {
            m.stas.clear();
            m.next_host = 0;
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
            let frame = build_beacon(channel);
            m.enqueue(mac, frame);
        });
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
        let ip = [192, 168, 4, host];
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
                0x4 => self.enqueue(src, build_probe_resp(src, 1)),
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
            let reply = build_dhcp_reply(src, ip, xid, mtype == 3);
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
                    let who = if tpa == AP_IP {
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
        // IPv4: route to the destination station if we know it, else drop.
        if let Some((dst_ip, _proto)) = parse_ipv4_dst(frame) {
            if dst_ip == AP_IP {
                // The AP hosts a UDP echo server on UDP_ECHO_PORT (proves an
                // app-data round-trip); other traffic to the AP is dropped.
                if let Some(echo) = build_udp_echo(src, frame) {
                    self.enqueue(src, echo);
                }
                return;
            }
            if let Some(dst_mac) = self.mac_for_ip(dst_ip) {
                // Re-frame as a from-DS data frame to the destination station,
                // preserving the LLC/SNAP + IP payload.
                if let Some(routed) = reframe_to_sta(frame, dst_mac) {
                    self.enqueue(dst_mac, routed);
                }
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

fn build_beacon(channel: u8) -> Vec<u8> {
    let mut f = mgmt_hdr(0x80, [0xFF; 6]);
    f.extend_from_slice(&[0u8; 8]); // timestamp
    f.extend_from_slice(&[0x64, 0x00]); // beacon interval
    f.extend_from_slice(&[0x01, 0x00]); // capability: ESS, OPEN
    f.push(0x00);
    f.push(AP_SSID.len() as u8);
    f.extend_from_slice(AP_SSID.as_bytes());
    f.extend_from_slice(&[0x01, 0x08, 0x82, 0x84, 0x8b, 0x96, 0x0c, 0x12, 0x18, 0x24]);
    f.extend_from_slice(&[0x03, 0x01, channel]);
    f
}

fn build_probe_resp(da: [u8; 6], channel: u8) -> Vec<u8> {
    let mut f = build_beacon(channel);
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

fn build_dhcp_reply(da: [u8; 6], yiaddr: [u8; 4], xid: [u8; 4], ack: bool) -> Vec<u8> {
    let mut dhcp = vec![0x02, 0x01, 0x06, 0x00];
    dhcp.extend_from_slice(&xid);
    dhcp.extend_from_slice(&[0x00, 0x00, 0x80, 0x00]); // secs, broadcast flag
    dhcp.extend_from_slice(&[0, 0, 0, 0]); // ciaddr
    dhcp.extend_from_slice(&yiaddr);
    dhcp.extend_from_slice(&AP_IP); // siaddr
    dhcp.extend_from_slice(&[0, 0, 0, 0]); // giaddr
    dhcp.extend_from_slice(&da); // chaddr (6)
    dhcp.extend_from_slice(&[0u8; 10]);
    dhcp.extend_from_slice(&[0u8; 64]); // sname
    dhcp.extend_from_slice(&[0u8; 128]); // file
    dhcp.extend_from_slice(&[0x63, 0x82, 0x53, 0x63]); // magic
    dhcp.extend_from_slice(&[53, 1, if ack { 5 } else { 2 }]);
    dhcp.extend_from_slice(&[54, 4, AP_IP[0], AP_IP[1], AP_IP[2], AP_IP[3]]);
    dhcp.extend_from_slice(&[51, 4, 0x00, 0x01, 0x51, 0x80]); // lease
    dhcp.extend_from_slice(&[1, 4, NETMASK[0], NETMASK[1], NETMASK[2], NETMASK[3]]);
    dhcp.extend_from_slice(&[3, 4, AP_IP[0], AP_IP[1], AP_IP[2], AP_IP[3]]);
    dhcp.extend_from_slice(&[6, 4, AP_IP[0], AP_IP[1], AP_IP[2], AP_IP[3]]);
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
    ip.extend_from_slice(&AP_IP);
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

/// If `frame` is a UDP datagram to the AP's echo port, build the echoed reply
/// (same payload, src/dst swapped) as a from-DS data frame to the sender `da`.
fn build_udp_echo(da: [u8; 6], frame: &[u8]) -> Option<Vec<u8>> {
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
    iph.extend_from_slice(&AP_IP);
    iph.extend_from_slice(&src_ip);
    let cks = inet_checksum(&iph);
    iph[10] = (cks >> 8) as u8;
    iph[11] = cks as u8;
    iph.extend_from_slice(&u);
    Some(data_frame(da, 0x0800, &iph))
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
}
