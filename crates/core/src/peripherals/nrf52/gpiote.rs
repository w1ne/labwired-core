// LabWired - Firmware Simulation Platform
// SPDX-License-Identifier: MIT

//! Nordic nRF52 GPIOTE peripheral.
//!
//! Source: nRF52840 PS rev 1.7 §6.9 (GPIOTE). 8 channels, each with a
//! CONFIG word plus task aliases (TASKS_OUT, TASKS_SET, TASKS_CLR) and
//! event aliases (EVENTS_IN, EVENTS_PORT).
//!
//! # What the model does
//!
//! - **Register surface**: all task/event/CONFIG/INTEN registers
//!   round-trip per spec (cross-validated by hw-oracle).
//! - **Task → pad drive**: writing TASKS_OUT/SET/CLR[i] looks up
//!   CONFIG[i].PORT/PSEL/POLARITY/OUTINIT and drives the target pin's
//!   **physical pad level**, reflected in `GPIO.IN` (offset 0x510 / `idr`).
//!   This matches silicon: when a pin is in GPIOTE Task mode the GPIOTE
//!   peripheral owns the pad; the GPIO peripheral's `OUT` register (0x504)
//!   is **not modified** by GPIOTE tasks.  The driven level is therefore
//!   observable at `GPIO.IN`, not `GPIO.OUT`.  The implementation performs
//!   a read-modify-write on the port's `idr` (0x510) to set or clear only
//!   the target pin's bit while leaving all other bits unchanged.
//! - **Event observation**: EVENTS_IN is *not* driven from GPIO input
//!   changes (no input-pin model yet). Firmware that polls EVENTS_IN
//!   without PPI seeing edges will never see them fire.

use crate::{Peripheral, PeripheralTickResult, SimResult};

const OFF_TASKS_OUT_0: u64 = 0x000;
const OFF_TASKS_OUT_7: u64 = 0x01C;
const OFF_TASKS_SET_0: u64 = 0x030;
const OFF_TASKS_SET_7: u64 = 0x04C;
const OFF_TASKS_CLR_0: u64 = 0x060;
const OFF_TASKS_CLR_7: u64 = 0x07C;
const OFF_EVENTS_IN_0: u64 = 0x100;
const OFF_EVENTS_IN_7: u64 = 0x11C;
const OFF_EVENTS_PORT: u64 = 0x17C;
const OFF_INTENSET: u64 = 0x304;
const OFF_INTENCLR: u64 = 0x308;
const OFF_CONFIG_0: u64 = 0x510;
const OFF_CONFIG_7: u64 = 0x52C;

/// Per nRF52840 PS table 79: writable bits in CONFIG[i].
///   MODE     [1:0]   → 0x0000_0003
///   PSEL     [12:8]  → 0x0000_1F00
///   PORT     [13]    → 0x0000_2000
///   POLARITY [17:16] → 0x0003_0000
///   OUTINIT  [20]    → 0x0010_0000
const CONFIG_WRITE_MASK: u32 = 0x0013_3F03;

// CONFIG bitfields.
const CONFIG_MODE_MASK: u32 = 0x3;
const CONFIG_MODE_TASK: u32 = 3;
const CONFIG_PSEL_SHIFT: u32 = 8;
const CONFIG_PSEL_MASK: u32 = 0x1F;
const CONFIG_PORT_BIT: u32 = 1 << 13;
const CONFIG_POLARITY_SHIFT: u32 = 16;
const CONFIG_POLARITY_MASK: u32 = 0x3;
const CONFIG_OUTINIT_BIT: u32 = 1 << 20;

// POLARITY values.
const POLARITY_NONE: u32 = 0;
const POLARITY_LO_TO_HI: u32 = 1;
const POLARITY_HI_TO_LO: u32 = 2;
const POLARITY_TOGGLE: u32 = 3;

// GPIO port bases on nRF52840 (PS §6.10).
const GPIO0_BASE: u32 = 0x5000_0000;
const GPIO1_BASE: u32 = 0x5000_0300;
/// Offset of the IN register within a GPIO port (nRF52840 PS §6.10).
/// GPIOTE drives the pad level here; GPIO.OUT (0x504) is left untouched.
const GPIO_IN_OFFSET: u32 = 0x510;

#[derive(Debug, Default)]
pub struct Nrf52Gpiote {
    events_in: [u32; 8],
    events_port: u32,
    inten: u32,
    config: [u32; 8],

    /// Per-channel current output level — needed to honor POLARITY=Toggle
    /// (which has to know whether the pin is currently high or low).
    /// Seeded from CONFIG[i].OUTINIT on the first task fire after a config
    /// write; updated on every TASKS_OUT/SET/CLR.
    channel_out_level: [u32; 8],

    /// Queued GPIO writes accumulated since the last tick(); drained into
    /// the bus's cross-peripheral mmio_writes on every tick.
    pending_gpio_writes: Vec<(u32, u32)>,

    /// Per-channel last-known input level, used to detect rising/falling
    /// edges against the GPIO IN registers that the bus snapshots each
    /// tick.  Initialized lazily when CONFIG is written.
    channel_in_level: [u32; 8],

    /// EVENTS_IN[i] offsets queued by `observe_gpio_change`; drained into
    /// the next tick's fired_events so PPI sees them and IRQ is pended
    /// at the same time GPIOTE's mmio_writes are applied.
    pending_in_events: Vec<u32>,

    /// Set to true on every GPIOTE channel that asserted EVENTS_IN since
    /// the last tick.  tick() returns irq:true if any bit overlaps INTEN.
    pending_in_mask: u32,

    /// Shadow of the `idr` word GPIOTE has written into each GPIO port's
    /// IN register (0x510).  Index 0 = GPIO0, index 1 = GPIO1.
    /// Used for read-modify-write when a task drives a single pin: we only
    /// flip the target bit and leave all other pad-driven bits unchanged.
    /// `GPIO.OUT` (0x504) is never touched by GPIOTE tasks.
    idr_shadow: [u32; 2],
}

impl Nrf52Gpiote {
    pub fn new() -> Self {
        Self::default()
    }

    /// Drive the target pin's pad level (reflected in `GPIO.IN` at offset 0x510).
    ///
    /// Silicon behaviour: when a pin is in GPIOTE Task mode the GPIOTE peripheral
    /// drives the physical pad; the GPIO `OUT` register (0x504) is **not touched**.
    ///
    /// We queue a full-word write to `GPIO.IN` (0x510 / `idr`) with the target
    /// pin's bit set or cleared.  Because `Nrf52Gpio::write_reg(0x510, v)` stores
    /// the whole word into `idr`, we maintain a per-port shadow of the idr value
    /// we have driven so that each successive task only flips the one pin it owns
    /// while leaving all other bits intact.  `GPIO.OUT` (0x504) is never written.
    fn queue_pin_action(&mut self, channel: usize, high: bool) {
        let cfg = self.config[channel];
        let pin = (cfg >> CONFIG_PSEL_SHIFT) & CONFIG_PSEL_MASK;
        let port_idx = if cfg & CONFIG_PORT_BIT != 0 {
            1usize
        } else {
            0usize
        };
        let port_base = if port_idx == 1 {
            GPIO1_BASE
        } else {
            GPIO0_BASE
        };
        let bit_mask = 1u32 << pin;
        // Read-modify-write against the per-port idr shadow so we drive only
        // the target pin; other pins (including those driven by other GPIOTE
        // channels) are left at their last written value.
        let prev_in = self.idr_shadow[port_idx];
        let new_in = if high {
            prev_in | bit_mask
        } else {
            prev_in & !bit_mask
        };
        self.idr_shadow[port_idx] = new_in;
        self.pending_gpio_writes
            .push((port_base + GPIO_IN_OFFSET, new_in));
        self.channel_out_level[channel] = high as u32;
    }

    fn fire_task(&mut self, channel: usize, kind: TaskKind) {
        let cfg = self.config[channel];
        let mode = cfg & CONFIG_MODE_MASK;
        if mode != CONFIG_MODE_TASK {
            // PS table 80: tasks are no-ops unless MODE = Task.
            return;
        }
        let new_level = match kind {
            TaskKind::Set => true,
            TaskKind::Clr => false,
            TaskKind::Out => {
                let polarity = (cfg >> CONFIG_POLARITY_SHIFT) & CONFIG_POLARITY_MASK;
                match polarity {
                    POLARITY_LO_TO_HI => true,
                    POLARITY_HI_TO_LO => false,
                    POLARITY_TOGGLE => self.channel_out_level[channel] == 0,
                    POLARITY_NONE => return, // no action
                    _ => return,
                }
            }
        };
        self.queue_pin_action(channel, new_level);
    }
}

#[derive(Copy, Clone)]
enum TaskKind {
    Out,
    Set,
    Clr,
}

impl Peripheral for Nrf52Gpiote {
    fn read(&self, _offset: u64) -> SimResult<u8> {
        Ok(0)
    }

    fn write(&mut self, _offset: u64, _value: u8) -> SimResult<()> {
        Ok(())
    }

    fn read_u32(&self, offset: u64) -> SimResult<u32> {
        Ok(match offset {
            OFF_TASKS_OUT_0..=OFF_TASKS_OUT_7 if offset.is_multiple_of(4) => 0,
            OFF_TASKS_SET_0..=OFF_TASKS_SET_7 if offset.is_multiple_of(4) => 0,
            OFF_TASKS_CLR_0..=OFF_TASKS_CLR_7 if offset.is_multiple_of(4) => 0,

            OFF_EVENTS_IN_0..=OFF_EVENTS_IN_7 if offset.is_multiple_of(4) => {
                self.events_in[((offset - OFF_EVENTS_IN_0) / 4) as usize]
            }
            OFF_EVENTS_PORT => self.events_port,

            OFF_INTENSET | OFF_INTENCLR => self.inten,

            OFF_CONFIG_0..=OFF_CONFIG_7 if offset.is_multiple_of(4) => {
                self.config[((offset - OFF_CONFIG_0) / 4) as usize]
            }
            _ => 0,
        })
    }

    fn write_u32(&mut self, offset: u64, value: u32) -> SimResult<()> {
        match offset {
            OFF_TASKS_OUT_0..=OFF_TASKS_OUT_7 if offset.is_multiple_of(4) && value & 1 != 0 => {
                let i = ((offset - OFF_TASKS_OUT_0) / 4) as usize;
                self.fire_task(i, TaskKind::Out);
            }
            OFF_TASKS_SET_0..=OFF_TASKS_SET_7 if offset.is_multiple_of(4) && value & 1 != 0 => {
                let i = ((offset - OFF_TASKS_SET_0) / 4) as usize;
                self.fire_task(i, TaskKind::Set);
            }
            OFF_TASKS_CLR_0..=OFF_TASKS_CLR_7 if offset.is_multiple_of(4) && value & 1 != 0 => {
                let i = ((offset - OFF_TASKS_CLR_0) / 4) as usize;
                self.fire_task(i, TaskKind::Clr);
            }

            OFF_EVENTS_IN_0..=OFF_EVENTS_IN_7 if offset.is_multiple_of(4) => {
                let i = ((offset - OFF_EVENTS_IN_0) / 4) as usize;
                self.events_in[i] = value & 1;
            }
            OFF_EVENTS_PORT => self.events_port = value & 1,

            OFF_INTENSET => self.inten |= value,
            OFF_INTENCLR => self.inten &= !value,

            OFF_CONFIG_0..=OFF_CONFIG_7 if offset.is_multiple_of(4) => {
                let i = ((offset - OFF_CONFIG_0) / 4) as usize;
                let new_cfg = value & CONFIG_WRITE_MASK;
                self.config[i] = new_cfg;
                // Seed channel level from OUTINIT so the first Toggle goes the
                // right way.
                self.channel_out_level[i] = if new_cfg & CONFIG_OUTINIT_BIT != 0 {
                    1
                } else {
                    0
                };
            }
            _ => {}
        }
        Ok(())
    }

    fn tick(&mut self) -> PeripheralTickResult {
        if self.pending_gpio_writes.is_empty()
            && self.pending_in_events.is_empty()
            && self.pending_in_mask == 0
        {
            return PeripheralTickResult::default();
        }
        let writes = std::mem::take(&mut self.pending_gpio_writes);
        let fired = std::mem::take(&mut self.pending_in_events);
        let irq = self.pending_in_mask & self.inten != 0;
        self.pending_in_mask = 0;
        PeripheralTickResult {
            irq,
            cycles: 1,
            mmio_writes: writes,
            fired_events: fired,
            ..Default::default()
        }
    }

    /// `tick()` is a genuine no-op (the early return above) unless it has a
    /// pending GPIO write, IN event, or IN-mask to deliver. Reporting exactly
    /// that condition (instead of the always-active default) drops the GPIOTE
    /// out of the per-cycle walk while idle and lets idle fast-forward engage
    /// during a tickless-idle WFI window — byte-identical, since every skipped
    /// cycle is one where `tick()` would have taken its no-op early return.
    fn legacy_tick_active(&self) -> bool {
        !self.pending_gpio_writes.is_empty()
            || !self.pending_in_events.is_empty()
            || self.pending_in_mask != 0
    }

    fn legacy_tick_dynamic(&self) -> bool {
        true
    }

    fn observe_gpio_change(&mut self, changes: &[(u8, u8, u8)]) {
        for &(port, pin, new_level) in changes {
            for ch in 0..8usize {
                let cfg = self.config[ch];
                let mode = cfg & CONFIG_MODE_MASK;
                if mode != 1 {
                    // 1 = Event; 3 = Task; 0 = Disabled.
                    continue;
                }
                let ch_pin = ((cfg >> CONFIG_PSEL_SHIFT) & CONFIG_PSEL_MASK) as u8;
                let ch_port = ((cfg >> 13) & 1) as u8;
                if ch_pin != pin || ch_port != port {
                    continue;
                }
                let polarity = (cfg >> CONFIG_POLARITY_SHIFT) & CONFIG_POLARITY_MASK;
                let prev = self.channel_in_level[ch] as u8;
                let edge_match = match polarity {
                    POLARITY_LO_TO_HI => prev == 0 && new_level == 1,
                    POLARITY_HI_TO_LO => prev == 1 && new_level == 0,
                    POLARITY_TOGGLE => prev != new_level,
                    _ => false,
                };
                self.channel_in_level[ch] = new_level as u32;
                if edge_match {
                    self.events_in[ch] = 1;
                    self.pending_in_events
                        .push(OFF_EVENTS_IN_0 as u32 + 4 * ch as u32);
                    self.pending_in_mask |= 1 << ch;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg_task(pin: u32, port: u32, polarity: u32, outinit: u32) -> u32 {
        CONFIG_MODE_TASK
            | ((pin & CONFIG_PSEL_MASK) << CONFIG_PSEL_SHIFT)
            | ((port & 1) << 13)
            | ((polarity & CONFIG_POLARITY_MASK) << CONFIG_POLARITY_SHIFT)
            | ((outinit & 1) << 20)
    }

    #[test]
    fn config0_round_trips_writable_bits() {
        let mut g = Nrf52Gpiote::new();
        g.write_u32(OFF_CONFIG_0, 0x0003_0D03).unwrap();
        assert_eq!(g.read_u32(OFF_CONFIG_0).unwrap() & 0x0007_1F03, 0x0003_0D03);
    }

    // ── silicon-faithful task-drive tests ────────────────────────────────────
    // GPIOTE tasks write to GPIO.IN (0x510 / idr), NOT GPIO.OUTSET/OUTCLR.
    // The mmio_write target is (port_base + 0x510) with the full idr word
    // (only the target pin bit set or cleared; all others zero since idr_shadow
    // starts at 0 and we do read-modify-write).

    #[test]
    fn task_set_drives_in_register_not_out() {
        let mut g = Nrf52Gpiote::new();
        // Channel 0: pin 26, port 0 — TASKS_SET should drive GPIO0.IN bit 26 high.
        g.write_u32(OFF_CONFIG_0, cfg_task(26, 0, POLARITY_NONE, 0))
            .unwrap();
        g.write_u32(OFF_TASKS_SET_0, 1).unwrap();
        let res = g.tick();
        // Target: GPIO0.IN (0x510) written with bit 26 set; OUT (0x504/0x508/0x50C) untouched.
        assert_eq!(
            res.mmio_writes,
            vec![(GPIO0_BASE + GPIO_IN_OFFSET, 1 << 26)]
        );
    }

    #[test]
    fn task_clr_drives_in_register_low_on_port1() {
        let mut g = Nrf52Gpiote::new();
        // Channel 1: pin 5, port 1 — start with bit 5 high in the shadow, then CLR.
        // First SET to put the pin high.
        g.write_u32(OFF_CONFIG_0 + 4, cfg_task(5, 1, POLARITY_NONE, 0))
            .unwrap();
        g.write_u32(OFF_TASKS_SET_0 + 4, 1).unwrap();
        let _ = g.tick(); // drains the SET write

        // Now CLR: idr_shadow[1] has bit 5 set → clearing should produce 0.
        g.write_u32(OFF_TASKS_CLR_0 + 4, 1).unwrap();
        let res = g.tick();
        assert_eq!(res.mmio_writes, vec![(GPIO1_BASE + GPIO_IN_OFFSET, 0)]);
    }

    #[test]
    fn task_out_toggle_alternates_in_register() {
        let mut g = Nrf52Gpiote::new();
        // Channel 0: pin 13, port 0, POLARITY=TOGGLE, OUTINIT=0.
        // Shadow starts at 0. Toggles: 0→1→0→1.
        g.write_u32(OFF_CONFIG_0, cfg_task(13, 0, POLARITY_TOGGLE, 0))
            .unwrap();

        g.write_u32(OFF_TASKS_OUT_0, 1).unwrap();
        let res1 = g.tick();
        assert_eq!(
            res1.mmio_writes,
            vec![(GPIO0_BASE + GPIO_IN_OFFSET, 1 << 13)]
        );

        g.write_u32(OFF_TASKS_OUT_0, 1).unwrap();
        let res2 = g.tick();
        assert_eq!(res2.mmio_writes, vec![(GPIO0_BASE + GPIO_IN_OFFSET, 0)]);

        g.write_u32(OFF_TASKS_OUT_0, 1).unwrap();
        let res3 = g.tick();
        assert_eq!(
            res3.mmio_writes,
            vec![(GPIO0_BASE + GPIO_IN_OFFSET, 1 << 13)]
        );
    }

    #[test]
    fn task_in_event_mode_is_noop() {
        let mut g = Nrf52Gpiote::new();
        // MODE = Event (not Task) → tasks should not drive pins.
        let cfg = 1 // MODE = Event
            | ((26 & CONFIG_PSEL_MASK) << CONFIG_PSEL_SHIFT)
            | ((POLARITY_LO_TO_HI & CONFIG_POLARITY_MASK) << CONFIG_POLARITY_SHIFT);
        g.write_u32(OFF_CONFIG_0, cfg).unwrap();
        g.write_u32(OFF_TASKS_OUT_0, 1).unwrap();
        let res = g.tick();
        assert!(res.mmio_writes.is_empty());
    }

    #[test]
    fn task_with_polarity_lo_to_hi_drives_in_high() {
        let mut g = Nrf52Gpiote::new();
        g.write_u32(OFF_CONFIG_0, cfg_task(7, 0, POLARITY_LO_TO_HI, 0))
            .unwrap();
        g.write_u32(OFF_TASKS_OUT_0, 1).unwrap();
        let res = g.tick();
        // POLARITY=LoToHi forces high on TASKS_OUT: GPIO0.IN bit 7 set.
        assert_eq!(res.mmio_writes, vec![(GPIO0_BASE + GPIO_IN_OFFSET, 1 << 7)]);
    }

    #[test]
    fn outinit_seeds_initial_toggle_direction() {
        let mut g = Nrf52Gpiote::new();
        // OUTINIT=1 → channel_out_level starts at 1, first Toggle goes low.
        // idr_shadow[0] starts at 0 (default) but channel_out_level is 1.
        // Toggle: current level = 1 → new level = 0.  idr_shadow[0] stays 0 after clear.
        g.write_u32(OFF_CONFIG_0, cfg_task(2, 0, POLARITY_TOGGLE, 1))
            .unwrap();
        g.write_u32(OFF_TASKS_OUT_0, 1).unwrap();
        let res = g.tick();
        // Pin 2 cleared: new_in = 0 & !4 = 0 (shadow was 0, bit 2 already 0).
        assert_eq!(res.mmio_writes, vec![(GPIO0_BASE + GPIO_IN_OFFSET, 0)]);
    }
}
