// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Maxim MAX30102 reflective PPG (heart-rate / SpO2) front-end as an
//! [`I2cDevice`].
//!
//! The MAX30102 is the standard open-hardware pulse-oximetry part (the
//! MAX86141 is its higher-channel-count sibling). A driver talks to it almost
//! exclusively through the **FIFO**: it configures MODE_CONFIG/SPO2_CONFIG,
//! zeroes the three pointer registers, then polls FIFO_WR_PTR (or waits on the
//! A_FULL / PPG_RDY interrupt) and burst-reads FIFO_DATA. Everything a real
//! driver depends on is therefore pointer and FIFO behaviour, and that is what
//! this model implements for real — a 32-deep sample FIFO with write/read
//! pointers, an overflow counter, rollover control and 3-bytes-per-channel
//! 18-bit left-justified sample words.
//!
//! ## Waveform
//!
//! Samples are not canned: [`Max30102::generate_sample`] synthesises a
//! plausible photoplethysmogram — a DC baseline (tissue/ambient reflection)
//! with a periodic pulse riding on it, shaped with the characteristic sharp
//! systolic upstroke, the dicrotic notch and the diastolic decay, plus a small
//! amount of noise. The pulse is a *dip* in the raw counts because more blood
//! in the tissue absorbs more light — that is the polarity a real reflective
//! PPG front-end reports, and drivers that AC-couple and peak-detect see it.
//!
//! All of it is integer math off a seeded LCG: no floating point, no libm, so a
//! given (seed, bpm, perfusion, sample-rate) tuple produces byte-identical
//! samples on every run and every host.
//!
//! ## Time base — read this before wiring the model into a lab
//!
//! [`crate::peripherals::i2c::I2cDevice`] exposes no simulation clock (unlike
//! [`crate::Peripheral`], slaves get neither `tick` nor `attach_cycle_clock`),
//! so this model cannot consult the machine's cycle counter. Sample production
//! is therefore driven two ways:
//!
//! * [`Max30102::advance_us`] — the exact, explicit path. Tests, the host
//!   stimulus bridge and any future clock wiring should use this.
//! * One sample period per completed I²C transaction ([`I2cDevice::stop`]),
//!   enabled by default and switchable with
//!   [`Max30102::set_transaction_advance`]. Without it a polling driver would
//!   spin forever on an empty FIFO, because nothing else would ever advance
//!   the sample clock. This is a deliberate stand-in for the missing clock
//!   hook, not silicon behaviour — see the crate-level report.

use crate::peripherals::i2c::I2cDevice;

/// Default 7-bit I²C address. Fixed on the MAX30102 (no address-select pin).
pub const MAX30102_ADDR: u8 = 0x57;

// ── Register map (MAX30102 datasheet, Table "Register Map") ─────────────────
const REG_INTR_STATUS_1: u8 = 0x00;
const REG_INTR_STATUS_2: u8 = 0x01;
const REG_INTR_ENABLE_1: u8 = 0x02;
const REG_INTR_ENABLE_2: u8 = 0x03;
const REG_FIFO_WR_PTR: u8 = 0x04;
const REG_OVF_COUNTER: u8 = 0x05;
const REG_FIFO_RD_PTR: u8 = 0x06;
const REG_FIFO_DATA: u8 = 0x07;
const REG_FIFO_CONFIG: u8 = 0x08;
const REG_MODE_CONFIG: u8 = 0x09;
const REG_SPO2_CONFIG: u8 = 0x0A;
const REG_LED1_PA: u8 = 0x0C;
const REG_LED2_PA: u8 = 0x0D;
const REG_MULTI_LED_1: u8 = 0x11;
const REG_MULTI_LED_2: u8 = 0x12;
const REG_TEMP_INTR: u8 = 0x1F;
const REG_TEMP_FRAC: u8 = 0x20;
const REG_TEMP_CONFIG: u8 = 0x21;
const REG_REV_ID: u8 = 0xFE;
const REG_PART_ID: u8 = 0xFF;

/// PART_ID reads 0x15 on every MAX30102 — the value drivers probe for.
pub const PART_ID_VALUE: u8 = 0x15;

/// REV_ID. The datasheet documents the register but not a fixed value (it is
/// die-revision dependent), so 0x00 is a MODEL CHOICE, not a datasheet value.
/// No driver should branch on it; the probe register is PART_ID.
const REV_ID_VALUE: u8 = 0x00;

// INTR_STATUS_1 bits.
const INTR1_A_FULL: u8 = 0x80;
const INTR1_PPG_RDY: u8 = 0x40;
const INTR1_PWR_RDY: u8 = 0x01;
// INTR_STATUS_2 bits.
const INTR2_DIE_TEMP_RDY: u8 = 0x02;

// FIFO_CONFIG bits: SMP_AVE[7:5], FIFO_ROLLOVER_EN[4], FIFO_A_FULL[3:0].
const FIFO_ROLLOVER_EN: u8 = 0x10;

// MODE_CONFIG bits: SHDN[7], RESET[6], MODE[2:0].
const MODE_SHDN: u8 = 0x80;
const MODE_RESET: u8 = 0x40;
const MODE_HR: u8 = 0x02;
const MODE_SPO2: u8 = 0x03;
const MODE_MULTI_LED: u8 = 0x07;

/// FIFO depth in samples. 32 on the MAX30102.
pub const FIFO_DEPTH: usize = 32;

/// ADC full scale: samples are 18-bit, left-justified into 3 bytes.
const SAMPLE_MASK: u32 = 0x0003_FFFF;

/// Nominal DC level of the IR channel, in ADC counts. Mid-scale-ish for a
/// finger/wrist contact at a typical LED current — a modelling choice, chosen
/// so the AC swing at realistic perfusion never clips the 18-bit range.
const DC_IR: i32 = 100_000;
/// Nominal DC level of the red channel. Lower than IR because tissue absorbs
/// red more strongly; the ratio-of-ratios a SpO2 driver computes therefore
/// lands in a believable range.
const DC_RED: i32 = 80_000;

/// Peak-to-peak noise, in ADC counts, added to every sample.
const NOISE_COUNTS: i32 = 96;

/// Piecewise-linear PPG pulse shape over one cardiac cycle. `x` is the phase in
/// 1/1024ths of a beat, `y` is the normalised pulse magnitude in 1/1000ths.
/// The knots encode: a sharp systolic upstroke (0 → peak in ~11 % of the
/// cycle), the rapid systolic decline, the dicrotic notch (the small dip as the
/// aortic valve closes), the dicrotic wave, and the diastolic runout.
const PULSE_KNOTS: [(u32, i32); 7] = [
    (0, 0),      // start of cycle, baseline
    (110, 1000), // systolic peak
    (300, 380),  // rapid decline
    (380, 300),  // dicrotic notch (trough)
    (450, 430),  // dicrotic wave (secondary peak)
    (620, 150),  // diastolic decay
    (1024, 0),   // back to baseline
];

/// Pulse magnitude (0..=1000) at `phase` (0..=1023) within a cardiac cycle.
fn pulse_shape(phase: u32) -> i32 {
    let p = phase & 0x3FF;
    let mut i = 0usize;
    while i + 1 < PULSE_KNOTS.len() && p >= PULSE_KNOTS[i + 1].0 {
        i += 1;
    }
    let (p0, v0) = PULSE_KNOTS[i];
    let (p1, v1) = PULSE_KNOTS[i + 1];
    let span = (p1 - p0) as i32;
    v0 + (v1 - v0) * (p - p0) as i32 / span
}

/// Maxim MAX30102 PPG front-end.
#[derive(Debug)]
pub struct Max30102 {
    address: u8,
    /// I²C register pointer.
    pointer: u8,
    /// False until the first byte after a START has selected the register.
    pointer_set: bool,

    // ── Register file ───────────────────────────────────────────────────────
    intr_status_1: u8,
    intr_status_2: u8,
    intr_enable_1: u8,
    intr_enable_2: u8,
    fifo_config: u8,
    mode_config: u8,
    spo2_config: u8,
    led1_pa: u8,
    led2_pa: u8,
    multi_led_1: u8,
    multi_led_2: u8,
    temp_intr: u8,
    temp_frac: u8,
    temp_config: u8,

    // ── FIFO ────────────────────────────────────────────────────────────────
    /// `[red, ir]` per slot. Only slot `[0]` is meaningful in HR mode.
    fifo: [[u32; 2]; FIFO_DEPTH],
    wr_ptr: u8,
    rd_ptr: u8,
    /// Unread samples. Kept alongside the pointers so a genuinely FULL FIFO
    /// (32 samples, `wr_ptr == rd_ptr`) is distinguishable from an empty one —
    /// exactly the ambiguity real firmware resolves via OVF_COUNTER.
    count: usize,
    ovf_counter: u8,
    /// Byte cursor inside the sample currently being burst-read out of
    /// FIFO_DATA. Advancing past the last byte of a sample pops it.
    fifo_byte: usize,

    // ── Waveform generator ──────────────────────────────────────────────────
    /// Cardiac phase accumulator, 20-bit fixed point (1 << 20 == one beat).
    phase: u32,
    /// Heart rate in hundredths of a bpm (integer math only).
    bpm_centi: u32,
    /// Perfusion index (AC/DC ratio) in hundredths of a percent.
    perfusion_centi: u32,
    /// LCG state. Seeded deterministically so runs reproduce byte-for-byte.
    rng: u32,
    /// Initial seed, retained so [`Max30102::reset_waveform`] can rewind.
    seed: u32,
    /// Samples produced since power-on (monotonic; survives FIFO pops).
    samples_produced: u64,
    /// Sub-sample-period remainder of the virtual clock, in microseconds.
    accum_us: u64,
    /// Advance one sample period on every completed I²C transaction. See the
    /// module docs — a stand-in for the clock hook slaves do not get.
    transaction_advance: bool,

    /// system.yaml `external_devices` id, stamped at attach (see
    /// [`crate::sim_input::SimInput::component_id`]).
    component_id: Option<String>,
}

impl Default for Max30102 {
    fn default() -> Self {
        Self::new(MAX30102_ADDR)
    }
}

impl Max30102 {
    pub fn new(address: u8) -> Self {
        Self {
            address,
            pointer: 0,
            pointer_set: false,

            // Power-on: PWR_RDY latched, everything else zero (datasheet reset
            // values are 0x00 for the whole register file except PART_ID).
            intr_status_1: INTR1_PWR_RDY,
            intr_status_2: 0,
            intr_enable_1: 0,
            intr_enable_2: 0,
            fifo_config: 0,
            mode_config: 0,
            spo2_config: 0,
            led1_pa: 0,
            led2_pa: 0,
            multi_led_1: 0,
            multi_led_2: 0,
            temp_intr: 0,
            temp_frac: 0,
            temp_config: 0,

            fifo: [[0; 2]; FIFO_DEPTH],
            wr_ptr: 0,
            rd_ptr: 0,
            count: 0,
            ovf_counter: 0,
            fifo_byte: 0,

            phase: 0,
            bpm_centi: 7200,       // 72.00 bpm resting default
            perfusion_centi: 1500, // 1.50 % perfusion index
            rng: 0x1234_5678,
            seed: 0x1234_5678,
            samples_produced: 0,
            accum_us: 0,
            transaction_advance: true,

            component_id: None,
        }
    }

    /// Override the waveform seed. Two devices with the same seed and the same
    /// configuration produce identical sample streams.
    pub fn with_seed(mut self, seed: u32) -> Self {
        self.seed = seed;
        self.rng = seed;
        self
    }

    /// Set the initial heart rate, in bpm (also drivable at runtime through
    /// [`crate::sim_input::SimInput`]).
    pub fn with_heart_rate_bpm(mut self, bpm: f64) -> Self {
        self.bpm_centi = (bpm.clamp(20.0, 250.0) * 100.0).round() as u32;
        self
    }

    /// Enable/disable the "one sample period per I²C transaction" fallback
    /// clock. Tests that want an exact number of samples turn it off and drive
    /// [`Max30102::advance_us`] themselves.
    pub fn set_transaction_advance(&mut self, on: bool) {
        self.transaction_advance = on;
    }

    /// Configured sample rate in Hz, from SPO2_CONFIG SPO2_SR[4:2].
    pub fn sample_rate_hz(&self) -> u32 {
        match (self.spo2_config >> 2) & 0x07 {
            0 => 50,
            1 => 100,
            2 => 200,
            3 => 400,
            4 => 800,
            5 => 1000,
            6 => 1600,
            _ => 3200,
        }
    }

    /// Sample period in microseconds.
    pub fn sample_period_us(&self) -> u64 {
        1_000_000 / self.sample_rate_hz() as u64
    }

    /// Number of ADC channels the configured MODE stores per FIFO slot: 1 in
    /// heart-rate mode, 2 in SpO2 / multi-LED mode, 0 when no mode is selected
    /// or the part is shut down (in which case the FIFO does not fill — which
    /// is exactly what silicon does, and what an unconfigured driver sees).
    pub fn active_channels(&self) -> usize {
        if self.mode_config & MODE_SHDN != 0 {
            return 0;
        }
        match self.mode_config & 0x07 {
            MODE_HR => 1,
            MODE_SPO2 | MODE_MULTI_LED => 2,
            _ => 0,
        }
    }

    /// Bytes FIFO_DATA serves per sample: 3 per active channel.
    fn bytes_per_sample(&self) -> usize {
        3 * self.active_channels()
    }

    /// Unread samples in the FIFO.
    pub fn fifo_occupancy(&self) -> usize {
        self.count
    }

    /// Value of OVF_COUNTER (samples lost / overwritten, saturating at 31 like
    /// the 5-bit silicon counter).
    pub fn overflow_counter(&self) -> u8 {
        self.ovf_counter
    }

    /// Total samples generated since power-on.
    pub fn samples_produced(&self) -> u64 {
        self.samples_produced
    }

    /// Current heart rate in bpm.
    pub fn heart_rate_bpm(&self) -> f64 {
        self.bpm_centi as f64 / 100.0
    }

    /// Current perfusion index (AC/DC) as a percentage.
    pub fn perfusion_percent(&self) -> f64 {
        self.perfusion_centi as f64 / 100.0
    }

    /// Rewind the waveform generator to its seed. Used by tests that assert
    /// determinism; not something silicon does.
    pub fn reset_waveform(&mut self) {
        self.rng = self.seed;
        self.phase = 0;
        self.samples_produced = 0;
    }

    // ── Sample production ───────────────────────────────────────────────────

    /// Advance the virtual sample clock by `us` microseconds, producing (and
    /// FIFO-pushing) every sample that falls in the interval.
    pub fn advance_us(&mut self, us: u64) {
        let period = self.sample_period_us().max(1);
        self.accum_us += us;
        // Bound the burst so a pathological `advance_us(u64::MAX)` cannot hang
        // the machine; anything beyond a full FIFO is unobservable anyway.
        let mut budget = FIFO_DEPTH as u64 * 4;
        while self.accum_us >= period && budget > 0 {
            self.accum_us -= period;
            budget -= 1;
            self.produce_sample();
        }
        if budget == 0 {
            self.accum_us %= period;
        }
    }

    /// Advance by exactly `n` sample periods.
    pub fn advance_samples(&mut self, n: usize) {
        for _ in 0..n {
            self.produce_sample();
        }
    }

    /// Next pseudo-random value. Numerical Recipes LCG — cheap, integer-only
    /// and identical on every platform.
    fn next_rand(&mut self) -> u32 {
        self.rng = self.rng.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        self.rng
    }

    /// Synthesise one `(red, ir)` sample pair at the current cardiac phase and
    /// advance the phase. Pure integer math.
    fn generate_sample(&mut self) -> (u32, u32) {
        let fs = self.sample_rate_hz() as u64;
        // Phase step for one sample: (1 << 20) beats-worth scaled by
        // bpm / (60 * fs). bpm is in hundredths, hence the 6000.
        let step = ((1u64 << 20) * self.bpm_centi as u64 / (6000 * fs)) as u32;
        let phase10 = self.phase >> 10; // 0..1023
        let shape = pulse_shape(phase10); // 0..1000
        self.phase = self.phase.wrapping_add(step) & 0x000F_FFFF;

        let mut out = [0u32; 2];
        for (i, dc) in [DC_RED, DC_IR].iter().enumerate() {
            // AC amplitude = perfusion index × DC.
            let ac = (*dc as i64 * self.perfusion_centi as i64 / 10_000) as i32;
            // More blood → more absorption → FEWER reflected counts, so the
            // systolic peak is a trough in the raw signal.
            let mut v = dc - (ac * shape / 1000);
            let r = self.next_rand();
            v += (r >> 20) as i32 % (NOISE_COUNTS + 1) - NOISE_COUNTS / 2;
            out[i] = v.clamp(0, SAMPLE_MASK as i32) as u32;
        }
        (out[0], out[1])
    }

    /// Produce one sample and push it into the FIFO, honouring rollover and
    /// updating the overflow counter and interrupt status exactly as the part
    /// does. A no-op when no MODE is selected (the FIFO does not fill).
    fn produce_sample(&mut self) {
        if self.active_channels() == 0 {
            return;
        }
        let (red, ir) = self.generate_sample();
        self.samples_produced += 1;

        if self.count == FIFO_DEPTH {
            // FIFO full. OVF_COUNTER counts the lost samples either way; with
            // FIFO_ROLLOVER_EN the oldest sample is overwritten, without it the
            // new sample is discarded (datasheet FIFO_CONFIG description).
            self.ovf_counter = (self.ovf_counter + 1).min(0x1F);
            self.intr_status_1 |= INTR1_A_FULL;
            if self.fifo_config & FIFO_ROLLOVER_EN == 0 {
                return;
            }
            self.rd_ptr = (self.rd_ptr + 1) % FIFO_DEPTH as u8;
            self.count -= 1;
            self.fifo_byte = 0;
        }

        self.fifo[self.wr_ptr as usize] = [red & SAMPLE_MASK, ir & SAMPLE_MASK];
        self.wr_ptr = (self.wr_ptr + 1) % FIFO_DEPTH as u8;
        self.count += 1;
        self.intr_status_1 |= INTR1_PPG_RDY;

        // A_FULL asserts once the number of EMPTY slots has fallen to the
        // FIFO_A_FULL[3:0] threshold.
        let empty = FIFO_DEPTH - self.count;
        if empty <= (self.fifo_config & 0x0F) as usize {
            self.intr_status_1 |= INTR1_A_FULL;
        }
    }

    /// Serve one byte out of FIFO_DATA, advancing the byte cursor and popping
    /// the sample (and advancing FIFO_RD_PTR) once it is fully read.
    fn read_fifo_data(&mut self) -> u8 {
        let per_sample = self.bytes_per_sample();
        if per_sample == 0 || self.count == 0 {
            // Reading an empty FIFO returns zeros; silicon serves stale data,
            // but no driver may rely on either (it checks the pointers first).
            return 0;
        }
        let sample = self.fifo[self.rd_ptr as usize];
        let ch = self.fifo_byte / 3;
        let within = self.fifo_byte % 3;
        // 18-bit value, left-justified into 3 bytes: the top byte carries only
        // bits [17:16], so its upper 6 bits read back as zero.
        let word = sample[ch] & SAMPLE_MASK;
        let byte = ((word >> (8 * (2 - within))) & 0xFF) as u8;

        self.fifo_byte += 1;
        if self.fifo_byte >= per_sample {
            self.fifo_byte = 0;
            self.rd_ptr = (self.rd_ptr + 1) % FIFO_DEPTH as u8;
            self.count -= 1;
        }
        byte
    }

    fn read_register(&mut self, reg: u8) -> u8 {
        match reg {
            REG_INTR_STATUS_1 => {
                // Interrupt flags are cleared by reading the status register.
                let v = self.intr_status_1;
                self.intr_status_1 = 0;
                v
            }
            REG_INTR_STATUS_2 => {
                let v = self.intr_status_2;
                self.intr_status_2 = 0;
                v
            }
            REG_INTR_ENABLE_1 => self.intr_enable_1,
            REG_INTR_ENABLE_2 => self.intr_enable_2,
            REG_FIFO_WR_PTR => self.wr_ptr & 0x1F,
            REG_OVF_COUNTER => self.ovf_counter & 0x1F,
            REG_FIFO_RD_PTR => self.rd_ptr & 0x1F,
            REG_FIFO_DATA => self.read_fifo_data(),
            REG_FIFO_CONFIG => self.fifo_config,
            REG_MODE_CONFIG => self.mode_config,
            REG_SPO2_CONFIG => self.spo2_config,
            REG_LED1_PA => self.led1_pa,
            REG_LED2_PA => self.led2_pa,
            REG_MULTI_LED_1 => self.multi_led_1,
            REG_MULTI_LED_2 => self.multi_led_2,
            REG_TEMP_INTR => self.temp_intr,
            REG_TEMP_FRAC => self.temp_frac,
            REG_TEMP_CONFIG => self.temp_config,
            REG_REV_ID => REV_ID_VALUE,
            REG_PART_ID => PART_ID_VALUE,
            _ => 0,
        }
    }

    fn write_register(&mut self, reg: u8, value: u8) {
        match reg {
            REG_INTR_ENABLE_1 => self.intr_enable_1 = value,
            REG_INTR_ENABLE_2 => self.intr_enable_2 = value,
            REG_FIFO_WR_PTR => {
                self.wr_ptr = value & 0x1F;
                self.count = 0;
                self.fifo_byte = 0;
                self.rd_ptr = self.wr_ptr;
            }
            REG_OVF_COUNTER => self.ovf_counter = value & 0x1F,
            REG_FIFO_RD_PTR => {
                self.rd_ptr = value & 0x1F;
                self.count = 0;
                self.fifo_byte = 0;
                self.wr_ptr = self.rd_ptr;
            }
            REG_FIFO_CONFIG => self.fifo_config = value,
            REG_MODE_CONFIG => {
                if value & MODE_RESET != 0 {
                    self.power_on_reset();
                    return;
                }
                self.mode_config = value;
            }
            REG_SPO2_CONFIG => self.spo2_config = value,
            REG_LED1_PA => self.led1_pa = value,
            REG_LED2_PA => self.led2_pa = value,
            REG_MULTI_LED_1 => self.multi_led_1 = value,
            REG_MULTI_LED_2 => self.multi_led_2 = value,
            REG_TEMP_CONFIG => {
                self.temp_config = value;
                if value & 0x01 != 0 {
                    // TEMP_EN: one-shot die-temperature conversion. The die
                    // temperature is not modelled thermally; a fixed 30.0625 °C
                    // (integer 30, 1/16ths = 1) stands in, which is what a
                    // driver's ambient-compensation path consumes.
                    self.temp_intr = 30;
                    self.temp_frac = 1;
                    self.temp_config &= !0x01; // TEMP_EN self-clears
                    self.intr_status_2 |= INTR2_DIE_TEMP_RDY;
                }
            }
            _ => {}
        }
    }

    /// MODE_CONFIG.RESET: all registers back to their power-on values.
    fn power_on_reset(&mut self) {
        let addr = self.address;
        let seed = self.seed;
        let bpm = self.bpm_centi;
        let perfusion = self.perfusion_centi;
        let advance = self.transaction_advance;
        let id = self.component_id.take();
        *self = Self::new(addr);
        self.seed = seed;
        self.rng = seed;
        self.bpm_centi = bpm;
        self.perfusion_centi = perfusion;
        self.transaction_advance = advance;
        self.component_id = id;
    }
}

impl I2cDevice for Max30102 {
    fn address(&self) -> u8 {
        self.address
    }

    fn read(&mut self) -> u8 {
        let reg = self.pointer;
        let val = self.read_register(reg);
        // FIFO_DATA is a streaming port: a burst read keeps returning FIFO
        // bytes rather than walking into FIFO_CONFIG. Every other register
        // auto-increments the pointer.
        if reg != REG_FIFO_DATA {
            self.pointer = self.pointer.wrapping_add(1);
        }
        val
    }

    fn write(&mut self, data: u8) {
        if !self.pointer_set {
            self.pointer = data;
            self.pointer_set = true;
        } else {
            self.write_register(self.pointer, data);
            self.pointer = self.pointer.wrapping_add(1);
        }
    }

    fn stop(&mut self) {
        self.pointer_set = false;
        if self.transaction_advance {
            let period = self.sample_period_us();
            self.advance_us(period);
        }
    }

    fn as_any(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }

    fn as_any_mut(&mut self) -> Option<&mut dyn std::any::Any> {
        Some(self)
    }

    fn as_sim_input_mut(&mut self) -> Option<&mut dyn crate::sim_input::SimInput> {
        Some(self)
    }
}

/// Drivable PPG channels. `bpm` sets the pulse rate of the synthesised
/// waveform (the periodicity a driver's peak detector recovers); `perfusion`
/// sets the AC/DC ratio — the perfusion index, in percent, which is what
/// controls whether a real driver can find a pulse at all (a weak contact
/// drops it below ~0.2 %). ONE table backs both the `SimInput` impl and any
/// kit metadata, so the schema and the runtime API cannot drift.
pub const INPUT_CHANNELS: &[crate::sim_input::InputChannel] = &[
    crate::sim_input::InputChannel {
        key: "bpm",
        label: "Heart Rate",
        unit: "bpm",
        min: 20.0,
        max: 250.0,
    },
    crate::sim_input::InputChannel {
        key: "perfusion",
        label: "Perfusion Index",
        unit: "%",
        min: 0.0,
        max: 20.0,
    },
];

impl crate::sim_input::SimInput for Max30102 {
    fn input_channels(&self) -> &'static [crate::sim_input::InputChannel] {
        INPUT_CHANNELS
    }

    fn set_input(&mut self, key: &str, value: f64) -> Result<(), crate::sim_input::SimInputError> {
        self.require_channel(key, value)?;
        match key {
            "bpm" => self.bpm_centi = (value * 100.0).round() as u32,
            "perfusion" => self.perfusion_centi = (value * 100.0).round() as u32,
            _ => unreachable!("require_channel validated the key"),
        }
        Ok(())
    }

    fn component_id(&self) -> Option<&str> {
        self.component_id.as_deref()
    }

    fn set_component_id(&mut self, id: String) {
        self.component_id = Some(id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sim_input::SimInput;

    /// Configure a device the way a driver does: SpO2 mode, 100 Hz, pointers
    /// zeroed. Transaction-advance is off so tests own the sample clock.
    fn configured() -> Max30102 {
        let mut d = Max30102::new(MAX30102_ADDR);
        d.set_transaction_advance(false);
        write_reg(&mut d, REG_MODE_CONFIG, MODE_SPO2);
        write_reg(&mut d, REG_SPO2_CONFIG, 0x27); // SPO2_SR=001 (100 Hz), PW=11
        write_reg(&mut d, REG_FIFO_WR_PTR, 0);
        write_reg(&mut d, REG_OVF_COUNTER, 0);
        write_reg(&mut d, REG_FIFO_RD_PTR, 0);
        d
    }

    /// Read `len` bytes starting at `reg` through the real I²C register
    /// interface — the same path firmware takes.
    fn read_regs(dev: &mut Max30102, reg: u8, len: usize) -> Vec<u8> {
        dev.stop();
        dev.write(reg);
        let out = (0..len).map(|_| dev.read()).collect();
        dev.stop();
        out
    }

    fn read_reg(dev: &mut Max30102, reg: u8) -> u8 {
        read_regs(dev, reg, 1)[0]
    }

    fn write_reg(dev: &mut Max30102, reg: u8, value: u8) {
        dev.stop();
        dev.write(reg);
        dev.write(value);
        dev.stop();
    }

    #[test]
    fn part_id_reads_0x15() {
        let mut d = Max30102::default();
        d.set_transaction_advance(false);
        assert_eq!(d.address(), 0x57);
        assert_eq!(read_reg(&mut d, REG_PART_ID), 0x15);
    }

    #[test]
    fn register_write_then_read_back_and_pointer_auto_increments() {
        let mut d = configured();
        write_reg(&mut d, REG_INTR_ENABLE_1, 0xC0);
        write_reg(&mut d, REG_INTR_ENABLE_2, 0x02);
        assert_eq!(read_reg(&mut d, REG_INTR_ENABLE_1), 0xC0);

        // One pointer set, three sequential reads walk 0x02, 0x03, 0x04.
        let regs = read_regs(&mut d, REG_INTR_ENABLE_1, 3);
        assert_eq!(regs[0], 0xC0, "INTR_ENABLE_1");
        assert_eq!(regs[1], 0x02, "INTR_ENABLE_2 via auto-increment");
        assert_eq!(regs[2], 0, "FIFO_WR_PTR via auto-increment");

        // A block write walks the pointer too: LED1_PA then LED2_PA.
        d.stop();
        d.write(REG_LED1_PA);
        d.write(0x24);
        d.write(0x24);
        d.stop();
        assert_eq!(read_reg(&mut d, REG_LED1_PA), 0x24);
        assert_eq!(read_reg(&mut d, REG_LED2_PA), 0x24);
    }

    #[test]
    fn intr_status_clears_on_read() {
        let mut d = configured();
        d.advance_samples(1);
        let s = read_reg(&mut d, REG_INTR_STATUS_1);
        assert_ne!(s & INTR1_PPG_RDY, 0, "PPG_RDY latched by a new sample");
        assert_eq!(
            read_reg(&mut d, REG_INTR_STATUS_1),
            0,
            "status is cleared by the read"
        );
    }

    #[test]
    fn fifo_fills_and_write_pointer_advances() {
        let mut d = configured();
        assert_eq!(read_reg(&mut d, REG_FIFO_WR_PTR), 0);
        d.advance_samples(5);
        assert_eq!(read_reg(&mut d, REG_FIFO_WR_PTR), 5);
        assert_eq!(read_reg(&mut d, REG_FIFO_RD_PTR), 0);
        assert_eq!(d.fifo_occupancy(), 5);
    }

    #[test]
    fn no_samples_produced_until_a_mode_is_selected() {
        let mut d = Max30102::new(MAX30102_ADDR);
        d.set_transaction_advance(false);
        d.advance_samples(10);
        assert_eq!(d.fifo_occupancy(), 0, "MODE=000 → FIFO stays empty");
        write_reg(&mut d, REG_MODE_CONFIG, MODE_HR);
        d.advance_samples(10);
        assert_eq!(d.fifo_occupancy(), 10);
    }

    #[test]
    fn fifo_data_read_advances_the_read_pointer() {
        let mut d = configured();
        d.advance_samples(3);
        // SpO2 mode → 6 bytes per sample. One burst read of 6 bytes pops one.
        let s0 = read_regs(&mut d, REG_FIFO_DATA, 6);
        assert_eq!(s0.len(), 6);
        assert_eq!(read_reg(&mut d, REG_FIFO_RD_PTR), 1);
        assert_eq!(d.fifo_occupancy(), 2);

        // 18-bit left-justified: the top byte only carries bits [17:16].
        assert_eq!(s0[0] & 0xFC, 0, "red MSB has only 2 significant bits");
        assert_eq!(s0[3] & 0xFC, 0, "ir MSB has only 2 significant bits");

        // The burst does NOT walk the register pointer off FIFO_DATA.
        let s1 = read_regs(&mut d, REG_FIFO_DATA, 12);
        assert_eq!(s1.len(), 12);
        assert_eq!(d.fifo_occupancy(), 0);
        assert_eq!(read_reg(&mut d, REG_FIFO_RD_PTR), 3);
    }

    #[test]
    fn hr_mode_serves_three_bytes_per_sample() {
        let mut d = configured();
        write_reg(&mut d, REG_MODE_CONFIG, MODE_HR);
        d.advance_samples(2);
        let bytes = read_regs(&mut d, REG_FIFO_DATA, 3);
        assert_eq!(bytes.len(), 3);
        assert_eq!(d.fifo_occupancy(), 1, "one sample popped after 3 bytes");
    }

    #[test]
    fn read_pointer_wraps_around_the_fifo() {
        let mut d = configured();
        // Park both pointers near the top of the ring, then push past the wrap.
        write_reg(&mut d, REG_FIFO_WR_PTR, 30);
        assert_eq!(read_reg(&mut d, REG_FIFO_RD_PTR), 30);
        d.advance_samples(4);
        assert_eq!(read_reg(&mut d, REG_FIFO_WR_PTR), 2, "30+4 mod 32");
        for _ in 0..4 {
            let _ = read_regs(&mut d, REG_FIFO_DATA, 6);
        }
        assert_eq!(read_reg(&mut d, REG_FIFO_RD_PTR), 2, "read pointer wrapped");
        assert_eq!(d.fifo_occupancy(), 0);
    }

    #[test]
    fn overflow_increments_ovf_counter_and_sets_a_full() {
        let mut d = configured();
        d.advance_samples(FIFO_DEPTH);
        assert_eq!(d.fifo_occupancy(), FIFO_DEPTH);
        assert_eq!(read_reg(&mut d, REG_OVF_COUNTER), 0, "not yet overflowed");

        // Rollover disabled (reset default): further samples are dropped.
        d.advance_samples(3);
        assert_eq!(read_reg(&mut d, REG_OVF_COUNTER), 3);
        assert_eq!(d.fifo_occupancy(), FIFO_DEPTH, "oldest data retained");
        assert_ne!(
            read_reg(&mut d, REG_INTR_STATUS_1) & INTR1_A_FULL,
            0,
            "A_FULL latched on overflow"
        );
        assert_eq!(read_reg(&mut d, REG_FIFO_WR_PTR), 0, "wr wrapped to rd");
    }

    #[test]
    fn rollover_overwrites_oldest_and_advances_read_pointer() {
        let mut d = configured();
        write_reg(&mut d, REG_FIFO_CONFIG, FIFO_ROLLOVER_EN);
        d.advance_samples(FIFO_DEPTH + 4);
        assert_eq!(read_reg(&mut d, REG_OVF_COUNTER), 4);
        assert_eq!(d.fifo_occupancy(), FIFO_DEPTH);
        assert_eq!(read_reg(&mut d, REG_FIFO_RD_PTR), 4, "oldest 4 overwritten");
    }

    #[test]
    fn a_full_threshold_from_fifo_config() {
        let mut d = configured();
        // FIFO_A_FULL = 4 → interrupt when only 4 slots remain empty.
        write_reg(&mut d, REG_FIFO_CONFIG, 0x04);
        d.advance_samples(27);
        assert_eq!(read_reg(&mut d, REG_INTR_STATUS_1) & INTR1_A_FULL, 0);
        d.advance_samples(1); // 28 stored → 4 empty
        assert_ne!(read_reg(&mut d, REG_INTR_STATUS_1) & INTR1_A_FULL, 0);
    }

    /// Capture `n` IR samples through the real FIFO_DATA path.
    fn capture_ir(d: &mut Max30102, n: usize) -> Vec<i64> {
        let mut ir = Vec::with_capacity(n);
        for _ in 0..n {
            d.advance_samples(1);
            let b = read_regs(d, REG_FIFO_DATA, 6);
            ir.push(((b[3] as u32) << 16 | (b[4] as u32) << 8 | b[5] as u32) as i64);
        }
        ir
    }

    /// Recover the beat period, in samples, from the raw IR channel: threshold
    /// the signal a quarter of the way down from the baseline and take the
    /// centre of each run below it — the systolic dips. Noise-robust, and the
    /// same thing a driver's peak detector does.
    fn beat_period_samples(d: &mut Max30102, n: usize) -> f64 {
        let ir = capture_ir(d, n);
        let lo = *ir.iter().min().unwrap();
        let hi = *ir.iter().max().unwrap();
        let threshold = lo + (hi - lo) / 4;

        let mut centres: Vec<usize> = Vec::new();
        let mut run: Option<(usize, usize)> = None;
        for (i, &v) in ir.iter().enumerate() {
            if v <= threshold {
                run = Some(match run {
                    Some((start, _)) => (start, i),
                    None => (i, i),
                });
            } else if let Some((start, end)) = run.take() {
                centres.push((start + end) / 2);
            }
        }
        assert!(
            centres.len() >= 3,
            "expected several beats, got {centres:?}"
        );
        let span = centres[centres.len() - 1] - centres[0];
        span as f64 / (centres.len() - 1) as f64
    }

    #[test]
    fn heart_rate_input_changes_waveform_periodicity() {
        // 100 Hz sample rate: 60 bpm → 100 samples/beat, 120 bpm → 50.
        let mut slow = configured();
        slow.set_input("bpm", 60.0).unwrap();
        let p_slow = beat_period_samples(&mut slow, 600);
        assert!(
            (p_slow - 100.0).abs() < 4.0,
            "60 bpm at 100 Hz ≈ 100 samples/beat, got {p_slow}"
        );

        let mut fast = configured();
        fast.set_input("bpm", 120.0).unwrap();
        let p_fast = beat_period_samples(&mut fast, 600);
        assert!(
            (p_fast - 50.0).abs() < 3.0,
            "120 bpm at 100 Hz ≈ 50 samples/beat, got {p_fast}"
        );
        assert!(p_fast < p_slow, "faster heart rate → shorter beat period");
    }

    #[test]
    fn perfusion_input_scales_the_ac_amplitude() {
        let collect = |perfusion: f64| -> i64 {
            let mut d = configured();
            d.set_input("perfusion", perfusion).unwrap();
            let ir = capture_ir(&mut d, 300);
            ir.iter().max().unwrap() - ir.iter().min().unwrap()
        };
        let weak = collect(0.5);
        let strong = collect(4.0);
        assert!(
            strong > weak * 4,
            "8× perfusion → ~8× AC swing (weak={weak}, strong={strong})"
        );
    }

    #[test]
    fn same_seed_produces_identical_samples() {
        let stream = |seed: u32| -> Vec<u8> {
            let mut d = Max30102::new(MAX30102_ADDR).with_seed(seed);
            d.set_transaction_advance(false);
            write_reg(&mut d, REG_MODE_CONFIG, MODE_SPO2);
            write_reg(&mut d, REG_SPO2_CONFIG, 0x27);
            write_reg(&mut d, REG_FIFO_WR_PTR, 0);
            d.advance_samples(16);
            read_regs(&mut d, REG_FIFO_DATA, 16 * 6)
        };
        let a = stream(0x1234_5678);
        let b = stream(0x1234_5678);
        assert_eq!(a, b, "same seed → byte-identical sample stream");
        let c = stream(0xDEAD_BEEF);
        assert_ne!(a, c, "a different seed perturbs the noise");
    }

    #[test]
    fn waveform_is_a_dip_with_a_dicrotic_notch() {
        // Zero-noise check of the shape itself: the systolic peak must be the
        // deepest point and the dicrotic notch a local recovery after it.
        assert_eq!(pulse_shape(0), 0);
        assert_eq!(pulse_shape(110), 1000, "systolic peak");
        let notch = pulse_shape(380);
        assert!(
            notch < pulse_shape(300) && notch < pulse_shape(450),
            "dicrotic notch is a local minimum between two maxima"
        );
    }

    #[test]
    fn sample_rate_follows_spo2_config() {
        let mut d = configured();
        write_reg(&mut d, REG_SPO2_CONFIG, 0x00);
        assert_eq!(d.sample_rate_hz(), 50);
        write_reg(&mut d, REG_SPO2_CONFIG, 0x1C); // SPO2_SR = 111
        assert_eq!(d.sample_rate_hz(), 3200);
        assert_eq!(d.sample_period_us(), 312);
    }

    #[test]
    fn advance_us_produces_one_sample_per_period() {
        let mut d = configured(); // 100 Hz → 10 000 µs per sample
        d.advance_us(10_000 * 7);
        assert_eq!(d.fifo_occupancy(), 7);
        d.advance_us(4_000);
        assert_eq!(d.fifo_occupancy(), 7, "partial period produces nothing");
        d.advance_us(6_000);
        assert_eq!(d.fifo_occupancy(), 8, "remainder carries over");
    }

    #[test]
    fn temp_conversion_sets_die_temp_rdy() {
        let mut d = configured();
        write_reg(&mut d, REG_TEMP_CONFIG, 0x01);
        assert_ne!(read_reg(&mut d, REG_INTR_STATUS_2) & INTR2_DIE_TEMP_RDY, 0);
        assert_eq!(read_reg(&mut d, REG_TEMP_INTR), 30);
        assert_eq!(read_reg(&mut d, REG_TEMP_FRAC), 1);
        assert_eq!(
            read_reg(&mut d, REG_TEMP_CONFIG) & 0x01,
            0,
            "TEMP_EN clears"
        );
    }

    #[test]
    fn mode_config_reset_clears_the_register_file() {
        let mut d = configured();
        write_reg(&mut d, REG_LED1_PA, 0x7F);
        d.advance_samples(4);
        write_reg(&mut d, REG_MODE_CONFIG, MODE_RESET);
        assert_eq!(read_reg(&mut d, REG_LED1_PA), 0);
        assert_eq!(read_reg(&mut d, REG_MODE_CONFIG), 0);
        assert_eq!(d.fifo_occupancy(), 0);
        assert_eq!(read_reg(&mut d, REG_PART_ID), 0x15);
    }

    #[test]
    fn transaction_advance_feeds_a_polling_driver() {
        let mut d = Max30102::default();
        write_reg(&mut d, REG_MODE_CONFIG, MODE_SPO2);
        write_reg(&mut d, REG_FIFO_WR_PTR, 0);
        // A driver that just polls the write pointer must eventually see data.
        let mut wr = 0;
        for _ in 0..8 {
            wr = read_reg(&mut d, REG_FIFO_WR_PTR);
        }
        assert!(wr > 0, "polling advances the sample clock");
    }

    #[test]
    fn sim_input_rejects_out_of_range_and_unknown_channels() {
        let mut d = configured();
        assert!(d.set_input("bpm", 300.0).is_err());
        assert!(d.set_input("spo2", 98.0).is_err());
        assert!(d.set_input("bpm", 88.0).is_ok());
        assert_eq!(d.heart_rate_bpm(), 88.0);
    }

    #[test]
    fn component_id_round_trips() {
        let mut d = configured();
        assert!(SimInput::component_id(&d).is_none());
        d.set_component_id("ppg".to_string());
        assert_eq!(SimInput::component_id(&d), Some("ppg"));
    }
}
