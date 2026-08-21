// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! GPIO peripheral for ESP32-S3.
//!
//! Base address `DR_REG_GPIO_BASE = 0x6000_4000`, architected span
//! 0x000..0x700 (last register `GPIO_DATE` @ 0x6FC). Per ESP32-S3 TRM §5.5.
//!
//! ## Behavioral model (Plan 3 — unchanged by the register-file slice)
//!
//! - Output direction (ENABLE/ENABLE_W1TS/ENABLE_W1TC @ 0x20/0x24/0x28)
//! - Output value (OUT/OUT_W1TS/OUT_W1TC @ 0x04/0x08/0x0C) with synchronous
//!   [`GpioObserver`] notification on every pin transition
//! - Input value (IN @ 0x3C / IN1 @ 0x40). A pad reads its external drive
//!   (`set_pin_input` / board stimulus) when one is asserted, otherwise the
//!   IO_MUX pad's weak pull-up (`FUN_WPU`) — this is what makes Arduino
//!   `pinMode(pin, INPUT_PULLUP)` read 1 on a released pin. The IO_MUX bank is
//!   shared in by `SystemBus::wire_esp32s3_pad_controls`; with no IO_MUX on the
//!   bus every released pad reads 0, exactly as before.
//! - Boot straps (STRAP @ 0x38, read-only): 0x8 = SPI_FAST_FLASH_BOOT
//!   (GPIO0 high), captured from silicon over JTAG — the SVD reset value (0)
//!   would send the boot ROM into download mode
//! - PIN0..31 int_type/int_ena fields (bits [9:7] / bit 13) kept in sync with
//!   the stored register word (GPIO-input IRQs not yet routed to the
//!   intmatrix in Plan 3)
//!
//! ## Register file
//!
//! All 397 architected registers of the ESP32-S3 SVD `GPIO` block are
//! modeled: each register is seeded with its SVD reset value and a write
//! applies the register's writable-bit mask
//! (`stored = (stored & !wmask) | (value & wmask)`) — read-only registers
//! (PCPU_INT, PCPU_NMI_INT, CPUSDIO_INT and their `1` twins, STATUS_NEXT,
//! STATUS_NEXT1, STRAP) ignore writes. The PIN0..53 array (0x74, stride 4),
//! FUNC0..255_IN_SEL_CFG (0x154) and FUNC0..53_OUT_SEL_CFG (0x554) arrays are
//! handled as offset ranges sharing one `(reset, wmask)` spec each. The
//! second-bank registers (OUT1/ENABLE1/STATUS/STATUS1 with their W1TS/W1TC
//! views, IN1, pins 32..53) are masked storage with architected
//! write-1-to-set / write-1-to-clear arithmetic — no interrupt semantics or
//! GPIO-matrix routing is invented on top.
//!
//! Offsets outside the architected map (the 0x630..0x6F8 hole and everything
//! at/above 0x700) read as zero and ignore writes, NOT round-trip, so the SVD
//! behavioral coverage probe cannot mistake this model for generic storage.
//!
//! Reset values and write masks are sourced from the ESP32-S3 SVD; they are
//! NOT validated against silicon dumps (except STRAP, see above). The SVD
//! marks IN/IN1's `DATA_NEXT` field read-write — the TRM documents the
//! registers as read-only on silicon — so a write to IN/IN1 stores into the
//! same cell `set_pin_input` drives, keeping read-back coherent.
//!
//! ## GpioObserver
//!
//! The peripheral notifies registered observers synchronously on every
//! pin transition. Observers receive `(pin, from, to, sim_cycle)` and
//! must not panic.
//!
//! Both the firmware OUT-register write path (`apply_out` for bank-0 GPIO0..31,
//! `apply_out1` for bank-1 GPIO32..53) and the peripheral-driven
//! `drive_pad_output` seam (RMT Stage 1 — what a timed WS2812/RMT playback
//! engine will call) funnel through those SAME chokepoints, so an observer
//! captures pad-level edges identically no matter which source or bank flips the
//! pad. Bank 1 matters because the onboard NeoPixel on most S3 boards is GPIO48.

use crate::{CycleClock, Peripheral, PeripheralTickResult, SimResult};
use std::sync::Arc;

const BT_SELECT: u64 = 0x00;
const OUT: u64 = 0x04;
const OUT_W1TS: u64 = 0x08;
const OUT_W1TC: u64 = 0x0C;
const OUT1: u64 = 0x10;
const OUT1_W1TS: u64 = 0x14;
const OUT1_W1TC: u64 = 0x18;
const SDIO_SELECT: u64 = 0x1C;
const ENABLE: u64 = 0x20;
const ENABLE_W1TS: u64 = 0x24;
const ENABLE_W1TC: u64 = 0x28;
const ENABLE1: u64 = 0x2C;
const ENABLE1_W1TS: u64 = 0x30;
const ENABLE1_W1TC: u64 = 0x34;
/// GPIO_STRAP_REG: latched boot-mode straps. The real boot ROM reads this to
/// choose flash-boot vs download. Reset seeded to 0x8 = SPI_FAST_FLASH_BOOT
/// (GPIO0 high), captured from silicon over JTAG.
const STRAP: u64 = 0x38;
const IN: u64 = 0x3C;
const IN1: u64 = 0x40;
const STATUS: u64 = 0x44;
const STATUS_W1TS: u64 = 0x48;
const STATUS_W1TC: u64 = 0x4C;
const STATUS1: u64 = 0x50;
const STATUS1_W1TS: u64 = 0x54;
const STATUS1_W1TC: u64 = 0x58;
/// PCPU_INT (0x5C), PCPU_NMI_INT (0x60), CPUSDIO_INT (0x64) — RO.
const PCPU_INT: u64 = 0x5C;
/// PCPU_INT1 (0x68), PCPU_NMI_INT1 (0x6C), CPUSDIO_INT1 (0x70) — RO.
const CPUSDIO_INT1: u64 = 0x70;
/// PIN0..PIN53 @ 0x74 + n*4 (SVD dim=54, stride 4).
const PIN0: u64 = 0x74;
const PIN31: u64 = PIN0 + 31 * 4;
const PIN53: u64 = PIN0 + 53 * 4;
const STATUS_NEXT: u64 = 0x14C;
const STATUS_NEXT1: u64 = 0x150;
/// FUNC0..255_IN_SEL_CFG @ 0x154 + n*4 (SVD dim=256, stride 4).
const FUNC0_IN_SEL_CFG: u64 = 0x154;
const FUNC255_IN_SEL_CFG: u64 = FUNC0_IN_SEL_CFG + 255 * 4;
/// FUNC0..53_OUT_SEL_CFG @ 0x554 + n*4 (SVD dim=54, stride 4).
const FUNC0_OUT_SEL_CFG: u64 = 0x554;
const FUNC53_OUT_SEL_CFG: u64 = FUNC0_OUT_SEL_CFG + 53 * 4;
const CLOCK_GATE: u64 = 0x62C;
/// GPIO_DATE (0x6FC) — version stamp, last architected register.
const REG_DATE: u64 = 0x6FC;

/// Second-bank registers carry GPIO32..53 → 22 valid bits.
const BANK1_MASK: u32 = 0x003F_FFFF;
/// PINn writable bits per SVD: sync stages [4:0] (bits 5/6 reserved),
/// pad_driver bit 7 + INT_TYPE [9:7] region, WAKEUP_ENABLE bit 10,
/// CONFIG [12:11], INT_ENA [17:13].
const PIN_WMASK: u32 = 0x0003_FF9F;

/// One word past the last architected register (`REG_DATE` @ 0x6FC).
const NWORDS: usize = 0x700 / 4;

/// Number of GPIO pads with a `FUNCn_OUT_SEL_CFG` entry (GPIO0..53).
const PAD_COUNT: u8 = 54;
/// `GPIO_FUNCn_OUT_SEL_CFG.out_sel` field width: bits [8:0] pick which internal
/// peripheral output signal drives pad `n` (ESP32-S3 TRM §5.4 GPIO matrix).
const OUT_SEL_MASK: u32 = 0x1FF;
/// `out_sel` sentinel meaning "pad driven directly by the `GPIO_OUT` latch"
/// (plain GPIO output, matrix bypassed). esp-idf `SIG_GPIO_OUT_IDX`; note this
/// is 256 on the S3, not 128 as on the C3. It is also the reset value of every
/// `FUNCn_OUT_SEL_CFG` register (see `spec()` → 0x100).
const SIG_GPIO_OUT: u32 = 256;

/// GPIO-matrix OUTPUT signal indices of the RMT TX channels (esp-idf
/// `soc/esp32s3/include/soc/gpio_sig_map.h` — `RMT_SIG_OUT0..3_IDX`; on the S3
/// the RMT in/out share one matrix index per channel). A timed RMT channel
/// finds the pad it drives by calling
/// [`Esp32s3Gpio::pads_for_output_signal`] with its channel's index.
/// GPIO-matrix OUTPUT signal indices of the I²C0 controller, from esp-idf
/// `soc/esp32s3/include/soc/gpio_sig_map.h` (`I2CEXT0_SCL_OUT_IDX` /
/// `I2CEXT0_SDA_OUT_IDX`). These are S3 numbers and differ from the C3's
/// 53/54 — the matrix index space is per-chip, so a pad routed by index alone
/// must never be assumed portable between ESP families.
const SIG_I2CEXT0_SCL: u32 = 89;
const SIG_I2CEXT0_SDA: u32 = 90;

/// GPIO-matrix OUTPUT signal indices of GP-SPI2 (FSPI) — the controller
/// arduino-esp32's `SPIClass SPI(FSPI)` drives. esp-idf
/// `soc/esp32s3/include/soc/gpio_sig_map.h`: `FSPICLK_OUT_IDX` :194,
/// `FSPID_OUT_IDX` :198, `FSPICS0_OUT_IDX` :212.
///
/// ⚠️ S3 numbers. The C3 spells the same three signals 63/65/68 and the classic
/// ESP32's VSPI is 63/65/68 as well — identical VALUES with a different
/// meaning, which is precisely why a sibling's constant must never be borrowed:
/// on the S3, 63 is not a SPI signal at all.
const SIG_FSPICLK: u32 = 101;
const SIG_FSPID: u32 = 103;
const SIG_FSPICS0: u32 = 110;
/// GPIO-matrix OUTPUT signal indices of the UART transmitters (esp-idf
/// `gpio_sig_map.h` :39 `U0TXD_OUT_IDX`, :45 `U1TXD_OUT_IDX`,
/// :51 `U2TXD_OUT_IDX`).
///
/// ⚠️ The DEFAULT `U0TXD` pad (GPIO43) reaches the pin through IO_MUX
/// function 0 (`io_mux_reg.h` :407), NOT through this matrix, so a stock
/// `Serial.begin()` leaves that route dark and the pad rightly keeps reading the
/// GPIO latch. The route lights up when firmware remaps TX — and UART2 has no
/// IO_MUX route on the S3 at all, so it is matrix-only.
const SIG_U0TXD: u32 = 12;
const SIG_U1TXD: u32 = 15;
const SIG_U2TXD: u32 = 18;

/// Line indices within the I²C controller's published pad lines.
const I2C_LINE_SCL: usize = super::i2c::LINE_SCL;
const I2C_LINE_SDA: usize = super::i2c::LINE_SDA;

pub const RMT_SIG_OUT0: u32 = 81;
pub const RMT_SIG_OUT1: u32 = 82;
pub const RMT_SIG_OUT2: u32 = 83;
pub const RMT_SIG_OUT3: u32 = 84;

/// GPIO-matrix OUTPUT signal index → name, for the signals the S3 model cares
/// about (currently the RMT TX channels). Unmapped indices → `None` (never a
/// guess), matching the C3 `c3_out_signal_name` convention.
fn s3_out_signal_name(idx: u32) -> Option<&'static str> {
    Some(match idx {
        SIG_I2CEXT0_SCL => "I2CEXT0_SCL",
        SIG_I2CEXT0_SDA => "I2CEXT0_SDA",
        RMT_SIG_OUT0 => "RMT_SIG_OUT0",
        RMT_SIG_OUT1 => "RMT_SIG_OUT1",
        RMT_SIG_OUT2 => "RMT_SIG_OUT2",
        RMT_SIG_OUT3 => "RMT_SIG_OUT3",
        _ => return None,
    })
}

/// `(reset value, writable-bit mask)` for the architected register at word
/// index `word` (offset `word * 4`), exactly per the ESP32-S3 SVD `GPIO`
/// block; `None` = hole in the register map (reads 0, ignores writes).
/// `wmask == 0` = read-only register (writes ignored, reset value sticks).
const fn spec(word: usize) -> Option<(u32, u32)> {
    match (word as u64) * 4 {
        BT_SELECT => Some((0x0000_0000, 0xFFFF_FFFF)),
        // OUT group: behavioral overlay (apply_out + observers).
        OUT..=OUT_W1TC => Some((0x0000_0000, 0xFFFF_FFFF)),
        OUT1..=OUT1_W1TC => Some((0x0000_0000, BANK1_MASK)),
        SDIO_SELECT => Some((0x0000_0000, 0x0000_00FF)),
        // ENABLE group: behavioral overlay.
        ENABLE..=ENABLE_W1TC => Some((0x0000_0000, 0xFFFF_FFFF)),
        ENABLE1..=ENABLE1_W1TC => Some((0x0000_0000, BANK1_MASK)),
        STRAP => Some((0x0000_0008, 0x0000_0000)), // RO, silicon-captured
        IN => Some((0x0000_0000, 0xFFFF_FFFF)),    // behavioral (effective_input)
        IN1 => Some((0x0000_0000, BANK1_MASK)),
        STATUS..=STATUS_W1TC => Some((0x0000_0000, 0xFFFF_FFFF)),
        STATUS1..=STATUS1_W1TC => Some((0x0000_0000, BANK1_MASK)),
        PCPU_INT..=CPUSDIO_INT1 => Some((0x0000_0000, 0x0000_0000)), // RO
        PIN0..=PIN53 => Some((0x0000_0000, PIN_WMASK)),
        STATUS_NEXT => Some((0x0000_0000, 0x0000_0000)), // RO
        STATUS_NEXT1 => Some((0x0000_0000, 0x0000_0000)), // RO
        FUNC0_IN_SEL_CFG..=FUNC255_IN_SEL_CFG => Some((0x0000_0000, 0x0000_00FF)),
        FUNC0_OUT_SEL_CFG..=FUNC53_OUT_SEL_CFG => Some((0x0000_0100, 0x0000_0FFF)),
        CLOCK_GATE => Some((0x0000_0001, 0x0000_0001)),
        REG_DATE => Some((0x0190_7040, 0x0FFF_FFFF)),
        _ => None,
    }
}

/// Notified synchronously inside the bus write path on every GPIO pin
/// transition. Observers must not panic — a panic propagates out of
/// `bus.write_u8` and crashes the simulator.
pub trait GpioObserver: Send + Sync + std::fmt::Debug {
    fn on_pin_change(&self, pin: u8, from: bool, to: bool, sim_cycle: u64);
}

/// ESP32-S3 GPIO peripheral. Mapped at 0x6000_4000.
/// Push-capture state for one S3 GPIO port. `scratch` caches each watched
/// pad's level across a register write so only real transitions are reported;
/// `line_chs` caches the per-line channel lists last pushed into the I²C wire,
/// so unrelated writes cost no mutex traffic.
#[derive(Debug)]
struct S3PortTap {
    tap: crate::logic_capture::LogicTap,
    watched: Vec<(u8, u32)>,
    scratch: Vec<Option<bool>>,
}

pub struct Esp32s3Gpio {
    /// Register file for the architected map (word-indexed; holes stay 0 and
    /// are never read back — `spec()` gates both directions). OUT, ENABLE and
    /// IN live in the dedicated behavioral fields below instead.
    regs: [u32; NWORDS],
    enable: u32,
    out: u32,
    /// Bank-1 output latch for GPIO32..53 (bits 0..21 = pins 32..53), masked to
    /// [`BANK1_MASK`]. Served in place of `regs[OUT1/4]` so `apply_out1` can fire
    /// observers on bank-1 pad transitions (e.g. the onboard NeoPixel on GPIO48).
    out1: u32,
    /// Host/board-driven pad levels, `[bank0 = GPIO0..31, bank1 = GPIO32..53]`.
    /// A bit only has electrical authority when the matching
    /// [`Self::external_drive_mask`] bit is set; otherwise the IO_MUX pull-up
    /// (if any) supplies the released level.
    external_levels: [u32; 2],
    external_drive_mask: [u32; 2],
    /// Shared IO_MUX per-pad register words (`IO_MUX_GPIOn_REG`). Installed by
    /// `SystemBus::wire_esp32s3_pad_controls`. The pad's weak pull-up is an
    /// electrical input condition, so it is kept separate from the output
    /// matrix in `pad_routes`.
    pad_controls: Option<super::io_mux::PadControls>,
    /// Live I²C0 wire levels shared with the I²C controller, installed at bus
    /// wiring time (`SystemBus::wire_esp32s3_i2c_pads`). Pads whose output
    /// matrix routes `I2CEXT0_SCL`/`SDA` read the wire here instead of the
    /// GPIO_OUT latch.
    /// Pads bound to peripheral wires, resolved against this port's live
    /// output matrix. Empty until `SystemBus::wire_esp32s3_i2c_pads` binds them.
    pad_routes: crate::peripherals::pad_routing::PadRoutes,
    /// `Some` while the logic analyzer watches pads on this port in push mode.
    /// Every register write then reports watched pad-level changes into the
    /// tap, and matrix-routed pads are registered with the I²C wire so IT
    /// reports their transitions at the cycles they occurred. Not snapshot
    /// state — the watch is re-armed by the frontend after a resume.
    tap: Option<S3PortTap>,
    int_enable: u32,
    int_type: [u8; 32],
    cycle: u64,
    /// Bus cycle clock — stamps pin edges when the walk is deleted.
    clock: Option<CycleClock>,
    observers: Vec<Arc<dyn GpioObserver>>,
}

impl Esp32s3Gpio {
    fn stamp_cycle(&self) -> u64 {
        self.clock.as_ref().map(|c| c.now()).unwrap_or(self.cycle)
    }

    pub fn new() -> Self {
        let mut regs = [0u32; NWORDS];
        let mut w = 0;
        while w < NWORDS {
            if let Some((reset, _)) = spec(w) {
                regs[w] = reset;
            }
            w += 1;
        }
        Self {
            regs,
            enable: 0,
            out: 0,
            out1: 0,
            external_levels: [0; 2],
            external_drive_mask: [0; 2],
            pad_controls: None,
            pad_routes: crate::peripherals::pad_routing::PadRoutes::new(),
            tap: None,
            int_enable: 0,
            int_type: [0; 32],
            cycle: 0,
            clock: None,
            observers: Vec::new(),
        }
    }

    pub fn add_observer(&mut self, obs: Arc<dyn GpioObserver>) {
        self.observers.push(obs);
    }

    /// Drive output pad `pin` (0..=48) to `level` from a *peripheral* source
    /// (e.g. the RMT / LED-strip output matrix bit-banging a WS2812 line),
    /// routing the transition through the SAME `apply_out` / `apply_out1`
    /// chokepoint — and therefore the same [`GpioObserver`] notification — a
    /// firmware OUT-register write takes. This is the RMT-Stage-1 output seam: a
    /// timed playback engine (Stage 2) calls this at each scheduled edge cycle
    /// and the observer sees `(pin, from, to, sim_cycle)` exactly as it would for
    /// a CPU-driven toggle.
    ///
    /// `pin < 32` drives bank-0 (`OUT`); `32..=48` drives bank-1 (`OUT1`, bit =
    /// `pin - 32`) — the onboard NeoPixel on most S3 boards is GPIO48. Costs
    /// nothing when `level` leaves the pad unchanged (both appliers early-out on
    /// `diff == 0`). Returns `false` for `pin > 48`.
    ///
    /// NOTE: this updates the OUT/OUT1 latch bit for `pin`, so a subsequent
    /// `read_gpio_output` (bank 0) or `OUT1` read (bank 1) reflects the
    /// peripheral-driven level. It does not touch ENABLE — Stage 1 assumes the
    /// pad is already configured as an output.
    pub fn drive_pad_output(&mut self, pin: u8, level: bool) -> bool {
        if pin > 48 {
            return false;
        }
        if pin < 32 {
            let mask = 1u32 << pin;
            let new_out = if level {
                self.out | mask
            } else {
                self.out & !mask
            };
            self.apply_out(new_out);
        } else {
            let mask = 1u32 << (pin - 32);
            let new_out1 = if level {
                self.out1 | mask
            } else {
                self.out1 & !mask
            };
            self.apply_out1(new_out1);
        }
        true
    }

    /// The GPIO-matrix output selector for pad `pin` (`FUNCn_OUT_SEL_CFG.out_sel`,
    /// bits [8:0]) — which internal peripheral output signal drives the pad, or
    /// [`SIG_GPIO_OUT`] (256) for a plain GPIO output. `None` for `pin >= 54`.
    fn out_sel(&self, pin: u8) -> Option<u32> {
        if pin >= PAD_COUNT {
            return None;
        }
        Some(self.reg(FUNC0_OUT_SEL_CFG + (pin as u64) * 4) & OUT_SEL_MASK)
    }

    /// Pads (0..=53) whose GPIO-matrix output selector routes peripheral output
    /// `signal_idx` to the pad — the faithful S3 output-matrix lookup an RMT
    /// channel uses to discover which GPIO its waveform reaches (call with
    /// [`RMT_SIG_OUT0`]..[`RMT_SIG_OUT3`]). A signal may fan out to several pads,
    /// so every match is returned, in ascending pad order.
    ///
    /// Passing [`SIG_GPIO_OUT`] (256) would match every pad still at its reset
    /// selector — callers resolving a *peripheral* signal never do that.
    /// Snapshot every watched pad's level before a mutation. One branch while
    /// no tap is installed.
    #[inline]
    fn tap_snapshot(&mut self) {
        let Some(mut t) = self.tap.take() else {
            return;
        };
        for (k, &(pin, _)) in t.watched.iter().enumerate() {
            t.scratch[k] = self.pad_level_for_tap(pin);
        }
        self.tap = Some(t);
    }

    /// Report watched pads whose level changed since the matching
    /// [`tap_snapshot`](Self::tap_snapshot), then re-sync the I²C wire
    /// registration in case the write re-routed a watched pad.
    #[inline]
    fn tap_report(&mut self) {
        let Some(t) = self.tap.take() else {
            return;
        };
        for (k, &(pin, ch)) in t.watched.iter().enumerate() {
            if let Some(level) = self.pad_level_for_tap(pin) {
                if t.scratch[k] != Some(level) {
                    t.tap.push(ch, level);
                }
            }
        }
        self.tap = Some(t);
        self.sync_i2c_line_taps();
    }

    /// The level a watched pad currently reads, through the same truth
    /// `read_gpio_pad` uses.
    fn pad_level_for_tap(&self, pin: u8) -> Option<bool> {
        <Self as Peripheral>::read_gpio_pad(self, pin)
    }

    /// Re-register watched pads with the wires that drive them, so a pad the
    /// matrix hands over (or takes back) follows its new source.
    fn sync_i2c_line_taps(&mut self) {
        if self.pad_routes.is_empty() {
            return;
        }
        let Some(t) = self.tap.take() else {
            return;
        };
        let mut routes = std::mem::take(&mut self.pad_routes);
        routes.sync_taps(&t.tap, &t.watched, |pin| self.out_sel(pin));
        self.pad_routes = routes;
        self.tap = Some(t);
    }

    /// Bind an I²C controller's wire to the pads the output matrix can route it
    /// to. Called once at bus wiring time; which pad is live at any moment is
    /// then decided by `FUNCn_OUT_SEL`, through the shared routing seam.
    pub(crate) fn set_i2c_lines(
        &mut self,
        lines: std::sync::Arc<crate::peripherals::pad_lines::PadLines>,
    ) {
        for pin in 0..PAD_COUNT {
            self.pad_routes.bind(
                &lines,
                pin,
                Some(SIG_I2CEXT0_SCL),
                I2C_LINE_SCL,
                "I2CEXT0_SCL",
            );
            self.pad_routes.bind(
                &lines,
                pin,
                Some(SIG_I2CEXT0_SDA),
                I2C_LINE_SDA,
                "I2CEXT0_SDA",
            );
        }
    }

    /// Bind GP-SPI2's SCK/MOSI/CS wire to every pad the output matrix can route
    /// it to. Which pad is live at any moment is then decided by
    /// `FUNCn_OUT_SEL_CFG`, through the shared routing seam — exactly as for
    /// I²C above. MISO is deliberately not bound; see
    /// [`crate::peripherals::esp_gpspi_wire`].
    pub(crate) fn bind_spi_lines(
        &mut self,
        lines: &std::sync::Arc<crate::peripherals::pad_lines::PadLines>,
    ) {
        use crate::peripherals::esp_gpspi_wire::{LINE_CS, LINE_MOSI, LINE_SCK};
        for pin in 0..PAD_COUNT {
            self.pad_routes
                .bind(lines, pin, Some(SIG_FSPICLK), LINE_SCK, "SPI2_SCK");
            self.pad_routes
                .bind(lines, pin, Some(SIG_FSPID), LINE_MOSI, "SPI2_MOSI");
            self.pad_routes
                .bind(lines, pin, Some(SIG_FSPICS0), LINE_CS, "SPI2_CS");
        }
    }

    /// Bind one UART's TX wire to every pad the output matrix can route it to.
    ///
    /// TX ONLY, for the reason `wire_rp2040_uart_pads` documents: nothing in the
    /// engine drives RX, so a bound RX pad would report a confident constant
    /// idle-high — worse than the GPIO-latch fallback, because it looks
    /// authoritative.
    pub(crate) fn bind_uart_tx_lines(
        &mut self,
        instance: usize,
        lines: &std::sync::Arc<crate::peripherals::pad_lines::PadLines>,
    ) {
        let (signal, func) = match instance {
            0 => (SIG_U0TXD, "UART0_TX"),
            1 => (SIG_U1TXD, "UART1_TX"),
            2 => (SIG_U2TXD, "UART2_TX"),
            _ => return,
        };
        for pin in 0..PAD_COUNT {
            self.pad_routes.bind(
                lines,
                pin,
                Some(signal),
                crate::peripherals::uart::LINE_TX,
                func,
            );
        }
    }

    /// Every signal name bound to this port's pads, live or not — the
    /// bus-visibility reporting seam. See
    /// [`crate::peripherals::pad_routing::PadRoutes::bound_functions`] for why
    /// this is the static question and `func()` is the live one.
    pub(crate) fn bound_pad_functions(&self) -> Vec<&'static str> {
        self.pad_routes.bound_functions()
    }

    pub fn pads_for_output_signal(&self, signal_idx: u32) -> Vec<u8> {
        let want = signal_idx & OUT_SEL_MASK;
        (0..PAD_COUNT)
            .filter(|&pin| self.out_sel(pin) == Some(want))
            .collect()
    }

    /// Set the input level on `pin` (0..=31). Used by tests, boards and
    /// stimulus generators. This is an *external drive*: it takes electrical
    /// priority over the pad's weak internal pull-up, so a button holding the
    /// pin low still reads 0 with `INPUT_PULLUP` configured.
    pub fn set_pin_input(&mut self, pin: u8, level: bool) {
        assert!(pin < 32, "set_pin_input: pin {pin} >= 32");
        if level {
            self.external_levels[0] |= 1u32 << pin;
        } else {
            self.external_levels[0] &= !(1u32 << pin);
        }
        self.external_drive_mask[0] |= 1u32 << pin;
    }

    /// Wire the S3 IO_MUX's shared per-pad controls after both peripherals
    /// exist on the system bus.
    pub(crate) fn set_pad_controls(&mut self, controls: super::io_mux::PadControls) {
        self.pad_controls = Some(controls);
    }

    /// `FUN_WPU` per pad, split into the two GPIO input banks. Pads 49..53 have
    /// no `IO_MUX_GPIOn_REG` on the S3, so they never carry a pull-up here.
    fn io_mux_pullup_mask(&self) -> [u32; 2] {
        let Some(controls) = &self.pad_controls else {
            return [0; 2];
        };
        controls
            .read()
            .expect("ESP32-S3 IO_MUX pad controls poisoned")
            .iter()
            .enumerate()
            .fold([0u32; 2], |mut mask, (pin, word)| {
                if word & super::io_mux::FUN_WPU != 0 {
                    if pin < 32 {
                        mask[0] |= 1u32 << pin;
                    } else {
                        mask[1] |= 1u32 << (pin - 32);
                    }
                }
                mask
            })
    }

    /// Firmware-visible input word for `bank` (0 = `IN`/GPIO0..31,
    /// 1 = `IN1`/GPIO32..53). An explicit external drive always beats a weak
    /// internal pull-up; otherwise the raw IO_MUX `FUN_WPU` bit supplies the
    /// released level, including its SVD-defined cold reset. Without the
    /// pull-up term a released `INPUT_PULLUP` pin reads 0 and every
    /// button-to-GND lab reads permanently pressed.
    fn effective_input(&self, bank: usize) -> u32 {
        let valid = if bank == 0 { u32::MAX } else { BANK1_MASK };
        ((self.external_levels[bank] & self.external_drive_mask[bank])
            | (self.io_mux_pullup_mask()[bank] & !self.external_drive_mask[bank]))
            & valid
    }

    /// Internal: apply a new `out` value, fire observers for each
    /// flipped bit.
    fn apply_out(&mut self, new_out: u32) {
        let old = self.out;
        let new = new_out;
        self.out = new;
        let diff = old ^ new;
        if diff == 0 {
            return;
        }
        for pin in 0u8..32 {
            let mask = 1u32 << pin;
            if diff & mask != 0 {
                let from = old & mask != 0;
                let to = new & mask != 0;
                for obs in &self.observers {
                    obs.on_pin_change(pin, from, to, self.stamp_cycle());
                }
            }
        }
    }

    /// Internal: apply a new bank-1 `out1` value (masked to [`BANK1_MASK`]),
    /// firing observers for each flipped bit as `pin = 32 + bit`. Mirrors
    /// [`apply_out`] exactly — same `diff == 0` early-out, same `sim_cycle`
    /// stamp — so a GPIO32..53 pad transition (register write or peripheral
    /// drive) reaches observers identically to a bank-0 one.
    fn apply_out1(&mut self, new_out1: u32) {
        let old = self.out1;
        let new = new_out1 & BANK1_MASK;
        self.out1 = new;
        let diff = old ^ new;
        if diff == 0 {
            return;
        }
        for bit in 0u8..22 {
            let mask = 1u32 << bit;
            if diff & mask != 0 {
                let from = old & mask != 0;
                let to = new & mask != 0;
                let pin = 32 + bit;
                for obs in &self.observers {
                    obs.on_pin_change(pin, from, to, self.stamp_cycle());
                }
            }
        }
    }

    /// Architected register-file read; holes read 0.
    fn reg(&self, off: u64) -> u32 {
        let w = (off / 4) as usize;
        if w < NWORDS && spec(w).is_some() {
            self.regs[w]
        } else {
            0
        }
    }

    /// Masked store into an architected register; no-op on holes and on
    /// fully read-only registers (`wmask == 0`).
    fn set_reg_masked(&mut self, off: u64, value: u32) {
        let w = (off / 4) as usize;
        if w < NWORDS {
            if let Some((_, wmask)) = spec(w) {
                self.regs[w] = (self.regs[w] & !wmask) | (value & wmask);
            }
        }
    }

    /// Internal: read a 32-bit register at the given word-aligned offset.
    fn read_word(&self, word_off: u64) -> u32 {
        match word_off {
            // W1TS/W1TC views read back the primary register's value.
            OUT | OUT_W1TS | OUT_W1TC => self.out,
            ENABLE | ENABLE_W1TS | ENABLE_W1TC => self.enable,
            IN => self.effective_input(0),
            IN1 => self.effective_input(1),
            // OUT1 and its W1TS/W1TC views read the behavioral bank-1 latch.
            OUT1 | OUT1_W1TS | OUT1_W1TC => self.out1,
            ENABLE1_W1TS | ENABLE1_W1TC => self.reg(ENABLE1),
            STATUS_W1TS | STATUS_W1TC => self.reg(STATUS),
            STATUS1_W1TS | STATUS1_W1TC => self.reg(STATUS1),
            // Everything else (incl. STRAP, IN1, PINn, FUNCn_*_SEL_CFG)
            // is served by the register file; holes read 0.
            off => self.reg(off),
        }
    }

    /// Internal: write a 32-bit value to the given word-aligned offset.
    fn write_word(&mut self, word_off: u64, value: u32) {
        match word_off {
            OUT => self.apply_out(value),
            OUT_W1TS => self.apply_out(self.out | value),
            OUT_W1TC => self.apply_out(self.out & !value),
            ENABLE => self.enable = value,
            ENABLE_W1TS => self.enable |= value,
            ENABLE_W1TC => self.enable &= !value,
            // The SVD marks IN/IN1.DATA_NEXT read-write: a write stores into
            // the same cell `set_pin_input` drives (the TRM documents the
            // registers as RO on silicon; firmware never writes them). It
            // asserts the whole bank as an external drive so the written word
            // reads back exactly, pull-ups included.
            IN => {
                self.external_levels[0] = value;
                self.external_drive_mask[0] = u32::MAX;
            }
            IN1 => {
                self.external_levels[1] = value & BANK1_MASK;
                self.external_drive_mask[1] = BANK1_MASK;
            }
            // OUT1 (bank-1 output latch) routes through apply_out1 so GPIO32..53
            // pad transitions fire observers, mirroring the bank-0 OUT path.
            OUT1 => self.apply_out1(value),
            OUT1_W1TS => self.apply_out1(self.out1 | value),
            OUT1_W1TC => self.apply_out1(self.out1 & !value),
            // Second-bank W1TS/W1TC arithmetic targets the primary register;
            // the spec wmask confines the effect to the architected bits.
            ENABLE1_W1TS => self.set_reg_masked(ENABLE1, self.reg(ENABLE1) | value),
            ENABLE1_W1TC => self.set_reg_masked(ENABLE1, self.reg(ENABLE1) & !value),
            STATUS_W1TS => self.set_reg_masked(STATUS, self.reg(STATUS) | value),
            STATUS_W1TC => self.set_reg_masked(STATUS, self.reg(STATUS) & !value),
            STATUS1_W1TS => self.set_reg_masked(STATUS1, self.reg(STATUS1) | value),
            STATUS1_W1TC => self.set_reg_masked(STATUS1, self.reg(STATUS1) & !value),
            // PIN0..31: masked storage + keep the behavioral int_type /
            // int_ena fields in sync (bits [9:7] / bit 13 per TRM §5.5).
            off @ PIN0..=PIN31 => {
                self.set_reg_masked(off, value);
                let stored = self.reg(off);
                let pin = ((off - PIN0) / 4) as usize;
                self.int_type[pin] = ((stored >> 7) & 0x7) as u8;
                if (stored >> 13) & 1 != 0 {
                    self.int_enable |= 1u32 << pin;
                } else {
                    self.int_enable &= !(1u32 << pin);
                }
            }
            // Everything else: masked store into the architected register;
            // RO registers (incl. STRAP) and holes ignore writes entirely.
            off => self.set_reg_masked(off, value),
        }
    }

    fn write_inner(&mut self, offset: u64, value: u8) -> SimResult<()> {
        let word_off = offset & !3;
        let byte_off = (offset & 3) * 8;
        // For W1TS, the existing word in the peripheral is read through
        // `read_word` which returns the primary register's value — so an
        // R-M-W byte write to OUT_W1TS at offset 0x08 byte 0 with value 0x04
        // becomes: word = OUT, word.byte0 = 0x04, then write_word(0x08, word)
        // sets bit 2 of OUT (OR-ing the current value back in is idempotent).
        // W1TC must merge against 0 instead: folding the current register
        // value into the unwritten bytes would clear every bit set there.
        let mut word = match word_off {
            OUT_W1TC | OUT1_W1TC | ENABLE_W1TC | ENABLE1_W1TC | STATUS_W1TC | STATUS1_W1TC => 0,
            off => self.read_word(off),
        };
        word &= !(0xFFu32 << byte_off);
        word |= (value as u32) << byte_off;
        self.write_word(word_off, word);
        Ok(())
    }
}

impl std::fmt::Debug for Esp32s3Gpio {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Esp32s3Gpio(enable=0x{:08x} out=0x{:08x} in=0x{:08x} cycle={} obs={})",
            self.enable,
            self.out,
            self.effective_input(0),
            self.cycle,
            self.observers.len(),
        )
    }
}

impl Default for Esp32s3Gpio {
    fn default() -> Self {
        Self::new()
    }
}

impl Peripheral for Esp32s3Gpio {
    // Inert walk: pin level changes are write-settled; tick() only advances an
    // observer cycle stamp. When the walk is deleted, stamps come from the
    // attached cycle clock (stamp_cycle). bus_tick consumers (RMT) use
    // drive_pad_output independently.
    fn needs_legacy_walk(&self) -> bool {
        false
    }

    fn attach_cycle_clock(&mut self, clock: CycleClock) {
        self.clock = Some(clock);
    }

    fn read(&self, offset: u64) -> SimResult<u8> {
        let word_off = offset & !3;
        let byte_off = (offset & 3) * 8;
        let word = self.read_word(word_off);
        Ok(((word >> byte_off) & 0xFF) as u8)
    }

    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        self.tap_snapshot();
        let result = self.write_inner(offset, value);
        self.tap_report();
        result
    }

    fn install_logic_tap(
        &mut self,
        tap: &crate::logic_capture::LogicTap,
        watched: &[(u8, u32)],
    ) -> bool {
        if watched.is_empty() {
            self.tap = None;
            self.pad_routes.clear_taps();
        } else {
            self.tap = Some(S3PortTap {
                tap: tap.clone(),
                watched: watched.to_vec(),
                scratch: vec![None; watched.len()],
            });
            // Seeded stale so the sync below always installs the current
            // routing into the wire.
            self.pad_routes.invalidate_registrations();
            self.sync_i2c_line_taps();
        }
        true
    }

    fn tick(&mut self) -> PeripheralTickResult {
        self.cycle = self.cycle.wrapping_add(1);
        PeripheralTickResult::default()
    }

    fn read_gpio_input(&self, pin: u8) -> Option<bool> {
        if pin >= 32 {
            return None;
        }
        Some((self.effective_input(0) & (1u32 << pin)) != 0)
    }

    fn read_gpio_output(&self, pin: u8) -> Option<bool> {
        if pin >= 32 {
            return None;
        }
        Some((self.out & (1u32 << pin)) != 0)
    }

    fn read_gpio_pad(&self, pin: u8) -> Option<bool> {
        if pin >= 32 {
            return None;
        }
        // A pad the output matrix hands to the I²C controller is driven by
        // that controller's wire, not by the GPIO_OUT latch — read the level
        // it publishes (see `crate::peripherals::pad_lines`). Without this the
        // pad reports the idle latch and an analyzer clipped here sees a flat
        // line while the bus is busy.
        if let Some(level) = self.pad_routes.level(pin, |p| self.out_sel(p)) {
            return Some(level);
        }
        let mask = 1u32 << pin;
        // ENABLE is the output driver: enabled pins show the OUT latch,
        // everything else shows the (externally driven) input level.
        Some(if (self.enable & mask) != 0 {
            (self.out & mask) != 0
        } else {
            (self.effective_input(0) & mask) != 0
        })
    }

    fn gpio_routing(&self, pin: u8) -> Option<crate::peripherals::gpio::GpioRouting> {
        use crate::peripherals::gpio::{GpioMode, GpioRouting};
        if pin >= PAD_COUNT {
            return None;
        }
        // Output driver enable lives in ENABLE (bank 0) / ENABLE1 (bank 1).
        let enabled = if pin < 32 {
            (self.enable & (1u32 << pin)) != 0
        } else {
            (self.reg(ENABLE1) & (1u32 << (pin - 32))) != 0
        };
        if !enabled {
            return Some(GpioRouting {
                mode: GpioMode::Input,
                func: None,
            });
        }
        // Output enabled: consult the GPIO-matrix output selector. The reset
        // sentinel (SIG_GPIO_OUT) means the pad is a plain GPIO output driven by
        // the OUT latch; any other index is a routed peripheral signal (e.g. an
        // RMT channel) → report it as an alternate function, naming it when known.
        match self.out_sel(pin) {
            Some(SIG_GPIO_OUT) | None => Some(GpioRouting {
                mode: GpioMode::Output,
                func: None,
            }),
            Some(sig) => Some(GpioRouting {
                mode: GpioMode::Af,
                func: s3_out_signal_name(sig).map(String::from),
            }),
        }
    }

    fn set_gpio_input(&mut self, pin: u8, level: bool) -> bool {
        if pin >= 32 {
            return false;
        }
        self.set_pin_input(pin, level);
        true
    }

    fn as_any(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }

    fn as_any_mut(&mut self) -> Option<&mut dyn std::any::Any> {
        Some(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    /// Simple recording observer for tests.
    #[derive(Debug, Default)]
    struct TestObserver {
        events: Mutex<Vec<(u8, bool, bool, u64)>>,
    }

    impl GpioObserver for TestObserver {
        fn on_pin_change(&self, pin: u8, from: bool, to: bool, sim_cycle: u64) {
            self.events.lock().unwrap().push((pin, from, to, sim_cycle));
        }
    }

    fn write_u32(g: &mut Esp32s3Gpio, off: u64, val: u32) {
        for byte in 0..4u64 {
            g.write(off + byte, ((val >> (byte * 8)) & 0xFF) as u8)
                .unwrap();
        }
    }

    fn read_u32(g: &Esp32s3Gpio, off: u64) -> u32 {
        let mut read = 0u32;
        for byte in 0..4u64 {
            read |= (g.read(off + byte).unwrap() as u32) << (byte * 8);
        }
        read
    }

    #[test]
    fn out_w1ts_sets_bit_and_fires_observer() {
        let mut g = Esp32s3Gpio::new();
        let obs = Arc::new(TestObserver::default());
        g.add_observer(obs.clone());

        // Write 0x04 (set bit 2) to OUT_W1TS at offset 0x08.
        // Use byte-level writes (the bus path).
        g.write(0x08, 0x04).unwrap();
        // Higher bytes are 0 — no-op, but they go through write_word too.
        g.write(0x09, 0x00).unwrap();
        g.write(0x0A, 0x00).unwrap();
        g.write(0x0B, 0x00).unwrap();

        assert_eq!(g.out & 0x04, 0x04, "OUT bit 2 must be set");
        let events = obs.events.lock().unwrap();
        assert!(
            events.iter().any(|&(p, f, t, _)| p == 2 && !f && t),
            "expected pin 2 0->1 transition; events: {events:?}"
        );
    }

    #[test]
    fn out_w1tc_clears_bit_and_fires_observer() {
        let mut g = Esp32s3Gpio::new();
        // Pre-set OUT bit 2.
        g.apply_out(0x04);
        let obs = Arc::new(TestObserver::default());
        g.add_observer(obs.clone());

        // Write 0x04 to OUT_W1TC at offset 0x0C.
        g.write(0x0C, 0x04).unwrap();
        g.write(0x0D, 0x00).unwrap();
        g.write(0x0E, 0x00).unwrap();
        g.write(0x0F, 0x00).unwrap();

        assert_eq!(g.out & 0x04, 0, "OUT bit 2 must be cleared");
        let events = obs.events.lock().unwrap();
        assert!(
            events.iter().any(|&(p, f, t, _)| p == 2 && f && !t),
            "expected pin 2 1->0 transition; events: {events:?}"
        );
    }

    #[test]
    fn direct_out_write_fires_observer_for_each_changed_bit() {
        let mut g = Esp32s3Gpio::new();
        let obs = Arc::new(TestObserver::default());
        g.add_observer(obs.clone());

        // Direct word-write to OUT setting bits 0, 5, 7 simultaneously.
        let val = (1u32 << 0) | (1u32 << 5) | (1u32 << 7);
        write_u32(&mut g, 0x04, val);

        let events = obs.events.lock().unwrap();
        let pins_set: Vec<u8> = events
            .iter()
            .filter(|&&(_, f, t, _)| !f && t)
            .map(|&(p, _, _, _)| p)
            .collect();
        assert!(pins_set.contains(&0), "pin 0 should have transitioned");
        assert!(pins_set.contains(&5), "pin 5 should have transitioned");
        assert!(pins_set.contains(&7), "pin 7 should have transitioned");
    }

    #[test]
    fn writing_same_value_does_not_fire_observer() {
        let mut g = Esp32s3Gpio::new();
        g.apply_out(0x04);
        let obs = Arc::new(TestObserver::default());
        g.add_observer(obs.clone());

        // Write OUT_W1TS bit 2 (already set).
        write_u32(&mut g, 0x08, 0x04);

        assert!(
            obs.events.lock().unwrap().is_empty(),
            "no observer event for unchanged bits"
        );
    }

    #[test]
    fn enable_w1ts_sets_enable_bit() {
        let mut g = Esp32s3Gpio::new();
        write_u32(&mut g, 0x24, 0x04);
        assert_eq!(g.enable & 0x04, 0x04);
    }

    #[test]
    fn enable_w1tc_clears_enable_bit() {
        let mut g = Esp32s3Gpio::new();
        g.enable = 0x04;
        write_u32(&mut g, 0x28, 0x04);
        assert_eq!(g.enable & 0x04, 0);
    }

    #[test]
    fn pin_reg_round_trips_int_type_and_int_ena() {
        let mut g = Esp32s3Gpio::new();
        // For pin 5: int_type = 3 (any-edge), int_ena (bit 13) = 1.
        // Word value: (3 << 7) | (1 << 13) = 0x180 | 0x2000 = 0x2180.
        let off = 0x74 + 5 * 4;
        let val = (3u32 << 7) | (1u32 << 13);
        write_u32(&mut g, off, val);
        assert_eq!(read_u32(&g, off), val, "PIN5_REG round-trip mismatch");
        assert_eq!(g.int_type[5], 3);
        assert_eq!(g.int_enable & (1u32 << 5), 1u32 << 5);
    }

    #[test]
    fn cycle_increments_each_tick_and_observer_sees_it() {
        let mut g = Esp32s3Gpio::new();
        let obs = Arc::new(TestObserver::default());
        g.add_observer(obs.clone());
        // Tick 5 times to advance cycle.
        for _ in 0..5 {
            g.tick();
        }
        // Now flip pin 0.
        write_u32(&mut g, 0x04, 0x01);

        let events = obs.events.lock().unwrap();
        let evt = events
            .iter()
            .find(|&&(p, _, _, _)| p == 0)
            .expect("pin 0 evt");
        assert_eq!(evt.3, 5, "cycle stamp must be 5 after 5 ticks");
    }

    #[test]
    fn multiple_observers_all_get_notified() {
        let mut g = Esp32s3Gpio::new();
        let a = Arc::new(TestObserver::default());
        let b = Arc::new(TestObserver::default());
        g.add_observer(a.clone());
        g.add_observer(b.clone());
        write_u32(&mut g, 0x08, 0x01);
        assert_eq!(a.events.lock().unwrap().len(), 1);
        assert_eq!(b.events.lock().unwrap().len(), 1);
    }

    // ── register-file slice ──────────────────────────────────────────────

    #[test]
    fn reset_defaults_seeded() {
        let g = Esp32s3Gpio::new();
        // STRAP keeps the silicon-captured SPI_FAST_FLASH_BOOT value.
        assert_eq!(read_u32(&g, STRAP), 0x0000_0008);
        // FUNCn_OUT_SEL_CFG resets to 0x100 (GPIO-matrix bypass) across the
        // whole array — spot-check first, middle, last members.
        assert_eq!(read_u32(&g, FUNC0_OUT_SEL_CFG), 0x0000_0100);
        assert_eq!(read_u32(&g, FUNC0_OUT_SEL_CFG + 26 * 4), 0x0000_0100);
        assert_eq!(read_u32(&g, FUNC53_OUT_SEL_CFG), 0x0000_0100);
        assert_eq!(read_u32(&g, CLOCK_GATE), 0x0000_0001);
        assert_eq!(read_u32(&g, REG_DATE), 0x0190_7040);
        // Zero-reset members of the arrays.
        assert_eq!(read_u32(&g, PIN53), 0);
        assert_eq!(read_u32(&g, FUNC255_IN_SEL_CFG), 0);
    }

    #[test]
    fn config_registers_store_under_write_mask() {
        let mut g = Esp32s3Gpio::new();
        // BT_SELECT is fully writable.
        write_u32(&mut g, BT_SELECT, 0x1234_5678);
        assert_eq!(read_u32(&g, BT_SELECT), 0x1234_5678);
        // SDIO_SELECT: only [7:0] writable.
        write_u32(&mut g, SDIO_SELECT, 0xFFFF_FFFF);
        assert_eq!(read_u32(&g, SDIO_SELECT), 0x0000_00FF);
        // PINn array members store only the architected bits (incl. n > 31).
        write_u32(&mut g, PIN53, 0xFFFF_FFFF);
        assert_eq!(read_u32(&g, PIN53), PIN_WMASK);
        // FUNCn_IN_SEL_CFG: [7:0] writable, array spot-checks.
        write_u32(&mut g, FUNC0_IN_SEL_CFG, 0xFFFF_FFFF);
        assert_eq!(read_u32(&g, FUNC0_IN_SEL_CFG), 0x0000_00FF);
        write_u32(&mut g, FUNC255_IN_SEL_CFG, 0xDEAD_BEA7);
        assert_eq!(read_u32(&g, FUNC255_IN_SEL_CFG), 0x0000_00A7);
        // FUNCn_OUT_SEL_CFG: [11:0] writable.
        write_u32(&mut g, FUNC53_OUT_SEL_CFG, 0xFFFF_FFFF);
        assert_eq!(read_u32(&g, FUNC53_OUT_SEL_CFG), 0x0000_0FFF);
        // CLOCK_GATE: bit 0 only.
        write_u32(&mut g, CLOCK_GATE, 0xFFFF_FFFE);
        assert_eq!(read_u32(&g, CLOCK_GATE), 0);
        write_u32(&mut g, CLOCK_GATE, 1);
        assert_eq!(read_u32(&g, CLOCK_GATE), 1);
    }

    #[test]
    fn bank1_out_enable_w1ts_w1tc_arithmetic() {
        let mut g = Esp32s3Gpio::new();
        // OUT1: set bits via W1TS, clear via W1TC; 22-bit mask applies.
        write_u32(&mut g, OUT1_W1TS, 0xFFFF_FFFF);
        assert_eq!(read_u32(&g, OUT1), BANK1_MASK);
        assert_eq!(read_u32(&g, OUT1_W1TS), BANK1_MASK, "W1TS reads OUT1");
        write_u32(&mut g, OUT1_W1TC, 0x0000_0005);
        assert_eq!(read_u32(&g, OUT1), BANK1_MASK & !0x5);
        write_u32(&mut g, OUT1, 0);
        assert_eq!(read_u32(&g, OUT1), 0);
        // ENABLE1 mirrors the same arithmetic.
        write_u32(&mut g, ENABLE1_W1TS, 0x0000_0030);
        assert_eq!(read_u32(&g, ENABLE1), 0x30);
        write_u32(&mut g, ENABLE1_W1TC, 0x0000_0010);
        assert_eq!(read_u32(&g, ENABLE1), 0x20);
        assert_eq!(read_u32(&g, ENABLE1_W1TC), 0x20, "W1TC reads ENABLE1");
    }

    #[test]
    fn status_w1ts_w1tc_arithmetic() {
        let mut g = Esp32s3Gpio::new();
        write_u32(&mut g, STATUS_W1TS, 0x8000_0001);
        assert_eq!(read_u32(&g, STATUS), 0x8000_0001);
        write_u32(&mut g, STATUS_W1TC, 0x8000_0000);
        assert_eq!(read_u32(&g, STATUS), 0x0000_0001);
        write_u32(&mut g, STATUS1_W1TS, 0xFFFF_FFFF);
        assert_eq!(read_u32(&g, STATUS1), BANK1_MASK);
        write_u32(&mut g, STATUS1_W1TC, BANK1_MASK);
        assert_eq!(read_u32(&g, STATUS1), 0);
    }

    #[test]
    fn in_write_stores_into_input_cell_per_svd_access() {
        let mut g = Esp32s3Gpio::new();
        // SVD marks IN.DATA_NEXT read-write: writes land in the same cell
        // set_pin_input drives, so read-back stays coherent.
        write_u32(&mut g, IN, 0x0000_00F0);
        assert_eq!(read_u32(&g, IN), 0x0000_00F0);
        g.set_pin_input(0, true);
        assert_eq!(read_u32(&g, IN), 0x0000_00F1);
        // IN1 stores under the 22-bit second-bank mask.
        write_u32(&mut g, IN1, 0xFFFF_FFFF);
        assert_eq!(read_u32(&g, IN1), BANK1_MASK);
    }

    #[test]
    fn read_only_registers_ignore_writes() {
        let mut g = Esp32s3Gpio::new();
        // STRAP keeps its silicon-captured value.
        write_u32(&mut g, STRAP, 0xFFFF_FFFF);
        assert_eq!(read_u32(&g, STRAP), 0x0000_0008);
        // Interrupt-status mirrors are RO and stay 0.
        for off in [
            0x5C,
            0x60,
            0x64,
            0x68,
            0x6C,
            0x70,
            STATUS_NEXT,
            STATUS_NEXT1,
        ] {
            write_u32(&mut g, off, 0xFFFF_FFFF);
            assert_eq!(read_u32(&g, off), 0, "RO reg at {off:#x}");
        }
    }

    // ── RMT Stage 1: output pad edges reach the observer ─────────────────

    /// Firmware-path proof: drive an output pad high→low→high the way firmware
    /// would (byte writes to OUT_W1TS / OUT_W1TC, exactly the bus write path),
    /// ticking the peripheral between edges, and assert the observer captured
    /// the EXACT edge sequence WITH correct per-edge timing (sim_cycle stamps).
    #[test]
    fn output_register_edges_reach_observer_with_timing() {
        let mut g = Esp32s3Gpio::new();
        let obs = Arc::new(TestObserver::default());
        g.add_observer(obs.clone());

        // cycle 0: nothing. Advance to cycle 10, then rising edge on pin 4.
        for _ in 0..10 {
            g.tick();
        }
        write_u32(&mut g, OUT_W1TS, 1 << 4); // pin 4: 0->1 @ cycle 10

        // Advance to cycle 25, falling edge.
        for _ in 0..15 {
            g.tick();
        }
        write_u32(&mut g, OUT_W1TC, 1 << 4); // pin 4: 1->0 @ cycle 25

        // Advance to cycle 40, rising again.
        for _ in 0..15 {
            g.tick();
        }
        write_u32(&mut g, OUT_W1TS, 1 << 4); // pin 4: 0->1 @ cycle 40

        let events = obs.events.lock().unwrap();
        assert_eq!(
            *events,
            vec![
                (4, false, true, 10),
                (4, true, false, 25),
                (4, false, true, 40)
            ],
            "exact pin-4 edge sequence with timing; got {events:?}"
        );
    }

    /// Peripheral-path proof (RMT Stage 1 seam): a `drive_pad_output` caller —
    /// standing in for a future timed RMT playback engine bit-banging a WS2812
    /// line — flips pad 6 through a short bit pattern, and the SAME observer
    /// mechanism captures the exact edge sequence with timing.
    #[test]
    fn drive_pad_output_edges_reach_observer_with_timing() {
        let mut g = Esp32s3Gpio::new();
        let obs = Arc::new(TestObserver::default());
        g.add_observer(obs.clone());

        // A 3-bit "waveform" on pin 6 at cycles 5, 8, 12.
        for _ in 0..5 {
            g.tick();
        }
        assert!(g.drive_pad_output(6, true)); // 0->1 @ 5
        for _ in 0..3 {
            g.tick();
        }
        assert!(g.drive_pad_output(6, false)); // 1->0 @ 8
        for _ in 0..4 {
            g.tick();
        }
        assert!(g.drive_pad_output(6, true)); // 0->1 @ 12

        // Pad state is observable back through the output accessor.
        assert_eq!(g.read_gpio_output(6), Some(true));

        let events = obs.events.lock().unwrap();
        assert_eq!(
            *events,
            vec![
                (6, false, true, 5),
                (6, true, false, 8),
                (6, false, true, 12)
            ],
            "exact pin-6 edge sequence with timing; got {events:?}"
        );
    }

    /// The peripheral seam must not double-fire on a redundant level: driving a
    /// pad to the level it already holds is a no-op (WS2812 encoders emit long
    /// same-level runs — those must not spam observers).
    #[test]
    fn drive_pad_output_same_level_does_not_fire() {
        let mut g = Esp32s3Gpio::new();
        g.drive_pad_output(3, true);
        let obs = Arc::new(TestObserver::default());
        g.add_observer(obs.clone());
        assert!(g.drive_pad_output(3, true)); // already high
        assert!(
            obs.events.lock().unwrap().is_empty(),
            "no observer event for an unchanged pad level"
        );
    }

    /// Out-of-range pins (> 48) are rejected without touching state/observers;
    /// pin 48 (onboard NeoPixel) is accepted.
    #[test]
    fn drive_pad_output_rejects_pins_above_48() {
        let mut g = Esp32s3Gpio::new();
        let obs = Arc::new(TestObserver::default());
        g.add_observer(obs.clone());
        assert!(!g.drive_pad_output(49, true));
        assert!(!g.drive_pad_output(200, false));
        assert!(g.drive_pad_output(48, true), "pin 48 is a valid bank-1 pad");
        // Only the pin-48 edge should be recorded.
        let events = obs.events.lock().unwrap();
        assert_eq!(*events, vec![(48, false, true, 0)]);
    }

    /// Bank-1 (GPIO32..53) proof: a GPIO48 edge reaches the observer both via an
    /// OUT1 register write AND via `drive_pad_output`, with correct timing —
    /// this is the onboard-NeoPixel pin on most S3 boards.
    #[test]
    fn bank1_gpio48_edges_reach_observer_with_timing() {
        // Route A: OUT1 register writes (bit 16 = pin 48).
        let mut g = Esp32s3Gpio::new();
        let obs = Arc::new(TestObserver::default());
        g.add_observer(obs.clone());
        for _ in 0..7 {
            g.tick();
        }
        write_u32(&mut g, OUT1_W1TS, 1 << 16); // pin 48: 0->1 @ cycle 7
        for _ in 0..5 {
            g.tick();
        }
        write_u32(&mut g, OUT1_W1TC, 1 << 16); // pin 48: 1->0 @ cycle 12
        assert_eq!(read_u32(&g, OUT1), 0, "OUT1 reads back the bank-1 latch");
        assert_eq!(
            *obs.events.lock().unwrap(),
            vec![(48, false, true, 7), (48, true, false, 12)],
            "OUT1 register write must fire GPIO48 observer edges with timing"
        );

        // Route B: peripheral drive via drive_pad_output.
        let mut g2 = Esp32s3Gpio::new();
        let obs2 = Arc::new(TestObserver::default());
        g2.add_observer(obs2.clone());
        for _ in 0..3 {
            g2.tick();
        }
        assert!(g2.drive_pad_output(48, true)); // 0->1 @ 3
        for _ in 0..9 {
            g2.tick();
        }
        assert!(g2.drive_pad_output(48, false)); // 1->0 @ 12
        assert_eq!(read_u32(&g2, OUT1), 0);
        assert_eq!(
            *obs2.events.lock().unwrap(),
            vec![(48, false, true, 3), (48, true, false, 12)],
            "drive_pad_output must fire GPIO48 observer edges with timing"
        );
    }

    // ── Task 2: GPIO-matrix output routing (FUNCn_OUT_SEL) ───────────────

    /// The output-matrix lookup resolves which pad(s) a peripheral output signal
    /// drives: route GPIO48 (and a fan-out pad) to RMT channel 0 and assert the
    /// lookup finds them. This is what a Stage-2 RMT channel calls to learn its
    /// pad instead of trusting board config.
    #[test]
    fn func_out_sel_routes_rmt_signal_to_pad() {
        let mut g = Esp32s3Gpio::new();
        // Reset: no pad routes an RMT signal (all sit at SIG_GPIO_OUT).
        assert!(g.pads_for_output_signal(RMT_SIG_OUT0).is_empty());
        // Route GPIO48 (onboard NeoPixel) to RMT channel 0's output signal.
        write_u32(&mut g, FUNC0_OUT_SEL_CFG + 48 * 4, RMT_SIG_OUT0);
        assert_eq!(g.pads_for_output_signal(RMT_SIG_OUT0), vec![48]);
        // A signal can fan out to multiple pads; results are pad-ascending.
        write_u32(&mut g, FUNC0_OUT_SEL_CFG + 5 * 4, RMT_SIG_OUT0);
        assert_eq!(g.pads_for_output_signal(RMT_SIG_OUT0), vec![5, 48]);
        // Other RMT channels stay unrouted.
        assert!(g.pads_for_output_signal(RMT_SIG_OUT1).is_empty());
    }

    /// `gpio_routing` now reads the output matrix: a routed RMT pad reports
    /// `Af` + the signal name, an enabled-but-unrouted pad reports plain
    /// `Output`, and a disabled pad reports `Input` — across both banks.
    #[test]
    fn gpio_routing_reports_rmt_af_and_plain_gpio() {
        use crate::peripherals::gpio::GpioMode;
        let mut g = Esp32s3Gpio::new();
        // Enable GPIO48 output (ENABLE1 bit 16) and route it to RMT_SIG_OUT2.
        write_u32(&mut g, ENABLE1_W1TS, 1 << 16);
        write_u32(&mut g, FUNC0_OUT_SEL_CFG + 48 * 4, RMT_SIG_OUT2);
        let r48 = g.gpio_routing(48).unwrap();
        assert_eq!(r48.mode, GpioMode::Af);
        assert_eq!(r48.func.as_deref(), Some("RMT_SIG_OUT2"));
        // Enable GPIO5 output, leave the reset selector → plain GPIO output.
        write_u32(&mut g, ENABLE_W1TS, 1 << 5);
        let r5 = g.gpio_routing(5).unwrap();
        assert_eq!(r5.mode, GpioMode::Output);
        assert!(r5.func.is_none(), "unrouted enabled pad is plain GPIO");
        // Disabled pad → Input; out-of-range → None.
        assert_eq!(g.gpio_routing(6).unwrap().mode, GpioMode::Input);
        assert!(g.gpio_routing(54).is_none());
    }

    // ── IO_MUX pull-up (Arduino INPUT_PULLUP) ─────────────────────────────
    //
    // Mirrors the ESP32-C3's `input_pullup_*` tests. Measured defect before
    // this landed: on an S3, `pinMode(p, INPUT_PULLUP); digitalRead(p)`
    // returned 0 with nothing wired to the pin (real silicon reads 1), so
    // every button-to-GND lab read permanently pressed.

    /// Absolute addresses on the S3 memory map, so the test exercises the same
    /// MMIO path firmware does.
    const GPIO_BASE: u64 = 0x6000_4000;
    const IO_MUX_BASE: u64 = 0x6000_9000;
    /// `IO_MUX_GPIOn_REG` = base + 0x04 + n*4 (SVD `GPIO%s`, dim 49).
    fn io_mux_gpio(pin: u64) -> u64 {
        IO_MUX_BASE + 0x04 + pin * 4
    }
    /// Arduino-core pad words: `FUN_DRV=2 | FUN_IE | MCU_SEL=GPIO`, with and
    /// without `FUN_WPU` (bit 8).
    const PAD_INPUT: u32 = 0x0000_1A00;
    const PAD_INPUT_PULLUP: u32 = 0x0000_1B00;

    /// Build the S3 the way the firmware-execution path does — the coded
    /// `configure_xtensa_esp32s3`, which is where the IO_MUX is registered.
    fn s3_bus() -> crate::bus::SystemBus {
        let mut bus = crate::bus::SystemBus::new();
        crate::system::xtensa::configure_xtensa_esp32s3(
            &mut bus,
            &crate::system::xtensa::Esp32s3Opts::default(),
        );
        bus
    }

    fn with_gpio<R>(bus: &mut crate::bus::SystemBus, f: impl FnOnce(&mut Esp32s3Gpio) -> R) -> R {
        let idx = bus
            .find_peripheral_index_by_name("gpio")
            .expect("S3 GPIO is present");
        let gpio = bus.peripherals[idx]
            .dev
            .as_any_mut()
            .and_then(|any| any.downcast_mut::<Esp32s3Gpio>())
            .expect("S3 GPIO model");
        f(gpio)
    }

    /// The headline behaviour: a pad with `FUN_WPU` set and nothing driving it
    /// reads 1; an external drive (a button to GND) still wins.
    #[test]
    fn esp32s3_input_pullup_releases_a_floating_pin_but_external_drive_wins() {
        use crate::Bus;
        let mut bus = s3_bus();

        // Cold reset already sets FUN_WPU (SVD GPIO%s reset = 0x0000_0B00).
        assert_eq!(bus.read_u32(io_mux_gpio(5)).unwrap(), 0x0000_0B00);
        with_gpio(&mut bus, |gpio| {
            assert_eq!(
                gpio.read_gpio_input(5),
                Some(true),
                "the cold IO_MUX FUN_WPU bit releases a floating GPIO5"
            );
            assert_eq!(gpio.read_gpio_pad(5), Some(true));
        });

        // pinMode(5, INPUT) clears FUN_WPU → the pad no longer reads high.
        bus.write_u32(io_mux_gpio(5), PAD_INPUT)
            .expect("emulate Arduino pinMode(GPIO5, INPUT)");
        with_gpio(&mut bus, |gpio| {
            assert_eq!(gpio.read_gpio_input(5), Some(false), "INPUT clears FUN_WPU");
            assert_eq!(gpio.read_gpio_pad(5), Some(false));
        });

        // pinMode(5, INPUT_PULLUP) sets it again → released pin reads 1.
        bus.write_u32(io_mux_gpio(5), PAD_INPUT_PULLUP)
            .expect("emulate Arduino pinMode(GPIO5, INPUT_PULLUP)");
        with_gpio(&mut bus, |gpio| {
            assert_eq!(
                gpio.read_gpio_input(5),
                Some(true),
                "pull-up releases GPIO5 high"
            );
            assert_eq!(gpio.read_gpio_pad(5), Some(true));
        });

        // A button pulling the pin to GND is an external drive and beats the
        // weak pull-up; releasing it drives high again.
        with_gpio(&mut bus, |gpio| {
            assert!(gpio.set_gpio_input(5, false));
            assert_eq!(
                gpio.read_gpio_input(5),
                Some(false),
                "an external drive beats the weak internal pull-up"
            );
            assert!(gpio.set_gpio_input(5, true));
            assert_eq!(gpio.read_gpio_input(5), Some(true));
        });
    }

    /// What `digitalRead` actually executes: a load from `GPIO_IN`. This is the
    /// register the measured sketch read as 0.
    #[test]
    fn esp32s3_input_pullup_is_visible_in_the_gpio_in_register() {
        use crate::Bus;
        let mut bus = s3_bus();

        // GPIO6 unconnected + INPUT_PULLUP, GPIO7 unconnected + INPUT.
        bus.write_u32(io_mux_gpio(6), PAD_INPUT_PULLUP).unwrap();
        bus.write_u32(io_mux_gpio(7), PAD_INPUT).unwrap();

        let in_word = bus.read_u32(GPIO_BASE + IN).unwrap();
        assert_ne!(
            in_word & (1 << 6),
            0,
            "NOWIRE-PU6 must read 1: FUN_WPU with nothing driving the pad"
        );
        assert_eq!(
            in_word & (1 << 7),
            0,
            "FLOAT7 has no pull-up and nothing driving it"
        );

        // A button on GPIO5 to GND, held closed, with INPUT_PULLUP.
        bus.write_u32(io_mux_gpio(5), PAD_INPUT_PULLUP).unwrap();
        with_gpio(&mut bus, |gpio| assert!(gpio.set_gpio_input(5, false)));
        assert_eq!(
            bus.read_u32(GPIO_BASE + IN).unwrap() & (1 << 5),
            0,
            "a closed button to GND still reads 0 through GPIO_IN"
        );
    }

    /// Bank 1 (`GPIO_IN1`, pads 32..48) goes through the same rule. Pads 49..53
    /// have no `IO_MUX_GPIOn_REG` on the S3 and therefore never float high.
    #[test]
    fn esp32s3_input_pullup_reaches_bank_one_through_in1() {
        use crate::Bus;
        let mut bus = s3_bus();

        let in1 = bus.read_u32(GPIO_BASE + IN1).unwrap();
        assert_ne!(in1 & (1 << (47 - 32)), 0, "GPIO47 floats high at reset");
        assert_ne!(in1 & (1 << (48 - 32)), 0, "GPIO48 floats high at reset");
        assert_eq!(
            in1 & !0x0001_FFFF,
            0,
            "GPIO49..53 have no IO_MUX pad word, so no pull-up"
        );

        bus.write_u32(io_mux_gpio(47), PAD_INPUT).unwrap();
        let in1 = bus.read_u32(GPIO_BASE + IN1).unwrap();
        assert_eq!(in1 & (1 << (47 - 32)), 0, "INPUT clears GPIO47's FUN_WPU");
        assert_ne!(in1 & (1 << (48 - 32)), 0, "GPIO48 is untouched");
    }

    /// Direction-aware: an enabled output driver shows the value it drives, not
    /// the pad's weak pull-up.
    #[test]
    fn esp32s3_enabled_output_driver_beats_the_pad_pullup() {
        use crate::Bus;
        let mut bus = s3_bus();
        bus.write_u32(io_mux_gpio(8), PAD_INPUT_PULLUP).unwrap();
        with_gpio(&mut bus, |gpio| {
            assert_eq!(gpio.read_gpio_pad(8), Some(true), "released, pulled up");
        });

        // pinMode(8, OUTPUT); digitalWrite(8, LOW).
        bus.write_u32(GPIO_BASE + ENABLE_W1TS, 1 << 8).unwrap();
        bus.write_u32(GPIO_BASE + OUT_W1TC, 1 << 8).unwrap();
        with_gpio(&mut bus, |gpio| {
            assert_eq!(
                gpio.read_gpio_pad(8),
                Some(false),
                "an enabled output driver shows the driven value"
            );
        });
        bus.write_u32(GPIO_BASE + OUT_W1TS, 1 << 8).unwrap();
        with_gpio(&mut bus, |gpio| {
            assert_eq!(gpio.read_gpio_pad(8), Some(true))
        });
    }

    /// No IO_MUX on the bus → no pull-ups, exactly as before this change. This
    /// is the guard that the new term cannot invent a high level on a port that
    /// was never wired to a pad-control bank.
    #[test]
    fn esp32s3_gpio_without_io_mux_wiring_reads_undriven_pads_low() {
        let mut g = Esp32s3Gpio::new();
        assert_eq!(g.read_gpio_input(5), Some(false));
        assert_eq!(g.read_gpio_pad(5), Some(false));
        assert_eq!(read_u32(&g, IN), 0);
        assert_eq!(read_u32(&g, IN1), 0);
        g.set_pin_input(5, true);
        assert_eq!(g.read_gpio_input(5), Some(true));
    }

    /// A `FUN_WPU` write after an external drive was asserted must NOT undo the
    /// drive — the released level only applies to pads nothing is driving.
    #[test]
    fn esp32s3_pullup_write_does_not_override_a_standing_external_drive() {
        use crate::Bus;
        let mut bus = s3_bus();
        bus.write_u32(io_mux_gpio(9), PAD_INPUT).unwrap();
        with_gpio(&mut bus, |gpio| {
            assert!(gpio.set_gpio_input(9, false));
            assert_eq!(gpio.read_gpio_input(9), Some(false));
        });
        bus.write_u32(io_mux_gpio(9), PAD_INPUT_PULLUP).unwrap();
        with_gpio(&mut bus, |gpio| {
            assert_eq!(
                gpio.read_gpio_input(9),
                Some(false),
                "the wire still holds GPIO9 low"
            );
        });
    }

    #[test]
    fn unmapped_offsets_read_zero_and_ignore_writes() {
        let mut g = Esp32s3Gpio::new();
        // The 0x630..0x6F8 hole and offsets at/above 0x700 must NOT
        // round-trip — the coverage probe's baseline depends on it.
        for off in [0x630u64, 0x680, 0x6F8, 0x700, 0x7FC] {
            write_u32(&mut g, off, 0xDEAD_BEEF);
            assert_eq!(read_u32(&g, off), 0, "hole at {off:#x}");
        }
    }
}
