// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

use crate::SimResult;
use std::{
    collections::VecDeque,
    sync::mpsc::{channel, Receiver, Sender},
};

pub mod candump;
pub mod egress;
pub mod mqtt;
pub mod sim;
pub mod sim_mqtt_fabric;
pub mod virtual_uart_wire;
pub use sim_mqtt_fabric::{
    CellularDelivery, CellularMqttBus, CellularPublish, FabricDelivery, FabricPublish,
    SimMqttFabric,
};
pub use virtual_uart_wire::{VirtualWireBus, VirtualWireEndpoint};

/// Trait for virtual interconnects between machines.
pub trait Interconnect: Send {
    /// Advance the interconnect state.
    fn tick(&mut self) -> SimResult<()>;

    /// Downcast hook for tests/tools that need the concrete interconnect type
    /// (e.g. to inject faults). Default `None`; concrete types opt in.
    fn as_any_mut(&mut self) -> Option<&mut dyn std::any::Any> {
        None
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CanFrame {
    pub id: u32,
    pub data: Vec<u8>,
    /// 29-bit extended identifier (XTD) rather than 11-bit standard.
    pub extended: bool,
    /// CAN-FD frame format (FDF/EDL).
    pub fd: bool,
    /// CAN-FD bitrate switch flag (BRS).
    pub bitrate_switch: bool,
    /// Remote-transmission-request frame.
    pub remote: bool,
}

impl CanFrame {
    pub fn classic(id: u32, data: Vec<u8>) -> Self {
        Self {
            id,
            data,
            extended: false,
            fd: false,
            bitrate_switch: false,
            remote: false,
        }
    }
}

/// One endpoint of a shared CAN medium.
///
/// The outgoing queue is intentionally distinct for each endpoint. The public
/// endpoint API remains `(Sender<CanFrame>, Receiver<CanFrame>)`, but this
/// private receiver lets `CanBus` know which endpoint submitted a frame and
/// avoid delivering that frame back to its transmitter.
struct CanBusEndpoint {
    outbound: Receiver<CanFrame>,
    inbound: Sender<CanFrame>,
}

pub struct CanBus {
    endpoints: Vec<CanBusEndpoint>,
    trace: VecDeque<CanFrame>,
    trace_dropped: u64,
}

impl Default for CanBus {
    fn default() -> Self {
        Self::new()
    }
}

impl CanBus {
    pub fn new() -> Self {
        Self {
            endpoints: Vec::new(),
            trace: VecDeque::new(),
            trace_dropped: 0,
        }
    }

    pub fn attach(&mut self) -> (Sender<CanFrame>, Receiver<CanFrame>) {
        let (outbound_tx, outbound_rx) = channel();
        let (inbound_tx, inbound_rx) = channel();
        self.endpoints.push(CanBusEndpoint {
            outbound: outbound_rx,
            inbound: inbound_tx,
        });
        (outbound_tx, inbound_rx)
    }

    /// Frames accepted by this shared medium, in deterministic delivery order.
    pub fn trace_snapshot(&self) -> Vec<CanFrame> {
        self.trace.iter().cloned().collect()
    }

    /// Maximum number of most-recent frames retained by [`Self::trace_snapshot`].
    pub const fn trace_capacity(&self) -> usize {
        4096
    }

    /// Number of oldest frames evicted since this bus was created.
    pub const fn trace_dropped(&self) -> u64 {
        self.trace_dropped
    }
}

impl Interconnect for CanBus {
    fn tick(&mut self) -> SimResult<()> {
        // Endpoints are traversed in attachment order. That gives a stable
        // ordering when several nodes transmit in the same world round while
        // preserving CAN's shared-medium fan-out to every *other* endpoint.
        for source_idx in 0..self.endpoints.len() {
            while let Ok(frame) = self.endpoints[source_idx].outbound.try_recv() {
                if self.trace.len() == self.trace_capacity() {
                    self.trace.pop_front();
                    self.trace_dropped += 1;
                }
                self.trace.push_back(frame.clone());
                for (target_idx, target) in self.endpoints.iter().enumerate() {
                    if target_idx != source_idx {
                        let _ = target.inbound.send(frame.clone());
                    }
                }
            }
        }
        Ok(())
    }

    fn as_any_mut(&mut self) -> Option<&mut dyn std::any::Any> {
        Some(self)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WirelessPacket {
    pub channel: u8,
    pub payload: Vec<u8>,
}

pub struct WirelessBus {
    rx: Receiver<WirelessPacket>,
    tx: Sender<WirelessPacket>,
    node_txs: Vec<Sender<WirelessPacket>>,
}

impl Default for WirelessBus {
    fn default() -> Self {
        Self::new()
    }
}

impl WirelessBus {
    pub fn new() -> Self {
        let (tx, rx) = channel();
        Self {
            rx,
            tx,
            node_txs: Vec::new(),
        }
    }

    pub fn attach(&mut self) -> (Sender<WirelessPacket>, Receiver<WirelessPacket>) {
        let (node_tx, node_rx) = channel();
        self.node_txs.push(node_tx);
        (self.tx.clone(), node_rx)
    }
}

impl Interconnect for WirelessBus {
    fn tick(&mut self) -> SimResult<()> {
        while let Ok(packet) = self.rx.try_recv() {
            // Simple broadcast for now, models a shared medium
            for node_tx in &self.node_txs {
                let _ = node_tx.send(packet.clone());
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn can_trace_reports_bounded_history_drops() {
        let mut bus = CanBus::new();
        let (tx, _rx) = bus.attach();
        let (_peer_tx, _peer_rx) = bus.attach();
        for id in 0..=bus.trace_capacity() {
            tx.send(CanFrame::classic(id as u32, vec![])).unwrap();
        }
        bus.tick().unwrap();
        let trace = bus.trace_snapshot();
        assert_eq!(trace.len(), bus.trace_capacity());
        assert_eq!(bus.trace_dropped(), 1);
        assert_eq!(trace.first().unwrap().id, 1);
    }
}
