// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! ARMv7-M ITM stimulus ports. Port 0 is a byte stream (the `ITM_SendChar`
//! printf path). Ports 1–31 accept an enabled store and drop the payload.
//!
//! `DEMCR.TRCENA` does not gate emission. CMSIS `ITM_SendChar` never reads it,
//! and a later debug-register phase makes `TRCENA` readable without turning it
//! into an ITM enable. Do not consult `DEMCR` from this model.

use std::sync::{Arc, Mutex};

use crate::{Peripheral, SimResult};

const STIM_WINDOW: u64 = 0x80;
const ITM_TER: u64 = 0xE00;
const ITM_TCR: u64 = 0xE80;
const ITM_LAR: u64 = 0xFB0;
const ITM_LSR: u64 = 0xFB4;

/// TCR.ITMENA. The rest of TCR is stored so a read-modify-write sticks, and
/// nothing else in the word changes emission.
const TCR_ITMENA: u32 = 1 << 0;

/// Registers plus the not-yet-drained port-0 bytes. Both the JSON snapshot and
/// the binary runtime snapshot carry this so a resume keeps a half-drained
/// stream.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
struct ItmState {
    tcr: u32,
    ter: u32,
    attached: bool,
    bytes_emitted: u64,
    bytes_dropped_other_ports: u64,
    captured: Vec<u8>,
}

#[derive(Debug)]
pub struct Itm {
    tcr: u32,
    ter: u32,
    /// Firmware has written TCR, TER, or any stimulus port. Reads do not count.
    attached: bool,
    bytes_emitted: u64,
    bytes_dropped_other_ports: u64,
    /// Port-0 bytes waiting for a drain. Absent retention (interactive echo
    /// with no capture sink) still counts `bytes_emitted` but does not grow
    /// this buffer.
    captured: Arc<Mutex<Vec<u8>>>,
    retain: bool,
    echo_stdout: bool,
}

impl Itm {
    // Nothing calls `Default`. The lint still wants the impl next to `new`.
    #[allow(clippy::new_without_default)]
    pub fn new() -> Self {
        Self {
            tcr: 0,
            ter: 0,
            attached: false,
            bytes_emitted: 0,
            bytes_dropped_other_ports: 0,
            captured: Arc::new(Mutex::new(Vec::new())),
            retain: true,
            echo_stdout: false,
        }
    }

    /// `sink == None` stops retaining (echo-only or explicitly suppressed).
    /// A capture sink replaces the buffer the drain reads.
    pub fn set_output(&mut self, sink: Option<Arc<Mutex<Vec<u8>>>>, echo_stdout: bool) {
        if let Some(sink) = sink {
            self.captured = sink;
            self.retain = true;
        } else {
            self.retain = false;
        }
        self.echo_stdout = echo_stdout;
    }

    pub fn drain_captured(&self) -> Vec<u8> {
        match self.captured.lock() {
            Ok(mut guard) => std::mem::take(&mut *guard),
            Err(_) => Vec::new(),
        }
    }

    pub fn attached(&self) -> bool {
        self.attached
    }

    pub fn bytes_emitted(&self) -> u64 {
        self.bytes_emitted
    }

    fn port_ready(&self, port: u32) -> bool {
        (self.tcr & TCR_ITMENA) != 0 && (self.ter & (1u32 << port)) != 0
    }

    fn emit(&mut self, port: u32, byte: u8) {
        if !self.port_ready(port) {
            return;
        }
        if port == 0 {
            self.bytes_emitted = self.bytes_emitted.saturating_add(1);
            if self.retain {
                if let Ok(mut guard) = self.captured.lock() {
                    guard.push(byte);
                }
            }
            if self.echo_stdout {
                use std::io::Write;
                let mut out = std::io::stdout();
                let _ = out.write_all(&[byte]);
                let _ = out.flush();
            }
        } else {
            self.bytes_dropped_other_ports = self.bytes_dropped_other_ports.saturating_add(1);
        }
    }

    fn state(&self) -> ItmState {
        ItmState {
            tcr: self.tcr,
            ter: self.ter,
            attached: self.attached,
            bytes_emitted: self.bytes_emitted,
            bytes_dropped_other_ports: self.bytes_dropped_other_ports,
            captured: self
                .captured
                .lock()
                .map(|guard| guard.clone())
                .unwrap_or_default(),
        }
    }

    fn apply_state(&mut self, state: ItmState) {
        self.tcr = state.tcr;
        self.ter = state.ter;
        self.attached = state.attached;
        self.bytes_emitted = state.bytes_emitted;
        self.bytes_dropped_other_ports = state.bytes_dropped_other_ports;
        if let Ok(mut guard) = self.captured.lock() {
            *guard = state.captured;
        }
    }

    fn read_word(&self, aligned: u64) -> u32 {
        match aligned {
            ITM_TER => self.ter,
            ITM_TCR => self.tcr,
            ITM_LAR | ITM_LSR => 0,
            off if off < STIM_WINDOW => u32::from(self.port_ready((off / 4) as u32)),
            _ => {
                crate::census_reg!("itm:Itm", aligned, "read");
                0
            }
        }
    }
}

impl Peripheral for Itm {
    // Emission is a store, not a tick. The trait defaults would put this
    // no-op on every Cortex-M walk-forcing set and block idle fast-forward.
    fn needs_legacy_walk(&self) -> bool {
        false
    }

    fn legacy_tick_active(&self) -> bool {
        false
    }

    fn read(&self, offset: u64) -> SimResult<u8> {
        let word = self.read_word(offset & !3);
        let lane = (offset & 3) as u32;
        Ok(((word >> (lane * 8)) & 0xFF) as u8)
    }

    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        let aligned = offset & !3;
        let lane = (offset & 3) * 8;
        let mask = 0xFFu32 << lane;
        let inserted = u32::from(value) << lane;
        match aligned {
            off if off < STIM_WINDOW => {
                // Any stimulus store counts as a touch, including one the
                // enable bits drop. The payload is emitted only when the port
                // is enabled, and only the lane this store wrote.
                self.attached = true;
                self.emit((off / 4) as u32, value);
            }
            ITM_TER => {
                self.attached = true;
                self.ter = (self.ter & !mask) | inserted;
            }
            ITM_TCR => {
                self.attached = true;
                self.tcr = (self.tcr & !mask) | inserted;
            }
            // Lock reads as open-not-implemented (0). Writes do not unlock and
            // do not count as the firmware touching the stimulus path.
            ITM_LAR | ITM_LSR => {}
            _ => {
                crate::census_reg!("itm:Itm", aligned, "write");
            }
        }
        Ok(())
    }

    fn peek(&self, offset: u64) -> Option<u8> {
        let aligned = offset & !3;
        let word = match aligned {
            ITM_TER => self.ter,
            ITM_TCR => self.tcr,
            ITM_LAR | ITM_LSR => 0,
            off if off < STIM_WINDOW => u32::from(self.port_ready((off / 4) as u32)),
            _ => return None,
        };
        let lane = (offset & 3) as u32;
        Some(((word >> (lane * 8)) & 0xFF) as u8)
    }

    fn as_any(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }

    fn as_any_mut(&mut self) -> Option<&mut dyn std::any::Any> {
        Some(self)
    }

    fn snapshot(&self) -> serde_json::Value {
        serde_json::to_value(self.state()).expect("Itm JSON snapshot")
    }

    fn restore(&mut self, state: serde_json::Value) -> SimResult<()> {
        if !state.is_object() {
            return Ok(());
        }
        let decoded = serde_json::from_value(state).map_err(|error| {
            crate::SimulationError::NotImplemented(format!("Itm snapshot decode: {error}"))
        })?;
        self.apply_state(decoded);
        Ok(())
    }

    fn runtime_snapshot(&self) -> Vec<u8> {
        bincode::serialize(&self.state()).expect("bincode serialize Itm")
    }

    fn restore_runtime_snapshot(&mut self, bytes: &[u8]) -> SimResult<()> {
        // Empty is the trait default from a snapshot taken before this model
        // overrode the hook. Leave the live peripheral alone.
        if bytes.is_empty() {
            return Ok(());
        }
        let state: ItmState = bincode::deserialize(bytes).map_err(|error| {
            crate::SimulationError::NotImplemented(format!("Itm snapshot decode: {error}"))
        })?;
        self.apply_state(state);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bus::SystemBus;
    use crate::system::cortex_m::configure_cortex_m;
    use crate::{Bus, Peripheral};

    const PORT0: u64 = 0xE000_0000;
    const TER: u64 = 0xE000_0E00;
    const TCR: u64 = 0xE000_0E80;

    fn bus() -> SystemBus {
        let mut bus = SystemBus::new();
        let _ = configure_cortex_m(&mut bus);
        bus
    }

    fn itm_mut(bus: &mut SystemBus) -> &mut Itm {
        bus.peripherals
            .iter_mut()
            .find(|p| p.name == "itm")
            .unwrap()
            .dev
            .as_any_mut()
            .unwrap()
            .downcast_mut::<Itm>()
            .unwrap()
    }

    #[test]
    fn installed_at_itm_base_without_widening_dwt() {
        let bus = bus();
        let itm = bus.peripherals.iter().find(|p| p.name == "itm").unwrap();
        assert_eq!(itm.base, 0xE000_0000);
        assert_eq!(itm.size, 0x1000);
        assert!(!itm.dev.needs_legacy_walk());
        assert!(!itm.dev.legacy_tick_active());
        let dwt = bus.peripherals.iter().find(|p| p.name == "dwt").unwrap();
        assert_eq!(dwt.base, 0xE000_1000);
        assert_eq!(dwt.size, 0x1000);
        assert!(bus.peripherals.iter().all(|p| p.base != 0xE004_0000));
    }

    #[test]
    fn disabled_store_emits_nothing_but_marks_attached() {
        let mut bus = bus();
        assert!(!bus.itm_attached());
        bus.write_u8(PORT0, b'X').unwrap();
        assert!(bus.drain_itm_output().is_empty());
        assert_eq!(bus.itm_bytes_emitted(), 0);
        // A stimulus store is a touch even when the byte is dropped.
        assert!(bus.itm_attached());
        assert_eq!(bus.read_u32(PORT0).unwrap(), 0);
    }

    #[test]
    fn enabled_u8_u16_and_u32_stores_emit_only_written_lanes() {
        let mut bus = bus();
        bus.write_u32(TCR, 0x1234_0001).unwrap();
        bus.write_u32(TER, 1).unwrap();
        assert_eq!(bus.read_u32(TCR).unwrap(), 0x1234_0001);
        assert_eq!(
            bus.read_u32(PORT0).unwrap(),
            1,
            "FIFO ready unblocks ITM_SendChar"
        );

        bus.write_u8(PORT0, b'A').unwrap();
        assert_eq!(bus.drain_itm_output(), b"A");

        // One lane, not the rest of the word.
        bus.write_u8(PORT0 + 1, 0x7F).unwrap();
        assert_eq!(bus.drain_itm_output(), &[0x7F]);

        bus.write_u16(PORT0, 0x4241).unwrap();
        assert_eq!(bus.drain_itm_output(), b"AB");

        // A word store of a character must not disappear, and must not invent
        // lanes the store did not write — a word writes all four.
        bus.write_u32(PORT0, 0x41).unwrap();
        assert_eq!(bus.drain_itm_output(), &[0x41, 0, 0, 0]);
    }

    #[test]
    fn other_ports_drop_only_when_enabled() {
        let mut bus = bus();
        bus.write_u32(TCR, 1).unwrap();
        bus.write_u8(PORT0 + 4, b'Z').unwrap();
        assert!(bus.drain_itm_output().is_empty());
        assert_eq!(itm_mut(&mut bus).bytes_dropped_other_ports, 0);

        bus.write_u32(TER, 1 << 1).unwrap();
        assert_eq!(bus.read_u32(PORT0 + 4).unwrap(), 1);
        bus.write_u32(PORT0 + 4, 0x1122_3344).unwrap();
        assert!(bus.drain_itm_output().is_empty());
        assert_eq!(itm_mut(&mut bus).bytes_dropped_other_ports, 4);
        assert_eq!(bus.itm_bytes_emitted(), 0);
    }

    #[test]
    fn clearing_itmena_stops_emission() {
        let mut bus = bus();
        bus.write_u32(TCR, 1).unwrap();
        bus.write_u32(TER, 1).unwrap();
        bus.write_u32(TCR, 0).unwrap();
        assert_eq!(bus.read_u32(TCR).unwrap(), 0);
        bus.write_u8(PORT0, b'Q').unwrap();
        assert!(bus.drain_itm_output().is_empty());
        assert_eq!(bus.read_u32(PORT0).unwrap(), 0);
    }

    #[test]
    fn lar_and_lsr_read_zero_and_ignore_writes() {
        let mut bus = bus();
        assert_eq!(bus.read_u32(0xE000_0FB0).unwrap(), 0);
        assert_eq!(bus.read_u32(0xE000_0FB4).unwrap(), 0);
        bus.write_u32(0xE000_0FB0, 0xC5AC_CE55).unwrap();
        bus.write_u32(0xE000_0FB4, 1).unwrap();
        assert_eq!(bus.read_u32(0xE000_0FB0).unwrap(), 0);
        assert_eq!(bus.read_u32(0xE000_0FB4).unwrap(), 0);
        assert!(!bus.itm_attached());
    }

    #[test]
    fn ter_or_tcr_write_attaches_without_emitting() {
        let mut bus = bus();
        bus.write_u8(TCR, 0).unwrap();
        assert!(bus.itm_attached());
        assert!(bus.drain_itm_output().is_empty());
    }

    #[test]
    fn snapshot_and_runtime_snapshot_round_trip_the_undrained_buffer() {
        let mut bus = bus();
        bus.write_u32(TCR, 1).unwrap();
        bus.write_u32(TER, 1).unwrap();
        bus.write_u8(PORT0, b'h').unwrap();
        bus.write_u8(PORT0, b'i').unwrap();

        let json = itm_mut(&mut bus).snapshot();
        let blob = itm_mut(&mut bus).runtime_snapshot();
        assert_eq!(bus.drain_itm_output(), b"hi");
        assert!(bus.drain_itm_output().is_empty());

        itm_mut(&mut bus).restore(json).unwrap();
        assert_eq!(bus.drain_itm_output(), b"hi");
        assert_eq!(bus.itm_bytes_emitted(), 2);

        itm_mut(&mut bus).restore_runtime_snapshot(&blob).unwrap();
        assert_eq!(bus.drain_itm_output(), b"hi");
        assert_eq!(itm_mut(&mut bus).bytes_emitted, 2);
        assert!(itm_mut(&mut bus).attached);

        let tcr = itm_mut(&mut bus).tcr;
        itm_mut(&mut bus).restore_runtime_snapshot(&[]).unwrap();
        assert_eq!(itm_mut(&mut bus).tcr, tcr);
    }
}
