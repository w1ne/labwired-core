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
        // ESP32: "GPIO17" → the single "gpio" peripheral, GPIO_OUT_REG at
        // base + 0x04 (TRM §4.10, bit = pin number for GPIO0..31).
        if let Some(bit) = Self::parse_esp32_gpio_pin(pin) {
            let idx = bus.find_peripheral_index_by_name("gpio")?;
            let is_esp32 = bus.peripherals[idx]
                .dev
                .as_any()
                .map(|a| {
                    a.downcast_ref::<crate::peripherals::esp32::gpio::Esp32Gpio>()
                        .is_some()
                })
                .unwrap_or(false);
            if is_esp32 {
                const GPIO_OUT_REG_OFFSET: u64 = 0x04;
                return Some((bus.peripherals[idx].base + GPIO_OUT_REG_OFFSET, bit));
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
        if num > 31 {
            return None;
        }
        Some(num)
    }

    /// Resolve an STM32 pin label to its `(IDR address, bit)` so a sensor can
    /// drive an MCU input line (e.g. the HC-SR04 ECHO pin).
    pub(crate) fn resolve_pin_idr(bus: &SystemBus, pin: &str) -> Option<(u64, u8)> {
        let (port_name, bit) = Self::parse_stm32_pin(pin)?;
        let idx = bus.find_peripheral_index_by_name(&port_name)?;
        let base = bus.peripherals[idx].base;
        let idr_off = bus.peripherals[idx]
            .dev
            .as_any()
            .and_then(|a| a.downcast_ref::<crate::peripherals::gpio::GpioPort>())
            .map(|g| g.idr_offset())?;
        Some((base + idr_off, bit))
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
        self.peripheral_hint.set(None);
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
    }

    pub fn refresh_peripheral_index(&mut self) {
        self.rebuild_peripheral_ranges();
    }

    pub(crate) fn find_peripheral_index(&self, addr: u64) -> Option<usize> {
        // Canonical routing: among the windows CONTAINING `addr`, the one
        // with the GREATEST start wins (last-start-wins; equal starts resolve
        // to the last-registered entry). This makes routing a pure function
        // of the address.
        //
        // The hint cache deliberately does NOT short-circuit on containment:
        // with layered windows (a narrow per-peripheral twin inside a broad
        // catch-all stub) a hint seeded by a broad-only access also CONTAINS
        // the twin's addresses, so a containment-only check hijacks them to
        // the catch-all and routing becomes a function of access history —
        // see bus::tests::overlapping_windows_route_history_independently.
        // The canonical path is already cheap: one partition_point (O(log n))
        // and, in the common non-overlapped case, one containment check.
        let mut idx = None;
        if self.peripheral_ranges.len() == self.peripherals.len() {
            let pos = self
                .peripheral_ranges
                .partition_point(|range| range.start <= addr);
            // Walk backwards through the candidate starts: the nearest
            // (greatest-start) window may have already ENDED below `addr`
            // while a broader, earlier-started window still covers it.
            for range in self.peripheral_ranges[..pos].iter().rev() {
                if addr < range.end {
                    idx = Some(range.index);
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
        idx
    }

    pub fn find_peripheral_index_by_name(&self, name: &str) -> Option<usize> {
        self.peripherals.iter().position(|p| p.name == name)
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
