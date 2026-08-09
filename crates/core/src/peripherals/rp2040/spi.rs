// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! RP2040 SPI — ARM PrimeCell PL022 SSP (datasheet §4.4, SPI0 base
//! `0x4003c000`).
//!
//! A minimal but real SSP transfer engine: the data register (`SSPDR`) feeds a
//! transmit path and a receive FIFO, and the status register (`SSPSR`) reports
//! FIFO state. With the controller enabled (`SSPCR1.SSE`) and internal loopback
//! selected (`SSPCR1.LBM`) — the PL022's built-in self-test mode — each byte
//! written to `SSPDR` is clocked straight back into the receive FIFO, so a
//! write-then-read pair returns the same byte: a genuine modelled transfer, not
//! storage. Transfers complete within the write, so `SSPSR.BSY` is never
//! observed busy and the TX FIFO always reads empty.
//!
//! Without loopback (and with no attached slave in the chip model) a written
//! byte still clocks a byte into the receive FIFO: SPI is full-duplex at the
//! physical layer, so every SCLK edge that shifts a TX bit out also shifts an
//! RX bit in from whatever MISO happens to be doing, wired or floating. The
//! value is undefined with nothing driving the line (modelled as the idle
//! level `0x00`, matching the STM32 PL022-family SPI model's precedent — see
//! `crates/core/src/peripherals/spi.rs`), but the *event* — RNE going high,
//! the RX FIFO gaining an entry — always happens. Pico-sdk's
//! `spi_write_read_blocking` (which is what Arduino's `SPI.transfer()` rides)
//! waits for `rx_remaining` to reach 0 one `spi_is_readable()` poll at a
//! time; if the model never produced RX data for an unconnected bus, that
//! wait never ends and any sketch calling `SPI.transfer()` hangs forever —
//! not a halt, an infinite spin, which is worse: it never surfaces as an
//! error at all.
//!
//! The receive FIFO is read-to-drain, so it lives behind a `RefCell`: the bus
//! read path is `&self`, but reading `SSPDR` must pop an entry.

use crate::peripherals::pad_lines::PadLines;
use crate::peripherals::spi_waveform::{NarrationFit, SpiFraming, SpiNarrator};
use crate::{CycleClock, Peripheral, PeripheralTickResult, SimResult};
use std::cell::RefCell;
use std::collections::VecDeque;
use std::sync::Arc;

// PL022 register offsets (RP2040 SVD, peripheral SPI0 @ 0x4003c000; identical
// in pico-sdk hardware/regs/spi.h).
const SSPCR0: u64 = 0x000;
const SSPCR1: u64 = 0x004;
const SSPDR: u64 = 0x008;
const SSPSR: u64 = 0x00c;
const SSPCPSR: u64 = 0x010;

// SSPCR0 fields (SVD bitRanges: SCR [15:8], SPH [7:7], SPO [6:6], FRF [5:4],
// DSS [3:0]). Writable mask is the SVD's SPI_SSPCR0_BITS == 0xFFFF.
const CR0_MASK: u32 = 0xFFFF;
const CR0_DSS: u32 = 0x000F;
const CR0_SPO: u32 = 1 << 6;
const CR0_SPH: u32 = 1 << 7;
const CR0_SCR_SHIFT: u32 = 8;
const CR0_SCR_MASK: u32 = 0xFF;

// SSPCR1 control bits.
const CR1_LBM: u32 = 1 << 0; // loopback mode
const CR1_SSE: u32 = 1 << 1; // synchronous serial port enable

// SSPCPSR: CPSDVSR [7:0].
const CPSR_MASK: u32 = 0x00FF;

// SSPSR status bits.
const SR_TFE: u32 = 1 << 0; // transmit FIFO empty
const SR_TNF: u32 = 1 << 1; // transmit FIFO not full
const SR_RNE: u32 = 1 << 2; // receive FIFO not empty
const SR_RFF: u32 = 1 << 3; // receive FIFO full
const SR_BSY: u32 = 1 << 4; // busy

// PL022 FIFOs are 8 entries deep.
const FIFO_DEPTH: usize = 8;

/// The pad lines this controller DRIVES, in the order a lab probes them.
///
/// MISO is deliberately absent. Nothing in the engine drives it — this model
/// has no attached devices at all and clocks in the idle level `0x00` (see the
/// module doc) — so a published MISO line would report a confident constant
/// level, and a pad bound to it would look authoritative while carrying
/// nothing. Same rule that keeps `wire_rp2040_uart_pads` TX-only. MISO joins
/// this list when something drives it, not before.
pub(crate) const SPI_LINES: &[&str] = &["SCK", "MOSI", "CSn"];
pub(crate) const LINE_SCK: usize = 0;
pub(crate) const LINE_MOSI: usize = 1;
pub(crate) const LINE_CSN: usize = 2;

/// Words a narration may hold waiting for the wire before it is published
/// anyway, compressed.
///
/// A firmware that outruns the programmed bit rate forever would otherwise
/// buffer forever and the trace would stay empty — the one outcome worse than a
/// compressed one. 256 is far deeper than the PL022's 8-entry TX FIFO, so a
/// burst reaches this only when the firmware is genuinely shifting faster than
/// the wire it programmed can carry.
const WIRE_BURST_CAP: usize = 256;

#[derive(Default)]
pub struct Rp2040Spi {
    cr0: u32,
    cr1: u32,
    cpsr: u32,
    rx_fifo: RefCell<VecDeque<u16>>,
    /// Wire levels published to FUNCSEL-routed SCK/MOSI/CSn pads, so a logic
    /// analyzer clipped to this bus measures a waveform instead of the SIO
    /// output latch. `None` — the common case, no lab routed the pads — costs
    /// one branch per transfer and publishes nothing.
    lines: Option<Arc<PadLines>>,
    /// Words shifted since the last narration flush, with the framing SSPCR0
    /// held AT THE MOMENT each was written. Carried per word rather than read
    /// back at flush time so a firmware that reprograms DSS/SPO/SPH mid-burst
    /// still narrates each frame the way it actually went out.
    wire_words: Vec<(u16, SpiFraming)>,
    /// Bit period the buffered burst is being narrated at, captured on the
    /// first word of the burst. A rate change mid-burst force-flushes what is
    /// held rather than repainting it at the new rate.
    wire_bit_time: u64,
    /// Cycle the last narration ran to — the floor the next one may not reach
    /// back past, or two bursts splice into frames neither transfer sent.
    wave_cursor: u64,
    /// Bus cycle clock, attached by the registration choke (`add_peripheral` /
    /// `push_peripheral`). Present ⇒ this model knows "now" and can hold a
    /// burst until the wire has had time to carry it. `None` (hand-built test
    /// buses) publishes nothing, which is the honest answer: with no clock
    /// there is no cycle axis to place a waveform on.
    clock: Option<CycleClock>,
    /// Monotonic token so a stale in-flight wakeup from a superseded arm is
    /// ignored.
    arm_seq: u32,
    /// True while a flush wakeup is in flight, so a burst of writes arms
    /// exactly one chain rather than one per write.
    scheduled: bool,
}

impl std::fmt::Debug for Rp2040Spi {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Rp2040Spi")
            .field("cr0", &self.cr0)
            .field("cr1", &self.cr1)
            .field("cpsr", &self.cpsr)
            .field("buffered_words", &self.wire_words.len())
            .finish()
    }
}

impl Rp2040Spi {
    pub fn new() -> Self {
        Self::default()
    }

    fn enabled(&self) -> bool {
        self.cr1 & CR1_SSE != 0
    }

    fn loopback(&self) -> bool {
        self.cr1 & CR1_LBM != 0
    }

    fn status(&self) -> u32 {
        // TX path drains immediately, so TX is always empty / not full and the
        // engine is never busy (SR_BSY is never asserted).
        let mut sr = SR_TFE | SR_TNF;
        let rx = self.rx_fifo.borrow();
        if !rx.is_empty() {
            sr |= SR_RNE;
        }
        if rx.len() >= FIFO_DEPTH {
            sr |= SR_RFF;
        }
        let _ = SR_BSY;
        sr
    }

    /// Pop the head of the receive FIFO (the `SSPDR` read port).
    fn pop_dr(&self) -> u32 {
        self.rx_fifo.borrow_mut().pop_front().unwrap_or(0) as u32
    }

    /// The shared pad-line cell for this controller's SCK/MOSI/CSn, created on
    /// first use at bus wiring time.
    ///
    /// SCK idles at the programmed polarity (SSPCR0.SPO, zero at reset), MOSI
    /// low, chip select RELEASED — an idle SPI bus reads CS high, and a model
    /// that idled it low would frame a transfer that never happened.
    pub(crate) fn pad_lines_arc(&mut self) -> Arc<PadLines> {
        let cpol = self.cr0 & CR0_SPO != 0;
        self.lines
            .get_or_insert_with(|| Arc::new(PadLines::new(SPI_LINES, &[cpol, false, true])))
            .clone()
    }

    /// How SSPCR0 currently frames a word on the wire.
    ///
    /// `bits` is DSS + 1. The SVD names 0000..0010 reserved, so 4 is the
    /// narrowest real frame; the clamp keeps a firmware that wrote a reserved
    /// value from producing a zero-bit frame rather than an error.
    fn framing(&self) -> SpiFraming {
        SpiFraming {
            cpol: self.cr0 & CR0_SPO != 0,
            cpha: self.cr0 & CR0_SPH != 0,
            bits: (((self.cr0 & CR0_DSS) + 1) as u8).clamp(4, 16),
        }
    }

    /// Engine cycles in one SCK period, from this controller's own timing
    /// registers.
    ///
    /// pico-sdk `hardware_spi/spi.c::spi_set_baudrate` programs
    /// `cpsr = prescale` and `SCR = postdiv - 1`, and returns
    /// `clk_peri / (prescale * postdiv)` — so one bit period is
    /// `CPSDVSR × (1 + SCR)` peripheral-clock ticks, which is what the SVD's
    /// SCR description states as `F_SSPCLK / (CPSDVSR × (1+SCR))`.
    ///
    /// Those are `clk_peri` ticks, used here as engine cycles. That is exact on
    /// the RP2040, whose `clk_peri` defaults to `clk_sys` — the same assumption
    /// `Uart::bit_time_cycles` documents for the PL011 divisors.
    ///
    /// `None` means firmware never programmed a prescaler. CPSDVSR resets to 0,
    /// which real silicon treats as an invalid divisor (the SVD requires "an
    /// even number from 2-254"), so narrating anyway would put a trace on the
    /// pad measuring a frequency the firmware never asked for. Silence beats a
    /// confident wrong answer.
    fn bit_time_cycles(&self) -> Option<u64> {
        let cpsdvsr = u64::from(self.cpsr & CPSR_MASK);
        if cpsdvsr == 0 {
            return None;
        }
        let scr = u64::from((self.cr0 >> CR0_SCR_SHIFT) & CR0_SCR_MASK);
        let ticks = cpsdvsr * (1 + scr);
        (ticks >= 2).then_some(ticks)
    }

    /// Queue a shifted word for narration. Buffered, not published — see
    /// [`Rp2040Spi::wire_flush`]. No routed pads or no programmed prescaler ⇒
    /// nothing to narrate, and the call costs one branch.
    fn wire_push(&mut self, word: u16) {
        if self.lines.is_none() {
            return;
        }
        let Some(bit_time) = self.bit_time_cycles() else {
            return;
        };
        // A rate change mid-burst: publish what is held at the rate it was
        // shifted at, THEN start a new burst. Repainting the held words at the
        // new rate would report a bit period no transfer ever used.
        if !self.wire_words.is_empty() && bit_time != self.wire_bit_time {
            self.wire_flush(true);
        }
        if self.wire_words.is_empty() {
            self.wire_bit_time = bit_time;
        }
        let framing = self.framing();
        self.wire_words.push((word, framing));
    }

    /// Cycles until the buffered burst has had its wire time — 0 when it is due
    /// now, when nothing is buffered, or when the burst has hit the cap and
    /// must be published compressed rather than held any longer.
    ///
    /// This is both the pacing test and the scheduler deadline, computed the
    /// ONE way, so a wakeup can never land before the burst is publishable (a
    /// wasted wakeup) or after it (a late trace).
    fn wire_ready_in(&self) -> u64 {
        if self.wire_words.is_empty() || self.wire_words.len() >= WIRE_BURST_CAP {
            return 0;
        }
        let Some(clock) = &self.clock else {
            return 0;
        };
        let duration: u64 = self
            .wire_words
            .iter()
            .map(|(_, framing)| framing.frame_bits() * self.wire_bit_time)
            .sum();
        self.wave_cursor
            .saturating_add(duration)
            .saturating_sub(clock.now())
    }

    /// Publish the buffered words onto the routed pads, once the wire has had
    /// time to carry them.
    ///
    /// The word-level model shifts a frame inside the `SSPDR` write and reports
    /// TX permanently empty, so pico-sdk's `spi_write_blocking` hands this model
    /// a whole buffer within a few dozen cycles. The WIRE cannot do that — eight
    /// bytes at the 1 MHz arduino-pico default take ~10 000 cycles at
    /// `clk_sys = 125 MHz` — and the capture layer (`LogicTap::push_at` →
    /// `LogicCapture::ingest_push`) only accepts stamps in the PAST and keeps a
    /// single level per channel per cycle. There is simply nowhere to put a
    /// frame that has not yet had time to cross.
    ///
    /// So the burst accumulates and is narrated as one waveform ending at the
    /// present cycle, exactly as the I²C and UART narrators publish. Holding the
    /// flush until `now` has passed the burst's wire time is what makes the
    /// common case EXACT: the trace then carries every frame at the rate
    /// CPSDVSR/SCR program, which is what the FIFO would really have drained.
    ///
    /// `force` publishes regardless — the cap path, and the rate-change path.
    /// The burst is then compressed: the words stay readable, the timebase does
    /// not.
    fn wire_flush(&mut self, force: bool) {
        if self.wire_words.is_empty() {
            return;
        }
        let (Some(lines), Some(clock)) = (self.lines.clone(), self.clock.clone()) else {
            self.wire_words.clear();
            return;
        };
        let now = clock.now();
        if !force && self.wire_ready_in() > 0 {
            return;
        }
        let mut narrator = SpiNarrator::with_lines(
            LINE_SCK,
            LINE_MOSI,
            Some(LINE_CSN),
            &[
                lines.level(LINE_SCK),
                lines.level(LINE_MOSI),
                lines.level(LINE_CSN),
            ],
            self.wire_bit_time,
        );
        for &(word, framing) in &self.wire_words {
            narrator.frame(word, framing);
        }
        if let NarrationFit::LevelsOnly { .. } =
            narrator.emit_between(&lines, self.wave_cursor, now)
        {
            // Fewer cycles exist than the waveform has transitions, so nothing
            // was drawn. Keep the words and the cursor: `now` only grows, so a
            // later wakeup will have the room. Clearing here would delete
            // frames that really crossed the bus and advance the cursor past
            // cycles nothing ever painted — silent, unrecoverable loss, and the
            // reason `emit_between` is `#[must_use]`.
            return;
        }
        self.wave_cursor = now;
        self.wire_words.clear();
    }
}

impl Peripheral for Rp2040Spi {
    // ⚠️ `needs_legacy_walk()` is DELIBERATELY not overridden here, and the
    // default (`true`) is the honest answer even though the TRANSFER is
    // write-driven.
    //
    // This model used to declare `needs_legacy_walk() -> false` with the comment
    // "pure write-driven transfer engine — tick() is the default no-op". That
    // was true while `tick()` really was the default. It is not any more: the
    // pad narration below needs a wakeup to publish on, and `tick()` is where
    // the featureless build takes it. Keeping the `false` claim alongside a
    // working `tick()` is exactly the walk-starvation bug class
    // `crate::tests::walk_starvation_contract` rule A exists to catch, and it
    // caught this — a `false` here plus whole-walk deletion means the burst is
    // never published at all.
    //
    // Returning the default costs nothing, which is the point: every consumer
    // reads the pair as `uses_scheduler() || !needs_legacy_walk()`
    // (`SystemBus::derive_walk_deletable` and the walk-differential tests), and
    // `uses_scheduler()` below is `true`, so the OR short-circuits and RP2040
    // walk-deletion lands exactly where it did before this change. Same shape as
    // `crate::peripherals::rp2040::i2c::Rp2040I2c`, which also leaves the
    // default standing and carries its delivery on the scheduler.

    /// Scheduler-driven: the burst flush rides an event chain, not the per-cycle
    /// walk.
    ///
    /// # Why an event chain and not `needs_legacy_walk() -> true`
    ///
    /// Restoring the walk would fix publication by making every RP2040 lab slow:
    /// `SystemBus::derive_walk_deletable` is all-or-nothing, so one forcing model
    /// drops the walk-deletion for the whole bus, including the majority of labs
    /// that never route an SPI pad. The chain below costs wakeups only while a
    /// burst is genuinely buffered — which requires BOTH routed pads and a
    /// programmed prescaler — and it arms at the exact cycle the wire finishes
    /// (`wire_ready_in`), so an eight-byte burst costs ONE wakeup, not 10 000.
    ///
    /// `derive_walk_deletable` is `uses_scheduler() || !needs_legacy_walk()`, so
    /// this claim leaves RP2040 walk-deletion exactly where it was.
    ///
    /// Left ungated (not `cfg!(feature = "event-scheduler")`) so walk-deletion
    /// derives identically in both builds; the walk's skip of scheduler models
    /// is itself feature-gated, so a featureless build flushes through `tick()`
    /// below instead.
    fn uses_scheduler(&self) -> bool {
        true
    }

    fn take_scheduled_events(&mut self) -> Vec<(u64, u32)> {
        // Only arm while a burst is actually held, and only once per chain.
        if self.wire_words.is_empty() || self.scheduled {
            return Vec::new();
        }
        self.arm_seq = self.arm_seq.wrapping_add(1);
        self.scheduled = true;
        // `collect_scheduled_events` converts this to `current_cycle + 1 + delay`,
        // so delay 0 is "the cycle the walk's next tick would have serviced it".
        vec![(self.wire_ready_in(), self.arm_seq)]
    }

    fn on_event(
        &mut self,
        event_token: u32,
        _sched: &mut crate::sched::EventScheduler,
        _bus: &mut dyn crate::Bus,
    ) -> crate::sched::EventResult {
        if event_token != self.arm_seq {
            // Stale token from a superseded arm; the live chain owns publication.
            return crate::sched::EventResult::default();
        }
        self.wire_flush(self.wire_words.len() >= WIRE_BURST_CAP);
        // A flush that reported LevelsOnly keeps its words; `wire_ready_in` is
        // then 0, so `max(1)` retries on the next cycle and converges as the run
        // grows. A successful flush leaves nothing and the chain stops.
        let pending = !self.wire_words.is_empty();
        self.scheduled = pending;
        crate::sched::EventResult {
            reschedule_delay: pending.then(|| self.wire_ready_in().max(1)),
            ..Default::default()
        }
    }

    /// Legacy-walk path only.
    ///
    /// Under `event-scheduler` the walk skips this model (`uses_scheduler`) and
    /// the chain above owns publication. Without the feature the walk still runs
    /// and this is where the burst reaches the pads. Inert — one `is_empty`
    /// check — on every bus that has not routed an SPI pad, which is every
    /// RP2040 lab that existed before this change.
    fn tick(&mut self) -> PeripheralTickResult {
        self.wire_flush(self.wire_words.len() >= WIRE_BURST_CAP);
        PeripheralTickResult::default()
    }

    fn attach_cycle_clock(&mut self, clock: CycleClock) {
        self.clock = Some(clock);
    }

    fn as_any(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }

    fn as_any_mut(&mut self) -> Option<&mut dyn std::any::Any> {
        Some(self)
    }

    fn read_u32(&self, offset: u64) -> SimResult<u32> {
        let val = match offset {
            // SSPCR0 and SSPCPSR are read-WRITE per the SVD, and pico-sdk's
            // `spi_set_baudrate` / `spi_set_format` both reach them through
            // `hw_write_masked`, i.e. a read-modify-write. A model that read
            // them as 0 silently discarded every field a RMW meant to preserve.
            SSPCR0 => self.cr0,
            SSPCR1 => self.cr1,
            SSPDR => self.pop_dr(), // reading SSPDR drains the RX FIFO
            SSPSR => self.status(),
            SSPCPSR => self.cpsr,
            _ => {
                crate::census_reg!("rp2040.spi:Rp2040Spi", offset, "read");
                0
            }
        };
        Ok(val)
    }

    fn write_u32(&mut self, offset: u64, value: u32) -> SimResult<()> {
        match offset {
            SSPCR0 => self.cr0 = value & CR0_MASK,
            SSPCR1 => self.cr1 = value,
            SSPCPSR => self.cpsr = value & CPSR_MASK,
            // Full duplex: every enabled write clocks a word into the RX FIFO.
            // Loopback wires MOSI straight to MISO, so the RX word is the TX
            // word; without loopback (no attached slave in the chip model) it is
            // the undefined/idle MISO level (`0x00`) — see the module doc
            // comment for why this must still happen.
            SSPDR if self.enabled() => {
                let word = (value & 0xffff) as u16;
                let rx_word = if self.loopback() { word } else { 0 };
                {
                    let mut rx = self.rx_fifo.borrow_mut();
                    if rx.len() < FIFO_DEPTH {
                        rx.push_back(rx_word);
                    }
                }
                // The word went out on the wire whether or not the RX FIFO had
                // room for what came back, so narrate it outside the FIFO guard.
                self.wire_push(word);
            }
            _ => {
                crate::census_reg!("rp2040.spi:Rp2040Spi", offset, "write");
            }
        }
        Ok(())
    }

    fn read(&self, offset: u64) -> SimResult<u8> {
        // Reading the data register (any byte lane of it) drains one FIFO entry.
        if (offset & !0x3) == SSPDR {
            let word = self.pop_dr();
            return Ok((word >> ((offset & 0x3) * 8)) as u8);
        }
        let word = self.read_u32(offset & !0x3)?;
        Ok((word >> ((offset & 0x3) * 8)) as u8)
    }

    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        let aligned = offset & !0x3;
        let shift = (offset & 0x3) * 8;
        // Avoid the read-modify-write going through the draining SSPDR read.
        let cur = if aligned == SSPDR {
            0
        } else {
            self.read_u32(aligned)?
        };
        let new = (cur & !(0xFF << shift)) | ((value as u32) << shift);
        self.write_u32(aligned, new)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn enable_loopback(spi: &mut Rp2040Spi) {
        spi.write_u32(SSPCR1, CR1_SSE | CR1_LBM).unwrap();
    }

    #[test]
    fn loopback_roundtrips_byte() {
        let mut spi = Rp2040Spi::new();
        enable_loopback(&mut spi);
        spi.write_u32(SSPDR, 0xA5).unwrap();
        // RNE must be set after the loopback transfer.
        assert_ne!(spi.read_u32(SSPSR).unwrap() & SR_RNE, 0);
        // Draining read returns the same byte.
        let rx = spi.read_u32(SSPDR).unwrap();
        assert_eq!(rx, 0xA5);
        // FIFO drained → RNE clear.
        assert_eq!(spi.read_u32(SSPSR).unwrap() & SR_RNE, 0);
    }

    #[test]
    fn non_loopback_transfer_still_completes() {
        // Full duplex: SPI.transfer()'s pico-sdk backend (spi_write_read_blocking)
        // waits for an RX byte per TX byte regardless of whether a slave is
        // wired. Without this, any real (non-loopback) SPI.transfer() sketch
        // hangs forever polling `spi_is_readable()`.
        let mut spi = Rp2040Spi::new();
        spi.write_u32(SSPCR1, CR1_SSE).unwrap(); // enabled, no loopback
        spi.write_u32(SSPDR, 0x5A).unwrap();
        assert_ne!(
            spi.read_u32(SSPSR).unwrap() & SR_RNE,
            0,
            "RNE must set on every enabled transfer, slave or no slave"
        );
        // The undefined/floating-MISO byte reads as the idle level.
        assert_eq!(spi.read_u32(SSPDR).unwrap(), 0);
    }

    #[test]
    fn disabled_port_does_not_capture() {
        let mut spi = Rp2040Spi::new();
        // Loopback selected but SSE not set → no transfer.
        spi.write_u32(SSPCR1, CR1_LBM).unwrap();
        spi.write_u32(SSPDR, 0x42).unwrap();
        assert_eq!(spi.read_u32(SSPSR).unwrap() & SR_RNE, 0);
    }

    #[test]
    fn status_reports_tx_empty_and_not_busy() {
        let spi = Rp2040Spi::new();
        let sr = spi.read_u32(SSPSR).unwrap();
        assert_ne!(sr & SR_TFE, 0, "TX FIFO empty at reset");
        assert_ne!(sr & SR_TNF, 0, "TX FIFO not full at reset");
        assert_eq!(sr & SR_BSY, 0, "not busy at reset");
    }

    #[test]
    fn control_registers_read_back_what_a_read_modify_write_left() {
        // pico-sdk reaches SSPCR0 only through `hw_write_masked`, which reads
        // the register, merges its field and writes back. While SSPCR0 read as
        // zero, `spi_set_format`'s DSS/SPO/SPH write silently erased the SCR
        // `spi_set_baudrate` had just programmed, and vice versa.
        let mut spi = Rp2040Spi::new();
        spi.write_u32(SSPCPSR, 2).unwrap();
        spi.write_u32(SSPCR0, 62 << CR0_SCR_SHIFT).unwrap();
        // The format write is a RMW: read, merge DSS/SPO/SPH, write back.
        let merged = (spi.read_u32(SSPCR0).unwrap() & !(CR0_DSS | CR0_SPO | CR0_SPH)) | 0x07;
        spi.write_u32(SSPCR0, merged).unwrap();
        assert_eq!(spi.read_u32(SSPCPSR).unwrap(), 2);
        assert_eq!(
            (spi.read_u32(SSPCR0).unwrap() >> CR0_SCR_SHIFT) & CR0_SCR_MASK,
            62,
            "the format write must not erase the baud rate",
        );
        assert_eq!(spi.framing().bits, 8, "DSS 0b0111 is an 8-bit frame");
    }

    #[test]
    fn the_bit_period_is_the_prescaler_times_one_plus_scr() {
        // pico-sdk spi_set_baudrate: baud = clk_peri / (CPSDVSR * (1 + SCR)).
        // For 1 MHz from a 125 MHz clk_peri it programs CPSDVSR=2, SCR=62.
        let mut spi = Rp2040Spi::new();
        assert_eq!(
            spi.bit_time_cycles(),
            None,
            "CPSDVSR resets to 0, an invalid divisor — no timebase, no waveform",
        );
        spi.write_u32(SSPCPSR, 2).unwrap();
        spi.write_u32(SSPCR0, 62 << CR0_SCR_SHIFT).unwrap();
        assert_eq!(spi.bit_time_cycles(), Some(126));
        // Dropping the (1 + SCR) factor would give 2 here.
        spi.write_u32(SSPCR0, 0).unwrap();
        assert_eq!(spi.bit_time_cycles(), Some(2), "SCR = 0 is a divide by 1");
    }

    #[test]
    fn a_controller_with_no_routed_pads_buffers_nothing() {
        // The zero-cost claim: every RP2040 lab that predates pad routing must
        // allocate nothing and arm no wakeup.
        let mut spi = Rp2040Spi::new();
        spi.write_u32(SSPCPSR, 2).unwrap();
        spi.write_u32(SSPCR1, CR1_SSE).unwrap();
        for byte in 0..64u32 {
            spi.write_u32(SSPDR, byte).unwrap();
        }
        assert!(spi.wire_words.is_empty());
        assert!(spi.take_scheduled_events().is_empty());
    }

    #[test]
    fn a_burst_is_held_until_the_wire_has_had_time_and_then_published_once() {
        use crate::CycleClock;
        let clock = CycleClock::default();
        let mut spi = Rp2040Spi::new();
        spi.attach_cycle_clock(clock.clone());
        let lines = spi.pad_lines_arc();
        spi.write_u32(SSPCPSR, 2).unwrap();
        spi.write_u32(SSPCR0, (62 << CR0_SCR_SHIFT) | 0x07).unwrap();
        spi.write_u32(SSPCR1, CR1_SSE).unwrap();

        clock.publish(1_000_000);
        spi.wave_cursor = 1_000_000;
        spi.write_u32(SSPDR, 0xA5).unwrap();
        // 8 bits + 2 framing periods, at 126 cycles a bit = 1260 cycles.
        assert_eq!(spi.wire_ready_in(), 1_260);
        spi.tick();
        assert_eq!(spi.wire_words.len(), 1, "not yet carried, so not published");

        clock.publish(1_000_000 + 1_260);
        assert_eq!(spi.wire_ready_in(), 0);
        spi.tick();
        assert!(spi.wire_words.is_empty(), "published exactly once");
        assert!(
            lines.level(LINE_CSN),
            "chip select released after the frame"
        );
    }
}
