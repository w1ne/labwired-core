// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! `EgressTap` observes UART TX bytes and forwards them to the egress channel.

use crate::network::egress::EgressItem;
use crate::peripherals::uart::UartStreamDevice;
use std::sync::mpsc::Sender;

/// A UART stream device that only observes TX (never drives RX). Each byte the
/// firmware transmits is forwarded to the egress channel. Sending on an
/// unbounded `mpsc::Sender` never blocks, so the sim thread stays deterministic.
pub struct EgressTap {
    tx: Sender<EgressItem>,
}

impl EgressTap {
    pub fn new(tx: Sender<EgressItem>) -> Self {
        Self { tx }
    }
}

impl UartStreamDevice for EgressTap {
    fn poll(&mut self, _elapsed_us: u32) -> Option<u8> {
        None
    }

    fn on_tx_byte(&mut self, byte: u8) {
        // Ignore send errors: a dropped receiver means egress was torn down.
        let _ = self.tx.send(EgressItem::Byte(byte));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::peripherals::uart::UartStreamDevice;
    use std::sync::mpsc::channel;

    #[test]
    fn tap_forwards_tx_bytes_and_never_emits_rx() {
        let (tx, rx) = channel();
        let mut tap = EgressTap::new(tx);
        // Tap is TX-observe-only: poll must never inject RX bytes.
        assert_eq!(tap.poll(1000), None);
        tap.on_tx_byte(0x41);
        tap.on_tx_byte(0x42);
        assert_eq!(rx.recv().unwrap(), EgressItem::Byte(0x41));
        assert_eq!(rx.recv().unwrap(), EgressItem::Byte(0x42));
    }
}
