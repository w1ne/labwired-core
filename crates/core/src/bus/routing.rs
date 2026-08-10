// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Address routing (peripheral-window lookup, bit-band) + GPIO pin resolution.
//! Split out of `bus/mod.rs`.

use super::*;

impl SystemBus {
    /// Parse a pin label into `(gpio peripheral id, bit)`. Accepts the STM32
    /// form "PC7" -> `("gpioc", 7)` and the Nordic form "P0.04" / "P1.15" ->
    /// `("gpio0", 4)` / `("gpio1", 15)`.
    pub(crate) fn parse_stm32_pin(pin: &str) -> Option<(String, u8)> {
        let s = pin.trim();
        let bytes = s.as_bytes();
        if bytes.len() < 3 || !bytes[0].eq_ignore_ascii_case(&b'P') {
            return None;
        }
        // Nordic ports are numbered and dot-separated; nRF52840 P0 has 32 pins.
        if let Some((port, num)) = s[1..].split_once('.') {
            let port: u8 = port.parse().ok()?;
            let num: u8 = num.parse().ok()?;
            if num > 31 {
                return None;
            }
            return Some((format!("gpio{port}"), num));
        }
        let port = (bytes[1] as char).to_ascii_lowercase();
        if !port.is_ascii_alphabetic() {
            return None;
        }
        let num: u8 = s[2..].parse().ok()?;
        if num > 15 {
            return None;
        }
        Some((format!("gpio{port}"), num))
    }

    /// Resolve an STM32 pin label to its `(ODR address, bit)` so a display's
    /// D/C line can be sampled directly from the driving GPIO's output register.
    /// Public wrapper exposed via [`AttachCtx::resolve_pin_odr`] so kits can
    /// hook MCU GPIO outputs into a SPI device's D/C line.
    pub fn resolve_pin_odr_pub(bus: &SystemBus, pin: &str) -> Option<(u64, u8)> {
        Self::resolve_pin_odr(bus, pin)
    }

    pub(crate) fn resolve_pin_odr(bus: &SystemBus, pin: &str) -> Option<(u64, u8)> {
        // 1. Chip-declared pin map is authoritative silicon truth. When a chip
        //    declares any pins, resolution goes THROUGH the map — an undeclared pin
        //    returns None rather than silently letter-parsing onto a wrong port.
        if !bus.pin_map.is_empty() {
            let (gpio_name, bit) = bus.pin_map.get(&pin.to_ascii_uppercase())?;
            let idx = bus.find_peripheral_index_by_name(gpio_name)?;
            let base = bus.peripherals[idx].base;
            let odr_off = bus.peripherals[idx]
                .dev
                .as_any()
                .and_then(|a| a.downcast_ref::<crate::peripherals::gpio::GpioPort>())
                .map(|g| g.odr_offset())?;
            return Some((base + odr_off, *bit));
        }
        // 2. No chip pin map → standard STM32/Nordic label parse.
        // STM32/Nordic: "PA5" / "P0.13" → per-port GpioPort with an ODR offset.
        if let Some((port_name, bit)) = Self::parse_stm32_pin(pin) {
            if let Some(idx) = bus.find_peripheral_index_by_name(&port_name) {
                let base = bus.peripherals[idx].base;
                if let Some(odr_off) = bus.peripherals[idx]
                    .dev
                    .as_any()
                    .and_then(|a| a.downcast_ref::<crate::peripherals::gpio::GpioPort>())
                    .map(|g| g.odr_offset())
                {
                    return Some((base + odr_off, bit));
                }
            }
        }
        // ESP32-family GPIO labels resolve against the single `gpio` block.
        if let Some(idx) = bus.find_peripheral_index_by_name("gpio") {
            let any = bus.peripherals[idx].dev.as_any();
            let is_classic_or_c3 = any
                .map(|a| {
                    a.downcast_ref::<crate::peripherals::esp32::gpio::Esp32Gpio>()
                        .is_some()
                        || a.downcast_ref::<crate::peripherals::esp32c3::gpio::Esp32c3Gpio>()
                            .is_some()
                })
                .unwrap_or(false);
            let is_s3 = any
                .map(|a| {
                    a.downcast_ref::<crate::peripherals::esp32s3::gpio::Esp32s3Gpio>()
                        .is_some()
                })
                .unwrap_or(false);
            let bit = if is_s3 {
                Self::parse_esp32s3_gpio_pin(pin)
            } else if is_classic_or_c3 {
                Self::parse_esp32_gpio_pin(pin)
            } else {
                None
            };
            if let Some(bit) = bit {
                let (offset, bit) = if bit >= 32 {
                    (0x10, bit - 32)
                } else {
                    (0x04, bit)
                };
                return Some((bus.peripherals[idx].base + offset, bit));
            }
        }
        None
    }

    /// Parse an ESP32 GPIO label ("GPIO17", "gpio17", "IO17", or a bare "17")
    /// into its bit number in GPIO_OUT_REG. Only the low bank (0..31) is
    /// modeled, matching [`Esp32Gpio`](crate::peripherals::esp32::gpio::Esp32Gpio).
    pub(crate) fn parse_esp32_gpio_pin(pin: &str) -> Option<u8> {
        let s = pin.trim();
        let digits = s
            .trim_start_matches(|c: char| c.is_ascii_alphabetic())
            .trim();
        let num: u8 = digits.parse().ok()?;
        // Classic ESP32 pads run 0..=39. The high bank (32..39) lives in
        // OUT1/ENABLE1 and is modelled, so it must resolve too — capping at 31
        // here made every high-bank pad unresolvable, and a device wired to one
        // (an Adafruit TFT FeatherWing puts D/C on GPIO33) silently lost its
        // control line: `set_dc_source` was never called, so the bus never
        // latched D/C and the panel framed every byte as a command.
        (num <= 39).then_some(num)
    }

    /// Parse an ESP32-S3 GPIO label ("GPIO48", "gpio8", "IO17", or a bare "48")
    /// into its pin number, accepting both banks (0..=48 — GPIO48 is the onboard
    /// NeoPixel on most S3 boards). Unlike [`parse_esp32_gpio_pin`] (classic
    /// ESP32, low bank only) this reaches bank 1, matching
    /// [`Esp32s3Gpio`](crate::peripherals::esp32s3::gpio::Esp32s3Gpio)'s
    /// `drive_pad_output` range.
    pub(crate) fn parse_esp32s3_gpio_pin(pin: &str) -> Option<u8> {
        let digits = pin
            .trim()
            .trim_start_matches(|c: char| c.is_ascii_alphabetic())
            .trim();
        let num: u8 = digits.parse().ok()?;
        (num <= 48).then_some(num)
    }

    /// Public wrapper for kits that drive MCU inputs (HX711 DT, etc.).
    pub fn resolve_pin_idr_pub(bus: &SystemBus, pin: &str) -> Option<(u64, u8)> {
        Self::resolve_pin_idr(bus, pin)
    }

    /// Hold an MCU input pin at `level`, for device status lines the firmware
    /// polls but nothing else drives (e-paper BUSY, sensor DRDY). Returns false
    /// if the pin does not resolve or the owning GPIO block rejects it.
    ///
    /// Goes through `set_gpio_input` rather than an MMIO store because input
    /// registers ignore writes — that is what makes them inputs. Shared by the
    /// kit `AttachCtx` and the ESP32 external-device factory so both wire a
    /// status line the same way on every chip.
    pub fn drive_pin_input(bus: &mut SystemBus, pin: &str, level: bool) -> bool {
        let Some((addr, bit)) = Self::resolve_pin_idr(bus, pin) else {
            return false;
        };
        bus.drive_input_bit(addr, bit, level)
    }

    /// Address-keyed twin of [`drive_pin_input`](Self::drive_pin_input), for
    /// callers that already hold a resolved `(IDR address, bit)` — a
    /// bus-resident device servicing the pin it was attached to, rather than a
    /// caller starting from a pad label. Same external-world seam
    /// (`set_gpio_input`), so both routes agree on every chip.
    pub fn drive_input_bit(&mut self, addr: u64, bit: u8, level: bool) -> bool {
        let Some(idx) = self
            .peripherals
            .iter()
            .position(|p| addr >= p.base && addr < p.base + p.size)
        else {
            return false;
        };
        self.peripherals[idx].dev.set_gpio_input(bit, level)
    }

    /// Resolve a pin label to its `(IDR address, bit)` so a sensor can
    /// drive an MCU input line (e.g. the HC-SR04 ECHO pin).
    ///
    /// Mirrors [`resolve_pin_odr`]: chip pin-map first, then STM32/Nordic pad
    /// labels, then ESP32 `GPIO`n / bare numbers into the single `gpio` block's
    /// input register. Without the ESP path, HC-SR04 on ESP32-C3 falls through
    /// to the STM32 default `PA9` and fails config.
    pub(crate) fn resolve_pin_idr(bus: &SystemBus, pin: &str) -> Option<(u64, u8)> {
        if !bus.pin_map.is_empty() {
            if let Some((gpio_name, bit)) = bus.pin_map.get(&pin.to_ascii_uppercase()) {
                let idx = bus.find_peripheral_index_by_name(gpio_name)?;
                let base = bus.peripherals[idx].base;
                let idr_off = bus.peripherals[idx]
                    .dev
                    .as_any()
                    .and_then(|a| a.downcast_ref::<crate::peripherals::gpio::GpioPort>())
                    .map(|g| g.idr_offset())?;
                return Some((base + idr_off, *bit));
            }
            // Label not in map — fall through (ESP bare/GPIO labels, etc.).
        }
        if let Some((port_name, bit)) = Self::parse_stm32_pin(pin) {
            if let Some(idx) = bus.find_peripheral_index_by_name(&port_name) {
                let base = bus.peripherals[idx].base;
                if let Some(idr_off) = bus.peripherals[idx]
                    .dev
                    .as_any()
                    .and_then(|a| a.downcast_ref::<crate::peripherals::gpio::GpioPort>())
                    .map(|g| g.idr_offset())
                {
                    return Some((base + idr_off, bit));
                }
            }
        }
        // ESP32 / ESP32-C3: "GPIO5", "gpio5", "IO5", or bare "5" → gpio peripheral IN reg.
        if let Some(bit) = Self::parse_esp32_gpio_pin(pin) {
            let idx = bus.find_peripheral_index_by_name("gpio")?;
            let is_esp32 = bus.peripherals[idx]
                .dev
                .as_any()
                .map(|a| {
                    a.downcast_ref::<crate::peripherals::esp32::gpio::Esp32Gpio>()
                        .is_some()
                        || a.downcast_ref::<crate::peripherals::esp32c3::gpio::Esp32c3Gpio>()
                            .is_some()
                })
                .unwrap_or(false);
            if is_esp32 {
                // TRM: GPIO_IN_REG at base+0x3C on classic ESP32; C3 uses the same
                // Esp32c3Gpio path — prefer the peripheral's idr_offset when present.
                if let Some(idr_off) = bus.peripherals[idx]
                    .dev
                    .as_any()
                    .and_then(|a| a.downcast_ref::<crate::peripherals::gpio::GpioPort>())
                    .map(|g| g.idr_offset())
                {
                    return Some((bus.peripherals[idx].base + idr_off, bit));
                }
                // Esp32c3Gpio is not GpioPort — use known IN register offset (0x3C / matches ODR+delta style).
                // Esp32c3 GPIO_IN_REG is at 0x3C from GPIO base (TRM); OUT is 0x04.
                const GPIO_IN_REG_OFFSET: u64 = 0x3C;
                return Some((bus.peripherals[idx].base + GPIO_IN_REG_OFFSET, bit));
            }
        }
        None
    }

    pub(crate) fn is_peripheral_addr(p: &PeripheralEntry, addr: u64) -> bool {
        addr >= p.base && addr < p.base + p.size
    }

    pub(crate) fn rebuild_peripheral_ranges(&mut self) {
        self.peripheral_ranges = self
            .peripherals
            .iter()
            .enumerate()
            .map(|(index, p)| PeripheralRange {
                start: p.base,
                end: p.base.saturating_add(p.size),
                index,
            })
            .collect();
        self.peripheral_ranges.sort_by_key(|r| r.start);
        self.legacy_tick_indices = self
            .peripherals
            .iter()
            .enumerate()
            .filter_map(|(index, p)| Self::legacy_tick_index_active(p).then_some(index))
            .collect();
        self.bus_tick_indices = self
            .peripherals
            .iter()
            .enumerate()
            .filter_map(|(index, p)| p.dev.needs_bus_tick().then_some(index))
            .collect();
        self.scheduler_driver_indices = self
            .peripherals
            .iter()
            .enumerate()
            .filter_map(|(index, p)| p.dev.uses_scheduler().then_some(index))
            .collect();
        self.peripheral_hint.set(None);
        self.last_route.set(None);
        self.last_gap.set(None);
        // Cache the DPORT index (classic-ESP32 only) so the per-step
        // cross-core IPI read is O(1) instead of scanning every peripheral.
        self.dport_idx = self.peripherals.iter().position(|p| {
            p.dev
                .as_any()
                .and_then(|a| a.downcast_ref::<crate::peripherals::esp32::dport::Dport>())
                .is_some()
        });
        // Cache the "rcc" peripheral index so the clock-gate check on the hot
        // read/write path is O(1). Matched by id, as the clock-gate config
        // references the RCC by the conventional "rcc" peripheral id.
        self.rcc_idx = self.peripherals.iter().position(|p| p.name == "rcc");
        // Cache whether any FLASH peripheral models hardware ops (H5 erase /
        // bank swap). Those ops are recorded as pending and must be drained and
        // applied per instruction, which only holds under cycle-accurate
        // execution — so `requires_cycle_accurate` reads this cached bool
        // instead of scanning peripherals on every run-loop iteration.
        self.flash_models_ops = self.peripherals.iter().any(|p| {
            p.dev
                .as_any()
                .and_then(|a| a.downcast_ref::<crate::peripherals::flash::Flash>())
                .is_some_and(|f| f.models_ops())
        });
        // Cache the index of a FLASH peripheral whose opt-in H5 program-error
        // gate is on, so the flash-region write path can validate programs
        // without scanning. `None` (gate off) ⇒ that path is unchanged.
        self.flash_error_flags_idx = self.peripherals.iter().position(|p| {
            p.dev
                .as_any()
                .and_then(|a| a.downcast_ref::<crate::peripherals::flash::Flash>())
                .is_some_and(|f| f.h5_error_flags_enabled())
        });
        // Cache the nRF52 NVMC (if this chip has one): the flash-region write
        // path consults it for Wen gating + 1→0 AND semantics.
        self.nrf52_nvmc_idx = self.peripherals.iter().position(|p| {
            p.dev
                .as_any()
                .and_then(|a| a.downcast_ref::<crate::peripherals::nrf52::nvmc::Nrf52Nvmc>())
                .is_some()
        });
        self.esp32c3_system_idx = self
            .peripherals
            .iter()
            .position(|p| p.name == "system" && p.base == 0x600C_0000);
        self.esp32c3_interrupt_core0_idx = self
            .peripherals
            .iter()
            .position(|p| p.name == "interrupt_core0" && p.base == 0x600C_2000);
        self.rebuild_esp32c3_irq_cache();
        // The C3 permission-control unit lives in the SENSITIVE block; its
        // model is a derived cache of that block's registers, so it is rebuilt
        // whenever the peripheral list changes.
        self.rebuild_esp32c3_pms();
        self.esp32s3_intmatrix_idx = self.peripherals.iter().position(|p| {
            p.dev
                .as_any()
                .and_then(|a| {
                    a.downcast_ref::<crate::peripherals::esp32s3::intmatrix::Esp32s3IntMatrix>()
                })
                .is_some()
        });
        self.esp32s3_irq_routing = self.esp32s3_intmatrix_idx.is_some();
        // Cache whether the per-cycle GPIO-edge/GPIOTE service pass has any
        // Nordic port to scan, so `tick_peripherals_fully` can early-out on
        // walk-free buses without scanning peripherals by name every cycle.
        self.nordic_gpio_service = self.find_peripheral_index_by_name("gpio0").is_some()
            || self.find_peripheral_index_by_name("gpio1").is_some();
    }

    pub(crate) fn rebuild_esp32c3_irq_cache(&mut self) {
        let Some(int_idx) = self.esp32c3_interrupt_core0_idx else {
            self.esp32c3_irq_cache = None;
            return;
        };

        let mut cache = crate::bus::Esp32c3IrqCache {
            int_enable: self
                .read_cached_declarative_u32(int_idx, 0x104)
                .unwrap_or(0),
            int_thresh: (self
                .read_cached_declarative_u32(int_idx, 0x194)
                .unwrap_or(0)
                & 0xF) as u8,
            ..Default::default()
        };

        for src in 0..cache.source_line.len() {
            cache.source_line[src] = (self
                .read_cached_declarative_u32(int_idx, (src as u64) * 4)
                .unwrap_or(0)
                & 0x1F) as u8;
        }
        for line in 0..cache.line_pri.len() {
            cache.line_pri[line] = (self
                .read_cached_declarative_u32(int_idx, 0x114 + (line as u64) * 4)
                .unwrap_or(0)
                & 0xF) as u8;
        }

        if let Some(system_idx) = self.esp32c3_system_idx {
            for n in 0..4 {
                let offset = 0x28 + (n as u64) * 4;
                if self
                    .read_cached_declarative_u32(system_idx, offset)
                    .unwrap_or(0)
                    & 1
                    != 0
                {
                    cache.from_cpu_pending |= 1 << n;
                }
            }
        }

        self.esp32c3_irq_cache = Some(cache);
        // Keep the routed line mask coherent with the rebuilt cache (no-op
        // unless C3 routing is active — `recompute` early-outs without it).
        if self.esp32c3_irq_routing {
            self.recompute_esp32c3_irq_lines();
        }
    }

    pub(crate) fn sync_esp32c3_irq_cache_write(&mut self, idx: usize, offset: u64) {
        // Walk-free de-assert glue (the last-walker bus addition): once the C3
        // bus is walk-DELETED (every peripheral migrated → `legacy_walk_disabled`)
        // the per-cycle walk no longer re-derives scheduler-driven peripheral
        // LEVELS each tick (`aggregate_esp32c3_irqs` stops running on the trivial
        // tick path). A write-armed level — an INT_RAW set by a transaction, or
        // the MAC event, and above all the acknowledge that CLEARS it
        // (INT_CLR / EVENT_CLR) — must therefore re-derive the routed line mask
        // AT THE WRITE, or a level would latch forever and re-enter its ISR. Do
        // it here, at the shared MMIO write choke, but ONLY for a
        // scheduler-driven peripheral (an ordinary register write pays nothing).
        //
        // Gated on `legacy_walk_disabled`: on a walk-ON bus the per-tick
        // aggregation already owns level derivation and `esp32c3_asserted_sources`
        // carries the walk-emitted sources — recomputing mid-instruction from
        // that (stale until the next tick rebuilds it) would perturb routing, so
        // the choke stays off there. On a walk-DELETED bus the walk never runs,
        // so `esp32c3_asserted_sources` is inert and the recompute is the clean,
        // authoritative level derivation. `recompute_esp32c3_irq_lines` also
        // no-ops without the INTC cache, keeping this inert on hand-built buses.
        #[cfg(feature = "event-scheduler")]
        if self.legacy_walk_disabled
            && self.esp32c3_irq_routing
            && self
                .peripherals
                .get(idx)
                .is_some_and(|p| p.dev.uses_scheduler())
        {
            self.refresh_esp32c3_sched_sources();
            self.recompute_esp32c3_irq_lines();
        }

        if self.esp32c3_irq_cache.is_none() {
            return;
        }

        let aligned = offset & !3;
        let Some(value) = self.read_cached_declarative_u32(idx, aligned) else {
            return;
        };

        let mut inputs_changed = false;
        if Some(idx) == self.esp32c3_interrupt_core0_idx {
            if let Some(cache) = &mut self.esp32c3_irq_cache {
                inputs_changed = true;
                match aligned {
                    0x104 => cache.int_enable = value,
                    0x194 => cache.int_thresh = (value & 0xF) as u8,
                    0x114..=0x190 if (aligned - 0x114) % 4 == 0 => {
                        let line = ((aligned - 0x114) / 4) as usize;
                        if let Some(pri) = cache.line_pri.get_mut(line) {
                            *pri = (value & 0xF) as u8;
                        }
                    }
                    off if off % 4 == 0 => {
                        let src = (off / 4) as usize;
                        if let Some(line) = cache.source_line.get_mut(src) {
                            *line = (value & 0x1F) as u8;
                        }
                    }
                    _ => {}
                }
            }
        } else if Some(idx) == self.esp32c3_system_idx && (0x28..=0x34).contains(&aligned) {
            let slot = ((aligned - 0x28) / 4) as u8;
            if slot < 4 {
                if let Some(cache) = &mut self.esp32c3_irq_cache {
                    inputs_changed = true;
                    if value & 1 != 0 {
                        cache.from_cpu_pending |= 1 << slot;
                    } else {
                        cache.from_cpu_pending &= !(1 << slot);
                    }
                }
            }
        }

        // Write-choke re-aggregation: a routing-input change (INTC config or
        // FROM_CPU IPI) updates `riscv_irq_lines` at the write instruction
        // instead of waiting for the next peripheral tick. At interval 1 the
        // tick-end rebuild recomputes the same mask before the CPU's next
        // interrupt check (byte-identical); at interval > 1 this removes the
        // up-to-one-interval delivery latency for yield/critical-section
        // transitions and lets a walk-free C3 bus keep IPI routing correct
        // with no per-cycle aggregation at all.
        if inputs_changed && self.esp32c3_irq_routing {
            self.recompute_esp32c3_irq_lines();
        }
    }

    pub fn refresh_peripheral_index(&mut self) {
        self.rebuild_peripheral_ranges();
    }

    pub(crate) fn refresh_legacy_tick_index(&mut self, idx: usize) -> bool {
        // Walk-deleted buses never consult `legacy_tick_indices` (the walk is
        // off). Skip the trait calls + linear index scan on every MMIO write —
        // the post-#555 C3 OLED hot path was spending more samples here than
        // in the write itself once fetch/route optims landed.
        if self.legacy_walk_disabled {
            return false;
        }
        let active = self
            .peripherals
            .get(idx)
            .is_some_and(Self::legacy_tick_index_active);
        let pos = self.legacy_tick_indices.iter().position(|&i| i == idx);
        match (active, pos) {
            (true, None) => {
                self.legacy_tick_indices.push(idx);
                self.legacy_tick_indices.sort_unstable();
            }
            (false, Some(pos)) => {
                self.legacy_tick_indices.swap_remove(pos);
            }
            _ => {}
        }
        active
    }

    pub(crate) fn refresh_bus_tick_index(&mut self, idx: usize) -> bool {
        // Fast reject: most C3 models never arm the bus-tick path. Avoid the
        // linear membership scan when the peripheral still reports inactive
        // and is not already tracked (the common steady-state write).
        let Some(p) = self.peripherals.get(idx) else {
            return false;
        };
        let active = p.dev.needs_bus_tick();
        let pos = self.bus_tick_indices.iter().position(|&i| i == idx);
        match (active, pos) {
            (true, None) => {
                self.bus_tick_indices.push(idx);
                self.bus_tick_indices.sort_unstable();
            }
            (false, Some(pos)) => {
                self.bus_tick_indices.remove(pos);
            }
            (false, None) => return false,
            _ => {}
        }
        active
    }

    pub(crate) fn find_peripheral_index(&self, addr: u64) -> Option<usize> {
        // Canonical routing: among the windows CONTAINING `addr`, the one
        // with the GREATEST start wins (last-start-wins; equal starts resolve
        // to the last-registered entry). This makes routing a pure function
        // of the address.
        //
        // Hot path: reuse the last winning window when `addr` still falls
        // inside it AND no narrower (greater-start) window steals `addr`.
        // When the next sorted range starts after `addr`, that check is O(1).
        if self.peripheral_ranges.len() == self.peripherals.len() {
            // Negative cache: a known peripheral-free gap answers misses O(1).
            if let Some((gs, ge)) = self.last_gap.get() {
                if addr >= gs && addr < ge {
                    self.peripheral_hint.set(None);
                    return None;
                }
            }
            if let Some((ord, start, end, index)) = self.last_route.get() {
                if addr >= start && addr < end {
                    let next_start = self
                        .peripheral_ranges
                        .get(ord + 1)
                        .map(|r| r.start)
                        .unwrap_or(u64::MAX);
                    if next_start > addr {
                        // No window has start in (start, addr] — ours still wins.
                        self.peripheral_hint.set(Some(index));
                        return Some(index);
                    }
                    // Possible nesting: only scan (ord, pos] for a stealer.
                    if !self.narrower_after(ord, addr) {
                        self.peripheral_hint.set(Some(index));
                        return Some(index);
                    }
                }
            }
        }

        let mut idx = None;
        let mut won: Option<(usize, u64, u64)> = None; // (range_ord, start, end)
        if self.peripheral_ranges.len() == self.peripherals.len() {
            let pos = self
                .peripheral_ranges
                .partition_point(|range| range.start <= addr);
            // Walk backwards through the candidate starts: the nearest
            // (greatest-start) window may have already ENDED below `addr`
            // while a broader, earlier-started window still covers it.
            for (ord, range) in self.peripheral_ranges[..pos].iter().enumerate().rev() {
                if addr < range.end {
                    idx = Some(range.index);
                    won = Some((ord, range.start, range.end));
                    break;
                }
            }
        } else {
            // Ranges index stale (mid-mutation, defensive only): validated
            // hint first, then a scan matching the canonical tie-break
            // (greatest base; max_by_key keeps the LAST of equal maxima,
            // i.e. the last-registered entry).
            idx = self.peripheral_hint.get().filter(|&i| {
                self.peripherals
                    .get(i)
                    .is_some_and(|p| Self::is_peripheral_addr(p, addr))
            });
            if idx.is_none() {
                idx = self
                    .peripherals
                    .iter()
                    .enumerate()
                    .filter(|(_, p)| Self::is_peripheral_addr(p, addr))
                    .max_by_key(|&(_, p)| p.base)
                    .map(|(i, _)| i);
            }
        }

        self.peripheral_hint.set(idx);
        if let (Some(i), Some((ord, s, e))) = (idx, won) {
            self.last_route.set(Some((ord, s, e, i)));
        } else {
            self.last_route.set(None);
            // Miss: cache the surrounding peripheral-free gap. `pos` ranges
            // start <= addr and (since none covered `addr`) all end <= addr,
            // so the gap floor is their greatest end; the ceiling is the next
            // range's start. No window can cover any address in between.
            if self.peripheral_ranges.len() == self.peripherals.len() {
                let pos = self
                    .peripheral_ranges
                    .partition_point(|range| range.start <= addr);
                let floor = self.peripheral_ranges[..pos]
                    .iter()
                    .map(|r| r.end)
                    .max()
                    .unwrap_or(0);
                let ceil = self
                    .peripheral_ranges
                    .get(pos)
                    .map(|r| r.start)
                    .unwrap_or(u64::MAX);
                if floor <= addr && addr < ceil {
                    self.last_gap.set(Some((floor, ceil)));
                }
            }
        }
        idx
    }

    /// True if some range after `outer_ord` with `start > outer_start` covers
    /// `addr` (would beat the cached window under greatest-start-wins).
    fn narrower_after(&self, outer_ord: usize, addr: u64) -> bool {
        for range in self.peripheral_ranges.iter().skip(outer_ord + 1) {
            if range.start > addr {
                break;
            }
            // Containment is the whole test. Every range after `outer_ord` has
            // `start >= outer_start` (the sort is stable), and the loop above
            // already established `range.start <= addr` — so any such range
            // covering `addr` beats the cached one, by a greater start OR by
            // being the later-registered entry at an EQUAL start.
            //
            // This used to require `range.start > outer_start`, which silently
            // excluded the equal-start case and made the cached path disagree
            // with the cold path's backwards walk: `find_peripheral_index`
            // returned the later-registered narrow twin on a cold bus and the
            // broad window once anything else had been routed first. Same
            // address, same bus, answer depending on query history — while this
            // function's own contract is that routing is a pure function of the
            // address. Equal starts are exactly the SYSCON/apb_ctrl case.
            if addr < range.end {
                return true;
            }
        }
        false
    }

    pub fn find_peripheral_index_by_name(&self, name: &str) -> Option<usize> {
        self.peripherals.iter().position(|p| p.name == name)
    }

    /// **The** lab-air bind path (browser multi-chip + CLI private air).
    ///
    /// - nRF / BLE hear each other on the virtual airs
    /// - BG770A shares the nRF [`RfMedium`] for path-loss CSQ and the
    ///   [`SimMqttFabric`](crate::network::SimMqttFabric) for topic fan-out
    ///
    /// `node_id` labels radios / UE pose. Calling again rebinds to a new air
    /// (playground shared air replaces CLI private air deliberately).
    pub fn attach_lab_air(
        &mut self,
        node_id: &str,
        nrf_air: crate::peripherals::nrf52::radio::VirtualAirBus,
        ble_air: crate::peripherals::ble_air::BleAirBus,
        cellular: crate::network::SimMqttFabric,
    ) {
        use crate::peripherals::components::QuectelBg770a;
        use crate::peripherals::esp32c3::bt::Esp32c3Bt;
        use crate::peripherals::nrf52::radio::Nrf52Radio;
        use crate::peripherals::uart::Uart;
        let medium = nrf_air.medium_slot();
        for entry in &mut self.peripherals {
            if let Some(radio) = entry
                .dev
                .as_any_mut()
                .and_then(|a| a.downcast_mut::<Nrf52Radio>())
            {
                radio.set_air(nrf_air.clone());
                radio.set_node_id(node_id);
            }
            if let Some(bt) = entry
                .dev
                .as_any_mut()
                .and_then(|a| a.downcast_mut::<Esp32c3Bt>())
            {
                bt.set_air(ble_air.clone());
            }
            if let Some(uart) = entry
                .dev
                .as_any_mut()
                .and_then(|a| a.downcast_mut::<Uart>())
            {
                for stream in uart.attached_streams.iter_mut() {
                    if let Some(modem) = stream
                        .as_any_mut()
                        .and_then(|a| a.downcast_mut::<QuectelBg770a>())
                    {
                        // One AirBus story: path-loss medium + cellular MQTT fabric.
                        modem.share_medium_slot(medium.clone());
                        modem.set_mqtt_net(cellular.clone());
                        // Always label the UE with the lab node id so multi-node
                        // fabric endpoints differ (device id is often both "modem").
                        modem.set_rf_node_id(node_id);
                    }
                }
            }
        }
    }

    /// True if any Quectel BG770A modem stream is attached (needs lab AirBus).
    pub fn has_cellular_modem(&self) -> bool {
        use crate::peripherals::components::QuectelBg770a;
        use crate::peripherals::uart::Uart;
        for entry in &self.peripherals {
            let Some(any) = entry.dev.as_any() else {
                continue;
            };
            let Some(uart) = any.downcast_ref::<Uart>() else {
                continue;
            };
            for stream in &uart.attached_streams {
                if stream
                    .as_any()
                    .and_then(|a| a.downcast_ref::<QuectelBg770a>())
                    .is_some()
                {
                    return true;
                }
            }
        }
        false
    }

    /// True if any modem's SimMqttFabric has a publish on `topic`, optionally
    /// requiring `payload_contains` as a UTF-8 substring of the latest payload.
    pub fn mqtt_fabric_matches(&self, topic: &str, payload_contains: Option<&str>) -> bool {
        use crate::peripherals::components::QuectelBg770a;
        use crate::peripherals::uart::Uart;
        for entry in &self.peripherals {
            let Some(any) = entry.dev.as_any() else {
                continue;
            };
            let Some(uart) = any.downcast_ref::<Uart>() else {
                continue;
            };
            for stream in &uart.attached_streams {
                let Some(modem) = stream
                    .as_any()
                    .and_then(|a| a.downcast_ref::<QuectelBg770a>())
                else {
                    continue;
                };
                let fabric = modem.mqtt_net();
                if !fabric.has_publish_on(topic) {
                    continue;
                }
                match payload_contains {
                    None => return true,
                    Some(needle) => {
                        if let Some(payload) = fabric.last_payload_on(topic) {
                            if String::from_utf8_lossy(&payload).contains(needle) {
                                return true;
                            }
                        }
                    }
                }
            }
        }
        false
    }

    /// Mint a private lab AirBus trio and bind via [`Self::attach_lab_air`].
    /// Used by `from_config` (CLI) and multi-node `World` so there is exactly
    /// one bind API: `attach_lab_air`.
    pub fn attach_private_lab_air(&mut self, node_id: &str) {
        use crate::network::SimMqttFabric;
        use crate::peripherals::ble_air::BleAirBus;
        use crate::peripherals::nrf52::radio::VirtualAirBus;
        use crate::peripherals::rf_medium::{PathLossParams, RfMedium};
        let nrf = VirtualAirBus::new();
        nrf.attach_medium(RfMedium::new(1).with_params(PathLossParams::default()));
        let ble = BleAirBus::new();
        let cellular = SimMqttFabric::new();
        self.attach_lab_air(node_id, nrf, ble, cellular);
    }

    /// Attach a UART stream device (e.g. an inter-chip wire endpoint) to the
    /// UART peripheral registered under `uart_id`. This is the post-build
    /// counterpart to `AttachCtx::uart().attach_stream(..)`, used by
    /// `World::from_manifest` to wire cross-link endpoints between nodes.
    /// Errors if no such peripheral exists or it is not a UART.
    pub fn attach_uart_stream_by_id(
        &mut self,
        uart_id: &str,
        dev: Box<dyn crate::peripherals::uart::UartStreamDevice>,
    ) -> anyhow::Result<()> {
        let idx = self
            .find_peripheral_index_by_name(uart_id)
            .ok_or_else(|| anyhow::anyhow!("no peripheral '{uart_id}'"))?;
        // Bind to the CAPABILITY, not a concrete struct. Downcasting to `Uart`
        // used to reject every family with its own UART model — an ESP32-C3's
        // `uart1` reported "is not a UART", so two C3s could not be linked at
        // all. Any model that implements `UartStreamHost` is wireable here.
        let uart = self.peripherals[idx]
            .dev
            .as_uart_stream_host()
            .ok_or_else(|| anyhow::anyhow!("peripheral '{uart_id}' is not a UART"))?;
        uart.detach_console_sink();
        uart.attach_stream_device(dev);
        // Stream attach can flip models that force walk when a peer is wired
        // (CanBus-style). Rebuild tick indices and recompute walk-deletion —
        // same seam as `attach_can_bus_by_id`. EspUart under event-scheduler
        // stays walk-free and polls peers from `on_event` / take_scheduled.
        self.rebuild_peripheral_ranges();
        self.recompute_walk_deletable();
        Ok(())
    }

    /// Attach one endpoint of a shared `CanBus` to an on-chip CAN controller or
    /// a CAN-capable device nested below an SPI controller.
    ///
    /// This is deliberately a post-build seam: system manifests build each
    /// node in isolation, while an environment manifest supplies the topology
    /// only after all of those buses exist. It rejects a missing identifier and
    /// any non-FDCAN device rather than silently routing a topology edge to a
    /// wrong peripheral.
    pub fn attach_can_endpoint_by_id(
        &mut self,
        can_id: &str,
        tx: std::sync::mpsc::Sender<crate::network::CanFrame>,
        rx: std::sync::mpsc::Receiver<crate::network::CanFrame>,
    ) -> anyhow::Result<()> {
        if can_id.trim().is_empty() {
            anyhow::bail!("CAN endpoint id must not be blank");
        }
        let mut channels = Some((tx, rx));
        let mut nested_owner = None;
        let named_peripheral = self.find_peripheral_index_by_name(can_id);
        if let Some(idx) = named_peripheral {
            let any = self.peripherals[idx]
                .dev
                .as_any_mut()
                .ok_or_else(|| anyhow::anyhow!("peripheral '{can_id}' is not a CAN controller"))?;
            if let Some(fdcan) = any.downcast_mut::<crate::peripherals::fdcan::Fdcan>() {
                let (tx, rx) = channels.take().expect("CAN channels are available");
                fdcan.attach_bus(tx, rx)?;
            } else if let Some(bxcan) = any.downcast_mut::<crate::peripherals::bxcan::BxCan>() {
                let (tx, rx) = channels.take().expect("CAN channels are available");
                bxcan.attach_bus(tx, rx)?;
            }
        }
        if channels.is_some() {
            let mut matched = false;
            for (peripheral_index, peripheral) in self.peripherals.iter_mut().enumerate() {
                let Some(any) = peripheral.dev.as_any_mut() else {
                    continue;
                };
                let devices: Option<&mut [Box<dyn crate::peripherals::spi::SpiDevice>]> =
                    if let Some(spi) = any.downcast_mut::<crate::peripherals::spi::Spi>() {
                        Some(&mut spi.attached_devices)
                    } else if let Some(spi) = any.downcast_mut::<crate::peripherals::esp32::spi::Esp32Spi>() {
                        Some(&mut spi.attached_devices)
                    } else if let Some(spi) = any.downcast_mut::<crate::peripherals::esp32c3::spi::Esp32c3Spi>() {
                        Some(spi.attached_devices_mut())
                    } else if let Some(spi) = any.downcast_mut::<crate::peripherals::esp32s3::gpspi::Esp32s3Spi>() {
                        Some(spi.attached_devices_mut())
                    } else if let Some(serial) = any.downcast_mut::<crate::peripherals::nrf52::serial_instance::Nrf52SerialInstance>() {
                        Some(serial.attached_spi_devices_mut())
                    } else {
                        None
                    };
                let Some(devices) = devices else { continue };
                if let Some(device) = devices
                    .iter_mut()
                    .find(|device| device.component_id() == Some(can_id))
                {
                    matched = true;
                    let (tx, rx) = channels
                        .take()
                        .expect("CAN endpoint ids are unique per system");
                    device.attach_can_bus(tx, rx).map_err(|error| {
                        anyhow::anyhow!(
                            "SPI device '{can_id}' cannot attach as CAN endpoint: {error}"
                        )
                    })?;
                    nested_owner = Some(peripheral_index);
                    break;
                }
            }
            if !matched {
                if named_peripheral.is_some() {
                    anyhow::bail!("peripheral '{can_id}' is not a CAN controller");
                }
                anyhow::bail!("no CAN endpoint '{can_id}'");
            }
        }
        // FDCAN is walk-free under `event-scheduler` until a CanBus endpoint is
        // attached (`scheduler_mode` is `event-scheduler && bus_rx.is_none()`).
        // from_config latches `legacy_walk_disabled` over the pre-attach set, so
        // without a recompute the multi-node bus stays walk-deleted and mpsc RX
        // never drains — world_can_bus / CanBus paths under cargo test --workspace
        // (feature-unified event-scheduler) receive zero frames.
        self.rebuild_peripheral_ranges();
        self.recompute_walk_deletable();
        if let Some(owner) = nested_owner {
            self.refresh_legacy_tick_index(owner);
            #[cfg(feature = "event-scheduler")]
            self.collect_scheduled_events(owner);
        }
        Ok(())
    }

    pub fn attach_can_bus_by_id(
        &mut self,
        can_id: &str,
        tx: std::sync::mpsc::Sender<crate::network::CanFrame>,
        rx: std::sync::mpsc::Receiver<crate::network::CanFrame>,
    ) -> anyhow::Result<()> {
        let idx = self
            .find_peripheral_index_by_name(can_id)
            .ok_or_else(|| anyhow::anyhow!("no peripheral '{can_id}'"))?;
        let any = self.peripherals[idx]
            .dev
            .as_any_mut()
            .ok_or_else(|| anyhow::anyhow!("peripheral '{can_id}' is not an FDCAN"))?;
        let fdcan = any
            .downcast_mut::<crate::peripherals::fdcan::Fdcan>()
            .ok_or_else(|| anyhow::anyhow!("peripheral '{can_id}' is not an FDCAN"))?;
        fdcan.attach_bus(tx, rx)?;
        self.rebuild_peripheral_ranges();
        self.recompute_walk_deletable();
        Ok(())
    }

    /// Detach a single UART (by peripheral id) from the shared console TX sink.
    ///
    /// `attach_uart_tx_sink` wires the human-readable serial monitor to *every*
    /// UART on the bus. A UART used as an inter-chip cross-link (see
    /// `attach_uart_stream_by_id` + `VirtualWireEndpoint`) carries raw protocol
    /// octets (e.g. IO-Link M-sequences), not console text — letting those bytes
    /// into the serial monitor floods it with binary garbage that looks
    /// identical on both peers. Calling this after wiring a cross-link keeps the
    /// protocol bytes out of the console while leaving them in the UART trace
    /// (the protocol analyzers read `trace_snapshot`, not the sink).
    pub fn detach_uart_sink_by_id(&mut self, uart_id: &str) -> anyhow::Result<()> {
        let idx = self
            .find_peripheral_index_by_name(uart_id)
            .ok_or_else(|| anyhow::anyhow!("no peripheral '{uart_id}'"))?;
        let any = self.peripherals[idx]
            .dev
            .as_any_mut()
            .ok_or_else(|| anyhow::anyhow!("peripheral '{uart_id}' is not introspectable"))?;
        let uart = any
            .downcast_mut::<crate::peripherals::uart::Uart>()
            .ok_or_else(|| anyhow::anyhow!("peripheral '{uart_id}' is not a UART"))?;
        uart.set_sink(None, false);
        Ok(())
    }

    /// Return the `(base, size)` of the peripheral the bus router would dispatch
    /// `addr` to, using the same last-start-wins binary-search logic as
    /// [`read_u32`] / [`write_u32`]. Unlike `iter().find()`, this correctly
    /// resolves overlapping entries where a narrower, later-registered twin
    /// (e.g. `uart0_s3`) shadows a broader catch-all stub (e.g. `low_mmio`)
    /// that has an equal or lower base address.
    pub fn resolve_window(&self, addr: u64) -> Option<(u64, u64)> {
        let idx = self.find_peripheral_index(addr)?;
        let p = &self.peripherals[idx];
        Some((p.base, p.size))
    }

    /// Smallest registered window start STRICTLY greater than `addr`, if any.
    ///
    /// Together with [`resolve_window`] this bounds the contiguous span from
    /// `addr` that is guaranteed to dispatch to the same peripheral entry:
    /// past the next window start, a narrower layered twin may take over even
    /// though `addr`'s own window continues underneath (last-start-wins).
    /// The SVD coverage probe uses this to keep its baseline samples inside
    /// the service region of the peripheral under probe.
    pub fn next_window_start(&self, addr: u64) -> Option<u64> {
        if self.peripheral_ranges.len() == self.peripherals.len() {
            let pos = self
                .peripheral_ranges
                .partition_point(|range| range.start <= addr);
            return self.peripheral_ranges.get(pos).map(|r| r.start);
        }
        self.peripherals
            .iter()
            .map(|p| p.base)
            .filter(|&b| b > addr)
            .min()
    }

    /// Translate a Cortex-M bit-band alias address to (physical_byte_addr, bit_index).
    ///
    /// Peripheral bit-band: alias 0x42000000–0x43FFFFFF → physical 0x40000000–0x400FFFFF
    /// SRAM bit-band:       alias 0x22000000–0x23FFFFFF → physical 0x20000000–0x200FFFFF
    ///
    /// Each alias *word* (4 bytes, naturally aligned) represents one physical bit.
    pub(crate) fn bit_band_translate(addr: u64) -> Option<(u64, u8)> {
        let (phys_base, alias_base) = if (0x42000000..0x44000000).contains(&addr) {
            (0x40000000u64, 0x42000000u64)
        } else if (0x22000000..0x24000000).contains(&addr) {
            (0x20000000u64, 0x22000000u64)
        } else {
            return None;
        };
        let offset = addr - alias_base;
        let bit_word = offset / 4; // each alias word = 1 physical bit
        let phys_byte = phys_base + bit_word / 8;
        let bit = (bit_word % 8) as u8;
        Some((phys_byte, bit))
    }
}
