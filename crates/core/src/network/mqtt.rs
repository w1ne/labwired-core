// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! In-sim MQTT 3.1.1 broker — a [`SimServer`](crate::network::sim::SimServer)
//! endpoint for the simulated-endpoints WiFi model.
//!
//! Scope: a **loopback single-client** broker. It speaks enough MQTT 3.1.1
//! to satisfy a connecting client's handshake and a publish/subscribe smoke
//! test against itself — the canonical IoT firmware check "connect →
//! subscribe `T` → publish `T` → receive my own message":
//!
//!   * `CONNECT`   → `CONNACK` (accepted, no auth).
//!   * `SUBSCRIBE` → `SUBACK` (granted QoS 0) and the filter is remembered.
//!   * `PUBLISH`   → `PUBACK` when QoS 1; and if the topic matches a live
//!     subscription, the message is looped straight back as a QoS-0
//!     `PUBLISH` in the same response stream.
//!   * `PINGREQ`   → `PINGRESP`.
//!   * `DISCONNECT`/`UNSUBSCRIBE` are accepted (UNSUBSCRIBE → `UNSUBACK`).
//!
//! Topic matching supports the MQTT wildcards `+` (single level) and `#`
//! (multi level). Cross-client routing (a real broker fan-out) needs a
//! server→connection push channel the `SimServer` model doesn't have yet;
//! that's a deliberate follow-up.
//!
//! Per-connection subscription state lives behind a `Mutex` since the broker
//! is shared (`Arc<dyn SimServer>`) across connections; this loopback model
//! assumes one active client, which the WiFi smoke tests use.

use super::sim::SimServer;
use std::sync::Mutex;

/// MQTT control packet type (high nibble of the fixed header).
mod packet {
    pub const CONNECT: u8 = 1;
    pub const CONNACK: u8 = 2;
    pub const PUBLISH: u8 = 3;
    pub const PUBACK: u8 = 4;
    pub const SUBSCRIBE: u8 = 8;
    pub const SUBACK: u8 = 9;
    pub const UNSUBSCRIBE: u8 = 10;
    pub const UNSUBACK: u8 = 11;
    pub const PINGREQ: u8 = 12;
    pub const PINGRESP: u8 = 13;
    pub const DISCONNECT: u8 = 14;
}

/// Loopback MQTT 3.1.1 broker. See module docs.
#[derive(Debug, Default)]
pub struct MqttBroker {
    subscriptions: Mutex<Vec<String>>,
}

impl MqttBroker {
    pub fn new() -> Self {
        Self::default()
    }

    /// Encode an MQTT "remaining length" varint (1–4 bytes).
    fn encode_remaining_length(mut len: usize, out: &mut Vec<u8>) {
        loop {
            let mut byte = (len % 128) as u8;
            len /= 128;
            if len > 0 {
                byte |= 0x80;
            }
            out.push(byte);
            if len == 0 {
                break;
            }
        }
    }

    /// Decode an MQTT "remaining length" varint at `pos`, advancing it.
    /// Returns `None` on a malformed (>4 byte) field or truncation.
    fn decode_remaining_length(buf: &[u8], pos: &mut usize) -> Option<usize> {
        let mut multiplier = 1usize;
        let mut value = 0usize;
        for _ in 0..4 {
            let byte = *buf.get(*pos)?;
            *pos += 1;
            value += (byte & 0x7F) as usize * multiplier;
            if byte & 0x80 == 0 {
                return Some(value);
            }
            multiplier *= 128;
        }
        None
    }

    /// Read a 2-byte big-endian length-prefixed UTF-8 string at `pos`.
    fn read_string(buf: &[u8], pos: &mut usize) -> Option<String> {
        let len = ((*buf.get(*pos)? as usize) << 8) | (*buf.get(*pos + 1)? as usize);
        *pos += 2;
        let bytes = buf.get(*pos..*pos + len)?;
        *pos += len;
        String::from_utf8(bytes.to_vec()).ok()
    }

    /// True if MQTT topic `name` matches subscription `filter` (`+`/`#`).
    fn topic_matches(filter: &str, name: &str) -> bool {
        let mut f = filter.split('/');
        let mut n = name.split('/');
        loop {
            match (f.next(), n.next()) {
                (Some("#"), _) => return true,
                (Some("+"), Some(_)) => continue,
                (Some(a), Some(b)) if a == b => continue,
                (None, None) => return true,
                _ => return false,
            }
        }
    }

    /// Build a QoS-0 PUBLISH packet for `topic`/`payload`.
    fn encode_publish(topic: &str, payload: &[u8]) -> Vec<u8> {
        let mut var = Vec::new();
        var.push((topic.len() >> 8) as u8);
        var.push((topic.len() & 0xFF) as u8);
        var.extend_from_slice(topic.as_bytes());
        var.extend_from_slice(payload);
        let mut out = vec![packet::PUBLISH << 4];
        Self::encode_remaining_length(var.len(), &mut out);
        out.extend_from_slice(&var);
        out
    }

    /// Handle one decoded control packet; append any reply bytes to `out`.
    fn handle_packet(&self, header: u8, body: &[u8], out: &mut Vec<u8>) {
        let kind = header >> 4;
        match kind {
            packet::CONNECT => {
                // CONNACK: session-present=0, return code 0 (accepted).
                out.extend_from_slice(&[packet::CONNACK << 4, 0x02, 0x00, 0x00]);
            }
            packet::SUBSCRIBE => {
                // [packet id(2)] then (topic + requested-qos) pairs.
                let mut pos = 0usize;
                let pid_hi = *body.first().unwrap_or(&0);
                let pid_lo = *body.get(1).unwrap_or(&0);
                pos += 2;
                let mut granted = Vec::new();
                while pos < body.len() {
                    let Some(topic) = Self::read_string(body, &mut pos) else {
                        break;
                    };
                    // Skip the requested QoS byte; we always grant QoS 0.
                    pos += 1;
                    self.subscriptions.lock().unwrap().push(topic);
                    granted.push(0u8);
                }
                let mut var = vec![pid_hi, pid_lo];
                var.extend_from_slice(&granted);
                out.push(packet::SUBACK << 4);
                Self::encode_remaining_length(var.len(), out);
                out.extend_from_slice(&var);
            }
            packet::UNSUBSCRIBE => {
                let mut pos = 0usize;
                let pid_hi = *body.first().unwrap_or(&0);
                let pid_lo = *body.get(1).unwrap_or(&0);
                pos += 2;
                let mut subs = self.subscriptions.lock().unwrap();
                while pos < body.len() {
                    let Some(topic) = Self::read_string(body, &mut pos) else {
                        break;
                    };
                    subs.retain(|t| t != &topic);
                }
                out.extend_from_slice(&[packet::UNSUBACK << 4, 0x02, pid_hi, pid_lo]);
            }
            packet::PUBLISH => {
                let qos = (header >> 1) & 0x03;
                let mut pos = 0usize;
                let Some(topic) = Self::read_string(body, &mut pos) else {
                    return;
                };
                if qos > 0 {
                    // QoS 1/2 carry a 2-byte packet id before the payload.
                    let pid_hi = *body.get(pos).unwrap_or(&0);
                    let pid_lo = *body.get(pos + 1).unwrap_or(&0);
                    pos += 2;
                    if qos == 1 {
                        out.extend_from_slice(&[packet::PUBACK << 4, 0x02, pid_hi, pid_lo]);
                    }
                }
                let payload = &body[pos.min(body.len())..];
                // Loopback delivery: if we're subscribed to this topic, send
                // the message straight back as a QoS-0 PUBLISH.
                let subscribed = self
                    .subscriptions
                    .lock()
                    .unwrap()
                    .iter()
                    .any(|f| Self::topic_matches(f, &topic));
                if subscribed {
                    out.extend_from_slice(&Self::encode_publish(&topic, payload));
                }
            }
            packet::PINGREQ => {
                out.extend_from_slice(&[packet::PINGRESP << 4, 0x00]);
            }
            packet::DISCONNECT => {}
            _ => {}
        }
    }
}

impl SimServer for MqttBroker {
    fn on_data(&self, _conn: u32, data: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        let mut pos = 0usize;
        // Process every complete control packet present in this chunk.
        while pos < data.len() {
            let header = data[pos];
            let mut after = pos + 1;
            let Some(rem) = Self::decode_remaining_length(data, &mut after) else {
                break;
            };
            let body_end = after + rem;
            if body_end > data.len() {
                break; // partial packet — wait for more (not modeled across sends)
            }
            self.handle_packet(header, &data[after..body_end], &mut out);
            pos = body_end;
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::network::sim::{SimNet, SimServer};
    use std::net::{Ipv4Addr, SocketAddrV4};
    use std::sync::Arc;

    // Minimal client-side packet builders.
    fn connect() -> Vec<u8> {
        // CONNECT with empty payload-ish header; the broker ignores the body.
        let var = b"\x00\x04MQTT\x04\x02\x00\x3c"; // protocol name/level/flags/keepalive
        let mut out = vec![super::packet::CONNECT << 4];
        MqttBroker::encode_remaining_length(var.len(), &mut out);
        out.extend_from_slice(var);
        out
    }
    fn subscribe(pid: u16, topic: &str) -> Vec<u8> {
        let mut var = vec![(pid >> 8) as u8, (pid & 0xFF) as u8];
        var.push((topic.len() >> 8) as u8);
        var.push((topic.len() & 0xFF) as u8);
        var.extend_from_slice(topic.as_bytes());
        var.push(0); // requested qos
        let mut out = vec![(super::packet::SUBSCRIBE << 4) | 0x02];
        MqttBroker::encode_remaining_length(var.len(), &mut out);
        out.extend_from_slice(&var);
        out
    }
    fn publish(topic: &str, payload: &[u8]) -> Vec<u8> {
        MqttBroker::encode_publish(topic, payload) // QoS 0
    }

    #[test]
    fn connect_yields_connack() {
        let b = MqttBroker::new();
        let r = b.on_data(0, &connect());
        assert_eq!(r, vec![super::packet::CONNACK << 4, 0x02, 0x00, 0x00]);
    }

    #[test]
    fn subscribe_yields_suback_granted_qos0() {
        let b = MqttBroker::new();
        let r = b.on_data(0, &subscribe(1, "sensors/temp"));
        // SUBACK header, remaining len 3, pid 0x0001, granted 0x00.
        assert_eq!(r, vec![super::packet::SUBACK << 4, 0x03, 0x00, 0x01, 0x00]);
    }

    #[test]
    fn publish_to_subscribed_topic_loops_back() {
        let b = MqttBroker::new();
        let _ = b.on_data(0, &subscribe(1, "sensors/+"));
        let r = b.on_data(0, &publish("sensors/temp", b"23.5"));
        // Expect a PUBLISH back with the same topic + payload.
        assert_eq!(r[0] >> 4, super::packet::PUBLISH);
        let text = String::from_utf8_lossy(&r);
        assert!(text.contains("sensors/temp"), "{text:?}");
        assert!(text.ends_with("23.5"), "{text:?}");
    }

    #[test]
    fn publish_to_unsubscribed_topic_is_silent() {
        let b = MqttBroker::new();
        let _ = b.on_data(0, &subscribe(1, "other/#"));
        let r = b.on_data(0, &publish("sensors/temp", b"x"));
        assert!(r.is_empty());
    }

    #[test]
    fn pingreq_yields_pingresp() {
        let b = MqttBroker::new();
        let r = b.on_data(0, &[super::packet::PINGREQ << 4, 0x00]);
        assert_eq!(r, vec![super::packet::PINGRESP << 4, 0x00]);
    }

    #[test]
    fn full_flow_over_simnet() {
        // End-to-end through SimNet: connect, subscribe, publish, recv own msg.
        let mut net = SimNet::new();
        let addr = SocketAddrV4::new(Ipv4Addr::new(192, 168, 4, 1), 1883);
        net.listen(addr, Arc::new(MqttBroker::new()));
        let c = net.connect(addr).unwrap();

        net.send(c, &connect()).unwrap();
        assert_eq!(net.recv(c)[0] >> 4, super::packet::CONNACK);

        net.send(c, &subscribe(1, "dev/+/cmd")).unwrap();
        assert_eq!(net.recv(c)[0] >> 4, super::packet::SUBACK);

        net.send(c, &publish("dev/42/cmd", b"on")).unwrap();
        let msg = net.recv(c);
        assert_eq!(msg[0] >> 4, super::packet::PUBLISH);
        assert!(String::from_utf8_lossy(&msg).ends_with("on"));
    }

    #[test]
    fn topic_wildcards() {
        assert!(MqttBroker::topic_matches("a/+/c", "a/b/c"));
        assert!(MqttBroker::topic_matches("a/#", "a/b/c/d"));
        assert!(MqttBroker::topic_matches("a/b", "a/b"));
        assert!(!MqttBroker::topic_matches("a/+", "a/b/c"));
        assert!(!MqttBroker::topic_matches("a/b", "a/c"));
    }
}
