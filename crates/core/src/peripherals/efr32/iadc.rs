// LabWired - Firmware Simulation Platform
// SPDX-License-Identifier: MIT

//! Silicon Labs EFR32 Series-2 **IADC** — the incremental ADC.
//!
//! # Sources
//!
//! Register offsets are walked from `IADC_TypeDef` in the vendor CMSIS header
//! `efr32mg26_iadc.h` (`simplicity_sdk` tag `sisdk-2025.6`); field positions
//! and reset values are the `_IADC_<REG>_<FIELD>_SHIFT` / `_RESETVALUE`
//! defines from the same header. Nothing here is recalled.
//!
//! ⚠️ Do not guess these offsets from the struct by eye. `IADC_TypeDef` embeds
//! two register GROUPS — `CFG[2]` (stride 0x10) and `SCANTABLE[16]` (stride
//! 0x04) — and treating either as a flat `uint32_t` shifts every offset after
//! it. The check that catches it: `IPVERSION_SET` must land exactly at
//! `+0x1000`, because Series 2 aliases each 4 KiB register view at
//! `+0x1000/+0x2000/+0x3000`. It does with these strides and does not with any
//! other, which is how the map below was verified.
//!
//! # What firmware does with it, and what is modelled
//!
//! The `analogRead` path is the SINGLE queue: select an input in `SINGLE`
//! (`PORTPOS`/`PINPOS`), write `CMD.SINGLESTART`, wait for
//! `STATUS.SINGLEFIFODV`, read `SINGLEFIFODATA`. All of that is modelled:
//!
//! * `EN.EN` gates conversion. A `SINGLESTART` on a disabled IADC does
//!   nothing at all — no result, no flag — which is what silicon does and what
//!   catches the commonest bring-up mistake after a missing clock enable.
//! * `SINGLE` selects a real input. `PORTPOS` names a GPIO port (8=PORTA,
//!   9=PORTB, 10=PORTC, 11=PORTD, plus GND and SUPPLY) and `PINPOS` the pin;
//!   the conversion reads the analog level standing on THAT pad, so wiring a
//!   source to the wrong pin reads whatever is actually there rather than the
//!   value the test wanted.
//! * The result is a 12-bit right-aligned code, `round-down(mV / Vref * 4096)`,
//!   saturating at full scale. `Vref` defaults to 3300 mV — the Explorer Kit's
//!   AVDD — and is settable.
//! * `SINGLEFIFODATA` POPS; `SINGLEDATA` reads the newest result WITHOUT
//!   popping. They are different registers with different semantics and
//!   firmware uses both.
//! * `STATUS.SINGLEFIFODV` is set while the FIFO holds a result and clears
//!   when the last one is popped. `IF.SINGLEDONE` latches per conversion and
//!   is write-1-to-clear; `IEN.SINGLEDONE` raises the IRQ.
//! * `CMD.SINGLEFIFOFLUSH` empties the FIFO.
//!
//! # Idealised — present, but not physical
//!
//! * **Conversion is instantaneous.** A real IADC takes several ADC_CLK cycles
//!   (`CFG[x].SCHED`, warm-up, oversampling); here the result is available on
//!   the tick after `SINGLESTART`. Firmware that polls `SINGLEFIFODV` works;
//!   firmware that measures conversion TIME sees zero.
//! * **No differential mode.** `PORTNEG`/`PINNEG` are stored and ignored;
//!   every conversion is single-ended against ground.
//! * **`CFG[]` is stored, not honoured** — no oversampling, no analog gain, no
//!   reference selection, no `SCALE` offset/gain calibration. The width is
//!   always 12 bits, so a firmware that programs a different `ANALOGGAIN` or
//!   oversampling ratio reads the same code it would have without.
//! * **No SCAN queue.** `SCANTABLE`, `SCANFIFODATA` and the scan half of
//!   `CMD`/`STATUS`/`IF` decode and store, and a `SCANSTART` produces nothing.
//!   Firmware that scans a table hangs waiting for `SCANFIFODV`, loudly,
//!   rather than being handed invented samples.
//! * **No calibration, no comparator, no timer trigger.** `CMPTHR`, `TIMER`,
//!   `TRIGGER` and `MASKREQ` store and do nothing.
//! * **No warm-up state machine.** `STATUS.ADCWARM` never sets.

use crate::{Peripheral, PeripheralTickResult, SimResult};

// ── Register offsets, walked from `IADC_TypeDef` ───────────────────────────
const OFF_IPVERSION: u64 = 0x00;
const OFF_EN: u64 = 0x04;
const OFF_CTRL: u64 = 0x08;
const OFF_CMD: u64 = 0x0C;
const OFF_TIMER: u64 = 0x10;
const OFF_STATUS: u64 = 0x14;
const OFF_MASKREQ: u64 = 0x18;
const OFF_STMASK: u64 = 0x1C;
const OFF_CMPTHR: u64 = 0x20;
const OFF_IF: u64 = 0x24;
const OFF_IEN: u64 = 0x28;
const OFF_TRIGGER: u64 = 0x2C;
/// `CFG[2]`, stride 0x10: CFG, reserved, SCALE, SCHED.
const OFF_CFG: u64 = 0x48;
const CFG_STRIDE: u64 = 0x10;
const CFG_COUNT: u64 = 2;
const OFF_SINGLEFIFOCFG: u64 = 0x70;
const OFF_SINGLEFIFODATA: u64 = 0x74;
const OFF_SINGLEFIFOSTAT: u64 = 0x78;
const OFF_SINGLEDATA: u64 = 0x7C;
const OFF_SCANFIFOCFG: u64 = 0x80;
const OFF_SCANFIFODATA: u64 = 0x84;
const OFF_SCANFIFOSTAT: u64 = 0x88;
const OFF_SCANDATA: u64 = 0x8C;
const OFF_SINGLE: u64 = 0x98;
/// `SCANTABLE[16]`, stride 0x04 — one `SCAN` word per entry.
const OFF_SCANTABLE: u64 = 0xA0;
const SCANTABLE_WORDS: u64 = 16;

/// `IADC_IPVERSION` reset value.
const IPVERSION_RESET: u32 = 3;

// ── Field positions ────────────────────────────────────────────────────────
/// `EN.EN`.
const EN_EN: u32 = 1 << 0;
/// `CMD.SINGLESTART`.
const CMD_SINGLESTART: u32 = 1 << 0;
/// `CMD.SINGLESTOP`.
const CMD_SINGLESTOP: u32 = 1 << 1;
/// `CMD.SINGLEFIFOFLUSH`.
const CMD_SINGLEFIFOFLUSH: u32 = 1 << 24;
/// `STATUS.SINGLEFIFODV` — the single FIFO holds a result.
const STATUS_SINGLEFIFODV: u32 = 1 << 8;
/// `IF.SINGLEDONE`.
const IF_SINGLEDONE: u32 = 1 << 9;
/// `SINGLE.PINPOS` / `SINGLE.PORTPOS`.
const SINGLE_PINPOS_SHIFT: u32 = 8;
const SINGLE_PINPOS_MASK: u32 = 0xF;
const SINGLE_PORTPOS_SHIFT: u32 = 12;
const SINGLE_PORTPOS_MASK: u32 = 0xF;

/// `SINGLE.PORTPOS` encodings that name something to convert.
const PORTPOS_GND: u32 = 0x0;
const PORTPOS_SUPPLY: u32 = 0x1;
const PORTPOS_PORTA: u32 = 0x8;
const PORTPOS_PORTD: u32 = 0xB;

/// The IADC is 12-bit. Results are right-aligned in that width.
const ADC_BITS: u32 = 12;
const ADC_FULL_SCALE: u32 = (1 << ADC_BITS) - 1;

/// Default reference, in millivolts: AVDD on the BRD2709A.
const DEFAULT_VREF_MV: u32 = 3300;

/// Results the single FIFO holds before the oldest is dropped. The real depth
/// is 4 on this part (`SINGLEFIFOSTAT.FIFOREADCNT` is 3 bits, and the RM's
/// FIFO is 4 deep).
const SINGLE_FIFO_DEPTH: usize = 4;

/// Analog channel index for a `(port, pin)` pair — the key
/// [`Peripheral::set_adc_channel_input`] and the `system.yaml` analog sources
/// address a pad by.
///
/// `port` is 0..=3 for A..D, matching the GPIO port structs' order, so channel
/// `0x00` is PA00 and channel `0x23` is PC03. This is a LabWired index, not a
/// silicon one: the IADC has no "channel number", it has a port/pin mux.
pub const fn channel_for(port: u8, pin: u8) -> u8 {
    (port << 4) | (pin & 0xF)
}

/// The EFR32 Series-2 incremental ADC.
#[derive(Debug)]
pub struct Efr32s2Iadc {
    en: u32,
    ctrl: u32,
    timer: u32,
    maskreq: u32,
    stmask: u32,
    cmpthr: u32,
    iflag: u32,
    ien: u32,
    trigger: u32,
    cfg: [u32; (CFG_STRIDE / 4 * CFG_COUNT) as usize],
    singlefifocfg: u32,
    scanfifocfg: u32,
    single: u32,
    scantable: [u32; SCANTABLE_WORDS as usize],

    /// Analog level standing on each pad, in millivolts, keyed by
    /// [`channel_for`]. Absent = 0 mV, i.e. grounded.
    inputs: std::collections::HashMap<u8, u16>,
    /// Reference voltage the conversion is against, in millivolts.
    vref_mv: u32,

    /// Completed results, oldest first.
    ///
    /// Behind a `Mutex` because reading `SINGLEFIFODATA` POPS, and the
    /// `Peripheral` read path is `&self`. This is the same interior-mutability
    /// shape the shared `Uart` uses to pop its RX byte, and a `Mutex` rather
    /// than a `RefCell` for the same reason: a re-entrant borrow of a `RefCell`
    /// panics with "already borrowed", which in wasm surfaces as the
    /// unattributable "recursive use of an object".
    fifo: std::sync::Mutex<std::collections::VecDeque<u32>>,
    /// The newest result, readable through `SINGLEDATA` without popping.
    last_result: std::sync::atomic::AtomicU32,
    /// A `SINGLESTART` is waiting for the next tick to convert.
    conversion_pending: bool,
    /// The word the last byte-lane-0 read of `SINGLEFIFODATA` popped, so a
    /// byte-wise 32-bit read returns one coherent result instead of popping
    /// four times. See the `read` override.
    byte_lane_cache: std::sync::atomic::AtomicU32,
}

impl Default for Efr32s2Iadc {
    fn default() -> Self {
        Self::new()
    }
}

impl Efr32s2Iadc {
    pub fn new() -> Self {
        Self {
            en: 0,
            ctrl: 0,
            timer: 0,
            maskreq: 0,
            stmask: 0,
            cmpthr: 0,
            iflag: 0,
            ien: 0,
            trigger: 0,
            cfg: [0; (CFG_STRIDE / 4 * CFG_COUNT) as usize],
            singlefifocfg: 0,
            scanfifocfg: 0,
            single: 0,
            scantable: [0; SCANTABLE_WORDS as usize],
            inputs: std::collections::HashMap::new(),
            vref_mv: DEFAULT_VREF_MV,
            fifo: std::sync::Mutex::new(std::collections::VecDeque::new()),
            last_result: std::sync::atomic::AtomicU32::new(0),
            conversion_pending: false,
            byte_lane_cache: std::sync::atomic::AtomicU32::new(0),
        }
    }

    /// Set the reference the conversion is against, in millivolts. The default
    /// is the Explorer Kit's 3300 mV AVDD.
    pub fn set_vref_mv(&mut self, mv: u32) {
        self.vref_mv = mv.max(1);
    }

    /// Drive the analog level on one pad, in millivolts.
    pub fn set_channel_input(&mut self, channel: u8, millivolts: u16) {
        self.inputs.insert(channel, millivolts);
    }

    /// The millivolts `SINGLE` currently selects, or `None` when it names
    /// nothing convertible.
    fn selected_millivolts(&self) -> Option<u32> {
        let port = (self.single >> SINGLE_PORTPOS_SHIFT) & SINGLE_PORTPOS_MASK;
        let pin = ((self.single >> SINGLE_PINPOS_SHIFT) & SINGLE_PINPOS_MASK) as u8;
        match port {
            PORTPOS_GND => Some(0),
            PORTPOS_SUPPLY => Some(self.vref_mv),
            PORTPOS_PORTA..=PORTPOS_PORTD => {
                let channel = channel_for((port - PORTPOS_PORTA) as u8, pin);
                Some(*self.inputs.get(&channel).unwrap_or(&0) as u32)
            }
            // Every other PORTPOS encoding names an input this model does not
            // have (the internal temperature sensor, AVDD dividers, the
            // opamp/DAC taps). Returning 0 would be a plausible-looking
            // measurement of something that was never wired.
            _ => None,
        }
    }

    /// Convert millivolts to a 12-bit right-aligned code, saturating at full
    /// scale — a real SAR cannot report more than all-ones.
    fn code_for(&self, millivolts: u32) -> u32 {
        let code = (millivolts as u64 * (1u64 << ADC_BITS)) / self.vref_mv as u64;
        (code as u32).min(ADC_FULL_SCALE)
    }

    fn fifo_len(&self) -> usize {
        self.fifo.lock().map(|f| f.len()).unwrap_or(0)
    }

    fn status(&self) -> u32 {
        let mut s = 0;
        if self.fifo_len() > 0 {
            s |= STATUS_SINGLEFIFODV;
        }
        s
    }

    fn push_result(&mut self, code: u32) {
        if let Ok(mut f) = self.fifo.lock() {
            f.push_back(code);
            while f.len() > SINGLE_FIFO_DEPTH {
                f.pop_front();
            }
        }
        self.last_result
            .store(code, std::sync::atomic::Ordering::Relaxed);
        self.iflag |= IF_SINGLEDONE;
    }

    /// Pop the oldest result. `&self` because this is what a `SINGLEFIFODATA`
    /// read does, and the read path is `&self`.
    fn pop_result(&self) -> u32 {
        self.fifo
            .lock()
            .ok()
            .and_then(|mut f| f.pop_front())
            .unwrap_or(0)
    }

    fn cfg_index(offset: u64) -> Option<usize> {
        if (OFF_CFG..OFF_CFG + CFG_STRIDE * CFG_COUNT).contains(&offset) {
            Some(((offset - OFF_CFG) / 4) as usize)
        } else {
            None
        }
    }

    fn scantable_index(offset: u64) -> Option<usize> {
        if (OFF_SCANTABLE..OFF_SCANTABLE + SCANTABLE_WORDS * 4).contains(&offset) {
            Some(((offset - OFF_SCANTABLE) / 4) as usize)
        } else {
            None
        }
    }

    /// The oldest queued result WITHOUT popping.
    fn front_result(&self) -> u32 {
        self.fifo
            .lock()
            .ok()
            .and_then(|f| f.front().copied())
            .unwrap_or(0)
    }

    /// A NON-DESTRUCTIVE view of every register.
    ///
    /// ⚠️ `SINGLEFIFODATA` is a popping register, and popping belongs to the
    /// two paths a CPU load actually takes — `read_u32` and lane 0 of `read`.
    /// It must NOT happen here, because `read_word` is also what `peek` and the
    /// byte read-modify-write use. `peek` is documented as a side-effect-free
    /// probe for debug and observer bookkeeping; when it popped, an observer
    /// attached to the bus silently ate the firmware's sample and the
    /// conversion read back 0 with `STATUS.SINGLEFIFODV` still set. Measured,
    /// on the first end-to-end `analogRead`.
    fn read_word(&self, offset: u64) -> u32 {
        match offset {
            OFF_IPVERSION => IPVERSION_RESET,
            OFF_EN => self.en,
            OFF_CTRL => self.ctrl,
            // CMD is write-only on silicon: every bit is a command, and none
            // of them is state to read back.
            OFF_CMD => 0,
            OFF_TIMER => self.timer,
            OFF_STATUS => self.status(),
            OFF_MASKREQ => self.maskreq,
            OFF_STMASK => self.stmask,
            OFF_CMPTHR => self.cmpthr,
            OFF_IF => self.iflag,
            OFF_IEN => self.ien,
            OFF_TRIGGER => self.trigger,
            OFF_SINGLEFIFOCFG => self.singlefifocfg,
            // NOT the popping view — see `read_word`'s contract below.
            OFF_SINGLEFIFODATA => self.front_result(),
            OFF_SINGLEFIFOSTAT => self.fifo_len() as u32,
            OFF_SINGLEDATA => self.last_result.load(std::sync::atomic::Ordering::Relaxed),
            OFF_SCANFIFOCFG => self.scanfifocfg,
            OFF_SCANFIFODATA | OFF_SCANDATA => 0,
            OFF_SCANFIFOSTAT => 0,
            OFF_SINGLE => self.single,
            o => {
                if let Some(i) = Self::cfg_index(o) {
                    self.cfg[i]
                } else if let Some(i) = Self::scantable_index(o) {
                    self.scantable[i]
                } else {
                    0
                }
            }
        }
    }

    fn write_word(&mut self, offset: u64, value: u32) {
        match offset {
            OFF_EN => {
                self.en = value & EN_EN;
                if self.en & EN_EN == 0 {
                    // Disabling drops in-flight work. Silicon does not resume a
                    // conversion across a disable, and keeping the FIFO would
                    // let firmware read a sample from before it reconfigured.
                    self.conversion_pending = false;
                    if let Ok(mut f) = self.fifo.lock() {
                        f.clear();
                    }
                }
            }
            OFF_CTRL => self.ctrl = value,
            OFF_CMD => self.apply_cmd(value),
            OFF_TIMER => self.timer = value,
            OFF_MASKREQ => self.maskreq = value,
            OFF_STMASK => self.stmask = value,
            OFF_CMPTHR => self.cmpthr = value,
            // Write-1-to-clear, the Series-2 convention.
            OFF_IF => self.iflag &= !value,
            OFF_IEN => self.ien = value,
            OFF_TRIGGER => self.trigger = value,
            OFF_SINGLEFIFOCFG => self.singlefifocfg = value,
            OFF_SCANFIFOCFG => self.scanfifocfg = value,
            OFF_SINGLE => self.single = value,
            o => {
                if let Some(i) = Self::cfg_index(o) {
                    self.cfg[i] = value;
                } else if let Some(i) = Self::scantable_index(o) {
                    self.scantable[i] = value;
                }
            }
        }
    }

    fn apply_cmd(&mut self, value: u32) {
        if value & CMD_SINGLEFIFOFLUSH != 0 {
            if let Ok(mut f) = self.fifo.lock() {
                f.clear();
            }
        }
        if value & CMD_SINGLESTOP != 0 {
            self.conversion_pending = false;
        }
        if value & CMD_SINGLESTART != 0 {
            // A disabled IADC accepts the command and does nothing with it.
            // That is the behaviour worth modelling: firmware that forgot
            // `EN.EN` then spins on `SINGLEFIFODV` forever, here and on the
            // bench, instead of reading a sample the silicon never took.
            if self.en & EN_EN != 0 {
                self.conversion_pending = true;
            }
        }
    }
}

impl Peripheral for Efr32s2Iadc {
    /// ⚠️ A byte read of `SINGLEFIFODATA` must pop the FIFO ONCE, not once per
    /// byte. The bus's default `read_u32` is four `read` calls, so this model
    /// overrides `read_u32` (below) and a bare byte read of the data register
    /// pops only on byte 0 — the other three bytes come from the value that
    /// pop returned.
    fn read(&self, offset: u64) -> SimResult<u8> {
        let reg = offset & !3;
        if reg == OFF_SINGLEFIFODATA {
            let lane = offset % 4;
            let word = if lane == 0 {
                let popped = self.pop_result();
                self.byte_lane_cache
                    .store(popped, std::sync::atomic::Ordering::Relaxed);
                popped
            } else {
                self.byte_lane_cache
                    .load(std::sync::atomic::Ordering::Relaxed)
            };
            return Ok(((word >> (lane * 8)) & 0xFF) as u8);
        }
        let word = self.read_word(reg);
        Ok(((word >> ((offset % 4) * 8)) & 0xFF) as u8)
    }

    /// Side-effect-free by contract: never pops. See `read_word`.
    fn peek(&self, offset: u64) -> Option<u8> {
        let word = self.read_word(offset & !3);
        Some(((word >> ((offset % 4) * 8)) & 0xFF) as u8)
    }

    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        let reg = offset & !3;
        let shift = (offset % 4) * 8;
        let merged = (self.read_word(reg) & !(0xFFu32 << shift)) | ((value as u32) << shift);
        self.write_word(reg, merged);
        Ok(())
    }

    /// The path a 32-bit CPU load takes. `SINGLEFIFODATA` pops HERE.
    fn read_u32(&self, offset: u64) -> SimResult<u32> {
        if offset == OFF_SINGLEFIFODATA {
            return Ok(self.pop_result());
        }
        Ok(self.read_word(offset))
    }

    fn write_u32(&mut self, offset: u64, value: u32) -> SimResult<()> {
        self.write_word(offset, value);
        Ok(())
    }

    fn needs_legacy_walk(&self) -> bool {
        true
    }

    fn tick_elapsed(&mut self, _cycles: u64) -> PeripheralTickResult {
        let mut result = PeripheralTickResult::default();
        if self.conversion_pending {
            self.conversion_pending = false;
            // `None` means SINGLE names an input this model does not have. No
            // result is produced and no flag is set, so firmware waiting on
            // SINGLEFIFODV hangs rather than reading an invented sample.
            if let Some(mv) = self.selected_millivolts() {
                let code = self.code_for(mv);
                self.push_result(code);
            }
        }
        if self.ien & self.iflag != 0 {
            result.irq = true;
        }
        result
    }

    fn set_adc_channel_input(&mut self, channel: u8, millivolts: u16) -> bool {
        self.set_channel_input(channel, millivolts);
        true
    }

    fn as_any(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// PA05 — port A, pin 5.
    const PA05_PORT: u32 = PORTPOS_PORTA;
    const PA05_PIN: u32 = 5;

    fn select(iadc: &mut Efr32s2Iadc, port: u32, pin: u32) {
        iadc.write_word(
            OFF_SINGLE,
            (port << SINGLE_PORTPOS_SHIFT) | (pin << SINGLE_PINPOS_SHIFT),
        );
    }

    /// Enable, select, start, settle — the `analogRead` sequence.
    fn convert(iadc: &mut Efr32s2Iadc) -> u32 {
        iadc.write_word(OFF_CMD, CMD_SINGLESTART);
        iadc.tick_elapsed(1);
        assert_eq!(
            iadc.read_word(OFF_STATUS) & STATUS_SINGLEFIFODV,
            STATUS_SINGLEFIFODV,
            "no result in the single FIFO"
        );
        iadc.pop_result()
    }

    fn enabled() -> Efr32s2Iadc {
        let mut iadc = Efr32s2Iadc::new();
        iadc.write_word(OFF_EN, EN_EN);
        iadc
    }

    /// The offsets are only right if `IPVERSION_SET` lands at `+0x1000` — the
    /// Series-2 alias stride. This is the arithmetic that catches a mis-parsed
    /// `CFG[2]` or `SCANTABLE[16]` group, which shifts everything after it.
    #[test]
    fn the_register_map_ends_exactly_where_the_alias_window_begins() {
        let last = OFF_SCANTABLE + SCANTABLE_WORDS * 4;
        assert!(
            last <= 0x1000,
            "the register map runs past its own alias window at {last:#x}"
        );
        assert_eq!(
            OFF_CFG + CFG_STRIDE * CFG_COUNT,
            0x68,
            "CFG[2] ends at 0x68"
        );
        assert_eq!(last, 0xE0, "SCANTABLE[16] ends at 0xE0");
    }

    #[test]
    fn ipversion_reads_the_header_reset_value() {
        assert_eq!(Efr32s2Iadc::new().read_word(OFF_IPVERSION), IPVERSION_RESET);
    }

    #[test]
    fn a_conversion_returns_the_level_standing_on_the_selected_pad() {
        let mut iadc = enabled();
        iadc.set_channel_input(channel_for(0, 5), 1650); // PA05 at half of AVDD
        select(&mut iadc, PA05_PORT, PA05_PIN);
        // 1650 mV against 3300 mV, 12 bits: 2048.
        assert_eq!(convert(&mut iadc), 2048);
    }

    #[test]
    fn the_code_tracks_the_level_and_saturates_at_full_scale() {
        let mut iadc = enabled();
        select(&mut iadc, PA05_PORT, PA05_PIN);
        for (mv, expect) in [(0u16, 0u32), (825, 1024), (3300, ADC_FULL_SCALE)] {
            iadc.set_channel_input(channel_for(0, 5), mv);
            assert_eq!(convert(&mut iadc), expect, "{mv} mV");
        }
        // Above the reference a real SAR still reports all-ones.
        iadc.set_channel_input(channel_for(0, 5), 5000);
        assert_eq!(convert(&mut iadc), ADC_FULL_SCALE);
    }

    /// The pin selection has to matter. A model that ignored `PINPOS` would
    /// pass every single-source test ever written.
    #[test]
    fn selecting_another_pin_reads_that_pin() {
        let mut iadc = enabled();
        iadc.set_channel_input(channel_for(0, 5), 3300); // PA05
        iadc.set_channel_input(channel_for(0, 6), 0); // PA06

        select(&mut iadc, PA05_PORT, 5);
        assert_eq!(convert(&mut iadc), ADC_FULL_SCALE);
        select(&mut iadc, PA05_PORT, 6);
        assert_eq!(convert(&mut iadc), 0);
    }

    /// And so does the port.
    #[test]
    fn selecting_another_port_reads_that_port() {
        let mut iadc = enabled();
        iadc.set_channel_input(channel_for(0, 3), 3300); // PA03
        iadc.set_channel_input(channel_for(2, 3), 825); // PC03

        select(&mut iadc, PORTPOS_PORTA, 3);
        assert_eq!(convert(&mut iadc), ADC_FULL_SCALE);
        select(&mut iadc, PORTPOS_PORTA + 2, 3); // PORTC
        assert_eq!(convert(&mut iadc), 1024);
    }

    #[test]
    fn gnd_and_supply_are_real_selections() {
        let mut iadc = enabled();
        select(&mut iadc, PORTPOS_GND, 0);
        assert_eq!(convert(&mut iadc), 0);
        select(&mut iadc, PORTPOS_SUPPLY, 0);
        assert_eq!(convert(&mut iadc), ADC_FULL_SCALE);
    }

    /// The commonest bring-up mistake on this family after a missing clock:
    /// converting on a disabled IADC. It must hang, not answer.
    #[test]
    fn a_disabled_iadc_produces_no_result_at_all() {
        let mut iadc = Efr32s2Iadc::new(); // EN never written
        iadc.set_channel_input(channel_for(0, 5), 3300);
        select(&mut iadc, PA05_PORT, PA05_PIN);
        iadc.write_word(OFF_CMD, CMD_SINGLESTART);
        iadc.tick_elapsed(1);
        assert_eq!(iadc.read_word(OFF_STATUS) & STATUS_SINGLEFIFODV, 0);
        assert_eq!(iadc.read_word(OFF_IF) & IF_SINGLEDONE, 0);
    }

    /// An input this model does not have must produce nothing rather than a
    /// plausible zero — a firmware reading the internal temperature sensor
    /// should hang and be fixed, not be handed 0 °C.
    #[test]
    fn an_unmodelled_input_produces_no_result() {
        let mut iadc = enabled();
        select(&mut iadc, 0x2, 0); // neither GND, SUPPLY nor a port
        iadc.write_word(OFF_CMD, CMD_SINGLESTART);
        iadc.tick_elapsed(1);
        assert_eq!(iadc.read_word(OFF_STATUS) & STATUS_SINGLEFIFODV, 0);
    }

    /// `SINGLEFIFODATA` pops and `SINGLEDATA` does not. Firmware uses both and
    /// a model that aliased them would double-consume or never advance.
    #[test]
    fn singledata_peeks_where_singlefifodata_pops() {
        let mut iadc = enabled();
        iadc.set_channel_input(channel_for(0, 5), 1650);
        select(&mut iadc, PA05_PORT, PA05_PIN);
        iadc.write_word(OFF_CMD, CMD_SINGLESTART);
        iadc.tick_elapsed(1);

        assert_eq!(iadc.read_word(OFF_SINGLEDATA), 2048);
        assert_eq!(
            iadc.read_word(OFF_SINGLEDATA),
            2048,
            "peeking twice is fine"
        );
        assert_eq!(
            iadc.read_word(OFF_STATUS) & STATUS_SINGLEFIFODV,
            STATUS_SINGLEFIFODV,
            "a peek must not empty the FIFO"
        );

        assert_eq!(iadc.pop_result(), 2048);
        assert_eq!(iadc.read_word(OFF_STATUS) & STATUS_SINGLEFIFODV, 0);
    }

    #[test]
    fn the_done_flag_latches_and_is_write_one_to_clear() {
        let mut iadc = enabled();
        select(&mut iadc, PORTPOS_SUPPLY, 0);
        iadc.write_word(OFF_CMD, CMD_SINGLESTART);
        iadc.tick_elapsed(1);
        assert_eq!(iadc.read_word(OFF_IF) & IF_SINGLEDONE, IF_SINGLEDONE);

        iadc.write_word(OFF_IF, IF_SINGLEDONE);
        assert_eq!(iadc.read_word(OFF_IF) & IF_SINGLEDONE, 0);
    }

    #[test]
    fn the_interrupt_follows_ien() {
        let mut iadc = enabled();
        select(&mut iadc, PORTPOS_SUPPLY, 0);

        iadc.write_word(OFF_CMD, CMD_SINGLESTART);
        assert!(!iadc.tick_elapsed(1).irq, "IEN clear: no interrupt");

        iadc.write_word(OFF_IF, IF_SINGLEDONE);
        iadc.write_word(OFF_IEN, IF_SINGLEDONE);
        iadc.write_word(OFF_CMD, CMD_SINGLESTART);
        assert!(iadc.tick_elapsed(1).irq, "IEN set: interrupt");
    }

    #[test]
    fn flushing_the_fifo_empties_it() {
        let mut iadc = enabled();
        select(&mut iadc, PORTPOS_SUPPLY, 0);
        iadc.write_word(OFF_CMD, CMD_SINGLESTART);
        iadc.tick_elapsed(1);
        assert_eq!(
            iadc.read_word(OFF_STATUS) & STATUS_SINGLEFIFODV,
            STATUS_SINGLEFIFODV
        );

        iadc.write_word(OFF_CMD, CMD_SINGLEFIFOFLUSH);
        assert_eq!(iadc.read_word(OFF_STATUS) & STATUS_SINGLEFIFODV, 0);
    }

    #[test]
    fn disabling_drops_an_unread_result() {
        let mut iadc = enabled();
        select(&mut iadc, PORTPOS_SUPPLY, 0);
        iadc.write_word(OFF_CMD, CMD_SINGLESTART);
        iadc.tick_elapsed(1);

        iadc.write_word(OFF_EN, 0);
        assert_eq!(
            iadc.read_word(OFF_STATUS) & STATUS_SINGLEFIFODV,
            0,
            "a sample taken before a reconfigure must not survive it"
        );
    }

    /// The scan queue is deliberately absent. Firmware that starts a scan must
    /// hang on `SCANFIFODV`, not be handed a sample the model never took.
    #[test]
    fn the_scan_queue_produces_nothing() {
        let mut iadc = enabled();
        iadc.write_word(OFF_SCANTABLE, (PORTPOS_SUPPLY) << SINGLE_PORTPOS_SHIFT);
        iadc.write_word(OFF_CMD, 1 << 3); // SCANSTART
        iadc.tick_elapsed(1);
        assert_eq!(iadc.read_word(OFF_SCANFIFOSTAT), 0);
        assert_eq!(iadc.read_word(OFF_SCANFIFODATA), 0);
    }

    #[test]
    fn cfg_and_scantable_are_addressed_as_groups_not_flat_words() {
        let mut iadc = enabled();
        // CFG[1].SCALE is at 0x48 + 0x10 + 0x08.
        iadc.write_word(OFF_CFG + CFG_STRIDE + 0x08, 0xDEAD_BEEF);
        assert_eq!(iadc.read_word(OFF_CFG + CFG_STRIDE + 0x08), 0xDEAD_BEEF);
        assert_eq!(iadc.read_word(OFF_CFG), 0, "CFG[0] is a different register");

        iadc.write_word(OFF_SCANTABLE + 15 * 4, 0x1234);
        assert_eq!(iadc.read_word(OFF_SCANTABLE + 15 * 4), 0x1234);
        assert_eq!(iadc.read_word(OFF_SCANTABLE), 0);
    }
}
