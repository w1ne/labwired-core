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
//! - **CONFIG → pad drive**: writing CONFIG[i] with MODE = Task drives the
//!   pin to OUTINIT at once, as the channel takes the pin over.
//! - **Task → pad drive**: writing TASKS_OUT/SET/CLR[i] looks up
//!   CONFIG[i].PORT/PSEL/POLARITY/OUTINIT and drives the target pin's
//!   **physical pad level**, reflected in `GPIO.IN` (offset 0x510 / `idr`).
//!   This matches silicon: when a pin is in GPIOTE Task mode the GPIOTE
//!   peripheral owns the pad; the GPIO peripheral's `OUT` register (0x504)
//!   is **not modified** by GPIOTE tasks.  The driven level is therefore
//!   observable at `GPIO.IN`, not `GPIO.OUT`. The drive latches only the
//!   target pin (the port's engine-internal per-pin latch,
//!   `gpio::NRF52_GPIO_PAD_LATCH`), so other pins keep their levels, and it
//!   reaches the PAD through the pin claim, which wins over the port's own
//!   DIR/OUT (bus wiring: `wire_nrf52_pads`).
//! - **Event observation**: EVENTS_IN is *not* driven from GPIO input
//!   changes (no input-pin model yet). Firmware that polls EVENTS_IN
//!   without PPI seeing edges will never see them fire.

use std::sync::Arc;

use crate::peripherals::nrf52::pin_select::{NrfPinClaim, NrfPinClaims};
use crate::peripherals::pad_lines::PadLines;
use crate::{Peripheral, PeripheralTickResult, SimResult};

/// One wire per channel: what a Task-mode channel drives onto the pad its
/// CONFIG names. Index = channel.
pub(crate) const GPIOTE_LINES: &[&str] = &[
    "OUT0", "OUT1", "OUT2", "OUT3", "OUT4", "OUT5", "OUT6", "OUT7",
];

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

/// Chip-YAML peripheral ids for the two GPIO ports GPIOTE can drive.
/// CONFIG[i].PORT selects the index.
const GPIO_PORT_IDS: [&str; 2] = ["gpio0", "gpio1"];

#[derive(Debug, Default)]
pub struct Nrf52Gpiote {
    events_in: [u32; 8],
    events_port: u32,
    inten: u32,
    config: [u32; 8],

    /// Per-channel current output level — needed to honor POLARITY=Toggle
    /// (which has to know whether the pin is currently high or low).
    /// Set from CONFIG[i].OUTINIT when CONFIG is written (a Task-mode write
    /// also drives the pin to it); updated on every TASKS_OUT/SET/CLR.
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

    /// Scheduler delay-0 drain chain armed.
    chain_live: bool,

    /// Per-pin latch address of each GPIO port, resolved from the descriptor at build
    /// time — index 0 = `gpio0`, index 1 = `gpio1`. `None` means the chip does
    /// not declare that port (nRF52832 has no P1), in which case a task
    /// targeting it drives nothing and says so, rather than writing a guessed
    /// address into whatever peripheral happens to own that window.
    port_latches: [Option<u32>; 2],

    /// The pad-level wires, one per channel, published into the GPIO ports'
    /// routing (bus wiring, [`Self::pad_lines_arc`]). A Task-mode channel OWNS
    /// its pin on silicon whatever the port's DIR/OUT/PIN_CNF say — CODAL's
    /// micro:bit V2 display leaves a column pin configured as a GPIO output
    /// after its light-sense strobe and hands it back to GPIOTE — so the level
    /// must reach the pad through the claim, not only through the IN latch
    /// (which the port ignores for a DIR=1 pin).
    lines: Option<Arc<PadLines>>,
    /// Each channel's claim on the pad its CONFIG names, live while the channel
    /// is in Task mode.
    claims: [NrfPinClaim; 8],
}

impl Nrf52Gpiote {
    /// Build the model against the chip's declared memory map.
    ///
    /// The GPIO port bases are looked up by chip-YAML id, so the model cannot
    /// disagree with the descriptor it was built from. There is deliberately no
    /// `new()` without a map: the previous hardcoded `GPIO1_BASE` produced a
    /// valid-but-wrong address that the bus swallowed silently, and a default
    /// constructor is exactly how that constant would grow back.
    pub fn new(map: crate::peripherals::chip_map::ChipMap<'_>) -> Self {
        let mut port_latches = [None; 2];
        for (idx, id) in GPIO_PORT_IDS.iter().enumerate() {
            port_latches[idx] = map
                .register_address(id, crate::peripherals::gpio::NRF52_GPIO_PAD_LATCH)
                .and_then(|b| u32::try_from(b).ok());
            if port_latches[idx].is_none() {
                tracing::debug!(
                    "nRF52 GPIOTE: chip declares no '{id}'; \
                     tasks targeting PORT={idx} will drive nothing"
                );
            }
        }
        Self {
            port_latches,
            ..Self::default()
        }
    }

    /// This model's per-channel pad wires. Created at bus wiring time.
    pub(crate) fn pad_lines_arc(&mut self) -> Arc<PadLines> {
        self.lines
            .get_or_insert_with(|| Arc::new(PadLines::new(GPIOTE_LINES, &[false; 8])))
            .clone()
    }

    /// Join the chip's pin-claim table: channel `i` claims under
    /// `first_token + i`. Config-build time only.
    pub(crate) fn install_pin_claims(&mut self, claims: &Arc<NrfPinClaims>, first_token: u32) {
        for (i, claim) in self.claims.iter_mut().enumerate() {
            claim.install(claims.clone(), first_token + i as u32);
        }
        for i in 0..8 {
            self.sync_claim(i);
        }
    }

    /// Republish channel `i`'s claim from its CONFIG: the pad CONFIG.PSEL /
    /// CONFIG.PORT names, held while MODE = Task. CONFIG's pin fields are laid
    /// out differently from a peripheral `PSEL` register (PIN [12:8], PORT
    /// [13] here; PIN [4:0], PORT [5] there), so they are re-packed.
    fn sync_claim(&mut self, i: usize) {
        let cfg = self.config[i];
        let pin = (cfg >> CONFIG_PSEL_SHIFT) & CONFIG_PSEL_MASK;
        let port = u32::from(cfg & CONFIG_PORT_BIT != 0);
        let live = cfg & CONFIG_MODE_MASK == CONFIG_MODE_TASK;
        self.claims[i].update(pin | (port << 5), live);
    }

    /// Drive the target pin's pad level (reflected in `GPIO.IN` at offset 0x510).
    ///
    /// Silicon behaviour: when a pin is in GPIOTE Task mode the GPIOTE peripheral
    /// drives the physical pad; the GPIO `OUT` register (0x504) is **not touched**.
    ///
    /// Publishes the level on the channel's pad wire (read through the pin
    /// claim) and queues a per-pin IN latch for the one pin it owns.
    /// `GPIO.OUT` (0x504) is never written.
    fn queue_pin_action(&mut self, channel: usize, high: bool) {
        if let Some(lines) = &self.lines {
            lines.set_line(channel, high);
        }
        let cfg = self.config[channel];
        let pin = (cfg >> CONFIG_PSEL_SHIFT) & CONFIG_PSEL_MASK;
        let port_idx = if cfg & CONFIG_PORT_BIT != 0 {
            1usize
        } else {
            0usize
        };
        // The port base comes from the chip descriptor. If the chip does not
        // declare this port there is no correct address to write, so drop the
        // drive loudly instead of picking one — a wrong address is still a
        // valid address, and the bus would absorb it without complaint.
        let Some(port_latch) = self.port_latches[port_idx] else {
            tracing::warn!(
                "nRF52 GPIOTE ch{channel}: CONFIG selects PORT={port_idx} ('{}'), \
                 which this chip does not declare; pin {pin} drive dropped",
                GPIO_PORT_IDS[port_idx]
            );
            self.channel_out_level[channel] = high as u32;
            return;
        };
        // Latch only THIS pin's IN bit (engine-internal per-pin op; see
        // `gpio::NRF52_GPIO_PAD_LATCH`). A whole-word IN write from a shadow
        // reset every other pin's latched external level on the port.
        self.pending_gpio_writes
            .push((port_latch, pin | (u32::from(high) << 8)));
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

    fn has_pending(&self) -> bool {
        !self.pending_gpio_writes.is_empty()
            || !self.pending_in_events.is_empty()
            || self.pending_in_mask != 0
    }

    fn drain_pending(&mut self) -> PeripheralTickResult {
        if !self.has_pending() {
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

    fn as_any(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }

    fn as_any_mut(&mut self) -> Option<&mut dyn std::any::Any> {
        Some(self)
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
            _ => {
                crate::census_reg!("nrf52.gpiote:Nrf52Gpiote", offset, "read");
                0
            }
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
                self.sync_claim(i);
                let outinit = new_cfg & CONFIG_OUTINIT_BIT != 0;
                if new_cfg & CONFIG_MODE_MASK == CONFIG_MODE_TASK {
                    // Task mode (PS §6.9, CONFIG.OUTINIT): the channel owns
                    // the pin as an output from the moment it is configured,
                    // at the OUTINIT level. Writing CONFIG therefore IS a
                    // drive — CODAL's micro:bit V2 LED matrix sets each
                    // column's level for a row this way, and toggles it later
                    // over PPI.
                    self.queue_pin_action(i, outinit);
                } else {
                    // Seed channel level from OUTINIT so the first Toggle goes
                    // the right way once the channel is put in Task mode.
                    self.channel_out_level[i] = outinit as u32;
                }
            }
            _ => {
                crate::census_reg!("nrf52.gpiote:Nrf52Gpiote", offset, "write");
            }
        }
        Ok(())
    }

    fn tick(&mut self) -> PeripheralTickResult {
        self.drain_pending()
    }

    /// `tick()` is a genuine no-op unless it has a pending GPIO write, IN
    /// event, or IN-mask to deliver.
    fn legacy_tick_active(&self) -> bool {
        self.has_pending()
    }

    fn legacy_tick_dynamic(&self) -> bool {
        true
    }

    fn uses_scheduler(&self) -> bool {
        // Pending OUT/IN delivery is a write/edge latch drained by delay-0
        // events (and still by the legacy walk when the feature is off).
        true
    }

    fn needs_legacy_walk(&self) -> bool {
        false
    }

    fn take_scheduled_events(&mut self) -> Vec<(u64, u32)> {
        if self.has_pending() && !self.chain_live {
            self.chain_live = true;
            vec![(0, 0)]
        } else {
            Vec::new()
        }
    }

    fn on_event(
        &mut self,
        _event_token: u32,
        _sched: &mut crate::sched::EventScheduler,
        _bus: &mut dyn crate::Bus,
    ) -> crate::sched::EventResult {
        let res = self.drain_pending();
        self.chain_live = self.has_pending();
        crate::sched::EventResult {
            raise_own_irq: res.irq,
            mmio_writes: res.mmio_writes,
            fired_events: res.fired_events,
            reschedule_delay: self.chain_live.then_some(1),
            ..Default::default()
        }
    }

    /// This model consumes GPIO edges, so the bus must keep its per-cycle
    /// edge-detection pass alive even on a walk-free fast path. See
    /// [`crate::Peripheral::observes_gpio_edges`].
    fn observes_gpio_edges(&self) -> bool {
        true
    }

    fn observe_gpio_change(&mut self, changes: &[(u8, u8, u8)]) -> bool {
        let latched_before = self.pending_in_events.len();
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
        // Only a channel that actually matched an edge needs the bus to harvest
        // a wake for it; an edge on a pin no channel watches is not work.
        self.pending_in_events.len() != latched_before
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::peripherals::chip_map::ChipMap;
    use labwired_config::PeripheralConfig;

    // Test fixture memory map. These are the addresses the nRF52840 chip YAML
    // declares (note gpio1 = 0x5000_1000, the remap — NOT Nordic's raw-silicon
    // P1 base 0x5000_0300, which sits inside gpio0's 4 KB window). They live
    // here as test *inputs*, so the model has no base address of its own to be
    // wrong about.
    const T_GPIO0_BASE: u32 = 0x5000_0000;
    const T_GPIO1_BASE: u32 = 0x5000_1000;

    fn port_cfg(id: &str, base: u64) -> PeripheralConfig {
        PeripheralConfig {
            id: id.to_string(),
            r#type: "gpio".to_string(),
            base_address: base,
            size: Some("4KB".to_string()),
            irq: None,
            irq_controller: None,
            clock: None,
            config: Default::default(),
        }
    }

    /// A GPIOTE built against a two-port map, as `from_config` would build it.
    fn gpiote() -> Nrf52Gpiote {
        let entries = vec![
            port_cfg("gpio0", T_GPIO0_BASE as u64),
            port_cfg("gpio1", T_GPIO1_BASE as u64),
        ];
        Nrf52Gpiote::new(ChipMap::new(&entries))
    }

    fn cfg_task(pin: u32, port: u32, polarity: u32, outinit: u32) -> u32 {
        CONFIG_MODE_TASK
            | ((pin & CONFIG_PSEL_MASK) << CONFIG_PSEL_SHIFT)
            | ((port & 1) << 13)
            | ((polarity & CONFIG_POLARITY_MASK) << CONFIG_POLARITY_SHIFT)
            | ((outinit & 1) << 20)
    }

    #[test]
    fn config0_round_trips_writable_bits() {
        let mut g = gpiote();
        g.write_u32(OFF_CONFIG_0, 0x0003_0D03).unwrap();
        assert_eq!(g.read_u32(OFF_CONFIG_0).unwrap() & 0x0007_1F03, 0x0003_0D03);
    }

    // ── silicon-faithful task-drive tests ────────────────────────────────────
    // GPIOTE tasks latch the pad's IN bit, NOT GPIO.OUTSET/OUTCLR. The
    // mmio_write target is the port's per-pin latch (port_base +
    // NRF52_GPIO_PAD_LATCH) carrying PIN | LEVEL << 8, so no other pin's
    // level is touched (the old whole-word IN write from a zeroed shadow
    // reset the others).

    #[test]
    fn task_set_drives_in_register_not_out() {
        let mut g = gpiote();
        // Channel 0: pin 26, port 0 — TASKS_SET should drive GPIO0.IN bit 26 high.
        g.write_u32(OFF_CONFIG_0, cfg_task(26, 0, POLARITY_NONE, 0))
            .unwrap();
        let _ = g.tick(); // CONFIG in Task mode drives OUTINIT
        g.write_u32(OFF_TASKS_SET_0, 1).unwrap();
        let res = g.tick();
        // Target: GPIO0 pin 26 latched high; OUT (0x504/0x508/0x50C) untouched.
        assert_eq!(
            res.mmio_writes,
            vec![(
                T_GPIO0_BASE + crate::peripherals::gpio::NRF52_GPIO_PAD_LATCH as u32,
                26 | 1 << 8
            )]
        );
    }

    #[test]
    fn task_clr_drives_in_register_low_on_port1() {
        let mut g = gpiote();
        // Channel 1: pin 5, port 1 — start with bit 5 high in the shadow, then CLR.
        // First SET to put the pin high.
        g.write_u32(OFF_CONFIG_0 + 4, cfg_task(5, 1, POLARITY_NONE, 0))
            .unwrap();
        let _ = g.tick(); // CONFIG in Task mode drives OUTINIT
        g.write_u32(OFF_TASKS_SET_0 + 4, 1).unwrap();
        let _ = g.tick(); // drains the SET write

        // Now CLR: pin 5 of port 1 latched low.
        g.write_u32(OFF_TASKS_CLR_0 + 4, 1).unwrap();
        let res = g.tick();
        // The write must target the port-1 base the nRF52840 chip YAML declares
        // (0x5000_1000), NOT the raw-silicon P1 base (0x5000_0300) — the latter
        // lands inside gpio0's 4 KB window and is silently swallowed.
        assert_eq!(
            res.mmio_writes,
            vec![(
                T_GPIO1_BASE + crate::peripherals::gpio::NRF52_GPIO_PAD_LATCH as u32,
                5
            )]
        );
    }

    #[test]
    fn task_out_toggle_alternates_in_register() {
        let mut g = gpiote();
        // Channel 0: pin 13, port 0, POLARITY=TOGGLE, OUTINIT=0.
        // Shadow starts at 0. Toggles: 0→1→0→1.
        g.write_u32(OFF_CONFIG_0, cfg_task(13, 0, POLARITY_TOGGLE, 0))
            .unwrap();
        let _ = g.tick(); // CONFIG in Task mode drives OUTINIT

        g.write_u32(OFF_TASKS_OUT_0, 1).unwrap();
        let res1 = g.tick();
        assert_eq!(
            res1.mmio_writes,
            vec![(
                T_GPIO0_BASE + crate::peripherals::gpio::NRF52_GPIO_PAD_LATCH as u32,
                13 | 1 << 8
            )]
        );

        g.write_u32(OFF_TASKS_OUT_0, 1).unwrap();
        let res2 = g.tick();
        assert_eq!(
            res2.mmio_writes,
            vec![(
                T_GPIO0_BASE + crate::peripherals::gpio::NRF52_GPIO_PAD_LATCH as u32,
                13
            )]
        );

        g.write_u32(OFF_TASKS_OUT_0, 1).unwrap();
        let res3 = g.tick();
        assert_eq!(
            res3.mmio_writes,
            vec![(
                T_GPIO0_BASE + crate::peripherals::gpio::NRF52_GPIO_PAD_LATCH as u32,
                13 | 1 << 8
            )]
        );
    }

    #[test]
    fn task_in_event_mode_is_noop() {
        let mut g = gpiote();
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
        let mut g = gpiote();
        g.write_u32(OFF_CONFIG_0, cfg_task(7, 0, POLARITY_LO_TO_HI, 0))
            .unwrap();
        let _ = g.tick(); // CONFIG in Task mode drives OUTINIT
        g.write_u32(OFF_TASKS_OUT_0, 1).unwrap();
        let res = g.tick();
        // POLARITY=LoToHi forces high on TASKS_OUT: GPIO0.IN bit 7 set.
        assert_eq!(
            res.mmio_writes,
            vec![(
                T_GPIO0_BASE + crate::peripherals::gpio::NRF52_GPIO_PAD_LATCH as u32,
                7 | 1 << 8
            )]
        );
    }

    #[test]
    fn outinit_seeds_initial_toggle_direction() {
        let mut g = gpiote();
        // OUTINIT=1 → channel_out_level starts at 1, first Toggle goes low.
        // CONFIG with OUTINIT=1 drives the pin high (drained above), so the
        // first Toggle goes low.
        g.write_u32(OFF_CONFIG_0, cfg_task(2, 0, POLARITY_TOGGLE, 1))
            .unwrap();
        let _ = g.tick(); // CONFIG in Task mode drives OUTINIT
        g.write_u32(OFF_TASKS_OUT_0, 1).unwrap();
        let res = g.tick();
        // Pin 2 cleared: new_in = 0 & !4 = 0 (shadow was 0, bit 2 already 0).
        assert_eq!(
            res.mmio_writes,
            vec![(
                T_GPIO0_BASE + crate::peripherals::gpio::NRF52_GPIO_PAD_LATCH as u32,
                2
            )]
        );
    }

    #[test]
    fn config_in_task_mode_drives_outinit() {
        let mut g = gpiote();
        // Task mode, OUTINIT=1: the CONFIG write alone drives the pin high.
        g.write_u32(OFF_CONFIG_0, cfg_task(28, 0, POLARITY_TOGGLE, 1))
            .unwrap();
        let res = g.tick();
        assert_eq!(
            res.mmio_writes,
            vec![(
                T_GPIO0_BASE + crate::peripherals::gpio::NRF52_GPIO_PAD_LATCH as u32,
                28 | 1 << 8
            )]
        );
        // Re-configured with OUTINIT=0 (CODAL does this per matrix row): low.
        g.write_u32(OFF_CONFIG_0, cfg_task(28, 0, POLARITY_TOGGLE, 0))
            .unwrap();
        let res = g.tick();
        assert_eq!(
            res.mmio_writes,
            vec![(
                T_GPIO0_BASE + crate::peripherals::gpio::NRF52_GPIO_PAD_LATCH as u32,
                28
            )]
        );
        // Not in Task mode: a CONFIG write drives nothing.
        g.write_u32(OFF_CONFIG_0, 0).unwrap();
        assert!(g.tick().mmio_writes.is_empty());
    }
}
