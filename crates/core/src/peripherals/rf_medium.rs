// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! Shared, seeded RF medium — path loss, capture-effect collisions, and a
//! frame decision trace. Radios (nRF52 RADIO, BLE air, ESP Wi-Fi) register as
//! nodes; the medium is the one place that decides deliver / drop.
//!
//! Determinism: every random draw is keyed by `(run_seed, tx, rx, sequence)`
//! via [`crate::peripherals::noise::channel_seed`], so the same seed replays
//! bit-identically. Default layout is co-located (zero distance) so existing
//! lossless fixtures keep working until a system sets `rf.nodes`.

use crate::peripherals::noise::{channel_seed, SplitMix64};
use std::collections::HashMap;

/// How a medium decision ended for one (tx, rx) pair.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeliveryOutcome {
    Delivered,
    LostCollision,
    LostPer,
    LostRssi,
}

/// One recorded decision — forensics for `labwired_inspect` / evidence.
#[derive(Debug, Clone)]
pub struct FrameTraceEntry {
    pub sequence: u64,
    pub tx: String,
    pub rx: String,
    pub channel: u32,
    pub rssi_dbm: f64,
    pub sinr_db: f64,
    pub outcome: DeliveryOutcome,
    pub payload_len: usize,
}

/// Node position in metres (planar).
#[derive(Debug, Clone, Copy)]
pub struct NodePosition {
    pub x: f64,
    pub y: f64,
}

impl Default for NodePosition {
    fn default() -> Self {
        Self { x: 0.0, y: 0.0 }
    }
}

/// Log-distance path-loss parameters (free-space-ish defaults).
#[derive(Debug, Clone, Copy)]
pub struct PathLossParams {
    /// Path-loss exponent (2.0 free space, ~2.7 indoor).
    pub exponent: f64,
    /// Reference loss at 1 m (dB).
    pub ref_loss_db: f64,
    /// Noise floor (dBm).
    pub noise_floor_dbm: f64,
    /// Capture margin (dB): stronger frame wins if ≥ this above the weaker.
    pub capture_margin_db: f64,
    /// Minimum RSSI (dBm) for a decode to succeed (below → LostRssi).
    pub rssi_floor_dbm: f64,
    /// Packet error rate at SNR=0 (scaled by SINR); 0 disables PER drops.
    pub per_at_0db: f64,
}

impl Default for PathLossParams {
    fn default() -> Self {
        Self {
            exponent: 2.0,
            ref_loss_db: 40.0,
            noise_floor_dbm: -95.0,
            capture_margin_db: 10.0,
            rssi_floor_dbm: -95.0,
            per_at_0db: 0.0,
        }
    }
}

/// In-flight frame while the medium decides multi-receiver / collision fate.
#[derive(Debug, Clone)]
pub struct MediumFrame {
    pub tx: String,
    pub channel: u32,
    pub tx_power_dbm: f64,
    pub bytes: Vec<u8>,
    pub sequence: u64,
}

/// Shared RF medium service.
#[derive(Debug)]
pub struct RfMedium {
    run_seed: u64,
    positions: HashMap<String, NodePosition>,
    params: PathLossParams,
    sequence: u64,
    /// Frames currently "on air" this decision window, keyed by channel.
    on_air: HashMap<u32, Vec<MediumFrame>>,
    trace: Vec<FrameTraceEntry>,
}

impl RfMedium {
    pub fn new(run_seed: u64) -> Self {
        Self {
            run_seed,
            positions: HashMap::new(),
            params: PathLossParams::default(),
            sequence: 0,
            on_air: HashMap::new(),
            trace: Vec::new(),
        }
    }

    pub fn with_params(mut self, params: PathLossParams) -> Self {
        self.params = params;
        self
    }

    pub fn set_node(&mut self, id: impl Into<String>, pos: NodePosition) {
        self.positions.insert(id.into(), pos);
    }

    pub fn run_seed(&self) -> u64 {
        self.run_seed
    }

    pub fn params(&self) -> PathLossParams {
        self.params
    }

    pub fn set_params(&mut self, params: PathLossParams) {
        self.params = params;
    }

    pub fn trace(&self) -> &[FrameTraceEntry] {
        &self.trace
    }

    /// Euclidean distance between two registered nodes (0 if either missing).
    pub fn distance_m(&self, a: &str, b: &str) -> f64 {
        let pa = self.positions.get(a).copied().unwrap_or_default();
        let pb = self.positions.get(b).copied().unwrap_or_default();
        let dx = pa.x - pb.x;
        let dy = pa.y - pb.y;
        (dx * dx + dy * dy).sqrt()
    }

    /// Log-distance path loss (dB). Distance 0 → 0 dB (co-located).
    pub fn path_loss_db(&self, distance_m: f64) -> f64 {
        if distance_m <= 0.0 {
            return 0.0;
        }
        let d = distance_m.max(0.01);
        self.params.ref_loss_db + 10.0 * self.params.exponent * d.log10()
    }

    pub fn rssi_dbm(&self, tx_power_dbm: f64, distance_m: f64) -> f64 {
        tx_power_dbm - self.path_loss_db(distance_m)
    }

    /// Push a TX frame into the medium (not yet delivered). Call
    /// [`resolve_channel`] after all concurrent TX on this channel are in.
    pub fn transmit(&mut self, tx: impl Into<String>, channel: u32, tx_power_dbm: f64, bytes: Vec<u8>) {
        let sequence = self.sequence;
        self.sequence = self.sequence.wrapping_add(1);
        self.on_air.entry(channel).or_default().push(MediumFrame {
            tx: tx.into(),
            channel,
            tx_power_dbm,
            bytes,
            sequence,
        });
    }

    /// Resolve all frames currently on `channel` for the given receivers.
    /// Returns (rx_id, frame_bytes, rssi) for each successful delivery.
    pub fn resolve_channel(
        &mut self,
        channel: u32,
        receivers: &[String],
    ) -> Vec<(String, Vec<u8>, f64)> {
        let frames = match self.on_air.remove(&channel) {
            Some(f) if !f.is_empty() => f,
            _ => return Vec::new(),
        };

        let mut delivered = Vec::new();
        for rx in receivers {
            // Rank candidate frames by RSSI at this receiver.
            let mut candidates: Vec<(f64, &MediumFrame)> = frames
                .iter()
                .filter(|f| f.tx != *rx)
                .map(|f| {
                    let d = self.distance_m(&f.tx, rx);
                    (self.rssi_dbm(f.tx_power_dbm, d), f)
                })
                .collect();
            candidates.sort_by(|a, b| b.0.total_cmp(&a.0));

            if candidates.is_empty() {
                continue;
            }

            let (best_rssi, best) = candidates[0];
            let second_rssi = candidates.get(1).map(|(r, _)| *r);

            let outcome = if best_rssi < self.params.rssi_floor_dbm {
                DeliveryOutcome::LostRssi
            } else if let Some(second) = second_rssi {
                if best_rssi - second < self.params.capture_margin_db {
                    DeliveryOutcome::LostCollision
                } else if self.per_drop(best, rx, best_rssi) {
                    DeliveryOutcome::LostPer
                } else {
                    DeliveryOutcome::Delivered
                }
            } else if self.per_drop(best, rx, best_rssi) {
                DeliveryOutcome::LostPer
            } else {
                DeliveryOutcome::Delivered
            };

            let interferer = second_rssi.unwrap_or(self.params.noise_floor_dbm);
            let sinr = best_rssi - interferer.max(self.params.noise_floor_dbm);

            self.trace.push(FrameTraceEntry {
                sequence: best.sequence,
                tx: best.tx.clone(),
                rx: rx.clone(),
                channel,
                rssi_dbm: best_rssi,
                sinr_db: sinr,
                outcome,
                payload_len: best.bytes.len(),
            });

            if outcome == DeliveryOutcome::Delivered {
                delivered.push((rx.clone(), best.bytes.clone(), best_rssi));
            }
        }
        delivered
    }

    /// Convenience: single TX, resolve immediately for all receivers.
    pub fn send_and_resolve(
        &mut self,
        tx: impl Into<String>,
        channel: u32,
        tx_power_dbm: f64,
        bytes: Vec<u8>,
        receivers: &[String],
    ) -> Vec<(String, Vec<u8>, f64)> {
        self.transmit(tx, channel, tx_power_dbm, bytes);
        self.resolve_channel(channel, receivers)
    }

    fn per_drop(&self, frame: &MediumFrame, rx: &str, rssi: f64) -> bool {
        if self.params.per_at_0db <= 0.0 {
            return false;
        }
        let sinr = rssi - self.params.noise_floor_dbm;
        // Simple logistic: higher SINR → lower PER.
        let per = (self.params.per_at_0db / (1.0 + (sinr / 6.0).exp())).clamp(0.0, 1.0);
        let seed = channel_seed(self.run_seed, &frame.tx, &format!("{rx}/{}", frame.sequence));
        let mut rng = SplitMix64::new(seed);
        rng.next_f64_open01() < per
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn co_located_delivers() {
        let mut m = RfMedium::new(1);
        m.set_node("a", NodePosition { x: 0.0, y: 0.0 });
        m.set_node("b", NodePosition { x: 0.0, y: 0.0 });
        let out = m.send_and_resolve("a", 0, 0.0, b"hi".to_vec(), &["b".into()]);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].1, b"hi");
        assert_eq!(m.trace().last().unwrap().outcome, DeliveryOutcome::Delivered);
    }

    #[test]
    fn distance_drops_below_rssi_floor() {
        let mut m = RfMedium::new(2).with_params(PathLossParams {
            rssi_floor_dbm: -60.0,
            ..PathLossParams::default()
        });
        m.set_node("a", NodePosition { x: 0.0, y: 0.0 });
        m.set_node("b", NodePosition { x: 100.0, y: 0.0 }); // far
        let out = m.send_and_resolve("a", 0, 0.0, b"x".to_vec(), &["b".into()]);
        assert!(out.is_empty());
        assert_eq!(m.trace().last().unwrap().outcome, DeliveryOutcome::LostRssi);
    }

    #[test]
    fn capture_effect_stronger_wins() {
        let mut m = RfMedium::new(3).with_params(PathLossParams {
            capture_margin_db: 10.0,
            rssi_floor_dbm: -100.0,
            ..PathLossParams::default()
        });
        // rx between them, closer to strong
        m.set_node("strong", NodePosition { x: 0.0, y: 0.0 });
        m.set_node("weak", NodePosition { x: 20.0, y: 0.0 });
        m.set_node("rx", NodePosition { x: 1.0, y: 0.0 });
        m.transmit("strong", 7, 0.0, b"S".to_vec());
        m.transmit("weak", 7, 0.0, b"W".to_vec());
        let out = m.resolve_channel(7, &["rx".into()]);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].1, b"S");
        assert_eq!(m.trace().last().unwrap().outcome, DeliveryOutcome::Delivered);
    }

    #[test]
    fn equal_strength_collision_loses_both() {
        let mut m = RfMedium::new(4).with_params(PathLossParams {
            capture_margin_db: 10.0,
            rssi_floor_dbm: -100.0,
            ..PathLossParams::default()
        });
        m.set_node("a", NodePosition { x: -5.0, y: 0.0 });
        m.set_node("b", NodePosition { x: 5.0, y: 0.0 });
        m.set_node("rx", NodePosition { x: 0.0, y: 0.0 });
        m.transmit("a", 1, 0.0, b"A".to_vec());
        m.transmit("b", 1, 0.0, b"B".to_vec());
        let out = m.resolve_channel(1, &["rx".into()]);
        assert!(out.is_empty());
        assert_eq!(
            m.trace().last().unwrap().outcome,
            DeliveryOutcome::LostCollision
        );
    }

    #[test]
    fn same_seed_replays_identically() {
        fn outcomes(seed: u64) -> Vec<DeliveryOutcome> {
            let mut m = RfMedium::new(seed).with_params(PathLossParams {
                per_at_0db: 0.9,
                noise_floor_dbm: -40.0,
                rssi_floor_dbm: -100.0,
                ..PathLossParams::default()
            });
            m.set_node("a", NodePosition { x: 0.0, y: 0.0 });
            m.set_node("b", NodePosition { x: 3.0, y: 0.0 });
            for i in 0..40 {
                m.send_and_resolve("a", 0, 0.0, vec![i as u8], &["b".into()]);
            }
            m.trace().iter().map(|e| e.outcome).collect()
        }
        assert_eq!(outcomes(42), outcomes(42));
        assert_ne!(outcomes(42), outcomes(99));
    }

    #[test]
    fn rssi_tracks_distance() {
        let m = RfMedium::new(0);
        let near = m.rssi_dbm(0.0, 1.0);
        let far = m.rssi_dbm(0.0, 10.0);
        assert!(near > far);
    }
}
