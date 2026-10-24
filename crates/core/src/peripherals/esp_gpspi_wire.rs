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
/// The lines of a GP-SPI that can also be a slave on a world `gpio_net`
/// (ESP32-C3/C6): [`SPI_LINES`] plus MISO, which a slave drives and a master
/// reads (`crate::peripherals::esp_gpspi_net`). The narration above never
/// touches MISO; off a net its line is an input and a pad routed to it
/// shows what the outside holds there, never a made-up level.
pub const SPI_NET_LINES: &[&str] = &["SCK", "MOSI", "CS", "MISO"];
pub const LINE_MISO: usize = 3;

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
    /// Earliest cycle at which a burst that last reported `LevelsOnly` can have
    /// the room it lacked. Until then every flush attempt would rebuild the
    /// same too-long narration and fail again; see [`Self::ready_in`].
    retry_at: u64,
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

    /// [`Self::pad_lines_arc`] with a MISO line ([`SPI_NET_LINES`]), for a
    /// controller that takes part in a world `gpio_net`. MISO starts as an
    /// input; the net engine says what each line does from then on.
    pub fn pad_lines_arc_with_miso(&mut self, cpol: bool) -> Arc<PadLines> {
        self.lines
            .get_or_insert_with(|| {
                let lines = PadLines::new(SPI_NET_LINES, &[cpol, false, true, false]);
                lines.set_mode(LINE_MISO, super::pad_lines::LineDrive::Input);
                Arc::new(lines)
            })
            .clone()
    }

    /// The shared cell itself, once created.
    pub fn lines_arc(&self) -> Option<Arc<PadLines>> {
        self.lines.clone()
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
    ///
    /// A burst that already failed for lack of cycles (`LevelsOnly`) waits for
    /// the deficit it reported before anything else, cap included. Without that
    /// a burst past the cap — a `writeBytes` of a whole camera frame at 40 MHz —
    /// was re-narrated on every single cycle while it grew, which made the run
    /// quadratic in the burst length.
    pub fn ready_in(&self, now: u64) -> u64 {
        if self.words.is_empty() {
            return 0;
        }
        if now < self.retry_at {
            return self.retry_at - now;
        }
        if self.words.len() >= WIRE_BURST_CAP {
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
        // Nobody records this wire's edges — no capture tap, no `gpio_net` — so
        // the only thing anyone can observe is where the lines END (a pad
        // read). Every frame drives all three lines itself, so the last byte
        // alone leaves them exactly where the whole burst would. Narrating
        // every byte here cost a third of a camera-to-display frame loop: a
        // 38 400-byte `writeBytes` is ~600 000 planned edges per frame that
        // nothing reads. A tap or a net brings the full narration back.
        let observed = lines.tap_clock().is_some() || lines.any_on_net();
        let narrated = if observed {
            &self.words[..]
        } else {
            &self.words[self.words.len() - 1..]
        };
        for &(byte, framing) in narrated {
            narrator.frame(u16::from(byte), framing);
        }
        if let NarrationFit::LevelsOnly {
            transitions,
            available,
        } = narrator.emit_between(&lines, self.wave_cursor, now)
        {
            // Fewer cycles exist than the waveform has transitions, so nothing
            // was drawn. Keep the bytes and the cursor: `now` only grows, so a
            // later wakeup will have the room. Clearing here would delete bytes
            // that really crossed the bus and advance the cursor past cycles
            // nothing painted — silent, unrecoverable loss, and the reason
            // `emit_between` is `#[must_use]`. The room arrives no sooner than
            // the deficit, so no attempt is made before then.
            self.retry_at = now.saturating_add(transitions.saturating_sub(available).max(1));
            return;
        }
        self.wave_cursor = now;
        self.retry_at = 0;
        self.words.clear();
    }
}

// ---------------------------------------------------------------------------
// The shared per-chip GP-SPI test suite.
//
// ESP32-C3 GP-SPI2 and ESP32-S3 GP-SPI2/SPI3 are one Espressif IP block, and
// their test modules carried separate copies of the same register-level suite
// (11 same-name pairs, seven of them ≥0.97). This macro writes it once.
//
// Unqualified names (`CLOCK`, `CMD`, `USR_BIT`, `MS_DLEN`, `W0`, ...) resolve
// at the invocation site, where `use super::*` gives each chip its own
// constants. Per-chip facts that differ INSIDE a shared body are parameters:
// the intr-matrix source, the reset-test name and the two C3-vs-S3 reset
// values, and the two `miso_fills` preloads (the C3 engine merges MISO
// byte-wise while the S3 overwrites the whole word — the preloads prove
// exactly that difference, so they must not be flattened).
// `s_name` is the test-local binding those fragments refer to (`s` on both
// chips), passed as an ident so the fragments can see it across macro hygiene.
// `int_st_pre/mid/post` are optional fragments carrying the C3's legacy-tick
// membership assertions around the shared `int_st_masks_with_ena_and_emits_source`.
#[cfg(test)]
macro_rules! esp_gpspi_suite {
    (
        ctrl: $ctrl:ty,
        s_name: $s:ident,
        source: $source:expr,
        reset_test: $reset_test:ident,
        dma_conf_reset: $dma_conf_reset:expr,
        date_reset: $date_reset:expr,
        w0_preload: $w0_preload:expr,
        w1_preload: $w1_preload:expr,
        $( int_st_pre: { $($int_st_pre:tt)* }, )?
        $( int_st_mid: { $($int_st_mid:tt)* }, )?
        $( int_st_post: { $($int_st_post:tt)* }, )?
    ) => {
        struct ExternalCanPoller(std::sync::Arc<std::sync::atomic::AtomicUsize>);
        impl SpiDevice for ExternalCanPoller {
            fn needs_external_bus_poll(&self) -> bool {
                true
            }
            fn component_id(&self) -> Option<&str> {
                Some("external-can")
            }
            fn poll_external_bus(&mut self) {
                self.0.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            }
            fn transfer(&mut self, _mosi: u8) -> u8 {
                0
            }
            fn cs_pin(&self) -> &str {
                "GPIO10"
            }
        }

        #[test]
        fn scheduler_arms_recurring_external_can_poll() {
            let polls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
            let mut spi = <$ctrl>::new($source);
            spi.push_device(Box::new(ExternalCanPoller(polls.clone())));
            spi.attach_cycle_clock(crate::CycleClock::default());

            let events = spi.take_scheduled_events();
            assert_eq!(
                events.len(),
                1,
                "scheduler mode must arm external CAN polling"
            );
            let mut scheduler = crate::sched::EventScheduler::new();
            let mut bus = crate::bus::SystemBus::new();
            let result = spi.on_event(events[0].1, &mut scheduler, &mut bus);
            assert_eq!(polls.load(std::sync::atomic::Ordering::SeqCst), 1);
            assert_eq!(result.reschedule_delay, Some(1));

            let legacy_polls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
            let mut legacy = <$ctrl>::new($source);
            legacy.push_device(Box::new(ExternalCanPoller(legacy_polls.clone())));
            assert!(legacy.take_scheduled_events().is_empty());
            legacy.tick();
            assert_eq!(legacy_polls.load(std::sync::atomic::Ordering::SeqCst), 1);
        }

        #[test]
        fn $reset_test() {
            let s = <$ctrl>::new($source);
            assert_eq!(s.read_u32(CLOCK).unwrap(), 0x8000_3043, "CLOCK");
            assert_eq!(s.read_u32(CTRL).unwrap(), 0x003C_0000, "CTRL");
            assert_eq!(s.read_u32(USER).unwrap(), 0x8000_00C0, "USER");
            assert_eq!(s.read_u32(USER1).unwrap(), 0xB841_0007, "USER1");
            assert_eq!(s.read_u32(USER2).unwrap(), 0x7800_0000, "USER2");
            assert_eq!(s.read_u32(MISC).unwrap(), 0x0000_003E, "MISC");
            assert_eq!(s.read_u32(DMA_CONF).unwrap(), $dma_conf_reset, "DMA_CONF");
            assert_eq!(s.read_u32(SLAVE).unwrap(), 0x0280_0000, "SLAVE");
            assert_eq!(s.read_u32(DATE).unwrap(), $date_reset, "DATE");
        }

        #[test]
        fn unmapped_offsets_read_zero_and_ignore_writes() {
            // Holes inside the map and offsets above DATE must NOT round-trip.
            let mut s = <$ctrl>::new($source);
            for off in [0x48u64, 0x90, 0xD8, 0xEC, 0xF4, 0xFFC] {
                s.write_u32(off, 0xDEAD_BEEF).unwrap();
                assert_eq!(s.read_u32(off).unwrap(), 0, "hole at {off:#x}");
            }
        }

        #[test]
        fn cmd_update_bit_self_clears() {
            // ESP-IDF spi_master writes UPDATE then polls for it to clear; a
            // round-tripping CMD would wedge the driver.
            let mut s = <$ctrl>::new($source);
            s.write_u32(CMD, UPDATE_BIT).unwrap();
            assert_eq!(
                s.read_u32(CMD).unwrap() & UPDATE_BIT,
                0,
                "UPDATE self-clears"
            );
        }

        #[test]
        fn dma_conf_afifo_rst_strobes_self_clear() {
            let mut s = <$ctrl>::new($source);
            s.write_u32(DMA_CONF, AFIFO_RST_BITS).unwrap();
            assert_eq!(
                s.read_u32(DMA_CONF).unwrap() & AFIFO_RST_BITS,
                0,
                "AFIFO reset strobes self-clear"
            );
        }

        #[test]
        fn slave_soft_reset_self_clears() {
            let mut s = <$ctrl>::new($source);
            s.write_u32(SLAVE, SOFT_RESET_BIT).unwrap();
            assert_eq!(
                s.read_u32(SLAVE).unwrap() & SOFT_RESET_BIT,
                0,
                "SOFT_RESET (WT) self-clears"
            );
        }

        #[test]
        fn w_buffer_round_trips() {
            let mut s = <$ctrl>::new($source);
            for w in 0..16u64 {
                s.write_u32(W0 + w * 4, 0xA000_0000 | (w as u32)).unwrap();
            }
            for w in 0..16u64 {
                assert_eq!(s.read_u32(W0 + w * 4).unwrap(), 0xA000_0000 | (w as u32));
            }
            assert_eq!(W15, W0 + 15 * 4, "W15 offset");
        }

        #[test]
        fn usr_start_auto_clears_and_latches_trans_done() {
            let mut s = <$ctrl>::new($source);
            s.write_u32(MS_DLEN, 32 - 1).unwrap(); // 32-bit transfer
            s.write_u32(CMD, USR_BIT).unwrap();
            // USR auto-cleared so `!(CMD & USR)` poll exits.
            assert_eq!(s.read_u32(CMD).unwrap() & USR_BIT, 0, "USR auto-clears");
            // TRANS_DONE latched in INT_RAW.
            assert_eq!(
                s.read_u32(DMA_INT_RAW).unwrap() & TRANS_DONE,
                TRANS_DONE,
                "TRANS_DONE latched"
            );
        }

        #[test]
        fn miso_fills_0xff_for_no_device() {
            let mut s = <$ctrl>::new($source);
            s.write_u32(W0, $w0_preload).unwrap();
            s.write_u32(W0 + 4, $w1_preload).unwrap();
            // 5-byte (40-bit) transfer → W0 fully 0xFF, W1 low byte 0xFF only.
            s.write_u32(MS_DLEN, 40 - 1).unwrap();
            s.write_u32(CMD, USR_BIT).unwrap();
            assert_eq!(
                s.read_u32(W0).unwrap(),
                0xFFFF_FFFF,
                "first 4 MISO bytes 0xFF"
            );
            assert_eq!(
                s.read_u32(W0 + 4).unwrap(),
                0x0000_00FF,
                "5th byte 0xFF, rest untouched"
            );
        }

        #[test]
        fn int_clr_is_w1c() {
            let mut s = <$ctrl>::new($source);
            s.write_u32(MS_DLEN, 8 - 1).unwrap();
            s.write_u32(CMD, USR_BIT).unwrap();
            assert_eq!(s.read_u32(DMA_INT_RAW).unwrap() & TRANS_DONE, TRANS_DONE);
            // W1C: writing the bit clears it; writing 0 leaves others intact.
            s.write_u32(DMA_INT_CLR, TRANS_DONE).unwrap();
            assert_eq!(
                s.read_u32(DMA_INT_RAW).unwrap() & TRANS_DONE,
                0,
                "W1C cleared"
            );
        }

        #[test]
        fn int_st_masks_with_ena_and_emits_source() {
            let mut $s = <$ctrl>::new($source);
            $( $($int_st_pre)* )?
            $s.write_u32(MS_DLEN, 8 - 1).unwrap();
            $s.write_u32(CMD, USR_BIT).unwrap();
            // No enable yet → ST gated, no source emitted.
            assert_eq!($s.read_u32(DMA_INT_ST).unwrap(), 0, "ST gated by ENA");
            assert_eq!($s.tick().explicit_irqs, None, "no source while ST==0");
            // Enable TRANS_DONE → ST asserts and the source is emitted.
            $s.write_u32(DMA_INT_ENA, TRANS_DONE).unwrap();
            assert_eq!($s.read_u32(DMA_INT_ST).unwrap() & TRANS_DONE, TRANS_DONE);
            $( $($int_st_mid)* )?
            assert_eq!($s.tick().explicit_irqs, Some(vec![$source]));
            // Clear the raw bit → ST drops, source stops.
            $s.write_u32(DMA_INT_CLR, TRANS_DONE).unwrap();
            $( $($int_st_post)* )?
            assert_eq!($s.tick().explicit_irqs, None);
        }
    };
}

#[cfg(test)]
pub(crate) use esp_gpspi_suite;

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

    /// A burst past the cap that cannot fit yet is not retried every cycle:
    /// it waits for exactly the cycles it was short, keeps every byte, and
    /// then publishes.
    #[test]
    fn a_burst_too_long_for_the_run_waits_for_its_deficit() {
        let mut wire = EspSpiWire::default();
        // A watched wire (here: on a net) is narrated in full; an unwatched
        // one only narrates its last byte and never runs short.
        wire.pad_lines_arc(false).mark_on_net(LINE_SCK);
        let now = 100;
        for _ in 0..WIRE_BURST_CAP + 1 {
            wire.push(0xA5, SpiFraming::default(), Some(2), now);
        }
        // Past the cap: due at once, but 257 bytes cannot be drawn in 100
        // cycles, so nothing is published.
        assert_eq!(wire.ready_in(now), 0);
        wire.flush(now, false);
        assert!(wire.is_pending(), "no byte may be dropped");
        let wait = wire.ready_in(now);
        assert!(wait > 1, "the retry waits for the deficit, got {wait}");
        // One cycle early it is still not due, so the narration is not rebuilt.
        assert_eq!(wire.ready_in(now + wait - 1), 1);
        wire.flush(now + wait, false);
        assert!(
            !wire.is_pending(),
            "with the room it lacked, the burst publishes"
        );
    }

    /// With no tap and no net only the final levels are observable, and the
    /// shortcut that narrates just the last byte must leave the lines exactly
    /// where narrating the whole burst does.
    #[test]
    fn an_unwatched_wire_ends_where_the_whole_burst_would() {
        for bytes in [
            vec![0xA5u8, 0x3C, 0xFF, 0x00, 0x81],
            vec![0x81, 0x00],
            vec![0xFF; 300],
        ] {
            let mut wire = EspSpiWire::default();
            let lines = wire.pad_lines_arc(false);
            for &b in &bytes {
                wire.push(b, SpiFraming::default(), Some(4), 0);
            }
            wire.flush(10_000_000, false);
            assert!(!wire.is_pending());

            let full = PadLines::new(SPI_LINES, &[false, false, true]);
            let mut narrator = SpiNarrator::with_lines(
                LINE_SCK,
                LINE_MOSI,
                Some(LINE_CS),
                &[false, false, true],
                4,
            );
            for &b in &bytes {
                narrator.frame(u16::from(b), SpiFraming::default());
            }
            let _ = narrator.emit_between(&full, 0, 10_000_000);
            for line in [LINE_SCK, LINE_MOSI, LINE_CS] {
                assert_eq!(
                    lines.level(line),
                    full.level(line),
                    "line {line} after {bytes:02x?}"
                );
            }
        }
    }
}
