// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! SystemBus construction, peripheral install, UART/IO-Link sinks, bit-band/atomic helpers.

use super::*;

impl SystemBus {
    pub fn new() -> Self {
        // Default initialization for tests
        let mut bus = Self {
            flash_thunks: std::collections::HashMap::new(),
            flash: LinearMemory::new(1024 * 1024, 0x0),
            ram: LinearMemory::new(1024 * 1024, 0x2000_0000),
            extra_mem: Vec::new(),
            peripherals: vec![
                PeripheralEntry {
                    name: "uart1".to_string(),
                    base: 0x4000_C000,
                    size: 0x400,
                    irq: Some(37),
                    dev: Box::new(crate::peripherals::uart::Uart::new()),
                    ticks_remaining: 0,
                    clock_gate: None,
                },
                PeripheralEntry {
                    name: "gpioa".to_string(),
                    base: 0x4001_0800,
                    size: 0x400,
                    irq: None,
                    dev: Box::new(crate::peripherals::gpio::GpioPort::new()),
                    ticks_remaining: 0,
                    clock_gate: None,
                },
                PeripheralEntry {
                    name: "rcc".to_string(),
                    base: 0x4002_1000,
                    size: 0x400,
                    irq: None,
                    dev: Box::new(crate::peripherals::rcc::Rcc::new()),
                    ticks_remaining: 0,
                    clock_gate: None,
                },
                PeripheralEntry {
                    name: "systick".to_string(),
                    base: 0xE000_E010,
                    size: 0x100,
                    irq: Some(15),
                    dev: Box::new(crate::peripherals::systick::Systick::new()),
                    ticks_remaining: 0,
                    clock_gate: None,
                },
            ],
            nvic: None,
            observers: Vec::new(),
            config: crate::SimulationConfig::default(),
            bit_band_enabled: true,
            pending_cpu_irqs: [0; 2],
            dport_idx: None,
            rcc_idx: None,
            clock_gating_bypass: false,
            fault_unclocked: std::collections::HashMap::new(),
            peripheral_ranges: Vec::new(),
            legacy_tick_indices: Vec::new(),
            bus_tick_indices: Vec::new(),
            scheduler_driver_indices: Vec::new(),
            matrix_source_scratch: Vec::new(),
            peripheral_hint: Cell::new(None),
            last_route: Cell::new(None),
            last_gap: Cell::new(None),
            last_gpio_in: [0; 2],
            current_cycle: 0,
            cycle_clock: crate::CycleClock::default(),
            pending_schedule: Vec::new(),
            freerunning_timer_poll_mmio: std::cell::Cell::new(0),
            side_effecting_mmio: std::cell::Cell::new(0),
            legacy_walk_disabled: false,
            reset_vector_offset: 0,
            atomic_register_aliases: false,
            hcsr04: Vec::new(),
            gpio_devices: Vec::new(),
            ws2812: Vec::new(),
            tm1637: Vec::new(),
            seven_segment: Vec::new(),
            analog_inputs: Vec::new(),
            can_diagnostic_testers: Vec::new(),
            can_uds_testers: Vec::new(),
            can_log_players: Vec::new(),
            esp32c3_irq_routing: false,
            riscv_irq_lines: 0,
            esp32c3_system_idx: None,
            esp32c3_interrupt_core0_idx: None,
            esp32c3_irq_cache: None,
            esp32c3_asserted_sources: [0; 2],
            esp32c3_sched_asserted_sources: [0; 2],
            esp32s3_irq_routing: false,
            esp32s3_intmatrix_idx: None,
            esp32s3_asserted_sources: [0; 2],
            esp32s3_sched_asserted_sources: [0; 2],
            flash_models_ops: false,
            iolink_master_attached: false,
            nordic_gpio_service: false,
            hcsr04_scheduling_disabled: false,
            flash_error_flags_idx: None,
            bus_trace: bus_trace::new_log(),
            logic_tap: crate::logic_capture::LogicTap::new(),
            pin_map: std::collections::HashMap::new(),
        };
        bus.rebuild_peripheral_ranges();
        bus
    }

    /// Construct an empty bus with no flash, RAM, or peripherals.
    ///
    /// Useful for tests that want to register peripherals manually without
    /// inheriting the STM32 defaults from `new()`. The flash and ram backings
    /// are zero-sized so they never satisfy a read.
    pub fn empty() -> Self {
        let mut bus = Self {
            flash_thunks: std::collections::HashMap::new(),
            flash: LinearMemory::new(0, 0),
            ram: LinearMemory::new(0, 0),
            extra_mem: Vec::new(),
            peripherals: Vec::new(),
            nvic: None,
            observers: Vec::new(),
            config: crate::SimulationConfig::default(),
            bit_band_enabled: false,
            pending_cpu_irqs: [0; 2],
            dport_idx: None,
            rcc_idx: None,
            clock_gating_bypass: false,
            fault_unclocked: std::collections::HashMap::new(),
            peripheral_ranges: Vec::new(),
            legacy_tick_indices: Vec::new(),
            bus_tick_indices: Vec::new(),
            scheduler_driver_indices: Vec::new(),
            matrix_source_scratch: Vec::new(),
            peripheral_hint: Cell::new(None),
            last_route: Cell::new(None),
            last_gap: Cell::new(None),
            last_gpio_in: [0; 2],
            current_cycle: 0,
            cycle_clock: crate::CycleClock::default(),
            pending_schedule: Vec::new(),
            freerunning_timer_poll_mmio: std::cell::Cell::new(0),
            side_effecting_mmio: std::cell::Cell::new(0),
            legacy_walk_disabled: false,
            reset_vector_offset: 0,
            atomic_register_aliases: false,
            hcsr04: Vec::new(),
            gpio_devices: Vec::new(),
            ws2812: Vec::new(),
            tm1637: Vec::new(),
            seven_segment: Vec::new(),
            analog_inputs: Vec::new(),
            can_diagnostic_testers: Vec::new(),
            can_uds_testers: Vec::new(),
            can_log_players: Vec::new(),
            esp32c3_irq_routing: false,
            riscv_irq_lines: 0,
            esp32c3_system_idx: None,
            esp32c3_interrupt_core0_idx: None,
            esp32c3_irq_cache: None,
            esp32c3_asserted_sources: [0; 2],
            esp32c3_sched_asserted_sources: [0; 2],
            esp32s3_irq_routing: false,
            esp32s3_intmatrix_idx: None,
            esp32s3_asserted_sources: [0; 2],
            esp32s3_sched_asserted_sources: [0; 2],
            flash_models_ops: false,
            iolink_master_attached: false,
            nordic_gpio_service: false,
            hcsr04_scheduling_disabled: false,
            flash_error_flags_idx: None,
            bus_trace: bus_trace::new_log(),
            logic_tap: crate::logic_capture::LogicTap::new(),
            pin_map: std::collections::HashMap::new(),
        };
        bus.rebuild_peripheral_ranges();
        bus
    }

    /// Mirror `Machine::total_cycles` into the bus AND publish it on the
    /// shared [`crate::CycleClock`] in one step, so the clock `&self`
    /// peripheral reads sync against can never skew from `current_cycle`.
    /// All engine refresh points (batch start/end, per-step, idle
    /// fast-forward) go through here.
    #[inline]
    pub fn set_current_cycle(&mut self, cycle: u64) {
        self.current_cycle = cycle;
        self.cycle_clock.publish(cycle);
    }

    /// Append a peripheral to the bus at runtime. Useful for tests and
    /// dynamic configuration that bypasses `from_config`.
    ///
    /// **No overlap check is performed.** If two peripherals claim overlapping
    /// address ranges, routing is last-start-wins (equal bases → last-registered
    /// entry). Callers are responsible for ensuring non-overlapping ranges, or
    /// for using [`replace_or_add_peripheral`] when a behavioral model should
    /// own a name already present as a declarative stub.
    pub fn add_peripheral(
        &mut self,
        name: &str,
        base: u64,
        size: u64,
        irq: Option<u32>,
        mut dev: Box<dyn Peripheral>,
    ) {
        // Attach choke point (walk-free plan Part 1): hand the peripheral the
        // bus's shared cycle clock before it is registered, so read-side lazy
        // sync is available from the first instruction.
        dev.attach_cycle_clock(self.cycle_clock.clone());
        // Twin of the `push_peripheral` attach — see there.
        dev.attach_irq_line(irq);
        self.peripherals.push(PeripheralEntry {
            name: name.to_string(),
            base,
            size,
            irq,
            dev,
            ticks_remaining: 0,
            clock_gate: None,
        });
        self.rebuild_peripheral_ranges();
    }

    /// Replace the first peripheral with `name`, or append if missing.
    ///
    /// Prefer this over [`add_peripheral`] when installing a behavioral twin
    /// of a chip-yaml declarative stub: name lookup (`find_peripheral_index_by_name`,
    /// `--watch-gpio`, inspect) and MMIO must agree on one entry. Stacking two
    /// `"rmt"` devices left TX_START on the later MMIO winner while LogicTap
    /// armed the earlier stub → `min_rmt_tx` green, `led_watch` edges zero.
    pub fn replace_or_add_peripheral(
        &mut self,
        name: &str,
        base: u64,
        size: u64,
        irq: Option<u32>,
        mut dev: Box<dyn Peripheral>,
    ) {
        dev.attach_cycle_clock(self.cycle_clock.clone());
        dev.attach_irq_line(irq);
        if let Some(idx) = self.peripherals.iter().position(|p| p.name == name) {
            let e = &mut self.peripherals[idx];
            e.base = base;
            e.size = size;
            e.irq = irq;
            e.dev = dev;
            e.ticks_remaining = 0;
            e.clock_gate = None;
            self.rebuild_peripheral_ranges();
        } else {
            // attach already done; push without double-attach
            self.peripherals.push(PeripheralEntry {
                name: name.to_string(),
                base,
                size,
                irq,
                dev,
                ticks_remaining: 0,
                clock_gate: None,
            });
            self.rebuild_peripheral_ranges();
        }
    }

    /// Look up a registered ROM thunk by absolute PC.
    ///
    /// Iterates the registered peripherals; if any is a `RomThunkBank` whose
    /// address range contains `pc`, asks it for a thunk at `pc`.  Returns
    /// `None` if no bank covers the PC or no thunk is registered.
    ///
    /// Used by the CPU's `BREAK 1, 14` dispatch in `xtensa_lx7.rs`.
    pub fn get_rom_thunk(
        &self,
        pc: u32,
    ) -> Option<crate::peripherals::esp_xtensa_common::rom_thunks::RomThunkFn> {
        // First check the Bus-level flash thunk table (for thunks installed
        // outside any RomThunkBank's range — typically firmware functions
        // resident in flash that we want to intercept).
        if let Some(&thunk) = self.flash_thunks.get(&pc) {
            return Some(thunk);
        }
        for p in &self.peripherals {
            let base = p.base as u32;
            let end = base.wrapping_add(p.size as u32);
            if pc >= base && pc < end {
                if let Some(any) = p.dev.as_any() {
                    if let Some(bank) =
                        any.downcast_ref::<crate::peripherals::esp_xtensa_common::rom_thunks::RomThunkBank>()
                    {
                        return bank.get(pc);
                    }
                }
            }
        }
        None
    }

    /// Install a thunk for `pc` outside any `RomThunkBank`. Writes
    /// `BREAK 1,14` at `pc` so instruction fetch dispatches to the
    /// CPU's break-handler path, where `get_rom_thunk(pc)` returns the
    /// supplied closure. Used to intercept firmware functions resident
    /// in flash (e.g. ESP-IDF's `multi_heap_register`).
    pub fn install_flash_thunk(
        &mut self,
        pc: u32,
        thunk: crate::peripherals::esp_xtensa_common::rom_thunks::RomThunkFn,
    ) -> SimResult<()> {
        let bytes = crate::peripherals::esp_xtensa_common::rom_thunks::ROM_THUNK_BREAK_BYTES;
        for (i, b) in bytes.iter().enumerate() {
            self.write_u8(pc as u64 + i as u64, *b)?;
        }
        self.flash_thunks.insert(pc, thunk);
        Ok(())
    }

    /// Plan 3: look up the cpu0 IRQ slot the registered intmatrix has bound
    /// to peripheral source `source_id`. Returns None if no intmatrix is
    /// registered or no binding exists for the source.
    pub fn route_irq_source_to_cpu_irq(&self, source_id: u32) -> Option<u8> {
        self.route_irq_source_to_cpu_irq_core(source_id, 0)
    }

    /// Plan 3 (SMP): look up the IRQ slot `source_id` is bound to on
    /// `core_id` (0 = PRO_CPU, 1 = APP_CPU) via the registered interrupt
    /// matrix's per-core map table. None if unregistered or unbound.
    pub fn route_irq_source_to_cpu_irq_core(&self, source_id: u32, core_id: u8) -> Option<u8> {
        let idx = self.esp32s3_intmatrix_idx?;
        self.peripherals
            .get(idx)?
            .dev
            .as_any()
            .and_then(|a| {
                a.downcast_ref::<crate::peripherals::esp32s3::intmatrix::Esp32s3IntMatrix>()
            })
            .and_then(|matrix| matrix.route_for_core(source_id, core_id))
    }

    /// Cross-core `FROM_CPU` IPI slots currently asserted for `core_id`,
    /// read live from the ESP32-classic DPORT interrupt matrix. Replaces the
    /// old test-harness IPI bridge that polled the same registers from
    /// outside the core. Returns 0 when no DPORT is mapped (non-ESP32 buses).
    pub(crate) fn dport_cross_core_pending(&self, core_id: u8) -> u32 {
        // O(1) via the index cached in `rebuild_peripheral_ranges`. No DPORT
        // (every ESP32-S3 bus) → no scan, just return 0.
        let Some(idx) = self.dport_idx else { return 0 };
        self.peripherals
            .get(idx)
            .and_then(|p| p.dev.as_any())
            .and_then(|a| a.downcast_ref::<crate::peripherals::esp32::dport::Dport>())
            .map(|dport| dport.cross_core_pending(core_id))
            .unwrap_or(0)
    }

    /// Attach a UART TX capture sink to any UART peripherals on this bus.
    ///
    /// When `echo_stdout` is false, UART writes will no longer be printed to stdout.
    pub fn attach_uart_tx_sink(&mut self, sink: Arc<Mutex<Vec<u8>>>, echo_stdout: bool) {
        use crate::peripherals::components::IolinkMaster;
        use crate::peripherals::esp32::uart::Esp32Uart;
        use crate::peripherals::esp32s3::uart::Esp32s3Uart;
        use crate::peripherals::nrf52::uarte::Nrf52Uarte;
        use crate::peripherals::nrf54l::uarte::Nrf54lUarte;
        for p in &mut self.peripherals {
            let Some(any) = p.dev.as_any_mut() else {
                continue;
            };
            // STM32-layout generic UART.
            if let Some(uart) = any.downcast_mut::<Uart>() {
                // UARTs carrying an IO-Link master are the binary IO-Link C/Q
                // wire, not a text console: their raw bytes must neither be
                // echoed to stdout nor captured into the assertion buffer (they
                // would pollute the console log and could collide with assertion
                // substrings). A freshly built `Uart` defaults to
                // `echo_stdout = true`, so we cannot simply skip it — we must
                // explicitly clear the sink AND disable the echo. The master's
                // own decoded records reach the capture sink via
                // `attach_iolink_master_log_sink`.
                let is_iolink_wire = uart
                    .attached_streams
                    .iter()
                    .any(|s| s.as_any().map(|a| a.is::<IolinkMaster>()).unwrap_or(false));
                if is_iolink_wire {
                    uart.set_sink(None, false);
                } else {
                    uart.set_sink(Some(sink.clone()), echo_stdout);
                }
                continue;
            }
            // Real ESP32-classic UART (echo is fixed at construction time).
            if let Some(uart) = any.downcast_mut::<Esp32Uart>() {
                uart.set_sink(Some(sink.clone()));
                continue;
            }
            // nRF52 UARTE console (EasyDMA): captured/echoed the same way.
            if let Some(uarte) = any.downcast_mut::<Nrf52Uarte>() {
                uarte.set_sink(Some(sink.clone()), echo_stdout);
                continue;
            }
            // nRF54L UARTE console (EasyDMA in the DMA.TX cluster). A separate
            // model from the nRF52 one — same console role, different silicon
            // register map — so it needs its own downcast arm here.
            if let Some(uarte) = any.downcast_mut::<Nrf54lUarte>() {
                uarte.set_sink(Some(sink.clone()), echo_stdout);
                continue;
            }
            // ESP32-S3 UART0 — the faithful ROM-boot console. The real mask ROM
            // and 2nd-stage bootloader print their banner/progress here, and
            // esp-hal's default `esp_println` targets UART0 too. Without this the
            // faithful S3 boot produces no captured serial (uart.log stays empty).
            // Its `echo_stdout` is fixed at construction (uart0 defaults to true;
            // the run service passes --no-uart-stdout, but only the CAPTURE sink
            // matters there), so `set_sink` only wires the capture buffer.
            if let Some(uart) = any.downcast_mut::<Esp32s3Uart>() {
                uart.set_sink(Some(sink.clone()));
                continue;
            }
            // RP2040 USB CDC: an Arduino Mbed-OS sketch's default `Serial` is
            // USB CDC, so the console text arrives on the USB bulk-IN endpoint,
            // not UART0. Route it into the same capture sink.
            if let Some(usb) = any.downcast_mut::<crate::peripherals::rp2040::usb::Rp2040Usb>() {
                usb.set_sink(Some(sink.clone()));
            }
        }
    }

    /// Wire a capture sink into any attached IO-Link master so it records what
    /// it received over IO-Link (`MASTER PD=`, `MASTER VERDICT`, `MASTER EVENT`)
    /// into the given buffer. Pass the same `Arc<Mutex<Vec<u8>>>` used for the
    /// UART-TX capture sink so `uart_contains` assertions can observe the
    /// MASTER side (not just the device console). No-op when no IO-Link master
    /// is attached.
    pub fn attach_iolink_master_log_sink(&mut self, sink: Arc<Mutex<Vec<u8>>>) {
        use crate::peripherals::components::IolinkMaster;
        for p in &mut self.peripherals {
            let Some(any) = p.dev.as_any_mut() else {
                continue;
            };
            let Some(uart) = any.downcast_mut::<Uart>() else {
                continue;
            };
            for stream in &mut uart.attached_streams {
                if let Some(sa) = stream.as_any_mut() {
                    if let Some(master) = sa.downcast_mut::<IolinkMaster>() {
                        master.set_log_sink(sink.clone());
                    }
                }
            }
        }
    }

    /// Attach a UART TX capture sink to one named UART peripheral.
    /// Returns false when no matching UART peripheral exists.
    pub fn attach_uart_tx_sink_named(
        &mut self,
        name: &str,
        sink: Arc<Mutex<Vec<u8>>>,
        echo_stdout: bool,
    ) -> bool {
        for p in &mut self.peripherals {
            let Some(any) = p.dev.as_any_mut() else {
                continue;
            };
            if let Some(uart) = any.downcast_mut::<Uart>() {
                uart.set_sink(None, false);
            }
        }

        for p in &mut self.peripherals {
            if p.name != name {
                continue;
            }
            let Some(any) = p.dev.as_any_mut() else {
                return false;
            };
            let Some(uart) = any.downcast_mut::<Uart>() else {
                return false;
            };
            uart.set_sink(Some(sink), echo_stdout);
            return true;
        }
        false
    }

    /// Collect shared RX buffer handles from all UART peripherals on this bus.
    /// The caller can push bytes into these buffers to inject serial input.
    pub fn attach_uart_rx_source(&self) -> Vec<Arc<Mutex<VecDeque<u8>>>> {
        let mut sources = Vec::new();
        for p in &self.peripherals {
            let Some(any) = p.dev.as_any() else {
                continue;
            };
            let Some(uart) = any.downcast_ref::<Uart>() else {
                continue;
            };
            sources.push(uart.rx_buffer());
        }
        sources
    }

    /// Whether this chip's core implements the Cortex-M bit-band feature.
    ///
    /// Bit-band aliasing is an optional feature of the Cortex-M3 and
    /// Cortex-M4 cores only — M0/M0+/M23/M33/M7 do not implement it.
    /// Chips without it may map real peripherals inside the would-be alias
    /// ranges (e.g. STM32H5/WBA M33 parts put GPIO at 0x4202_xxxx), so
    /// translating there would shadow those peripherals.
    ///
    /// A chip yaml without a `core` field keeps the historical default
    /// (enabled on Arm) so pre-existing third-party configs that rely on
    /// bit-band keep working; all in-tree Arm chip configs declare `core`.
    pub(crate) fn chip_has_bit_band(chip: &ChipDescriptor) -> bool {
        match chip.core.as_deref() {
            Some(core) => {
                let c = core.trim().to_ascii_lowercase();
                let c = c.strip_prefix("cortex-").unwrap_or(&c);
                matches!(c, "m3" | "m4" | "m4f")
            }
            None => matches!(chip.arch, labwired_config::Arch::Arm),
        }
    }

    /// Decode an RP2040 atomic register-alias access. Returns the aligned base
    /// register address and the atomic op when `addr` lands on a `+0x1000`
    /// (XOR), `+0x2000` (SET) or `+0x3000` (CLR) alias of a peripheral register
    /// in the APB/AHB-Lite peripheral window; `None` for a normal (`+0x0000`)
    /// access or any address outside the window. Only consulted when
    /// `atomic_register_aliases` is set, so it is a no-op for other parts.
    #[inline]
    pub fn atomic_alias_redirect(&self, addr: u64) -> Option<(u64, AtomicAliasOp)> {
        const APB_AHB: std::ops::Range<u64> = 0x4000_0000..0x5040_0000;
        if !APB_AHB.contains(&addr) {
            return None;
        }
        let op = match (addr >> 12) & 0x3 {
            0 => return None,
            1 => AtomicAliasOp::Xor,
            2 => AtomicAliasOp::Set,
            _ => AtomicAliasOp::Clr,
        };
        Some((addr & !0x3000, op))
    }

    /// Whether the 8 bytes at `addr` form a plausible Cortex-M reset vector:
    /// word[0] (the initial SP) points into RAM and word[1] (the initial PC)
    /// points into flash. Used by `Machine::load_firmware` to decide whether a
    /// candidate vector table (flash base vs. post-stage-2 offset) is the real
    /// one, so a second-stage bootloader (RP2040 boot2) can be skipped.
    pub fn vector_pair_valid(&self, addr: u64) -> bool {
        let (Some(sp), Some(pc)) = (
            self.read_u32(addr).ok(),
            self.read_u32(addr.wrapping_add(4)).ok(),
        ) else {
            return false;
        };
        let pc = pc & !1; // strip the Thumb bit
                          // The initial SP is the top of the full-descending stack, conventionally
                          // one past the last RAM byte (ram.base + ram.size), so the upper bound
                          // is inclusive.
        let in_ram = (sp as u64) >= self.ram.base_addr
            && (sp as u64) <= self.ram.base_addr + self.ram.data.len() as u64;
        let in_flash = (pc as u64) >= self.flash.base_addr
            && (pc as u64) < self.flash.base_addr + self.flash.data.len() as u64;
        in_ram && in_flash
    }

    /// Place a built peripheral on the bus using the descriptor's window size
    /// (default 4KB) and IRQ. Shared by the per-family factory dispatch and the
    /// generic-match path in [`Self::from_config`] so both stay in lockstep.
    pub(crate) fn push_peripheral(
        &mut self,
        p_cfg: &labwired_config::PeripheralConfig,
        mut dev: Box<dyn Peripheral>,
    ) -> anyhow::Result<()> {
        let size = match &p_cfg.size {
            Some(size) => parse_size(size)?,
            None => 0x1000,
        };
        // Attach choke point (walk-free plan Part 1): hand the peripheral the
        // bus's shared cycle clock before it is registered — the `from_config`
        // twin of the same attach in `add_peripheral`, so descriptor-built
        // models (SysTick et al.) get read-side lazy sync too. No-op for the
        // vast majority of models (the default `attach_cycle_clock` discards).
        dev.attach_cycle_clock(self.cycle_clock.clone());
        // Same choke point, same contract: tell the model whether its own-IRQ is
        // wired, so one whose only per-cycle wakeup holds a level-triggered IRQ
        // can stop scheduling itself on a bus where that pend is dropped.
        dev.attach_irq_line(p_cfg.irq);
        self.peripherals.push(PeripheralEntry {
            name: p_cfg.id.clone(),
            base: p_cfg.base_address,
            size,
            irq: p_cfg.irq,
            dev,
            ticks_remaining: 0,
            // Resolved in a post-pass once every peripheral (incl. the RCC) is
            // on the bus — see `resolve_clock_gates`.
            clock_gate: None,
        });
        Ok(())
    }

    /// Resolve every peripheral's optional `clock: { reg, bit }` declaration into
    /// a concrete [`ResolvedClockGate`] (RCC register offset + bit). Run as a
    /// post-pass by `from_config` after all peripherals — crucially the RCC —
    /// are on the bus, so the symbolic `reg` name can be mapped to the active
    /// chip family's RCC offset via [`Rcc::enable_reg_offset`] regardless of the
    /// order peripherals appear in the config.
    ///
    /// A peripheral with no `clock` field is left ungated. A declared gate whose
    /// `reg` name the family doesn't recognise is a hard config error (a silent
    /// "never gate" would mask a typo that lets unclocked firmware falsely pass).
    pub(crate) fn resolve_clock_gates(
        &mut self,
        peripherals: &[labwired_config::PeripheralConfig],
    ) -> anyhow::Result<()> {
        // Find the RCC model once (clock-gating requires one).
        let rcc_off = |bus: &SystemBus, reg: &str| -> Option<u64> {
            let idx = bus.rcc_idx?;
            bus.peripherals[idx]
                .dev
                .as_any()
                .and_then(|a| a.downcast_ref::<crate::peripherals::rcc::Rcc>())
                .and_then(|rcc| rcc.enable_reg_offset(reg))
        };
        for p_cfg in peripherals {
            let Some(gate) = &p_cfg.clock else { continue };
            let Some(idx) = self.find_peripheral_index_by_name(&p_cfg.id) else {
                continue;
            };
            let Some(reg_offset) = rcc_off(self, &gate.reg) else {
                return Err(anyhow::anyhow!(
                    "peripheral '{}' declares clock gate reg '{}' which the chip's \
                     RCC model does not expose (no such enable register, or no RCC \
                     peripheral is registered)",
                    p_cfg.id,
                    gate.reg
                ));
            };
            self.peripherals[idx].clock_gate = Some(ResolvedClockGate {
                reg_offset,
                bit: gate.bit,
            });
        }
        Ok(())
    }

    pub fn signal_nvic_irq(&self, irq: u32) {
        if let Some(nvic) = &self.nvic {
            if irq >= 16 {
                let idx = (irq / 32) as usize;
                let bit = irq % 32;
                if idx < 8 {
                    nvic.ispr[idx].fetch_or(1 << bit, Ordering::SeqCst);
                }
            } else {
                // Core exceptions are handled differently if needed,
                // but signal_nvic_irq is mostly for external IRQs.
                tracing::warn!("signal_nvic_irq called for core exception {}", irq);
            }
        }
    }
}
