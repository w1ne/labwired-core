// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Lightweight **cellular MQTT fabric** — the network-side peer for Quectel
//! BG770A AT MQTT (`QMTOPEN` / `QMTCONN` / `QMTSUB` / `QMTPUB`).
//!
//! This is **not** a full cellular core or MQTT 3.1.1 wire broker. It is enough
//! so that firmware "networks work":
//!
//!   * Opens/connects are tracked per modem endpoint + client index.
//!   * Publishes are retained in a ring log (inspect / smoke / multi-node).
//!   * Subscribers on the same fabric receive `+QMTRECV` deliveries (loopback
//!     and cross-modem), topic-matched with `+` / `#` wildcards.
//!
//! Single-board labs share a process-default bus; multi-chip worlds can mint a
//! private bus and attach every modem to it (same pattern as VirtualAirBus).

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::{Arc, Mutex, OnceLock};

/// One retained publish for forensics / smoke.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CellularPublish {
    pub endpoint: String,
    pub client_id: u8,
    pub broker_host: String,
    pub broker_port: u16,
    pub topic: String,
    pub payload: Vec<u8>,
}

/// Pending downlink for a modem endpoint (becomes `+QMTRECV`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CellularDelivery {
    pub client_id: u8,
    pub topic: String,
    pub payload: Vec<u8>,
}

#[derive(Debug, Default)]
struct EndpointState {
    /// Connected MQTT client indices (after QMTCONN).
    connected: HashSet<u8>,
    /// Subscriptions per client index.
    subs: HashMap<u8, Vec<String>>,
    /// Broker host/port last opened per client (informational).
    brokers: HashMap<u8, (String, u16)>,
    /// Downlink queue for this modem instance.
    pending: VecDeque<CellularDelivery>,
}

#[derive(Debug, Default)]
struct Inner {
    endpoints: HashMap<String, EndpointState>,
    /// Most-recent-first cap.
    log: VecDeque<CellularPublish>,
    log_cap: usize,
}

/// Shared fabric: clone freely (`Arc` inside).
#[derive(Debug, Clone)]
pub struct CellularMqttBus {
    inner: Arc<Mutex<Inner>>,
}

impl Default for CellularMqttBus {
    fn default() -> Self {
        Self::new()
    }
}

impl CellularMqttBus {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(Inner {
                endpoints: HashMap::new(),
                log: VecDeque::new(),
                log_cap: 64,
            })),
        }
    }

    /// Process-default bus for single-board labs (one fabric per process).
    pub fn default_bus() -> CellularMqttBus {
        static BUS: OnceLock<CellularMqttBus> = OnceLock::new();
        BUS.get_or_init(CellularMqttBus::new).clone()
    }

    fn with_mut<R>(&self, f: impl FnOnce(&mut Inner) -> R) -> R {
        let mut g = self.inner.lock().expect("cellular mqtt bus lock");
        f(&mut g)
    }

    /// Record that `endpoint` opened an MQTT network to host:port.
    pub fn open(&self, endpoint: &str, client_id: u8, host: &str, port: u16) {
        self.with_mut(|inner| {
            let ep = inner.endpoints.entry(endpoint.to_string()).or_default();
            ep.brokers
                .insert(client_id, (host.to_string(), port));
        });
    }

    pub fn connect(&self, endpoint: &str, client_id: u8) {
        self.with_mut(|inner| {
            let ep = inner.endpoints.entry(endpoint.to_string()).or_default();
            ep.connected.insert(client_id);
        });
    }

    pub fn disconnect(&self, endpoint: &str, client_id: u8) {
        self.with_mut(|inner| {
            if let Some(ep) = inner.endpoints.get_mut(endpoint) {
                ep.connected.remove(&client_id);
                ep.subs.remove(&client_id);
            }
        });
    }

    pub fn subscribe(&self, endpoint: &str, client_id: u8, topic_filter: &str) {
        self.with_mut(|inner| {
            let ep = inner.endpoints.entry(endpoint.to_string()).or_default();
            ep.subs
                .entry(client_id)
                .or_default()
                .push(topic_filter.to_string());
        });
    }

    /// Publish from `endpoint`. Queues deliveries for every connected subscriber
    /// (including the publisher if it subscribed) whose filter matches.
    pub fn publish(
        &self,
        endpoint: &str,
        client_id: u8,
        topic: &str,
        payload: &[u8],
    ) -> usize {
        self.with_mut(|inner| {
            let (host, port) = inner
                .endpoints
                .get(endpoint)
                .and_then(|e| e.brokers.get(&client_id))
                .cloned()
                .unwrap_or_else(|| ("broker.labwired.local".into(), 1883));

            let msg = CellularPublish {
                endpoint: endpoint.to_string(),
                client_id,
                broker_host: host,
                broker_port: port,
                topic: topic.to_string(),
                payload: payload.to_vec(),
            };
            inner.log.push_front(msg);
            while inner.log.len() > inner.log_cap {
                inner.log.pop_back();
            }

            let mut deliveries = 0usize;
            // Snapshot matching (endpoint, client) pairs then enqueue.
            let mut targets: Vec<(String, u8)> = Vec::new();
            for (ep_id, ep) in &inner.endpoints {
                for (&cid, filters) in &ep.subs {
                    if !ep.connected.contains(&cid) {
                        continue;
                    }
                    if filters.iter().any(|f| topic_matches(f, topic)) {
                        targets.push((ep_id.clone(), cid));
                    }
                }
            }
            for (ep_id, cid) in targets {
                if let Some(ep) = inner.endpoints.get_mut(&ep_id) {
                    ep.pending.push_back(CellularDelivery {
                        client_id: cid,
                        topic: topic.to_string(),
                        payload: payload.to_vec(),
                    });
                    deliveries += 1;
                }
            }
            deliveries
        })
    }

    /// Drain downlink for one modem endpoint.
    pub fn take_pending(&self, endpoint: &str) -> Vec<CellularDelivery> {
        self.with_mut(|inner| {
            inner
                .endpoints
                .get_mut(endpoint)
                .map(|ep| ep.pending.drain(..).collect())
                .unwrap_or_default()
        })
    }

    /// Most-recent-first publish log (cloned).
    pub fn publish_log(&self) -> Vec<CellularPublish> {
        self.with_mut(|inner| inner.log.iter().cloned().collect())
    }

    /// True if any publish matches `topic` (exact).
    pub fn has_publish_on(&self, topic: &str) -> bool {
        self.with_mut(|inner| inner.log.iter().any(|m| m.topic == topic))
    }

    /// Latest payload for an exact topic, if any.
    pub fn last_payload_on(&self, topic: &str) -> Option<Vec<u8>> {
        self.with_mut(|inner| {
            inner
                .log
                .iter()
                .find(|m| m.topic == topic)
                .map(|m| m.payload.clone())
        })
    }

    pub fn clear(&self) {
        self.with_mut(|inner| {
            inner.endpoints.clear();
            inner.log.clear();
        });
    }
}

/// MQTT topic match (`+` single-level, `#` multi-level).
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn publish_loopback_to_subscriber() {
        let bus = CellularMqttBus::new();
        bus.open("m1", 0, "broker.labwired.local", 1883);
        bus.connect("m1", 0);
        bus.subscribe("m1", 0, "telematics/#");
        let n = bus.publish("m1", 0, "telematics/location", b"{\"lat\":1}");
        assert_eq!(n, 1);
        let pending = bus.take_pending("m1");
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].topic, "telematics/location");
        assert_eq!(pending[0].payload, b"{\"lat\":1}");
        assert!(bus.has_publish_on("telematics/location"));
    }

    #[test]
    fn cross_endpoint_fanout() {
        let bus = CellularMqttBus::new();
        bus.open("pub", 0, "b", 1883);
        bus.connect("pub", 0);
        bus.open("sub", 0, "b", 1883);
        bus.connect("sub", 0);
        bus.subscribe("sub", 0, "telematics/location");
        let n = bus.publish("pub", 0, "telematics/location", b"hi");
        assert_eq!(n, 1);
        assert!(bus.take_pending("pub").is_empty());
        let d = bus.take_pending("sub");
        assert_eq!(d.len(), 1);
        assert_eq!(d[0].payload, b"hi");
    }
}
