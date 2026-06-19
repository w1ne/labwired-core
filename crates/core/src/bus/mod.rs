// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

use crate::memory::LinearMemory;
use crate::peripherals::nvic::NvicState;
use crate::peripherals::uart::Uart;
use crate::{Bus, Peripheral, SimResult};
use labwired_config::{parse_size, ChipDescriptor, PeripheralConfig, SystemManifest};
use std::cell::Cell;
use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::sync::Mutex;

mod accessors;
mod from_config;
mod routing;
mod tick;

impl SystemBus {}

/// A peripheral's RCC clock-gate, resolved to a concrete RCC register offset +
/// bit at bus-build time (the symbolic `reg` name from the yaml is mapped to the
/// active chip family's offset via [`Rcc::enable_reg_offset`]). When present, a
/// CPU access to the owning peripheral only takes effect while `bit` is set in
/// the RCC enable register at `reg_offset` — modelling silicon clock-gating.
#[derive(Debug, Clone, Copy)]
pub struct ResolvedClockGate {
    /// Byte offset of the RCC enable register within the rcc peripheral.
    pub reg_offset: u64,
    /// Enable-bit position within that register.
    pub bit: u8,
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
    /// Optional RCC clock-gate (silicon clock-gating model). `None` (the common
    /// case) → the peripheral is never gated and accesses always pass through.
    /// `Some` → accesses are dropped (writes ignored, reads return 0) while the
    /// gate bit is clear in the RCC, exactly like an unclocked peripheral on
    /// real silicon. Resolved from `PeripheralConfig::clock` in `from_config`.
    pub clock_gate: Option<ResolvedClockGate>,
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
    pub flash_thunks: std::collections::HashMap<
        u32,
        crate::peripherals::esp_xtensa_common::rom_thunks::RomThunkFn,
    >,
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
    /// Cached index of the "rcc" peripheral, if one is registered. Recomputed in
    /// `rebuild_peripheral_ranges` (same staleness contract as `dport_idx`). Lets
    /// the clock-gate check on the hot read/write path resolve the RCC peripheral
    /// in O(1) instead of scanning by name. `None` on buses with no RCC (e.g.
    /// most non-STM32 chips), in which case no peripheral is ever gated.
    rcc_idx: Option<usize>,
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
    /// Reusable CAN diagnostic clients declared as external devices. They
    /// inject configured CAN frames into a named FDCAN peripheral once it is
    /// running, so ECU examples can be driven by a virtual off-board tester
    /// instead of self-loopback firmware.
    pub can_diagnostic_testers: Vec<CanDiagnosticTester>,
    /// Stateful ISO-TP/UDS testers declared as external devices. Each is a real
    /// second CAN node driving a multi-frame SecurityAccess exchange against a
    /// named CAN peripheral (bxCAN or FDCAN) running in normal mode. Empty by
    /// default → zero per-tick cost.
    pub can_uds_testers: Vec<CanUdsTester>,
    /// ESP32-C3 (RISC-V) interrupt routing: when true, each tick the bus routes
    /// asserted peripheral sources and the SYSTEM FROM_CPU IPI registers
    /// (0x600C0028..0x34) through the INTERRUPT_CORE0 matrix MAP registers into
    /// `riscv_irq_lines`. Set by the C3 rom-boot setup; false everywhere else
    /// so no other architecture's bus is affected.
    pub esp32c3_irq_routing: bool,
    /// ESP32-C3 level-sensitive bitmask of asserted CPU interrupt lines (1..31),
    /// recomputed every tick by `aggregate_esp32c3_irqs`. Read by the RISC-V
    /// core via `Bus::external_irq_lines`. 0 when `esp32c3_irq_routing` is false.
    pub riscv_irq_lines: u32,
}

pub struct CanDiagnosticTester {
    pub id: String,
    pub connection: String,
    pub request_id: u32,
    pub request_data: Vec<u8>,
    pub sent: bool,
}

/// Stateful ISO-TP / UDS tester driving a *multi-frame* SecurityAccess exchange
/// against an emulated ECU's CAN controller running in **normal** mode (not
/// loopback). Unlike [`CanDiagnosticTester`] (a one-shot single-frame injector),
/// this is a real second CAN node: it injects a FirstFrame, waits for the ECU's
/// FlowControl, injects the ConsecutiveFrame, then waits for the ECU's
/// SecurityAccess positive response — exactly the handshake a physical UDS
/// tester would perform over ISO 15765-2.
///
/// The ECU side is driven entirely through the peripheral's *public* API: we
/// drain its `tx_frames` (frames it transmitted in normal mode) and inject our
/// frames via `deliver_rx` (bxCAN) / `receive_frame` (FDCAN). Injection is
/// filter-gated, so a `false` return (filter not yet configured, FIFO full)
/// leaves the FSM parked on the same send to retry next tick.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CanUdsTesterState {
    /// Need to inject the FirstFrame.
    Start,
    /// FirstFrame sent; waiting for the ECU's FlowControl frame.
    AwaitFc,
    /// ConsecutiveFrame sent; waiting for the ECU's positive response.
    AwaitResp,
    /// SecurityAccess positive response observed — handshake complete.
    Done,
    /// Timed out before completion (broken / silent ECU).
    Failed,
}

pub struct CanUdsTester {
    pub id: String,
    /// Name of the connected CAN peripheral (e.g. `bxcan1` / `fdcan1`).
    pub connection: String,
    /// Tester → ECU request id (ISO-TP single physical address). Default 0x111.
    pub request_id: u32,
    /// ECU → tester response id. Default 0x222.
    pub reply_id: u32,
    /// ISO-TP FirstFrame payload injected in state `Start`.
    pub first_frame: Vec<u8>,
    /// ISO-TP ConsecutiveFrame payload injected on FlowControl.
    pub consecutive_frame: Vec<u8>,
    /// Current FSM state. Exposed for tests.
    pub state: CanUdsTesterState,
    /// Ticks elapsed since the tester started; used for the give-up timeout.
    pub ticks: u64,
    /// Tick budget before declaring `Failed`.
    pub max_ticks: u64,
}

impl CanUdsTester {
    /// Default tester ↔ ECU ids and ISO-TP payloads for the SecurityAccess
    /// SeedRequest exchange the firmware contract expects.
    pub const DEFAULT_REQUEST_ID: u32 = 0x111;
    pub const DEFAULT_REPLY_ID: u32 = 0x222;
    pub const DEFAULT_FIRST_FRAME: [u8; 8] = [0x10, 0x0B, 0x27, 0x01, 0x5A, 0x11, 0x22, 0x33];
    pub const DEFAULT_CONSECUTIVE_FRAME: [u8; 8] =
        [0x21, 0x44, 0x55, 0x66, 0x77, 0x88, 0x55, 0x55];
    const DEFAULT_MAX_TICKS: u64 = 200_000;

    pub fn new(id: String, connection: String) -> Self {
        Self {
            id,
            connection,
            request_id: Self::DEFAULT_REQUEST_ID,
            reply_id: Self::DEFAULT_REPLY_ID,
            first_frame: Self::DEFAULT_FIRST_FRAME.to_vec(),
            consecutive_frame: Self::DEFAULT_CONSECUTIVE_FRAME.to_vec(),
            state: CanUdsTesterState::Start,
            ticks: 0,
            max_ticks: Self::DEFAULT_MAX_TICKS,
        }
    }

    fn is_terminal(&self) -> bool {
        matches!(
            self.state,
            CanUdsTesterState::Done | CanUdsTesterState::Failed
        )
    }

    /// Observe one frame the ECU transmitted. In `AwaitFc` an ISO-TP
    /// FlowControl (`(data[0] & 0xF0) == 0x30`) on `reply_id` clears the wait;
    /// in `AwaitResp` a SecurityAccess single-frame positive response
    /// (`data[0] == 0x06 && data[1] == 0x67`) on `reply_id` completes the
    /// handshake. Returns the payload to inject next (if the observation
    /// unblocks a send), else `None`.
    fn observe_ecu_frame(&mut self, id: u32, data: &[u8]) -> Option<Vec<u8>> {
        if id != self.reply_id {
            return None;
        }
        match self.state {
            CanUdsTesterState::AwaitFc => {
                if data.first().map(|b| b & 0xF0) == Some(0x30) {
                    // FlowControl seen → time to send the ConsecutiveFrame.
                    return Some(self.consecutive_frame.clone());
                }
                None
            }
            CanUdsTesterState::AwaitResp => {
                if data.first() == Some(&0x06) && data.get(1) == Some(&0x67) {
                    self.state = CanUdsTesterState::Done;
                }
                None
            }
            _ => None,
        }
    }
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
            | "usb_otg" | "bxcan" | "fdcan" | "sdmmc" | "comp" | "tsc" | "fmc" => {
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

    pub(crate) fn profile_name(p_cfg: &PeripheralConfig) -> anyhow::Result<Option<&str>> {
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

    pub(crate) fn parse_profile_or_default<T>(
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

    /// True when the wired devices need cycle-accurate (non-batched) execution
    /// to behave correctly. Some external devices are driven from `tick_peripherals`
    /// and observed by cycle-tight firmware loops — e.g. the HC-SR04 holds ECHO
    /// high for a pulse the firmware times by polling GPIO IN in a busy loop.
    /// Batched execution advances many instructions before ticking peripherals,
    /// so the firmware polls a frozen ECHO and measures nothing. Runners should
    /// disable instruction batching when this returns true (correctness > speed).
    /// New per-tick GPIO-timing devices should extend this predicate.
    pub fn requires_cycle_accurate(&self) -> bool {
        !self.hcsr04.is_empty()
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

    pub(crate) fn service_can_diagnostic_testers(&mut self) {
        if self.can_diagnostic_testers.is_empty() {
            return;
        }

        for i in 0..self.can_diagnostic_testers.len() {
            if self.can_diagnostic_testers[i].sent {
                continue;
            }

            let connection = self.can_diagnostic_testers[i].connection.clone();
            let Some(idx) = self.find_peripheral_index_by_name(&connection) else {
                continue;
            };
            let Some(fdcan) = self.peripherals[idx]
                .dev
                .as_any_mut()
                .and_then(|any| any.downcast_mut::<crate::peripherals::fdcan::Fdcan>())
            else {
                continue;
            };

            let frame = crate::network::CanFrame {
                id: self.can_diagnostic_testers[i].request_id,
                data: self.can_diagnostic_testers[i].request_data.clone(),
                extended: false,
                fd: self.can_diagnostic_testers[i].request_data.len() > 8,
                bitrate_switch: self.can_diagnostic_testers[i].request_data.len() > 8,
                remote: false,
            };
            if fdcan.receive_frame(frame) {
                self.can_diagnostic_testers[i].sent = true;
            }
        }
    }

    /// Per-tick service for the stateful ISO-TP/UDS testers. For each tester:
    /// resolve its peripheral by name, drain the ECU's outbound `tx_frames`,
    /// advance the FSM, and inject the next ISO-TP frame (filter-gated) when due.
    ///
    /// Works against both bxCAN (`deliver_rx`) and FDCAN (`receive_frame`); the
    /// downcast picks whichever is wired. A filtered/dropped injection (return
    /// `false`) leaves the FSM parked on the same send so it retries next tick —
    /// important on the first ticks before the ECU has configured its filter.
    pub(crate) fn service_can_uds_testers(&mut self) {
        if self.can_uds_testers.is_empty() {
            return;
        }

        for i in 0..self.can_uds_testers.len() {
            if self.can_uds_testers[i].is_terminal() {
                continue;
            }

            // Timeout guard so a broken/silent ECU never hangs the sim.
            self.can_uds_testers[i].ticks += 1;
            if self.can_uds_testers[i].ticks > self.can_uds_testers[i].max_ticks {
                self.can_uds_testers[i].state = CanUdsTesterState::Failed;
                continue;
            }

            let connection = self.can_uds_testers[i].connection.clone();
            let Some(idx) = self.find_peripheral_index_by_name(&connection) else {
                continue;
            };

            // Drain the ECU's outbound frames and feed the FSM. `observe_ecu_frame`
            // may return a payload to inject (e.g. the CF unblocked by FlowControl);
            // the actual injection happens below so both peripheral kinds share one
            // filter-gated send path.
            let request_id = self.can_uds_testers[i].request_id;
            let mut pending_inject: Option<Vec<u8>> = None;

            // Resolve the peripheral once; reborrow per phase to satisfy the
            // borrow checker (drain, then inject).
            let drained: Vec<crate::network::CanFrame> = {
                let any = self.peripherals[idx].dev.as_any_mut();
                match any {
                    Some(a) => {
                        if let Some(bx) =
                            a.downcast_mut::<crate::peripherals::bxcan::BxCan>()
                        {
                            bx.tx_frames.drain(..).collect()
                        } else if let Some(fd) =
                            a.downcast_mut::<crate::peripherals::fdcan::Fdcan>()
                        {
                            fd.tx_frames.drain(..).collect()
                        } else {
                            continue;
                        }
                    }
                    None => continue,
                }
            };

            for frame in &drained {
                if let Some(payload) = self.can_uds_testers[i].observe_ecu_frame(frame.id, &frame.data)
                {
                    pending_inject = Some(payload);
                }
            }

            // Decide what (if anything) to inject this tick.
            let to_send: Option<Vec<u8>> = match self.can_uds_testers[i].state {
                CanUdsTesterState::Start => {
                    Some(self.can_uds_testers[i].first_frame.clone())
                }
                CanUdsTesterState::AwaitFc => pending_inject,
                _ => None,
            };

            let Some(payload) = to_send else {
                continue;
            };

            let frame = crate::network::CanFrame::classic(request_id, payload);
            let injected = {
                let any = self.peripherals[idx].dev.as_any_mut();
                match any {
                    Some(a) => {
                        if let Some(bx) =
                            a.downcast_mut::<crate::peripherals::bxcan::BxCan>()
                        {
                            bx.deliver_rx(frame)
                        } else if let Some(fd) =
                            a.downcast_mut::<crate::peripherals::fdcan::Fdcan>()
                        {
                            fd.receive_frame(frame)
                        } else {
                            false
                        }
                    }
                    None => false,
                }
            };

            if injected {
                // Advance only on a successful (accepted) injection; otherwise
                // stay parked and retry next tick.
                match self.can_uds_testers[i].state {
                    CanUdsTesterState::Start => {
                        self.can_uds_testers[i].state = CanUdsTesterState::AwaitFc
                    }
                    CanUdsTesterState::AwaitFc => {
                        self.can_uds_testers[i].state = CanUdsTesterState::AwaitResp
                    }
                    _ => {}
                }
            }
        }
    }

    fn yaml_u32(value: Option<&serde_yaml::Value>, default: u32) -> u32 {
        match value {
            Some(serde_yaml::Value::Number(n)) => n.as_u64().map(|v| v as u32).unwrap_or(default),
            Some(serde_yaml::Value::String(s)) => {
                let s = s.trim();
                if let Some(hex) = s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")) {
                    u32::from_str_radix(&hex.replace('_', ""), 16).unwrap_or(default)
                } else {
                    s.replace('_', "").parse::<u32>().unwrap_or(default)
                }
            }
            _ => default,
        }
    }

    fn yaml_bytes(value: Option<&serde_yaml::Value>, default: &[u8]) -> Vec<u8> {
        match value {
            Some(serde_yaml::Value::Sequence(seq)) => seq
                .iter()
                .map(|value| Self::yaml_u32(Some(value), 0) as u8)
                .collect(),
            Some(serde_yaml::Value::String(s)) => s
                .split(|c: char| c.is_ascii_whitespace() || c == ',' || c == ':')
                .filter(|part| !part.is_empty())
                .map(|part| {
                    let part = part.trim();
                    if let Some(hex) = part.strip_prefix("0x").or_else(|| part.strip_prefix("0X")) {
                        u8::from_str_radix(hex, 16).unwrap_or(0)
                    } else {
                        u8::from_str_radix(part, 16)
                            .unwrap_or_else(|_| part.parse::<u8>().unwrap_or(0))
                    }
                })
                .collect(),
            _ => default.to_vec(),
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
        use crate::peripherals::esp32::spi::Esp32Spi;
        use crate::peripherals::spi::{Spi, SpiDevice};

        // Borrow the attached-device list off whichever SPI peripheral kind
        // this is (generic `Spi` for STM32/Nordic, `Esp32Spi` for ESP32).
        fn attached_ref(any: &dyn std::any::Any) -> Option<&Vec<Box<dyn SpiDevice>>> {
            if let Some(s) = any.downcast_ref::<Spi>() {
                return Some(&s.attached_devices);
            }
            if let Some(s) = any.downcast_ref::<Esp32Spi>() {
                return Some(&s.attached_devices);
            }
            None
        }
        fn attached_mut(any: &mut dyn std::any::Any) -> Option<&mut Vec<Box<dyn SpiDevice>>> {
            if any.is::<Spi>() {
                return any.downcast_mut::<Spi>().map(|s| &mut s.attached_devices);
            }
            if any.is::<Esp32Spi>() {
                return any
                    .downcast_mut::<Esp32Spi>()
                    .map(|s| &mut s.attached_devices);
            }
            None
        }

        // Phase 1: collect (attached_index, odr_addr, bit) — immutable borrow.
        let sources: Vec<(usize, u64, u8)> = {
            let Some(any) = self.peripherals[idx].dev.as_any() else {
                return;
            };
            let Some(devs) = attached_ref(any) else {
                return;
            };
            devs.iter()
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
            if let Some(devs) = attached_mut(any) {
                for (i, lvl) in levels {
                    if let Some(d) = devs.get_mut(i) {
                        d.set_dc_level(lvl);
                    }
                }
            }
        }
    }

    /// Whether peripheral `idx` is currently clocked. `true` (always-on) for any
    /// peripheral without a declared clock-gate — the safe default that keeps
    /// every existing config/firmware working. For a gated peripheral, reads the
    /// RCC enable register the gate points at and returns whether the gate bit is
    /// set. If no RCC peripheral is registered, or its register read fails, the
    /// peripheral is treated as clocked (fail-open: never wedge a chip that has
    /// no modelled RCC). Cheap: one `Option` check, then on the rare gated path a
    /// single cached-index RCC register read.
    fn is_peripheral_clocked(&self, idx: usize) -> bool {
        let Some(gate) = self
            .peripherals
            .get(idx)
            .and_then(|p| p.clock_gate.as_ref())
        else {
            return true; // ungated → always accessible
        };
        let Some(rcc_idx) = self.rcc_idx else {
            return true; // no RCC modelled → don't gate
        };
        match self.peripherals[rcc_idx].dev.read_u32(gate.reg_offset) {
            Ok(reg) => (reg >> gate.bit) & 1 != 0,
            Err(_) => true,
        }
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
                    clock_gate: None,
                },
                PeripheralEntry {
                    name: "gpioa".to_string(),
                    base: 0x4001_0800,
                    size: 0x400,
                    irq: None,
                    dev: Box::new(crate::peripherals::gpio::GpioPort::new()),
                    ticks_remaining: 0,
                    generation: 0,
                    clock_gate: None,
                },
                PeripheralEntry {
                    name: "rcc".to_string(),
                    base: 0x4002_1000,
                    size: 0x400,
                    irq: None,
                    dev: Box::new(crate::peripherals::rcc::Rcc::new()),
                    ticks_remaining: 0,
                    generation: 0,
                    clock_gate: None,
                },
                PeripheralEntry {
                    name: "systick".to_string(),
                    base: 0xE000_E010,
                    size: 0x100,
                    irq: Some(15),
                    dev: Box::new(crate::peripherals::systick::Systick::new()),
                    ticks_remaining: 0,
                    generation: 0,
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
            peripheral_ranges: Vec::new(),
            peripheral_hint: Cell::new(None),
            last_gpio_in: [0; 2],
            current_cycle: 0,
            pending_schedule: Vec::new(),
            legacy_walk_disabled: false,
            hcsr04: Vec::new(),
            can_diagnostic_testers: Vec::new(),
            can_uds_testers: Vec::new(),
            esp32c3_irq_routing: false,
            riscv_irq_lines: 0,
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
            peripheral_ranges: Vec::new(),
            peripheral_hint: Cell::new(None),
            last_gpio_in: [0; 2],
            current_cycle: 0,
            pending_schedule: Vec::new(),
            legacy_walk_disabled: false,
            hcsr04: Vec::new(),
            can_diagnostic_testers: Vec::new(),
            can_uds_testers: Vec::new(),
            esp32c3_irq_routing: false,
            riscv_irq_lines: 0,
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
            clock_gate: None,
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
        use crate::peripherals::esp32::uart::Esp32Uart;
        for p in &mut self.peripherals {
            let Some(any) = p.dev.as_any_mut() else {
                continue;
            };
            // STM32-layout generic UART.
            if let Some(uart) = any.downcast_mut::<Uart>() {
                uart.set_sink(Some(sink.clone()), echo_stdout);
                continue;
            }
            // Real ESP32-classic UART (echo is fixed at construction time).
            if let Some(uart) = any.downcast_mut::<Esp32Uart>() {
                uart.set_sink(Some(sink.clone()));
            }
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

    /// Place a built peripheral on the bus using the descriptor's window size
    /// (default 4KB) and IRQ. Shared by the per-family factory dispatch and the
    /// generic-match path in [`Self::from_config`] so both stay in lockstep.
    fn push_peripheral(
        &mut self,
        p_cfg: &labwired_config::PeripheralConfig,
        dev: Box<dyn Peripheral>,
    ) -> anyhow::Result<()> {
        let size = match &p_cfg.size {
            Some(size) => parse_size(size)?,
            None => 0x1000,
        };
        self.peripherals.push(PeripheralEntry {
            name: p_cfg.id.clone(),
            base: p_cfg.base_address,
            size,
            irq: p_cfg.irq,
            dev,
            ticks_remaining: 0,
            generation: 0,
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
    fn resolve_clock_gates(
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
            if !self.is_peripheral_clocked(idx) {
                return Ok(0); // unclocked peripheral reads 0 (silicon gating)
            }
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
            if !self.is_peripheral_clocked(idx) {
                return Ok(()); // unclocked peripheral: write dropped (gating)
            }
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
            if !self.is_peripheral_clocked(idx) {
                return Ok(0); // unclocked peripheral reads 0 (silicon gating)
            }
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
            if !self.is_peripheral_clocked(idx) {
                return Ok(()); // unclocked peripheral: write dropped (gating)
            }
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
            memory_regions: Vec::new(),
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
                clock: None,
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

    #[test]
    fn test_from_config_can_diagnostic_tester_injects_frame_into_fdcan() {
        let chip: ChipDescriptor = serde_yaml::from_str(
            r#"
name: "h563-test"
arch: "arm"
core: "cortex-m33"
flash:
  base: 0x08000000
  size: "128KB"
ram:
  base: 0x20000000
  size: "64KB"
peripherals:
  - id: "fdcan1"
    type: "fdcan"
    base_address: 0x4000A400
    size: "4KB"
"#,
        )
        .unwrap();
        let manifest: SystemManifest = serde_yaml::from_str(
            r#"
name: "uds-tester"
chip: "unused"
external_devices:
  - id: "uds_tester"
    type: "can-diagnostic-tester"
    connection: "fdcan1"
    config:
      request_id: "0x7E0"
      request_data: "03 22 F1 90"
board_io: []
"#,
        )
        .unwrap();
        let mut bus = SystemBus::from_config(&chip, &manifest).unwrap();
        assert_eq!(bus.can_diagnostic_testers.len(), 1);

        // Still in INIT: tester retries but cannot inject into a stopped FDCAN.
        bus.tick_peripherals_fully();
        {
            let idx = bus.find_peripheral_index_by_name("fdcan1").unwrap();
            let fdcan = bus.peripherals[idx]
                .dev
                .as_any()
                .unwrap()
                .downcast_ref::<crate::peripherals::fdcan::Fdcan>()
                .unwrap();
            assert!(fdcan.trace_snapshot("fdcan1").is_empty());
        }

        // Leave INIT; next bus tick lets the reusable tester drive the CAN frame.
        bus.write_u32(0x4000_A400 + 0x018, 0).unwrap();
        bus.tick_peripherals_fully();
        let idx = bus.find_peripheral_index_by_name("fdcan1").unwrap();
        let fdcan = bus.peripherals[idx]
            .dev
            .as_any()
            .unwrap()
            .downcast_ref::<crate::peripherals::fdcan::Fdcan>()
            .unwrap();
        let trace = fdcan.trace_snapshot("fdcan1");
        assert_eq!(trace.len(), 1);
        assert_eq!(trace[0].direction, "rx");
        assert_eq!(trace[0].id, 0x7E0);
        assert_eq!(trace[0].data, vec![0x03, 0x22, 0xF1, 0x90]);
        assert!(bus.can_diagnostic_testers[0].sent);
    }

    /// Pure FSM walk: FirstFrame → (ECU FlowControl) → ConsecutiveFrame →
    /// (ECU positive response) → Done, driving the tester's state machine by
    /// feeding ECU frames manually (no peripheral, no bus tick). This exercises
    /// the exact observe/advance logic `service_can_uds_testers` reuses.
    #[test]
    fn uds_tester_fsm_drives_ff_fc_cf_response() {
        let mut t = CanUdsTester::new("t".into(), "bxcan1".into());
        assert_eq!(t.state, CanUdsTesterState::Start);
        assert_eq!(t.request_id, 0x111);
        assert_eq!(t.reply_id, 0x222);

        // Start: the next frame to inject is the FirstFrame; on a (simulated)
        // accepted inject the FSM advances to AwaitFc.
        assert_eq!(t.first_frame, CanUdsTester::DEFAULT_FIRST_FRAME.to_vec());
        t.state = CanUdsTesterState::AwaitFc;

        // A non-FlowControl frame, or one on the wrong id, does not unblock.
        assert!(t.observe_ecu_frame(0x999, &[0x30, 0x00, 0x00]).is_none());
        assert!(t.observe_ecu_frame(0x222, &[0x06, 0x67]).is_none());
        assert_eq!(t.state, CanUdsTesterState::AwaitFc);

        // ECU FlowControl (0x30..) on reply_id → returns the ConsecutiveFrame.
        let cf = t
            .observe_ecu_frame(0x222, &[0x30, 0x00, 0x00, 0, 0, 0, 0, 0])
            .expect("FlowControl unblocks the ConsecutiveFrame");
        assert_eq!(cf, CanUdsTester::DEFAULT_CONSECUTIVE_FRAME.to_vec());

        // Simulate the accepted CF inject.
        t.state = CanUdsTesterState::AwaitResp;

        // A wrong response (negative / different service) does not complete.
        assert!(t.observe_ecu_frame(0x222, &[0x03, 0x7F, 0x27]).is_none());
        assert_eq!(t.state, CanUdsTesterState::AwaitResp);

        // SecurityAccess positive single-frame response → Done.
        assert!(t
            .observe_ecu_frame(0x222, &[0x06, 0x67, 0x01, 0xAA, 0xBB, 0xCC, 0xDD, 0xEE])
            .is_none());
        assert_eq!(t.state, CanUdsTesterState::Done);
        assert!(t.is_terminal());
    }

    /// End-to-end against a real `BxCan` registered on the bus and configured
    /// (valid BTR + accept-0x111 filter, NORMAL mode — no loopback) so
    /// `deliver_rx` accepts the tester's frames. We drive the full bus tick:
    /// FF → (ECU emits FlowControl) → CF → (ECU emits positive response) → Done.
    /// The ECU's "transmit" side is modeled by pushing frames into the bxCAN's
    /// public `tx_frames`, which the tester drains exactly as it would for a
    /// firmware-driven controller in normal mode.
    #[test]
    fn uds_tester_completes_against_real_bxcan() {
        use crate::peripherals::bxcan::BxCan;

        // bxCAN register offsets (RM0008 §24.9) addressed via the bus.
        const MCR: u64 = 0x000;
        const BTR: u64 = 0x01C;
        const FMR: u64 = 0x200;
        const FM1R: u64 = 0x204;
        const FS1R: u64 = 0x20C;
        const FFA1R: u64 = 0x214;
        const FA1R: u64 = 0x21C;
        const FBANK: u64 = 0x240;
        const VALID_BTR: u32 = 0x00DC_0009; // valid TS1/TS2, no loopback bit.

        let base: u64 = 0x4000_6400;
        let mut bus = SystemBus::empty();
        bus.add_peripheral("bxcan1", base, 0x400, None, Box::new(BxCan::new()));

        // Bring the controller up in NORMAL mode and install a bank-0 mask
        // filter accepting exactly 0x111 into FIFO0.
        bus.write_u32(base + MCR, 1).unwrap(); // INRQ: request init
        bus.write_u32(base + BTR, VALID_BTR).unwrap(); // valid timing, NOT loopback
        bus.write_u32(base + FMR, 1).unwrap(); // FINIT: filter init
        bus.write_u32(base + FS1R, 0x1).unwrap(); // bank0 32-bit
        bus.write_u32(base + FM1R, 0x0).unwrap(); // bank0 mask mode
        bus.write_u32(base + FFA1R, 0x0).unwrap(); // bank0 -> FIFO0
        bus.write_u32(base + FBANK, (0x111u32) << 21).unwrap(); // F0R1
        bus.write_u32(base + FBANK + 4, (0x111u32) << 21).unwrap(); // F0R2 mask
        bus.write_u32(base + FA1R, 0x1).unwrap(); // bank0 active
        bus.write_u32(base + FMR, 0x0).unwrap(); // clear FINIT: filters live
        bus.write_u32(base + MCR, 0).unwrap(); // leave init -> running (normal)

        bus.can_uds_testers
            .push(CanUdsTester::new("uds".into(), "bxcan1".into()));

        // Tick 1: tester injects the FirstFrame (filter accepts) → AwaitFc.
        bus.service_can_uds_testers();
        assert_eq!(bus.can_uds_testers[0].state, CanUdsTesterState::AwaitFc);

        // The injected FF landed in the ECU's RX FIFO0 (filter-accepted).
        {
            let idx = bus.find_peripheral_index_by_name("bxcan1").unwrap();
            let bx = bus.peripherals[idx]
                .dev
                .as_any_mut()
                .unwrap()
                .downcast_mut::<BxCan>()
                .unwrap();
            // ECU "transmits" a FlowControl frame in normal mode (id = reply_id).
            bx.tx_frames.push_back(crate::network::CanFrame::classic(
                0x222,
                vec![0x30, 0x00, 0x00, 0, 0, 0, 0, 0],
            ));
        }

        // Tick 2: tester drains the FlowControl and injects the CF → AwaitResp.
        bus.service_can_uds_testers();
        assert_eq!(bus.can_uds_testers[0].state, CanUdsTesterState::AwaitResp);

        // ECU "transmits" the SecurityAccess positive single-frame response.
        {
            let idx = bus.find_peripheral_index_by_name("bxcan1").unwrap();
            let bx = bus.peripherals[idx]
                .dev
                .as_any_mut()
                .unwrap()
                .downcast_mut::<BxCan>()
                .unwrap();
            bx.tx_frames.push_back(crate::network::CanFrame::classic(
                0x222,
                vec![0x06, 0x67, 0x01, 0xAA, 0xBB, 0xCC, 0xDD, 0xEE],
            ));
        }

        // Tick 3: tester observes the positive response → Done.
        bus.service_can_uds_testers();
        assert_eq!(bus.can_uds_testers[0].state, CanUdsTesterState::Done);
    }

    /// Config parsing: a `uds-tester` external device populates a
    /// `CanUdsTester` with the configured ids and payloads.
    #[test]
    fn uds_tester_parsed_from_config() {
        let chip: ChipDescriptor = serde_yaml::from_str(
            r#"
name: "f103"
arch: "arm"
core: "cortex-m3"
flash:
  base: 0x08000000
  size: "128KB"
ram:
  base: 0x20000000
  size: "20KB"
peripherals:
  - id: "bxcan1"
    type: "bxcan"
    base_address: 0x40006400
    size: "1KB"
"#,
        )
        .unwrap();
        let manifest: SystemManifest = serde_yaml::from_str(
            r#"
name: "uds-multiframe"
chip: "f103"
external_devices:
  - id: "uds_node"
    type: "uds-tester"
    connection: "bxcan1"
    config:
      request_id: "0x111"
      reply_id: "0x222"
      first_frame: "10 0B 27 01 5A 11 22 33"
      consecutive_frame: "21 44 55 66 77 88 55 55"
board_io: []
"#,
        )
        .unwrap();
        let bus = SystemBus::from_config(&chip, &manifest).unwrap();
        assert_eq!(bus.can_uds_testers.len(), 1);
        let t = &bus.can_uds_testers[0];
        assert_eq!(t.request_id, 0x111);
        assert_eq!(t.reply_id, 0x222);
        assert_eq!(t.first_frame, CanUdsTester::DEFAULT_FIRST_FRAME.to_vec());
        assert_eq!(
            t.consecutive_frame,
            CanUdsTester::DEFAULT_CONSECUTIVE_FRAME.to_vec()
        );
        assert_eq!(t.state, CanUdsTesterState::Start);
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
            memory_regions: Vec::new(),
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
                    clock: None,
                    config: HashMap::new(),
                },
                PeripheralConfig {
                    id: "uart1".to_string(),
                    r#type: "uart".to_string(),
                    base_address: 0x4000_3800,
                    size: Some("1KB".to_string()),
                    irq: Some(37),
                    clock: None,
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
            extra_mem: Vec::new(),
            peripherals: Vec::new(),
            nvic: None,
            observers: Vec::new(),
            config: crate::SimulationConfig::default(),
            bit_band_enabled: true,
            pending_cpu_irqs: [0; 2],
            dport_idx: None,
            rcc_idx: None,
            flash_thunks: std::collections::HashMap::new(),
            peripheral_ranges: Vec::new(),
            peripheral_hint: Cell::new(None),
            last_gpio_in: [0; 2],
            current_cycle: 0,
            pending_schedule: Vec::new(),
            legacy_walk_disabled: false,
            hcsr04: Vec::new(),
            can_diagnostic_testers: Vec::new(),
            can_uds_testers: Vec::new(),
            esp32c3_irq_routing: false,
            riscv_irq_lines: 0,
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
            extra_mem: Vec::new(),
            peripherals: vec![
                PeripheralEntry {
                    name: "high".to_string(),
                    base: 0x5000_0000,
                    size: 0x1000,
                    irq: None,
                    dev: Box::new(crate::peripherals::uart::Uart::new()),
                    ticks_remaining: 0,
                    generation: 0,
                    clock_gate: None,
                },
                PeripheralEntry {
                    name: "low".to_string(),
                    base: 0x4000_0000,
                    size: 0x1000,
                    irq: None,
                    dev: Box::new(crate::peripherals::uart::Uart::new()),
                    ticks_remaining: 0,
                    generation: 0,
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
            flash_thunks: std::collections::HashMap::new(),
            peripheral_ranges: Vec::new(),
            peripheral_hint: Cell::new(None),
            last_gpio_in: [0; 2],
            current_cycle: 0,
            pending_schedule: Vec::new(),
            legacy_walk_disabled: false,
            hcsr04: Vec::new(),
            can_diagnostic_testers: Vec::new(),
            can_uds_testers: Vec::new(),
            esp32c3_irq_routing: false,
            riscv_irq_lines: 0,
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
            transform: None,
        };
        bus.execute_dma(&[req]).unwrap();

        assert_eq!(bus.read_u8(0x2000_0020).unwrap(), 0xAB);
    }

    #[test]
    fn test_dma_tick_executes_copy_and_raises_irq() {
        let mut bus = SystemBus {
            flash: LinearMemory::new(256, 0x0800_0000),
            ram: LinearMemory::new(256, 0x2000_0000),
            extra_mem: Vec::new(),
            peripherals: vec![PeripheralEntry {
                name: "dma1".to_string(),
                base: 0x4002_0000,
                size: 0x400,
                irq: Some(16),
                dev: Box::new(crate::peripherals::dma::Dma1::new()),
                ticks_remaining: 0,
                generation: 0,
                clock_gate: None,
            }],
            nvic: None,
            observers: Vec::new(),
            config: crate::SimulationConfig::default(),
            bit_band_enabled: true,
            pending_cpu_irqs: [0; 2],
            dport_idx: None,
            rcc_idx: None,
            flash_thunks: std::collections::HashMap::new(),
            peripheral_ranges: Vec::new(),
            peripheral_hint: Cell::new(None),
            last_gpio_in: [0; 2],
            current_cycle: 0,
            pending_schedule: Vec::new(),
            legacy_walk_disabled: false,
            hcsr04: Vec::new(),
            can_diagnostic_testers: Vec::new(),
            can_uds_testers: Vec::new(),
            esp32c3_irq_routing: false,
            riscv_irq_lines: 0,
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

    /// RCC clock-gating (silicon fidelity): a peripheral with a declared
    /// `clock:` gate is inert until its RCC enable bit is set — writes are
    /// dropped and reads return 0 — and behaves normally once clocked. The
    /// reg-name → offset mapping is family-aware (F1 apb2enr @ 0x18).
    #[test]
    fn gated_peripheral_is_inert_until_rcc_bit_set() {
        let chip: ChipDescriptor = serde_yaml::from_str(
            r#"
name: "f1-clockgate-test"
arch: "arm"
core: "cortex-m3"
flash:
  base: 0x08000000
  size: "64KB"
ram:
  base: 0x20000000
  size: "20KB"
peripherals:
  - id: "rcc"
    type: "rcc"
    base_address: 0x40021000
    size: "1KB"
  - id: "uart1"
    type: "uart"
    base_address: 0x40013800
    size: "1KB"
    clock: { reg: "apb2enr", bit: 14 }
  - id: "uart2"
    type: "uart"
    base_address: 0x40004400
    size: "1KB"
"#,
        )
        .unwrap();
        let manifest: SystemManifest = serde_yaml::from_str(
            r#"
name: "clockgate"
chip: "unused"
external_devices: []
board_io: []
"#,
        )
        .unwrap();
        let mut bus = SystemBus::from_config(&chip, &manifest).unwrap();

        // USART1_CR1 @ 0x4001_380C. Clock is OFF out of reset → the write is
        // dropped and the register reads back 0 (an unclocked peripheral).
        const CR1: u64 = 0x4001_380C;
        const CR1_UE_TE: u32 = (1 << 13) | (1 << 3);
        bus.write_u32(CR1, CR1_UE_TE).unwrap();
        assert_eq!(
            bus.read_u32(CR1).unwrap(),
            0,
            "unclocked USART1 must drop writes and read 0"
        );

        // The ungated uart2 (no clock declared) is unaffected — accessible now.
        const UART2_CR1: u64 = 0x4000_440C;
        bus.write_u32(UART2_CR1, CR1_UE_TE).unwrap();
        assert_eq!(
            bus.read_u32(UART2_CR1).unwrap() & CR1_UE_TE,
            CR1_UE_TE,
            "ungated uart2 must work regardless of RCC"
        );

        // Enable RCC_APB2ENR.USART1EN (bit 14). RCC itself is never gated.
        const RCC_APB2ENR: u64 = 0x4002_1018;
        bus.write_u32(RCC_APB2ENR, 1 << 14).unwrap();
        assert_eq!(bus.read_u32(RCC_APB2ENR).unwrap() & (1 << 14), 1 << 14);

        // Now USART1 is clocked: the same write takes effect and reads back.
        bus.write_u32(CR1, CR1_UE_TE).unwrap();
        assert_eq!(
            bus.read_u32(CR1).unwrap() & CR1_UE_TE,
            CR1_UE_TE,
            "clocked USART1 must accept writes"
        );

        // Drop the clock again → the peripheral goes inert (reads 0).
        bus.write_u32(RCC_APB2ENR, 0).unwrap();
        assert_eq!(
            bus.read_u32(CR1).unwrap(),
            0,
            "USART1 must go inert again when its clock is removed"
        );
    }

    #[test]
    fn gated_peripheral_resolves_l4_rcc_offsets() {
        // The SAME symbolic reg names that map to F1 offsets above must resolve
        // to the L4 family's offsets via Rcc::enable_reg_offset: apb1enr1 @ 0x58
        // (not F1's 0x1C) and ahb2enr @ 0x4C. Mirrors the al2205 (USART2 on
        // apb1enr1) and nokia5110 (GPIOA on ahb2enr) gates on the L476.
        let chip: ChipDescriptor = serde_yaml::from_str(
            r#"
name: "l4-clockgate-test"
arch: "arm"
core: "cortex-m4"
flash:
  base: 0x08000000
  size: "1MB"
ram:
  base: 0x20000000
  size: "96KB"
peripherals:
  - id: "rcc"
    type: "rcc"
    base_address: 0x40021000
    size: "1KB"
    config:
      profile: "stm32l4"
  - id: "gpioa"
    type: "gpio"
    base_address: 0x48000000
    size: "1KB"
    config:
      profile: "stm32v2"
    clock: { reg: "ahb2enr", bit: 0 }
  - id: "uart2"
    type: "uart"
    base_address: 0x40004400
    size: "1KB"
    config:
      profile: "stm32v2"
    clock: { reg: "apb1enr1", bit: 17 }
"#,
        )
        .unwrap();
        let manifest: SystemManifest = serde_yaml::from_str(
            r#"
name: "clockgate-l4"
chip: "unused"
external_devices: []
board_io: []
"#,
        )
        .unwrap();
        let mut bus = SystemBus::from_config(&chip, &manifest).unwrap();

        // USART2_CR1 @ 0x4000_4400 (stm32v2 layout: CR1 at offset 0x00).
        // Clock OFF out of reset.
        const U2_CR1: u64 = 0x4000_4400;
        const CR1_UE_TE: u32 = (1 << 0) | (1 << 3);
        bus.write_u32(U2_CR1, CR1_UE_TE).unwrap();
        assert_eq!(
            bus.read_u32(U2_CR1).unwrap(),
            0,
            "unclocked USART2 must drop writes and read 0"
        );

        // RCC_APB1ENR1 @ 0x58 (L4 offset, NOT the F1 0x1C). USART2EN = bit 17.
        const RCC_APB1ENR1: u64 = 0x4002_1058;
        bus.write_u32(RCC_APB1ENR1, 1 << 17).unwrap();
        bus.write_u32(U2_CR1, CR1_UE_TE).unwrap();
        assert_eq!(
            bus.read_u32(U2_CR1).unwrap() & CR1_UE_TE,
            CR1_UE_TE,
            "clocked USART2 must accept writes once apb1enr1.17 is set"
        );

        // GPIOA_MODER @ 0x4800_0000, gated on RCC_AHB2ENR @ 0x4C bit 0.
        const GPIOA_MODER: u64 = 0x4800_0000;
        bus.write_u32(GPIOA_MODER, 0x55).unwrap();
        assert_eq!(
            bus.read_u32(GPIOA_MODER).unwrap(),
            0,
            "unclocked GPIOA must drop writes and read 0"
        );
        const RCC_AHB2ENR: u64 = 0x4002_104C;
        bus.write_u32(RCC_AHB2ENR, 1 << 0).unwrap();
        bus.write_u32(GPIOA_MODER, 0x55).unwrap();
        assert_eq!(
            bus.read_u32(GPIOA_MODER).unwrap() & 0x55,
            0x55,
            "clocked GPIOA must accept writes once ahb2enr.0 is set"
        );
    }
}
