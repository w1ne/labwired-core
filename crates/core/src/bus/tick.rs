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
    /// Phase 2B.1 (issue #192): pend an NVIC IRQ on behalf of an event
    /// handler. Mirrors the per-tick `pend_nvic` path but collects
    /// non-NVIC fallthroughs into the supplied vector for the caller to
    /// forward to `cpu.set_exception_pending`.
    pub fn pend_irq_for_event(&self, irq: u32, fallthrough: &mut Vec<u32>) {
        pend_nvic(&self.nvic, fallthrough, irq);
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
            }
        }
    }

    /// Phase 2B.2 (issue #192): if the peripheral at `idx` is scheduler-driven,
    /// advance its lazy state to the current peripheral-tick index before an
    /// MMIO write observes it. The tick index is `current_cycle /
    /// peripheral_tick_interval` — the same quantum the legacy walk advanced by
    /// one per `tick()`. One virtual `uses_scheduler()` call for legacy
    /// peripherals (false → return); the sync only runs for opted-in ones.
    #[cfg(feature = "event-scheduler")]
    #[inline]
    pub(crate) fn sync_scheduler_peripheral(&mut self, idx: usize) {
        let interval = (self.config.peripheral_tick_interval as u64).max(1);
        let tick_now = self.current_cycle / interval;
        let p = &mut self.peripherals[idx];
        if p.dev.uses_scheduler() {
            p.dev.sync_to(tick_now);
        }
    }

    /// Phase 2B.3a (issue #192): after an MMIO write to a scheduler-driven
    /// peripheral, harvest any events it wants scheduled (e.g. a just-armed
    /// TX interrupt) into `pending_schedule` for `Machine` to enqueue. One
    /// virtual `uses_scheduler()` call for legacy peripherals (false → return).
    #[cfg(feature = "event-scheduler")]
    #[inline]
    pub(crate) fn collect_scheduled_events(&mut self, idx: usize) {
        if !self.peripherals[idx].dev.uses_scheduler() {
            return;
        }
        for (delay, token) in self.peripherals[idx].dev.take_scheduled_events() {
            self.pending_schedule.push((idx, delay, token));
        }
    }

    #[allow(clippy::type_complexity)]
    fn tick_peripherals_phase1(
        &mut self,
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

        // ── Pre-tick bus-aware pass ─────────────────────────────────────────
        // Some peripherals (currently just RADIO) need to read/write the bus
        // BEFORE their `tick()` runs so the work they schedule (e.g. setting
        // a bit-rate countdown after reading PACKETPTR-pointed RAM) is
        // visible to that same tick(). The swap dance below temporarily
        // removes the peripheral from `self.peripherals` so we can lend
        // `&mut self` into `tick_with_bus`; a no-op stub stands in for the
        // duration. `needs_bus_tick` returning false skips this for
        // everyone else at near-zero cost.
        for i in 0..self.peripherals.len() {
            if !self.peripherals[i].dev.needs_bus_tick() {
                continue;
            }
            let placeholder: Box<dyn Peripheral> =
                Box::new(crate::peripherals::stub::StubPeripheral::new(0));
            let mut dev = std::mem::replace(&mut self.peripherals[i].dev, placeholder);
            dev.tick_with_bus(self);
            self.peripherals[i].dev = dev;
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

        for (peripheral_index, p) in self.peripherals.iter_mut().enumerate() {
            #[cfg(feature = "event-scheduler")]
            if legacy_walk_disabled {
                break;
            }
            // Phase 2B.2 (issue #192): scheduler-driven peripherals are advanced
            // lazily via `sync_to` on MMIO access (and by the event drain in
            // `Machine::step`), never by this per-cycle walk. Skipping them here
            // is the actual orchestration saving. Gated so the legacy build is
            // byte-identical.
            #[cfg(feature = "event-scheduler")]
            if p.dev.uses_scheduler() {
                continue;
            }

            if p.ticks_remaining > tick_interval {
                p.ticks_remaining -= tick_interval;
                continue;
            }

            let res = p.dev.tick();

            p.ticks_remaining = res.ticks_until_next.unwrap_or(0);

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
                for sig in signals {
                    dma_signals_out.push((p.name.clone(), sig));
                }
            }

            if res.irq {
                if let Some(irq) = p.irq {
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
                fired_events_global.push((p.base as u32).wrapping_add(off));
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

        // GPIO edge-detection pass: snapshot the IN registers of GPIO ports
        // 0 and 1, diff against last-known state, and notify every
        // peripheral of changed pins.  GPIOTE overrides observe_gpio_change
        // to drive EVENTS_IN[i] when a channel watches a matching pin.
        //
        // We look up peripheral bases by name so the addresses stay valid
        // even when a chip yaml relocates GPIO ports (e.g. the onboarding
        // yaml's non-standard gpio1 at 0x50001000).
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

        // HC-SR04 service pass: read each sensor's TRIG output level and drive
        // the computed ECHO input level. Empty list → skipped entirely.
        self.service_hcsr04();
        self.service_can_diagnostic_testers();
        self.service_can_uds_testers();

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
    /// ESP32-C3 (RISC-V) interrupt routing. Each tick, build the level-sensitive
    /// bitmask of asserted CPU interrupt lines from the SYSTEM FROM_CPU IPI
    /// registers (0x600C0028..0x34, bit0) — the mechanism FreeRTOS `vPortYield`
    /// uses to request a context switch. Each asserted source is routed to a CPU
    /// line via its INTERRUPT_CORE0 MAP register (0x600C2000 + source*4, low 5
    /// bits), gated by CPU_INT_ENABLE and per-line priority vs CPU_INT_THRESH.
    /// The result lands in `riscv_irq_lines`, which the core ORs into `mip`.
    /// No-op unless `esp32c3_irq_routing` is set (only the C3 rom-boot path sets
    /// it). Peripheral `explicit_irqs` (e.g. the reused S3 SYSTIMER model) are
    /// not routed yet — their source IDs don't match the C3 matrix.
    fn aggregate_esp32c3_irqs(&mut self, source_ids: &[u32]) {
        if !self.esp32c3_irq_routing {
            return;
        }
        const INTMATRIX_BASE: u64 = 0x600C_2000;
        // SYSTEM FROM_CPU IPI registers → source IDs 50..53 (matching the
        // INTERRUPT_CORE0 CPU_INTR_FROM_CPU_n_MAP offsets at 200..212).
        const FROM_CPU: [(u64, u32); 4] = [
            (0x600C_0028, 50),
            (0x600C_002C, 51),
            (0x600C_0030, 52),
            (0x600C_0034, 53),
        ];

        // Route peripheral `explicit_irqs` (e.g. the SYSTIMER tick alarm, which
        // the C3 wiring configures to emit matrix source 37) plus the FROM_CPU
        // IPI sources (the FreeRTOS yield mechanism).
        let mut asserted: Vec<u32> = source_ids.to_vec();
        for (addr, src) in FROM_CPU {
            if self.read_u32(addr).map(|v| v & 1 != 0).unwrap_or(false) {
                asserted.push(src);
            }
        }

        // INTC control registers (offsets verified against interrupt_core0.yaml):
        //   CPU_INT_ENABLE 0x104, CPU_INT_PRI_n 0x114+n*4, CPU_INT_THRESH 0x194.
        // A line fires only while it is enabled AND its priority >= threshold —
        // the C3 enables/masks via these INTC registers, NOT the RISC-V `mie`
        // CSR (FreeRTOS critical sections raise the threshold to mask).
        let enable = self.read_u32(INTMATRIX_BASE + 0x104).unwrap_or(0);
        let thresh = self.read_u32(INTMATRIX_BASE + 0x194).unwrap_or(0) & 0xF;

        let mut mask = 0u32;
        for src in asserted {
            // MAP register holds the destination CPU interrupt line (1..31).
            let line = self
                .read_u32(INTMATRIX_BASE + (src as u64) * 4)
                .map(|v| v & 0x1F)
                .unwrap_or(0);
            let pri = self
                .read_u32(INTMATRIX_BASE + 0x114 + (line as u64) * 4)
                .unwrap_or(0)
                & 0xF;
            if src == 0 && std::env::var("LABWIRED_RXBUF_TRACE").is_ok() {
                use std::sync::atomic::{AtomicU32, Ordering};
                static N: AtomicU32 = AtomicU32::new(0);
                if N.fetch_add(1, Ordering::Relaxed) < 3 {
                    eprintln!(
                        "[macirq] src0 line={line} enable_bit={} pri={pri} thresh={thresh}",
                        (enable >> line) & 1
                    );
                }
            }
            if line == 0 || (enable & (1 << line)) == 0 {
                continue;
            }
            if pri >= thresh {
                mask |= 1u32 << line;
            }
        }
        self.riscv_irq_lines = mask;
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
        let has_intmatrix = self.peripherals.iter().any(|p| {
            p.dev
                .as_any()
                .and_then(|a| {
                    a.downcast_ref::<crate::peripherals::esp32s3::intmatrix::Esp32s3IntMatrix>()
                })
                .is_some()
        });
        if !has_intmatrix {
            return;
        }
        let mut routed = [0u32; 2];
        let mut intr_status = [0u32; 4];
        for &source_id in source_ids {
            // Route each asserting source through BOTH cores' map tables;
            // a source delivers to whichever core(s) bound it (the SMP
            // cross-core IPI relies on this: source 79 → core 0, 80 → core 1).
            if let Some(slot) = self.route_irq_source_to_cpu_irq_core(source_id, 0) {
                routed[0] |= 1u32 << slot;
            }
            if let Some(slot) = self.route_irq_source_to_cpu_irq_core(source_id, 1) {
                routed[1] |= 1u32 << slot;
            }
            // Mirror into PRO_INTR_STATUS_REG_n bitmap so esp-hal's
            // __level_*_interrupt can discover which source asserted.
            let reg = (source_id / 32) as usize;
            let bit = source_id & 31;
            if reg < intr_status.len() {
                intr_status[reg] |= 1u32 << bit;
            }
        }
        self.pending_cpu_irqs = routed;
        // Push the live source-assertion bitmap into the intmatrix peripheral.
        // No-op for buses without an intmatrix registered.
        for p in self.peripherals.iter_mut() {
            if let Some(any) = p.dev.as_any_mut() {
                if let Some(matrix) =
                    any.downcast_mut::<crate::peripherals::esp32s3::intmatrix::Esp32s3IntMatrix>()
                {
                    matrix.set_pending_sources(intr_status);
                    break;
                }
            }
        }
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
            self.tick_peripherals_phase1();
        // Plan 3: route ESP32-S3 source IDs through the intmatrix and update
        // the pending cpu IRQ bitmap + intmatrix INTR_STATUS mirror.
        self.aggregate_esp32s3_explicit_irqs(&explicit_source_ids);
        self.aggregate_esp32c3_irqs(&explicit_source_ids);
        self.collect_enabled_nvic_interrupts(&mut interrupts);

        (interrupts, costs, dma_requests)
    }

    pub fn tick_peripherals_fully(&mut self) -> (Vec<u32>, Vec<PeripheralTickCost>) {
        let (mut interrupts, costs, pending_dma, dma_signals, explicit_source_ids) =
            self.tick_peripherals_phase1();
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
