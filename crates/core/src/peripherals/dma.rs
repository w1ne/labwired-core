// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

use crate::{CycleClock, DmaDirection, DmaRequest, Peripheral, PeripheralTickResult, SimResult};
use std::any::Any;

#[derive(Debug, Default, serde::Serialize)]
struct DmaChannel {
    ccr: u32,
    cndtr: u32,
    cpar: u32,
    cmar: u32,
    active: bool,
    /// Internal pointers used during transfer. Real STM32 silicon does
    /// NOT modify the user-facing CPAR / CMAR registers as a transfer
    /// runs — it uses internal next-address registers and leaves the
    /// configured base addresses readable for the firmware. Modelling
    /// the increment as a separate field preserves that contract.
    cpar_ptr: u32,
    cmar_ptr: u32,
    /// Initial CNDTR value. Used to fire HTIF when the transfer crosses
    /// half-way (CNDTR == cndtr_initial / 2).
    cndtr_initial: u32,
    /// Scheduler mode only: `true` while this channel has a transfer event
    /// live in the scheduler heap. Prevents a second write (or DMA request)
    /// from arming a duplicate chain — the one live event self-perpetuates
    /// (mem2mem) or fires once (peripheral-paced) reading live register
    /// state, so at most one event per channel is ever outstanding.
    #[serde(skip)]
    chain_live: bool,
}

/// STM32 DMA1 controller — 7 channels, F1/F4/L4 compatible.
///
/// L4 adds the CSELR register at offset 0xA8: a 32-bit field with 4 bits
/// per channel selecting which peripheral's request line drives the DMA
/// channel (RM0351 §11.6.7). Older F1/F4 chips ignore writes to that
/// offset; sim accepts the write and reads back unchanged on both.
///
/// ## Drive modes (walk-free plan Part 2, batch B4)
///
/// Two mutually exclusive time sources, selected by ONE predicate
/// (`scheduler_mode`), following the SysTick (B1) / TIMx (B2/B3) exemplars:
///
/// * **Scheduler mode** (`event-scheduler` feature + a [`CycleClock`] attached
///   at bus registration): `uses_scheduler()` is true, the per-cycle walk skips
///   this peripheral entirely, and each channel's element transfers ride
///   **scheduled events** instead of the walk:
///   - enabling a channel (CCR.EN 0→1) or a routed peripheral request
///     ([`Peripheral::dma_request`]) makes the channel `active`; the write /
///     request choke harvests a **delay-0** event via `take_scheduled_events`
///     (bus converts to `current_cycle + 1` — the exact cycle the legacy walk's
///     *next* tick would have serviced it);
///   - `on_event` transfers ONE element (emitting the SAME [`DmaRequest`] the
///     legacy `tick()` emits, decrementing CNDTR, advancing the internal
///     PINC/MINC pointers, latching HTIF/TCIF/GIF and pending the channel's
///     NVIC line on HTIE/TCIE through the same `pend_irq_for_event` choke), and
///     a **mem2mem** channel self-perpetuates at delay 1 while `CNDTR > 0`
///     (one element per cycle — byte-identical to the walk's per-tick pacing).
///     A peripheral-paced channel goes inert after its one element and re-arms
///     on the next `dma_request`, exactly like the walk.
///
/// * **Legacy mode** (feature off, or no clock attached — hand-built test
///   buses that bypass the bus registration chokes): the per-cycle walk drives
///   `tick()`, byte-identical to the historical model.
///
/// ### Why no lazy `advance_to`
///
/// Unlike a free-running timer counter, every readable DMA register (ISR flags,
/// CNDTR) mutates ONLY at a transfer event. At tick interval 1 the event fires
/// on the exact cycle the walk tick would have, so a firmware read observes the
/// same register state without any closed-form replay. (At interval N > 1 reads
/// are quantised to the batch grid — the same ≤ one-interval bound the write
/// path `sync_to` documents, and strictly better than the legacy walk at
/// interval N, which paces the whole transfer N× slower.)
///
/// ### Preserved semantics (differentially pinned — including the model's
/// known quirks, kept so the two drive modes are byte-identical)
///
/// - **Sticky `active`**: CCR.EN 0→1 sets `active` and snapshots the pointers /
///   initial CNDTR; CCR.EN 1→0 does NOT clear `active`, so a mem2mem transfer
///   in flight runs to completion even if firmware clears EN mid-way. The event
///   chain reads live `active`/`cndtr`, reproducing this exactly.
/// - **One element per active tick**: a non-mem2mem channel transfers a single
///   element then clears `active` (peripheral-request paced); a mem2mem channel
///   stays `active` and drains one element per tick.
/// - **Byte-granular copy with width-strided pointers**: each element moves one
///   byte through [`DmaRequest`] while PINC/MINC advance CPAR/CMAR internal
///   pointers by the PSIZE/MSIZE width — the historical model detail, preserved.
/// - **HTIF at the half-way crossing, TCIF at completion, GIF tracks the OR** —
///   set on the exact element the walk sets them.
///
/// ### Tick-cost normalization (B4)
///
/// The legacy model charged `cycles: 1` on any tick that emitted a transfer
/// request, inflating `total_cycles` — a sim artifact (a real DMA controller is
/// a bus master, it does not steal CPU cycles) structurally incompatible with
/// deleting the walk. Both modes now charge zero, so the walk-on reference and
/// the scheduler path agree cycle-for-cycle (the same normalization B1/B2/B3
/// applied to SysTick / TIMx).
#[derive(Debug, Default, serde::Serialize)]
pub struct Dma1 {
    isr: u32,
    ifcr: u32,
    /// L4 channel-selection register. Each nibble (4 bits) selects which
    /// peripheral request line drives the corresponding channel:
    ///   bits 3:0   - C1S  (channel 1)
    ///   bits 7:4   - C2S
    ///   ...
    ///   bits 27:24 - C7S
    cselr: u32,
    channels: [DmaChannel; 7],

    /// Bus-published cycle clock (walk-free plan Part 1). `Some` once the bus
    /// registration choke attaches it; `None` keeps the model on the legacy
    /// walk path.
    #[serde(skip)]
    clock: Option<CycleClock>,
}

impl Dma1 {
    pub fn new() -> Self {
        Self::default()
    }

    /// True when the event scheduler owns this controller's pacing (feature on
    /// AND bus clock attached). Everything drive-mode-related branches on this
    /// ONE predicate so the two modes can never mix.
    #[inline]
    fn scheduler_mode(&self) -> bool {
        cfg!(feature = "event-scheduler") && self.clock.is_some()
    }

    fn tick_channels_once(&mut self) -> PeripheralTickResult {
        let mut dma_requests: Option<Vec<DmaRequest>> = None;
        let mut irq = false;

        for i in 0..7 {
            let (req, chan_irq) = self.service_channel_once(i);
            if let Some(r) = req {
                dma_requests.get_or_insert_with(Vec::new).push(r);
            }
            irq |= chan_irq;
        }

        PeripheralTickResult {
            irq,
            // B4 tick-cost normalization: charge zero in every state so the
            // walk-on reference and scheduler path agree cycle-for-cycle.
            cycles: 0,
            dma_requests,
            ..Default::default()
        }
    }

    /// Test/differential knob: detach the cycle clock, pinning the model to the
    /// legacy walk path (`uses_scheduler() == false`). Used by the walk-on-vs-
    /// scheduler differential gates to build the reference config from the same
    /// bus assembly.
    pub fn force_legacy_walk(&mut self) {
        self.clock = None;
    }

    fn read_reg(&self, offset: u64) -> u32 {
        match offset {
            0x00 => self.isr,
            // IFCR is write-1-to-clear; it reads as 0. Handling it explicitly
            // also avoids the `offset - 0x08` underflow below (a byte write to
            // IFCR does a read-modify-write, which would otherwise panic).
            0x04 => 0,
            0xA8 => self.cselr, // L4 channel-selection register
            // Channel register block starts at 0x08; guard against any stray
            // sub-0x08 access so the subtraction can never underflow.
            _ if offset >= 0x08 => {
                let chan_idx = ((offset - 0x08) / 20) as usize;
                let reg_off = (offset - 0x08) % 20;
                if chan_idx < 7 {
                    match reg_off {
                        0x00 => self.channels[chan_idx].ccr,
                        0x04 => self.channels[chan_idx].cndtr,
                        0x08 => self.channels[chan_idx].cpar,
                        0x0C => self.channels[chan_idx].cmar,
                        _ => 0,
                    }
                } else {
                    0
                }
            }
            _ => 0, // sub-0x08 offsets other than ISR/IFCR read as 0
        }
    }

    fn write_reg(&mut self, offset: u64, value: u32) {
        match offset {
            0x00 => {} // ISR is read-only; ignore writes (and avoid underflow)
            0x04 => {
                // IFCR: Write 1 to clear corresponding ISR bits
                self.isr &= !value;
            }
            0xA8 => {
                // CSELR: only the low 28 bits are valid (4 bits × 7 channels).
                self.cselr = value & 0x0FFF_FFFF;
            }
            _ if offset >= 0x08 => {
                let chan_idx = ((offset - 0x08) / 20) as usize;
                let reg_off = (offset - 0x08) % 20;
                if chan_idx < 7 {
                    match reg_off {
                        0x00 => {
                            let old_en = (self.channels[chan_idx].ccr & 1) != 0;
                            // CCR writable bits are [14:0]; [31:15] reserved read
                            // 0 (silicon-confirmed on F103). The reserved PSIZE/
                            // MSIZE=0b11 encoding clamps on silicon — a value
                            // detail beyond this flat mask.
                            self.channels[chan_idx].ccr = value & 0x0000_7FFF;
                            let new_en = (value & 1) != 0;
                            if !old_en && new_en {
                                let chan = &mut self.channels[chan_idx];
                                chan.active = true;
                                chan.cpar_ptr = chan.cpar;
                                chan.cmar_ptr = chan.cmar;
                                chan.cndtr_initial = chan.cndtr;
                            }
                        }
                        0x04 => self.channels[chan_idx].cndtr = value & 0xFFFF,
                        0x08 => self.channels[chan_idx].cpar = value,
                        0x0C => self.channels[chan_idx].cmar = value,
                        _ => {}
                    }
                }
            }
            _ => {} // sub-0x08 offsets other than ISR/IFCR: ignore
        }
    }

    /// Transfer a single element on channel `i` if it is `active` with
    /// `CNDTR > 0`, returning the emitted [`DmaRequest`] (if any) and whether
    /// the channel's completion/half events pend the NVIC line this element.
    ///
    /// This is the ONE transfer body both drive modes call: the legacy `tick()`
    /// loops it over all 7 channels; the scheduler `on_event` calls it for the
    /// fired channel. Sharing the body makes the two modes identical by
    /// construction. Logic relocated verbatim from the pre-migration `tick()`.
    fn service_channel_once(&mut self, i: usize) -> (Option<DmaRequest>, bool) {
        let mut irq = false;
        let chan = &mut self.channels[i];
        if !(chan.active && chan.cndtr > 0) {
            return (None, false);
        }

        let dir_bit = (chan.ccr >> 4) & 1;
        let mem2mem = (chan.ccr >> 14) & 1;

        // Use internal pointers for the actual transfer; leave the
        // user-facing CPAR / CMAR registers untouched so firmware reads
        // them at the configured base, matching real STM32 hardware.
        let (src, dst, direction) = if mem2mem == 1 {
            // STM32 mem-to-mem mode (RM0351 §11.4.7): MEM2MEM=1 requires
            // DIR=1, and the data flows CMAR -> CPAR. CMAR is "memory side"
            // (source), CPAR is "peripheral side" (destination).
            (chan.cmar_ptr, chan.cpar_ptr, DmaDirection::Copy)
        } else if dir_bit == 1 {
            // Memory -> peripheral: read from CMAR, write to CPAR.
            (chan.cmar_ptr, chan.cpar_ptr, DmaDirection::Write)
        } else {
            // Peripheral -> memory: read from CPAR, write to CMAR.
            (chan.cpar_ptr, chan.cmar_ptr, DmaDirection::Read)
        };

        let request = DmaRequest {
            src_addr: src as u64,
            addr: dst as u64,
            val: 0,
            direction,
            transform: None,
        };

        chan.cndtr -= 1;
        // Increment internal memory/peripheral pointers if MINC/PINC is set.
        // The CCR PSIZE/MSIZE bits select 1/2/4 byte width; we treat each
        // tick as one element so the increment matches.
        if (chan.ccr & (1 << 7)) != 0 {
            chan.cmar_ptr = chan.cmar_ptr.wrapping_add(if (chan.ccr & (1 << 10)) != 0 {
                4
            } else if (chan.ccr & (1 << 8)) != 0 {
                2
            } else {
                1
            });
        }
        if (chan.ccr & (1 << 6)) != 0 {
            chan.cpar_ptr = chan.cpar_ptr.wrapping_add(if (chan.ccr & (1 << 11)) != 0 {
                4
            } else if (chan.ccr & (1 << 8)) != 0 {
                2
            } else {
                1
            });
        }

        let cndtr_after = chan.cndtr;
        let cndtr_initial = chan.cndtr_initial;
        let htie = (chan.ccr & (1 << 2)) != 0;
        let tcie = (chan.ccr & (1 << 1)) != 0;

        // HTIF: set when transfer crosses the halfway mark. Matches what
        // real silicon does for any non-trivial CNDTR.
        if cndtr_initial >= 2
            && cndtr_after <= cndtr_initial / 2
            && (self.isr & (1 << (i * 4 + 2))) == 0
        {
            self.isr |= 1 << (i * 4 + 2); // HTIF_x
            self.isr |= 1 << (i * 4); // GIF_x
            if htie {
                irq = true;
            }
        }

        if cndtr_after == 0 {
            self.channels[i].active = false;
            self.isr |= 1 << (i * 4 + 1); // TCIF_x
            self.isr |= 1 << (i * 4); // GIF_x — global IF tracks the
                                      // logical-OR of TCIF/HTIF/TEIF.
            if tcie {
                irq = true;
            }
        } else if mem2mem == 0 {
            self.channels[i].active = false;
        }

        (Some(request), irq)
    }
}

impl Peripheral for Dma1 {
    fn read(&self, offset: u64) -> SimResult<u8> {
        let reg_offset = offset & !3;
        let byte_offset = (offset % 4) as u32;
        let reg_val = self.read_reg(reg_offset);
        Ok(((reg_val >> (byte_offset * 8)) & 0xFF) as u8)
    }

    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        let reg_offset = offset & !3;
        let byte_offset = (offset % 4) as u32;

        let mut reg_val = self.read_reg(reg_offset);
        let mask = 0xFF << (byte_offset * 8);
        reg_val &= !mask;
        reg_val |= (value as u32) << (byte_offset * 8);

        self.write_reg(reg_offset, reg_val);
        Ok(())
    }

    fn dma_request(&mut self, request_id: u32) {
        // request_id usually corresponds to the channel (1-7) or a mapping
        let chan_idx = (request_id.saturating_sub(1)) as usize;
        if chan_idx < 7 {
            let chan = &mut self.channels[chan_idx];
            if (chan.ccr & 1) != 0 {
                // Channel is enabled, mark as active for the next tick. In
                // scheduler mode the bus follows this with
                // `collect_scheduled_events`, which arms the element event.
                chan.active = true;
            }
        }
    }

    fn tick(&mut self) -> PeripheralTickResult {
        // Never runs in scheduler mode (the walk skips `uses_scheduler()`
        // peripherals; the guard keeps a stray direct call from double-driving
        // the event-paced channels).
        if self.scheduler_mode() {
            return PeripheralTickResult::default();
        }

        self.tick_channels_once()
    }

    fn tick_elapsed_forced(&mut self, _cycles: u64) -> PeripheralTickResult {
        // Hardware-oracle settle mode freezes the CPU and intentionally asks
        // for the pre-scheduler one-element transition. This never runs from
        // production Machine execution, where the event chain remains the
        // sole owner of scheduler-mode DMA pacing.
        self.tick_channels_once()
    }

    fn uses_scheduler(&self) -> bool {
        // True once the bus attached its cycle clock (event-scheduler builds):
        // channel transfers ride scheduled events and the walk is unnecessary.
        // Without a clock (feature off / hand-built buses) stay on the legacy
        // walk with exact historical semantics.
        self.scheduler_mode()
    }

    fn needs_legacy_walk(&self) -> bool {
        // Everything this model's `tick()` does (per-channel element transfer,
        // HTIF/TCIF/GIF latching, held NVIC pend) is event-expressible: a
        // transfer is armed by a write (CCR.EN) or a routed `dma_request`, and
        // mem2mem self-pacing is a delay-1 event chain. No configuration needs
        // a dynamic walk fallback. In legacy mode (no clock / feature off) the
        // walk does real work and the conservative `true` stands.
        !self.scheduler_mode()
    }

    fn sync_to(&mut self, _now_cycle: u64) {
        // No free-running state to advance: every readable register mutates
        // only at a transfer event, so there is nothing to replay on the write
        // path. (Kept as an explicit no-op override for symmetry with the other
        // scheduler-migrated models.)
    }

    fn take_scheduled_events(&mut self) -> Vec<(u64, u32)> {
        if !self.scheduler_mode() {
            return Vec::new();
        }
        // Arm a delay-0 element event (bus → deadline `current_cycle + 1`, the
        // cycle the legacy walk's next tick would have serviced this channel)
        // for every channel that just became active and has no live chain. The
        // per-channel `chain_live` guard makes duplicate arming impossible: a
        // later write / request while the chain is in flight returns nothing
        // and the one live event keeps reading live register state.
        let mut events = Vec::new();
        for i in 0..7 {
            let chan = &mut self.channels[i];
            if chan.active && chan.cndtr > 0 && !chan.chain_live {
                chan.chain_live = true;
                events.push((0u64, i as u32));
            }
        }
        events
    }

    fn on_event(
        &mut self,
        event_token: u32,
        sched: &mut crate::sched::EventScheduler,
        _bus: &mut dyn crate::Bus,
    ) -> crate::sched::EventResult {
        let _ = sched;
        let i = event_token as usize;
        if !self.scheduler_mode() || i >= 7 {
            return crate::sched::EventResult::default();
        }
        let (req, irq) = self.service_channel_once(i);
        // A mem2mem channel stays `active` with `CNDTR > 0` → perpetuate at
        // delay 1 (one element per cycle, exactly the walk's pacing). Anything
        // else (completion, peripheral-paced single element, a channel drained
        // to zero by a firmware CNDTR write) ends the chain; the channel
        // re-arms on the next enable / `dma_request`.
        let chan = &mut self.channels[i];
        let reschedule = chan.active && chan.cndtr > 0;
        chan.chain_live = reschedule;
        crate::sched::EventResult {
            raise_own_irq: irq,
            reschedule_delay: reschedule.then_some(1),
            dma_requests: req.into_iter().collect(),
            ..Default::default()
        }
    }

    fn attach_cycle_clock(&mut self, clock: CycleClock) {
        self.clock = Some(clock);
    }

    fn as_any(&self) -> Option<&dyn Any> {
        Some(self)
    }
    fn as_any_mut(&mut self) -> Option<&mut dyn Any> {
        Some(self)
    }

    fn snapshot(&self) -> serde_json::Value {
        serde_json::to_value(self).unwrap_or(serde_json::Value::Null)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cselr_round_trips_l4_channel_selection() {
        let mut dma = Dma1::new();
        // Map ch1 -> request 4, ch7 -> request 5 (typical USART2_TX / SPI1_RX patterns).
        dma.write_reg(0xA8, 4 | (5 << 24));
        let v = dma.read_reg(0xA8);
        assert_eq!(v & 0xF, 4);
        assert_eq!((v >> 24) & 0xF, 5);
    }

    #[test]
    fn test_dma_channel_completes_and_sets_irq_on_tcie() {
        let mut dma = Dma1::new();
        // CH1: CCR=EN|TCIE|DIR|MINC|PINC, one byte transfer.
        dma.write_reg(0x10, 0x2000_0010); // CH1 CPAR
        dma.write_reg(0x0C, 1); // CH1 CNDTR
        dma.write_reg(0x14, 0x2000_0020); // CH1 CMAR
        dma.write_reg(0x08, (1 << 0) | (1 << 1) | (1 << 4) | (1 << 6) | (1 << 7));

        let res = dma.tick();
        assert!(res.irq);
        assert!(res.dma_requests.is_some());
        let reqs = res.dma_requests.unwrap();
        assert_eq!(reqs.len(), 1);
        assert_eq!(reqs[0].direction, DmaDirection::Write);
        assert_eq!(reqs[0].src_addr, 0x2000_0020);
        assert_eq!(reqs[0].addr, 0x2000_0010);
        // CH1 TCIF is bit 1.
        assert_ne!(dma.read_reg(0x00) & (1 << 1), 0);
    }

    #[test]
    fn legacy_tick_charges_zero_cost() {
        // B4 tick-cost normalization: a transferring tick must not inflate
        // total_cycles, so the walk-on reference and the scheduler path agree.
        let mut dma = Dma1::new();
        dma.write_reg(0x0C, 4); // CH1 CNDTR = 4
        dma.write_reg(0x14, 0x2000_0000); // CH1 CMAR
        dma.write_reg(0x10, 0x2000_0100); // CH1 CPAR
                                          // EN|MEM2MEM|MINC|PINC — self-paced, transfers every tick.
        dma.write_reg(0x08, (1 << 0) | (1 << 14) | (1 << 4) | (1 << 6) | (1 << 7));
        for _ in 0..8 {
            assert_eq!(dma.tick().cycles, 0);
        }
    }

    #[test]
    fn without_clock_stays_on_legacy_tick_path() {
        let dma = Dma1::new();
        assert!(
            !dma.uses_scheduler(),
            "no cycle clock attached → the model must stay on the legacy walk"
        );
        assert!(dma.needs_legacy_walk());
    }

    #[cfg(feature = "event-scheduler")]
    mod scheduler_mode {
        use super::*;
        use crate::CycleClock;

        fn en_mem2mem(minc_pinc: bool) -> u32 {
            let mut v = (1 << 0) | (1 << 14); // EN | MEM2MEM
            if minc_pinc {
                v |= (1 << 6) | (1 << 7); // PINC | MINC
            }
            v
        }

        /// Drive a scheduler-mode DMA exactly the way `Machine` + `SystemBus`
        /// do at tick interval 1: publish the clock each cycle, convert
        /// write-armed / request-armed events at `cycle + 1 + delay`, drain due
        /// events through `on_event` (rescheduling at `now + delay`), and record
        /// the cycles on which a channel pends the own-IRQ and the requests it
        /// emits.
        struct SchedHarness {
            dma: Dma1,
            clock: CycleClock,
            bus: crate::bus::SystemBus,
            /// (deadline, token).
            events: Vec<(u64, u32)>,
            now: u64,
            pends: Vec<u64>,
            reqs: Vec<(u64, DmaRequest)>,
        }

        impl SchedHarness {
            fn new() -> Self {
                let clock = CycleClock::default();
                let mut dma = Dma1::new();
                dma.attach_cycle_clock(clock.clone());
                Self {
                    dma,
                    clock,
                    bus: crate::bus::SystemBus::new(),
                    events: Vec::new(),
                    now: 0,
                    pends: Vec::new(),
                    reqs: Vec::new(),
                }
            }

            /// MMIO write at the current cycle through the bus chokes' contract:
            /// sync first, write, then harvest `(delay, token)` as
            /// `now + 1 + delay` (the `collect_scheduled_events` identity).
            fn write(&mut self, offset: u64, value: u32) {
                self.dma.sync_to(self.now);
                self.dma.write_reg(offset, value);
                for (delay, token) in self.dma.take_scheduled_events() {
                    self.events.push((self.now + 1 + delay, token));
                }
            }

            /// A routed peripheral DMA request at the current cycle: sets the
            /// channel active, then the bus harvests the freshly-armed event
            /// (the `route_dma_signal` → `collect_scheduled_events` hook).
            fn request(&mut self, request_id: u32) {
                self.dma.dma_request(request_id);
                for (delay, token) in self.dma.take_scheduled_events() {
                    self.events.push((self.now + 1 + delay, token));
                }
            }

            /// Advance one cycle and drain due events.
            fn step(&mut self) {
                self.now += 1;
                self.clock.publish(self.now);
                let due: Vec<(u64, u32)> = self
                    .events
                    .iter()
                    .copied()
                    .filter(|(d, _)| *d <= self.now)
                    .collect();
                self.events.retain(|(d, _)| *d > self.now);
                let mut sched = crate::sched::EventScheduler::new();
                sched.advance_to(self.now);
                for (_, token) in due {
                    let res = self.dma.on_event(token, &mut sched, &mut self.bus);
                    if res.raise_own_irq {
                        self.pends.push(self.now);
                    }
                    for r in res.dma_requests {
                        self.reqs.push((self.now, r));
                    }
                    if let Some(delay) = res.reschedule_delay {
                        self.events.push((self.now + delay, token));
                    }
                }
            }
        }

        /// Legacy per-tick oracle: one `Dma1::tick()` on a forced-legacy model,
        /// returning (irq, requests) for this tick.
        fn walk_tick(dma: &mut Dma1) -> (bool, Vec<DmaRequest>) {
            let res = dma.tick();
            (res.irq, res.dma_requests.unwrap_or_default())
        }

        /// The heart of the fidelity gate: replay a register/request script
        /// against (a) the legacy per-tick walk and (b) the event path, and
        /// compare, at EVERY cycle, the full register snapshot, the emitted
        /// request stream, and the exact set of IRQ-pend cycles.
        ///
        /// `script`: `(cycle, kind)` where kind is `Write(offset, value)` or
        /// `Request(request_id)`, applied before that cycle's tick.
        fn assert_walk_identical(script: &[(u64, Op)], cycles: u64, what: &str) {
            let mut walk = Dma1::new(); // no clock → legacy
            let mut sched = SchedHarness::new();

            let mut walk_pends: Vec<u64> = Vec::new();
            let mut walk_reqs: Vec<(u64, DmaRequest)> = Vec::new();

            for c in 1..=cycles {
                for (sc, op) in script {
                    if *sc == c {
                        match *op {
                            Op::Write(off, val) => {
                                walk.write_reg(off, val);
                                sched.now = c - 1;
                                sched.write(off, val);
                            }
                            Op::Request(id) => {
                                walk.dma_request(id);
                                sched.now = c - 1;
                                sched.request(id);
                            }
                        }
                    }
                }
                let (wi, wr) = walk_tick(&mut walk);
                if wi {
                    walk_pends.push(c);
                }
                for r in wr {
                    walk_reqs.push((c, r));
                }
                sched.now = c - 1;
                sched.step();

                // Full register snapshot must match every cycle.
                assert_eq!(
                    walk.snapshot(),
                    sched.dma.snapshot(),
                    "{what}: register state diverged at cycle {c}"
                );
            }
            assert_eq!(walk_pends, sched.pends, "{what}: IRQ pend cycles diverged");
            assert_eq!(
                walk_reqs, sched.reqs,
                "{what}: emitted DMA request stream diverged"
            );
        }

        #[derive(Clone, Copy)]
        enum Op {
            Write(u64, u32),
            Request(u32),
        }

        #[test]
        fn clock_attach_flips_to_scheduler_and_walk_tick_is_inert() {
            let mut dma = Dma1::new();
            dma.attach_cycle_clock(CycleClock::default());
            assert!(dma.uses_scheduler());
            assert!(!dma.needs_legacy_walk());
            dma.write_reg(0x0C, 4);
            dma.write_reg(0x08, en_mem2mem(true));
            let r = dma.tick();
            assert!(r.dma_requests.is_none(), "tick inert in scheduler mode");
            assert_eq!(dma.channels[0].cndtr, 4, "no element transferred by tick");
        }

        #[test]
        fn mem2mem_self_paced_transfer_walk_identity() {
            // CH1 mem2mem, 6 elements, MINC|PINC — self-drains one per cycle.
            let script = [
                (1u64, Op::Write(0x0C, 6)),        // CNDTR = 6
                (1, Op::Write(0x14, 0x2000_0000)), // CMAR (src)
                (1, Op::Write(0x10, 0x2000_0100)), // CPAR (dst)
                (1, Op::Write(0x08, en_mem2mem(true))),
                (20, Op::Write(0x04, 0xFFFF_FFFF)), // IFCR clear-all mid-idle
            ];
            assert_walk_identical(&script, 40, "mem2mem 6-element self-paced");
        }

        #[test]
        fn mem2mem_tcie_irq_walk_identity() {
            // TCIE set: the completion element pends the NVIC line on the exact
            // cycle in both modes; HTIE too for the half-way crossing.
            let script = [
                (1u64, Op::Write(0x0C, 8)),
                (1, Op::Write(0x14, 0x2000_0000)),
                (1, Op::Write(0x10, 0x2000_0100)),
                // EN|MEM2MEM|MINC|PINC|HTIE|TCIE
                (1, Op::Write(0x08, en_mem2mem(true) | (1 << 1) | (1 << 2))),
            ];
            assert_walk_identical(&script, 30, "mem2mem HTIE+TCIE");
        }

        #[test]
        fn peripheral_paced_channel_request_driven_walk_identity() {
            // Non-mem2mem channel: enable arms ONE element, then each routed
            // request drives one more — the walk sets `active` on the request,
            // services it the next tick; the event path arms a delay-0 event.
            let script = [
                (1u64, Op::Write(0x0C, 5)),        // CNDTR = 5
                (1, Op::Write(0x14, 0x2000_0000)), // CMAR
                (1, Op::Write(0x10, 0x2000_0100)), // CPAR
                // EN|DIR(mem->periph)|MINC|TCIE — no MEM2MEM.
                (
                    1,
                    Op::Write(0x08, (1 << 0) | (1 << 4) | (1 << 7) | (1 << 1)),
                ),
                (6, Op::Request(1)),
                (10, Op::Request(1)),
                (14, Op::Request(1)),
                (18, Op::Request(1)),
            ];
            assert_walk_identical(&script, 30, "peripheral-paced request-driven");
        }

        #[test]
        fn cndtr_rewrite_mid_transfer_walk_identity() {
            // Firmware rewrites CNDTR while a mem2mem chain is in flight — the
            // live chain must pick up the new count (no duplicate chain armed).
            let script = [
                (1u64, Op::Write(0x0C, 10)),
                (1, Op::Write(0x14, 0x2000_0000)),
                (1, Op::Write(0x10, 0x2000_0100)),
                (1, Op::Write(0x08, en_mem2mem(true) | (1 << 1))),
                (4, Op::Write(0x04, 0x0000_0002)), // clear CH1 TCIF-ish bit (a write → collect)
                (5, Op::Write(0x0C, 3)),           // shrink CNDTR mid-flight
            ];
            assert_walk_identical(&script, 25, "CNDTR rewrite mid-transfer");
        }

        #[test]
        fn multi_channel_concurrent_walk_identity() {
            // Two mem2mem channels running at once: both transfer the same
            // cycle in the walk; two independent event chains in scheduler mode.
            let script = [
                (1u64, Op::Write(0x0C, 5)),        // CH1 CNDTR
                (1, Op::Write(0x14, 0x2000_0000)), // CH1 CMAR
                (1, Op::Write(0x10, 0x2000_0100)), // CH1 CPAR
                (1, Op::Write(0x08, en_mem2mem(true))),
                (1, Op::Write(0x20, 4)), // CH2 CNDTR (0x08 + 20 + 0x04)
                (1, Op::Write(0x28, 0x2000_0200)), // CH2 CMAR
                (1, Op::Write(0x24, 0x2000_0300)), // CH2 CPAR
                (1, Op::Write(0x1C, en_mem2mem(true))), // CH2 CCR
            ];
            assert_walk_identical(&script, 20, "two concurrent mem2mem channels");
        }
    }
}
