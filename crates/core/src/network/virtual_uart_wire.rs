// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! The UART cross-link medium — one implementation for every host.
//!
//! A [`VirtualWireBus`] is shared by cloning, so bytes one endpoint transmits
//! land in the peer endpoint's inbox with no per-byte round-trip and no owner
//! to tick it. That property is what lets a single medium serve every host:
//! a native `World` that owns its machines, and the browser, where each chip is
//! a separate `WasmSimulator` inside the same wasm module and there is no
//! `World` to own channels.
//!
//! This replaced a second, native-only medium (`UartCrossLink`) that carried
//! bytes over mpsc channels and needed a `World::step_all` tick to move them.
//! Two media for one job meant a link behaved differently depending on which
//! host ran it; the bus is a superset, so the mpsc one was deleted rather than
//! kept in parallel.
//!
//! Every [`VirtualWireEndpoint`] minted from the *same* bus exchanges bytes;
//! endpoints from *different* buses are fully isolated. This is what lets two
//! labs (or two workers) hold independent wires without colliding on a link id —
//! the behaviour the former process-static `WIRE` registry could not offer.
//!
//! A [`VirtualWireEndpoint`] is a [`UartStreamDevice`], so it attaches to a
//! chip's UART through the existing `attach_uart_stream_by_id` seam.

use crate::peripherals::uart::UartStreamDevice;
use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};

#[derive(Default)]
struct Link {
    /// `inbox[s]` holds bytes waiting to be received by the endpoint on side `s`.
    inbox: [VecDeque<u8>; 2],
    /// `corrupt[s]` is how many further bytes *transmitted by* side `s` get
    /// flipped before clean forwarding resumes — the wire-fault injection the
    /// IO-Link station tests use to prove a master recovers from line noise.
    corrupt: [u32; 2],
}

#[derive(Default)]
struct VirtualWire {
    links: HashMap<u32, Link>,
}

/// A shared UART cross-link medium. Cloning a bus (or minting endpoints from it)
/// shares one underlying wire; two distinct buses are isolated. `Arc<Mutex<…>>`
/// keeps endpoints `Send` so they stay valid inside a `Machine` (native requires
/// `MachineTrait: Send`); the browser is single-threaded so the mutex never
/// contends.
#[derive(Clone, Default)]
pub struct VirtualWireBus {
    inner: Arc<Mutex<VirtualWire>>,
}

impl VirtualWireBus {
    pub fn new() -> Self {
        Self::default()
    }

    /// Mint an endpoint on `side` (0/1) of `link_id`. The two endpoints of a
    /// link share this bus and use opposite sides.
    pub fn endpoint(&self, link_id: u32, side: u8) -> VirtualWireEndpoint {
        VirtualWireEndpoint {
            wire: self.inner.clone(),
            link_id,
            side: (side & 1) as usize,
        }
    }

    /// Drop every link's buffered bytes on this bus — call between lab loads so a
    /// stale link doesn't leak bytes into a freshly loaded station.
    pub fn clear(&self) {
        if let Ok(mut w) = self.inner.lock() {
            w.links.clear();
        }
    }

    /// Flip the next `n` bytes transmitted by `side` on `link_id` (each XORed
    /// with `0xFF`), then forward cleanly again. Injecting the fault on the
    /// medium rather than on an endpoint keeps it available to any host, and
    /// means a corrupted byte is indistinguishable from line noise to both peers.
    pub fn corrupt_next(&self, link_id: u32, side: u8, n: u32) {
        if let Ok(mut w) = self.inner.lock() {
            w.links.entry(link_id).or_default().corrupt[(side & 1) as usize] = n;
        }
    }
}

/// One endpoint of a shared UART cross-link. The two endpoints of a link are
/// minted from the same [`VirtualWireBus`] with opposite `side`s (0 and 1).
/// Attach to a chip's UART via `SystemBus::attach_uart_stream_by_id`.
pub struct VirtualWireEndpoint {
    wire: Arc<Mutex<VirtualWire>>,
    link_id: u32,
    side: usize,
}

impl UartStreamDevice for VirtualWireEndpoint {
    fn poll(&mut self, _elapsed_us: u32) -> Option<u8> {
        let mut w = self.wire.lock().ok()?;
        w.links.get_mut(&self.link_id)?.inbox[self.side].pop_front()
    }

    fn on_tx_byte(&mut self, byte: u8) {
        if let Ok(mut w) = self.wire.lock() {
            let link = w.links.entry(self.link_id).or_default();
            let byte = if link.corrupt[self.side] > 0 {
                link.corrupt[self.side] -= 1;
                byte ^ 0xFF
            } else {
                byte
            };
            // Transmitted bytes are delivered to the PEER side's inbox.
            link.inbox[self.side ^ 1].push_back(byte);
        }
    }

    /// A wire between two chips carries whatever protocol the firmware speaks,
    /// not console text.
    fn carries_protocol_octets(&self) -> bool {
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_bus_delivers_bytes_to_the_peer_endpoint() {
        let bus = VirtualWireBus::new();
        let mut a = bus.endpoint(7, 0);
        let mut b = bus.endpoint(7, 1);

        // A transmits → B receives.
        a.on_tx_byte(0x5A);
        assert_eq!(b.poll(0), Some(0x5A));
        assert_eq!(b.poll(0), None);

        // B transmits → A receives (full-duplex).
        b.on_tx_byte(0xC3);
        assert_eq!(a.poll(0), Some(0xC3));
        assert_eq!(a.poll(0), None);

        // A different link id on the same bus is isolated.
        let mut other = bus.endpoint(99, 0);
        assert_eq!(other.poll(0), None);
    }

    /// Wire-fault injection, ported from the deleted mpsc medium so the
    /// behaviour the IO-Link station tests depend on is still covered here.
    #[test]
    fn corrupts_next_n_bytes_then_forwards_clean() {
        let bus = VirtualWireBus::new();
        let mut a = bus.endpoint(1, 0);
        let mut b = bus.endpoint(1, 1);

        bus.corrupt_next(1, 0, 1);
        a.on_tx_byte(0x55);
        a.on_tx_byte(0x66);
        assert_eq!(b.poll(0), Some(0xAA), "first byte flipped (0x55 ^ 0xFF)");
        assert_eq!(
            b.poll(0),
            Some(0x66),
            "clean again after the budget runs out"
        );
    }

    /// A fault on one direction must not disturb the other.
    #[test]
    fn corruption_is_per_direction() {
        let bus = VirtualWireBus::new();
        let mut a = bus.endpoint(2, 0);
        let mut b = bus.endpoint(2, 1);

        bus.corrupt_next(2, 1, 1); // only bytes transmitted BY side 1
        a.on_tx_byte(0x11);
        b.on_tx_byte(0x22);
        assert_eq!(b.poll(0), Some(0x11), "A→B untouched");
        assert_eq!(a.poll(0), Some(0xDD), "B→A flipped (0x22 ^ 0xFF)");
    }

    #[test]
    fn separate_buses_do_not_cross() {
        // The whole point of instance-scoping: two labs on the same link id must
        // NOT hear each other. The old process-static wire could not do this.
        let lab_a = VirtualWireBus::new();
        let lab_b = VirtualWireBus::new();

        let mut a_master = lab_a.endpoint(1, 0);
        let mut b_device = lab_b.endpoint(1, 1); // same link id, different bus

        a_master.on_tx_byte(0xAA);
        assert_eq!(
            b_device.poll(0),
            None,
            "byte leaked across independent buses"
        );

        // lab_a's own peer still receives it.
        let mut a_device = lab_a.endpoint(1, 1);
        assert_eq!(a_device.poll(0), Some(0xAA));
    }

    #[test]
    fn clear_drops_buffered_bytes_on_that_bus_only() {
        let lab_a = VirtualWireBus::new();
        let lab_b = VirtualWireBus::new();
        let mut a_tx = lab_a.endpoint(2, 0);
        let mut b_tx = lab_b.endpoint(2, 0);
        a_tx.on_tx_byte(0x11);
        b_tx.on_tx_byte(0x22);

        lab_a.clear();

        assert_eq!(
            lab_a.endpoint(2, 1).poll(0),
            None,
            "cleared bus still held bytes"
        );
        assert_eq!(
            lab_b.endpoint(2, 1).poll(0),
            Some(0x22),
            "clear leaked to another bus"
        );
    }
}
