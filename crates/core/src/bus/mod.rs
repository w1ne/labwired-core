// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

use crate::memory::LinearMemory;
use crate::peripherals::gpio::GpioRegisterLayout;
use crate::peripherals::nvic::NvicState;
use crate::peripherals::rcc::RccRegisterLayout;
use crate::peripherals::uart::Uart;
use crate::peripherals::uart::UartRegisterLayout;
use crate::{Bus, DmaRequest, Peripheral, SimResult, SimulationError};
use anyhow::Context;
use labwired_config::{parse_size, ChipDescriptor, PeripheralConfig, SystemManifest};
use std::cell::Cell;
use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::sync::Mutex;

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
    fn sync_scheduler_peripheral(&mut self, idx: usize) {
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
    fn collect_scheduled_events(&mut self, idx: usize) {
        if !self.peripherals[idx].dev.uses_scheduler() {
            return;
        }
        for (delay, token) in self.peripherals[idx].dev.take_scheduled_events() {
            self.pending_schedule.push((idx, delay, token));
        }
    }
}

pub struct PeripheralEntry {
    pub name: String,
    pub base: u64,
    pub size: u64,
    pub irq: Option<u32>,
    pub dev: Box<dyn Peripheral>,
    pub ticks_remaining: u64,
    /// Phase 2B.1 (issue #192): lazy cancel token for the event scheduler.
    /// Bumped when the peripheral resets; `EventScheduler::drain_due` drops
    /// entries whose generation no longer matches the snapshot.
    pub generation: u32,
}

pub struct SystemBus {
    pub flash: LinearMemory,
    pub ram: LinearMemory,
    /// Extra CPU-visible RAM/ROM windows beyond `flash`/`ram` (e.g. ESP32 IRAM
    /// `0x4037C000` and flash-DROM `0x3C000000`), from the chip's
    /// `memory_regions`. Checked after `ram`/`flash`, before peripherals.
    pub extra_mem: Vec<LinearMemory>,
    pub peripherals: Vec<PeripheralEntry>,
    pub nvic: Option<Arc<NvicState>>,
    pub observers: Vec<Arc<dyn crate::SimulationObserver>>,
    pub config: crate::SimulationConfig,
    /// Enable Cortex-M peripheral/SRAM bit-band alias translation.
    /// False for architectures (e.g. RISC-V) whose memory maps collide with
    /// the bit-band alias ranges 0x42000000–0x44000000 / 0x22000000–0x24000000.
    pub bit_band_enabled: bool,
    /// Plan 3: per-core bitmask of pending cpu IRQ slots (32 bits each;
    /// index 0 = PRO_CPU, 1 = APP_CPU). Aggregated by
    /// `tick_peripherals_with_costs` from peripheral `explicit_irqs` source
    /// IDs routed through the registered interrupt matrix's per-core map
    /// tables. Cleared per slot via `clear_cpu_irq_pending`.
    pub pending_cpu_irqs: [u32; 2],
    /// Bus-level thunk table for addresses outside any `RomThunkBank`.
    /// Used to intercept calls to firmware functions resident in flash
    /// (e.g. ESP-IDF's `multi_heap_register` at 0x40194954) so we can
    /// substitute a sim-side Rust implementation. To install: write
    /// BREAK 1,14 bytes (`ROM_THUNK_BREAK_BYTES`) at `pc` in flash AND
    /// `bus.flash_thunks.insert(pc, thunk)`. The CPU's BREAK 1,14
    /// dispatcher (xtensa_lx7.rs) calls `bus.get_rom_thunk(pc)` which
    /// checks both this table and any `RomThunkBank` peripherals.
    pub flash_thunks:
        std::collections::HashMap<u32, crate::peripherals::esp32s3::rom_thunks::RomThunkFn>,
    peripheral_ranges: Vec<PeripheralRange>,
    peripheral_hint: Cell<Option<usize>>,
    /// Cached index of the classic-ESP32 DPORT peripheral, if one is
    /// registered (`None` otherwise — the common case, incl. every ESP32-S3
    /// bus). Recomputed in `rebuild_peripheral_ranges` on each peripheral
    /// add/refresh, same staleness contract as `peripheral_ranges`. Lets
    /// `dport_cross_core_pending` — called on the per-step interrupt path —
    /// skip an O(peripherals) scan that would otherwise return 0 every step
    /// on buses with no DPORT.
    dport_idx: Option<usize>,
    /// Last-known IN value of GPIO ports 0 and 1, used by the per-tick
    /// edge-detection pass that drives GPIOTE EVENTS_IN. Both default to
    /// 0 at construction; the first tick after a GPIO write will produce
    /// edge events for any non-zero bits, which matches Nordic
    /// hardware's "reset to zero, edge on first set" behavior.
    last_gpio_in: [u32; 2],
    /// Phase 2B.2 (issue #192): the current CPU cycle count, mirrored from
    /// `Machine::total_cycles` once per step. Read by the MMIO write path to
    /// lazily sync scheduler-driven peripherals (`uses_scheduler() == true`)
    /// to "now" before a register write observes their state. Only consulted
    /// under the `event-scheduler` feature; harmlessly 0 otherwise.
    pub current_cycle: u64,
    /// Phase 2B.3a (issue #192): write-context schedule requests buffered
    /// during MMIO writes. A scheduler-driven peripheral can't reach the
    /// scheduler from `write`, so after the write the bus collects its
    /// `take_scheduled_events()` here as `(peripheral_idx, delay_ticks,
    /// token)`; `Machine::drain_scheduler_events` enqueues and clears them.
    /// Only populated under the `event-scheduler` feature.
    pub pending_schedule: Vec<(usize, u64, u32)>,
    /// Phase 2B.3c (issue #192): when true, `tick_peripherals_phase1` skips the
    /// entire per-cycle peripheral walk — the actual ~2.4x win. Set ONLY for a
    /// config whose every peripheral is migrated (`uses_scheduler`) or inert
    /// (no real `tick()` work), e.g. ESP32-classic via `configure_xtensa_esp32`.
    /// Read only under the `event-scheduler` feature; flag-off the walk always
    /// runs, so the shipped build is unchanged.
    pub legacy_walk_disabled: bool,
    /// HC-SR04 ultrasonic sensors wired to GPIO TRIG/ECHO pins. The echo window
    /// is armed by the TRIG GPIO write-hook (`maybe_arm_hcsr04`); a cheap
    /// per-tick pass (`service_hcsr04`) drives the computed ECHO input level,
    /// touching the bus only on a transition. Empty by default → zero cost.
    pub hcsr04: Vec<crate::peripherals::hc_sr04::HcSr04>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct PeripheralRange {
    start: u64,
    end: u64,
    index: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PeripheralTickCost {
    pub index: usize,
    pub cycles: u32,
}

impl Default for SystemBus {
    fn default() -> Self {
        Self::new()
    }
}

impl SystemBus {
    pub(crate) fn canonical_peripheral_type(raw_type: &str) -> String {
        let t = raw_type.to_ascii_lowercase();

        // Keep explicit core types as-is.
        match t.as_str() {
            "uart" | "gpio" | "rcc" | "systick" | "timer" | "i2c" | "spi" | "exti" | "afio"
            | "dma" | "gpdma" | "adc" | "pio" | "declarative" | "strict_ir"
            | "strict_ir_internal" | "pwr" | "flash" | "rng" | "crc" | "rtc" | "rtc_f1"
            | "rtc_v3" | "iwdg" | "wwdg" | "dac" | "dbgmcu" | "lptim" | "quadspi" | "sai"
            | "usb_otg" | "bxcan" | "sdmmc" | "comp" | "tsc" | "fmc" => {
                return t;
            }
            _ => {}
        }

        // Nordic-specific pre-emption — keep these ahead of the generic
        // mappers so types like `nrf52840_saadc` (contains "adc") and
        // `nrf52840_qspi` (contains "spi" + "qspi") aren't coerced to
        // STM32 layouts.
        if t == "nrf52840_saadc" || t == "nrf52_saadc" || t == "nrf52840_adc" {
            return "nrf52_saadc".to_string();
        }
        if t == "nrf52840_qspi" || t == "nrf52_qspi" {
            return "nrf52_qspi".to_string();
        }
        // SPIS / TWIS / TWIM must be intercepted before the generic "contains(spi)"
        // and "contains(i2c)" / "ends_with(_twi)" matchers, otherwise they
        // would be mis-routed to the STM32 SPI / I2C models.
        if t == "nrf52840_spis" || t == "nrf52_spis" {
            return "nrf52840_spis".to_string();
        }
        if t == "nrf52840_twis" || t == "nrf52_twis" {
            return "nrf52840_twis".to_string();
        }
        // TWIM / TWI master: nRF52 I²C master with EasyDMA. Must precede the
        // generic "contains(i2c)" / "ends_with(_twi)" fuzzy matchers.
        if t == "nrf52840_i2c" || t == "nrf52840_twim" || t == "nrf52_twim" || t == "nrf52_i2c" {
            return "nrf52840_twim".to_string();
        }
        // UARTE: nRF52 UART with EasyDMA — must be intercepted before the
        // generic "contains(uart)" matcher, which would coerce it to the
        // STM32-style generic Uart model and lose PSEL/BAUDRATE/CONFIG.
        if t == "nrf52840_uart" || t == "nrf52_uart" || t == "nrf52_uarte" {
            return "nrf52840_uart".to_string();
        }

        // Specific mappers first — must come before fuzzy matchers so e.g.
        // "quadspi" doesn't get swallowed by the generic "contains(spi)" rule.
        if t.contains("quadspi") || t == "qspi" {
            return "quadspi".to_string();
        }
        if t.contains("lptim") || t == "low_power_timer" {
            return "lptim".to_string();
        }
        if t == "sai" || t.starts_with("sai_") || t.contains("audio") {
            return "sai".to_string();
        }
        if t.contains("otg") || t == "usb_fs" || t == "usb_otg_fs" {
            return "usb_otg".to_string();
        }
        if t == "bxcan" || t == "stm32_can" {
            return "bxcan".to_string();
        }
        if t == "sdmmc" || t == "sdio" || t.starts_with("sdmmc_") {
            return "sdmmc".to_string();
        }
        if t == "comp" || t == "comparator" || t.starts_with("comp_") {
            return "comp".to_string();
        }
        if t == "tsc" || t == "touchsense" {
            return "tsc".to_string();
        }
        if t == "fmc" || t == "fsmc" || t == "memorycontroller" {
            return "fmc".to_string();
        }

        if t.contains("uart") || t.contains("usart") || t == "leuart" || t.ends_with("_sci") {
            return "uart".to_string();
        }
        // Nordic GPIOTE shares "gpio" in its name but is a task/event
        // controller with a totally different register surface; route it
        // to the dedicated nRF52 model before the generic gpio matcher.
        if t == "nrf52840_gpiotasksevents" || t == "nrf52_gpiote" {
            return "nrf52_gpiote".to_string();
        }
        if t == "sam4s_pio" || (t.contains("gpio") && t != "pio") {
            return "gpio".to_string();
        }
        if t.contains("i2c") || t.contains("iic") || t.contains("smbus") || t.ends_with("_twi") {
            return "i2c".to_string();
        }
        if t.contains("spi") {
            return "spi".to_string();
        }
        if t == "udma" || t.contains("dma") {
            return "dma".to_string();
        }
        // Nordic CLOCK shares its name with the generic "rcc" bin in the
        // canonicalize, but its register layout is Nordic-specific and it
        // is unioned with the POWER peripheral at the same base address.
        // Route it to the dedicated nRF52 model.
        if t == "nrf_clock" || t == "nrf52_clock" || t == "nrf52840_clock" {
            return "nrf52_clock".to_string();
        }
        if t.contains("rcc") || t.contains("cmu") {
            return "rcc".to_string();
        }
        if t == "arm_generictimer" || t == "arm_globaltimer" || t == "arm_sp804_timer" {
            return "systick".to_string();
        }
        if t.contains("timer") || t.ends_with("_gpt") || t.ends_with("_agt") {
            return "timer".to_string();
        }
        if t.contains("adc") {
            return "adc".to_string();
        }

        t
    }

    fn profile_name(p_cfg: &PeripheralConfig) -> anyhow::Result<Option<&str>> {
        if let Some(value) = p_cfg.config.get("profile") {
            return value.as_str().map(Some).ok_or_else(|| {
                anyhow::anyhow!("Peripheral '{}' config.profile must be a string", p_cfg.id)
            });
        }
        if let Some(value) = p_cfg.config.get("register_layout") {
            return value.as_str().map(Some).ok_or_else(|| {
                anyhow::anyhow!(
                    "Peripheral '{}' config.register_layout must be a string",
                    p_cfg.id
                )
            });
        }
        Ok(None)
    }

    fn parse_profile_or_default<T>(
        p_cfg: &PeripheralConfig,
        peripheral_kind: &str,
    ) -> anyhow::Result<T>
    where
        T: FromStr<Err = String> + Default,
    {
        let Some(profile_name) = Self::profile_name(p_cfg)? else {
            return Ok(T::default());
        };
        T::from_str(profile_name).map_err(|e| {
            anyhow::anyhow!(
                "Peripheral '{}' has invalid {} profile '{}': {}",
                p_cfg.id,
                peripheral_kind,
                profile_name,
                e
            )
        })
    }

    fn resolve_peripheral_path(manifest: &SystemManifest, descriptor_path: &str) -> PathBuf {
        let raw = PathBuf::from(descriptor_path);
        if raw.is_absolute() {
            return raw;
        }

        let chip_path = Path::new(&manifest.chip);
        let chip_dir = chip_path.parent().unwrap_or_else(|| Path::new("."));
        let chip_relative = chip_dir.join(descriptor_path);
        if chip_relative.exists() {
            chip_relative
        } else {
            raw
        }
    }

    /// Parse a pin label into `(gpio peripheral id, bit)`. Accepts the STM32
    /// form "PC7" -> `("gpioc", 7)` and the Nordic form "P0.04" / "P1.15" ->
    /// `("gpio0", 4)` / `("gpio1", 15)`.
    fn parse_stm32_pin(pin: &str) -> Option<(String, u8)> {
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

    fn resolve_pin_odr(bus: &SystemBus, pin: &str) -> Option<(u64, u8)> {
        let (port_name, bit) = Self::parse_stm32_pin(pin)?;
        let idx = bus.find_peripheral_index_by_name(&port_name)?;
        let base = bus.peripherals[idx].base;
        let odr_off = bus.peripherals[idx]
            .dev
            .as_any()
            .and_then(|a| a.downcast_ref::<crate::peripherals::gpio::GpioPort>())
            .map(|g| g.odr_offset())?;
        Some((base + odr_off, bit))
    }

    /// Resolve an STM32 pin label to its `(IDR address, bit)` so a sensor can
    /// drive an MCU input line (e.g. the HC-SR04 ECHO pin).
    fn resolve_pin_idr(bus: &SystemBus, pin: &str) -> Option<(u64, u8)> {
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

    /// Service all HC-SR04 sensors for one tick: compute each sensor's ECHO
    /// level from its (write-hook-armed) echo window and drive it onto the ECHO
    /// input register, touching the bus only on a level transition. TRIG is NOT
    /// polled here — `maybe_arm_hcsr04` arms the window on the GPIO write, which
    /// is cycle-exact (see `Machine::step`). No-op when no sensors are wired.
    pub(crate) fn service_hcsr04(&mut self) {
        if self.hcsr04.is_empty() {
            return;
        }
        let now = self.current_cycle;
        for i in 0..self.hcsr04.len() {
            // TRIG is no longer polled here — `maybe_arm_hcsr04` arms the window
            // on the GPIO write (cycle-exact, see the note in `Machine::step`).
            // The per-cycle work is two integer comparisons plus, only on a
            // transition, one read-modify-write of the ECHO input bit.
            let echo_high = self.hcsr04[i].echo_high_at(now);
            if echo_high == self.hcsr04[i].last_echo_high() {
                continue;
            }
            let echo_addr = self.hcsr04[i].echo_idr_addr;
            let echo_bit = self.hcsr04[i].echo_bit;
            let idr = self.read_u32(echo_addr).unwrap_or(0);
            let new_idr = if echo_high {
                idr | (1 << echo_bit)
            } else {
                idr & !(1 << echo_bit)
            };
            if new_idr != idr {
                let _ = self.write_u32(echo_addr, new_idr);
            }
            self.hcsr04[i].set_last_echo_high(echo_high);
        }
    }

    /// Write-hook mirror of [`maybe_latch_dc`](Self::maybe_latch_dc) for the
    /// HC-SR04: after an MMIO write to peripheral `idx`, if that peripheral is
    /// the GPIO hosting any sensor's TRIG line, re-read the TRIG ODR bit and run
    /// the sensor's rising-edge/arm logic at `now = self.current_cycle`.
    ///
    /// Because TRIG only changes via a GPIO write, edge detection on the write is
    /// exactly equivalent to the old per-cycle TRIG poll, and `current_cycle`
    /// here equals the value the immediately-following `service_hcsr04` tick sees
    /// (see `Machine::step`), so the arming is cycle-exact.
    fn maybe_arm_hcsr04(&mut self, idx: usize) {
        if self.hcsr04.is_empty() {
            return;
        }
        let now = self.current_cycle;
        for i in 0..self.hcsr04.len() {
            // Resolve & cache the TRIG GPIO's peripheral index on first use.
            let trig_idx = match self.hcsr04[i].trig_peripheral_idx() {
                Some(t) => t,
                None => {
                    let trig_addr = self.hcsr04[i].trig_odr_addr;
                    match self.find_peripheral_index(trig_addr) {
                        Some(t) => {
                            self.hcsr04[i].set_trig_peripheral_idx(t);
                            t
                        }
                        None => continue,
                    }
                }
            };
            if trig_idx != idx {
                continue;
            }
            let trig_addr = self.hcsr04[i].trig_odr_addr;
            let trig_bit = self.hcsr04[i].trig_bit;
            let trig_high = self
                .read_u32(trig_addr)
                .map(|v| (v >> trig_bit) & 1 != 0)
                .unwrap_or(false);
            self.hcsr04[i].observe_trig(trig_high, now);
        }
    }

    /// Before an SPI transfer, refresh the D/C level of any attached
    /// display that observes a D/C GPIO line (e.g. the PCD8544 Nokia 5110)
    /// by reading the driving GPIO's output bit. No-op for non-SPI writes and
    /// for SPI peripherals with no D/C-observing device (one cheap downcast).
    fn maybe_latch_dc(&mut self, idx: usize) {
        // Phase 1: collect (attached_index, odr_addr, bit) — immutable borrow.
        let sources: Vec<(usize, u64, u8)> = {
            let Some(any) = self.peripherals[idx].dev.as_any() else {
                return;
            };
            let Some(spi) = any.downcast_ref::<crate::peripherals::spi::Spi>() else {
                return;
            };
            spi.attached_devices
                .iter()
                .enumerate()
                .filter_map(|(i, d)| d.dc_source().map(|(a, b)| (i, a, b)))
                .collect()
        };
        if sources.is_empty() {
            return;
        }
        // Phase 2: sample the GPIO output bits via the bus.
        let levels: Vec<(usize, bool)> = sources
            .iter()
            .map(|&(i, addr, bit)| {
                let lvl = crate::Bus::read_u32(self, addr)
                    .map(|v| (v >> bit) & 1 != 0)
                    .unwrap_or(false);
                (i, lvl)
            })
            .collect();
        // Phase 3: push the latched levels into the devices — mutable borrow.
        if let Some(any) = self.peripherals[idx].dev.as_any_mut() {
            if let Some(spi) = any.downcast_mut::<crate::peripherals::spi::Spi>() {
                for (i, lvl) in levels {
                    if let Some(d) = spi.attached_devices.get_mut(i) {
                        d.set_dc_level(lvl);
                    }
                }
            }
        }
    }

    fn is_peripheral_addr(p: &PeripheralEntry, addr: u64) -> bool {
        addr >= p.base && addr < p.base + p.size
    }

    fn rebuild_peripheral_ranges(&mut self) {
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
    }

    pub fn refresh_peripheral_index(&mut self) {
        self.rebuild_peripheral_ranges();
    }

    fn find_peripheral_index(&self, addr: u64) -> Option<usize> {
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
                    generation: 0,
                },
                PeripheralEntry {
                    name: "gpioa".to_string(),
                    base: 0x4001_0800,
                    size: 0x400,
                    irq: None,
                    dev: Box::new(crate::peripherals::gpio::GpioPort::new()),
                    ticks_remaining: 0,
                    generation: 0,
                },
                PeripheralEntry {
                    name: "rcc".to_string(),
                    base: 0x4002_1000,
                    size: 0x400,
                    irq: None,
                    dev: Box::new(crate::peripherals::rcc::Rcc::new()),
                    ticks_remaining: 0,
                    generation: 0,
                },
                PeripheralEntry {
                    name: "systick".to_string(),
                    base: 0xE000_E010,
                    size: 0x100,
                    irq: Some(15),
                    dev: Box::new(crate::peripherals::systick::Systick::new()),
                    ticks_remaining: 0,
                    generation: 0,
                },
            ],
            nvic: None,
            observers: Vec::new(),
            config: crate::SimulationConfig::default(),
            bit_band_enabled: true,
            pending_cpu_irqs: [0; 2],
            dport_idx: None,
            peripheral_ranges: Vec::new(),
            peripheral_hint: Cell::new(None),
            last_gpio_in: [0; 2],
            current_cycle: 0,
            pending_schedule: Vec::new(),
            legacy_walk_disabled: false,
            hcsr04: Vec::new(),
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
            peripheral_ranges: Vec::new(),
            peripheral_hint: Cell::new(None),
            last_gpio_in: [0; 2],
            current_cycle: 0,
            pending_schedule: Vec::new(),
            legacy_walk_disabled: false,
            hcsr04: Vec::new(),
        };
        bus.rebuild_peripheral_ranges();
        bus
    }

    /// Append a peripheral to the bus at runtime. Useful for tests and
    /// dynamic configuration that bypasses `from_config`.
    ///
    /// **No overlap check is performed.** If two peripherals claim overlapping
    /// address ranges, reads and writes are routed to the **first** matching
    /// peripheral in registration order (i.e. the earlier-registered peripheral
    /// wins). Callers are responsible for ensuring non-overlapping ranges.
    pub fn add_peripheral(
        &mut self,
        name: &str,
        base: u64,
        size: u64,
        irq: Option<u32>,
        dev: Box<dyn Peripheral>,
    ) {
        self.peripherals.push(PeripheralEntry {
            name: name.to_string(),
            base,
            size,
            irq,
            dev,
            ticks_remaining: 0,
            generation: 0,
        });
        self.rebuild_peripheral_ranges();
    }

    /// Phase 2B.1 (issue #192): snapshot of every peripheral's lazy-cancel
    /// generation, indexed by `peripheral_idx`. Threaded into
    /// `EventScheduler::drain_due` / `next_event_deadline` so stale events
    /// (scheduled before a peripheral reset) are dropped.
    pub fn peripheral_generations(&self) -> Vec<u32> {
        self.peripherals.iter().map(|p| p.generation).collect()
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
    ) -> Option<crate::peripherals::esp32s3::rom_thunks::RomThunkFn> {
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
                        any.downcast_ref::<crate::peripherals::esp32s3::rom_thunks::RomThunkBank>()
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
        thunk: crate::peripherals::esp32s3::rom_thunks::RomThunkFn,
    ) -> SimResult<()> {
        let bytes = crate::peripherals::esp32s3::rom_thunks::ROM_THUNK_BREAK_BYTES;
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
        for p in &self.peripherals {
            if let Some(any) = p.dev.as_any() {
                if let Some(matrix) =
                    any.downcast_ref::<crate::peripherals::esp32s3::intmatrix::Esp32s3IntMatrix>()
                {
                    return matrix.route_for_core(source_id, core_id);
                }
            }
        }
        None
    }

    /// Cross-core `FROM_CPU` IPI slots currently asserted for `core_id`,
    /// read live from the ESP32-classic DPORT interrupt matrix. Replaces the
    /// old test-harness IPI bridge that polled the same registers from
    /// outside the core. Returns 0 when no DPORT is mapped (non-ESP32 buses).
    fn dport_cross_core_pending(&self, core_id: u8) -> u32 {
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
        for p in &mut self.peripherals {
            let Some(any) = p.dev.as_any_mut() else {
                continue;
            };
            let Some(uart) = any.downcast_mut::<Uart>() else {
                continue;
            };
            uart.set_sink(Some(sink.clone()), echo_stdout);
        }
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
    fn chip_has_bit_band(chip: &ChipDescriptor) -> bool {
        match chip.core.as_deref() {
            Some(core) => {
                let c = core.trim().to_ascii_lowercase();
                let c = c.strip_prefix("cortex-").unwrap_or(&c);
                matches!(c, "m3" | "m4" | "m4f")
            }
            None => matches!(chip.arch, labwired_config::Arch::Arm),
        }
    }

    pub fn from_config(chip: &ChipDescriptor, manifest: &SystemManifest) -> anyhow::Result<Self> {
        let flash_size = parse_size(&chip.flash.size)?;
        let ram_size = parse_size(&chip.ram.size)?;

        let mut extra_mem = Vec::with_capacity(chip.memory_regions.len());
        for region in &chip.memory_regions {
            let size = parse_size(&region.size)?;
            extra_mem.push(LinearMemory::new(size as usize, region.base));
        }

        let mut bus = Self {
            flash_thunks: std::collections::HashMap::new(),
            flash: LinearMemory::new(flash_size as usize, chip.flash.base),
            ram: LinearMemory::new(ram_size as usize, chip.ram.base),
            extra_mem,
            peripherals: Vec::new(),
            nvic: None,
            observers: Vec::new(),
            config: crate::SimulationConfig::default(),
            bit_band_enabled: Self::chip_has_bit_band(chip),
            pending_cpu_irqs: [0; 2],
            dport_idx: None,
            peripheral_ranges: Vec::new(),
            peripheral_hint: Cell::new(None),
            last_gpio_in: [0; 2],
            current_cycle: 0,
            pending_schedule: Vec::new(),
            legacy_walk_disabled: false,
            hcsr04: Vec::new(),
        };

        let mut merged_peripherals = chip.peripherals.clone();
        for m_p in &manifest.peripherals {
            if let Some(existing) = merged_peripherals.iter_mut().find(|p| p.id == m_p.id) {
                // Merge config map
                for (k, v) in &m_p.config {
                    existing.config.insert(k.clone(), v.clone());
                }
                // Also override other fields if provided
                if m_p.base_address != 0 {
                    existing.base_address = m_p.base_address;
                }
                if m_p.irq.is_some() {
                    existing.irq = m_p.irq;
                }
                if m_p.size.is_some() {
                    existing.size = m_p.size.clone();
                }
            } else {
                merged_peripherals.push(m_p.clone());
            }
        }

        for p_cfg in &merged_peripherals {
            let canonical_type = Self::canonical_peripheral_type(&p_cfg.r#type);
            if canonical_type != p_cfg.r#type.to_ascii_lowercase() {
                tracing::debug!(
                    "Canonicalized peripheral type '{}' -> '{}' for id '{}'",
                    p_cfg.r#type,
                    canonical_type,
                    p_cfg.id
                );
            }

            let dev: Box<dyn Peripheral> = match canonical_type.as_str() {
                // nRF52 UARTE: full register surface including PSEL/BAUDRATE/CONFIG/DMA.
                "nrf52840_uart" => Box::new(crate::peripherals::nrf52::uarte::Nrf52Uarte::new()),
                "uart" | "stm32_uart" | "stm32f1_uart" | "stm32f2_uart" | "stm32f4_uart"
                | "stm32f7_usart" | "stm32h5_usart" | "efm32_uart" | "nxp_lpuart" | "ns16550"
                | "pl011" | "gaislerapbuart" => {
                    let layout: UartRegisterLayout =
                        if p_cfg.r#type.contains("stm32h5") || p_cfg.r#type.contains("stm32f7") {
                            UartRegisterLayout::Stm32V2
                        } else if p_cfg.r#type.contains("nrf") {
                            UartRegisterLayout::Nrf52
                        } else {
                            Self::parse_profile_or_default(p_cfg, "UART")?
                        };
                    // CR3 writable mask is a per-part delta on the shared F1 map:
                    // F1 implements [10:0] (0x07FF), F4 adds bit 11 ONEBIT (0x0FFF).
                    // YAML: `config: { cr3_mask: 0xFFF }`; default F1.
                    let cr3_mask: u32 = p_cfg
                        .config
                        .get("cr3_mask")
                        .and_then(|v| v.as_u64())
                        .map(|n| n as u32)
                        .unwrap_or(0x0000_07FF);
                    Box::new(crate::peripherals::uart::Uart::new_with_layout_cr3(
                        layout, cr3_mask,
                    ))
                }
                "systick" | "arm_generictimer" => {
                    // CALIB is implementation-defined per chip; the yaml can
                    // supply the silicon value via `config: { calib: ... }`.
                    match p_cfg.config.get("calib").and_then(|v| v.as_u64()) {
                        Some(calib) => Box::new(crate::peripherals::systick::Systick::with_calib(
                            calib as u32,
                        )),
                        None => Box::new(crate::peripherals::systick::Systick::new()),
                    }
                }
                "gpio" | "stm32_gpioport" | "stm32f4_gpio" | "efmgpioport" | "npcx_gpio"
                | "imxrt_gpio" => {
                    let layout: GpioRegisterLayout = if p_cfg.r#type.contains("nrf") {
                        GpioRegisterLayout::Nrf52
                    } else if p_cfg.r#type.contains("stm32f4") || p_cfg.r#type.contains("h5") {
                        GpioRegisterLayout::Stm32V2
                    } else {
                        Self::parse_profile_or_default(p_cfg, "GPIO")?
                    };
                    // For nRF52 ports, an optional `num_pins` config key caps the
                    // valid-pin range (e.g. 16 for nRF52840 P1 which has P1.0–P1.15).
                    // Writes outside that range are discarded; reads return 0.
                    if layout == GpioRegisterLayout::Nrf52 {
                        let num_pins: u32 = p_cfg
                            .config
                            .get("num_pins")
                            .and_then(|v| v.as_u64())
                            .map(|n| n as u32)
                            .unwrap_or(32);
                        Box::new(crate::peripherals::gpio::GpioPort::new_nrf52(num_pins))
                    } else if layout == GpioRegisterLayout::Stm32V2
                        && p_cfg.config.contains_key("reset_moder")
                    {
                        // Per-port silicon reset values (MODER/OSPEEDR/PUPDR)
                        // supplied by the chip yaml; missing keys default to 0.
                        let cfg_u32 = |key: &str| -> u32 {
                            p_cfg
                                .config
                                .get(key)
                                .and_then(|v| v.as_u64())
                                .map(|n| n as u32)
                                .unwrap_or(0)
                        };
                        Box::new(crate::peripherals::gpio::GpioPort::new_stm32v2_with_resets(
                            cfg_u32("reset_moder"),
                            cfg_u32("reset_ospeedr"),
                            cfg_u32("reset_pupdr"),
                        ))
                    } else {
                        Box::new(crate::peripherals::gpio::GpioPort::new_with_layout(layout))
                    }
                }
                "rcc" => {
                    let layout: RccRegisterLayout = Self::parse_profile_or_default(p_cfg, "RCC")?;
                    let mut rcc = crate::peripherals::rcc::Rcc::new_with_layout(layout);
                    // F4 ENR writable masks are per-part (implemented-peripheral
                    // set). YAML: `config: { rcc_ahb1enr_mask, rcc_apb1enr_mask,
                    // rcc_apb2enr_mask }`; default unmasked (0xFFFF_FFFF).
                    let m = |k: &str| -> u32 {
                        p_cfg
                            .config
                            .get(k)
                            .and_then(|v| v.as_u64())
                            .map(|n| n as u32)
                            .unwrap_or(0xFFFF_FFFF)
                    };
                    rcc.set_f4_enr_masks(
                        m("rcc_ahb1enr_mask"),
                        m("rcc_apb1enr_mask"),
                        m("rcc_apb2enr_mask"),
                    );
                    Box::new(rcc)
                }
                "dbgmcu" => {
                    // Pull IDCODE from YAML config (`idcode: "0x10076415"` or
                    // `idcode: 269009941`). Default 0 — firmware probing
                    // DBGMCU_IDCODE will then read 0; logged.
                    let idcode: u32 = p_cfg
                        .config
                        .get("idcode")
                        .and_then(|v| {
                            if let Some(s) = v.as_str() {
                                let s = s.trim();
                                if let Some(rest) =
                                    s.strip_prefix("0x").or_else(|| s.strip_prefix("0X"))
                                {
                                    u32::from_str_radix(rest, 16).ok()
                                } else {
                                    s.parse::<u32>().ok()
                                }
                            } else {
                                v.as_u64().map(|n| n as u32)
                            }
                        })
                        .unwrap_or(0);
                    if idcode == 0 {
                        tracing::warn!(
                            "dbgmcu peripheral '{}' has no idcode configured \
                             — firmware probing DBGMCU_IDCODE will read 0",
                            p_cfg.id
                        );
                    }
                    Box::new(crate::peripherals::dbgmcu::Dbgmcu::new(idcode))
                }
                "timer" | "stm32_timer" | "efm32timer" | "renesasra_agt" | "stm32l0_lptimer" => {
                    if p_cfg.r#type.contains("nrf") {
                        // Nordic TIMER is task/event-driven and shares no
                        // register layout with the STM32 TIMx family —
                        // route to the dedicated nRF52 model.
                        // TIMER3/4 have 6 CC; TIMER0/1/2 have 4 (default).
                        let num_cc: usize = p_cfg
                            .config
                            .get("num_cc")
                            .and_then(|v| v.as_u64())
                            .map(|n| n as usize)
                            .unwrap_or(4);
                        Box::new(crate::peripherals::nrf52::timer::Nrf52Timer::new_with_cc(
                            num_cc,
                        ))
                    } else {
                        // Width selector for 32-bit TIM2/TIM5 (STM32L4 etc).
                        // YAML: `config: { width: 32 }`. Defaults to 16 for
                        // back-compat with F1-class general-purpose timers.
                        // `advanced: true` enables RCR/BDTR/CCR5/6 (TIM1/TIM8).
                        let width: u8 = p_cfg
                            .config
                            .get("width")
                            .and_then(|v| v.as_u64())
                            .map(|n| n as u8)
                            .unwrap_or(16);
                        let advanced = p_cfg
                            .config
                            .get("advanced")
                            .and_then(|v| v.as_bool())
                            .unwrap_or(false);
                        // `basic: true` (TIM6/TIM7) → counter + UIF only, no
                        // capture/compare channels.
                        let basic = p_cfg
                            .config
                            .get("basic")
                            .and_then(|v| v.as_bool())
                            .unwrap_or(false);
                        Box::new(
                            crate::peripherals::timer::Timer::new_with_layout(width, advanced)
                                .basic(basic),
                        )
                    }
                }
                "i2c"
                | "stm32f1_i2c"
                | "stm32f2_i2c"
                | "stm32f4_i2c"
                | "stm32f7_i2c"
                | "efm32ggi2ccontroller" => {
                    let layout: crate::peripherals::i2c::I2cRegisterLayout =
                        Self::parse_profile_or_default(p_cfg, "I2C")?;
                    let mut i2c = crate::peripherals::i2c::I2c::new_with_layout(layout);
                    for ext in &manifest.external_devices {
                        if ext.connection != p_cfg.id {
                            continue;
                        }
                        match crate::peripherals::components::build_i2c_device(
                            &ext.r#type,
                            &ext.config,
                        ) {
                            Some(device) => {
                                tracing::info!(
                                    "i2c attach: '{}' (type={}) -> '{}'",
                                    ext.id,
                                    ext.r#type,
                                    p_cfg.id
                                );
                                i2c.attach(device);
                            }
                            None => {
                                tracing::warn!(
                                    "i2c attach skipped: unknown device type '{}' for external id '{}' on bus '{}'",
                                    ext.r#type,
                                    ext.id,
                                    p_cfg.id
                                );
                            }
                        }
                    }
                    Box::new(i2c)
                }
                "spi" | "stm32spi" => {
                    let layout: crate::peripherals::spi::SpiRegisterLayout =
                        if p_cfg.r#type.contains("nrf") {
                            crate::peripherals::spi::SpiRegisterLayout::Nrf52Spim
                        } else {
                            Self::parse_profile_or_default(p_cfg, "SPI")?
                        };
                    // Classic-SPI CR2 mask is a per-part delta: F1 0xE7, F4 adds
                    // FRF bit 4 → 0xF7. YAML: `config: { cr2_mask: 0xF7 }`.
                    let cr2_mask: u32 = p_cfg
                        .config
                        .get("cr2_mask")
                        .and_then(|v| v.as_u64())
                        .map(|n| n as u32)
                        .unwrap_or(0x0000_00E7);
                    Box::new(crate::peripherals::spi::Spi::new_with_layout_cr2(
                        layout, cr2_mask,
                    ))
                }
                "pwr" => Box::new(crate::peripherals::pwr::Pwr::new()),
                "flash" | "flash_iface" => {
                    // Layout selected via `config: { profile: stm32f1 | stm32l4 }`
                    // in the chip yaml. Missing/unknown profile keeps the L4
                    // default — backward compatible with existing chip configs.
                    let layout: crate::peripherals::flash::FlashRegisterLayout =
                        Self::parse_profile_or_default(p_cfg, "FLASH")?;
                    Box::new(crate::peripherals::flash::Flash::new_with_layout(layout))
                }
                "rng" => Box::new(crate::peripherals::rng::Rng::new()),
                "crc" => {
                    // IDR scratch register width: 8-bit on F0/F1/L0, 32-bit
                    // on F2+/L4+. YAML: `config: { idr_width: 8 }`; default 32.
                    let idr_width: u8 = p_cfg
                        .config
                        .get("idr_width")
                        .and_then(|v| v.as_u64())
                        .map(|n| n as u8)
                        .unwrap_or(32);
                    Box::new(crate::peripherals::crc::Crc::new().with_idr_width(idr_width))
                }
                "rtc" => Box::new(crate::peripherals::rtc::Rtc::new()),
                "rtc_f1" => Box::new(crate::peripherals::rtc_f1::RtcF1::new()),
                "rtc_v3" => Box::new(crate::peripherals::rtc_v3::RtcV3::new()),
                "iwdg" => Box::new(crate::peripherals::iwdg::Iwdg::new()),
                "wwdg" => Box::new(crate::peripherals::wwdg::Wwdg::new()),
                "dac" => Box::new(crate::peripherals::dac::Dac::new()),
                "lptim" => Box::new(crate::peripherals::lptim::Lptim::new()),
                "quadspi" => Box::new(crate::peripherals::quadspi::Quadspi::new()),
                "sai" => Box::new(crate::peripherals::sai::Sai::new()),
                "usb_otg" => Box::new(crate::peripherals::usb_otg::UsbOtg::new()),
                "bxcan" => Box::new(crate::peripherals::bxcan::BxCan::new()),
                "sdmmc" => Box::new(crate::peripherals::sdmmc::Sdmmc::new()),
                "comp" => Box::new(crate::peripherals::comp::Comp::new()),
                "tsc" => Box::new(crate::peripherals::tsc::Tsc::new()),
                "fmc" => Box::new(crate::peripherals::fmc::Fmc::new()),
                "exti" => {
                    let layout: crate::peripherals::exti::ExtiRegisterLayout =
                        Self::parse_profile_or_default(p_cfg, "EXTI")?;
                    // Implemented-line count is part-specific (F103 = 19). YAML:
                    // `config: { lines: 19 }`; default 20 for back-compat.
                    let lines: u32 = p_cfg
                        .config
                        .get("lines")
                        .and_then(|v| v.as_u64())
                        .map(|n| n as u32)
                        .unwrap_or(20);
                    let line_mask = if lines >= 32 {
                        0xFFFF_FFFF
                    } else {
                        (1u32 << lines) - 1
                    };
                    Box::new(crate::peripherals::exti::Exti::new_with_layout_lines(
                        layout, line_mask,
                    ))
                }
                "afio" => Box::new(crate::peripherals::afio::Afio::new()),
                "dma" | "stm32dma" => Box::new(crate::peripherals::dma::Dma1::new()),
                "gpdma" => Box::new(crate::peripherals::gpdma::Gpdma::new()),
                "adc" => {
                    let layout: crate::peripherals::adc::AdcRegisterLayout =
                        Self::parse_profile_or_default(p_cfg, "ADC")?;
                    Box::new(crate::peripherals::adc::Adc::new_with_layout(layout))
                }
                "pio" => {
                    let mut pio = crate::peripherals::pio::Pio::new();
                    if let Some(program) = p_cfg.config.get("program").and_then(|v| v.as_str()) {
                        pio.load_program_asm(program)?;
                    }
                    Box::new(pio)
                }
                // Nordic peripherals — register-surface models cross-validated
                // by hw-oracle::nrf52_onboarding_diff. See peripherals/nrf52/.
                "nrf52840_rtc" | "nrf52_rtc" => {
                    // RTC1/RTC2 have 4 CC; RTC0 has 3 (default).
                    let num_cc: usize = p_cfg
                        .config
                        .get("num_cc")
                        .and_then(|v| v.as_u64())
                        .map(|n| n as usize)
                        .unwrap_or(3);
                    Box::new(crate::peripherals::nrf52::rtc::Nrf52Rtc::new_with_cc(
                        num_cc,
                    ))
                }
                "nrf52840_rng" | "nrf52_rng" => {
                    Box::new(crate::peripherals::nrf52::rng::Nrf52Rng::new())
                }
                "nrf52840_watchdog" | "nrf52_watchdog" | "nrf52_wdt" => {
                    Box::new(crate::peripherals::nrf52::wdt::Nrf52Wdt::new())
                }
                "nrf52840_ppi" | "nrf52_ppi" => {
                    Box::new(crate::peripherals::nrf52::ppi::Nrf52Ppi::new())
                }
                "nrf52840_pdm" | "nrf52_pdm" => {
                    Box::new(crate::peripherals::nrf52::pdm::Nrf52Pdm::new())
                }
                "nrf52_gpiote" | "nrf52840_gpiotasksevents" => {
                    Box::new(crate::peripherals::nrf52::gpiote::Nrf52Gpiote::new())
                }
                "nrf52840_ecb" | "nrf52_ecb" => {
                    Box::new(crate::peripherals::nrf52::ecb::Nrf52Ecb::new())
                }
                "nrf52_clock" => Box::new(crate::peripherals::nrf52::clock::Nrf52Clock::new()),
                "nrf52840_temp" | "nrf52_temp" => {
                    Box::new(crate::peripherals::nrf52::temp::Nrf52Temp::new())
                }
                "nrf52840_adc" | "nrf52840_saadc" | "nrf52_saadc" => {
                    Box::new(crate::peripherals::nrf52::saadc::Nrf52Saadc::new())
                }
                "nrf52840_pwm" | "nrf52_pwm" => {
                    Box::new(crate::peripherals::nrf52::pwm::Nrf52Pwm::new())
                }
                "nrf52840_qspi" | "nrf52_qspi" => {
                    Box::new(crate::peripherals::nrf52::qspi::Nrf52Qspi::new())
                }
                "nrf52840_nfct" | "nrf52_nfct" => {
                    Box::new(crate::peripherals::nrf52::nfct::Nrf52Nfct::new())
                }
                "nrf52840_ficr" | "nrf52_ficr" => {
                    Box::new(crate::peripherals::nrf52::ficr::Nrf52Ficr::new())
                }
                "nrf52840_uicr" | "nrf52_uicr" => {
                    Box::new(crate::peripherals::nrf52::uicr::Nrf52Uicr::new())
                }
                "nrf52840_nvmc" | "nrf52_nvmc" => {
                    Box::new(crate::peripherals::nrf52::nvmc::Nrf52Nvmc::new())
                }
                "nrf52840_egu" | "nrf52_egu" => {
                    Box::new(crate::peripherals::nrf52::egu::Nrf52Egu::new())
                }
                "nrf52840_comp" | "nrf52_comp" => {
                    Box::new(crate::peripherals::nrf52::comp::Nrf52Comp::new())
                }
                "nrf52840_lpcomp" | "nrf52_lpcomp" => {
                    Box::new(crate::peripherals::nrf52::lpcomp::Nrf52Lpcomp::new())
                }
                "nrf52840_qdec" | "nrf52_qdec" => {
                    Box::new(crate::peripherals::nrf52::qdec::Nrf52Qdec::new())
                }
                "nrf52840_i2s" | "nrf52_i2s" => {
                    Box::new(crate::peripherals::nrf52::i2s::Nrf52I2s::new())
                }
                "nrf52840_mwu" | "nrf52_mwu" => {
                    Box::new(crate::peripherals::nrf52::mwu::Nrf52Mwu::new())
                }
                "nrf52840_aar" | "nrf52_aar" => {
                    Box::new(crate::peripherals::nrf52::aar::Nrf52Aar::new())
                }
                "nrf52840_ccm" | "nrf52_ccm" => {
                    Box::new(crate::peripherals::nrf52::ccm::Nrf52Ccm::new())
                }
                "nrf52840_bprot" | "nrf52_bprot" => {
                    Box::new(crate::peripherals::nrf52::bprot::Nrf52Bprot::new())
                }
                "nrf52840_radio" | "nrf52_radio" => {
                    Box::new(crate::peripherals::nrf52::radio::Nrf52Radio::new())
                }
                "nrf52840_usbd" | "nrf52_usbd" => {
                    Box::new(crate::peripherals::nrf52::usbd::Nrf52Usbd::new())
                }
                "nrf52840_acl" | "nrf52_acl" => {
                    Box::new(crate::peripherals::nrf52::acl::Nrf52Acl::new())
                }
                "nrf52840_cryptocell" | "nrf52_cryptocell" => {
                    Box::new(crate::peripherals::nrf52::cryptocell::Nrf52Cryptocell::new())
                }
                "nrf52840_usbregulator" | "nrf52_usbregulator" => {
                    Box::new(crate::peripherals::nrf52::usbregulator::Nrf52UsbRegulator::new())
                }
                "nrf52840_spis" | "nrf52_spis" => {
                    Box::new(crate::peripherals::nrf52::spis::Nrf52Spis::new())
                }
                "nrf52840_twis" | "nrf52_twis" => {
                    Box::new(crate::peripherals::nrf52::twis::Nrf52Twis::new())
                }
                // TWIM (I²C master with EasyDMA) — nRF52840 PS §6.31.
                // `nrf52840_i2c` is the canonical chip-YAML type; `nrf52840_twim`
                // and `nrf52_twim` are also accepted so firmware configs that
                // name it more precisely still resolve here.
                "nrf52840_twim" | "nrf52_twim" => {
                    let mut twim = crate::peripherals::nrf52::twim::Nrf52Twim::new();
                    for ext in &manifest.external_devices {
                        if ext.connection != p_cfg.id {
                            continue;
                        }
                        match crate::peripherals::components::build_i2c_device(
                            &ext.r#type,
                            &ext.config,
                        ) {
                            Some(device) => {
                                tracing::info!(
                                    "twim attach: '{}' (type={}) -> '{}'",
                                    ext.id,
                                    ext.r#type,
                                    p_cfg.id
                                );
                                twim.attach(device);
                            }
                            None => {
                                tracing::warn!(
                                    "twim attach skipped: unknown device type '{}' \
                                     for external id '{}' on bus '{}'",
                                    ext.r#type,
                                    ext.id,
                                    p_cfg.id
                                );
                            }
                        }
                    }
                    Box::new(twim)
                }
                // ESP32-family Timer Group (TIMG0/TIMG1) — the same IP block is
                // used by the classic ESP32, S3, and C3.  All share the register
                // layout: T0CONFIG=0x00, T0LO=0x04, T0HI=0x08, T0UPDATE=0x0C.
                // Wiring via this type string gives C3 (RISC-V, from_config path)
                // the same live counter that the Xtensa chips get via their
                // hard-wired system builders.
                "esp32_timg" => Box::new(crate::peripherals::esp32::timg::Timg::new(
                    p_cfg.base_address as u32,
                )),
                "declarative" => {
                    let descriptor_path = p_cfg
                        .config
                        .get("path")
                        .and_then(|v| v.as_str())
                        .ok_or_else(|| {
                            anyhow::anyhow!(
                                "Field 'path' is required in 'config' for declarative peripheral '{}'",
                                p_cfg.id
                            )
                        })?;

                    let resolved_path = Self::resolve_peripheral_path(manifest, descriptor_path);
                    let desc = labwired_config::PeripheralDescriptor::from_file(&resolved_path)
                        .with_context(|| {
                            format!(
                                "Failed to load declarative descriptor for '{}' from '{}' (resolved to '{}')",
                                p_cfg.id,
                                descriptor_path,
                                resolved_path.display()
                            )
                        })?;

                    Box::new(crate::peripherals::declarative::GenericPeripheral::new(
                        desc,
                    ))
                }
                "strict_ir" => {
                    let descriptor_path = p_cfg
                        .config
                        .get("path")
                        .and_then(|v| v.as_str())
                        .ok_or_else(|| {
                            anyhow::anyhow!(
                                "Field 'path' is required in 'config' for strict_ir peripheral '{}'",
                                p_cfg.id
                            )
                        })?;

                    let resolved_path = Self::resolve_peripheral_path(manifest, descriptor_path);
                    let content = std::fs::read_to_string(&resolved_path).with_context(|| {
                        format!(
                            "Failed to read IR file '{}' (resolved to '{}')",
                            descriptor_path,
                            resolved_path.display()
                        )
                    })?;
                    let ir_peripheral = match serde_json::from_str::<labwired_ir::IrPeripheral>(
                        &content,
                    ) {
                        Ok(peripheral) => peripheral,
                        Err(peripheral_err) => {
                            let device: labwired_ir::IrDevice = serde_json::from_str(&content)
                                .with_context(|| {
                                    format!(
                                        "Failed to parse Strict IR from {} as IrPeripheral ({}) or IrDevice",
                                        resolved_path.display(),
                                        peripheral_err
                                    )
                                })?;

                            if let Some(peripheral) = device.peripherals.get(&p_cfg.id) {
                                peripheral.clone()
                            } else if device.peripherals.len() == 1 {
                                device
                                    .peripherals
                                    .into_values()
                                    .next()
                                    .expect("len() checked above")
                            } else {
                                let available = device
                                    .peripherals
                                    .keys()
                                    .cloned()
                                    .collect::<Vec<_>>()
                                    .join(", ");
                                return Err(anyhow::anyhow!(
                                    "Strict IR '{}' contains multiple peripherals [{}]; no match for id '{}'",
                                    resolved_path.display(),
                                    available,
                                    p_cfg.id
                                ));
                            }
                        }
                    };

                    let desc: labwired_config::PeripheralDescriptor = ir_peripheral.into();

                    Box::new(crate::peripherals::declarative::GenericPeripheral::new(
                        desc,
                    ))
                }
                "strict_ir_internal" => {
                    let val = p_cfg.config.get("internal_ir_peripheral").ok_or_else(|| {
                        anyhow::anyhow!("Missing internal_ir_peripheral config for converted IR")
                    })?;
                    // Convert yaml Value (which was serde_yaml::to_value(p)) back to IrPeripheral
                    let ir_peripheral: labwired_ir::IrPeripheral =
                        serde_yaml::from_value(val.clone())?;
                    let desc: labwired_config::PeripheralDescriptor = ir_peripheral.into();

                    Box::new(crate::peripherals::declarative::GenericPeripheral::new(
                        desc,
                    ))
                }
                _other => {
                    tracing::debug!(
                        "Mapping unknown peripheral type '{}' to Stub for id '{}'",
                        p_cfg.r#type,
                        p_cfg.id
                    );
                    Box::new(crate::peripherals::stub::StubPeripheral::new(0x00))
                }
            };

            // Map peripheral window size + IRQ from descriptor when provided.
            // Defaults keep older descriptors working.
            let size = if let Some(size) = &p_cfg.size {
                parse_size(size)?
            } else {
                0x1000 // Default 4KB page
            };

            // SysTick raises its IRQ via PeripheralTickResult::system_exception,
            // not via the NVIC IRQ position field — leave its irq as None
            // unless the yaml explicitly sets one.
            let irq = p_cfg.irq;

            bus.peripherals.push(PeripheralEntry {
                name: p_cfg.id.clone(),
                base: p_cfg.base_address,
                size,
                irq,
                dev,
                ticks_remaining: 0,
                generation: 0,
            });
        }

        for ext in &manifest.external_devices {
            // First-pass: peripherals that have migrated to the unified
            // `PeripheralKit` contract are dispatched through the registry,
            // so each one ships its own `attach` next to its model instead
            // of a hand-written arm here.
            if let Some(kit) = crate::peripherals::kit::registry::lookup(&ext.r#type) {
                let mut ctx = crate::peripherals::kit::AttachCtx::new(&mut bus, ext);
                kit.attach(&mut ctx)?;
                continue;
            }
            match ext.r#type.as_str() {
                // ili9341, adxl345/mpu6050/bme280/oled-ssd1306, neo6m-gps,
                // and bg770a-cellular dispatch through the PeripheralKit
                // registry above — see `peripherals::kit`.
                // iolink-master dispatches through the PeripheralKit registry above.
                // max31855, sn74hc165, ssd1680_tricolor_290, and pcd8544
                // dispatch through the PeripheralKit registry above.
                "hc-sr04" | "hcsr04" => {
                    // GPIO-wired ultrasonic sensor — no SPI/I2C connection. The
                    // bus services it each tick: reads TRIG (an MCU output) and
                    // drives ECHO (an MCU input) with a distance-proportional
                    // pulse. `distance_cm` is the host-controlled "hand position".
                    let trig = ext
                        .config
                        .get("trig_pin")
                        .and_then(|v| v.as_str())
                        .unwrap_or("PA8")
                        .to_string();
                    let echo = ext
                        .config
                        .get("echo_pin")
                        .and_then(|v| v.as_str())
                        .unwrap_or("PA9")
                        .to_string();
                    let distance_cm = ext
                        .config
                        .get("distance_cm")
                        .and_then(|v| v.as_f64())
                        .unwrap_or(50.0) as f32;
                    let cpu_hz = ext
                        .config
                        .get("cpu_hz")
                        .and_then(|v| v.as_u64())
                        .unwrap_or(80_000_000);

                    let (trig_addr, trig_bit) =
                        Self::resolve_pin_odr(&bus, &trig).ok_or_else(|| {
                            anyhow::anyhow!(
                                "HC-SR04 '{}' trig_pin '{}' could not be resolved to a GPIO",
                                ext.id,
                                trig
                            )
                        })?;
                    let (echo_addr, echo_bit) =
                        Self::resolve_pin_idr(&bus, &echo).ok_or_else(|| {
                            anyhow::anyhow!(
                                "HC-SR04 '{}' echo_pin '{}' could not be resolved to a GPIO",
                                ext.id,
                                echo
                            )
                        })?;

                    bus.hcsr04.push(crate::peripherals::hc_sr04::HcSr04::new(
                        ext.id.clone(),
                        trig_addr,
                        trig_bit,
                        echo_addr,
                        echo_bit,
                        cpu_hz,
                        distance_cm,
                    ));
                }
                // ntc-thermistor dispatches through the PeripheralKit registry above.
                _ => {
                    tracing::warn!(
                        "Unsupported external device '{}' type '{}' on connection '{}'; skipping",
                        ext.id,
                        ext.r#type,
                        ext.connection
                    );
                    continue;
                }
            }
        }

        bus.rebuild_peripheral_ranges();
        // Per-config walk-deletion opt-in. The field is only consulted under the
        // `event-scheduler` feature (the legacy build always walks), so this is a
        // no-op there. Safe only because the manifest author verified the
        // firmware runs byte-identical walk-free (see the walk-identity test).
        bus.legacy_walk_disabled = manifest.walk_deleted;
        Ok(bus)
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

    pub fn read_u32(&self, addr: u64) -> SimResult<u32> {
        if self.config.optimized_bus_access {
            if let Some(val) = self.ram.read_u32(addr) {
                return Ok(val);
            }
            if let Some(val) = self.flash.read_u32(addr) {
                return Ok(val);
            }
            for mem in &self.extra_mem {
                if let Some(val) = mem.read_u32(addr) {
                    return Ok(val);
                }
            }
            // Boot alias handle
            if self.flash.base_addr != 0 {
                let alias_end = self.flash.data.len() as u64;
                if addr + 3 < alias_end {
                    if let Some(val) = self.flash.read_u32(self.flash.base_addr + addr) {
                        return Ok(val);
                    }
                }
            }
        }

        if let Some(idx) = self.find_peripheral_index(addr) {
            let p = &self.peripherals[idx];
            return p.dev.read_u32(addr - p.base);
        }

        let b0 = self.read_u8(addr)? as u32;
        let b1 = self.read_u8(addr + 1)? as u32;
        let b2 = self.read_u8(addr + 2)? as u32;
        let b3 = self.read_u8(addr + 3)? as u32;
        Ok(b0 | (b1 << 8) | (b2 << 16) | (b3 << 24))
    }

    pub fn write_u32(&mut self, addr: u64, value: u32) -> SimResult<()> {
        if self.config.optimized_bus_access && self.ram.write_u32(addr, value) {
            return Ok(());
        }
        if self.config.optimized_bus_access {
            for mem in &mut self.extra_mem {
                if mem.write_u32(addr, value) {
                    return Ok(());
                }
            }
        }
        // Flash is read-only via bus writes usually, but let's stick to the behavior of write_u8
        // which would likely fail or do nothing if it's flash.
        // Actually write_u8 checks flash_alias_old etc.

        if let Some(idx) = self.find_peripheral_index(addr) {
            #[cfg(feature = "event-scheduler")]
            self.sync_scheduler_peripheral(idx);
            self.maybe_latch_dc(idx);
            let p = &mut self.peripherals[idx];
            p.ticks_remaining = 0;
            let r = p.dev.write_u32(addr - p.base, value);
            self.maybe_arm_hcsr04(idx);
            #[cfg(feature = "event-scheduler")]
            self.collect_scheduled_events(idx);
            return r;
        }

        self.write_u8(addr, (value & 0xFF) as u8)?;
        self.write_u8(addr + 1, ((value >> 8) & 0xFF) as u8)?;
        self.write_u8(addr + 2, ((value >> 16) & 0xFF) as u8)?;
        self.write_u8(addr + 3, ((value >> 24) & 0xFF) as u8)?;
        Ok(())
    }

    pub fn read_u16(&self, addr: u64) -> SimResult<u16> {
        if self.config.optimized_bus_access {
            if let Some(val) = self.ram.read_u16(addr) {
                return Ok(val);
            }
            if let Some(val) = self.flash.read_u16(addr) {
                return Ok(val);
            }
            for mem in &self.extra_mem {
                if let Some(val) = mem.read_u16(addr) {
                    return Ok(val);
                }
            }
            // Boot alias handle
            if self.flash.base_addr != 0 {
                let alias_end = self.flash.data.len() as u64;
                if addr + 1 < alias_end {
                    if let Some(val) = self.flash.read_u16(self.flash.base_addr + addr) {
                        return Ok(val);
                    }
                }
            }
        }

        if let Some(idx) = self.find_peripheral_index(addr) {
            let p = &self.peripherals[idx];
            return p.dev.read_u16(addr - p.base);
        }

        let b0 = self.read_u8(addr)? as u16;
        let b1 = self.read_u8(addr + 1)? as u16;
        Ok(b0 | (b1 << 8))
    }

    pub fn write_u16(&mut self, addr: u64, value: u16) -> SimResult<()> {
        if self.config.optimized_bus_access && self.ram.write_u16(addr, value) {
            return Ok(());
        }
        if let Some(idx) = self.find_peripheral_index(addr) {
            #[cfg(feature = "event-scheduler")]
            self.sync_scheduler_peripheral(idx);
            self.maybe_latch_dc(idx);
            let p = &mut self.peripherals[idx];
            p.ticks_remaining = 0;
            let r = p.dev.write_u16(addr - p.base, value);
            self.maybe_arm_hcsr04(idx);
            #[cfg(feature = "event-scheduler")]
            self.collect_scheduled_events(idx);
            return r;
        }

        self.write_u8(addr, (value & 0xFF) as u8)?;
        self.write_u8(addr + 1, ((value >> 8) & 0xFF) as u8)?;
        Ok(())
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
        self.collect_enabled_nvic_interrupts(&mut interrupts);

        (interrupts, costs, dma_requests)
    }

    pub fn tick_peripherals_fully(&mut self) -> (Vec<u32>, Vec<PeripheralTickCost>) {
        let (mut interrupts, costs, pending_dma, dma_signals, explicit_source_ids) =
            self.tick_peripherals_phase1();
        // Plan 3: route ESP32-S3 source IDs through the intmatrix.
        self.aggregate_esp32s3_explicit_irqs(&explicit_source_ids);

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
                    if let Ok(val) = self.read_u8(req.src_addr) {
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
    fn bit_band_translate(addr: u64) -> Option<(u64, u8)> {
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

impl crate::Bus for SystemBus {
    fn read_u8(&self, addr: u64) -> SimResult<u8> {
        if let Some(val) = self.ram.read_u8(addr) {
            return Ok(val);
        }
        if let Some(val) = self.flash.read_u8(addr) {
            return Ok(val);
        }
        for mem in &self.extra_mem {
            if let Some(val) = mem.read_u8(addr) {
                return Ok(val);
            }
        }
        // Cortex-M boot alias: address 0x0000_0000 mirrors flash start on many STM32 parts.
        // This lets reset-vector fetch work when flash is configured at 0x0800_0000.
        if self.flash.base_addr != 0 {
            let alias_end = self.flash.data.len() as u64;
            if addr < alias_end {
                if let Some(val) = self.flash.read_u8(self.flash.base_addr + addr) {
                    return Ok(val);
                }
            }
        }

        // Dynamic Peripherals
        if let Some(idx) = self.find_peripheral_index(addr) {
            let p = &self.peripherals[idx];
            return p.dev.read(addr - p.base);
        }

        if std::env::var("LABWIRED_TRACE_VIOLATIONS").is_ok() {
            eprintln!("BUS_VIOLATION read_u8 addr=0x{:08X}", addr);
        }
        Err(SimulationError::MemoryViolation(addr))
    }

    fn write_u8(&mut self, addr: u64, value: u8) -> SimResult<()> {
        let flash_alias_old = if self.flash.base_addr != 0 && addr < self.flash.data.len() as u64 {
            self.flash.read_u8(self.flash.base_addr + addr)
        } else {
            None
        };

        // Avoid calling `read_u8` here since peripheral reads may carry side effects.
        let old_value = self
            .ram
            .read_u8(addr)
            .or_else(|| self.flash.read_u8(addr))
            .or(flash_alias_old)
            .or_else(|| self.extra_mem.iter().find_map(|m| m.read_u8(addr)))
            .or_else(|| {
                self.find_peripheral_index(addr).and_then(|idx| {
                    let p = &self.peripherals[idx];
                    p.dev.peek(addr - p.base)
                })
            })
            .unwrap_or(0);

        let flash_alias_write = self.flash.base_addr != 0
            && addr < self.flash.data.len() as u64
            && self.flash.write_u8(self.flash.base_addr + addr, value);

        let res = if self.ram.write_u8(addr, value)
            || self.flash.write_u8(addr, value)
            || flash_alias_write
            || self.extra_mem.iter_mut().any(|m| m.write_u8(addr, value))
        {
            Ok(())
        } else {
            // Dynamic Peripherals
            if let Some(idx) = self.find_peripheral_index(addr) {
                #[cfg(feature = "event-scheduler")]
                self.sync_scheduler_peripheral(idx);
                self.maybe_latch_dc(idx);
                let p = &mut self.peripherals[idx];
                let r = p.dev.write(addr - p.base, value);
                self.maybe_arm_hcsr04(idx);
                #[cfg(feature = "event-scheduler")]
                self.collect_scheduled_events(idx);
                r
            } else {
                if std::env::var("LABWIRED_TRACE_VIOLATIONS").is_ok() {
                    eprintln!(
                        "BUS_VIOLATION write_u8 addr=0x{:08X} val=0x{:02X}",
                        addr, value
                    );
                }
                Err(SimulationError::MemoryViolation(addr))
            }
        };

        if res.is_ok() {
            // Wake up the peripheral
            if let Some(idx) = self.find_peripheral_index(addr) {
                self.peripherals[idx].ticks_remaining = 0;
            }

            // Trigger observers
            for observer in &self.observers {
                observer.on_memory_write(addr, old_value, value);
            }
        }

        res
    }

    fn read_u16(&self, addr: u64) -> SimResult<u16> {
        if let Some(val) = self.ram.read_u16(addr) {
            return Ok(val);
        }
        if let Some(val) = self.flash.read_u16(addr) {
            return Ok(val);
        }
        if self.flash.base_addr != 0 && addr + 1 < self.flash.data.len() as u64 {
            if let Some(val) = self.flash.read_u16(self.flash.base_addr + addr) {
                return Ok(val);
            }
        }
        if let Some(idx) = self.find_peripheral_index(addr) {
            let p = &self.peripherals[idx];
            return p.dev.read_u16(addr - p.base);
        }
        let b0 = self.read_u8(addr)? as u16;
        let b1 = self.read_u8(addr + 1)? as u16;
        Ok(b0 | (b1 << 8))
    }

    fn read_u32(&self, addr: u64) -> SimResult<u32> {
        // Cortex-M bit-band alias: return 0 or 1 based on the physical bit.
        if self.bit_band_enabled {
            if let Some((phys_byte, bit)) = Self::bit_band_translate(addr) {
                let byte_val = self.read_u8(phys_byte)?;
                return Ok(((byte_val >> bit) & 1) as u32);
            }
        }

        if let Some(val) = self.ram.read_u32(addr) {
            return Ok(val);
        }
        if let Some(val) = self.flash.read_u32(addr) {
            return Ok(val);
        }
        if self.flash.base_addr != 0 && addr + 3 < self.flash.data.len() as u64 {
            if let Some(val) = self.flash.read_u32(self.flash.base_addr + addr) {
                return Ok(val);
            }
        }
        if let Some(idx) = self.find_peripheral_index(addr) {
            let p = &self.peripherals[idx];
            return p.dev.read_u32(addr - p.base);
        }
        let b0 = self.read_u8(addr)? as u32;
        let b1 = self.read_u8(addr + 1)? as u32;
        let b2 = self.read_u8(addr + 2)? as u32;
        let b3 = self.read_u8(addr + 3)? as u32;
        Ok(b0 | (b1 << 8) | (b2 << 16) | (b3 << 24))
    }

    fn write_u16(&mut self, addr: u64, value: u16) -> SimResult<()> {
        let mut wrote = self.ram.write_u16(addr, value) || self.flash.write_u16(addr, value);
        if !wrote && self.flash.base_addr != 0 && addr + 1 < self.flash.data.len() as u64 {
            wrote = self.flash.write_u16(self.flash.base_addr + addr, value);
        }
        if wrote {
            return Ok(());
        }
        if let Some(idx) = self.find_peripheral_index(addr) {
            #[cfg(feature = "event-scheduler")]
            self.sync_scheduler_peripheral(idx);
            self.maybe_latch_dc(idx);
            let p = &mut self.peripherals[idx];
            p.ticks_remaining = 0;
            let r = p.dev.write_u16(addr - p.base, value);
            self.maybe_arm_hcsr04(idx);
            #[cfg(feature = "event-scheduler")]
            self.collect_scheduled_events(idx);
            return r;
        }
        self.write_u8(addr, (value & 0xFF) as u8)?;
        self.write_u8(addr + 1, ((value >> 8) & 0xFF) as u8)?;
        Ok(())
    }

    fn write_u32(&mut self, addr: u64, value: u32) -> SimResult<()> {
        // Cortex-M bit-band alias translation (peripheral: 0x42000000-0x43FFFFFF,
        // SRAM: 0x22000000-0x23FFFFFF).  Each alias word maps to one bit of the
        // physical address.  Writing 1 sets the bit; writing 0 clears it.
        if self.bit_band_enabled {
            if let Some((phys_byte, bit)) = Self::bit_band_translate(addr) {
                let old = self.read_u8(phys_byte)?;
                let new_byte = if value & 1 != 0 {
                    old | (1 << bit)
                } else {
                    old & !(1 << bit)
                };
                return self.write_u8(phys_byte, new_byte);
            }
        }

        let mut wrote = self.ram.write_u32(addr, value) || self.flash.write_u32(addr, value);
        if !wrote && self.flash.base_addr != 0 && addr + 3 < self.flash.data.len() as u64 {
            wrote = self.flash.write_u32(self.flash.base_addr + addr, value);
        }
        if wrote {
            return Ok(());
        }
        if let Some(idx) = self.find_peripheral_index(addr) {
            #[cfg(feature = "event-scheduler")]
            self.sync_scheduler_peripheral(idx);
            self.maybe_latch_dc(idx);
            let p = &mut self.peripherals[idx];
            p.ticks_remaining = 0;
            let r = p.dev.write_u32(addr - p.base, value);
            self.maybe_arm_hcsr04(idx);
            #[cfg(feature = "event-scheduler")]
            self.collect_scheduled_events(idx);
            return r;
        }
        self.write_u8(addr, (value & 0xFF) as u8)?;
        self.write_u8(addr + 1, ((value >> 8) & 0xFF) as u8)?;
        self.write_u8(addr + 2, ((value >> 16) & 0xFF) as u8)?;
        self.write_u8(addr + 3, ((value >> 24) & 0xFF) as u8)?;
        Ok(())
    }

    /// Fast-path fetch slice for the CPU instruction-fetch cache
    /// (#119 Phase 1.2). Returns `Some((base, end, slice))` when `pc`
    /// lands inside a `RamPeripheral` we can serve directly; falls
    /// through to `None` (slow path) for any other peripheral kind or
    /// unmapped addresses.
    ///
    /// The returned slice borrows the peripheral's backing buffer for
    /// the duration of the call. The CPU stashes a raw pointer derived
    /// from it; the `RamPeripheral` INVARIANT (no resize) keeps that
    /// pointer valid until the peripheral is dropped, but the CPU MUST
    /// invalidate the cache on any bus write into the cached range
    /// and on snapshot restore. Reads from non-RAM peripherals (e.g.
    /// `RomThunkBank`, GPIO, declarative peripherals) keep going
    /// through the slow path so side effects fire as before.
    fn fetch_slice(&self, pc: u64) -> Option<(u64, u64, &[u8])> {
        let idx = self.find_peripheral_index(pc)?;
        let entry = self.peripherals.get(idx)?;
        let any = entry.dev.as_any()?;
        let ram = any.downcast_ref::<crate::system::xtensa::RamPeripheral>()?;
        let (ptr, len) = ram.backing_ptr_len();
        // SAFETY: `RamPeripheral`'s backing `Vec` is fixed-size at
        // construction (see struct-level INVARIANT in
        // `system::xtensa::RamPeripheral`). The `&self` borrow on the
        // bus keeps the peripheral entry alive for the duration of
        // this borrow. We're only producing a read-only `&[u8]` from
        // a `*const u8`; no concurrent `borrow_mut` is in flight
        // because reads don't mutate.
        let slice = unsafe { std::slice::from_raw_parts(ptr, len) };
        Some((entry.base, entry.base.saturating_add(entry.size), slice))
    }

    fn execute_dma(&mut self, requests: &[crate::DmaRequest]) -> SimResult<()> {
        for req in requests {
            match req.direction {
                crate::DmaDirection::Read => {
                    let _ = self.read_u8(req.addr)?;
                }
                crate::DmaDirection::Write => {
                    self.write_u8(req.addr, req.val)?;
                }
                crate::DmaDirection::Copy => {
                    let val = self.read_u8(req.src_addr)?;
                    self.write_u8(req.addr, val)?;
                }
            }
        }
        Ok(())
    }

    fn config(&self) -> &crate::SimulationConfig {
        &self.config
    }

    fn as_any(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }

    fn as_any_mut(&mut self) -> Option<&mut dyn std::any::Any> {
        Some(self)
    }

    fn tick_peripherals(&mut self) -> Vec<u32> {
        let (interrupts, _costs) = self.tick_peripherals_fully();
        interrupts
    }

    fn clear_nvic_pending(&mut self, exception_num: u32) {
        if exception_num >= 16 {
            if let Some(nvic) = &self.nvic {
                let irq = exception_num - 16;
                let idx = (irq / 32) as usize;
                let bit = irq % 32;
                if idx < 8 {
                    nvic.ispr[idx].fetch_and(!(1 << bit), Ordering::SeqCst);
                }
            }
        }
    }

    fn get_rom_thunk(
        &self,
        pc: u32,
    ) -> Option<crate::peripherals::esp32s3::rom_thunks::RomThunkFn> {
        SystemBus::get_rom_thunk(self, pc)
    }

    fn route_irq_source_to_cpu_irq(&self, source_id: u32) -> Option<u8> {
        SystemBus::route_irq_source_to_cpu_irq(self, source_id)
    }

    fn pending_cpu_irqs(&self, core_id: u8) -> u32 {
        // Two cross-core delivery paths coexist after the dual-core merge:
        //   * ESP32-S3 (intmatrix registered): the aggregator routes every
        //     asserting source — including the FROM_CPU IPI sources
        //     79→core0 / 80→core1 — into this per-core array.
        //   * ESP32-classic (no intmatrix, DPORT instead): the array stays
        //     empty and cross-core FROM_CPU IPIs come from the DPORT matrix.
        // Each path contributes 0 on the other chip, so OR-ing is safe and
        // keeps both dual-core models working.
        self.pending_cpu_irqs[(core_id & 1) as usize] | self.dport_cross_core_pending(core_id)
    }

    fn clear_cpu_irq_pending(&mut self, core_id: u8, slot: u8) {
        self.pending_cpu_irqs[(core_id & 1) as usize] &= !(1u32 << slot);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use labwired_config::{ChipDescriptor, SystemManifest};
    use std::path::PathBuf;

    /// Minimal fixed-value peripheral for routing tests: reads return a
    /// constant tag byte, writes are ignored.
    #[derive(Debug)]
    struct TagPeripheral(u8);
    impl crate::Peripheral for TagPeripheral {
        fn read(&self, _offset: u64) -> crate::SimResult<u8> {
            Ok(self.0)
        }
        fn write(&mut self, _offset: u64, _value: u8) -> crate::SimResult<()> {
            Ok(())
        }
    }

    /// Routing must be a pure function of the address — never of access
    /// history. A broad catch-all window with a narrower twin layered inside
    /// it (the ESP32-S3 low-MMIO + per-peripheral twin pattern) must route
    /// the twin's addresses to the twin even when the immediately preceding
    /// access touched a broad-window-only address (which seeds the hint
    /// cache with the broad entry — containment alone must not let it
    /// short-circuit the canonical last-start-wins search).
    #[test]
    fn pin_labels_parse_for_both_vendor_forms() {
        // STM32 letter ports.
        assert_eq!(
            SystemBus::parse_stm32_pin("PC7"),
            Some(("gpioc".to_string(), 7))
        );
        assert_eq!(SystemBus::parse_stm32_pin("PA16"), None); // STM32 ports stop at 15
                                                              // Nordic numbered ports: nRF52840 P0.00-P0.31, P1.00-P1.15.
        assert_eq!(
            SystemBus::parse_stm32_pin("P0.04"),
            Some(("gpio0".to_string(), 4))
        );
        assert_eq!(
            SystemBus::parse_stm32_pin("P1.15"),
            Some(("gpio1".to_string(), 15))
        );
        assert_eq!(SystemBus::parse_stm32_pin("P0.32"), None);
        assert_eq!(SystemBus::parse_stm32_pin("P0."), None);
    }

    #[test]
    fn overlapping_windows_route_history_independently() {
        let mut bus = SystemBus::new();
        // Broad catch-all: 0x7000_0000..0x7000_8000, reads 0xBB.
        bus.add_peripheral(
            "broad",
            0x7000_0000,
            0x8000,
            None,
            Box::new(TagPeripheral(0xBB)),
        );
        // Narrow twin layered inside: 0x7000_4000..0x7000_5000, reads 0xAA.
        bus.add_peripheral(
            "narrow",
            0x7000_4000,
            0x1000,
            None,
            Box::new(TagPeripheral(0xAA)),
        );

        // Cold route: twin wins its window.
        assert_eq!(
            bus.read_u8(0x7000_4000).unwrap(),
            0xAA,
            "cold: twin owns it"
        );

        // Poison the hint with the broad entry, then re-route a twin address.
        assert_eq!(
            bus.read_u8(0x7000_0008).unwrap(),
            0xBB,
            "broad-only address"
        );
        assert_eq!(
            bus.read_u8(0x7000_4FFC).unwrap(),
            0xAA,
            "hint poisoned by broad entry must not hijack the twin's window"
        );

        // resolve_window must agree with dispatch, in both hint states.
        assert_eq!(bus.read_u8(0x7000_0008).unwrap(), 0xBB); // re-poison
        assert_eq!(
            bus.resolve_window(0x7000_4000),
            Some((0x7000_4000, 0x1000)),
            "resolve_window must return the twin, not the hinted broad entry"
        );

        // Addresses in the broad window above the twin still go broad —
        // including right after a twin access (reverse poisoning), and the
        // fallback must pick the GREATEST containing start, not the
        // first-registered entry.
        assert_eq!(bus.read_u8(0x7000_4000).unwrap(), 0xAA);
        assert_eq!(
            bus.read_u8(0x7000_5000).unwrap(),
            0xBB,
            "past the twin's end the broad window resumes"
        );

        // next_window_start: the twin's start bounds the broad window's
        // uniform service region (used by the coverage probe's baseline).
        assert_eq!(bus.next_window_start(0x7000_0000), Some(0x7000_4000));
        assert_eq!(
            bus.next_window_start(0x7000_4000),
            Some(0xE000_E010),
            "above the twin the next start is the default bus's systick"
        );
    }

    #[test]
    fn test_system_bus_from_config_declarative() {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let chip_path = root.join("tests/fixtures/test_chip_declarative.yaml");
        let manifest_path = root.join("tests/fixtures/test_system_declarative.yaml");

        let chip = ChipDescriptor::from_file(&chip_path).unwrap();
        let manifest = SystemManifest::from_file(&manifest_path).unwrap();

        let bus =
            SystemBus::from_config(&chip, &manifest).expect("Failed to create bus from config");

        // Verify TIMER1 is present at 0x40001000
        let found = bus
            .peripherals
            .iter()
            .find(|p| p.name == "TIMER1")
            .expect("TIMER1 not found");
        assert_eq!(found.base, 0x40001000);
        assert_eq!(found.size, 1024);

        // Verify we can read/write to it through the bus
        // Address 0x40001000 + 0x00 = CTRL register (reset value 0)
        let ctrl_val = bus.read_u32(0x40001000).unwrap();
        assert_eq!(ctrl_val, 0);

        // Address 0x40001000 + 0x04 = COUNT register
        let mut bus = bus;
        bus.write_u32(0x40001004, 0x12345678).unwrap();
        let count_val = bus.read_u32(0x40001004).unwrap();
        assert_eq!(count_val, 0x12345678);
    }

    #[test]
    fn test_system_bus_resolves_descriptor_path_relative_to_chip_file() {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let chip_path = root.join("tests/fixtures/test_chip_declarative.yaml");
        let manifest_path = root.join("tests/fixtures/test_system_declarative.yaml");

        let mut chip = ChipDescriptor::from_file(&chip_path).unwrap();
        let mut manifest = SystemManifest::from_file(&manifest_path).unwrap();

        // Simulate a descriptor path that is relative to chip.yaml location.
        if let Some(path) = chip.peripherals[0].config.get_mut("path") {
            *path = serde_yaml::Value::String("test_timer_descriptor.yaml".to_string());
        }
        manifest.chip = chip_path.to_string_lossy().into_owned();

        let bus =
            SystemBus::from_config(&chip, &manifest).expect("Failed to create bus from config");

        let found = bus
            .peripherals
            .iter()
            .find(|p| p.name == "TIMER1")
            .expect("TIMER1 not found");
        assert_eq!(found.base, 0x40001000);
    }

    #[test]
    fn test_from_config_attaches_adxl345_external_device_to_i2c() {
        use labwired_config::{
            Arch, ChipDescriptor, ExternalDevice, MemoryRange, PeripheralConfig, SystemManifest,
        };
        use std::collections::HashMap;

        let chip = ChipDescriptor {
            schema_version: "1.0".to_string(),
            name: "stm32f103-test".to_string(),
            arch: Arch::Arm,
            core: None,
            flash: MemoryRange {
                base: 0x0800_0000,
                size: "64KB".to_string(),
            },
            ram: MemoryRange {
                base: 0x2000_0000,
                size: "20KB".to_string(),
            },
            peripherals: vec![PeripheralConfig {
                id: "i2c1".to_string(),
                r#type: "i2c".to_string(),
                base_address: 0x4000_5400,
                size: Some("1KB".to_string()),
                irq: Some(31),
                config: HashMap::new(),
            }],
        };

        let mut config = HashMap::new();
        config.insert(
            "i2c_address".to_string(),
            serde_yaml::Value::Number(0x53.into()),
        );
        let manifest = SystemManifest {
            walk_deleted: false,
            schema_version: "1.0".to_string(),
            name: "adxl345-test".to_string(),
            chip: "../chips/stm32f103.yaml".to_string(),
            memory_overrides: HashMap::new(),
            external_devices: vec![ExternalDevice {
                id: "adxl345".to_string(),
                r#type: "adxl345".to_string(),
                connection: "i2c1".to_string(),
                config,
            }],
            board_io: Vec::new(),
            peripherals: Vec::new(),
        };

        let mut bus = SystemBus::from_config(&chip, &manifest).unwrap();
        let i2c_idx = bus.find_peripheral_index_by_name("i2c1").unwrap();
        let any = bus.peripherals[i2c_idx].dev.as_any_mut().unwrap();
        let i2c = any.downcast_mut::<crate::peripherals::i2c::I2c>().unwrap();
        assert_eq!(i2c.attached_devices().len(), 1);
    }

    /// Parse a minimal chip yaml with the given header lines (name/arch/core).
    fn bit_band_test_chip(header: &str, gpio_base: &str, gpio_profile: &str) -> ChipDescriptor {
        let yaml = format!(
            r#"
{header}
flash:
  base: 0x08000000
  size: "128KB"
ram:
  base: 0x20000000
  size: "64KB"
peripherals:
  - id: "gpiox"
    type: "gpio"
    base_address: {gpio_base}
    size: "1KB"
    config:
      profile: "{gpio_profile}"
"#
        );
        serde_yaml::from_str(&yaml).expect("test chip yaml must parse")
    }

    fn empty_manifest() -> SystemManifest {
        SystemManifest {
            walk_deleted: false,
            schema_version: "1.0".to_string(),
            name: "bit-band-test".to_string(),
            chip: "unused".to_string(),
            memory_overrides: std::collections::HashMap::new(),
            external_devices: Vec::new(),
            board_io: Vec::new(),
            peripherals: Vec::new(),
        }
    }

    /// Cortex-M33 parts (STM32H5/WBA) have no bit-band feature and map real
    /// peripherals inside 0x4200_0000-0x43FF_FFFF. Word accesses there must
    /// reach the peripheral model, never be alias-translated.
    #[test]
    fn from_config_m33_gpio_in_alias_range_receives_word_accesses() {
        let chip = bit_band_test_chip(
            "name: \"m33-test\"\narch: \"arm\"\ncore: \"cortex-m33\"",
            "0x42020400",
            "stm32v2",
        );
        let mut bus = SystemBus::from_config(&chip, &empty_manifest()).unwrap();

        // Go through the `crate::Bus` trait — the CPU's access path, where
        // bit-band translation lives (the inherent methods skip it).
        // BSRR (V2 offset 0x18): set pin 0.
        crate::Bus::write_u32(&mut bus, 0x4202_0418, 0x0000_0001)
            .expect("BSRR word write must reach the GPIO model, not bit-band");
        // ODR (V2 offset 0x14) must show the pin high.
        let odr = crate::Bus::read_u32(&bus, 0x4202_0414)
            .expect("ODR word read must reach the GPIO model, not bit-band");
        assert_eq!(odr & 1, 1, "GPIO BSRR write was shadowed by bit-band alias");
    }

    /// Cortex-M3 parts (STM32F1) DO have the bit-band feature: word accesses
    /// to the 0x4200_0000 alias region must keep translating to single-bit
    /// operations on the underlying 0x4000_0000 peripheral registers.
    #[test]
    fn from_config_m3_bit_band_alias_still_translates() {
        let chip = bit_band_test_chip(
            "name: \"m3-test\"\narch: \"arm\"\ncore: \"cortex-m3\"",
            "0x40011000",
            "stm32f1",
        );
        let mut bus = SystemBus::from_config(&chip, &empty_manifest()).unwrap();

        // Alias word for GPIOC_ODR (0x4001100C) bit 0:
        // 0x42000000 + (0x1100C * 32) + (0 * 4) = 0x42220180.
        // Trait path (`crate::Bus`) — the CPU's access path with bit-band.
        crate::Bus::write_u32(&mut bus, 0x4222_0180, 1)
            .expect("bit-band alias write must translate on M3");
        let odr = crate::Bus::read_u32(&bus, 0x4001_100C).unwrap();
        assert_eq!(odr & 1, 1, "bit-band alias write must set ODR bit 0");
        assert_eq!(
            crate::Bus::read_u32(&bus, 0x4222_0180).unwrap(),
            1,
            "bit-band alias read must return the physical bit"
        );
    }

    /// Bit-band gating matrix: only M3/M4 cores have the feature. Absent
    /// core info on an Arm chip preserves the historical default (enabled)
    /// for configs that predate the `core` field.
    #[test]
    fn from_config_bit_band_gated_on_core() {
        let manifest = empty_manifest();
        let cases: &[(&str, bool)] = &[
            ("core: \"cortex-m3\"", true),
            ("core: \"cortex-m4\"", true),
            ("core: \"cortex-m0+\"", false),
            ("core: \"cortex-m7\"", false),
            ("core: \"cortex-m23\"", false),
            ("core: \"cortex-m33\"", false),
            ("", true), // absent core on Arm: historical default
        ];
        for (core_line, expected) in cases {
            let header = format!("name: \"gate-test\"\narch: \"arm\"\n{core_line}");
            let chip = bit_band_test_chip(&header, "0x40011000", "stm32f1");
            let bus = SystemBus::from_config(&chip, &manifest).unwrap();
            assert_eq!(
                bus.bit_band_enabled, *expected,
                "bit_band_enabled mismatch for chip header {header:?}"
            );
        }
    }

    fn chip_with_i2c_and_uart() -> labwired_config::ChipDescriptor {
        use labwired_config::{Arch, MemoryRange, PeripheralConfig};
        use std::collections::HashMap;

        labwired_config::ChipDescriptor {
            schema_version: "1.0".to_string(),
            name: "stm32f103-test".to_string(),
            arch: Arch::Arm,
            core: None,
            flash: MemoryRange {
                base: 0x0800_0000,
                size: "64KB".to_string(),
            },
            ram: MemoryRange {
                base: 0x2000_0000,
                size: "20KB".to_string(),
            },
            peripherals: vec![
                PeripheralConfig {
                    id: "i2c1".to_string(),
                    r#type: "i2c".to_string(),
                    base_address: 0x4000_5400,
                    size: Some("1KB".to_string()),
                    irq: Some(31),
                    config: HashMap::new(),
                },
                PeripheralConfig {
                    id: "uart1".to_string(),
                    r#type: "uart".to_string(),
                    base_address: 0x4000_3800,
                    size: Some("1KB".to_string()),
                    irq: Some(37),
                    config: HashMap::new(),
                },
            ],
        }
    }

    fn manifest_with_external_device(
        r#type: &str,
        connection: &str,
        config: std::collections::HashMap<String, serde_yaml::Value>,
    ) -> labwired_config::SystemManifest {
        labwired_config::SystemManifest {
            walk_deleted: false,
            schema_version: "1.0".to_string(),
            name: "adxl345-test".to_string(),
            chip: "../chips/stm32f103.yaml".to_string(),
            memory_overrides: std::collections::HashMap::new(),
            external_devices: vec![labwired_config::ExternalDevice {
                id: "sensor1".to_string(),
                r#type: r#type.to_string(),
                connection: connection.to_string(),
                config,
            }],
            board_io: Vec::new(),
            peripherals: Vec::new(),
        }
    }

    fn assert_external_device_error_contains_context(
        err: anyhow::Error,
        ext_type: &str,
        connection: &str,
    ) {
        let message = err.to_string();
        assert!(
            message.contains("sensor1"),
            "error missing external device id: {message}"
        );
        assert!(
            message.contains(ext_type),
            "error missing external device type: {message}"
        );
        assert!(
            message.contains(connection),
            "error missing external device connection: {message}"
        );
    }

    fn expect_from_config_error(
        chip: &labwired_config::ChipDescriptor,
        manifest: &labwired_config::SystemManifest,
    ) -> anyhow::Error {
        match SystemBus::from_config(chip, manifest) {
            Ok(_) => panic!("expected SystemBus::from_config to reject manifest"),
            Err(err) => err,
        }
    }

    #[test]
    fn test_from_config_errors_for_missing_external_device_connection() {
        let chip = chip_with_i2c_and_uart();
        let manifest = manifest_with_external_device(
            "adxl345",
            "missing-i2c",
            std::collections::HashMap::new(),
        );

        let err = expect_from_config_error(&chip, &manifest);

        assert_external_device_error_contains_context(err, "adxl345", "missing-i2c");
    }

    #[test]
    fn test_from_config_errors_for_external_device_on_non_i2c_connection() {
        let chip = chip_with_i2c_and_uart();
        let manifest =
            manifest_with_external_device("adxl345", "uart1", std::collections::HashMap::new());

        let err = expect_from_config_error(&chip, &manifest);

        assert_external_device_error_contains_context(err, "adxl345", "uart1");
    }

    #[test]
    fn test_from_config_skips_unsupported_external_device_type() {
        let chip = chip_with_i2c_and_uart();
        let mut config = std::collections::HashMap::new();
        config.insert(
            "i2c_address".to_string(),
            serde_yaml::Value::Number(0x48.into()),
        );
        // Use a clearly-fictional device type — tmp102/adxl345/etc. are all
        // real components now, so we need something the factory will refuse.
        let manifest = manifest_with_external_device("definitely_not_a_device", "i2c1", config);

        let mut bus = SystemBus::from_config(&chip, &manifest).unwrap();
        let i2c_idx = bus.find_peripheral_index_by_name("i2c1").unwrap();
        let any = bus.peripherals[i2c_idx].dev.as_any_mut().unwrap();
        let i2c = any.downcast_mut::<crate::peripherals::i2c::I2c>().unwrap();

        assert_eq!(i2c.attached_devices().len(), 0);
    }

    #[test]
    fn test_from_config_errors_for_invalid_external_device_i2c_address() {
        for value in [
            serde_yaml::Value::String("0x53".to_string()),
            serde_yaml::Value::Number(0x80.into()),
        ] {
            let chip = chip_with_i2c_and_uart();
            let mut config = std::collections::HashMap::new();
            config.insert("i2c_address".to_string(), value);
            let manifest = manifest_with_external_device("adxl345", "i2c1", config);

            let err = expect_from_config_error(&chip, &manifest);

            assert_external_device_error_contains_context(err, "adxl345", "i2c1");
        }
    }

    #[test]
    fn test_system_bus_memory_observer() {
        use std::sync::Arc;
        use std::sync::Mutex;

        #[derive(Debug)]
        struct MockObserver {
            writes: Arc<Mutex<Vec<(u64, u8, u8)>>>,
        }

        impl crate::SimulationObserver for MockObserver {
            fn on_step_end(&self, _cycles: u32, _registers: &[u32]) {}
            fn on_memory_write(&self, addr: u64, old: u8, new: u8) {
                self.writes.lock().unwrap().push((addr, old, new));
            }
        }

        let writes = Arc::new(Mutex::new(Vec::new()));
        let mut bus = SystemBus::new();
        bus.observers.push(Arc::new(MockObserver {
            writes: writes.clone(),
        }));

        // Write to RAM (e.g., 0x20000000)
        bus.write_u8(0x20000000, 0xAA).unwrap();
        {
            let w = writes.lock().unwrap();
            assert_eq!(w.len(), 1);
            assert_eq!(w[0], (0x20000000, 0, 0xAA));
        }

        // Write to Peripheral (e.g., UART at 0x4000C000)
        bus.write_u8(0x4000C000, 0xBB).unwrap();
        {
            let w = writes.lock().unwrap();
            assert_eq!(w.len(), 2);
            assert_eq!(w[1], (0x4000C000, 0xC0, 0xBB));
        }
    }

    #[test]
    fn test_flash_boot_alias_read_and_write() {
        let mut bus = SystemBus {
            flash: LinearMemory::new(256, 0x0800_0000),
            ram: LinearMemory::new(256, 0x2000_0000),
            peripherals: Vec::new(),
            nvic: None,
            observers: Vec::new(),
            config: crate::SimulationConfig::default(),
            bit_band_enabled: true,
            pending_cpu_irqs: [0; 2],
            dport_idx: None,
            flash_thunks: std::collections::HashMap::new(),
            peripheral_ranges: Vec::new(),
            peripheral_hint: Cell::new(None),
            last_gpio_in: [0; 2],
            current_cycle: 0,
            pending_schedule: Vec::new(),
            legacy_walk_disabled: false,
            hcsr04: Vec::new(),
        };

        bus.flash.write_u8(0x0800_0000, 0x12);
        bus.flash.write_u8(0x0800_0001, 0x34);

        // Read through aliased 0x0000_0000 boot window.
        assert_eq!(bus.read_u8(0x0000_0000).unwrap(), 0x12);
        assert_eq!(bus.read_u8(0x0000_0001).unwrap(), 0x34);

        // Write through alias and verify backing flash changed.
        bus.write_u8(0x0000_0001, 0xAB).unwrap();
        assert_eq!(bus.flash.read_u8(0x0800_0001), Some(0xAB));
    }

    #[test]
    fn test_peripheral_range_index_lookup() {
        let mut bus = SystemBus {
            flash: LinearMemory::new(256, 0x0800_0000),
            ram: LinearMemory::new(256, 0x2000_0000),
            peripherals: vec![
                PeripheralEntry {
                    name: "high".to_string(),
                    base: 0x5000_0000,
                    size: 0x1000,
                    irq: None,
                    dev: Box::new(crate::peripherals::uart::Uart::new()),
                    ticks_remaining: 0,
                    generation: 0,
                },
                PeripheralEntry {
                    name: "low".to_string(),
                    base: 0x4000_0000,
                    size: 0x1000,
                    irq: None,
                    dev: Box::new(crate::peripherals::uart::Uart::new()),
                    ticks_remaining: 0,
                    generation: 0,
                },
            ],
            nvic: None,
            observers: Vec::new(),
            config: crate::SimulationConfig::default(),
            bit_band_enabled: true,
            pending_cpu_irqs: [0; 2],
            dport_idx: None,
            flash_thunks: std::collections::HashMap::new(),
            peripheral_ranges: Vec::new(),
            peripheral_hint: Cell::new(None),
            last_gpio_in: [0; 2],
            current_cycle: 0,
            pending_schedule: Vec::new(),
            legacy_walk_disabled: false,
            hcsr04: Vec::new(),
        };

        bus.rebuild_peripheral_ranges();
        let low_idx = bus.find_peripheral_index(0x4000_0004);
        let high_idx = bus.find_peripheral_index(0x5000_0004);

        assert_eq!(low_idx, Some(1));
        assert_eq!(high_idx, Some(0));
    }

    #[test]
    fn test_execute_dma_copy_request() {
        let mut bus = SystemBus::new();
        bus.write_u8(0x2000_0010, 0xAB).unwrap();
        bus.write_u8(0x2000_0020, 0x00).unwrap();

        let req = crate::DmaRequest {
            src_addr: 0x2000_0010,
            addr: 0x2000_0020,
            val: 0,
            direction: crate::DmaDirection::Copy,
        };
        bus.execute_dma(&[req]).unwrap();

        assert_eq!(bus.read_u8(0x2000_0020).unwrap(), 0xAB);
    }

    #[test]
    fn test_dma_tick_executes_copy_and_raises_irq() {
        let mut bus = SystemBus {
            flash: LinearMemory::new(256, 0x0800_0000),
            ram: LinearMemory::new(256, 0x2000_0000),
            peripherals: vec![PeripheralEntry {
                name: "dma1".to_string(),
                base: 0x4002_0000,
                size: 0x400,
                irq: Some(16),
                dev: Box::new(crate::peripherals::dma::Dma1::new()),
                ticks_remaining: 0,
                generation: 0,
            }],
            nvic: None,
            observers: Vec::new(),
            config: crate::SimulationConfig::default(),
            bit_band_enabled: true,
            pending_cpu_irqs: [0; 2],
            dport_idx: None,
            flash_thunks: std::collections::HashMap::new(),
            peripheral_ranges: Vec::new(),
            peripheral_hint: Cell::new(None),
            last_gpio_in: [0; 2],
            current_cycle: 0,
            pending_schedule: Vec::new(),
            legacy_walk_disabled: false,
            hcsr04: Vec::new(),
        };
        bus.rebuild_peripheral_ranges();

        // Per STM32 RM mem-to-mem semantics: data flows CMAR -> CPAR
        // (CMAR is the source, CPAR is the destination). Set up source
        // at SRC_ADDR via CMAR; expect destination at DST_ADDR (CPAR).
        const SRC_ADDR: u64 = 0x2000_0010;
        const DST_ADDR: u64 = 0x2000_0020;
        bus.write_u8(SRC_ADDR, 0x5A).unwrap();
        bus.write_u8(DST_ADDR, 0x00).unwrap();

        // Program DMA1 Channel1:
        //   CMAR (source) = SRC_ADDR
        //   CPAR (destination) = DST_ADDR
        //   CNDTR = 1, CCR = EN | TCIE | PINC | MINC | DIR | MEM2MEM
        bus.write_u32(0x4002_0014, SRC_ADDR as u32).unwrap(); // CMAR1
        bus.write_u32(0x4002_0010, DST_ADDR as u32).unwrap(); // CPAR1
        bus.write_u32(0x4002_000C, 1).unwrap(); // CNDTR1
        bus.write_u32(
            0x4002_0008,
            (1 << 0) | (1 << 1) | (1 << 4) | (1 << 6) | (1 << 7) | (1 << 14),
        )
        .unwrap(); // CCR1 (EN | TCIE | DIR | PINC | MINC | MEM2MEM)

        let (interrupts, _costs) = bus.tick_peripherals_fully();
        assert_eq!(
            bus.read_u8(DST_ADDR).unwrap(),
            0x5A,
            "DST should hold the SRC byte after mem-to-mem copy"
        );
        assert!(interrupts.contains(&16), "TCIE should pend NVIC IRQ 16");
    }
}
