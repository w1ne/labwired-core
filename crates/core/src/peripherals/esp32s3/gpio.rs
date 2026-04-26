// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! GPIO peripheral for ESP32-S3 (GPIO0..31 only).
//!
//! Per ESP32-S3 TRM §5.5. Plan 3 scope:
//! - Output direction (ENABLE/ENABLE_W1TS/ENABLE_W1TC)
//! - Output value (OUT/OUT_W1TS/OUT_W1TC)
//! - Input value (IN, read-only; settable via `set_pin_input` for tests)
//! - Per-pin int_type/int_ena registers (round-trip storage; GPIO-input
//!   IRQs not yet routed to the intmatrix in Plan 3)
//!
//! ## GpioObserver
//!
//! The peripheral notifies registered observers synchronously on every
//! pin transition. Observers receive `(pin, from, to, sim_cycle)` and
//! must not panic.

use crate::{Peripheral, PeripheralTickResult, SimResult};
use std::sync::Arc;

/// Notified synchronously inside the bus write path on every GPIO pin
/// transition. Observers must not panic — a panic propagates out of
/// `bus.write_u8` and crashes the simulator.
pub trait GpioObserver: Send + Sync + std::fmt::Debug {
    fn on_pin_change(&self, pin: u8, from: bool, to: bool, sim_cycle: u64);
}

/// ESP32-S3 GPIO peripheral. Mapped at 0x6000_4000.
pub struct Esp32s3Gpio {
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
        Self {
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

    /// Internal: read a 32-bit register at the given word-aligned offset.
    fn read_word(&self, word_off: u64) -> u32 {
        match word_off {
            0x04 => self.out,
            0x08 => self.out,            // OUT_W1TS read returns OUT value
            0x0C => self.out,            // OUT_W1TC read returns OUT value
            0x20 => self.enable,
            0x24 => self.enable,
            0x28 => self.enable,
            0x3C => self.in_data,
            // PINn_REG at 0x74 + pin*4
            off if (0x74..0x74 + 32 * 4).contains(&off) => {
                let pin = ((off - 0x74) / 4) as usize;
                let int_type = self.int_type[pin] as u32;
                let int_ena = (self.int_enable >> pin) & 1;
                // Bits per TRM §5.5 GPIO_PINn_REG:
                //   bits[9:7]  INT_TYPE
                //   bits[16:13] INT_ENA (we model only bit 13 = cpu0 enable)
                (int_type << 7) | (int_ena << 13)
            }
            _ => 0,
        }
    }

    /// Internal: write a 32-bit value to the given word-aligned offset.
    fn write_word(&mut self, word_off: u64, value: u32) {
        match word_off {
            0x04 => self.apply_out(value),
            0x08 => {
                let new = self.out | value;
                self.apply_out(new);
            }
            0x0C => {
                let new = self.out & !value;
                self.apply_out(new);
            }
            0x20 => self.enable = value,
            0x24 => self.enable |= value,
            0x28 => self.enable &= !value,
            // GPIO_IN_REG at 0x3C is read-only; ignore writes.
            0x3C => {}
            off if (0x74..0x74 + 32 * 4).contains(&off) => {
                let pin = ((off - 0x74) / 4) as usize;
                self.int_type[pin] = ((value >> 7) & 0x7) as u8;
                let bit = (value >> 13) & 1;
                if bit != 0 {
                    self.int_enable |= 1u32 << pin;
                } else {
                    self.int_enable &= !(1u32 << pin);
                }
            }
            _ => {}
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
        let mut word = self.read_word(word_off);
        // For W1TS / W1TC, the existing word in the peripheral is read
        // through `read_word` which returns the OUT value — so an R-M-W
        // byte write to OUT_W1TS at offset 0x08 byte 0 with value 0x04
        // becomes: word = OUT, word.byte0 = 0x04, then write_word(0x08, word)
        // sets bit 2 of OUT. That's the desired behaviour.
        word &= !(0xFFu32 << byte_off);
        word |= (value as u32) << byte_off;
        self.write_word(word_off, word);
        Ok(())
    }

    fn tick(&mut self) -> PeripheralTickResult {
        self.cycle = self.cycle.wrapping_add(1);
        PeripheralTickResult::default()
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
        assert!(events.iter().any(|&(p, f, t, _)| p == 2 && !f && t),
                "expected pin 2 0->1 transition; events: {events:?}");
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
        assert!(events.iter().any(|&(p, f, t, _)| p == 2 && f && !t),
                "expected pin 2 1->0 transition; events: {events:?}");
    }

    #[test]
    fn direct_out_write_fires_observer_for_each_changed_bit() {
        let mut g = Esp32s3Gpio::new();
        let obs = Arc::new(TestObserver::default());
        g.add_observer(obs.clone());

        // Direct word-write to OUT setting bits 0, 5, 7 simultaneously.
        let val = (1u32 << 0) | (1u32 << 5) | (1u32 << 7);
        for byte in 0..4u64 {
            g.write(0x04 + byte, ((val >> (byte * 8)) & 0xFF) as u8).unwrap();
        }

        let events = obs.events.lock().unwrap();
        let pins_set: Vec<u8> = events.iter()
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
        g.write(0x08, 0x04).unwrap();
        g.write(0x09, 0x00).unwrap();
        g.write(0x0A, 0x00).unwrap();
        g.write(0x0B, 0x00).unwrap();

        assert!(obs.events.lock().unwrap().is_empty(),
                "no observer event for unchanged bits");
    }

    #[test]
    fn enable_w1ts_sets_enable_bit() {
        let mut g = Esp32s3Gpio::new();
        g.write(0x24, 0x04).unwrap();
        g.write(0x25, 0x00).unwrap();
        g.write(0x26, 0x00).unwrap();
        g.write(0x27, 0x00).unwrap();
        assert_eq!(g.enable & 0x04, 0x04);
    }

    #[test]
    fn enable_w1tc_clears_enable_bit() {
        let mut g = Esp32s3Gpio::new();
        g.enable = 0x04;
        g.write(0x28, 0x04).unwrap();
        g.write(0x29, 0x00).unwrap();
        g.write(0x2A, 0x00).unwrap();
        g.write(0x2B, 0x00).unwrap();
        assert_eq!(g.enable & 0x04, 0);
    }

    #[test]
    fn pin_reg_round_trips_int_type_and_int_ena() {
        let mut g = Esp32s3Gpio::new();
        // For pin 5: int_type = 3 (any-edge), int_ena (bit 13) = 1.
        // Word value: (3 << 7) | (1 << 13) = 0x180 | 0x2000 = 0x2180.
        let off = 0x74 + 5 * 4;
        let val = (3u32 << 7) | (1u32 << 13);
        for byte in 0..4u64 {
            g.write(off + byte, ((val >> (byte * 8)) & 0xFF) as u8).unwrap();
        }
        // Read back.
        let mut read = 0u32;
        for byte in 0..4u64 {
            read |= (g.read(off + byte).unwrap() as u32) << (byte * 8);
        }
        assert_eq!(read, val, "PIN5_REG round-trip mismatch");
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
        g.write(0x04, 0x01).unwrap();
        g.write(0x05, 0x00).unwrap();
        g.write(0x06, 0x00).unwrap();
        g.write(0x07, 0x00).unwrap();

        let events = obs.events.lock().unwrap();
        let evt = events.iter().find(|&&(p, _, _, _)| p == 0).expect("pin 0 evt");
        assert_eq!(evt.3, 5, "cycle stamp must be 5 after 5 ticks");
    }

    #[test]
    fn multiple_observers_all_get_notified() {
        let mut g = Esp32s3Gpio::new();
        let a = Arc::new(TestObserver::default());
        let b = Arc::new(TestObserver::default());
        g.add_observer(a.clone());
        g.add_observer(b.clone());
        g.write(0x08, 0x01).unwrap();
        g.write(0x09, 0x00).unwrap();
        g.write(0x0A, 0x00).unwrap();
        g.write(0x0B, 0x00).unwrap();
        assert_eq!(a.events.lock().unwrap().len(), 1);
        assert_eq!(b.events.lock().unwrap().len(), 1);
    }
}
