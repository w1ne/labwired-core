// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! GPIO peripheral for ESP32-S3.
//!
//! Base address `DR_REG_GPIO_BASE = 0x6000_4000`, architected span
//! 0x000..0x700 (last register `GPIO_DATE` @ 0x6FC). Per ESP32-S3 TRM §5.5.
//!
//! ## Behavioral model (Plan 3 — unchanged by the register-file slice)
//!
//! - Output direction (ENABLE/ENABLE_W1TS/ENABLE_W1TC @ 0x20/0x24/0x28)
//! - Output value (OUT/OUT_W1TS/OUT_W1TC @ 0x04/0x08/0x0C) with synchronous
//!   [`GpioObserver`] notification on every pin transition
//! - Input value (IN @ 0x3C; settable via `set_pin_input` for tests/boards)
//! - Boot straps (STRAP @ 0x38, read-only): 0x8 = SPI_FAST_FLASH_BOOT
//!   (GPIO0 high), captured from silicon over JTAG — the SVD reset value (0)
//!   would send the boot ROM into download mode
//! - PIN0..31 int_type/int_ena fields (bits [9:7] / bit 13) kept in sync with
//!   the stored register word (GPIO-input IRQs not yet routed to the
//!   intmatrix in Plan 3)
//!
//! ## Register file
//!
//! All 397 architected registers of the ESP32-S3 SVD `GPIO` block are
//! modeled: each register is seeded with its SVD reset value and a write
//! applies the register's writable-bit mask
//! (`stored = (stored & !wmask) | (value & wmask)`) — read-only registers
//! (PCPU_INT, PCPU_NMI_INT, CPUSDIO_INT and their `1` twins, STATUS_NEXT,
//! STATUS_NEXT1, STRAP) ignore writes. The PIN0..53 array (0x74, stride 4),
//! FUNC0..255_IN_SEL_CFG (0x154) and FUNC0..53_OUT_SEL_CFG (0x554) arrays are
//! handled as offset ranges sharing one `(reset, wmask)` spec each. The
//! second-bank registers (OUT1/ENABLE1/STATUS/STATUS1 with their W1TS/W1TC
//! views, IN1, pins 32..53) are masked storage with architected
//! write-1-to-set / write-1-to-clear arithmetic — no interrupt semantics or
//! GPIO-matrix routing is invented on top.
//!
//! Offsets outside the architected map (the 0x630..0x6F8 hole and everything
//! at/above 0x700) read as zero and ignore writes, NOT round-trip, so the SVD
//! behavioral coverage probe cannot mistake this model for generic storage.
//!
//! Reset values and write masks are sourced from the ESP32-S3 SVD; they are
//! NOT validated against silicon dumps (except STRAP, see above). The SVD
//! marks IN/IN1's `DATA_NEXT` field read-write — the TRM documents the
//! registers as read-only on silicon — so a write to IN/IN1 stores into the
//! same cell `set_pin_input` drives, keeping read-back coherent.
//!
//! ## GpioObserver
//!
//! The peripheral notifies registered observers synchronously on every
//! pin transition. Observers receive `(pin, from, to, sim_cycle)` and
//! must not panic.

use crate::{Peripheral, PeripheralTickResult, SimResult};
use std::sync::Arc;

const BT_SELECT: u64 = 0x00;
const OUT: u64 = 0x04;
const OUT_W1TS: u64 = 0x08;
const OUT_W1TC: u64 = 0x0C;
const OUT1: u64 = 0x10;
const OUT1_W1TS: u64 = 0x14;
const OUT1_W1TC: u64 = 0x18;
const SDIO_SELECT: u64 = 0x1C;
const ENABLE: u64 = 0x20;
const ENABLE_W1TS: u64 = 0x24;
const ENABLE_W1TC: u64 = 0x28;
const ENABLE1: u64 = 0x2C;
const ENABLE1_W1TS: u64 = 0x30;
const ENABLE1_W1TC: u64 = 0x34;
/// GPIO_STRAP_REG: latched boot-mode straps. The real boot ROM reads this to
/// choose flash-boot vs download. Reset seeded to 0x8 = SPI_FAST_FLASH_BOOT
/// (GPIO0 high), captured from silicon over JTAG.
const STRAP: u64 = 0x38;
const IN: u64 = 0x3C;
const IN1: u64 = 0x40;
const STATUS: u64 = 0x44;
const STATUS_W1TS: u64 = 0x48;
const STATUS_W1TC: u64 = 0x4C;
const STATUS1: u64 = 0x50;
const STATUS1_W1TS: u64 = 0x54;
const STATUS1_W1TC: u64 = 0x58;
/// PCPU_INT (0x5C), PCPU_NMI_INT (0x60), CPUSDIO_INT (0x64) — RO.
const PCPU_INT: u64 = 0x5C;
/// PCPU_INT1 (0x68), PCPU_NMI_INT1 (0x6C), CPUSDIO_INT1 (0x70) — RO.
const CPUSDIO_INT1: u64 = 0x70;
/// PIN0..PIN53 @ 0x74 + n*4 (SVD dim=54, stride 4).
const PIN0: u64 = 0x74;
const PIN31: u64 = PIN0 + 31 * 4;
const PIN53: u64 = PIN0 + 53 * 4;
const STATUS_NEXT: u64 = 0x14C;
const STATUS_NEXT1: u64 = 0x150;
/// FUNC0..255_IN_SEL_CFG @ 0x154 + n*4 (SVD dim=256, stride 4).
const FUNC0_IN_SEL_CFG: u64 = 0x154;
const FUNC255_IN_SEL_CFG: u64 = FUNC0_IN_SEL_CFG + 255 * 4;
/// FUNC0..53_OUT_SEL_CFG @ 0x554 + n*4 (SVD dim=54, stride 4).
const FUNC0_OUT_SEL_CFG: u64 = 0x554;
const FUNC53_OUT_SEL_CFG: u64 = FUNC0_OUT_SEL_CFG + 53 * 4;
const CLOCK_GATE: u64 = 0x62C;
/// GPIO_DATE (0x6FC) — version stamp, last architected register.
const REG_DATE: u64 = 0x6FC;

/// Second-bank registers carry GPIO32..53 → 22 valid bits.
const BANK1_MASK: u32 = 0x003F_FFFF;
/// PINn writable bits per SVD: sync stages [4:0] (bits 5/6 reserved),
/// pad_driver bit 7 + INT_TYPE [9:7] region, WAKEUP_ENABLE bit 10,
/// CONFIG [12:11], INT_ENA [17:13].
const PIN_WMASK: u32 = 0x0003_FF9F;

/// One word past the last architected register (`REG_DATE` @ 0x6FC).
const NWORDS: usize = 0x700 / 4;

/// `(reset value, writable-bit mask)` for the architected register at word
/// index `word` (offset `word * 4`), exactly per the ESP32-S3 SVD `GPIO`
/// block; `None` = hole in the register map (reads 0, ignores writes).
/// `wmask == 0` = read-only register (writes ignored, reset value sticks).
const fn spec(word: usize) -> Option<(u32, u32)> {
    match (word as u64) * 4 {
        BT_SELECT => Some((0x0000_0000, 0xFFFF_FFFF)),
        // OUT group: behavioral overlay (apply_out + observers).
        OUT..=OUT_W1TC => Some((0x0000_0000, 0xFFFF_FFFF)),
        OUT1..=OUT1_W1TC => Some((0x0000_0000, BANK1_MASK)),
        SDIO_SELECT => Some((0x0000_0000, 0x0000_00FF)),
        // ENABLE group: behavioral overlay.
        ENABLE..=ENABLE_W1TC => Some((0x0000_0000, 0xFFFF_FFFF)),
        ENABLE1..=ENABLE1_W1TC => Some((0x0000_0000, BANK1_MASK)),
        STRAP => Some((0x0000_0008, 0x0000_0000)), // RO, silicon-captured
        IN => Some((0x0000_0000, 0xFFFF_FFFF)),    // behavioral (in_data)
        IN1 => Some((0x0000_0000, BANK1_MASK)),
        STATUS..=STATUS_W1TC => Some((0x0000_0000, 0xFFFF_FFFF)),
        STATUS1..=STATUS1_W1TC => Some((0x0000_0000, BANK1_MASK)),
        PCPU_INT..=CPUSDIO_INT1 => Some((0x0000_0000, 0x0000_0000)), // RO
        PIN0..=PIN53 => Some((0x0000_0000, PIN_WMASK)),
        STATUS_NEXT => Some((0x0000_0000, 0x0000_0000)), // RO
        STATUS_NEXT1 => Some((0x0000_0000, 0x0000_0000)), // RO
        FUNC0_IN_SEL_CFG..=FUNC255_IN_SEL_CFG => Some((0x0000_0000, 0x0000_00FF)),
        FUNC0_OUT_SEL_CFG..=FUNC53_OUT_SEL_CFG => Some((0x0000_0100, 0x0000_0FFF)),
        CLOCK_GATE => Some((0x0000_0001, 0x0000_0001)),
        REG_DATE => Some((0x0190_7040, 0x0FFF_FFFF)),
        _ => None,
    }
}

/// Notified synchronously inside the bus write path on every GPIO pin
/// transition. Observers must not panic — a panic propagates out of
/// `bus.write_u8` and crashes the simulator.
pub trait GpioObserver: Send + Sync + std::fmt::Debug {
    fn on_pin_change(&self, pin: u8, from: bool, to: bool, sim_cycle: u64);
}

/// ESP32-S3 GPIO peripheral. Mapped at 0x6000_4000.
pub struct Esp32s3Gpio {
    /// Register file for the architected map (word-indexed; holes stay 0 and
    /// are never read back — `spec()` gates both directions). OUT, ENABLE and
    /// IN live in the dedicated behavioral fields below instead.
    regs: [u32; NWORDS],
    enable: u32,
    out: u32,
    in_data: u32,
    int_enable: u32,
    int_type: [u8; 32],
    cycle: u64,
    observers: Vec<Arc<dyn GpioObserver>>,
}

impl Esp32s3Gpio {
    pub fn new() -> Self {
        let mut regs = [0u32; NWORDS];
        let mut w = 0;
        while w < NWORDS {
            if let Some((reset, _)) = spec(w) {
                regs[w] = reset;
            }
            w += 1;
        }
        Self {
            regs,
            enable: 0,
            out: 0,
            in_data: 0,
            int_enable: 0,
            int_type: [0; 32],
            cycle: 0,
            observers: Vec::new(),
        }
    }

    pub fn add_observer(&mut self, obs: Arc<dyn GpioObserver>) {
        self.observers.push(obs);
    }

    /// Set the input level on `pin` (0..=31). Used by tests / future
    /// stimulus generators.
    pub fn set_pin_input(&mut self, pin: u8, level: bool) {
        assert!(pin < 32, "set_pin_input: pin {pin} >= 32");
        if level {
            self.in_data |= 1u32 << pin;
        } else {
            self.in_data &= !(1u32 << pin);
        }
    }

    /// Internal: apply a new `out` value, fire observers for each
    /// flipped bit.
    fn apply_out(&mut self, new_out: u32) {
        let old = self.out;
        let new = new_out;
        self.out = new;
        let diff = old ^ new;
        if diff == 0 {
            return;
        }
        for pin in 0u8..32 {
            let mask = 1u32 << pin;
            if diff & mask != 0 {
                let from = old & mask != 0;
                let to = new & mask != 0;
                for obs in &self.observers {
                    obs.on_pin_change(pin, from, to, self.cycle);
                }
            }
        }
    }

    /// Architected register-file read; holes read 0.
    fn reg(&self, off: u64) -> u32 {
        let w = (off / 4) as usize;
        if w < NWORDS && spec(w).is_some() {
            self.regs[w]
        } else {
            0
        }
    }

    /// Masked store into an architected register; no-op on holes and on
    /// fully read-only registers (`wmask == 0`).
    fn set_reg_masked(&mut self, off: u64, value: u32) {
        let w = (off / 4) as usize;
        if w < NWORDS {
            if let Some((_, wmask)) = spec(w) {
                self.regs[w] = (self.regs[w] & !wmask) | (value & wmask);
            }
        }
    }

    /// Internal: read a 32-bit register at the given word-aligned offset.
    fn read_word(&self, word_off: u64) -> u32 {
        match word_off {
            // W1TS/W1TC views read back the primary register's value.
            OUT | OUT_W1TS | OUT_W1TC => self.out,
            ENABLE | ENABLE_W1TS | ENABLE_W1TC => self.enable,
            IN => self.in_data,
            OUT1_W1TS | OUT1_W1TC => self.reg(OUT1),
            ENABLE1_W1TS | ENABLE1_W1TC => self.reg(ENABLE1),
            STATUS_W1TS | STATUS_W1TC => self.reg(STATUS),
            STATUS1_W1TS | STATUS1_W1TC => self.reg(STATUS1),
            // Everything else (incl. STRAP, OUT1, IN1, PINn, FUNCn_*_SEL_CFG)
            // is served by the register file; holes read 0.
            off => self.reg(off),
        }
    }

    /// Internal: write a 32-bit value to the given word-aligned offset.
    fn write_word(&mut self, word_off: u64, value: u32) {
        match word_off {
            OUT => self.apply_out(value),
            OUT_W1TS => self.apply_out(self.out | value),
            OUT_W1TC => self.apply_out(self.out & !value),
            ENABLE => self.enable = value,
            ENABLE_W1TS => self.enable |= value,
            ENABLE_W1TC => self.enable &= !value,
            // The SVD marks IN.DATA_NEXT read-write: a write stores into the
            // same cell `set_pin_input` drives (the TRM documents the
            // register as RO on silicon; firmware never writes it).
            IN => self.in_data = value,
            // Second-bank W1TS/W1TC arithmetic targets the primary register;
            // the spec wmask confines the effect to the architected bits.
            OUT1_W1TS => self.set_reg_masked(OUT1, self.reg(OUT1) | value),
            OUT1_W1TC => self.set_reg_masked(OUT1, self.reg(OUT1) & !value),
            ENABLE1_W1TS => self.set_reg_masked(ENABLE1, self.reg(ENABLE1) | value),
            ENABLE1_W1TC => self.set_reg_masked(ENABLE1, self.reg(ENABLE1) & !value),
            STATUS_W1TS => self.set_reg_masked(STATUS, self.reg(STATUS) | value),
            STATUS_W1TC => self.set_reg_masked(STATUS, self.reg(STATUS) & !value),
            STATUS1_W1TS => self.set_reg_masked(STATUS1, self.reg(STATUS1) | value),
            STATUS1_W1TC => self.set_reg_masked(STATUS1, self.reg(STATUS1) & !value),
            // PIN0..31: masked storage + keep the behavioral int_type /
            // int_ena fields in sync (bits [9:7] / bit 13 per TRM §5.5).
            off @ PIN0..=PIN31 => {
                self.set_reg_masked(off, value);
                let stored = self.reg(off);
                let pin = ((off - PIN0) / 4) as usize;
                self.int_type[pin] = ((stored >> 7) & 0x7) as u8;
                if (stored >> 13) & 1 != 0 {
                    self.int_enable |= 1u32 << pin;
                } else {
                    self.int_enable &= !(1u32 << pin);
                }
            }
            // Everything else: masked store into the architected register;
            // RO registers (incl. STRAP) and holes ignore writes entirely.
            off => self.set_reg_masked(off, value),
        }
    }
}

impl std::fmt::Debug for Esp32s3Gpio {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Esp32s3Gpio(enable=0x{:08x} out=0x{:08x} in=0x{:08x} cycle={} obs={})",
            self.enable,
            self.out,
            self.in_data,
            self.cycle,
            self.observers.len(),
        )
    }
}

impl Default for Esp32s3Gpio {
    fn default() -> Self {
        Self::new()
    }
}

impl Peripheral for Esp32s3Gpio {
    fn read(&self, offset: u64) -> SimResult<u8> {
        let word_off = offset & !3;
        let byte_off = (offset & 3) * 8;
        let word = self.read_word(word_off);
        Ok(((word >> byte_off) & 0xFF) as u8)
    }

    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        let word_off = offset & !3;
        let byte_off = (offset & 3) * 8;
        // For W1TS, the existing word in the peripheral is read through
        // `read_word` which returns the primary register's value — so an
        // R-M-W byte write to OUT_W1TS at offset 0x08 byte 0 with value 0x04
        // becomes: word = OUT, word.byte0 = 0x04, then write_word(0x08, word)
        // sets bit 2 of OUT (OR-ing the current value back in is idempotent).
        // W1TC must merge against 0 instead: folding the current register
        // value into the unwritten bytes would clear every bit set there.
        let mut word = match word_off {
            OUT_W1TC | OUT1_W1TC | ENABLE_W1TC | ENABLE1_W1TC | STATUS_W1TC | STATUS1_W1TC => 0,
            off => self.read_word(off),
        };
        word &= !(0xFFu32 << byte_off);
        word |= (value as u32) << byte_off;
        self.write_word(word_off, word);
        Ok(())
    }

    fn tick(&mut self) -> PeripheralTickResult {
        self.cycle = self.cycle.wrapping_add(1);
        PeripheralTickResult::default()
    }

    fn read_gpio_input(&self, pin: u8) -> Option<bool> {
        if pin >= 32 {
            return None;
        }
        Some((self.in_data & (1u32 << pin)) != 0)
    }

    fn read_gpio_output(&self, pin: u8) -> Option<bool> {
        if pin >= 32 {
            return None;
        }
        Some((self.out & (1u32 << pin)) != 0)
    }

    fn read_gpio_pad(&self, pin: u8) -> Option<bool> {
        if pin >= 32 {
            return None;
        }
        let mask = 1u32 << pin;
        // ENABLE is the output driver: enabled pins show the OUT latch,
        // everything else shows the (externally driven) input level.
        Some(if (self.enable & mask) != 0 {
            (self.out & mask) != 0
        } else {
            (self.in_data & mask) != 0
        })
    }

    fn set_gpio_input(&mut self, pin: u8, level: bool) -> bool {
        if pin >= 32 {
            return false;
        }
        self.set_pin_input(pin, level);
        true
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
    use std::sync::Mutex;

    /// Simple recording observer for tests.
    #[derive(Debug, Default)]
    struct TestObserver {
        events: Mutex<Vec<(u8, bool, bool, u64)>>,
    }

    impl GpioObserver for TestObserver {
        fn on_pin_change(&self, pin: u8, from: bool, to: bool, sim_cycle: u64) {
            self.events.lock().unwrap().push((pin, from, to, sim_cycle));
        }
    }

    fn write_u32(g: &mut Esp32s3Gpio, off: u64, val: u32) {
        for byte in 0..4u64 {
            g.write(off + byte, ((val >> (byte * 8)) & 0xFF) as u8)
                .unwrap();
        }
    }

    fn read_u32(g: &Esp32s3Gpio, off: u64) -> u32 {
        let mut read = 0u32;
        for byte in 0..4u64 {
            read |= (g.read(off + byte).unwrap() as u32) << (byte * 8);
        }
        read
    }

    #[test]
    fn out_w1ts_sets_bit_and_fires_observer() {
        let mut g = Esp32s3Gpio::new();
        let obs = Arc::new(TestObserver::default());
        g.add_observer(obs.clone());

        // Write 0x04 (set bit 2) to OUT_W1TS at offset 0x08.
        // Use byte-level writes (the bus path).
        g.write(0x08, 0x04).unwrap();
        // Higher bytes are 0 — no-op, but they go through write_word too.
        g.write(0x09, 0x00).unwrap();
        g.write(0x0A, 0x00).unwrap();
        g.write(0x0B, 0x00).unwrap();

        assert_eq!(g.out & 0x04, 0x04, "OUT bit 2 must be set");
        let events = obs.events.lock().unwrap();
        assert!(
            events.iter().any(|&(p, f, t, _)| p == 2 && !f && t),
            "expected pin 2 0->1 transition; events: {events:?}"
        );
    }

    #[test]
    fn out_w1tc_clears_bit_and_fires_observer() {
        let mut g = Esp32s3Gpio::new();
        // Pre-set OUT bit 2.
        g.apply_out(0x04);
        let obs = Arc::new(TestObserver::default());
        g.add_observer(obs.clone());

        // Write 0x04 to OUT_W1TC at offset 0x0C.
        g.write(0x0C, 0x04).unwrap();
        g.write(0x0D, 0x00).unwrap();
        g.write(0x0E, 0x00).unwrap();
        g.write(0x0F, 0x00).unwrap();

        assert_eq!(g.out & 0x04, 0, "OUT bit 2 must be cleared");
        let events = obs.events.lock().unwrap();
        assert!(
            events.iter().any(|&(p, f, t, _)| p == 2 && f && !t),
            "expected pin 2 1->0 transition; events: {events:?}"
        );
    }

    #[test]
    fn direct_out_write_fires_observer_for_each_changed_bit() {
        let mut g = Esp32s3Gpio::new();
        let obs = Arc::new(TestObserver::default());
        g.add_observer(obs.clone());

        // Direct word-write to OUT setting bits 0, 5, 7 simultaneously.
        let val = (1u32 << 0) | (1u32 << 5) | (1u32 << 7);
        write_u32(&mut g, 0x04, val);

        let events = obs.events.lock().unwrap();
        let pins_set: Vec<u8> = events
            .iter()
            .filter(|&&(_, f, t, _)| !f && t)
            .map(|&(p, _, _, _)| p)
            .collect();
        assert!(pins_set.contains(&0), "pin 0 should have transitioned");
        assert!(pins_set.contains(&5), "pin 5 should have transitioned");
        assert!(pins_set.contains(&7), "pin 7 should have transitioned");
    }

    #[test]
    fn writing_same_value_does_not_fire_observer() {
        let mut g = Esp32s3Gpio::new();
        g.apply_out(0x04);
        let obs = Arc::new(TestObserver::default());
        g.add_observer(obs.clone());

        // Write OUT_W1TS bit 2 (already set).
        write_u32(&mut g, 0x08, 0x04);

        assert!(
            obs.events.lock().unwrap().is_empty(),
            "no observer event for unchanged bits"
        );
    }

    #[test]
    fn enable_w1ts_sets_enable_bit() {
        let mut g = Esp32s3Gpio::new();
        write_u32(&mut g, 0x24, 0x04);
        assert_eq!(g.enable & 0x04, 0x04);
    }

    #[test]
    fn enable_w1tc_clears_enable_bit() {
        let mut g = Esp32s3Gpio::new();
        g.enable = 0x04;
        write_u32(&mut g, 0x28, 0x04);
        assert_eq!(g.enable & 0x04, 0);
    }

    #[test]
    fn pin_reg_round_trips_int_type_and_int_ena() {
        let mut g = Esp32s3Gpio::new();
        // For pin 5: int_type = 3 (any-edge), int_ena (bit 13) = 1.
        // Word value: (3 << 7) | (1 << 13) = 0x180 | 0x2000 = 0x2180.
        let off = 0x74 + 5 * 4;
        let val = (3u32 << 7) | (1u32 << 13);
        write_u32(&mut g, off, val);
        assert_eq!(read_u32(&g, off), val, "PIN5_REG round-trip mismatch");
        assert_eq!(g.int_type[5], 3);
        assert_eq!(g.int_enable & (1u32 << 5), 1u32 << 5);
    }

    #[test]
    fn cycle_increments_each_tick_and_observer_sees_it() {
        let mut g = Esp32s3Gpio::new();
        let obs = Arc::new(TestObserver::default());
        g.add_observer(obs.clone());
        // Tick 5 times to advance cycle.
        for _ in 0..5 {
            g.tick();
        }
        // Now flip pin 0.
        write_u32(&mut g, 0x04, 0x01);

        let events = obs.events.lock().unwrap();
        let evt = events
            .iter()
            .find(|&&(p, _, _, _)| p == 0)
            .expect("pin 0 evt");
        assert_eq!(evt.3, 5, "cycle stamp must be 5 after 5 ticks");
    }

    #[test]
    fn multiple_observers_all_get_notified() {
        let mut g = Esp32s3Gpio::new();
        let a = Arc::new(TestObserver::default());
        let b = Arc::new(TestObserver::default());
        g.add_observer(a.clone());
        g.add_observer(b.clone());
        write_u32(&mut g, 0x08, 0x01);
        assert_eq!(a.events.lock().unwrap().len(), 1);
        assert_eq!(b.events.lock().unwrap().len(), 1);
    }

    // ── register-file slice ──────────────────────────────────────────────

    #[test]
    fn reset_defaults_seeded() {
        let g = Esp32s3Gpio::new();
        // STRAP keeps the silicon-captured SPI_FAST_FLASH_BOOT value.
        assert_eq!(read_u32(&g, STRAP), 0x0000_0008);
        // FUNCn_OUT_SEL_CFG resets to 0x100 (GPIO-matrix bypass) across the
        // whole array — spot-check first, middle, last members.
        assert_eq!(read_u32(&g, FUNC0_OUT_SEL_CFG), 0x0000_0100);
        assert_eq!(read_u32(&g, FUNC0_OUT_SEL_CFG + 26 * 4), 0x0000_0100);
        assert_eq!(read_u32(&g, FUNC53_OUT_SEL_CFG), 0x0000_0100);
        assert_eq!(read_u32(&g, CLOCK_GATE), 0x0000_0001);
        assert_eq!(read_u32(&g, REG_DATE), 0x0190_7040);
        // Zero-reset members of the arrays.
        assert_eq!(read_u32(&g, PIN53), 0);
        assert_eq!(read_u32(&g, FUNC255_IN_SEL_CFG), 0);
    }

    #[test]
    fn config_registers_store_under_write_mask() {
        let mut g = Esp32s3Gpio::new();
        // BT_SELECT is fully writable.
        write_u32(&mut g, BT_SELECT, 0x1234_5678);
        assert_eq!(read_u32(&g, BT_SELECT), 0x1234_5678);
        // SDIO_SELECT: only [7:0] writable.
        write_u32(&mut g, SDIO_SELECT, 0xFFFF_FFFF);
        assert_eq!(read_u32(&g, SDIO_SELECT), 0x0000_00FF);
        // PINn array members store only the architected bits (incl. n > 31).
        write_u32(&mut g, PIN53, 0xFFFF_FFFF);
        assert_eq!(read_u32(&g, PIN53), PIN_WMASK);
        // FUNCn_IN_SEL_CFG: [7:0] writable, array spot-checks.
        write_u32(&mut g, FUNC0_IN_SEL_CFG, 0xFFFF_FFFF);
        assert_eq!(read_u32(&g, FUNC0_IN_SEL_CFG), 0x0000_00FF);
        write_u32(&mut g, FUNC255_IN_SEL_CFG, 0xDEAD_BEA7);
        assert_eq!(read_u32(&g, FUNC255_IN_SEL_CFG), 0x0000_00A7);
        // FUNCn_OUT_SEL_CFG: [11:0] writable.
        write_u32(&mut g, FUNC53_OUT_SEL_CFG, 0xFFFF_FFFF);
        assert_eq!(read_u32(&g, FUNC53_OUT_SEL_CFG), 0x0000_0FFF);
        // CLOCK_GATE: bit 0 only.
        write_u32(&mut g, CLOCK_GATE, 0xFFFF_FFFE);
        assert_eq!(read_u32(&g, CLOCK_GATE), 0);
        write_u32(&mut g, CLOCK_GATE, 1);
        assert_eq!(read_u32(&g, CLOCK_GATE), 1);
    }

    #[test]
    fn bank1_out_enable_w1ts_w1tc_arithmetic() {
        let mut g = Esp32s3Gpio::new();
        // OUT1: set bits via W1TS, clear via W1TC; 22-bit mask applies.
        write_u32(&mut g, OUT1_W1TS, 0xFFFF_FFFF);
        assert_eq!(read_u32(&g, OUT1), BANK1_MASK);
        assert_eq!(read_u32(&g, OUT1_W1TS), BANK1_MASK, "W1TS reads OUT1");
        write_u32(&mut g, OUT1_W1TC, 0x0000_0005);
        assert_eq!(read_u32(&g, OUT1), BANK1_MASK & !0x5);
        write_u32(&mut g, OUT1, 0);
        assert_eq!(read_u32(&g, OUT1), 0);
        // ENABLE1 mirrors the same arithmetic.
        write_u32(&mut g, ENABLE1_W1TS, 0x0000_0030);
        assert_eq!(read_u32(&g, ENABLE1), 0x30);
        write_u32(&mut g, ENABLE1_W1TC, 0x0000_0010);
        assert_eq!(read_u32(&g, ENABLE1), 0x20);
        assert_eq!(read_u32(&g, ENABLE1_W1TC), 0x20, "W1TC reads ENABLE1");
    }

    #[test]
    fn status_w1ts_w1tc_arithmetic() {
        let mut g = Esp32s3Gpio::new();
        write_u32(&mut g, STATUS_W1TS, 0x8000_0001);
        assert_eq!(read_u32(&g, STATUS), 0x8000_0001);
        write_u32(&mut g, STATUS_W1TC, 0x8000_0000);
        assert_eq!(read_u32(&g, STATUS), 0x0000_0001);
        write_u32(&mut g, STATUS1_W1TS, 0xFFFF_FFFF);
        assert_eq!(read_u32(&g, STATUS1), BANK1_MASK);
        write_u32(&mut g, STATUS1_W1TC, BANK1_MASK);
        assert_eq!(read_u32(&g, STATUS1), 0);
    }

    #[test]
    fn in_write_stores_into_input_cell_per_svd_access() {
        let mut g = Esp32s3Gpio::new();
        // SVD marks IN.DATA_NEXT read-write: writes land in the same cell
        // set_pin_input drives, so read-back stays coherent.
        write_u32(&mut g, IN, 0x0000_00F0);
        assert_eq!(read_u32(&g, IN), 0x0000_00F0);
        g.set_pin_input(0, true);
        assert_eq!(read_u32(&g, IN), 0x0000_00F1);
        // IN1 stores under the 22-bit second-bank mask.
        write_u32(&mut g, IN1, 0xFFFF_FFFF);
        assert_eq!(read_u32(&g, IN1), BANK1_MASK);
    }

    #[test]
    fn read_only_registers_ignore_writes() {
        let mut g = Esp32s3Gpio::new();
        // STRAP keeps its silicon-captured value.
        write_u32(&mut g, STRAP, 0xFFFF_FFFF);
        assert_eq!(read_u32(&g, STRAP), 0x0000_0008);
        // Interrupt-status mirrors are RO and stay 0.
        for off in [
            0x5C,
            0x60,
            0x64,
            0x68,
            0x6C,
            0x70,
            STATUS_NEXT,
            STATUS_NEXT1,
        ] {
            write_u32(&mut g, off, 0xFFFF_FFFF);
            assert_eq!(read_u32(&g, off), 0, "RO reg at {off:#x}");
        }
    }

    #[test]
    fn unmapped_offsets_read_zero_and_ignore_writes() {
        let mut g = Esp32s3Gpio::new();
        // The 0x630..0x6F8 hole and offsets at/above 0x700 must NOT
        // round-trip — the coverage probe's baseline depends on it.
        for off in [0x630u64, 0x680, 0x6F8, 0x700, 0x7FC] {
            write_u32(&mut g, off, 0xDEAD_BEEF);
            assert_eq!(read_u32(&g, off), 0, "hole at {off:#x}");
        }
    }
}
