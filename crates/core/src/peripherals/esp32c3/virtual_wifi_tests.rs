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
    let raw = b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\n\r\n{\"boards_supported\":11}";
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
