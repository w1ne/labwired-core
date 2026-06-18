// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! Shared, in-process UART cross-link medium for browser multi-chip labs.
//!
//! The native [`crate::network::UartCrossLink`] wires two UARTs with mpsc
//! channels owned by a `World`. In the browser, each chip is a separate
//! `WasmSimulator` running inside the *same* wasm module, so there is no `World`
//! to own channels — but the wasm module's process statics ARE shared across
//! every per-chip simulator. This mirrors the nRF radio's `virtual_air`
//! registry: a process-static [`VirtualWire`] keyed by link id, with two sides.
//!
//! A [`VirtualWireEndpoint`] is a [`UartStreamDevice`], so it attaches to a
//! chip's UART through the existing `attach_uart_stream_by_id` seam. Bytes one
//! endpoint transmits land in the peer endpoint's inbox with no per-byte host
//! round-trip — chips can keep stepping in batches and still exchange data.

use crate::peripherals::uart::UartStreamDevice;
use std::collections::{HashMap, VecDeque};
use std::sync::{Mutex, OnceLock};

#[derive(Default)]
struct Link {
    /// `inbox[s]` holds bytes waiting to be received by the endpoint on side `s`.
    inbox: [VecDeque<u8>; 2],
}

#[derive(Default)]
struct VirtualWire {
    links: HashMap<u32, Link>,
}

fn wire() -> &'static Mutex<VirtualWire> {
    static WIRE: OnceLock<Mutex<VirtualWire>> = OnceLock::new();
    WIRE.get_or_init(|| Mutex::new(VirtualWire::default()))
}

/// Clear every link (test/reset helper — call between lab loads so a stale link
/// doesn't leak bytes into a freshly loaded station).
pub fn clear_virtual_uart_wires() {
    if let Ok(mut w) = wire().lock() {
        w.links.clear();
    }
}

/// One endpoint of a shared UART cross-link. The two endpoints of a link share
/// the same `link_id` and use opposite `side`s (0 and 1). Attach to a chip's
/// UART via `SystemBus::attach_uart_stream_by_id`.
pub struct VirtualWireEndpoint {
    link_id: u32,
    side: usize,
}

impl VirtualWireEndpoint {
    pub fn new(link_id: u32, side: u8) -> Self {
        Self {
            link_id,
            side: (side & 1) as usize,
        }
    }
}

impl UartStreamDevice for VirtualWireEndpoint {
    fn poll(&mut self, _elapsed_us: u32) -> Option<u8> {
        let mut w = wire().lock().ok()?;
        w.links.get_mut(&self.link_id)?.inbox[self.side].pop_front()
    }

    fn on_tx_byte(&mut self, byte: u8) {
        if let Ok(mut w) = wire().lock() {
            // Transmitted bytes are delivered to the PEER side's inbox.
            w.links.entry(self.link_id).or_default().inbox[self.side ^ 1].push_back(byte);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn virtual_wire_delivers_bytes_to_the_peer_endpoint() {
        clear_virtual_uart_wires();
        let mut a = VirtualWireEndpoint::new(7, 0);
        let mut b = VirtualWireEndpoint::new(7, 1);

        // A transmits → B receives.
        a.on_tx_byte(0x5A);
        assert_eq!(b.poll(0), Some(0x5A));
        assert_eq!(b.poll(0), None);

        // B transmits → A receives (full-duplex).
        b.on_tx_byte(0xC3);
        assert_eq!(a.poll(0), Some(0xC3));
        assert_eq!(a.poll(0), None);

        // A different link id is isolated.
        let mut other = VirtualWireEndpoint::new(99, 0);
        assert_eq!(other.poll(0), None);
        clear_virtual_uart_wires();
    }
}
