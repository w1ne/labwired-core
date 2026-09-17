// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Per-cycle peripheral tick orchestration, DMA, and NVIC/interrupt-matrix
//! aggregation. Split out of `bus/mod.rs`.

use super::*;
use crate::{Bus, DmaRequest, Peripheral};
use std::sync::atomic::Ordering;
use std::sync::Arc;

/// Pend a peripheral-raised IRQ. Behaviour depends on whether the chip
/// has an NVIC modelled:
///
/// - **With NVIC** (production chip configs): `irq` is the NVIC IRQ
///   position (0-based, as it appears in chip yaml — DMA1_CH1 = 11 on
///   STM32L4, USART2 = 38). We pend it on ISPR and let
///   `collect_enabled_nvic_interrupts` translate to an exception number
///   (16 + position) when ISER also has it enabled. The previous code
///   special-cased `irq < 16`, which silently routed DMA1_CH1 (irq=11)
///   through the system-exception path and ended up calling SVCall
///   on every DMA TC — invisible until firmware actually hooked the
///   IRQ.
///
/// - **Without NVIC** (legacy unit-test fixtures with no NVIC entry):
///   pass `irq` through unchanged. Single-peripheral test machines
///   call `tick_peripherals()` and read the result directly; they treat
///   the irq value as whatever convention the test author chose.
///
/// Keep a LEVEL source's NVIC pending bit in step with its line, both
/// directions. Asserted: pend and MARK (`level_pended`), so the bit's origin is
/// distinguishable from a software ISPR write. Deasserted: un-pend ONLY a
/// marked bit — firmware that cleared the status flag inside the handler is
/// not re-entered for the same event (the measured 1.95-entries-per-update
/// double-fire), while a software pend of a low line still fires once, as on
/// silicon. Active state is deliberately NOT consulted: the deassert that
/// matters happens precisely while the handler is active.
pub(crate) fn reconcile_nvic_level(
    nvic: &Option<Arc<crate::peripherals::nvic::NvicState>>,
    irq: u32,
    level: bool,
) {
    if let Some(nvic) = nvic {
        let idx = (irq / 32) as usize;
        let bit = 1u32 << (irq % 32);
        if idx < 8 {
            if level {
                nvic.ispr[idx].fetch_or(bit, std::sync::atomic::Ordering::SeqCst);
                nvic.level_pended[idx].fetch_or(bit, std::sync::atomic::Ordering::SeqCst);
            } else if nvic.level_pended[idx].load(std::sync::atomic::Ordering::SeqCst) & bit != 0 {
                nvic.ispr[idx].fetch_and(!bit, std::sync::atomic::Ordering::SeqCst);
                nvic.level_pended[idx].fetch_and(!bit, std::sync::atomic::Ordering::SeqCst);
            }
        }
    }
}

fn pend_nvic(
    nvic: &Option<Arc<crate::peripherals::nvic::NvicState>>,
    interrupts: &mut Vec<u32>,
    irq: u32,
) {
    if let Some(nvic) = nvic {
        let idx = (irq / 32) as usize;
        let bit = irq % 32;
        if idx < 8 {
            nvic.ispr[idx].fetch_or(1 << bit, std::sync::atomic::Ordering::SeqCst);
        }
    } else {
        interrupts.push(irq);
    }
}

impl SystemBus {
    /// Config-time derivation of walk-deletability (issue: browser-perf chain).
    ///
    /// Returns `true` iff EVERY peripheral currently on the bus is provably
    /// *walk-independent for all reachable firmware states* — meaning deleting
    /// the per-cycle legacy walk (`legacy_tick_indices` iteration in
    /// `tick_peripherals_phase1`) cannot change any observable output no matter
    /// what the firmware does. A peripheral qualifies when either:
    ///
    /// 1. `uses_scheduler()` — the walk loop already returns `default()` for it
    ///    every cycle (it is driven by the event scheduler, not the walk), so
    ///    skipping the whole loop changes nothing; or
    /// 2. `!needs_legacy_walk()` — its `tick()`/`tick_elapsed()` is a structural
    ///    no-op for ALL states (a pure register bank, a stub, or a
    ///    lazily-evaluated model that never mutates observable state from the
    ///    walk). See the `Peripheral::needs_legacy_walk` contract.
    ///
    /// CONSERVATIVE by construction: the default `needs_legacy_walk()` is
    /// `true`, so any peripheral whose walk-independence is not *proven* (an
    /// unknown native model, a timer/ADC/DMA/EXTI/SysTick whose `tick()` does
    /// real work once firmware arms it, a declarative bank with timed inflight
    /// events) forces `false` here and the walk stays on. Getting this wrong
    /// would silently starve a peripheral of ticks, so the predicate errs
    /// entirely toward keeping the walk.
    ///
    /// Note this is strictly weaker than a hand `walk_deleted: true`: the hand
    /// flag can assert firmware-*specific* byte-identity (e.g. "this firmware
    /// never touches the 11 timers the chip descriptor instantiates"), which no
    /// config-time predicate can prove. Such configs must keep the explicit
    /// opt-in.
    pub(crate) fn derive_walk_deletable(&self) -> bool {
        self.peripherals
            .iter()
            .all(|p| p.dev.uses_scheduler() || !p.dev.needs_legacy_walk())
    }

    /// Re-run [`Self::derive_walk_deletable`] and latch it into
    /// `legacy_walk_disabled`. Callers that flip a peripheral's drive mode AFTER
    /// bus assembly — swapping a model in, or pinning one back onto the walk with
    /// `force_legacy_walk` — must call this so the walk-deletion flag matches the
    /// live peripheral set (the rom-boot path derives it once over the initial
    /// set). Without it, a peripheral pinned back to the walk on an
    /// already-walk-deleted bus is silently starved of ticks. Mirrors the inline
    /// recompute the rom-boot path (`esp32c3_rom`) and the in-crate routing gates
    /// already perform; exposed publicly for out-of-crate test harnesses.
    pub fn recompute_walk_deletable(&mut self) {
        self.legacy_walk_disabled = self.derive_walk_deletable();
    }

    pub(crate) fn tick_profile_entry_counts(&self) -> (usize, usize) {
        let bus_tick_entries = self.bus_tick_indices.len();
        let legacy_tick_entries = if cfg!(feature = "event-scheduler") && self.legacy_walk_disabled
        {
            0
        } else {
            self.legacy_tick_indices.len()
        };
        (bus_tick_entries, legacy_tick_entries)
    }

    pub(crate) fn read_cached_declarative_u32(&self, idx: usize, offset: u64) -> Option<u32> {
        self.peripherals
            .get(idx)?
            .dev
            .as_any()?
            .downcast_ref::<crate::peripherals::declarative::GenericPeripheral>()?
            .peek_u32_raw(offset)
    }

    /// Phase 2B.1 (issue #192): pend an NVIC IRQ on behalf of an event
    /// handler. Mirrors the per-tick `pend_nvic` path but collects
    /// non-NVIC fallthroughs into the supplied vector for the caller to
    /// forward to `cpu.set_exception_pending`.
    ///
    /// Same-tick CPU visibility (walk-free B2/B3): the legacy walk pends
    /// ISPR and scans ISPR&ISER into the CPU **in the same
    /// `tick_peripherals` call**, so a walk-raised IRQ dispatches before the
    /// very next instruction. The event drain runs after that scan, so an
    /// event-raised IRQ left only in ISPR would become CPU-visible one tick
    /// late. To keep event-path IRQ delivery cycle-identical to the walk,
    /// an ISER-enabled pend is ALSO pushed into `fallthrough` as its
    /// exception number (16 + position) for the caller's
    /// `cpu.set_exception_pending` — exactly what the walk's same-tick NVIC
    /// scan would have produced. A not-yet-enabled pend stays ISPR-only and
    /// is picked up by the per-tick scan once firmware enables it (identical
    /// to the walk).
    pub fn pend_irq_for_event(&self, irq: u32, fallthrough: &mut Vec<u32>) {
        pend_nvic(&self.nvic, fallthrough, irq);
        if let Some(nvic) = &self.nvic {
            let idx = (irq / 32) as usize;
            let bit = irq % 32;
            if idx < 8 && (nvic.iser[idx].load(Ordering::SeqCst) & (1 << bit)) != 0 {
                fallthrough.push(16 + irq);
            }
        }
    }

    /// Route a peripheral DMA signal (`source_name` + `request_id`) to its
    /// target DMA channel. Single source of truth shared by the legacy
    /// `tick_peripherals_with_costs` path and the event path
    /// (`Machine::apply_event_result`), so both behave identically.
    pub fn route_dma_signal(&mut self, source_name: &str, request_id: u32) {
        // Simplified routing for Top-5 targets (e.g. STM32F1):
        // UART1_TX (signal ID 1) -> DMA1 Channel 1 (H5 uses GPDMA; mocked here).
        let target_dma = if (source_name == "uart1" || source_name == "uart3") && request_id == 1 {
            Some(("dma1", 1))
        } else {
            None
        };
        if let Some((dma_name, channel)) = target_dma {
            if let Some(p_idx) = self.find_peripheral_index_by_name(dma_name) {
                self.peripherals[p_idx].dev.dma_request(channel);
                // Walk-free B4: a routed request makes the channel active. On a
                // scheduler-driven DMA (walk-skipped) the transfer has to ride
                // an event, so harvest the freshly-armed element event into
                // `pending_schedule` at deadline `current_cycle + 1` — the exact
                // cycle the legacy walk's next tick would have serviced it. No-op
                // for a legacy-walk DMA (`collect_scheduled_events` guards on
                // `uses_scheduler()`), so the walk path is unchanged.
                #[cfg(feature = "event-scheduler")]
                self.collect_scheduled_events(p_idx);
            }
        }
    }

    /// Phase 2B.2 (issue #192): if the peripheral at `idx` is scheduler-driven,
    /// advance its lazy state to the current CPU cycle (`current_cycle`, the
    /// batch-start cycle — the same cycle count the legacy walk advances by via
    /// `tick_elapsed(interval)`) before an MMIO write observes it. One virtual
    /// `uses_scheduler()` call for legacy peripherals (false → return); the
    /// sync only runs for opted-in ones.
    #[cfg(feature = "event-scheduler")]
    #[inline]
    pub(crate) fn sync_scheduler_peripheral(&mut self, idx: usize) {
        let p = &mut self.peripherals[idx];
        if p.dev.uses_scheduler() {
            p.dev.sync_to(self.current_cycle);
        }
    }

    /// Phase 2B.3a (issue #192): after an MMIO write to a scheduler-driven
    /// peripheral, harvest any events it wants scheduled (e.g. a just-armed
    /// TX interrupt) into `pending_schedule` for `Machine` to enqueue. One
    /// virtual `uses_scheduler()` call for legacy peripherals (false → return).
    ///
    /// The peripheral's `(delay_cycles, token)` is relative to its just-synced
    /// state (`sync_to(current_cycle)` ran before the write), so it is
    /// converted to the absolute cycle deadline `current_cycle + 1 + delay`
    /// here — pinning the deadline to the write instead of to whenever the
    /// next scheduler drain happens to run. The `+ 1` preserves the historical
    /// contract exactly: delays were relative to the next drain, and at tick
    /// interval 1 the next drain always runs one cycle after the write, so
    /// interval-1 deadlines are byte-identical to the pre-conversion build. At
    /// interval > 1 the deadline no longer stretches with the drain cadence —
    /// an SPI half-period of N cycles stays N cycles.
    /// No-op without the scheduler so external input paths can share this
    /// collection seam without adding their own feature branches.
    #[inline]
    pub(crate) fn collect_scheduled_events(&mut self, _idx: usize) {
        #[cfg(feature = "event-scheduler")]
        {
            if !self.peripherals[_idx].dev.uses_scheduler() {
                return;
            }
            for (delay, token) in self.peripherals[_idx].dev.take_scheduled_events() {
                self.pending_schedule
                    .push((_idx, self.current_cycle + 1 + delay, token));
            }
        }
    }

    /// Pre-tick bus-aware pass: lend `&mut self` into every `tick_with_bus`
    /// peripheral (RADIO Easy-DMA, the WiFi descriptor-ring/medium pump). The
    /// swap dance temporarily removes each peripheral so it can borrow the bus;
    /// a no-op stub stands in for the duration. Extracted so the CPU idle
    /// fast-forward path can run exactly the same pump at the poll deadline (via
    /// [`Self::run_idle_poll_bus_tick`]) instead of duplicating it.
    pub(crate) fn run_bus_tick_pass(&mut self) {
        let mut bus_tick_pos = 0;
        while bus_tick_pos < self.bus_tick_indices.len() {
            let i = self.bus_tick_indices[bus_tick_pos];
            let placeholder: Box<dyn Peripheral> =
                Box::new(crate::peripherals::stub::StubPeripheral::new(0));
            let mut dev = std::mem::replace(&mut self.peripherals[i].dev, placeholder);
            dev.tick_with_bus(self);
            self.peripherals[i].dev = dev;
            let still_needs_bus_tick = self.refresh_bus_tick_index(i);
            if self.peripherals[i].dev.legacy_tick_dynamic() {
                self.refresh_legacy_tick_index(i);
            }
            if still_needs_bus_tick {
                bus_tick_pos += 1;
            }
        }
    }

    /// Forced-walk twin of [`Self::run_bus_tick_pass`] for the bare-CPU
    /// hardware-oracle boundary ([`Self::tick_peripherals_fully_forced`]).
    ///
    /// `bus_tick_indices` is derived from `needs_bus_tick()`, which a
    /// scheduler-driven bus-mover (the RP2040 DMA) reports `false` for so the
    /// walk cannot double-drive the transfer its `on_event` chain owns. That
    /// also means the model is entirely absent from the cached set, so
    /// `tick_elapsed_forced` alone cannot reach it — the forced pass must
    /// re-derive membership from `needs_bus_tick_forced()` in peripheral-index
    /// order, exactly as the legacy tick walk re-derives `forced_tick_indices`
    /// from `legacy_tick_active()`. Defaults forward to the ordinary hooks, so
    /// on any bus without a forced-only bus-mover this reproduces
    /// `run_bus_tick_pass` entry-for-entry.
    fn run_bus_tick_pass_forced(&mut self) {
        let mut forced: Vec<usize> = (0..self.peripherals.len())
            .filter(|&i| self.peripherals[i].dev.needs_bus_tick_forced())
            .collect();
        let mut pos = 0;
        while pos < forced.len() {
            let i = forced[pos];
            let placeholder: Box<dyn Peripheral> =
                Box::new(crate::peripherals::stub::StubPeripheral::new(0));
            let mut dev = std::mem::replace(&mut self.peripherals[i].dev, placeholder);
            dev.tick_with_bus_forced(self);
            self.peripherals[i].dev = dev;
            // Keep the cached sets honest for the models that DO live on the
            // ordinary pass (a WiFi MAC that just went quiet), mirroring
            // `run_bus_tick_pass`.
            let _ = self.refresh_bus_tick_index(i);
            if self.peripherals[i].dev.legacy_tick_dynamic() {
                self.refresh_legacy_tick_index(i);
            }
            if self.peripherals[i].dev.needs_bus_tick_forced() {
                pos += 1;
            } else {
                forced.remove(pos);
            }
        }
    }

    /// True when a bus peripheral services an external medium that must keep
    /// being polled at a bounded cadence through a CPU idle skip (see
    /// [`Peripheral::idle_poll_bus_tick`]) — currently a medium-mode WiFi MAC.
    /// Only the tiny `bus_tick_indices` set is scanned (empty on every non-WiFi
    /// bus, so this is ~free on the idle-fast-forward hot check). Only the
    /// event-scheduler fast-forward path consults it.
    #[cfg(feature = "event-scheduler")]
    pub(crate) fn idle_poll_bus_tick_active(&self) -> bool {
        self.bus_tick_indices
            .iter()
            .any(|&i| self.peripherals[i].dev.idle_poll_bus_tick())
    }

    /// Run the bus-tick pump once from the CPU idle fast-forward path, after the
    /// skip has advanced `current_cycle`, so an external medium's inbound frames
    /// (and device-cycle-keyed beacons) are serviced at the poll deadline
    /// instead of being starved for the whole idle window.
    #[cfg(feature = "event-scheduler")]
    pub(crate) fn run_idle_poll_bus_tick(&mut self) {
        self.run_bus_tick_pass();
    }

    #[allow(clippy::type_complexity)]
    fn tick_peripherals_phase1(
        &mut self,
        force_scheduler_walk: bool,
    ) -> (
        Vec<u32>,
        Vec<PeripheralTickCost>,
        Vec<DmaRequest>,
        Vec<(String, u32)>,
        Vec<u32>,
    ) {
        self.service_motor_models();
        let mut interrupts = Vec::new();
        let mut costs = Vec::new();
        let mut dma_requests = Vec::new();
        let mut dma_signals_out = Vec::new();

        // Some older tests and internal harnesses still mutate
        // `bus.peripherals` directly instead of going through `add_peripheral`.
        // Detect that structural drift once here so the cached tick set remains
        // correct without reinstating the old every-cycle full peripheral walk.
        if self.peripheral_ranges.len() != self.peripherals.len() {
            self.rebuild_peripheral_ranges();
        }

        // ── Pre-tick bus-aware pass ─────────────────────────────────────────
        // Some peripherals (currently just RADIO) need to read/write the bus
        // BEFORE their `tick()` runs so the work they schedule (e.g. setting
        // a bit-rate countdown after reading PACKETPTR-pointed RAM) is
        // visible to that same tick(). The swap dance below temporarily
        // removes the peripheral from `self.peripherals` so we can lend
        // `&mut self` into `tick_with_bus`; a no-op stub stands in for the
        // duration. `needs_bus_tick` returning false skips this for
        // everyone else at near-zero cost.
        //
        // The bare-CPU oracle boundary takes the forced twin, which re-derives
        // membership from `needs_bus_tick_forced()` so a scheduler-driven
        // bus-mover (the RP2040 DMA) still performs its legacy one-tick
        // transfer with no `Machine` around to drain its event chain.
        if force_scheduler_walk {
            self.run_bus_tick_pass_forced();
        } else {
            self.run_bus_tick_pass();
        }

        // Plan 3: collect ESP32-S3 explicit_irq source IDs during pass 1 so
        // they can be routed through the intmatrix in a follow-up pass that
        // requires `&self` (incompatible with the iter_mut borrow here).
        let mut explicit_source_ids: Vec<u32> = Vec::new();

        // Cross-peripheral side-effects collected during phase 1 and
        // applied after the iter_mut borrow ends.
        let mut pending_mmio: Vec<(u32, u32)> = Vec::new();
        let mut fired_events_global: Vec<u32> = Vec::new();

        let tick_interval = self.config.peripheral_tick_interval as u64;

        // Phase 2B.3c (issue #192): if every peripheral on this bus is migrated
        // or inert, the whole walk is skipped — the actual orchestration win.
        // Read once before the borrow; gated so flag-off always walks.
        #[cfg(feature = "event-scheduler")]
        let legacy_walk_disabled = self.legacy_walk_disabled;

        // The hardware-oracle compatibility path must reproduce the legacy
        // full walk even when scheduler-driven entries were intentionally
        // omitted from `legacy_tick_indices`. Reconstruct the pre-scheduler
        // active set in original peripheral-index order; ordering matters for
        // collected MMIO, DMA, event, and IRQ effects. The production path
        // keeps using the allocation-free cached slice.
        let forced_tick_indices = if force_scheduler_walk {
            self.peripherals
                .iter()
                .enumerate()
                .filter_map(|(idx, p)| p.dev.legacy_tick_active().then_some(idx))
                .collect()
        } else {
            Vec::new()
        };

        let mut tick_pos = 0;
        #[cfg(feature = "event-scheduler")]
        if legacy_walk_disabled && !force_scheduler_walk {
            tick_pos = self.legacy_tick_indices.len();
        }
        while let Some(peripheral_index) = if force_scheduler_walk {
            forced_tick_indices.get(tick_pos).copied()
        } else {
            self.legacy_tick_indices.get(tick_pos).copied()
        } {
            let Some((res, irq, base, refresh_after_tick)) =
                self.peripherals.get_mut(peripheral_index).map(|p| {
                    // Phase 2B.2 (issue #192): scheduler-driven peripherals are advanced
                    // lazily via `sync_to` on MMIO access (and by the event drain in
                    // `Machine::step`), never by this per-cycle walk. Skipping them here
                    // is the actual orchestration saving. Gated so the legacy build is
                    // byte-identical.
                    #[cfg(feature = "event-scheduler")]
                    if p.dev.uses_scheduler() && !force_scheduler_walk {
                        return (
                            crate::PeripheralTickResult::default(),
                            p.irq,
                            p.base,
                            p.dev.legacy_tick_dynamic(),
                        );
                    }

                    if p.ticks_remaining > tick_interval {
                        p.ticks_remaining -= tick_interval;
                        return (
                            crate::PeripheralTickResult::default(),
                            p.irq,
                            p.base,
                            p.dev.legacy_tick_dynamic(),
                        );
                    }

                    let res = if force_scheduler_walk {
                        p.dev.tick_elapsed_forced(tick_interval)
                    } else {
                        p.dev.tick_elapsed(tick_interval)
                    };
                    p.ticks_remaining = res.ticks_until_next.unwrap_or(0);
                    (res, p.irq, p.base, p.dev.legacy_tick_dynamic())
                })
            else {
                tick_pos += 1;
                continue;
            };
            let still_active = if refresh_after_tick {
                self.refresh_legacy_tick_index(peripheral_index)
            } else {
                true
            };

            if res.cycles > 0 {
                costs.push(PeripheralTickCost {
                    index: peripheral_index,
                    cycles: res.cycles,
                });
            }

            if let Some(reqs) = res.dma_requests {
                dma_requests.extend(reqs);
            }

            if let Some(signals) = res.dma_signals {
                let name = self.peripherals[peripheral_index].name.clone();
                for sig in signals {
                    dma_signals_out.push((name.clone(), sig));
                }
            }

            // A LEVEL source is reconciled in both directions from its own
            // line; `res.irq` is redundant for it (the walk re-raises while
            // held). Everything else keeps pulse semantics unchanged.
            match (self.peripherals[peripheral_index].dev.irq_line_level(), irq) {
                (Some(level), Some(irq)) => {
                    reconcile_nvic_level(&self.nvic, irq, level);
                    if res.irq && self.nvic.is_none() {
                        interrupts.push(irq);
                    }
                }
                _ => {
                    if res.irq {
                        if let Some(irq) = irq {
                            pend_nvic(&self.nvic, &mut interrupts, irq);
                        }
                    }
                }
            }

            if let Some(irqs) = res.explicit_irqs {
                for irq in &irqs {
                    pend_nvic(&self.nvic, &mut interrupts, *irq);
                }
                // Plan 3: stash source IDs for pass-2 intmatrix routing.
                explicit_source_ids.extend(irqs);
            }

            // System exceptions (SysTick = 15, etc) bypass NVIC and are
            // pushed directly so the CPU sees them on next dispatch.
            if let Some(exc) = res.system_exception {
                interrupts.push(exc);
            }

            // Cross-peripheral writes: collected here, applied below
            // (we can't call self.write_u32 while iter_mut holds the
            // borrow).
            pending_mmio.extend(res.mmio_writes);

            // Globalise event offsets (relative to peripheral window) into
            // absolute bus addresses so PPI sees them at the same address
            // firmware uses for CH[i].EEP.
            for off in res.fired_events {
                fired_events_global.push((base as u32).wrapping_add(off));
            }

            // Forced mode walks a fixed one-shot snapshot. Refresh the normal
            // cache for future production ticks, but always advance this
            // snapshot cursor even when a dynamic entry just became inactive.
            if force_scheduler_walk || still_active {
                tick_pos += 1;
            }
        }

        // Apply any cross-peripheral mmio writes the peripherals requested
        // (e.g. GPIOTE → GPIO OUTSET/OUTCLR).  Errors are logged but not
        // propagated — these are best-effort side-effects, not core sim
        // failures.
        for (addr, val) in pending_mmio.drain(..) {
            if let Err(e) = self.write_u32(addr as u64, val) {
                tracing::warn!("phase1 mmio_write 0x{addr:08X} = 0x{val:08X} failed: {e:?}");
            }
        }

        // PPI routing pass: feed every fired event through any peripheral
        // that overrides route_ppi_events (only Nrf52Ppi does).  Each
        // returned absolute address is a task to trigger by writing 1.
        if !fired_events_global.is_empty() {
            let mut pending_tasks: Vec<u32> = Vec::new();
            for p in self.peripherals.iter_mut() {
                let tasks = p.dev.route_ppi_events(&fired_events_global);
                pending_tasks.extend(tasks);
            }
            for task_addr in pending_tasks {
                if let Err(e) = self.write_u32(task_addr as u64, 1) {
                    tracing::warn!("PPI task trigger 0x{task_addr:08X} failed: {e:?}");
                }
            }
        }

        if !self.irq_fabric.esp32c3.routing {
            // GPIO edge-detection pass: snapshot the IN registers of GPIO ports
            // 0 and 1, diff against last-known state, and notify every
            // peripheral of changed pins. GPIOTE overrides observe_gpio_change
            // to drive EVENTS_IN[i] when a channel watches a matching pin.
            //
            // ESP32-C3 does not use this Nordic GPIO/GPIOTE service path; its
            // board inputs write the C3 GPIO register model directly. Skipping
            // this block is important because C3 ROM-boot needs very frequent
            // ticks for interrupt-matrix correctness.
            // Which peripheral is which port. Numbered (Nordic) and lettered
            // (Silicon Labs, ST) spellings map onto the same index space, so a
            // chip's ports are 0..3 whatever it calls them — and the tuple a
            // watcher receives means the same thing on both.
            //
            // ⚠️ Levels come from `Peripheral::read_gpio_input`, NOT from a
            // hardcoded register offset. This pass used to read `base + 0x510`,
            // which is the Nordic IN register and is DOUT on a Series-2 port —
            // so an EFR32 edge was never observed at all and its EXTI could
            // never fire. `read_gpio_input` asks each model for its own input
            // register, and answers the identical value for the Nordic ports
            // it already served.
            const GPIO_PORT_IDS: [(&str, usize); 8] = [
                ("gpio0", 0),
                ("gpio1", 1),
                ("gpio2", 2),
                ("gpio3", 3),
                ("gpioa", 0),
                ("gpiob", 1),
                ("gpioc", 2),
                ("gpiod", 3),
            ];
            // Resolved once, then cached — see `SystemBus::gpio_port_idx`. Doing
            // it per tick is eight linear scans of the peripheral list with a
            // string compare each, on every boundary of every chip, to arrive at
            // the same four indices every time.
            let gpio_idx = match self.gpio_port_idx {
                Some(cached) => cached,
                None => {
                    let mut resolved: [Option<usize>; 4] = [None; 4];
                    for (name, port) in GPIO_PORT_IDS {
                        if resolved[port].is_none() {
                            resolved[port] = self.find_peripheral_index_by_name(name);
                        }
                    }
                    self.gpio_port_idx = Some(resolved);
                    resolved
                }
            };
            let mut changes: Vec<(u8, u8, u8)> = Vec::new();
            // First pass ADOPTS the live levels as the baseline (see
            // `last_gpio_in`): nothing has transitioned yet, so `baseline` is
            // `None` and no change is reported for any pin the outside world
            // already holds.
            let baseline = self.last_gpio_in;
            let mut current_in = baseline.unwrap_or([0; 4]);
            for (port, idx) in gpio_idx.iter().enumerate() {
                let Some(idx) = idx else { continue };
                // Whole port in one call. `read_gpio_input_word`'s default IS
                // the pin-by-pin loop that used to be written out here (first
                // `None` ends the port, so a port narrower than 32 pins leaves
                // the rest of the word clear), so a model that does not
                // override it produces the identical word; `GpioPort` does
                // override it, and answers with one register read instead of
                // 32 evaluations of a computed input register.
                let cur = self.peripherals[*idx].dev.read_gpio_input_word();
                current_in[port] = cur;
                let Some(prev) = baseline.map(|b| b[port]) else {
                    continue;
                };
                let diff = cur ^ prev;
                if diff != 0 {
                    for pin in 0..32u8 {
                        if diff & (1 << pin) != 0 {
                            let level = ((cur >> pin) & 1) as u8;
                            changes.push((port as u8, pin, level));
                        }
                    }
                }
            }
            self.last_gpio_in = Some(current_in);
            if !changes.is_empty() {
                // A GPIO edge can make a dynamic peripheral (e.g. GPIOTE)
                // newly walk-active through `observe_gpio_change` — a
                // CROSS-peripheral activation that the per-MMIO-write refresh
                // choke never observes. Refresh the legacy-tick index of every
                // dynamic peripheral so a freshly-armed GPIOTE re-enters the
                // walk and drains its EVENTS_IN on the next tick (before this,
                // a GPIOTE that opts out of the walk while idle would never be
                // re-added and its input event would be lost). Guarded by an
                // actual edge, so this is off the steady-state hot path.
                for idx in 0..self.peripherals.len() {
                    let latched = self.peripherals[idx].dev.observe_gpio_change(&changes);
                    if self.peripherals[idx].dev.legacy_tick_dynamic() {
                        self.refresh_legacy_tick_index(idx);
                    }
                    // Walk-free: a scheduler-driven peripheral (GPIOTE) that
                    // latched pending work FROM THIS EDGE needs its delay-0
                    // drain event harvested here — the MMIO write choke never
                    // sees a cross-peripheral edge.
                    //
                    // Only the models that latched. Harvesting from every
                    // scheduler-driven peripheral re-arms a duplicate wake on
                    // models that already have one in flight and cannot latch
                    // anything from a GPIO edge: `take_scheduled_events` is a
                    // query of live state, not a one-shot take, so a second
                    // harvest at a later cycle produces a second heap entry at
                    // a DIFFERENT deadline, which the scheduler's byte-identical
                    // dedup cannot collapse. For the RADIO that duplicate fires
                    // inside the same drain as the EasyDMA event and runs
                    // `tick()` while the air-time countdown is pinned at 1 —
                    // raising ADDRESS/PAYLOAD/END immediately and erasing the
                    // whole packet's transmission time.
                    #[cfg(feature = "event-scheduler")]
                    if latched && self.peripherals[idx].dev.uses_scheduler() {
                        self.collect_scheduled_events(idx);
                    }
                    #[cfg(not(feature = "event-scheduler"))]
                    let _ = latched;
                }
            }

            // CAN synthetic services stay Nordic/non-C3: C3 ROM-boot labs do
            // not host them and the high-frequency IRQ tick must stay lean.
            self.service_can_diagnostic_testers();
            self.service_can_uds_testers();
            self.service_can_log_players();
        }

        // Bus-resident external devices (DHT22/DHT11, rotary, keypad) and the
        // HC-SR04 per-tick ECHO drive must run on every chip family — including
        // ESP32-C3, where the Nordic GPIO/GPIOTE block above is skipped.
        //
        // Leaving these inside `!irq_fabric.esp32c3.routing` made every C3 freehand
        // DHT lab print DHT_NAN forever: the write-hook armed the frame, but
        // `service_gpio_devices` never drove external_levels, so digitalRead
        // only ever saw idle/pull-up (live direct twin, 2026-08-11).
        //
        // When HC-SR04 is event-scheduled, ECHO edges come from
        // `Machine::drain_scheduler_events` at exact cycles — skip the per-tick
        // pass so both paths don't drive the pad. `service_gpio_devices`
        // early-outs on an empty list; `per_cycle_tick_is_trivial` already
        // refuses the walk-free fast path when a device needs service.
        if !self.hcsr04_event_scheduled() {
            self.service_hcsr04();
        }
        self.service_gpio_devices();

        (
            interrupts,
            costs,
            dma_requests,
            dma_signals_out,
            explicit_source_ids,
        )
    }

    /// Plan 3: route a batch of ESP32-S3 explicit_irq source IDs through the
    /// registered intmatrix peripheral. Updates `self.pending_cpu_irqs` and
    /// pushes the per-source assertion bitmap into the intmatrix's
    /// PRO_INTR_STATUS_REG_n mirror via `set_pending_sources`. No-op for buses
    /// without an intmatrix peripheral.
    /// ESP32-C3 (RISC-V) interrupt routing. Each tick, record the bitmap of
    /// asserting peripheral interrupt-matrix sources (`explicit_irqs` from the
    /// walk — e.g. the SYSTIMER tick alarm on source 37) and rebuild the
    /// level-sensitive bitmask of asserted CPU interrupt lines from them plus
    /// the SYSTEM FROM_CPU IPI registers (0x600C0028..0x34, bit0) — the
    /// mechanism FreeRTOS `vPortYield` uses to request a context switch. Each
    /// asserted source is routed to a CPU line via its INTERRUPT_CORE0 MAP
    /// register (0x600C2000 + source*4, low 5 bits), gated by CPU_INT_ENABLE
    /// and per-line priority vs CPU_INT_THRESH. The result lands in
    /// `irq_fabric.esp32c3.irq_lines`, which the core ORs into `mip`. No-op unless
    /// `irq_fabric.esp32c3.routing` is set (only the C3 rom-boot path sets it).
    ///
    /// This tick-time pass is no longer the only aggregation point: MMIO
    /// writes that change the routing inputs (INTC enable/threshold/priority/
    /// map, FROM_CPU IPI set/clear) re-aggregate immediately from the write
    /// choke (`sync_esp32c3_irq_cache_write`), so at a tick interval above
    /// one a mid-batch yield/critical-section change is reflected at the
    /// write instruction instead of waiting for the tick boundary. Peripheral
    /// source assert/de-assert stays tick-quantised (≤ one interval — the
    /// same bound the write-path `sync_to` documents). At interval 1 the
    /// tick-end rebuild below runs before the CPU's next instruction-boundary
    /// interrupt check, so behaviour is byte-identical to the pre-choke code.
    fn aggregate_esp32c3_irqs(&mut self, source_ids: &[u32]) {
        if !self.irq_fabric.esp32c3.routing {
            return;
        }

        // Record the level sources asserting THIS tick (rebuilt from scratch,
        // so a de-asserting source drops out at the tick boundary), then
        // recompute the routed line mask from the shared choke.
        let mut asserted = [0u64; 2];
        for &src in source_ids {
            let idx = (src / 64) as usize;
            if idx < asserted.len() {
                asserted[idx] |= 1u64 << (src % 64);
            }
        }
        self.irq_fabric.esp32c3.walk_sources = asserted;
        // Re-derive scheduler-driven peripheral levels (SYSTIMER once migrated
        // off the walk) so their level-sensitive matrix IRQ persists across
        // walk ticks and de-asserts the tick after firmware clears it.
        self.refresh_esp32c3_sched_sources();

        if self.irq_fabric.esp32c3.intc.is_some() {
            self.recompute_esp32c3_irq_lines();
            return;
        }

        // Fallback for buses without the declarative INTC cache (hand-built
        // test buses): read the routing registers directly each tick.
        const INTMATRIX_BASE: u64 = 0x600C_2000;
        const FROM_CPU: [(u64, u32); 4] = [
            (0x600C_0028, 50),
            (0x600C_002C, 51),
            (0x600C_0030, 52),
            (0x600C_0034, 53),
        ];
        let read_intcore = |bus: &SystemBus, offset: u64| {
            bus.irq_fabric
                .esp32c3
                .interrupt_core0_idx
                .and_then(|idx| bus.read_cached_declarative_u32(idx, offset))
                .or_else(|| bus.read_u32(INTMATRIX_BASE + offset).ok())
                .unwrap_or(0)
        };
        let enable = read_intcore(self, 0x104);
        let thresh = read_intcore(self, 0x194) & 0xF;

        let mut mask = 0u32;
        let mut route_source = |src: u32| {
            // MAP register holds the destination CPU interrupt line (1..31).
            let line = read_intcore(self, (src as u64) * 4) & 0x1F;
            let pri = read_intcore(self, 0x114 + (line as u64) * 4) & 0xF;
            if line == 0 || (enable & (1 << line)) == 0 {
                return;
            }
            if pri >= thresh {
                mask |= 1u32 << line;
            }
        };

        // Route peripheral `explicit_irqs` (e.g. the SYSTIMER tick alarm, which
        // the C3 wiring configures to emit matrix source 37) plus the FROM_CPU
        // IPI sources (the FreeRTOS yield mechanism), without allocating on the
        // no-interrupt hot path.
        for &src in source_ids {
            route_source(src);
        }
        // Scheduler-driven peripheral levels (SYSTIMER off the walk) — refreshed
        // into the persistent bitmap above.
        let sched = self.irq_fabric.esp32c3.sched_sources;
        for (word, bits) in sched.iter().enumerate() {
            let mut bits = *bits;
            while bits != 0 {
                let bit = bits.trailing_zeros();
                route_source(word as u32 * 64 + bit);
                bits &= !(1u64 << bit);
            }
        }
        for (addr, src) in FROM_CPU {
            let from_cpu = self
                .irq_fabric
                .esp32c3
                .system_idx
                .and_then(|idx| {
                    let offset = addr.checked_sub(self.peripherals[idx].base)?;
                    self.read_cached_declarative_u32(idx, offset)
                })
                .or_else(|| self.read_u32(addr).ok())
                .unwrap_or(0);
            if from_cpu & 1 != 0 {
                route_source(src);
            }
        }
        self.irq_fabric.esp32c3.irq_lines = mask;
    }

    /// Rebuild `irq_fabric.esp32c3.irq_lines` from the cached C3 routing state: the INTC
    /// register cache (enable/threshold/priority/map — maintained at the MMIO
    /// write choke), the cached FROM_CPU IPI pending bits, and the peripheral
    /// sources recorded by the most recent tick. The single aggregation body
    /// shared by the per-tick pass and the write-choke re-aggregation, so both
    /// produce identical masks from identical inputs.
    ///
    /// INTC control registers (offsets verified against interrupt_core0.yaml):
    ///   CPU_INT_ENABLE 0x104, CPU_INT_PRI_n 0x114+n*4, CPU_INT_THRESH 0x194.
    /// A line fires only while it is enabled AND its priority >= threshold —
    /// the C3 enables/masks via these INTC registers, NOT the RISC-V `mie`
    /// CSR (FreeRTOS critical sections raise the threshold to mask).
    pub(crate) fn recompute_esp32c3_irq_lines(&mut self) {
        const FROM_CPU_SOURCE_BASE: u32 = 50;
        // Latched PMS violations assert their matrix source
        // (`ETS_CORE0_{I,D}RAM0_PMS_INTR_SOURCE`) until firmware pulses
        // VIOLATE_CLR — the same level semantics as every other source here.
        // Read before `cache` is borrowed so the two immutable borrows of
        // `self` do not overlap the closure below.
        let pms_sources = self.esp32c3_pms_sources();
        let Some(cache) = &self.irq_fabric.esp32c3.intc else {
            return;
        };
        let mut mask = 0u32;
        let mut route_source = |src: u32| {
            let Some(&line) = cache.source_line.get(src as usize) else {
                return;
            };
            if line == 0 || (cache.int_enable & (1u32 << line)) == 0 {
                return;
            }
            let pri = cache.line_pri.get(line as usize).copied().unwrap_or(0);
            if pri >= cache.int_thresh {
                mask |= 1u32 << line;
            }
        };

        for (word, (&walk, (&sched, &pms))) in self
            .irq_fabric
            .esp32c3
            .walk_sources
            .iter()
            .zip(
                self.irq_fabric
                    .esp32c3
                    .sched_sources
                    .iter()
                    .zip(&pms_sources),
            )
            .enumerate()
        {
            // Union of walk-emitted level sources (rebuilt each tick),
            // scheduler-driven peripheral level sources (re-derived from
            // `matrix_irq_sources`, so a SYSTIMER migrated off the walk keeps
            // its level-sensitive alarm IRQ routed), and latched PMS
            // violations.
            let mut bits = walk | sched | pms;
            while bits != 0 {
                let bit = bits.trailing_zeros();
                route_source(word as u32 * 64 + bit);
                bits &= !(1u64 << bit);
            }
        }
        let mut pending = cache.from_cpu_pending;
        while pending != 0 {
            let slot = pending.trailing_zeros();
            route_source(FROM_CPU_SOURCE_BASE + slot);
            pending &= !(1 << slot);
        }
        self.irq_fabric.esp32c3.irq_lines = mask;
    }

    /// Re-derive the C3 matrix sources asserted by SCHEDULER-driven peripherals
    /// (skipped by the per-cycle walk) from their live level
    /// (`Peripheral::matrix_irq_sources`). Rebuilt from scratch — level
    /// semantics — so a source that stops asserting (e.g. after the SYSTIMER
    /// alarm's INT_CLR) drops out on the next re-derivation. Called from the
    /// event path (`Machine::apply_event_result`, exact-cycle delivery) and the
    /// walk-tick aggregation (steady-state persistence + de-assert). No-op
    /// unless C3 routing is active. Does NOT recompute — the caller decides
    /// when to fold this into `irq_fabric.esp32c3.irq_lines`.
    pub(crate) fn refresh_esp32c3_sched_sources(&mut self) {
        if !self.irq_fabric.esp32c3.routing {
            return;
        }
        self.irq_fabric.esp32c3.sched_sources = self.poll_scheduler_matrix_sources();
    }

    /// Shared per-fabric primitive: the interrupt-matrix source-ID bitmap
    /// asserted RIGHT NOW by every SCHEDULER-driven peripheral on the bus
    /// (`uses_scheduler()` models, polled via `Peripheral::matrix_irq_sources`).
    /// Fabric-independent — the C3 (RISC-V matrix) and S3 (Xtensa intmatrix)
    /// refresh methods both derive their per-fabric `*_sched_asserted_sources`
    /// bitmap from this ONE poll, so the two fabrics share identical
    /// level-derivation semantics (rebuilt from scratch → a source that stops
    /// asserting drops out on the next poll) and only their storage/routing
    /// differ. Sources ≥ 128 (none on either SoC) are ignored.
    fn poll_scheduler_matrix_sources(&mut self) -> [u64; 2] {
        let mut asserted = [0u64; 2];
        // Fill each peripheral's asserted source IDs into ONE retained scratch
        // buffer (`matrix_irq_sources_into`) instead of allocating a fresh `Vec`
        // per peripheral per poll. Taken out so `self.peripherals` can be
        // borrowed immutably alongside; restored before return.
        let mut scratch = std::mem::take(&mut self.matrix_source_scratch);
        // Prefer the cached scheduler-driver index list (filled in
        // `rebuild_peripheral_ranges`) so a walk-deleted C3 bus does not
        // virtual-dispatch `uses_scheduler` across every peripheral on every
        // MMIO write that re-derives levels.
        if !self.scheduler_driver_indices.is_empty() {
            for &i in &self.scheduler_driver_indices {
                let Some(p) = self.peripherals.get(i) else {
                    continue;
                };
                scratch.clear();
                p.dev.matrix_irq_sources_into(&mut scratch);
                for &src in &scratch {
                    let idx = (src / 64) as usize;
                    if idx < asserted.len() {
                        asserted[idx] |= 1u64 << (src % 64);
                    }
                }
            }
        } else {
            for p in &self.peripherals {
                if !p.dev.uses_scheduler() {
                    continue;
                }
                scratch.clear();
                p.dev.matrix_irq_sources_into(&mut scratch);
                for &src in &scratch {
                    let idx = (src / 64) as usize;
                    if idx < asserted.len() {
                        asserted[idx] |= 1u64 << (src % 64);
                    }
                }
            }
        }
        self.matrix_source_scratch = scratch;
        asserted
    }

    /// ESP32-S3 twin of [`Self::refresh_esp32c3_sched_sources`]: re-derive the
    /// intmatrix sources asserted by scheduler-driven peripherals (the SYSTIMER
    /// alarm once migrated off the walk) into the persistent
    /// `irq_fabric.esp32s3.sched_sources` bitmap. Rebuilt from scratch each call
    /// (level semantics), so a source drops out the poll after firmware writes
    /// INT_CLR. Called from the event path (`deliver_scheduled_irq_levels`,
    /// exact-cycle delivery) and the walk-tick aggregation (steady-state
    /// persistence + de-assert). No-op unless the S3 intmatrix is registered.
    pub(crate) fn refresh_esp32s3_sched_sources(&mut self) {
        if !self.irq_fabric.esp32s3.routing {
            return;
        }
        self.irq_fabric.esp32s3.sched_sources = self.poll_scheduler_matrix_sources();
    }

    /// Rebuild the ESP32-S3 routed `pending_cpu_irqs` bitmap (per core) and the
    /// intmatrix `INTR_STATUS` mirror from the UNION of the walk-emitted level
    /// sources (`irq_fabric.esp32s3.walk_sources`, rebuilt each walk tick) and the
    /// scheduler-driven levels (`irq_fabric.esp32s3.sched_sources`, re-derived
    /// from `matrix_irq_sources`). The S3 twin of
    /// [`Self::recompute_esp32c3_irq_lines`]: the single aggregation body shared
    /// by the per-tick walk pass (`aggregate_esp32s3_explicit_irqs`) and the
    /// event-path choke (`deliver_scheduled_irq_levels`), so both produce an
    /// identical routed bitmap from identical inputs. esp-hal's
    /// `__level_*_interrupt` reads INTR_STATUS to discover which source fired, so
    /// the mirror must see the same union as the routed bits. No-op unless the S3
    /// intmatrix is registered.
    pub(crate) fn recompute_esp32s3_irq_lines(&mut self) {
        let Some(intmatrix_idx) = self.irq_fabric.esp32s3.intmatrix_idx else {
            return;
        };
        if !self.irq_fabric.esp32s3.routing {
            return;
        }
        let mut routed = [0u32; 2];
        let mut intr_status = [0u32; 4];
        for word in 0..self.irq_fabric.esp32s3.walk_sources.len() {
            let mut bits = self.irq_fabric.esp32s3.walk_sources[word]
                | self.irq_fabric.esp32s3.sched_sources[word];
            while bits != 0 {
                let bit = bits.trailing_zeros();
                let source_id = word as u32 * 64 + bit;
                bits &= !(1u64 << bit);
                // Route each asserting source through BOTH cores' map tables;
                // a source delivers to whichever core(s) bound it (the SMP
                // cross-core IPI relies on this: source 79 → core 0, 80 → core 1).
                if let Some(slot) = self.route_irq_source_to_cpu_irq_core(source_id, 0) {
                    routed[0] |= 1u32 << slot;
                }
                if let Some(slot) = self.route_irq_source_to_cpu_irq_core(source_id, 1) {
                    routed[1] |= 1u32 << slot;
                }
                // Mirror into PRO_INTR_STATUS_REG_n so esp-hal's
                // __level_*_interrupt can discover which source asserted.
                let reg = (source_id / 32) as usize;
                if reg < intr_status.len() {
                    intr_status[reg] |= 1u32 << (source_id & 31);
                }
            }
        }
        self.pending_cpu_irqs = routed;
        if let Some(any) = self.peripherals[intmatrix_idx].dev.as_any_mut() {
            if let Some(matrix) =
                any.downcast_mut::<crate::peripherals::esp32s3::intmatrix::Esp32s3IntMatrix>()
            {
                matrix.set_pending_sources(intr_status);
            }
        }
    }

    /// The ESP32-S3 intmatrix `INTR_STATUS` mirror as the bus last routed it,
    /// or all-zero on a bus with no intmatrix. The second half of the S3
    /// fabric's routed output (the first is `pending_cpu_irqs`), read back so
    /// the audit compares the WHOLE result.
    ///
    /// Read out of the intmatrix's own register file at
    /// `PRO_INTR_STATUS_REG_0..3` (offset 0x18C) rather than by downcasting to
    /// the model: this is the same four words esp-hal's `__level_*_interrupt`
    /// loads to discover which source fired, so the audit checks the bytes the
    /// GUEST would see and not an internal field that happens to back them.
    #[cfg(feature = "event-scheduler")]
    fn esp32s3_intr_status_mirror(&self) -> [u32; 4] {
        /// `PRO_INTR_STATUS_REG_0` offset within the intmatrix bank.
        const INTR_STATUS_BASE: u64 = 0x18C;
        let Some(p) = self
            .irq_fabric
            .esp32s3
            .intmatrix_idx
            .and_then(|idx| self.peripherals.get(idx))
        else {
            return [0; 4];
        };
        let mut out = [0u32; 4];
        for (reg, word) in out.iter_mut().enumerate() {
            let mut bytes = [0u8; 4];
            for (i, b) in bytes.iter_mut().enumerate() {
                *b = p
                    .dev
                    .read(INTR_STATUS_BASE + (reg as u64) * 4 + i as u64)
                    .unwrap_or(0);
            }
            *word = u32::from_le_bytes(bytes);
        }
        out
    }

    /// Compare the S3 routed interrupt state the walk-free path LEFT BEHIND
    /// against the state a full re-poll would produce, at this bus boundary.
    ///
    /// This is the gate on the whole walk-free S3 claim. The fast path above
    /// skips `refresh_esp32s3_sched_sources` + `recompute_esp32s3_irq_lines`
    /// every cycle on the grounds that the write choke
    /// (`sync_esp32s3_irq_write`) and the event path
    /// (`deliver_scheduled_irq_levels`) already re-derived the same answer at
    /// the exact cycle any input moved. Here that is checked rather than
    /// believed: poll every scheduler-driven peripheral, recompute, and record
    /// any disagreement — in the routed per-core slot bitmap, in the
    /// `INTR_STATUS` mirror esp-hal reads back, or in the underlying
    /// scheduler-source bitmap.
    ///
    /// The polled answer is left in place. An audited run therefore reproduces
    /// the pre-optimisation build exactly, which is what makes the audit
    /// non-destructive to the firmware under it — and it is why the audit is a
    /// measurement, not a repair: a divergence is reported, and only reported.
    ///
    /// Costs one `Option` null check per walk-free boundary when not installed.
    #[cfg(feature = "event-scheduler")]
    fn audit_esp32s3_irq_boundary(&mut self) {
        if !self.irq_fabric.esp32s3.routing {
            // Not an S3 bus: count nothing, so a test that audits the wrong
            // machine reports zero boundaries and fails on THAT.
            return;
        }
        let cached_routed = self.pending_cpu_irqs;
        let cached_intr_status = self.esp32s3_intr_status_mirror();
        let cached_sched_sources = self.irq_fabric.esp32s3.sched_sources;

        self.refresh_esp32s3_sched_sources();
        self.recompute_esp32s3_irq_lines();

        let polled_routed = self.pending_cpu_irqs;
        let polled_intr_status = self.esp32s3_intr_status_mirror();
        let polled_sched_sources = self.irq_fabric.esp32s3.sched_sources;
        let cycle = self.current_cycle;

        let Some(audit) = self.esp32s3_irq_audit.as_mut() else {
            return;
        };
        audit.boundaries += 1;
        if polled_routed != [0, 0] {
            audit.boundaries_with_routed_irq += 1;
        }
        if polled_sched_sources != [0, 0] {
            audit.boundaries_with_sched_sources += 1;
            audit.sched_source_union[0] |= polled_sched_sources[0];
            audit.sched_source_union[1] |= polled_sched_sources[1];
        }
        if cached_routed == polled_routed
            && cached_intr_status == polled_intr_status
            && cached_sched_sources == polled_sched_sources
        {
            return;
        }
        audit.divergence_count += 1;
        if audit.divergences.len() < crate::bus::Esp32s3IrqAudit::MAX_RECORDED {
            audit.divergences.push(crate::bus::Esp32s3IrqDivergence {
                cycle,
                cached_routed,
                polled_routed,
                cached_intr_status,
                polled_intr_status,
                cached_sched_sources,
                polled_sched_sources,
            });
        }
    }

    /// The ONE per-fabric choke the event path uses to deliver a scheduler-
    /// driven peripheral's level-sensitive IRQ at its exact firing cycle. Every
    /// MCU family follows the SAME shape (poll `matrix_irq_sources` → fold the
    /// level into the fabric's routed state); this method specialises only where
    /// the interrupt fabric differs, and a new fabric slots in by adding ONE
    /// branch here:
    ///   * ESP32-C3 (RISC-V interrupt matrix) → `irq_fabric.esp32c3.irq_lines`;
    ///   * ESP32-S3 (Xtensa interrupt matrix) → `pending_cpu_irqs` + INTR_STATUS.
    ///
    /// Returns `true` when a matrix fabric handled delivery; `false` on an NVIC
    /// bus (Cortex-M / nRF), where the caller pends the peripheral's explicit
    /// lines through `pend_irq_for_event` instead (the classic ESP32 DPORT
    /// fabric is a documented TODO — see the PR body).
    #[cfg(feature = "event-scheduler")]
    pub(crate) fn deliver_scheduled_irq_levels(&mut self) -> bool {
        if self.irq_fabric.esp32c3.routing {
            self.refresh_esp32c3_sched_sources();
            self.recompute_esp32c3_irq_lines();
            true
        } else if self.irq_fabric.esp32s3.routing {
            self.refresh_esp32s3_sched_sources();
            self.recompute_esp32s3_irq_lines();
            true
        } else {
            false
        }
    }

    /// Classic ESP32 DPORT interrupt-matrix routing (TRM §7).
    ///
    /// Peripherals emit matrix *source* IDs via `explicit_irqs` (e.g. UART0 =
    /// 34). Each core has its own MAP table (`PRO_*` @ 0x104, `APP_*` @ 0x208);
    /// firmware binds sources to CPU IRQ slots per core. Without this, APP_CPU
    /// never sees UART TX-empty (loopTask is pinned to core 1) and
    /// `Serial.println` stays stuck in the driver ring buffer — no `LW_L0_OK`.
    ///
    /// Level-sensitive rebuild each tick (same contract as S3/C3). No-op when
    /// DPORT is absent. Does not touch S3/C3 routing flags.
    fn aggregate_esp32_classic_irqs(&mut self, source_ids: &[u32]) {
        // Skip when a chip interrupt MATRIX already owns the routed CPU-interrupt
        // state — the seam answers this without the classic-ESP32 path naming
        // the two SoCs that could be holding it.
        if self.irq_fabric.matrix_owns_cpu_irqs() {
            return;
        }
        let Some(idx) = self.dport_idx else {
            return;
        };
        let routed = self.peripherals.get(idx).and_then(|p| {
            p.dev
                .as_any()
                .and_then(|a| a.downcast_ref::<crate::peripherals::esp32::dport::Dport>())
                .map(|d| d.route_sources(source_ids))
        });
        if let Some(routed) = routed {
            self.pending_cpu_irqs = routed;
        }
    }

    fn aggregate_esp32s3_explicit_irqs(&mut self, source_ids: &[u32]) {
        // Rebuild the per-core routed pending bitmap as a faithful LEVEL
        // reflection of the sources asserting THIS tick — set while a source
        // asserts, cleared the tick it stops. (Was OR-accumulate + clear only
        // on dispatch + early-return when empty, which LATCHED a stale bit
        // after a level source de-asserted.) A level source like the systimer
        // tick re-emits its ID every tick while INT_RAW is set and stops the
        // tick after firmware writes INT_CLR; with the old latch the source
        // kept re-emitting during the ISR — after dispatch had cleared the
        // routed bit — so a stale bit survived the ISR's INT_CLR and re-fired
        // the tick interrupt the instant the ISR returned, wedging the
        // FreeRTOS SMP scheduler in an endless tick-ISR loop (never returning
        // to the dispatched task). Runs every tick, including empty, so a
        // de-asserting source clears its routed bit.
        // Isolation: this aggregation is ESP32-S3-specific. If no ESP32-S3
        // interrupt matrix is registered, this is some other architecture's
        // bus (ARM/RISC-V/nRF use the NVIC path and never read
        // `pending_cpu_irqs`) — return without touching any state so the
        // model stays fully self-contained and cannot influence other models.
        if self.irq_fabric.esp32s3.intmatrix_idx.is_none() || !self.irq_fabric.esp32s3.routing {
            return;
        }
        // Record the walk-emitted level sources asserting THIS tick (rebuilt
        // from scratch → a de-asserting source drops out at the tick boundary),
        // re-derive the scheduler-driven peripheral levels (the SYSTIMER alarm
        // once migrated off the walk — skipped by the per-cycle walk, so the
        // walk `source_ids` never carry it), then recompute the routed bitmap +
        // INTR_STATUS mirror from the UNION via the shared body that the event
        // path also uses. This is the S3 twin of `aggregate_esp32c3_irqs`.
        let mut asserted = [0u64; 2];
        for &src in source_ids {
            let idx = (src / 64) as usize;
            if idx < asserted.len() {
                asserted[idx] |= 1u64 << (src % 64);
            }
        }
        self.irq_fabric.esp32s3.walk_sources = asserted;
        self.refresh_esp32s3_sched_sources();
        self.recompute_esp32s3_irq_lines();
    }

    /// One DMA source-unit -> destination-unit copy with the STM32H5 GPDMA
    /// data-handling semantics (RM0481 §15): width conversion via PAM
    /// (zero-pad / sign-extend / left- or right-truncate) and the SBX / DBX /
    /// DHX byte / half-word exchanges. Pinned by the DMA_DataHandling HAL
    /// example's expected-result vectors and its on-board run.
    pub(crate) fn dma_copy_unit(
        &mut self,
        src: u64,
        dst: u64,
        t: crate::DmaUnitTransform,
    ) -> crate::SimResult<()> {
        let sw = (t.src_width.max(1) as usize).min(4);
        let dw = (t.dst_width.max(1) as usize).min(4);

        let mut unit = [0u8; 4];
        for (k, b) in unit.iter_mut().enumerate().take(sw) {
            *b = self.read_u8(src + k as u64)?;
        }
        // SBX: exchange the two middle bytes of a word-width source.
        if t.sbx && sw == 4 {
            unit.swap(1, 2);
        }

        let mut out = [0u8; 4];
        if dw >= sw {
            // Narrow -> wide: right-aligned (LSBs hold the source unit);
            // upper bytes zero-padded (PAM=0) or sign-extended (PAM=1).
            out[..sw].copy_from_slice(&unit[..sw]);
            let fill = if t.pam & 1 != 0 && unit[sw - 1] & 0x80 != 0 {
                0xFF
            } else {
                0
            };
            for b in out.iter_mut().take(dw).skip(sw) {
                *b = fill;
            }
        } else {
            // Wide -> narrow: PAM=0 keeps the LSBs (right-aligned,
            // left-truncated); PAM=1 keeps the MSBs (left-aligned,
            // right-truncated).
            let from = if t.pam & 1 != 0 { sw - dw } else { 0 };
            out[..dw].copy_from_slice(&unit[from..from + dw]);
        }
        // DBX: swap bytes within each destination half-word.
        if t.dbx && dw >= 2 {
            out.swap(0, 1);
            if dw == 4 {
                out.swap(2, 3);
            }
        }
        // DHX: swap the half-words of a word-width destination.
        if t.dhx && dw == 4 {
            out.swap(0, 2);
            out.swap(1, 3);
        }

        for (k, b) in out.iter().enumerate().take(dw) {
            self.write_u8(dst + k as u64, *b)?;
        }
        Ok(())
    }

    fn collect_enabled_nvic_interrupts(&self, interrupts: &mut Vec<u32>) {
        if let Some(nvic) = &self.nvic {
            for idx in 0..8 {
                let mask =
                    nvic.iser[idx].load(Ordering::SeqCst) & nvic.ispr[idx].load(Ordering::SeqCst);
                if mask != 0 {
                    for bit in 0..32 {
                        if (mask & (1 << bit)) != 0 {
                            let irq = 16 + (idx as u32 * 32) + bit;
                            interrupts.push(irq);
                        }
                    }
                }
            }
        }
    }

    pub fn tick_peripherals_with_costs(
        &mut self,
    ) -> (Vec<u32>, Vec<PeripheralTickCost>, Vec<DmaRequest>) {
        let (mut interrupts, costs, dma_requests, _dma_signals, explicit_source_ids) =
            self.tick_peripherals_phase1(false);
        // Plan 3: route ESP32-S3 source IDs through the intmatrix and update
        // the pending cpu IRQ bitmap + intmatrix INTR_STATUS mirror.
        self.aggregate_esp32s3_explicit_irqs(&explicit_source_ids);
        self.aggregate_esp32c3_irqs(&explicit_source_ids);
        self.aggregate_esp32_classic_irqs(&explicit_source_ids);
        self.collect_enabled_nvic_interrupts(&mut interrupts);

        (interrupts, costs, dma_requests)
    }

    pub fn tick_peripherals_fully(&mut self) -> (Vec<u32>, Vec<PeripheralTickCost>) {
        self.tick_peripherals_fully_impl(false)
    }

    /// Allocation-free twin of [`Self::tick_peripherals_fully`]: writes the
    /// pending interrupts and per-peripheral costs into caller-owned scratch
    /// buffers (cleared first, then filled) instead of returning fresh `Vec`s.
    /// The per-tick machine hot path (`Machine::commit_advance_boundary`) uses
    /// this with retained scratch so the steady-state SYSTIMER tick allocates
    /// nothing. Behaviour is byte-identical to `tick_peripherals_fully`; the
    /// walk-free fast path below mirrors `tick_peripherals_fully_impl`.
    pub fn tick_peripherals_fully_into(
        &mut self,
        interrupts: &mut Vec<u32>,
        costs: &mut Vec<PeripheralTickCost>,
    ) {
        self.service_motor_models();
        interrupts.clear();
        costs.clear();
        // Walk-free fast path (mirror of `tick_peripherals_fully_impl`): the
        // only per-cycle duty is aggregating enabled+pending NVIC interrupts,
        // pushed directly into the retained buffer (zero alloc after warmup).
        #[cfg(feature = "event-scheduler")]
        if self.per_cycle_tick_is_trivial() {
            // Walk-free S3 differential audit. `None` in every production
            // build; see `audit_esp32s3_irq_boundary`.
            if self.esp32s3_irq_audit.is_some() {
                self.audit_esp32s3_irq_boundary();
            }
            self.collect_enabled_nvic_interrupts(interrupts);
            return;
        }
        let (mut i, mut c) = self.tick_peripherals_fully_impl(false);
        interrupts.append(&mut i);
        costs.append(&mut c);
    }

    /// Advances one peripheral-only tick while deliberately bypassing the
    /// event-scheduler walk deletion.
    ///
    /// This is a specialized compatibility boundary for hardware-oracle
    /// harnesses that freeze a bare CPU and settle autonomous peripherals.
    /// Production machine execution must use [`Self::tick_peripherals_fully`]
    /// through `Machine::advance` instead.
    #[doc(hidden)]
    pub fn tick_peripherals_fully_forced(&mut self) -> (Vec<u32>, Vec<PeripheralTickCost>) {
        self.tick_peripherals_fully_impl(true)
    }

    fn tick_peripherals_fully_impl(
        &mut self,
        force_scheduler_walk: bool,
    ) -> (Vec<u32>, Vec<PeripheralTickCost>) {
        let span = crate::profile::span();
        let out = self.tick_peripherals_fully_inner(force_scheduler_walk);
        crate::profile::record_tick(span);
        out
    }

    fn tick_peripherals_fully_inner(
        &mut self,
        force_scheduler_walk: bool,
    ) -> (Vec<u32>, Vec<PeripheralTickCost>) {
        self.service_motor_models();
        // Walk-free fast path: on a bus whose per-cycle tick has no orchestration
        // work (walk deleted, no bus-tick/GPIO/CAN services, HC-SR04 event-
        // scheduled), the only per-cycle duty left is aggregating enabled+pending
        // NVIC interrupts. Returning here skips the whole phase-1 pass and its
        // allocations. `Vec::new()` does not allocate until pushed, so the
        // no-pending-IRQ case is allocation-free.
        #[cfg(feature = "event-scheduler")]
        if !force_scheduler_walk && self.per_cycle_tick_is_trivial() {
            // Walk-free S3 differential audit. `None` in every production
            // build; see `audit_esp32s3_irq_boundary`.
            if self.esp32s3_irq_audit.is_some() {
                self.audit_esp32s3_irq_boundary();
            }
            let mut interrupts = Vec::new();
            self.collect_enabled_nvic_interrupts(&mut interrupts);
            return (interrupts, Vec::new());
        }
        let (mut interrupts, costs, pending_dma, dma_signals, explicit_source_ids) =
            self.tick_peripherals_phase1(force_scheduler_walk);
        if self.irq_fabric.esp32c3.routing {
            self.aggregate_esp32c3_irqs(&explicit_source_ids);
            return (interrupts, costs);
        }
        // Plan 3: route ESP32-S3 source IDs through the intmatrix.
        self.aggregate_esp32s3_explicit_irqs(&explicit_source_ids);
        self.aggregate_esp32c3_irqs(&explicit_source_ids);
        self.aggregate_esp32_classic_irqs(&explicit_source_ids);

        // Phase 1.5: Route DMA signals
        for (source_name, request_id) in dma_signals {
            self.route_dma_signal(&source_name, request_id);
        }

        // Phase 2: Execute DMA requests (this now has access to self.flash/ram via write_u8)
        for req in pending_dma {
            match req.direction {
                crate::DmaDirection::Read => {
                    if let Ok(val) = self.read_u8(req.addr) {
                        tracing::trace!("DMA Read: {:#x} -> {:#x}", req.addr, val);
                    }
                }
                crate::DmaDirection::Write => {
                    let _ = self.write_u8(req.addr, req.val);
                    tracing::trace!("DMA Write: {:#x} <- {:#x}", req.addr, req.val);
                }
                crate::DmaDirection::Copy => {
                    if let Some(t) = req.transform {
                        let _ = self.dma_copy_unit(req.src_addr, req.addr, t);
                    } else if let Ok(val) = self.read_u8(req.src_addr) {
                        let _ = self.write_u8(req.addr, val);
                        tracing::trace!(
                            "DMA Copy: {:#x} -> {:#x} ({:#x})",
                            req.src_addr,
                            req.addr,
                            val
                        );
                    }
                }
            }
        }

        // Phase 3: Scan NVIC
        self.collect_enabled_nvic_interrupts(&mut interrupts);

        (interrupts, costs)
    }
}

#[cfg(test)]
#[path = "tick_forced_oracle_walk_tests.rs"]
mod forced_oracle_walk_tests;

#[cfg(test)]
#[path = "tick_nrf_spim_gpio_cs_scheduling_tests.rs"]
mod nrf_spim_gpio_cs_scheduling_tests;

#[cfg(test)]
#[path = "tick_walk_free_campaign.rs"]
mod walk_free_campaign;

/// Walk-free C3 SYSTIMER batch — the interrupt-matrix ROUTING identity.
///
/// A SYSTIMER migrated off the per-cycle walk delivers its alarm as a scheduled
/// event; the C3 routing arm (`Machine::apply_event_result` → this module's
/// `refresh_esp32c3_sched_sources` + `recompute_esp32c3_irq_lines`) must route
/// that level to `irq_fabric.esp32c3.irq_lines` EXACTLY as the legacy walk did when the
/// SYSTIMER re-emitted source 37 every tick (`aggregate_esp32c3_irqs`). This
/// pins that equivalence at the bus level (the OLED-lab gate proves it
/// end-to-end through the real FreeRTOS tick).
#[cfg(all(test, feature = "event-scheduler"))]
#[path = "tick_c3_systimer_matrix_routing.rs"]
mod c3_systimer_matrix_routing;

/// Walk-free C3 level-only peripheral batch (`spi2` + `apb_saradc`) — the
/// interrupt-matrix ROUTING identity.
///
/// Both are one-shot LEVEL re-emitters: `int_raw` is write-armed by a
/// transaction / conversion (no free-running counter), and the model re-asserts
/// its matrix source while `int_raw & int_ena != 0`. On the legacy walk that
/// level is re-emitted every tick via `tick()`'s `explicit_irqs`; migrated off
/// the walk it is exported through `matrix_irq_sources` and re-derived by the
/// bus (`refresh_esp32c3_sched_sources` inside `aggregate_esp32c3_irqs`). This
/// gate proves the two paths deliver the routed CPU line IDENTICALLY — the
/// end-to-end fidelity contract the OLED differential can't exercise (spi2 /
/// apb_saradc never fire in that lab). Same trigger, armed on two buses; one
/// left scheduler-driven, one pinned back to the walk with `force_legacy_walk`.
#[cfg(all(test, feature = "event-scheduler"))]
#[path = "tick_c3_level_peripheral_matrix_routing.rs"]
mod c3_level_peripheral_matrix_routing;

/// Walk-free C3 LEDC timer-port batch — the interrupt-matrix ROUTING identity.
///
/// Unlike the level-only pair (`spi2`/`apb_saradc`, write-armed), LEDC is a
/// TIME-driven pinner: its four low-speed timers advance as up-counters and
/// latch `LSTIMERx_OVF` on wrap. Migrated off the walk, that overflow is
/// materialised by a scheduled event and the level is exported through
/// `matrix_irq_sources` (re-derived by the bus); on the legacy walk the same
/// level is re-emitted every tick via `tick()`'s `explicit_irqs`. This gate
/// proves the two paths route the LEDC OVF interrupt to the SAME CPU line, and
/// that INT_CLR de-asserts it — the same bus-level equivalence
/// `c3_systimer_matrix_routing` pins for the SYSTIMER (the OLED lab can't
/// exercise it: the demo never configures LEDC).
#[cfg(all(test, feature = "event-scheduler"))]
#[path = "tick_c3_ledc_matrix_routing.rs"]
mod c3_ledc_matrix_routing;

/// Walk-free C3 WiFi-MAC batch — the LAST walk pinner on the OLED rom-boot bus.
///
/// `wifi_mac` pinned the walk on TWO axes; both are migrated here with NO new
/// event machinery:
///
///   * the interrupt LEVEL (matrix source 0, asserted while a MAC event is
///     pending) — was re-emitted every walk tick by `tick()`; now exported
///     through `matrix_irq_sources` and re-derived by the bus. On a walk-DELETED
///     bus there is no walk tick to re-derive it, so a write-armed level change
///     (the `EVENT_CLR` acknowledge) is re-routed at the MMIO WRITE CHOKE
///     (`sync_esp32c3_irq_cache_write`). This module proves both: the scheduler
///     and walk paths route the MAC level to the SAME CPU line, and the
///     write-choke de-asserts it on a fully walk-deleted bus.
///
///   * the descriptor-ring PUMP (`tick_with_bus`) — was pinned by an
///     unconditional `needs_bus_tick() == true`; now honestly gated so it only
///     ticks while WiFi is up (`rx_ring != 0` / a pending TX / medium mode). The
///     companion `c3_wifi_mac_walk_differential` module drives a real TX + RX
///     session and proves the pump's ring writeback + IRQ delivery are
///     byte-identical walk-vs-scheduler at interval 1 AND 64.
#[cfg(all(test, feature = "event-scheduler"))]
#[path = "tick_c3_wifi_mac_matrix_routing.rs"]
mod c3_wifi_mac_matrix_routing;

/// Walk-free C3 WiFi-MAC PUMP fidelity — the WiFi-EXERCISING byte-identity
/// differential. Drives a real descriptor-ring session (RX ring delivery + a TX
/// kick) on a scheduler-driven MAC and a walk-driven MAC, at tick interval 1 and
/// 64, and asserts every observable (descriptor writeback, MAC event word,
/// captured TX frame, routed CPU line) is BYTE-IDENTICAL. Non-vacuity: the pump
/// must actually move frames (RX delivered + TX captured > 0). The periodic
/// beacon rides the very same `tick_with_bus` code (medium mode keeps the MAC
/// resident every tick in both drive modes), so it is byte-identical by
/// construction; it is exercised by the two-C3 CLI runs, not here (the medium is
/// a process-global static that must not be touched from parallel unit tests).
#[cfg(all(test, feature = "event-scheduler"))]
#[path = "tick_c3_wifi_mac_walk_differential.rs"]
mod c3_wifi_mac_walk_differential;

/// The classic-ESP32 DPORT fabric and the two matrix fabrics all write the same
/// routed output, [`SystemBus::pending_cpu_irqs`]. Exactly one of them may own
/// it on a given bus, and the shared aggregation asks
/// [`InterruptFabric::matrix_owns_cpu_irqs`] — not two chip flags — which.
///
/// This is the gate on that predicate. A version that only consulted the C3
/// flag would still pass the C3 leg and fail the S3 one, which is the whole
/// point of asking the seam instead of naming a chip at the call site.
#[cfg(test)]
#[path = "tick_classic_dport_defers_to_a_matrix_fabric.rs"]
mod classic_dport_defers_to_a_matrix_fabric;
