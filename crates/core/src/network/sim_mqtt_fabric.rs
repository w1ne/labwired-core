// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! [`SimMqttFabric`] — **just enough to send messages and collect data**.
//!
//! Scope (intentional, not a full network):
//!   * **Send:** firmware `QMTPUB` lands a topic + payload on the fabric
//!   * **Collect:** ring log + `inspect` / smoke / playground strip
//!   * Optional: same-fabric `QMTSUB` → `+QMTRECV` fan-out (loopback / multi-UE)
//!
//! Not in scope: MQTT wire protocol, EPC/RAN, TLS, real brokers, host egress.
//! Lives on lab AirBus (or CLI private lab air via `attach_lab_air`).

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::{Arc, Mutex};

/// One retained publish for forensics / smoke / UI.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FabricPublish {
    pub endpoint: String,
    pub client_id: u8,
    pub broker_host: String,
    pub broker_port: u16,
    pub topic: String,
    pub payload: Vec<u8>,
}

/// Pending downlink for a modem endpoint (becomes `+QMTRECV`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FabricDelivery {
    pub client_id: u8,
    pub topic: String,
    pub payload: Vec<u8>,
}

// Back-compat aliases used during rename.
pub type CellularPublish = FabricPublish;
pub type CellularDelivery = FabricDelivery;

#[derive(Debug, Default)]
struct EndpointState {
    connected: HashSet<u8>,
    subs: HashMap<u8, Vec<String>>,
    brokers: HashMap<u8, (String, u16)>,
    pending: VecDeque<FabricDelivery>,
}

#[derive(Debug, Default)]
struct Inner {
    endpoints: HashMap<String, EndpointState>,
    log: VecDeque<FabricPublish>,
    log_cap: usize,
}

/// Shared simulated MQTT fabric. Clone freely (`Arc` inside).
#[derive(Debug, Clone)]
pub struct SimMqttFabric {
    inner: Arc<Mutex<Inner>>,
}

/// Historical name — same type as [`SimMqttFabric`].
pub type CellularMqttBus = SimMqttFabric;

impl Default for SimMqttFabric {
    fn default() -> Self {
        Self::new()
    }
}

impl SimMqttFabric {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(Inner {
                endpoints: HashMap::new(),
                log: VecDeque::new(),
                log_cap: 64,
            })),
        }
    }

    fn with_mut<R>(&self, f: impl FnOnce(&mut Inner) -> R) -> R {
        let mut g = self.inner.lock().expect("sim mqtt fabric lock");
        f(&mut g)
    }

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

    /// Publish from `endpoint`. Returns number of subscriber deliveries queued.
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

            let msg = FabricPublish {
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
            let mut deliveries = 0usize;
            for (ep_id, cid) in targets {
                if let Some(ep) = inner.endpoints.get_mut(&ep_id) {
                    ep.pending.push_back(FabricDelivery {
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

    pub fn take_pending(&self, endpoint: &str) -> Vec<FabricDelivery> {
        self.with_mut(|inner| {
            inner
                .endpoints
                .get_mut(endpoint)
                .map(|ep| ep.pending.drain(..).collect())
                .unwrap_or_default()
        })
    }

    pub fn publish_log(&self) -> Vec<FabricPublish> {
        self.with_mut(|inner| inner.log.iter().cloned().collect())
    }

    pub fn has_publish_on(&self, topic: &str) -> bool {
        self.with_mut(|inner| inner.log.iter().any(|m| m.topic == topic))
    }

    pub fn last_payload_on(&self, topic: &str) -> Option<Vec<u8>> {
        self.with_mut(|inner| {
            inner
                .log
                .iter()
                .find(|m| m.topic == topic)
                .map(|m| m.payload.clone())
        })
    }

    /// Snapshot for UI: up to `limit` most-recent publishes as
    /// `"topic\\tpayload_utf8"` lines (lossy UTF-8).
    pub fn inspect_lines(&self, limit: usize) -> Vec<String> {
        self.with_mut(|inner| {
            inner
                .log
                .iter()
                .take(limit)
                .map(|m| {
                    format!(
                        "{}\t{}",
                        m.topic,
                        String::from_utf8_lossy(&m.payload)
                    )
                })
                .collect()
        })
    }

    pub fn clear(&self) {
        self.with_mut(|inner| {
            inner.endpoints.clear();
            inner.log.clear();
        });
    }
}

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
        let bus = SimMqttFabric::new();
        bus.open("m1", 0, "broker.labwired.local", 1883);
        bus.connect("m1", 0);
        bus.subscribe("m1", 0, "telematics/#");
        let n = bus.publish("m1", 0, "telematics/location", b"{\"lat\":1}");
        assert_eq!(n, 1);
        let pending = bus.take_pending("m1");
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].topic, "telematics/location");
        assert!(bus.has_publish_on("telematics/location"));
    }

    #[test]
    fn cross_endpoint_fanout() {
        let bus = SimMqttFabric::new();
        bus.open("pub", 0, "b", 1883);
        bus.connect("pub", 0);
        bus.open("sub", 0, "b", 1883);
        bus.connect("sub", 0);
        bus.subscribe("sub", 0, "telematics/location");
        assert_eq!(bus.publish("pub", 0, "telematics/location", b"hi"), 1);
        assert!(bus.take_pending("pub").is_empty());
        assert_eq!(bus.take_pending("sub")[0].payload, b"hi");
    }
}
