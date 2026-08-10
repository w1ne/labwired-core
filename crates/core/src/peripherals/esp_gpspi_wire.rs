// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! The ONE way an Espressif GP-SPI master puts a real waveform on its pads.
//!
//! # Why this is shared
//!
//! The ESP32-C3 `SPI2` (FSPI) and the ESP32-S3 `SPI2`/`SPI3` are the same
//! Espressif GP-SPI IP. Every register this module reads sits at the same
//! offset with the same bit positions on both parts, verified field by field
//! against the local esp-idf headers:
//!
//! | field | reg | bits | C3 `spi_reg.h` | S3 `spi_reg.h` |
//! |---|---|---|---|---|
//! | `SPI_CLK_EQU_SYSCLK` | CLOCK 0x0C | 31 | :140 | :169 |
//! | `SPI_CLKDIV_PRE` | CLOCK 0x0C | 21:18 | :147 | :176 |
//! | `SPI_CLKCNT_N` | CLOCK 0x0C | 17:12 | :154 | :183 |
//! | `SPI_CK_OUT_EDGE` (CPHA) | USER 0x10 | 9 | :262 | :298 |
//! | `SPI_CK_IDLE_EDGE` (CPOL) | MISC 0x20 | 29 | :398 | :441 |
//!
//! (paths under
//! `framework-arduinoespressif32/tools/sdk/<chip>/include/soc/<chip>/include/soc/`.)
//!
//! The two controllers keep their own register files, their own transaction
//! engines and their own interrupt-matrix ids — everything that genuinely
//! differs. What they share is this: how a completed transaction becomes edges.
//! Two copies of it would be two places to get MSB-order or the CS framing
//! wrong, and only one of them would be under test.
//!
//! # The classic ESP32 shares the machinery and NOT the decode
//!
//! `peripherals::esp32::spi::Esp32Spi` (the LX6 part's SPI0/1/HSPI/VSPI) is also
//! transaction-level and also holds an [`EspSpiWire`], because the buffering,
//! the burst pacing and the CS framing are the same problem. Its REGISTERS are
//! not: `SPI_CLOCK` is at 0x18 rather than 0x0C, `CLKDIV_PRE` is thirteen bits
//! rather than four, `SPI_CK_OUT_EDGE` (CPHA) is `USER` bit **7** rather than
//! bit 9, and there is no `SPI_MISC` at all — `SPI_CK_IDLE_EDGE` (CPOL) lives in
//! `SPI_PIN` at 0x34. So the classic model keeps its own `bit_time_cycles` and
//! `framing` and calls only [`EspSpiWire::push`] / [`EspSpiWire::flush`] /
//! [`EspSpiWire::ready_in`] here. The free functions below are C3/S3 ONLY;
//! calling them with a classic register would read three fields out of the wrong
//! bits of the wrong words.
//!
//! # What the caller owes this type
//!
//! * Call [`EspSpiWire::pad_lines_arc`] once at bus wiring time, AFTER the GPIO
//!   port has been resolved — creating the cell is what turns narration on, and
//!   a controller narrating into a wire no pad reaches is pure cost.
//! * Call [`EspSpiWire::push`] with each MOSI byte at the moment the
//!   transaction engine puts it on the bus.
//! * Call [`EspSpiWire::flush`] from a wakeup — `tick()` on the legacy walk, an
//!   event chain under the scheduler — so a burst is published once the wire has
//!   had time to carry it.
//!
//! # What is NOT bound
//!
//! MISO. The S3's W-buffer path fills the MISO region with a constant `0xFF`
//! without consulting an attached device at all, so a bound MISO pad would
//! report a confident idle level while a display or flash was supposedly
//! answering — worse than the GPIO-latch fallback it would replace, because it
//! looks authoritative. Same rule that keeps `wire_rp2040_uart_pads` TX-only.
//! MISO joins the table when the W-buffer path genuinely exchanges, not before.

use std::sync::Arc;

use super::pad_lines::PadLines;
use super::spi_waveform::{NarrationFit, SpiFraming, SpiNarrator};

/// APB clock the GP-SPI divisors count. 80 MHz on both parts — the same source
/// clock `esp_uart` scales its baud divisor against.
pub const APB_CLK_HZ: u64 = 80_000_000;

/// The pad lines a GP-SPI master DRIVES, in the order a lab probes them.
/// See the module header for why MISO is absent.
pub const SPI_LINES: &[&str] = &["SCK", "MOSI", "CS"];
pub const LINE_SCK: usize = 0;
pub const LINE_MOSI: usize = 1;
pub const LINE_CS: usize = 2;

/// Bytes a narration may hold waiting for the wire before it is published
/// anyway, compressed. Deeper than the 64-byte W buffer, so a burst reaches
/// this only when firmware is launching transactions faster than the rate it
/// programmed can carry them.
const WIRE_BURST_CAP: usize = 256;

/// `SPI_CLK_EQU_SYSCLK` — SPI clock equals the APB clock, divisors bypassed.
const CLK_EQU_SYSCLK: u32 = 1 << 31;
/// `SPI_CLKDIV_PRE` [21:18].
const CLKDIV_PRE_SHIFT: u32 = 18;
const CLKDIV_PRE_MASK: u32 = 0xF;
/// `SPI_CLKCNT_N` [17:12].
const CLKCNT_N_SHIFT: u32 = 12;
const CLKCNT_N_MASK: u32 = 0x3F;
/// `SPI_CK_OUT_EDGE` in `SPI_USER` — CPHA.
const CK_OUT_EDGE: u32 = 1 << 9;
/// `SPI_CK_IDLE_EDGE` in `SPI_MISC` — CPOL.
const CK_IDLE_EDGE: u32 = 1 << 29;

/// Engine cycles in one SCK period, from the controller's own `SPI_CLOCK`.
///
/// `f_spi = f_apb / ((CLKDIV_PRE + 1) * (CLKCNT_N + 1))`, or `f_apb` outright
/// when `CLK_EQU_SYSCLK` is set (the reset state, and what
/// `spi_ll_master_set_clock` programs for an 80 MHz request). Scaled from APB
/// ticks into CPU cycles by `cpu_clock_hz / APB_CLK_HZ`, because the engine's
/// cycle axis is CPU cycles — the same conversion `EspUart::cycles_per_byte`
/// applies to `CLKDIV`, and exact on both parts (160 and 240 MHz are whole
/// multiples of 80).
///
/// `None` below two cycles per bit, where [`super::wave_plan::WavePlan`] cannot
/// keep a period's halves distinct and nothing honest can be drawn.
pub fn bit_time_cycles(clock_reg: u32, cpu_clock_hz: u64) -> Option<u64> {
    let apb_ticks = if clock_reg & CLK_EQU_SYSCLK != 0 {
        1
    } else {
        let pre = u64::from((clock_reg >> CLKDIV_PRE_SHIFT) & CLKDIV_PRE_MASK) + 1;
        let n = u64::from((clock_reg >> CLKCNT_N_SHIFT) & CLKCNT_N_MASK) + 1;
        pre * n
    };
    let ticks = apb_ticks * cpu_clock_hz / APB_CLK_HZ;
    (ticks >= 2).then_some(ticks)
}

/// How `SPI_MISC` and `SPI_USER` currently frame a byte on the wire.
///
/// Width is fixed at 8: the GP-SPI's `SPI_MS_DLEN` counts the transaction in
/// BITS and the transaction engines above walk the W buffer a byte at a time,
/// so a byte is the unit that reaches this wire. `SPI_WR_BIT_ORDER` (CTRL bit
/// 26 on the C3, [26:25] on the S3) selects LSB-first and is NOT read here —
/// the narrator only draws MSB-first, which is what every ESP-IDF path
/// programs; a firmware that flipped it would get a trace at the right rate
/// with the bits reversed, so it is called out rather than silently assumed.
pub fn framing(misc_reg: u32, user_reg: u32) -> SpiFraming {
    SpiFraming {
        cpol: misc_reg & CK_IDLE_EDGE != 0,
        cpha: user_reg & CK_OUT_EDGE != 0,
        bits: 8,
    }
}

/// The narration state one GP-SPI controller carries.
#[derive(Debug, Default)]
pub struct EspSpiWire {
    /// Levels published to matrix-routed pads. `None` — the common case, no lab
    /// routed an SPI pad — costs one branch per transaction and publishes
    /// nothing.
    lines: Option<Arc<PadLines>>,
    /// Bytes shifted since the last flush, each with the framing the registers
    /// held AT THE MOMENT it went out. Carried per byte rather than re-read at
    /// flush time so firmware that reprograms CPOL/CPHA between transactions
    /// still narrates each byte the way it actually went.
    words: Vec<(u8, SpiFraming)>,
    /// Bit period the held burst is narrated at, captured on its first byte. A
    /// rate change mid-burst force-flushes what is held rather than repainting
    /// it at a rate no transaction used.
    bit_time: u64,
    /// Cycle the last narration ran to — the floor the next may not reach back
    /// past, or two bursts splice into bytes neither transaction sent.
    wave_cursor: u64,
    /// True while a flush wakeup is in flight, so a run of transactions arms one
    /// chain rather than one per transaction.
    pub scheduled: bool,
    /// Monotonic token so a stale wakeup from a superseded arm is ignored.
    pub arm_seq: u32,
}

impl EspSpiWire {
    /// The shared pad-line cell for SCK/MOSI/CS, created on first use.
    ///
    /// SCK idles at the programmed polarity, MOSI low, chip select RELEASED —
    /// an idle SPI bus reads CS high, and a model that idled it low would frame
    /// a transaction that never happened.
    pub fn pad_lines_arc(&mut self, cpol: bool) -> Arc<PadLines> {
        self.lines
            .get_or_insert_with(|| Arc::new(PadLines::new(SPI_LINES, &[cpol, false, true])))
            .clone()
    }

    /// `true` once a lab has routed pads to this controller.
    pub fn is_bound(&self) -> bool {
        self.lines.is_some()
    }

    /// The wire this controller publishes, for a
    /// [`LogicSource::Wire`](crate::logic_capture::LogicSource::Wire) channel.
    /// `None` until a lab routes pads, because that is when the cell is
    /// created and until then nothing is narrated into it either.
    pub fn wire_lines(&self) -> Option<&PadLines> {
        self.lines.as_deref()
    }

    /// `true` while a burst is held and unpublished.
    pub fn is_pending(&self) -> bool {
        !self.words.is_empty()
    }

    /// Queue one MOSI byte. Buffered, not published — see [`Self::flush`]. No
    /// routed pads or no usable bit period ⇒ nothing to narrate, and the call
    /// costs one branch.
    pub fn push(&mut self, byte: u8, framing: SpiFraming, bit_time: Option<u64>, now: u64) {
        if self.lines.is_none() {
            return;
        }
        let Some(bit_time) = bit_time else {
            return;
        };
        if !self.words.is_empty() && bit_time != self.bit_time {
            self.flush(now, true);
        }
        if self.words.is_empty() {
            self.bit_time = bit_time;
        }
        self.words.push((byte, framing));
    }

    /// Cycles until the held burst has had its wire time — 0 when it is due now,
    /// when nothing is held, or when it has hit the cap and must be published
    /// compressed rather than held longer.
    ///
    /// Both the pacing test and the scheduler deadline, computed the ONE way, so
    /// a wakeup can never land before the burst is publishable (a wasted wakeup)
    /// or after it (a late trace).
    pub fn ready_in(&self, now: u64) -> u64 {
        if self.words.is_empty() || self.words.len() >= WIRE_BURST_CAP {
            return 0;
        }
        let duration: u64 = self
            .words
            .iter()
            .map(|(_, framing)| framing.frame_bits() * self.bit_time)
            .sum();
        self.wave_cursor
            .saturating_add(duration)
            .saturating_sub(now)
    }

    /// Publish the held bytes onto the routed pads, once the wire has had time
    /// to carry them.
    ///
    /// The transaction engines above run a whole `SPI_CMD.USR` launch inside one
    /// MMIO write and clear `USR` immediately, so firmware hands this model a
    /// 64-byte buffer within a few cycles. The WIRE cannot do that — 64 bytes at
    /// the arduino-esp32 default of 1 MHz take ~102 000 CPU cycles on a 160 MHz
    /// C3 — and the capture layer (`LogicTap::push_at` →
    /// `LogicCapture::ingest_push`) accepts stamps in the PAST only, keeping one
    /// level per channel per cycle. There is nowhere to put a byte that has not
    /// yet had time to cross.
    ///
    /// So the burst accumulates and is narrated as one waveform ending at the
    /// present cycle. Holding until `now` has passed its wire time is what makes
    /// the common case EXACT: the trace then carries every byte at the rate
    /// `SPI_CLOCK` programs.
    ///
    /// `force` publishes regardless — the cap path and the rate-change path. The
    /// burst is then compressed: the bytes stay readable, the timebase does not.
    pub fn flush(&mut self, now: u64, force: bool) {
        if self.words.is_empty() {
            return;
        }
        let Some(lines) = self.lines.clone() else {
            self.words.clear();
            return;
        };
        if !force && self.ready_in(now) > 0 {
            return;
        }
        let mut narrator = SpiNarrator::with_lines(
            LINE_SCK,
            LINE_MOSI,
            Some(LINE_CS),
            &[
                lines.level(LINE_SCK),
                lines.level(LINE_MOSI),
                lines.level(LINE_CS),
            ],
            self.bit_time,
        );
        for &(byte, framing) in &self.words {
            narrator.frame(u16::from(byte), framing);
        }
        if let NarrationFit::LevelsOnly { .. } =
            narrator.emit_between(&lines, self.wave_cursor, now)
        {
            // Fewer cycles exist than the waveform has transitions, so nothing
            // was drawn. Keep the bytes and the cursor: `now` only grows, so a
            // later wakeup will have the room. Clearing here would delete bytes
            // that really crossed the bus and advance the cursor past cycles
            // nothing painted — silent, unrecoverable loss, and the reason
            // `emit_between` is `#[must_use]`.
            return;
        }
        self.wave_cursor = now;
        self.words.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The reset `SPI_CLOCK` (0x8000_3043 on both parts) has
    /// `CLK_EQU_SYSCLK` set, which really is "SPI runs at the APB rate" — one
    /// APB tick per bit, i.e. 2 CPU cycles on a 160 MHz C3 and 3 on a 240 MHz
    /// S3. Not a refusal: it is what the register says, and what silicon would
    /// do if firmware launched without touching CLOCK.
    #[test]
    fn the_reset_clock_register_means_the_apb_rate() {
        assert_eq!(bit_time_cycles(0x8000_3043, 160_000_000), Some(2));
        assert_eq!(bit_time_cycles(0x8000_3043, 240_000_000), Some(3));
    }

    /// 1 MHz from an 80 MHz APB is a divide by 80, and `CLKCNT_N` is only SIX
    /// bits — 79 does not fit, which is exactly why `spi_ll_master_cal_clock_reg`
    /// searches for a `(pre, n)` PAIR. `pre_reg = 1` (÷2) with `n_reg = 39` (÷40)
    /// is one such pair. On the C3 that is 160 CPU cycles per bit; on the S3,
    /// 240.
    #[test]
    fn the_divisors_give_the_programmed_rate() {
        let clock = (1 << CLKDIV_PRE_SHIFT) | (39 << CLKCNT_N_SHIFT);
        assert_eq!(bit_time_cycles(clock, 160_000_000), Some(160));
        assert_eq!(bit_time_cycles(clock, 240_000_000), Some(240));
    }

    /// ⚠️ Both divisor fields are narrow and a value that overflows one spills
    /// into the next. Writing 79 into `CLKCNT_N` keeps only its low six bits
    /// (15 ⇒ ÷16) AND sets `CLKDIV_PRE` to 1 (⇒ ÷2), giving ÷32 — not the ÷80
    /// the caller meant. Decoding those fields as if they were wide enough is a
    /// trace at a frequency the firmware never asked for.
    #[test]
    fn an_overflowing_divisor_is_masked_not_widened() {
        let clock = 79 << CLKCNT_N_SHIFT;
        assert_eq!(bit_time_cycles(clock, 160_000_000), Some(2 * 16 * 160 / 80));
    }

    /// CPOL/CPHA come from the two registers the datasheet puts them in, and
    /// nowhere else. Reading `CK_IDLE_EDGE` out of `USER` (or `CK_OUT_EDGE` out
    /// of `MISC`) would give a trace at the right rate sampled on the wrong
    /// edge, which decodes to garbage that looks plausible.
    #[test]
    fn mode_bits_come_from_misc_and_user() {
        assert_eq!(
            framing(0, 0),
            SpiFraming {
                cpol: false,
                cpha: false,
                bits: 8
            }
        );
        assert!(framing(CK_IDLE_EDGE, 0).cpol);
        assert!(!framing(CK_IDLE_EDGE, 0).cpha);
        assert!(framing(0, CK_OUT_EDGE).cpha);
        assert!(!framing(0, CK_OUT_EDGE).cpol);
        // The bit positions must not be confused for one another.
        assert!(!framing(CK_OUT_EDGE, 0).cpol, "USER's bit is not MISC's");
        assert!(!framing(0, CK_IDLE_EDGE).cpha, "MISC's bit is not USER's");
    }

    /// An unbound wire — every lab that never routed an SPI pad — buffers
    /// nothing at all, so the whole path costs one branch per byte.
    #[test]
    fn an_unrouted_controller_holds_nothing() {
        let mut wire = EspSpiWire::default();
        wire.push(0xA5, SpiFraming::default(), Some(160), 0);
        assert!(!wire.is_pending());
        assert_eq!(wire.ready_in(0), 0);
    }

    /// A held burst is due exactly when its wire time has elapsed, never before.
    #[test]
    fn a_held_burst_is_due_when_the_wire_has_carried_it() {
        let mut wire = EspSpiWire::default();
        let _ = wire.pad_lines_arc(false);
        wire.push(0xA5, SpiFraming::default(), Some(160), 0);
        // One 8-bit frame occupies 10 bit periods (8 clocked + CS + idle).
        let duration = SpiFraming::default().frame_bits() * 160;
        assert_eq!(wire.ready_in(0), duration);
        assert_eq!(wire.ready_in(duration - 1), 1);
        assert_eq!(wire.ready_in(duration), 0);
    }
}
