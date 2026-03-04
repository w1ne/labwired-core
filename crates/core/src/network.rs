// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

use crate::SimResult;
use std::sync::mpsc::{channel, Receiver, Sender};

/// Trait for virtual interconnects between machines.
pub trait Interconnect: Send {
    /// Advance the interconnect state.
    fn tick(&mut self) -> SimResult<()>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CanFrame {
    pub id: u32,
    pub data: Vec<u8>,
}

pub struct CanBus {
    rx: Receiver<CanFrame>,
    tx: Sender<CanFrame>,
    node_txs: Vec<Sender<CanFrame>>,
}

impl Default for CanBus {
    fn default() -> Self {
        Self::new()
    }
}

impl CanBus {
    pub fn new() -> Self {
        let (tx, rx) = channel();
        Self {
            rx,
            tx,
            node_txs: Vec::new(),
        }
    }

    pub fn attach(&mut self) -> (Sender<CanFrame>, Receiver<CanFrame>) {
        let (node_tx, node_rx) = channel();
        self.node_txs.push(node_tx);
        (self.tx.clone(), node_rx)
    }
}

impl Interconnect for CanBus {
    fn tick(&mut self) -> SimResult<()> {
        while let Ok(frame) = self.rx.try_recv() {
            for node_tx in &self.node_txs {
                let _ = node_tx.send(frame.clone());
            }
        }
        Ok(())
    }
}

/// A simple cross-link between two UART peripherals.
pub struct UartCrossLink {
    pub node_a: String,
    pub node_b: String,
    // Add buffers and peripheral references
}

impl Interconnect for UartCrossLink {
    fn tick(&mut self) -> SimResult<()> {
        // TODO: Move bytes between node buffers
        Ok(())
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
