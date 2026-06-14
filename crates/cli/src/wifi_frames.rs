// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! Virtual Wi-Fi AP harness for the `wifi-bridge` command: pure 802.11 / DHCP /
//! ARP / UDP frame builders, parsers, and the AP responder. Extracted from
//! main.rs to keep the CLI entry point focused on command dispatch.
//!
//! NB: the esp32c3 `virtual_wifi` peripheral has its own (diverged) frame
//! helpers for the station side; these are the external-AP side and are kept
//! separate deliberately.

// Virtual-AP addressing.
pub const BRIDGE_BSSID: [u8; 6] = [0x02, 0x00, 0x00, 0x00, 0x00, 0x01];
pub const BRIDGE_STA: [u8; 6] = [0x00, 0x00, 0x00, 0x00, 0x00, 0x00];
pub const AP_IP: [u8; 4] = [192, 168, 4, 1];
pub const STA_IP: [u8; 4] = [192, 168, 4, 2];
pub const NETMASK: [u8; 4] = [255, 255, 255, 0];
pub const UDP_ECHO_PORT: u16 = 9999;

/// Fast-boot path for RISC-V chip descriptors (e.g. ESP32-C3).
///
/// Loads the chip's declarative peripherals from the YAML via
/// `SystemBus::from_config`, creates a RISC-V CPU, loads the ELF at its entry
/// point, and runs the step loop up to `max_steps`. UART output is echoed to
/// stdout, which is how the Tier-1 harness reads protocol lines
/// (`TIER1 <class> PASS|FAIL` / `TIER1 done`).
/// Build a minimal OPEN 802.11 beacon frame (no rx-control prefix; the C3 MAC
/// RX buffer holds the raw 802.11 frame from offset 0). Used by the WiFi bridge
/// to advertise a virtual AP the firmware's scan can find. BSSID 02:00:00:00:00:01.
pub fn build_open_beacon(ssid: &str, channel: u8) -> Vec<u8> {
    let bssid = [0x02u8, 0x00, 0x00, 0x00, 0x00, 0x01];
    let mut f = Vec::new();
    f.extend_from_slice(&[0x80, 0x00]); // frame control: mgmt / beacon
    f.extend_from_slice(&[0x00, 0x00]); // duration
    f.extend_from_slice(&[0xFF; 6]); // addr1 = broadcast
    f.extend_from_slice(&bssid); // addr2 = BSSID
    f.extend_from_slice(&bssid); // addr3 = BSSID
    f.extend_from_slice(&[0x00, 0x00]); // seq/frag
    f.extend_from_slice(&[0u8; 8]); // timestamp
    f.extend_from_slice(&[0x64, 0x00]); // beacon interval (100 TU)
    f.extend_from_slice(&[0x01, 0x00]); // capability: ESS, no privacy (OPEN)
    f.push(0x00); // SSID element id
    f.push(ssid.len() as u8);
    f.extend_from_slice(ssid.as_bytes());
    // Supported rates: 1,2,5.5,11,6,9,12,18 Mbps.
    f.extend_from_slice(&[0x01, 0x08, 0x82, 0x84, 0x8b, 0x96, 0x0c, 0x12, 0x18, 0x24]);
    f.extend_from_slice(&[0x03, 0x01, channel]); // DS param: current channel
    f
}

/// 802.11 mgmt header (24 bytes) for an AP→STA management frame of the given
/// subtype (in the high nibble of the frame-control byte).
pub fn mgmt_hdr(subtype_fc0: u8) -> Vec<u8> {
    let mut f = Vec::new();
    f.extend_from_slice(&[subtype_fc0, 0x00]); // frame control
    f.extend_from_slice(&[0x00, 0x00]); // duration
    f.extend_from_slice(&BRIDGE_STA); // addr1 = DA (the STA)
    f.extend_from_slice(&BRIDGE_BSSID); // addr2 = SA (AP)
    f.extend_from_slice(&BRIDGE_BSSID); // addr3 = BSSID
    f.extend_from_slice(&[0x00, 0x00]); // seq/frag
    f
}

/// OPEN-system authentication response (algorithm 0, transaction seq 2, status
/// 0 = success). Frame-control subtype 11 (auth) → FC byte 0xB0.
pub fn build_auth_resp() -> Vec<u8> {
    let mut f = mgmt_hdr(0xB0);
    f.extend_from_slice(&[0x00, 0x00]); // auth algorithm = open
    f.extend_from_slice(&[0x02, 0x00]); // auth transaction seq = 2
    f.extend_from_slice(&[0x00, 0x00]); // status = success
    f
}

/// Association response (status 0 = success, AID 1). FC subtype 1 → 0x10.
pub fn build_assoc_resp() -> Vec<u8> {
    let mut f = mgmt_hdr(0x10);
    f.extend_from_slice(&[0x01, 0x00]); // capability info (ESS)
    f.extend_from_slice(&[0x00, 0x00]); // status = success
    f.extend_from_slice(&[0x01, 0xC0]); // AID = 1 (top 2 bits set per spec)
    f.extend_from_slice(&[0x01, 0x08, 0x82, 0x84, 0x8b, 0x96, 0x0c, 0x12, 0x18, 0x24]); // rates
    f
}

/// One's-complement Internet checksum over a byte slice.
pub fn inet_checksum(data: &[u8]) -> u16 {
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

/// Parse a captured TX frame; if it's a DHCP discover (1) or request (3) from
/// the STA, return (xid[4], dhcp_message_type). The captured frame is raw
/// 802.11: [data hdr 24/26][LLC/SNAP 8][IPv4][UDP][DHCP].
pub fn parse_dhcp_request(frame: &[u8]) -> Option<([u8; 4], u8)> {
    if frame.len() < 2 || (frame[0] >> 2) & 3 != 2 {
        return None; // not a data frame
    }
    let hdr = if frame[0] & 0x80 != 0 { 26 } else { 24 };
    let snap = hdr + 8;
    let ip = snap; // IPv4 header start
    if frame.len() < ip + 20 || frame[ip] >> 4 != 4 {
        return None;
    }
    let ihl = (frame[ip] & 0xF) as usize * 4;
    if frame[ip + 9] != 17 {
        return None; // not UDP
    }
    let udp = ip + ihl;
    if frame.len() < udp + 8 {
        return None;
    }
    let dport = u16::from_be_bytes([frame[udp + 2], frame[udp + 3]]);
    if dport != 67 {
        return None; // not DHCP server port
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
    // DHCP options start after the 236-byte fixed part + 4-byte magic cookie.
    let mut o = dhcp + 240;
    let mut msg_type = 0u8;
    while o + 1 < frame.len() {
        let opt = frame[o];
        if opt == 255 {
            break;
        }
        if opt == 0 {
            o += 1;
            continue;
        }
        let l = frame[o + 1] as usize;
        if opt == 53 && l >= 1 {
            msg_type = frame[o + 2];
        }
        o += 2 + l;
    }
    Some((xid, msg_type))
}

/// Build a DHCP reply (offer if !ack, ack if ack) for the captured request,
/// fully encapsulated as an AP→STA 802.11 data frame ready for RX injection.
pub fn build_dhcp_reply(xid: [u8; 4], ack: bool) -> Vec<u8> {
    // ── DHCP payload ──
    let mut dhcp = Vec::new();
    dhcp.push(0x02); // op = BOOTREPLY
    dhcp.extend_from_slice(&[0x01, 0x06, 0x00]); // htype=eth, hlen=6, hops=0
    dhcp.extend_from_slice(&xid);
    dhcp.extend_from_slice(&[0x00, 0x00, 0x80, 0x00]); // secs=0, flags=broadcast
    dhcp.extend_from_slice(&[0, 0, 0, 0]); // ciaddr
    dhcp.extend_from_slice(&STA_IP); // yiaddr
    dhcp.extend_from_slice(&AP_IP); // siaddr (next server)
    dhcp.extend_from_slice(&[0, 0, 0, 0]); // giaddr
    dhcp.extend_from_slice(&BRIDGE_STA); // chaddr (6)
    dhcp.extend_from_slice(&[0u8; 10]); // chaddr padding to 16
    dhcp.extend_from_slice(&[0u8; 64]); // sname
    dhcp.extend_from_slice(&[0u8; 128]); // file
    dhcp.extend_from_slice(&[0x63, 0x82, 0x53, 0x63]); // magic cookie
    dhcp.extend_from_slice(&[53, 1, if ack { 5 } else { 2 }]); // msg type: ACK/OFFER
    dhcp.extend_from_slice(&[54, 4, AP_IP[0], AP_IP[1], AP_IP[2], AP_IP[3]]); // server id
    dhcp.extend_from_slice(&[51, 4, 0x00, 0x01, 0x51, 0x80]); // lease 86400s
    dhcp.extend_from_slice(&[1, 4, NETMASK[0], NETMASK[1], NETMASK[2], NETMASK[3]]); // subnet
    dhcp.extend_from_slice(&[3, 4, AP_IP[0], AP_IP[1], AP_IP[2], AP_IP[3]]); // router
    dhcp.extend_from_slice(&[6, 4, AP_IP[0], AP_IP[1], AP_IP[2], AP_IP[3]]); // dns
    dhcp.push(255); // end

    // ── UDP (src 67 → dst 68), checksum 0 (optional for IPv4) ──
    let udp_len = (8 + dhcp.len()) as u16;
    let mut udp = Vec::new();
    udp.extend_from_slice(&67u16.to_be_bytes());
    udp.extend_from_slice(&68u16.to_be_bytes());
    udp.extend_from_slice(&udp_len.to_be_bytes());
    udp.extend_from_slice(&[0, 0]); // checksum 0
    udp.extend_from_slice(&dhcp);

    // ── IPv4 (AP → 255.255.255.255 broadcast) ──
    let ip_total = (20 + udp.len()) as u16;
    let mut ip = vec![
        0x45,
        0x00, // ver/ihl, dscp
        (ip_total >> 8) as u8,
        ip_total as u8,
        0x00,
        0x00,
        0x00,
        0x00, // id, flags/frag
        0x40,
        0x11, // ttl=64, proto=UDP
        0x00,
        0x00, // checksum (filled below)
    ];
    ip.extend_from_slice(&AP_IP);
    ip.extend_from_slice(&[255, 255, 255, 255]); // dst broadcast
    let cks = inet_checksum(&ip);
    ip[10] = (cks >> 8) as u8;
    ip[11] = cks as u8;
    ip.extend_from_slice(&udp);

    // ── 802.11 data frame, from-DS (AP→STA): FC 0x08 0x02 ──
    let mut f = Vec::new();
    f.extend_from_slice(&[0x08, 0x02]); // data, from-DS
    f.extend_from_slice(&[0x00, 0x00]); // duration
    f.extend_from_slice(&BRIDGE_STA); // addr1 = DA (STA)
    f.extend_from_slice(&BRIDGE_BSSID); // addr2 = BSSID
    f.extend_from_slice(&BRIDGE_BSSID); // addr3 = SA (server)
    f.extend_from_slice(&[0x00, 0x00]); // seq/frag
    f.extend_from_slice(&[0xAA, 0xAA, 0x03, 0x00, 0x00, 0x00, 0x08, 0x00]); // LLC/SNAP, IPv4
    f.extend_from_slice(&ip);
    f
}

/// Probe response: like a beacon but unicast to the STA (subtype 5 → FC 0x50),
/// answering an active scan's probe request.
pub fn build_probe_resp(ssid: &str, channel: u8) -> Vec<u8> {
    let mut f = build_open_beacon(ssid, channel);
    f[0] = 0x50; // mgmt subtype 5 = probe response
    f[4..10].copy_from_slice(&BRIDGE_STA); // addr1 = the scanning STA (unicast)
    f
}

/// Parse a captured TX data frame as ARP: returns (oper, sender_ip, target_ip)
/// if it carries an ARP packet (ethertype 0x0806 after LLC/SNAP).
pub fn parse_arp(frame: &[u8]) -> Option<(u16, [u8; 4], [u8; 4])> {
    if frame.len() < 2 || (frame[0] >> 2) & 3 != 2 {
        return None; // not a data frame
    }
    let hdr = if frame[0] & 0x80 != 0 { 26 } else { 24 };
    let snap = hdr + 8;
    // LLC/SNAP ethertype is the 2 bytes just before the payload.
    if frame.len() < snap + 28 {
        return None;
    }
    let ethertype = u16::from_be_bytes([frame[snap - 2], frame[snap - 1]]);
    if ethertype != 0x0806 {
        return None;
    }
    let a = snap; // ARP packet start
    let oper = u16::from_be_bytes([frame[a + 6], frame[a + 7]]);
    let mut spa = [0u8; 4];
    spa.copy_from_slice(&frame[a + 14..a + 18]);
    let mut tpa = [0u8; 4];
    tpa.copy_from_slice(&frame[a + 24..a + 28]);
    Some((oper, spa, tpa))
}

/// ARP reply (oper 2): "`who_ip` is at the AP's BSSID", addressed to the STA.
/// Wrapped as an AP→STA 802.11 from-DS data frame with LLC/SNAP ethertype 0x0806.
pub fn build_arp_reply(who_ip: [u8; 4], target_ip: [u8; 4]) -> Vec<u8> {
    let mut arp = Vec::new();
    arp.extend_from_slice(&[0x00, 0x01]); // htype = Ethernet
    arp.extend_from_slice(&[0x08, 0x00]); // ptype = IPv4
    arp.extend_from_slice(&[0x06, 0x04]); // hlen, plen
    arp.extend_from_slice(&[0x00, 0x02]); // oper = reply
    arp.extend_from_slice(&BRIDGE_BSSID); // sender hw = AP
    arp.extend_from_slice(&who_ip); // sender proto = the resolved IP
    arp.extend_from_slice(&BRIDGE_STA); // target hw = STA
    arp.extend_from_slice(&target_ip); // target proto = STA's IP

    let mut f = Vec::new();
    f.extend_from_slice(&[0x08, 0x02]); // data, from-DS
    f.extend_from_slice(&[0x00, 0x00]); // duration
    f.extend_from_slice(&BRIDGE_STA); // addr1 = DA (STA)
    f.extend_from_slice(&BRIDGE_BSSID); // addr2 = BSSID
    f.extend_from_slice(&BRIDGE_BSSID); // addr3 = SA
    f.extend_from_slice(&[0x00, 0x00]); // seq/frag
    f.extend_from_slice(&[0xAA, 0xAA, 0x03, 0x00, 0x00, 0x00, 0x08, 0x06]); // LLC/SNAP, ARP
    f.extend_from_slice(&arp);
    f
}

/// Event-driven virtual-AP logic: given one frame the STA just transmitted,
/// return the frames the AP should inject in response (each with a short label
/// for logging). This is the single place that models AP behaviour, so the same
/// STA TX always produces the same response regardless of sim timing — which is
/// what makes association + DHCP deterministic rather than flaky.
pub fn ap_respond(tx: &[u8]) -> Vec<(Vec<u8>, &'static str)> {
    if tx.len() < 2 {
        return Vec::new();
    }
    let ftype = (tx[0] >> 2) & 3; // 0=mgmt, 2=data
    let subtype = tx[0] >> 4;
    let mut out: Vec<(Vec<u8>, &'static str)> = Vec::new();
    if ftype == 0 {
        // Management.
        match subtype {
            0x4 => out.push((build_probe_resp("labwired-ap", 1), "probe-req->resp")),
            0xB => out.push((build_auth_resp(), "auth-req->resp")),
            0x0 | 0x2 => out.push((build_assoc_resp(), "assoc-req->resp")),
            _ => {}
        }
        return out;
    }
    if ftype == 2 {
        // Data: DHCP or ARP.
        if let Some((xid, mtype)) = parse_dhcp_request(tx) {
            // discover(1) → offer; request(3) → ack.
            let label = if mtype == 3 {
                "dhcp-request->ack"
            } else {
                "dhcp-discover->offer"
            };
            out.push((build_dhcp_reply(xid, mtype == 3), label));
        } else if let Some((oper, _spa, tpa)) = parse_arp(tx) {
            // Only answer ARP *requests* (oper 1). Crucially, do NOT answer the
            // DHCP CHECKING self-probe (target == STA's own offered IP) — that
            // would trigger lwIP dhcp_arp_reply→dhcp_decline and abort the bind.
            // Answer the gateway ARP (target == AP_IP) so post-bind traffic flows.
            if oper == 1 && tpa != STA_IP {
                out.push((build_arp_reply(tpa, STA_IP), "arp-req->reply"));
            }
        } else if let Some(echo) = build_udp_echo(tx, UDP_ECHO_PORT) {
            // A tiny UDP echo server at AP_IP:UDP_ECHO_PORT — proves real
            // bidirectional socket data over the simulated WiFi.
            out.push((echo, "udp->echo"));
        }
    }
    out
}

/// If the captured TX frame is a UDP datagram from the STA to the AP's echo
/// port, build the echoed reply (same payload) as an AP→STA 802.11 data frame.
pub fn build_udp_echo(frame: &[u8], echo_port: u16) -> Option<Vec<u8>> {
    if frame.len() < 2 || (frame[0] >> 2) & 3 != 2 {
        return None;
    }
    let hdr = if frame[0] & 0x80 != 0 { 26 } else { 24 };
    let snap = hdr + 8;
    let ip = snap;
    if frame.len() < ip + 20 || frame[ip] >> 4 != 4 || frame[ip + 9] != 17 {
        return None; // not IPv4/UDP
    }
    let ihl = (frame[ip] & 0xF) as usize * 4;
    let udp = ip + ihl;
    if frame.len() < udp + 8 {
        return None;
    }
    let sport = u16::from_be_bytes([frame[udp], frame[udp + 1]]);
    let dport = u16::from_be_bytes([frame[udp + 2], frame[udp + 3]]);
    if dport != echo_port {
        return None;
    }
    let ulen = u16::from_be_bytes([frame[udp + 4], frame[udp + 5]]) as usize;
    if ulen < 8 || frame.len() < udp + ulen {
        return None;
    }
    let payload = &frame[udp + 8..udp + ulen];

    // Echo: UDP src=echo_port → dst=sport, same payload, checksum 0.
    let udp_len = (8 + payload.len()) as u16;
    let mut u = Vec::new();
    u.extend_from_slice(&echo_port.to_be_bytes());
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
        0x00,
        0x00,
        0x00,
        0x00,
        0x40,
        0x11,
        0x00,
        0x00,
    ];
    iph.extend_from_slice(&AP_IP);
    iph.extend_from_slice(&STA_IP);
    let cks = inet_checksum(&iph);
    iph[10] = (cks >> 8) as u8;
    iph[11] = cks as u8;
    iph.extend_from_slice(&u);

    let mut f = Vec::new();
    f.extend_from_slice(&[0x08, 0x02]); // data, from-DS
    f.extend_from_slice(&[0x00, 0x00]);
    f.extend_from_slice(&BRIDGE_STA); // addr1 = DA (STA)
    f.extend_from_slice(&BRIDGE_BSSID); // addr2 = BSSID
    f.extend_from_slice(&BRIDGE_BSSID); // addr3 = SA
    f.extend_from_slice(&[0x00, 0x00]);
    f.extend_from_slice(&[0xAA, 0xAA, 0x03, 0x00, 0x00, 0x00, 0x08, 0x00]); // LLC/SNAP IPv4
    f.extend_from_slice(&iph);
    Some(f)
}

/// Short human label for a captured STA TX frame, for bridge tracing.
pub fn tx_kind(tx: &[u8]) -> String {
    if tx.len() < 2 {
        return "runt".into();
    }
    let ftype = (tx[0] >> 2) & 3;
    let subtype = tx[0] >> 4;
    match (ftype, subtype) {
        (0, 0x4) => "mgmt/probe-req".into(),
        (0, 0xB) => "mgmt/auth".into(),
        (0, 0x0) => "mgmt/assoc-req".into(),
        (0, 0x2) => "mgmt/reassoc-req".into(),
        (0, s) => format!("mgmt/sub{s:#x}"),
        (1, _) => "ctrl".into(),
        (2, _) => {
            if let Some((_x, mt)) = parse_dhcp_request(tx) {
                format!("data/dhcp(type={mt})")
            } else if let Some((op, spa, tpa)) = parse_arp(tx) {
                format!(
                    "data/arp(op={op} {}.{}.{}.{}→{}.{}.{}.{})",
                    spa[0], spa[1], spa[2], spa[3], tpa[0], tpa[1], tpa[2], tpa[3]
                )
            } else {
                // Decode the LLC/SNAP ethertype + a payload preview so unknown
                // data frames (post-bind traffic, declines, etc.) are identifiable.
                let hdr = if tx[0] & 0x80 != 0 { 26 } else { 24 };
                let snap = hdr + 8;
                if tx.len() >= snap + 4 {
                    let et = u16::from_be_bytes([tx[snap - 2], tx[snap - 1]]);
                    let prev: Vec<String> = tx[snap..(snap + 24).min(tx.len())]
                        .iter()
                        .map(|b| format!("{b:02x}"))
                        .collect();
                    // For IPv4, surface proto + dst port so UDP services (mDNS,
                    // DNS, NTP) and TCP are obvious.
                    if et == 0x0800 && tx.len() >= snap + 20 {
                        let proto = tx[snap + 9];
                        let ihl = (tx[snap] & 0xF) as usize * 4;
                        let l4 = snap + ihl;
                        let dport = if tx.len() >= l4 + 4 {
                            u16::from_be_bytes([tx[l4 + 2], tx[l4 + 3]])
                        } else {
                            0
                        };
                        let dst = &tx[snap + 16..snap + 20];
                        format!(
                            "data/ipv4(proto={proto} dport={dport} dst={}.{}.{}.{})",
                            dst[0], dst[1], dst[2], dst[3]
                        )
                    } else {
                        format!("data/other(et={et:#06x} {})", prev.join(" "))
                    }
                } else {
                    "data/other".into()
                }
            }
        }
        _ => "?".into(),
    }
}
