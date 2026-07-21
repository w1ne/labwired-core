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
    #[cfg(feature = "event-scheduler")]
    #[inline]
    pub(crate) fn collect_scheduled_events(&mut self, idx: usize) {
        if !self.peripherals[idx].dev.uses_scheduler() {
            return;
        }
        for (delay, token) in self.peripherals[idx].dev.take_scheduled_events() {
            self.pending_schedule
                .push((idx, self.current_cycle + 1 + delay, token));
        }
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

            if res.irq {
                if let Some(irq) = irq {
                    pend_nvic(&self.nvic, &mut interrupts, irq);
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

        if !self.esp32c3_irq_routing {
            // GPIO edge-detection pass: snapshot the IN registers of GPIO ports
            // 0 and 1, diff against last-known state, and notify every
            // peripheral of changed pins. GPIOTE overrides observe_gpio_change
            // to drive EVENTS_IN[i] when a channel watches a matching pin.
            //
            // ESP32-C3 does not use this Nordic GPIO/GPIOTE service path; its
            // board inputs write the C3 GPIO register model directly. Skipping
            // this block is important because C3 ROM-boot needs very frequent
            // ticks for interrupt-matrix correctness.
            let gpio_bases: [Option<u64>; 2] = [
                self.find_peripheral_index_by_name("gpio0")
                    .map(|i| self.peripherals[i].base),
                self.find_peripheral_index_by_name("gpio1")
                    .map(|i| self.peripherals[i].base),
            ];
            let mut changes: Vec<(u8, u8, u8)> = Vec::new();
            let mut current_in = self.last_gpio_in;
            for (port, base) in gpio_bases.iter().enumerate() {
                let Some(base) = base else { continue };
                // GPIO IN register is at offset 0x510 in the Nordic layout.
                let cur = self.read_u32(*base + 0x510).unwrap_or(0);
                let prev = self.last_gpio_in[port];
                let diff = cur ^ prev;
                if diff != 0 {
                    for pin in 0..32u8 {
                        if diff & (1 << pin) != 0 {
                            let level = ((cur >> pin) & 1) as u8;
                            changes.push((port as u8, pin, level));
                        }
                    }
                }
                current_in[port] = cur;
            }
            self.last_gpio_in = current_in;
            if !changes.is_empty() {
                for p in self.peripherals.iter_mut() {
                    p.dev.observe_gpio_change(&changes);
                }
            }

            // HC-SR04, DHT22 and CAN synthetic services are not present on C3
            // ROM-boot labs; keep them off the C3 high-frequency tick path.
            //
            // When the sensor is event-scheduled, its ECHO edges are driven by
            // `Machine::drain_scheduler_events` at their exact cycles instead —
            // skip the per-cycle pass so the two paths don't both drive the pad
            // (and so a walk-free bus can early-out of the tick entirely).
            if !self.hcsr04_event_scheduled() {
                self.service_hcsr04();
            }
            self.service_gpio_devices();
            self.service_can_diagnostic_testers();
            self.service_can_uds_testers();
            self.service_can_log_players();
        }

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
    /// `riscv_irq_lines`, which the core ORs into `mip`. No-op unless
    /// `esp32c3_irq_routing` is set (only the C3 rom-boot path sets it).
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
        if !self.esp32c3_irq_routing {
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
        self.esp32c3_asserted_sources = asserted;
        // Re-derive scheduler-driven peripheral levels (SYSTIMER once migrated
        // off the walk) so their level-sensitive matrix IRQ persists across
        // walk ticks and de-asserts the tick after firmware clears it.
        self.refresh_esp32c3_sched_sources();

        if self.esp32c3_irq_cache.is_some() {
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
            bus.esp32c3_interrupt_core0_idx
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
        let sched = self.esp32c3_sched_asserted_sources;
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
                .esp32c3_system_idx
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
        self.riscv_irq_lines = mask;
    }

    /// Rebuild `riscv_irq_lines` from the cached C3 routing state: the INTC
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
        let Some(cache) = &self.esp32c3_irq_cache else {
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

        for word in 0..self.esp32c3_asserted_sources.len() {
            // Union of walk-emitted level sources (rebuilt each tick) and
            // scheduler-driven peripheral level sources (re-derived from
            // `matrix_irq_sources`), so a SYSTIMER migrated off the walk keeps
            // its level-sensitive alarm IRQ routed.
            let mut bits =
                self.esp32c3_asserted_sources[word] | self.esp32c3_sched_asserted_sources[word];
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
        self.riscv_irq_lines = mask;
    }

    /// Re-derive the C3 matrix sources asserted by SCHEDULER-driven peripherals
    /// (skipped by the per-cycle walk) from their live level
    /// (`Peripheral::matrix_irq_sources`). Rebuilt from scratch — level
    /// semantics — so a source that stops asserting (e.g. after the SYSTIMER
    /// alarm's INT_CLR) drops out on the next re-derivation. Called from the
    /// event path (`Machine::apply_event_result`, exact-cycle delivery) and the
    /// walk-tick aggregation (steady-state persistence + de-assert). No-op
    /// unless C3 routing is active. Does NOT recompute — the caller decides
    /// when to fold this into `riscv_irq_lines`.
    pub(crate) fn refresh_esp32c3_sched_sources(&mut self) {
        if !self.esp32c3_irq_routing {
            return;
        }
        self.esp32c3_sched_asserted_sources = self.poll_scheduler_matrix_sources();
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
    /// `esp32s3_sched_asserted_sources` bitmap. Rebuilt from scratch each call
    /// (level semantics), so a source drops out the poll after firmware writes
    /// INT_CLR. Called from the event path (`deliver_scheduled_irq_levels`,
    /// exact-cycle delivery) and the walk-tick aggregation (steady-state
    /// persistence + de-assert). No-op unless the S3 intmatrix is registered.
    pub(crate) fn refresh_esp32s3_sched_sources(&mut self) {
        if !self.esp32s3_irq_routing {
            return;
        }
        self.esp32s3_sched_asserted_sources = self.poll_scheduler_matrix_sources();
    }

    /// Rebuild the ESP32-S3 routed `pending_cpu_irqs` bitmap (per core) and the
    /// intmatrix `INTR_STATUS` mirror from the UNION of the walk-emitted level
    /// sources (`esp32s3_asserted_sources`, rebuilt each walk tick) and the
    /// scheduler-driven levels (`esp32s3_sched_asserted_sources`, re-derived
    /// from `matrix_irq_sources`). The S3 twin of
    /// [`Self::recompute_esp32c3_irq_lines`]: the single aggregation body shared
    /// by the per-tick walk pass (`aggregate_esp32s3_explicit_irqs`) and the
    /// event-path choke (`deliver_scheduled_irq_levels`), so both produce an
    /// identical routed bitmap from identical inputs. esp-hal's
    /// `__level_*_interrupt` reads INTR_STATUS to discover which source fired, so
    /// the mirror must see the same union as the routed bits. No-op unless the S3
    /// intmatrix is registered.
    pub(crate) fn recompute_esp32s3_irq_lines(&mut self) {
        let Some(intmatrix_idx) = self.esp32s3_intmatrix_idx else {
            return;
        };
        if !self.esp32s3_irq_routing {
            return;
        }
        let mut routed = [0u32; 2];
        let mut intr_status = [0u32; 4];
        for word in 0..self.esp32s3_asserted_sources.len() {
            let mut bits =
                self.esp32s3_asserted_sources[word] | self.esp32s3_sched_asserted_sources[word];
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

    /// The ONE per-fabric choke the event path uses to deliver a scheduler-
    /// driven peripheral's level-sensitive IRQ at its exact firing cycle. Every
    /// MCU family follows the SAME shape (poll `matrix_irq_sources` → fold the
    /// level into the fabric's routed state); this method specialises only where
    /// the interrupt fabric differs, and a new fabric slots in by adding ONE
    /// branch here:
    ///   * ESP32-C3 (RISC-V interrupt matrix) → `riscv_irq_lines`;
    ///   * ESP32-S3 (Xtensa interrupt matrix) → `pending_cpu_irqs` + INTR_STATUS.
    ///
    /// Returns `true` when a matrix fabric handled delivery; `false` on an NVIC
    /// bus (Cortex-M / nRF), where the caller pends the peripheral's explicit
    /// lines through `pend_irq_for_event` instead (the classic ESP32 DPORT
    /// fabric is a documented TODO — see the PR body).
    #[cfg(feature = "event-scheduler")]
    pub(crate) fn deliver_scheduled_irq_levels(&mut self) -> bool {
        if self.esp32c3_irq_routing {
            self.refresh_esp32c3_sched_sources();
            self.recompute_esp32c3_irq_lines();
            true
        } else if self.esp32s3_irq_routing {
            self.refresh_esp32s3_sched_sources();
            self.recompute_esp32s3_irq_lines();
            true
        } else {
            false
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
        if self.esp32s3_intmatrix_idx.is_none() || !self.esp32s3_irq_routing {
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
        self.esp32s3_asserted_sources = asserted;
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
        interrupts.clear();
        costs.clear();
        // Walk-free fast path (mirror of `tick_peripherals_fully_impl`): the
        // only per-cycle duty is aggregating enabled+pending NVIC interrupts,
        // pushed directly into the retained buffer (zero alloc after warmup).
        #[cfg(feature = "event-scheduler")]
        if self.per_cycle_tick_is_trivial() {
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
        // Walk-free fast path: on a bus whose per-cycle tick has no orchestration
        // work (walk deleted, no bus-tick/GPIO/CAN services, HC-SR04 event-
        // scheduled), the only per-cycle duty left is aggregating enabled+pending
        // NVIC interrupts. Returning here skips the whole phase-1 pass and its
        // allocations. `Vec::new()` does not allocate until pushed, so the
        // no-pending-IRQ case is allocation-free.
        #[cfg(feature = "event-scheduler")]
        if !force_scheduler_walk && self.per_cycle_tick_is_trivial() {
            let mut interrupts = Vec::new();
            self.collect_enabled_nvic_interrupts(&mut interrupts);
            return (interrupts, Vec::new());
        }
        let (mut interrupts, costs, pending_dma, dma_signals, explicit_source_ids) =
            self.tick_peripherals_phase1(force_scheduler_walk);
        if self.esp32c3_irq_routing {
            self.aggregate_esp32c3_irqs(&explicit_source_ids);
            return (interrupts, costs);
        }
        // Plan 3: route ESP32-S3 source IDs through the intmatrix.
        self.aggregate_esp32s3_explicit_irqs(&explicit_source_ids);
        self.aggregate_esp32c3_irqs(&explicit_source_ids);

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
mod forced_oracle_walk_tests {
    use super::SystemBus;
    use crate::{Peripheral, PeripheralTickResult, SimResult};
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Arc, Mutex,
    };

    #[derive(Debug)]
    struct OrderedTick {
        value: u32,
        scheduler: bool,
        order: Arc<Mutex<Vec<u32>>>,
    }

    impl Peripheral for OrderedTick {
        fn read(&self, _offset: u64) -> SimResult<u8> {
            Ok(0)
        }

        fn write(&mut self, _offset: u64, _value: u8) -> SimResult<()> {
            Ok(())
        }

        fn tick(&mut self) -> PeripheralTickResult {
            self.order.lock().unwrap().push(self.value);
            PeripheralTickResult::default()
        }

        fn uses_scheduler(&self) -> bool {
            self.scheduler
        }
    }

    #[derive(Debug)]
    struct OneShotDynamic {
        active: bool,
        ticks: Arc<AtomicUsize>,
    }

    impl Peripheral for OneShotDynamic {
        fn read(&self, _offset: u64) -> SimResult<u8> {
            Ok(0)
        }

        fn write(&mut self, _offset: u64, _value: u8) -> SimResult<()> {
            Ok(())
        }

        fn tick(&mut self) -> PeripheralTickResult {
            self.ticks.fetch_add(1, Ordering::SeqCst);
            self.active = false;
            PeripheralTickResult::default()
        }

        fn legacy_tick_active(&self) -> bool {
            self.active
        }

        fn legacy_tick_dynamic(&self) -> bool {
            true
        }
    }

    #[test]
    fn forced_walk_preserves_registration_order_across_drive_modes() {
        let order = Arc::new(Mutex::new(Vec::new()));
        let mut bus = SystemBus::empty();
        bus.add_peripheral(
            "scheduler_first",
            0x1000,
            0x100,
            None,
            Box::new(OrderedTick {
                value: 1,
                scheduler: true,
                order: order.clone(),
            }),
        );
        bus.add_peripheral(
            "legacy_second",
            0x2000,
            0x100,
            None,
            Box::new(OrderedTick {
                value: 2,
                scheduler: false,
                order: order.clone(),
            }),
        );

        bus.tick_peripherals_fully_forced();

        assert_eq!(*order.lock().unwrap(), vec![1, 2]);
    }

    #[test]
    fn forced_walk_advances_past_dynamic_entry_that_turns_inactive() {
        let ticks = Arc::new(AtomicUsize::new(0));
        let mut bus = SystemBus::empty();
        bus.add_peripheral(
            "one_shot",
            0x1000,
            0x100,
            None,
            Box::new(OneShotDynamic {
                active: true,
                ticks: ticks.clone(),
            }),
        );

        bus.tick_peripherals_fully_forced();

        assert_eq!(ticks.load(Ordering::SeqCst), 1);
    }
}

#[cfg(test)]
mod walk_free_campaign {
    //! Pins the walk-free STM32 campaign's *remaining surface* on the L476
    //! nokia5110-invaders bus as it is actually executed (`from_config` +
    //! `configure_cortex_m`, exactly how every e2e/capture harness builds it).
    //! The bus is built with any hand `walk_deleted` flag stripped, so the
    //! assertion reflects only what the models themselves prove — not a manifest
    //! override.
    //!
    //! The walk-forcing set is `needs_legacy_walk() && !uses_scheduler()` — the
    //! exact predicate `derive_walk_deletable` negates (see this module's parent).
    //! uart×6 / spi×3 are already event-migrated (`uses_scheduler()==true`) so
    //! they carry the default `needs_legacy_walk()==true` but do NOT force the
    //! walk; they are correctly excluded here.
    //!
    //! Why `configure_cortex_m`: the Cortex-M core installs the *real* SCB and
    //! NVIC (the chip descriptor only carries inert placeholders for those ids)
    //! and appends DWT (CYCCNT), which is not in the descriptor at all. In a
    //! featureless build SCB's `tick()` drains software-pended exceptions and
    //! DWT's advances CYCCNT — both real walk work. Omitting this step would hide
    //! SCB's real tick and DWT entirely and under-report the surface, so the
    //! runtime-faithful bus is the honest one to pin. (Under `event-scheduler`,
    //! `configure_cortex_m` attaches the bus cycle clock to DWT, migrating it to
    //! the lazy-read scheduler path — see the cfg-split lists below.)
    //!
    //! After batch **B0** (Class-A inert sweep) the forcing set is the plan's 21
    //! Class-B instances still awaiting scheduler migration, PLUS the core DWT
    //! (a lazy-read CYCCNT counter the plan calls out separately as the purest
    //! read-sync case). Each later batch (SysTick+SCB, timers, DMA, the DWT
    //! lazy-CYCCNT migration, …) moves a slice onto the scheduler, flipping its
    //! `uses_scheduler()`, so this expected set shrinks batch by batch. Keep it
    //! in lockstep with the plan's inventory.
    //!
    //! Batch **B1** (SysTick + SCB → scheduler) removes `systick` and `scb`
    //! from the forcing set — but only in `event-scheduler` builds:
    //! `uses_scheduler()` needs both the feature AND the bus-attached
    //! [`crate::CycleClock`], so the featureless lane honestly keeps them on
    //! the walk (the two lists below are cfg-split for exactly that reason).

    use crate::bus::SystemBus;
    use crate::system::cortex_m::configure_cortex_m;
    use labwired_config::{ChipDescriptor, SystemManifest};
    use std::path::PathBuf;

    /// Walk-forcing ids on the runtime invaders bus after the I2C migration
    /// (event-scheduler builds): the prior 6 (B2/B3 timers minus the 2 `dma*`,
    /// with the DWT lazy-CYCCNT migration from #522) minus the 3 `i2c*` = 2.
    /// The 3 `i2c*` NOW migrate: the STM32 F1/L4 transaction engine is self-paced
    /// by the SAME held-level, delay-1 self-perpetuating event chain the Kinetis
    /// variant uses. The chain runs `F1I2c::tick()`/`L4I2c::tick()` every cycle
    /// while a transfer is *active* (countdown in flight OR SR2.BUSY set), so the
    /// `&self`-read side effects (`rxne_consumed` / device byte pulls) are
    /// observed by the already-live chain's next `on_event` exactly as the walk's
    /// next `tick()` would — no event needs arming from the read path. Proven
    /// byte-identical (registers + read bytes + NVIC-pend cycles) by the
    /// `kinetis_scheduler` differential module (`f1_*`/`l4_*` cases). Remaining
    /// plan Class-B on this bus: adc + exti.
    ///
    /// bxCAN (`can1`) NO LONGER forces the walk: its `tick()` only drains a
    /// `CanBus` mpsc interconnect, so `needs_legacy_walk()` now reports
    /// `bus_rx.is_some()` — false on this bus (no multi-node CanBus is wired;
    /// can-player replay is pushed by `service_can_log_players`, not the tick).
    ///
    /// EXTI (`exti`) NO LONGER forces the walk: its held-level `explicit_irqs`
    /// re-emission is driven by a delay-1 self-perpetuating event chain (armed on
    /// the MMIO write that raises a masked pending line, stopping when firmware
    /// clears PR). Byte-identical proof: `exti::scheduler_diff`.
    ///
    /// ADC (`adc1`) NO LONGER forces the walk: its F1 conversion countdown is
    /// event-scheduled (delay-1 chain armed on SWSTART, perpetuating through
    /// continuous mode), and the legacy `cycles: 1` per-converting-tick cost is
    /// normalized to zero in BOTH modes (SysTick B1 pattern) so `total_cycles`
    /// agrees. Byte-identical proof: `adc::scheduler_diff`.
    ///
    /// The forcing set is now EMPTY — with every Class-B walker migrated the
    /// runtime invaders (L476) bus derives walk-deletion with no hand flag: the
    /// campaign's full STM32 board flip.
    #[cfg(feature = "event-scheduler")]
    const EXPECTED_WALK_FORCING: &[&str] = &[];

    /// Featureless builds: the scheduler does not exist, so SysTick and SCB
    /// stay on the legacy walk. bxCAN (`can1`) is excluded regardless of the
    /// feature — its walk-forcing is gated on an attached interconnect, not the
    /// scheduler.
    #[cfg(not(feature = "event-scheduler"))]
    const EXPECTED_WALK_FORCING: &[&str] = &[
        "systick", "tim1", "tim2", "tim3", "tim4", "tim5", "tim6", "tim7", "tim8", "tim15",
        "tim16", "tim17", "dma1", "dma2", "i2c1", "i2c2", "i2c3", "adc1", "exti", "scb", "dwt",
    ];

    fn invaders_bus_walk_stripped() -> SystemBus {
        let system_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../examples/nokia5110-invaders-lab/system.yaml");
        let mut manifest = SystemManifest::from_file(&system_path).expect("load invaders manifest");
        // Construct WITHOUT the lab's hand `walk_deleted: true`: the campaign
        // surface must come from the models, not the manifest escape hatch.
        manifest.walk_deleted = None;
        let chip_path = system_path.parent().unwrap().join(&manifest.chip);
        let chip = ChipDescriptor::from_file(&chip_path).expect("load l476 chip");
        let mut bus = SystemBus::from_config(&chip, &manifest).expect("build invaders bus");
        // Install the real SCB/NVIC and the core DWT — the executed bus.
        let _ = configure_cortex_m(&mut bus);
        bus
    }

    #[test]
    fn remaining_walk_forcing_set_matches_campaign_inventory() {
        let bus = invaders_bus_walk_stripped();

        let mut forcing: Vec<&str> = bus
            .peripherals
            .iter()
            .filter(|p| p.dev.needs_legacy_walk() && !p.dev.uses_scheduler())
            .map(|p| p.name.as_str())
            .collect();
        forcing.sort_unstable();

        let mut expected: Vec<&str> = EXPECTED_WALK_FORCING.to_vec();
        expected.sort_unstable();

        assert_eq!(
            forcing,
            expected,
            "walk-forcing set (needs_legacy_walk && !uses_scheduler) drifted from the \
             campaign inventory (currently: post-B1).\n  got ({}):      {:?}\n  expected ({}): {:?}\n\
             A model newly (un)marked `needs_legacy_walk()` or migrated to the scheduler \
             must update EXPECTED_WALK_FORCING to match the campaign's remaining surface.",
            forcing.len(),
            forcing,
            expected.len(),
            expected,
        );
    }

    #[test]
    fn invaders_bus_now_flips_with_every_class_b_migrated() {
        // With I2C, EXTI and ADC all event-scheduled, the runtime invaders
        // (L476) bus has an EMPTY walk-forcing set and derives walk-deletion
        // with no hand `walk_deleted` flag — the campaign's full STM32 board
        // flip. (bxCAN never forced it here — its walk work is gated on an
        // attached CanBus interconnect, absent on this bus.)
        let bus = invaders_bus_walk_stripped();
        #[cfg(feature = "event-scheduler")]
        assert!(
            bus.derive_walk_deletable(),
            "invaders bus should be walk-deletable once every Class-B walker \
             (i2c/exti/adc) is migrated and the forcing set is empty"
        );
        // Featureless builds have no scheduler, so the migrated models honestly
        // stay on the walk and the bus does NOT flip.
        #[cfg(not(feature = "event-scheduler"))]
        assert!(
            !bus.derive_walk_deletable(),
            "featureless build keeps the walk (no scheduler to migrate onto)"
        );
    }
}

/// Walk-free C3 SYSTIMER batch — the interrupt-matrix ROUTING identity.
///
/// A SYSTIMER migrated off the per-cycle walk delivers its alarm as a scheduled
/// event; the C3 routing arm (`Machine::apply_event_result` → this module's
/// `refresh_esp32c3_sched_sources` + `recompute_esp32c3_irq_lines`) must route
/// that level to `riscv_irq_lines` EXACTLY as the legacy walk did when the
/// SYSTIMER re-emitted source 37 every tick (`aggregate_esp32c3_irqs`). This
/// pins that equivalence at the bus level (the OLED-lab gate proves it
/// end-to-end through the real FreeRTOS tick).
#[cfg(all(test, feature = "event-scheduler"))]
mod c3_systimer_matrix_routing {
    use crate::bus::SystemBus;
    use crate::peripherals::esp32s3::systimer::Systimer;
    use crate::Bus;
    use crate::Peripheral;
    use labwired_config::{ChipDescriptor, SystemManifest};
    use std::path::PathBuf;

    const SYSTIMER_BASE: u64 = 0x6002_3000;
    const INTMATRIX: u64 = 0x600C_2000;
    const SYSTIMER_TARGET0_SOURCE: u64 = 37;
    const LINE: u32 = 5;

    /// Build a devkit C3 bus (real `interrupt_core0` → INTC cache), enable C3
    /// routing, swap the declarative SYSTIMER stub for a real scheduler-driven
    /// `Systimer`, and route SYSTIMER_TARGET0 (source 37) → CPU line 5.
    fn setup() -> SystemBus {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let chip = ChipDescriptor::from_file(root.join("../../configs/chips/esp32c3.yaml"))
            .expect("load esp32c3 chip yaml");
        let manifest =
            SystemManifest::from_file(root.join("../../configs/systems/esp32c3-devkit.yaml"))
                .expect("load esp32c3-devkit system yaml");
        let mut bus = SystemBus::from_config(&chip, &manifest).expect("build c3 devkit bus");

        // Enable the RISC-V interrupt routing (the ROM-boot path sets this; the
        // from_config bus does not) and rebuild the INTC cache.
        bus.esp32c3_irq_routing = true;
        bus.refresh_peripheral_index();

        // Swap the declarative SYSTIMER stub for the real scheduler model and
        // hand it the bus clock (as `add_peripheral` would).
        let idx = bus
            .find_peripheral_index_by_name("systimer")
            .expect("devkit bus carries a systimer");
        let mut dev = Systimer::new_with_source(160_000_000, SYSTIMER_TARGET0_SOURCE as u32);
        dev.attach_cycle_clock(bus.cycle_clock.clone());
        bus.peripherals[idx].dev = Box::new(dev);
        bus.refresh_peripheral_index();

        // Route source 37 → line 5, priority 1, threshold 1, line enabled.
        bus.write_u32(INTMATRIX + SYSTIMER_TARGET0_SOURCE * 4, LINE)
            .unwrap();
        bus.write_u32(INTMATRIX + 0x114 + (LINE as u64) * 4, 1)
            .unwrap();
        bus.write_u32(INTMATRIX + 0x194, 1).unwrap();
        bus.write_u32(INTMATRIX + 0x104, 1 << LINE).unwrap();

        // Arm TARGET0 in target mode at 3 SYSTIMER ticks, enable its IRQ.
        bus.write_u32(SYSTIMER_BASE + 0x64, 1).unwrap(); // INT_ENA bit0
        bus.write_u32(SYSTIMER_BASE + 0x1C, 0).unwrap(); // TARGET0_HI
        bus.write_u32(SYSTIMER_BASE + 0x20, 3).unwrap(); // TARGET0_LO
        bus.write_u32(SYSTIMER_BASE + 0x50, 1).unwrap(); // COMP0_LOAD
        let conf = bus.read_u32(SYSTIMER_BASE).unwrap();
        bus.write_u32(SYSTIMER_BASE, conf | (1 << 24)).unwrap(); // TARGET0_WORK_EN
        bus
    }

    fn systimer_mut(bus: &mut SystemBus) -> &mut Systimer {
        let idx = bus.find_peripheral_index_by_name("systimer").unwrap();
        bus.peripherals[idx]
            .dev
            .as_any_mut()
            .unwrap()
            .downcast_mut::<Systimer>()
            .unwrap()
    }

    /// The scheduler routing arm and the legacy walk aggregation produce the
    /// SAME `riscv_irq_lines` for the SAME SYSTIMER level.
    #[test]
    fn scheduler_routing_matches_walk_routing_for_same_level() {
        let mut bus = setup();

        // Advance the SYSTIMER past the target so the alarm latches
        // pending && int_ena → the model asserts matrix source 37.
        systimer_mut(&mut bus).sync_to(10_000);
        assert_eq!(
            systimer_mut(&mut bus).matrix_irq_sources(),
            vec![SYSTIMER_TARGET0_SOURCE as u32],
            "armed+fired SYSTIMER must assert matrix source 37"
        );

        // Scheduler routing arm (what `apply_event_result` runs on the C3 bus).
        bus.refresh_esp32c3_sched_sources();
        bus.recompute_esp32c3_irq_lines();
        let scheduler_lines = bus.riscv_irq_lines;
        assert_eq!(
            scheduler_lines,
            1 << LINE,
            "scheduler routing must assert the routed CPU line for source 37"
        );

        // Legacy walk routing reference: source 37 re-emitted by the walk. Must
        // land on the identical line mask.
        bus.aggregate_esp32c3_irqs(&[SYSTIMER_TARGET0_SOURCE as u32]);
        assert_eq!(
            bus.riscv_irq_lines, scheduler_lines,
            "walk aggregation and scheduler routing must produce identical riscv_irq_lines"
        );
    }

    /// Clearing the SYSTIMER level (INT_CLR) de-asserts the routed line on the
    /// next re-derivation — same level semantics as the walk (which stops
    /// re-emitting the source the tick after INT_CLR).
    #[test]
    fn clearing_level_deasserts_routed_line() {
        let mut bus = setup();
        systimer_mut(&mut bus).sync_to(10_000);
        bus.refresh_esp32c3_sched_sources();
        bus.recompute_esp32c3_irq_lines();
        assert_eq!(bus.riscv_irq_lines, 1 << LINE);

        // INT_CLR bit0 clears the pending latch → level drops.
        bus.write_u32(SYSTIMER_BASE + 0x6C, 1).unwrap();
        assert!(
            systimer_mut(&mut bus).matrix_irq_sources().is_empty(),
            "after INT_CLR the SYSTIMER asserts no matrix source"
        );
        bus.refresh_esp32c3_sched_sources();
        bus.recompute_esp32c3_irq_lines();
        assert_eq!(
            bus.riscv_irq_lines, 0,
            "routed line must de-assert once the SYSTIMER level clears"
        );
    }
}

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
mod c3_level_peripheral_matrix_routing {
    use crate::bus::SystemBus;
    use crate::peripherals::esp32c3::apb_saradc::{Esp32c3ApbSarAdc, APB_SARADC_INTR_SOURCE_ID};
    use crate::peripherals::esp32c3::spi::{Esp32c3Spi, SPI2_INTR_SOURCE_ID, TRANS_DONE};
    use crate::Bus;
    use labwired_config::{ChipDescriptor, SystemManifest};
    use std::path::PathBuf;

    const INTMATRIX: u64 = 0x600C_2000;
    const LINE: u32 = 7;

    // spi2 register offsets + bits (private in spi.rs; mirrored for the test).
    const SPI_CMD: u64 = 0x00;
    const SPI_DMA_INT_ENA: u64 = 0x34;
    const SPI_DMA_INT_CLR: u64 = 0x38;
    const SPI_USR_BIT: u32 = 1 << 24;

    // apb_saradc register offsets + bits (private in apb_saradc.rs; mirrored).
    const SARADC_ONETIME_SAMPLE: u64 = 0x20;
    const SARADC_INT_ENA: u64 = 0x40;
    const SARADC_INT_CLR: u64 = 0x4C;
    const SAR1_DONE_INT: u32 = 1 << 31;
    const SAR1_ONETIME_SAMPLE: u32 = 1 << 31;
    const ONETIME_START: u32 = 1 << 29;

    /// Build a devkit C3 bus (real `interrupt_core0` → INTC cache), enable C3
    /// routing, and route `source` → CPU line 7 (priority 1, threshold 1,
    /// enabled) — the same wiring the SYSTIMER routing gate uses.
    fn routed_bus(source: u32) -> SystemBus {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let chip = ChipDescriptor::from_file(root.join("../../configs/chips/esp32c3.yaml"))
            .expect("load esp32c3 chip yaml");
        let manifest =
            SystemManifest::from_file(root.join("../../configs/systems/esp32c3-devkit.yaml"))
                .expect("load esp32c3-devkit system yaml");
        let mut bus = SystemBus::from_config(&chip, &manifest).expect("build c3 devkit bus");
        bus.esp32c3_irq_routing = true;
        bus.refresh_peripheral_index();
        bus.write_u32(INTMATRIX + (source as u64) * 4, LINE)
            .unwrap();
        bus.write_u32(INTMATRIX + 0x114 + (LINE as u64) * 4, 1)
            .unwrap();
        bus.write_u32(INTMATRIX + 0x194, 1).unwrap();
        bus.write_u32(INTMATRIX + 0x104, 1 << LINE).unwrap();
        bus
    }

    fn idx(bus: &SystemBus, name: &str) -> usize {
        bus.find_peripheral_index_by_name(name)
            .unwrap_or_else(|| panic!("c3 devkit bus carries {name}"))
    }

    /// Drive the level directly on the peripheral's own register write path (the
    /// same effect an MMIO write has on the device), bypassing any bus-side
    /// clock gate so the test is about the routing, not RCC bring-up. Refreshes
    /// the legacy walk tick-set membership afterwards exactly as the real MMIO
    /// write choke (`SystemBus::write_u32`) does, so a walk-pinned peripheral
    /// that just asserted its level is actually ticked.
    fn write_dev(bus: &mut SystemBus, name: &str, off: u64, val: u32) {
        let i = idx(bus, name);
        bus.peripherals[i].dev.write_u32(off, val).unwrap();
        bus.refresh_legacy_tick_index(i);
    }

    fn arm_spi2(bus: &mut SystemBus) {
        write_dev(bus, "spi2", SPI_DMA_INT_ENA, TRANS_DONE); // enable TRANS_DONE
        write_dev(bus, "spi2", SPI_CMD, SPI_USR_BIT); // launch → latches TRANS_DONE
    }

    fn arm_saradc(bus: &mut SystemBus) {
        write_dev(bus, "apb_saradc", SARADC_INT_ENA, SAR1_DONE_INT); // enable
        write_dev(
            bus,
            "apb_saradc",
            SARADC_ONETIME_SAMPLE,
            SAR1_ONETIME_SAMPLE | ONETIME_START | (3 << 25), // one-shot ch3 → DONE
        );
    }

    fn pin_to_walk(bus: &mut SystemBus, name: &str) {
        let i = idx(bus, name);
        let dev = bus.peripherals[i].dev.as_any_mut().unwrap();
        if let Some(spi) = dev.downcast_mut::<Esp32c3Spi>() {
            spi.force_legacy_walk();
        } else if let Some(adc) = dev.downcast_mut::<Esp32c3ApbSarAdc>() {
            adc.force_legacy_walk();
        } else {
            panic!("{name} is not a known C3 level peripheral");
        }
        // Flipping uses_scheduler false changes walk-set membership; refresh it
        // so an already-armed peripheral joins the walk (the arm's own refresh
        // ran while it was still scheduler-driven and thus excluded).
        bus.refresh_legacy_tick_index(i);
        // Re-derive walk-deletion: once every C3 timer/level model migrated off
        // the walk (the LEDC timer port emptied the last real pinner on this
        // no-wifi_mac devkit bus), `from_config` builds the bus walk-DELETED, so
        // the per-cycle walk is globally skipped. Pinning a peripheral back onto
        // the walk (`force_legacy_walk` → `needs_legacy_walk() == true`) makes it
        // no longer deletable; recompute the flag so the walk path this gate
        // exercises actually runs.
        bus.legacy_walk_disabled = bus.derive_walk_deletable();
    }

    /// Shared body: arm `name` on a scheduler bus and a walk bus, tick each, and
    /// assert the routed CPU line is delivered identically; then clear the level
    /// and assert both de-assert. `source` is the peripheral's matrix source ID,
    /// `int_clr`/`clr_bit` the W1C acknowledge.
    fn assert_walk_scheduler_identical(
        name: &str,
        source: u32,
        int_clr: u64,
        clr_bit: u32,
        arm: fn(&mut SystemBus),
    ) {
        let mut sched = routed_bus(source);
        let mut walk = routed_bus(source);
        arm(&mut sched);
        arm(&mut walk);
        pin_to_walk(&mut walk, name);

        // Scheduler bus: the peripheral is walk-skipped and exports its level.
        let si = idx(&sched, name);
        assert!(
            sched.peripherals[si].dev.uses_scheduler(),
            "{name} must be scheduler-driven once the bus attached its clock"
        );
        assert_eq!(
            sched.peripherals[si].dev.matrix_irq_sources(),
            vec![source],
            "{name} must export its matrix source while armed+enabled"
        );
        // Walk bus: pinned back, so it re-emits via the walk (`tick`), not the
        // scheduler export. (`matrix_irq_sources` still reflects the raw level,
        // but the bus never polls it for a non-`uses_scheduler` peripheral.)
        let wi = idx(&walk, name);
        assert!(
            !walk.peripherals[wi].dev.uses_scheduler(),
            "force_legacy_walk must pin {name} back onto the per-cycle walk"
        );
        assert_eq!(
            walk.peripherals[wi].dev.tick().explicit_irqs,
            Some(vec![source]),
            "a walk-pinned {name} must re-emit its level source from the walk tick"
        );

        // One walk tick on each aggregates the routed line mask.
        sched.tick_peripherals_with_costs();
        walk.tick_peripherals_with_costs();
        assert_eq!(
            sched.riscv_irq_lines,
            1 << LINE,
            "scheduler path must route {name} source {source} to CPU line {LINE}"
        );
        assert_eq!(
            walk.riscv_irq_lines,
            1 << LINE,
            "walk path must route {name} source {source} to CPU line {LINE}"
        );
        assert_eq!(
            sched.riscv_irq_lines, walk.riscv_irq_lines,
            "walk vs scheduler IRQ delivery for {name} must be byte-identical"
        );

        // Acknowledge the level on both; the routed line must de-assert on the
        // next tick — same level semantics on either path.
        write_dev(&mut sched, name, int_clr, clr_bit);
        write_dev(&mut walk, name, int_clr, clr_bit);
        sched.tick_peripherals_with_costs();
        walk.tick_peripherals_with_costs();
        assert_eq!(
            sched.riscv_irq_lines, 0,
            "scheduler path must de-assert {name} after INT_CLR"
        );
        assert_eq!(
            walk.riscv_irq_lines, 0,
            "walk path must de-assert {name} after INT_CLR"
        );
    }

    #[test]
    fn spi2_irq_delivered_identically_walk_and_scheduler() {
        assert_walk_scheduler_identical(
            "spi2",
            SPI2_INTR_SOURCE_ID,
            SPI_DMA_INT_CLR,
            TRANS_DONE,
            arm_spi2,
        );
    }

    #[test]
    fn apb_saradc_irq_delivered_identically_walk_and_scheduler() {
        assert_walk_scheduler_identical(
            "apb_saradc",
            APB_SARADC_INTR_SOURCE_ID,
            SARADC_INT_CLR,
            SAR1_DONE_INT,
            arm_saradc,
        );
    }
}

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
mod c3_ledc_matrix_routing {
    use crate::bus::SystemBus;
    use crate::peripherals::esp32c3::ledc::{Esp32c3Ledc, LEDC_BASE, LEDC_INTR_SOURCE_ID};
    use crate::Bus;
    use crate::Peripheral;
    use labwired_config::{ChipDescriptor, SystemManifest};
    use std::path::PathBuf;

    const INTMATRIX: u64 = 0x600C_2000;
    const LINE: u32 = 9;

    // LEDC register offsets (private in ledc.rs; mirrored for the test).
    const TIMER0_CONF: u64 = 0xA0;
    const INT_ENA: u64 = 0xC8;
    const INT_CLR: u64 = 0xCC;
    // TIMER0_CONF: DUTY_RES=4 (period 16), CLK_DIV integer part 1, running.
    const TIMER0_RUN: u32 = 4 | ((1u32 << 8) << 4);
    // Cycle comfortably past the first overflow (16 counts × divider 1).
    const PAST_OVF: u64 = 20;

    /// Build a devkit C3 bus (real `interrupt_core0` → INTC cache), enable C3
    /// routing, route the LEDC source (23) → CPU line 9, and arm timer0 with its
    /// OVF interrupt enabled — all through ordinary MMIO writes.
    fn setup() -> SystemBus {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let chip = ChipDescriptor::from_file(root.join("../../configs/chips/esp32c3.yaml"))
            .expect("load esp32c3 chip yaml");
        let manifest =
            SystemManifest::from_file(root.join("../../configs/systems/esp32c3-devkit.yaml"))
                .expect("load esp32c3-devkit system yaml");
        let mut bus = SystemBus::from_config(&chip, &manifest).expect("build c3 devkit bus");
        bus.esp32c3_irq_routing = true;
        bus.refresh_peripheral_index();

        // Route source 23 → line 9, priority 1, threshold 1, line enabled.
        bus.write_u32(INTMATRIX + LEDC_INTR_SOURCE_ID as u64 * 4, LINE)
            .unwrap();
        bus.write_u32(INTMATRIX + 0x114 + (LINE as u64) * 4, 1)
            .unwrap();
        bus.write_u32(INTMATRIX + 0x194, 1).unwrap();
        bus.write_u32(INTMATRIX + 0x104, 1 << LINE).unwrap();

        // Arm timer0 (period 16, divider 1) and enable its LSTIMER0_OVF int.
        bus.write_u32(LEDC_BASE as u64 + INT_ENA, 1).unwrap();
        bus.write_u32(LEDC_BASE as u64 + TIMER0_CONF, TIMER0_RUN)
            .unwrap();
        bus
    }

    fn ledc_mut(bus: &mut SystemBus) -> &mut Esp32c3Ledc {
        let idx = bus.find_peripheral_index_by_name("ledc").unwrap();
        bus.peripherals[idx]
            .dev
            .as_any_mut()
            .unwrap()
            .downcast_mut::<Esp32c3Ledc>()
            .unwrap()
    }

    /// The scheduler routing arm and the legacy walk aggregation produce the
    /// SAME `riscv_irq_lines` for the SAME LEDC overflow level.
    #[test]
    fn scheduler_routing_matches_walk_routing_for_overflow() {
        let mut bus = setup();

        // Advance the LEDC past its first overflow (publishes the clock so the
        // scheduler-driven model's lazy counters materialise LSTIMER0_OVF).
        bus.set_current_cycle(PAST_OVF);
        assert_eq!(
            ledc_mut(&mut bus).matrix_irq_sources(),
            vec![LEDC_INTR_SOURCE_ID],
            "an overflowed+enabled LEDC must assert matrix source 23"
        );

        // Scheduler routing arm (what `apply_event_result` runs on the C3 bus).
        bus.refresh_esp32c3_sched_sources();
        bus.recompute_esp32c3_irq_lines();
        let scheduler_lines = bus.riscv_irq_lines;
        assert_eq!(
            scheduler_lines,
            1 << LINE,
            "scheduler routing must assert the routed CPU line for source 23"
        );

        // Legacy walk routing reference: source 23 re-emitted by the walk. Must
        // land on the identical line mask.
        bus.aggregate_esp32c3_irqs(&[LEDC_INTR_SOURCE_ID]);
        assert_eq!(
            bus.riscv_irq_lines, scheduler_lines,
            "walk aggregation and scheduler routing must produce identical riscv_irq_lines"
        );
    }

    /// A LEDC pinned back onto the per-cycle walk (`force_legacy_walk`) re-emits
    /// the SAME matrix source from its `tick()` — the level the scheduler path
    /// exports via `matrix_irq_sources` — so both drive modes deliver the OVF IRQ
    /// identically.
    #[test]
    fn force_legacy_walk_reemits_same_source() {
        let mut bus = setup();
        let ledc = ledc_mut(&mut bus);
        ledc.force_legacy_walk();
        assert!(
            !ledc.uses_scheduler(),
            "force_legacy_walk must pin LEDC back onto the per-cycle walk"
        );
        // Drive the walk past the overflow; the level source must re-emit.
        let mut irqs = None;
        for _ in 0..PAST_OVF {
            irqs = ledc.tick().explicit_irqs;
        }
        assert_eq!(
            irqs,
            Some(vec![LEDC_INTR_SOURCE_ID]),
            "a walk-pinned LEDC must re-emit its OVF level source from the walk tick"
        );
    }

    /// Clearing the LEDC overflow (INT_CLR) de-asserts the routed line on the
    /// next re-derivation — same level semantics as the walk.
    #[test]
    fn clearing_overflow_deasserts_routed_line() {
        let mut bus = setup();
        bus.set_current_cycle(PAST_OVF);
        bus.refresh_esp32c3_sched_sources();
        bus.recompute_esp32c3_irq_lines();
        assert_eq!(bus.riscv_irq_lines, 1 << LINE);

        // INT_CLR bit0 clears the LSTIMER0_OVF latch → level drops.
        bus.write_u32(LEDC_BASE as u64 + INT_CLR, 1).unwrap();
        assert!(
            ledc_mut(&mut bus).matrix_irq_sources().is_empty(),
            "after INT_CLR the LEDC asserts no matrix source"
        );
        bus.refresh_esp32c3_sched_sources();
        bus.recompute_esp32c3_irq_lines();
        assert_eq!(
            bus.riscv_irq_lines, 0,
            "routed line must de-assert once the LEDC overflow level clears"
        );
    }
}

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
mod c3_wifi_mac_matrix_routing {
    use crate::bus::SystemBus;
    use crate::peripherals::esp32c3::wifi_mac::Esp32c3WifiMac;
    use crate::Bus;
    use crate::Peripheral;
    use labwired_config::{ChipDescriptor, SystemManifest};
    use std::path::PathBuf;

    const MAC_BASE: u64 = 0x6003_3000;
    const INTMATRIX: u64 = 0x600C_2000;
    const LINE: u32 = 6;
    /// WiFi MAC interrupt-matrix source (MAC_INTR_MAP @ offset 0).
    const MAC_SOURCE: u32 = 0;

    // wifi_mac register offsets (private in wifi_mac.rs; mirrored for the test).
    const EVENT_GET: u64 = 0xC3C; // MAC event word (HW-set; read by the ISR)
    const EVENT_CLR: u64 = 0xC40; // W1C acknowledge
    const EVENT_RX_DONE: u32 = 0x0100_4000;

    /// Build a devkit C3 bus (real `interrupt_core0` → INTC cache), enable C3
    /// routing, swap the declarative `wifi_mac` stub for the real behavioral
    /// model, and route the MAC source (0) → CPU line 6. `scheduler` selects the
    /// drive mode: with the bus cycle clock attached the model is
    /// scheduler-driven (walk-skipped, level exported via `matrix_irq_sources`);
    /// without it the model stays on the legacy per-cycle walk.
    fn setup(scheduler: bool) -> SystemBus {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let chip = ChipDescriptor::from_file(root.join("../../configs/chips/esp32c3.yaml"))
            .expect("load esp32c3 chip yaml");
        let manifest =
            SystemManifest::from_file(root.join("../../configs/systems/esp32c3-devkit.yaml"))
                .expect("load esp32c3-devkit system yaml");
        let mut bus = SystemBus::from_config(&chip, &manifest).expect("build c3 devkit bus");
        bus.esp32c3_irq_routing = true;

        // Swap the declarative wifi_mac for the real behavioral model at its base.
        let idx = bus
            .find_peripheral_index_by_name("wifi_mac")
            .expect("devkit bus carries a wifi_mac");
        let mut dev = Esp32c3WifiMac::new();
        if scheduler {
            dev.attach_cycle_clock(bus.cycle_clock.clone());
        }
        bus.peripherals[idx].dev = Box::new(dev);
        bus.refresh_peripheral_index();

        // Route source 0 → line 6, priority 1, threshold 1, line enabled.
        bus.write_u32(INTMATRIX + MAC_SOURCE as u64 * 4, LINE)
            .unwrap();
        bus.write_u32(INTMATRIX + 0x114 + (LINE as u64) * 4, 1)
            .unwrap();
        bus.write_u32(INTMATRIX + 0x194, 1).unwrap();
        bus.write_u32(INTMATRIX + 0x104, 1 << LINE).unwrap();
        bus
    }

    fn mac_mut(bus: &mut SystemBus) -> &mut Esp32c3WifiMac {
        let idx = bus.find_peripheral_index_by_name("wifi_mac").unwrap();
        bus.peripherals[idx]
            .dev
            .as_any_mut()
            .unwrap()
            .downcast_mut::<Esp32c3WifiMac>()
            .unwrap()
    }

    /// The scheduler routing arm and the legacy walk aggregation produce the
    /// SAME `riscv_irq_lines` for the SAME pending MAC event level.
    #[test]
    fn scheduler_routing_matches_walk_routing_for_mac_event() {
        let mut bus = setup(true);

        // Arm the MAC event (HW normally sets it; the ISR reads it). The level
        // asserts source 0 while the event word is non-zero.
        bus.write_u32(MAC_BASE + EVENT_GET, EVENT_RX_DONE).unwrap();
        assert_eq!(
            mac_mut(&mut bus).matrix_irq_sources(),
            vec![MAC_SOURCE],
            "a MAC with a pending event must assert matrix source 0"
        );

        // Scheduler routing arm (what `apply_event_result` runs on the C3 bus).
        bus.refresh_esp32c3_sched_sources();
        bus.recompute_esp32c3_irq_lines();
        let scheduler_lines = bus.riscv_irq_lines;
        assert_eq!(
            scheduler_lines,
            1 << LINE,
            "scheduler routing must assert the routed CPU line for source 0"
        );

        // Legacy walk routing reference: source 0 re-emitted by the walk. Must
        // land on the identical line mask.
        bus.aggregate_esp32c3_irqs(&[MAC_SOURCE]);
        assert_eq!(
            bus.riscv_irq_lines, scheduler_lines,
            "walk aggregation and scheduler routing must produce identical riscv_irq_lines"
        );
    }

    /// A wifi_mac pinned back onto the per-cycle walk (`force_legacy_walk`)
    /// re-emits the SAME matrix source from its `tick()` — the level the
    /// scheduler path exports via `matrix_irq_sources` — so both drive modes
    /// deliver the MAC IRQ identically.
    #[test]
    fn force_legacy_walk_reemits_same_source() {
        let mut bus = setup(false);
        bus.write_u32(MAC_BASE + EVENT_GET, EVENT_RX_DONE).unwrap();
        let mac = mac_mut(&mut bus);
        assert!(
            !mac.uses_scheduler(),
            "no clock attached → wifi_mac stays on the per-cycle walk"
        );
        assert_eq!(
            mac.tick().explicit_irqs,
            Some(vec![MAC_SOURCE]),
            "a walk-driven wifi_mac must re-emit its MAC level source from the walk tick"
        );
    }

    /// THE write-choke proof: on a fully walk-DELETED bus (no per-cycle walk to
    /// re-derive the level), the `EVENT_CLR` acknowledge must de-assert the
    /// routed line AT THE WRITE — otherwise the MAC level latches forever and
    /// re-enters its ISR. This is the one legitimate bus addition the last
    /// walker needs (`sync_esp32c3_irq_cache_write`).
    #[test]
    fn event_clr_deasserts_on_walk_deleted_bus_at_the_write() {
        let mut bus = setup(true);
        // Delete the walk: with wifi_mac scheduler-driven and every other model
        // inert/scheduler, the devkit bus derives walk-deletion.
        bus.legacy_walk_disabled = bus.derive_walk_deletable();
        assert!(
            bus.legacy_walk_disabled,
            "scheduler-driven wifi_mac must let the devkit bus derive walk-deletion"
        );

        // Arm the MAC level and route it at the write choke (writing EVENT_GET is
        // a MAC-window write → the choke re-derives the scheduler level).
        bus.write_u32(MAC_BASE + EVENT_GET, EVENT_RX_DONE).unwrap();
        assert_eq!(
            bus.riscv_irq_lines,
            1 << LINE,
            "MAC event must route to the CPU line at the write, with NO walk tick"
        );

        // Acknowledge via EVENT_CLR (W1C). On a walk-deleted bus the ONLY thing
        // that can de-assert the level is the write choke re-derivation.
        bus.write_u32(MAC_BASE + EVENT_CLR, EVENT_RX_DONE).unwrap();
        assert!(
            mac_mut(&mut bus).matrix_irq_sources().is_empty(),
            "after EVENT_CLR the MAC asserts no matrix source"
        );
        assert_eq!(
            bus.riscv_irq_lines, 0,
            "EVENT_CLR must de-assert the routed line at the write on a walk-deleted bus"
        );
    }
}

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
mod c3_wifi_mac_walk_differential {
    use crate::bus::SystemBus;
    use crate::peripherals::esp32c3::wifi_mac::Esp32c3WifiMac;
    use crate::{Bus, Peripheral};
    use labwired_config::{ChipDescriptor, SystemManifest};
    use std::path::PathBuf;

    const MAC_BASE: u64 = 0x6003_3000;
    const INTMATRIX: u64 = 0x600C_2000;
    const LINE: u32 = 6;
    const MAC_SOURCE: u32 = 0;

    // wifi_mac register offsets / bits (private; mirrored for the test).
    const RX_RING_BASE: u64 = 0x88;
    const EVENT_CLR: u64 = 0xC40;
    const PLCP0_AC0: u64 = 0xD08;
    const TX_KICK_BITS: u32 = 0xC000_0000;
    const EVENT_RX_DONE: u32 = 0x0100_4000;
    const EVENT_TX_DONE: u32 = 0x80;
    const DESC_OWNER: u32 = 1 << 31;
    const DESC_EOF: u32 = 1 << 30;

    // DRAM layout, inside the devkit 0x3FC8_0000 400KB SRAM AND the
    // 0x3fc0_0000..0x3fd0_0000 window `deliver_one_rx` accepts.
    const RX_DESC: u32 = 0x3FC8_1000;
    const RX_BUF: u32 = 0x3FC8_1200;
    const TX_DESC: u32 = 0x3FC8_2000;
    const TX_BUF: u32 = 0x3FC8_2200;

    /// A tiny "802.11" data frame (enough header for the medium/len parse).
    fn tx_frame() -> Vec<u8> {
        let mut f = vec![0u8; 40];
        f[0] = 0x08; // data frame (type 2)
                     // addr2 (SA) bytes 10..16 — a nonzero source so the model is happy.
        f[10..16].copy_from_slice(&[0x02, 0, 0, 0, 0, 0x02]);
        f
    }

    fn rx_frame() -> Vec<u8> {
        vec![0xB0u8, 0x00, 0xAA, 0xBB, 0xCC, 0xDD]
    }

    fn build_bus(scheduler: bool) -> SystemBus {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let chip = ChipDescriptor::from_file(root.join("../../configs/chips/esp32c3.yaml"))
            .expect("load esp32c3 chip yaml");
        let manifest =
            SystemManifest::from_file(root.join("../../configs/systems/esp32c3-devkit.yaml"))
                .expect("load esp32c3-devkit system yaml");
        let mut bus = SystemBus::from_config(&chip, &manifest).expect("build c3 devkit bus");
        bus.esp32c3_irq_routing = true;

        let idx = bus
            .find_peripheral_index_by_name("wifi_mac")
            .expect("devkit bus carries a wifi_mac");
        let mut dev = Esp32c3WifiMac::new();
        if scheduler {
            dev.attach_cycle_clock(bus.cycle_clock.clone());
        }
        bus.peripherals[idx].dev = Box::new(dev);
        bus.refresh_peripheral_index();

        // Route source 0 → line 6.
        bus.write_u32(INTMATRIX + MAC_SOURCE as u64 * 4, LINE)
            .unwrap();
        bus.write_u32(INTMATRIX + 0x114 + (LINE as u64) * 4, 1)
            .unwrap();
        bus.write_u32(INTMATRIX + 0x194, 1).unwrap();
        bus.write_u32(INTMATRIX + 0x104, 1 << LINE).unwrap();

        // Recompute walk-deletion over the swapped-in MAC: scheduler-driven ⇒
        // walk-deleted (the real deploy path); walk-driven ⇒ the walk stays live
        // so `tick()` re-emits the MAC level. (Without this the walk bus would
        // inherit `from_config`'s stale `true`, computed when the declarative
        // wifi_mac stub was still inert, and wrongly skip the walk.)
        bus.legacy_walk_disabled = bus.derive_walk_deletable();
        bus
    }

    fn wifi_mut(bus: &mut SystemBus) -> &mut Esp32c3WifiMac {
        let idx = bus.find_peripheral_index_by_name("wifi_mac").unwrap();
        bus.peripherals[idx]
            .dev
            .as_any_mut()
            .unwrap()
            .downcast_mut::<Esp32c3WifiMac>()
            .unwrap()
    }

    #[derive(PartialEq, Debug)]
    struct SessionResult {
        rx_desc_w0: u32,
        rx_buf_frame: Vec<u8>,
        event_word: u32,
        line_while_pending: u32,
        line_after_clear: u32,
        tx_frames: Vec<Vec<u8>>,
    }

    /// Drive one full WiFi session and capture every observable. Steps the bus
    /// one cycle at a time exactly as `Machine::step` does: publish the cycle,
    /// then run the peripheral tick only every `interval` cycles (the pump
    /// processes one TX/RX per tick call). The pump runs inside
    /// `tick_peripherals_with_costs` (the bus-tick pass), armed by the ring-enable
    /// / TX-kick MMIO writes below.
    fn run_session(scheduler: bool, interval: u32) -> SessionResult {
        let mut bus = build_bus(scheduler);

        // Lay out a 1-entry RX ring: an owner-held descriptor + its buffer.
        bus.write_u32(RX_DESC as u64, DESC_OWNER | 1600).unwrap(); // owner, cap 1600
        bus.write_u32(RX_DESC as u64 + 4, RX_BUF).unwrap();
        bus.write_u32(RX_DESC as u64 + 8, 0).unwrap();
        // Enable the RX ring (MMIO write → arms the pump via the write choke).
        bus.write_u32(MAC_BASE + RX_RING_BASE, RX_DESC).unwrap();

        // Lay out a TX lldesc (word1 = buffer ptr) + the frame in DRAM.
        let frame = tx_frame();
        bus.write_u32(TX_DESC as u64, 0).unwrap();
        bus.write_u32(TX_DESC as u64 + 4, TX_BUF).unwrap();
        for (i, b) in frame.iter().enumerate() {
            bus.write_u8(TX_BUF as u64 + i as u64, *b).unwrap();
        }

        // Queue a received frame (non-MMIO injection; the live ring keeps the MAC
        // resident so it is pumped without any extra re-arm).
        wifi_mut(&mut bus).queue_rx_frame(rx_frame());

        // Kick a TX: PLCP0 low-20 bits point at the TX lldesc, kick bits set.
        let plcp0 = TX_KICK_BITS | (TX_DESC & 0x000F_FFFF);
        bus.write_u32(MAC_BASE + PLCP0_AC0, plcp0).unwrap();

        // Step long enough to drain both rings (one TX + one RX, one per tick).
        for c in 1..=(interval as u64 * 8) {
            bus.set_current_cycle(c);
            if c % interval as u64 == 0 {
                bus.tick_peripherals_with_costs();
            }
        }

        let rx_desc_w0 = bus.read_u32(RX_DESC as u64).unwrap();
        let mut rx_buf_frame = vec![0u8; rx_frame().len()];
        for (i, b) in rx_buf_frame.iter_mut().enumerate() {
            // The rx-control header is 48 bytes; the 802.11 frame follows.
            *b = bus.read_u8(RX_BUF as u64 + 48 + i as u64).unwrap();
        }
        let event_word = bus.read_u32(MAC_BASE + 0xC3C).unwrap();
        let line_while_pending = bus.riscv_irq_lines;
        let tx_frames = wifi_mut(&mut bus).take_tx_frames();

        // Acknowledge every event (W1C) and confirm the routed line drops.
        bus.write_u32(MAC_BASE + EVENT_CLR, event_word).unwrap();
        // On a walk bus (walk still live) one more tick re-derives the level;
        // on a scheduler/walk-deleted bus the write choke already did.
        bus.set_current_cycle(interval as u64 * 9);
        bus.tick_peripherals_with_costs();
        let line_after_clear = bus.riscv_irq_lines;

        SessionResult {
            rx_desc_w0,
            rx_buf_frame,
            event_word,
            line_while_pending,
            line_after_clear,
            tx_frames,
        }
    }

    /// Non-vacuity: a captured session must actually have moved frames and
    /// asserted the routed MAC line.
    fn assert_non_vacuous(r: &SessionResult, what: &str) {
        assert_eq!(
            r.rx_desc_w0 & DESC_OWNER,
            0,
            "{what}: RX descriptor owner cleared (frame delivered)"
        );
        assert_ne!(r.rx_desc_w0 & DESC_EOF, 0, "{what}: RX descriptor EOF set");
        assert_eq!(
            r.rx_buf_frame,
            rx_frame(),
            "{what}: RX frame DMAed into the buffer"
        );
        assert_ne!(
            r.event_word & EVENT_RX_DONE,
            0,
            "{what}: RX-done event latched"
        );
        assert_ne!(
            r.event_word & EVENT_TX_DONE,
            0,
            "{what}: TX-done event latched"
        );
        assert_eq!(
            r.tx_frames.len(),
            1,
            "{what}: exactly one TX frame captured"
        );
        assert_eq!(
            r.line_while_pending,
            1 << LINE,
            "{what}: MAC level routed to the CPU line"
        );
        assert_eq!(
            r.line_after_clear, 0,
            "{what}: routed line de-asserts after EVENT_CLR"
        );
    }

    #[test]
    fn wifi_session_walk_vs_scheduler_byte_identical_interval_1() {
        let walk = run_session(false, 1);
        let sched = run_session(true, 1);
        assert_non_vacuous(&walk, "walk@1");
        assert_eq!(
            walk, sched,
            "WiFi session must be byte-identical walk-vs-scheduler at interval 1"
        );
    }

    #[test]
    fn wifi_session_walk_vs_scheduler_byte_identical_interval_64() {
        let walk = run_session(false, 64);
        let sched = run_session(true, 64);
        assert_non_vacuous(&sched, "sched@64");
        assert_eq!(
            walk, sched,
            "WiFi session must be byte-identical walk-vs-scheduler at interval 64"
        );
    }

    #[test]
    fn wifi_session_scheduler_interval_independent() {
        // The pump's drained ring state + captured TX + event must not depend on
        // the tick interval (batching is fidelity-safe for the descriptor rings).
        let s1 = run_session(true, 1);
        let s64 = run_session(true, 64);
        assert_non_vacuous(&s1, "sched@1");
        assert_eq!(
            s1, s64,
            "scheduler WiFi session must be interval-independent (1 == 64)"
        );
    }
}
