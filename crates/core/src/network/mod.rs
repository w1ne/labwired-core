// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

use crate::SimResult;
use std::sync::mpsc::{channel, Receiver, Sender};

pub mod candump;
pub mod egress;
pub mod mqtt;
pub mod sim;
pub mod virtual_uart_wire;

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
}

impl Interconnect for CanBus {
    fn tick(&mut self) -> SimResult<()> {
        // Endpoints are traversed in attachment order. That gives a stable
        // ordering when several nodes transmit in the same world round while
        // preserving CAN's shared-medium fan-out to every *other* endpoint.
        for source_idx in 0..self.endpoints.len() {
            while let Ok(frame) = self.endpoints[source_idx].outbound.try_recv() {
                for (target_idx, target) in self.endpoints.iter().enumerate() {
                    if target_idx != source_idx {
                        let _ = target.inbound.send(frame.clone());
                    }
                }
            }
        }
        Ok(())
    }
}

/// One end of the point-to-point UART wire, attached to a chip's UART via
/// `UartStreamDevice`. Bytes the firmware transmits land in `out` (drained by
/// the link); bytes the link delivers land in `inbox` (fed to the chip RX).
pub struct UartWireEndpoint {
    out: Sender<u8>,
    inbox: Receiver<u8>,
}

impl crate::peripherals::uart::UartStreamDevice for UartWireEndpoint {
    fn poll(&mut self, _elapsed_us: u32) -> Option<u8> {
        self.inbox.try_recv().ok()
    }
    fn on_tx_byte(&mut self, byte: u8) {
        let _ = self.out.send(byte);
    }
}

/// Point-to-point full-duplex UART link between two nodes' UARTs (the simulated
/// IO-Link C/Q wire). Construct with [`UartCrossLink::new`], attach the two
/// returned [`UartWireEndpoint`]s to each node's UART, and register the link as
/// a `World` interconnect; `tick()` shuttles bytes both directions each step.
pub struct UartCrossLink {
    pub node_a: String,
    pub node_b: String,
    a_out: Receiver<u8>, // bytes node A's firmware transmitted
    b_in: Sender<u8>,    // -> node B inbox (RX)
    b_out: Receiver<u8>, // bytes node B's firmware transmitted
    a_in: Sender<u8>,    // -> node A inbox (RX)
    /// Fault injection: next N bytes forwarded A->B are XORed with 0xFF.
    corrupt_a_to_b: u32,
    /// Fault injection: next N bytes forwarded B->A are XORed with 0xFF.
    corrupt_b_to_a: u32,
}

impl UartCrossLink {
    /// Corrupt the next `n` bytes forwarded from node A to node B (each XORed
    /// with 0xFF), then forward cleanly again.
    pub fn set_corrupt_a_to_b(&mut self, n: u32) {
        self.corrupt_a_to_b = n;
    }

    /// Corrupt the next `n` bytes forwarded from node B to node A (each XORed
    /// with 0xFF), then forward cleanly again.
    pub fn set_corrupt_b_to_a(&mut self, n: u32) {
        self.corrupt_b_to_a = n;
    }

    pub fn new(node_a: String, node_b: String) -> (Self, UartWireEndpoint, UartWireEndpoint) {
        let (a_tx, a_out) = channel(); // A firmware TX -> link
        let (a_in, a_inbox) = channel(); // link -> A RX
        let (b_tx, b_out) = channel(); // B firmware TX -> link
        let (b_in, b_inbox) = channel(); // link -> B RX
        let endpoint_a = UartWireEndpoint {
            out: a_tx,
            inbox: a_inbox,
        };
        let endpoint_b = UartWireEndpoint {
            out: b_tx,
            inbox: b_inbox,
        };
        let link = Self {
            node_a,
            node_b,
            a_out,
            b_in,
            b_out,
            a_in,
            corrupt_a_to_b: 0,
            corrupt_b_to_a: 0,
        };
        (link, endpoint_a, endpoint_b)
    }
}

impl Interconnect for UartCrossLink {
    fn tick(&mut self) -> SimResult<()> {
        while let Ok(byte) = self.a_out.try_recv() {
            let byte = if self.corrupt_a_to_b > 0 {
                self.corrupt_a_to_b -= 1;
                byte ^ 0xFF
            } else {
                byte
            };
            let _ = self.b_in.send(byte);
        }
        while let Ok(byte) = self.b_out.try_recv() {
            let byte = if self.corrupt_b_to_a > 0 {
                self.corrupt_b_to_a -= 1;
                byte ^ 0xFF
            } else {
                byte
            };
            let _ = self.a_in.send(byte);
        }
        Ok(())
    }

    fn as_any_mut(&mut self) -> Option<&mut dyn std::any::Any> {
        Some(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::peripherals::uart::UartStreamDevice;

    #[test]
    fn uart_cross_link_moves_bytes_both_directions() {
        let (mut link, mut a, mut b) = UartCrossLink::new("nodeA".into(), "nodeB".into());

        // Firmware on A transmits 0x55; B receives it after a tick.
        a.on_tx_byte(0x55);
        link.tick().unwrap();
        assert_eq!(b.poll(1000), Some(0x55));
        assert_eq!(b.poll(1000), None);

        // Reverse direction.
        b.on_tx_byte(0xAA);
        link.tick().unwrap();
        assert_eq!(a.poll(1000), Some(0xAA));
        assert_eq!(a.poll(1000), None);
    }

    #[test]
    fn crosslink_corrupts_next_n_bytes_then_forwards_clean() {
        let (mut link, mut ep_a, mut ep_b) = UartCrossLink::new("a".into(), "b".into());
        link.set_corrupt_a_to_b(1);
        ep_a.on_tx_byte(0x55);
        ep_a.on_tx_byte(0x66);
        link.tick().unwrap();
        assert_eq!(ep_b.poll(0), Some(0xAA)); // 0x55 ^ 0xFF
        assert_eq!(ep_b.poll(0), Some(0x66)); // clean again
    }

    #[test]
    fn interconnect_downcasts_to_crosslink() {
        let (link, _a, _b) = UartCrossLink::new("a".into(), "b".into());
        let mut boxed: Box<dyn Interconnect> = Box::new(link);
        let any = boxed.as_any_mut().expect("crosslink exposes as_any_mut");
        assert!(any.downcast_mut::<UartCrossLink>().is_some());
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
