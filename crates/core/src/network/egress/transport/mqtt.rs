// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! MQTT 3.1.1 publish transport (QoS 0). Packet layout mirrors the in-sim
//! broker in `network/mqtt.rs`. Connects lazily on the first `send`.

use crate::network::egress::transport::EgressTransport;
use std::io::{Read, Write};
use std::net::TcpStream;

pub struct MqttPublisher {
    host: String,
    port: u16,
    topic: String,
    stream: Option<TcpStream>,
}

impl MqttPublisher {
    /// Lazy: stores connection details; connects + CONNACK on first `send`.
    pub fn lazy(host: String, port: u16, topic: String) -> Self {
        Self {
            host,
            port,
            topic,
            stream: None,
        }
    }

    /// Eager: connects and completes the MQTT handshake (used by the net-tests).
    pub fn connect(host: &str, port: u16, topic: String) -> anyhow::Result<Self> {
        let mut me = Self::lazy(host.to_string(), port, topic);
        me.ensure_connected()?;
        Ok(me)
    }

    fn ensure_connected(&mut self) -> anyhow::Result<&mut TcpStream> {
        if self.stream.is_none() {
            let mut stream = TcpStream::connect((self.host.as_str(), self.port))?;
            stream.write_all(&connect_packet("labwired-egress"))?;
            let mut connack = [0u8; 4];
            stream.read_exact(&mut connack)?;
            anyhow::ensure!(connack[0] == 0x20, "unexpected MQTT CONNACK");
            self.stream = Some(stream);
        }
        Ok(self.stream.as_mut().unwrap())
    }
}

impl EgressTransport for MqttPublisher {
    fn send(&mut self, payload: &[u8]) -> anyhow::Result<()> {
        let packet = publish_packet(&self.topic.clone(), payload);
        let stream = self.ensure_connected()?;
        stream.write_all(&packet)?;
        stream.flush()?;
        Ok(())
    }
}

/// MQTT 3.1.1 CONNECT with clean session, no will/auth. Keep-alive is 0
/// (disabled): this QoS-0 fire-and-forget publisher never sends PINGREQ, so
/// advertising a non-zero interval would be a promise it can't keep.
fn connect_packet(client_id: &str) -> Vec<u8> {
    let mut var = Vec::new();
    var.extend_from_slice(&[0x00, 0x04]); // "MQTT" length
    var.extend_from_slice(b"MQTT");
    var.push(0x04); // protocol level 4 (3.1.1)
    var.push(0x02); // connect flags: clean session
    var.extend_from_slice(&[0x00, 0x00]); // keep-alive disabled
    let id = client_id.as_bytes();
    var.extend_from_slice(&(id.len() as u16).to_be_bytes());
    var.extend_from_slice(id);
    let mut pkt = vec![0x10];
    pkt.extend_from_slice(&remaining_length(var.len()));
    pkt.extend_from_slice(&var);
    pkt
}

/// MQTT PUBLISH, QoS 0 (no packet id).
fn publish_packet(topic: &str, payload: &[u8]) -> Vec<u8> {
    let mut var = Vec::new();
    let t = topic.as_bytes();
    var.extend_from_slice(&(t.len() as u16).to_be_bytes());
    var.extend_from_slice(t);
    var.extend_from_slice(payload);
    let mut pkt = vec![0x30];
    pkt.extend_from_slice(&remaining_length(var.len()));
    pkt.extend_from_slice(&var);
    pkt
}

/// MQTT variable-length "remaining length" encoding.
fn remaining_length(mut len: usize) -> Vec<u8> {
    let mut out = Vec::new();
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
    out
}

#[cfg(all(test, feature = "net-tests"))]
mod tests {
    use super::*;
    use crate::network::egress::transport::EgressTransport;
    use std::io::{Read, Write};
    use std::net::TcpListener;

    #[test]
    fn publishes_connect_then_publish() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let handle = std::thread::spawn(move || {
            let (mut sock, _) = listener.accept().unwrap();
            let mut first = [0u8; 1];
            sock.read_exact(&mut first).unwrap();
            assert_eq!(first[0], 0x10, "first packet must be CONNECT");
            // Drain the rest of the CONNECT variable header/payload.
            let mut scratch = [0u8; 64];
            let _ = sock.read(&mut scratch).unwrap();
            sock.write_all(&[0x20, 0x02, 0x00, 0x00]).unwrap(); // CONNACK
            let mut pubbyte = [0u8; 1];
            sock.read_exact(&mut pubbyte).unwrap();
            pubbyte[0]
        });
        let mut pubr =
            MqttPublisher::connect(&addr.ip().to_string(), addr.port(), "t/topic".to_string())
                .unwrap();
        pubr.send(b"hi").unwrap();
        assert_eq!(handle.join().unwrap(), 0x30, "expected PUBLISH");
    }
}
