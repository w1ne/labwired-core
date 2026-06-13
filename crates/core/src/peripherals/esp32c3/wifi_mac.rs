// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! ESP32-C3 WiFi MAC (`0x6003_3000`, 12 KiB) — behavioral model for the
//! MAC ↔ SimNet bridge.
//!
//! Most of the MAC window is register-backed (the driver's bring-up does
//! read-modify-write + driver-managed scratch — see the Layer-2 RE in
//! `docs/esp32c3_wifi_mac_bridge.md`). On top of that this model implements the
//! pieces that need real behaviour to move frames:
//!
//! * **MAC-ready** (`0xD14` bit0): the HAL busy-polls it before `mac_txrx_init`;
//!   reported set (folds in the former standalone `wifi_mac_ready` override).
//! * **RX descriptor ring**: the driver writes the ring base to `0x88` — a
//!   singly-linked list of `{flags|len, buffer_ptr, next_ptr}` descriptors
//!   (word0 bit31 = owner/HW-may-fill, low 16 = buffer capacity, 1600).
//! * **Interrupt + event register** (`0xC3C` get / `0xC40` W1C clear): the MAC
//!   ISR `wDev_ProcessFiq` reads the event word; RX-success is `0x0100_4000`.
//!   The MAC interrupt is interrupt-matrix source 0.
//!
//! **RX inject** (`queue_rx_frame`): a received 802.11 frame is queued; on the
//! next bus tick the model walks the RX ring for an owner descriptor, DMAs the
//! frame into its buffer, writes back word0 (received length, owner cleared),
//! sets the RX-success event bits, and — while the event is pending — emits MAC
//! interrupt source 0 (level-sensitive, like the SYSTIMER), so the trap path
//! runs `wDev_ProcessFiq` → `lmacProcessRxSucData`.
//!
//! This is the SimNet-facing endpoint of the bridge; the frame source/sink (a
//! frame-level `VirtualAp`) pushes/pulls via `queue_rx_frame` / `take_tx_frames`.

use crate::{Bus, Peripheral, PeripheralTickResult, SimResult};
use std::collections::VecDeque;

const MAC_READY: u64 = 0xD14; // bit0 polled by hal_init
const RX_RING_BASE: u64 = 0x88; // driver writes the RX descriptor-list head here
const EVENT_GET: u64 = 0xC3C; // hal_mac_interrupt_get_event reads
const EVENT_CLR: u64 = 0xC40; // hal_mac_interrupt_clr_event writes (W1C)

/// RX-success event bits `wDev_ProcessFiq` routes to `lmacProcessRxSucData`.
const EVENT_RX_DONE: u32 = 0x0100_4000;
/// Descriptor word0 owner bit (HW may fill the buffer).
const DESC_OWNER: u32 = 1 << 31;
/// Interrupt-matrix source ID for the WiFi MAC (MAC_INTR_MAP @ offset 0).
const MAC_INTR_SOURCE: u32 = 0;

#[derive(Debug)]
pub struct Esp32c3WifiMac {
    regs: Vec<u32>,
    /// RX descriptor-ring head (DRAM pointer the driver wrote to `0x88`).
    rx_ring: u32,
    /// Frames received from the virtual network, awaiting DMA into the RX ring.
    pending_rx: VecDeque<Vec<u8>>,
    /// Frames the real MAC transmitted (captured TX), for the bridge to drain.
    tx_out: VecDeque<Vec<u8>>,
}

impl Default for Esp32c3WifiMac {
    fn default() -> Self {
        Self::new()
    }
}

impl Esp32c3WifiMac {
    pub fn new() -> Self {
        Self {
            regs: vec![0u32; 0x3000 / 4],
            rx_ring: 0,
            pending_rx: VecDeque::new(),
            tx_out: VecDeque::new(),
        }
    }

    /// Queue an 802.11 frame received from the virtual network; delivered to the
    /// driver's RX ring on the next bus tick.
    pub fn queue_rx_frame(&mut self, frame: Vec<u8>) {
        self.pending_rx.push_back(frame);
    }

    /// Drain frames the real MAC transmitted (captured on TX kick).
    pub fn take_tx_frames(&mut self) -> Vec<Vec<u8>> {
        self.tx_out.drain(..).collect()
    }

    fn event(&self) -> u32 {
        self.regs[(EVENT_GET / 4) as usize]
    }

    fn set_event(&mut self, bits: u32) {
        self.regs[(EVENT_GET / 4) as usize] |= bits;
    }

    /// Deliver one pending RX frame into the next owner-held RX descriptor.
    /// Returns true if a frame was delivered (RX event then set).
    fn deliver_one_rx(&mut self, bus: &mut dyn Bus) -> bool {
        if self.pending_rx.is_empty() || self.rx_ring == 0 {
            return false;
        }
        // Walk the singly-linked descriptor list for an owner-held descriptor.
        let mut desc = self.rx_ring;
        for _ in 0..16 {
            if desc == 0 || !(0x3fc0_0000..0x3fd0_0000).contains(&desc) {
                break;
            }
            let w0 = bus.read_u32(desc as u64).unwrap_or(0);
            let buf = bus.read_u32(desc as u64 + 4).unwrap_or(0);
            let next = bus.read_u32(desc as u64 + 8).unwrap_or(0);
            let cap = (w0 & 0xFFFF) as usize;
            if w0 & DESC_OWNER != 0 && buf != 0 && cap > 0 {
                let frame = self.pending_rx.pop_front().unwrap();
                let n = frame.len().min(cap);
                for (i, b) in frame.iter().take(n).enumerate() {
                    let _ = bus.write_u8(buf as u64 + i as u64, *b);
                }
                // Write back word0: received length in low 16, owner cleared.
                let new_w0 = (w0 & !0xFFFF & !DESC_OWNER) | (n as u32 & 0xFFFF);
                let _ = bus.write_u32(desc as u64, new_w0);
                self.set_event(EVENT_RX_DONE);
                return true;
            }
            desc = next;
        }
        false
    }
}

impl Peripheral for Esp32c3WifiMac {
    fn read(&self, offset: u64) -> SimResult<u8> {
        let w = self.read_u32(offset & !3)?;
        Ok((w >> ((offset & 3) * 8)) as u8)
    }

    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        let aligned = offset & !3;
        let sh = (offset & 3) * 8;
        let cur = *self.regs.get((aligned / 4) as usize).unwrap_or(&0);
        self.write_u32(aligned, (cur & !(0xFFu32 << sh)) | ((value as u32) << sh))
    }

    fn read_u32(&self, offset: u64) -> SimResult<u32> {
        let stored = *self.regs.get((offset / 4) as usize).unwrap_or(&0);
        Ok(match offset {
            // hal_init busy-polls bit0 for MAC clock/reset ready.
            MAC_READY => stored | 1,
            _ => stored,
        })
    }

    fn write_u32(&mut self, offset: u64, value: u32) -> SimResult<()> {
        match offset {
            // W1C: the ISR clears the event bits it handled.
            EVENT_CLR => {
                if let Some(slot) = self.regs.get_mut((EVENT_GET / 4) as usize) {
                    *slot &= !value;
                }
                if let Some(slot) = self.regs.get_mut((offset / 4) as usize) {
                    *slot = value;
                }
            }
            RX_RING_BASE => {
                self.rx_ring = value;
                if let Some(slot) = self.regs.get_mut((offset / 4) as usize) {
                    *slot = value;
                }
            }
            _ => {
                if let Some(slot) = self.regs.get_mut((offset / 4) as usize) {
                    *slot = value;
                }
            }
        }
        Ok(())
    }

    fn needs_bus_tick(&self) -> bool {
        true
    }

    fn tick_with_bus(&mut self, bus: &mut dyn Bus) {
        // Deliver at most one queued RX frame per tick into the descriptor ring.
        self.deliver_one_rx(bus);
    }

    fn tick(&mut self) -> PeripheralTickResult {
        // Level-sensitive: while an RX (or other) event is pending, keep
        // asserting the MAC interrupt source so wDev_ProcessFiq runs and clears
        // it via EVENT_CLR. Matches the SYSTIMER alarm delivery model.
        if self.event() != 0 {
            PeripheralTickResult {
                explicit_irqs: Some(vec![MAC_INTR_SOURCE]),
                ..PeripheralTickResult::default()
            }
        } else {
            PeripheralTickResult::default()
        }
    }

    fn as_any(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }

    fn as_any_mut(&mut self) -> Option<&mut dyn std::any::Any> {
        Some(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bus::SystemBus;

    #[test]
    fn mac_ready_bit_set() {
        let m = Esp32c3WifiMac::new();
        assert_eq!(m.read_u32(MAC_READY).unwrap() & 1, 1);
    }

    #[test]
    fn event_clear_is_w1c() {
        let mut m = Esp32c3WifiMac::new();
        m.set_event(EVENT_RX_DONE);
        assert_eq!(m.read_u32(EVENT_GET).unwrap(), EVENT_RX_DONE);
        m.write_u32(EVENT_CLR, EVENT_RX_DONE).unwrap();
        assert_eq!(m.read_u32(EVENT_GET).unwrap(), 0);
    }

    #[test]
    fn tick_emits_mac_irq_while_event_pending() {
        let mut m = Esp32c3WifiMac::new();
        assert!(m.tick().explicit_irqs.is_none());
        m.set_event(EVENT_RX_DONE);
        assert_eq!(
            m.tick().explicit_irqs.as_deref(),
            Some(&[MAC_INTR_SOURCE][..])
        );
    }

    #[test]
    fn rx_inject_walks_ring_and_fills_descriptor() {
        // Lay out a 1-entry RX ring in RAM: desc @ 0x3fca4904, buffer @ 0x3fca4980.
        let mut bus = SystemBus::new();
        bus.ram.base_addr = 0x3fc8_0000;
        bus.ram.data = vec![0u8; 0x40000];
        let desc = 0x3fca_4904u32;
        let buf = 0x3fca_4980u32;
        bus.write_u32(desc as u64, DESC_OWNER | 1600).unwrap(); // owner, cap=1600
        bus.write_u32(desc as u64 + 4, buf).unwrap(); // buffer ptr
        bus.write_u32(desc as u64 + 8, 0).unwrap(); // next = end

        let mut mac = Esp32c3WifiMac::new();
        mac.write_u32(RX_RING_BASE, desc).unwrap();
        let frame = vec![0xB0u8, 0x00, 0xAA, 0xBB]; // a tiny "802.11" frame
        mac.queue_rx_frame(frame.clone());
        mac.tick_with_bus(&mut bus);

        // Descriptor word0: owner cleared, length = 4.
        let w0 = bus.read_u32(desc as u64).unwrap();
        assert_eq!(w0 & DESC_OWNER, 0, "owner cleared after RX");
        assert_eq!(w0 & 0xFFFF, 4, "received length written");
        // Frame bytes DMA'd into the buffer.
        for (i, b) in frame.iter().enumerate() {
            assert_eq!(bus.read_u8(buf as u64 + i as u64).unwrap(), *b);
        }
        // RX event set → MAC IRQ asserted.
        assert_ne!(mac.event() & EVENT_RX_DONE, 0);
        assert_eq!(
            mac.tick().explicit_irqs.as_deref(),
            Some(&[MAC_INTR_SOURCE][..])
        );
    }
}
