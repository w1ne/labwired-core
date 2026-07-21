// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Texas Instruments DRV2605L LRA/ERM haptic driver as an [`I2cDevice`].
//!
//! The DRV2605L is the standard part behind a vibration motor on a wearable.
//! The firmware sequence is always the same: read the Status register (its top
//! three bits are the DEVICE_ID, which is how a driver confirms it is talking
//! to a DRV2605L rather than a DRV2604), take the part out of standby by
//! writing MODE, select an effect library, queue one to eight effects in the
//! waveform sequencer, then write `GO = 1`. The part plays the sequence and
//! clears GO when it finishes.
//!
//! ## What this model observes
//!
//! Modelling a motor is only worth anything if a test — and the fidelity /
//! oracle layer — can assert *"the motor was actually commanded on"*. That is
//! what [`Drv2605::is_driven`], [`Drv2605::amplitude`],
//! [`Drv2605::total_drive_time_us`] and [`Drv2605::sequences_played`] expose,
//! following the same "read the captured actuator command back off the model"
//! convention as [`crate::peripherals::components::Pca9685::channel_off`] and
//! the servo driver.
//!
//! ## Time base
//!
//! [`crate::peripherals::i2c::I2cDevice`] gives slaves no simulation clock, so
//! playback is advanced explicitly with [`Drv2605::advance_us`] (tests, the
//! host stimulus bridge, and any future clock wiring). Nothing advances on its
//! own: a haptic effect started and never stepped stays asserted, which is the
//! honest behaviour — the model never pretends time passed that the caller did
//! not give it.
//!
//! ## Fidelity boundary — READ THIS
//!
//! Effect *durations and amplitudes* for the ROM waveform library are NOT
//! published in the datasheet (TI ships them as a licensed effect library), so
//! the per-effect duration and amplitude below are deterministic MODEL
//! APPROXIMATIONS, not datasheet values. What is faithful is the control
//! surface a driver actually depends on: standby gating, the mode field, the
//! GO handshake, sequencer traversal, the wait-time encoding (sequencer bit 7)
//! and real-time playback.

use crate::peripherals::i2c::I2cDevice;

/// Default (and only) 7-bit I²C address of the DRV2605L.
pub const DRV2605_ADDR: u8 = 0x5A;

// ── Register map (DRV2605L datasheet §7.6, "Register Maps") ─────────────────
const REG_STATUS: u8 = 0x00;
const REG_MODE: u8 = 0x01;
const REG_RTP_INPUT: u8 = 0x02;
const REG_LIBRARY: u8 = 0x03;
/// Waveform Sequencer 1..8 occupy 0x04..=0x0B.
const REG_WAVEFORM_SEQ_BASE: u8 = 0x04;
const REG_GO: u8 = 0x0C;
const REG_ODT: u8 = 0x0D; // overdrive time offset
const REG_SPT: u8 = 0x0E; // sustain time offset, positive
const REG_SNT: u8 = 0x0F; // sustain time offset, negative
const REG_BRT: u8 = 0x10; // brake time offset
const REG_FEEDBACK: u8 = 0x1A;
const REG_CONTROL1: u8 = 0x1B;
const REG_CONTROL2: u8 = 0x1C;
const REG_CONTROL3: u8 = 0x1D;

/// Number of waveform sequencer slots.
pub const SEQUENCER_SLOTS: usize = 8;

/// DEVICE_ID field (Status bits 7:5). The DRV2605L reports 7; 3 is the
/// DRV2605, 4 the DRV2604 and 6 the DRV2604L (DRV2605L datasheet, Status
/// register DEVICE_ID description). Reported here as the raw Status reset
/// value 0xE0. Taken from the datasheet register description; NOT verified
/// against physical silicon in this repo.
pub const DEVICE_ID_DRV2605L: u8 = 7;
const STATUS_RESET: u8 = DEVICE_ID_DRV2605L << 5;

/// MODE bit 6: STANDBY. Set at power-up; firmware must clear it before the
/// part will drive the motor.
const MODE_STANDBY: u8 = 0x40;
/// MODE bit 7: DEV_RESET.
const MODE_DEV_RESET: u8 = 0x80;
/// MODE[2:0] = 0: internal trigger — playback starts on GO.
const MODE_INTERNAL_TRIGGER: u8 = 0x00;
/// MODE[2:0] = 5: real-time playback — the RTP register drives the motor
/// directly, with no GO handshake.
const MODE_RTP: u8 = 0x05;

/// GO bit 0.
const GO_BIT: u8 = 0x01;

/// Waveform sequencer bit 7 turns the slot into a wait, of
/// `(value & 0x7F) × 10 ms`. This encoding IS in the datasheet.
const SEQ_WAIT: u8 = 0x80;
const WAIT_STEP_US: u64 = 10_000;

/// One waveform-sequencer slot's playback, once decoded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Slot {
    /// Drive the motor at `amplitude` for `duration_us`.
    Effect { amplitude: u8, duration_us: u64 },
    /// Idle for `duration_us` (sequencer bit 7 set).
    Wait { duration_us: u64 },
}

/// Playback duration of ROM effect `n`. **Approximation** — see the module
/// docs. Deterministic, in the 30–100 ms band real click/buzz effects occupy.
fn effect_duration_us(effect: u8) -> u64 {
    30_000 + (effect as u64 % 8) * 10_000
}

/// Drive amplitude of ROM effect `n`, 0..=255. **Approximation** — see the
/// module docs. Deterministic and always in the upper half of the range, since
/// every ROM effect is an audible/palpable click rather than a whisper.
fn effect_amplitude(effect: u8) -> u8 {
    128 + ((effect as u32 * 37) % 128) as u8
}

/// TI DRV2605L haptic driver.
#[derive(Debug)]
pub struct Drv2605 {
    address: u8,
    /// I²C register pointer.
    pointer: u8,
    /// False until the first byte after a START has selected the register.
    pointer_set: bool,

    // ── Register file ───────────────────────────────────────────────────────
    status: u8,
    mode: u8,
    rtp_input: u8,
    library: u8,
    sequencer: [u8; SEQUENCER_SLOTS],
    go: u8,
    odt: u8,
    spt: u8,
    snt: u8,
    brt: u8,
    feedback: u8,
    control1: u8,
    control2: u8,
    control3: u8,

    // ── Playback engine ─────────────────────────────────────────────────────
    /// Sequencer slot currently playing, when a sequence is in flight.
    seq_index: Option<usize>,
    /// What that slot decodes to.
    slot: Option<Slot>,
    /// Microseconds left in the current slot.
    remaining_us: u64,
    /// Cumulative time the motor has actually been driven, in microseconds.
    total_drive_us: u64,
    /// Sequences that have run to completion.
    sequences_played: u32,
}

impl Default for Drv2605 {
    fn default() -> Self {
        Self::new(DRV2605_ADDR)
    }
}

impl Drv2605 {
    pub fn new(address: u8) -> Self {
        Self {
            address,
            pointer: 0,
            pointer_set: false,

            status: STATUS_RESET,
            // Datasheet reset values. MODE 0x40 (STANDBY) is the important one
            // — a driver that forgets to clear it gets no vibration, here as on
            // hardware. FEEDBACK 0x36 / CONTROL1 0x93 / CONTROL2 0xF5 /
            // CONTROL3 0xA0 are the values the datasheet register map lists;
            // they are stored and read back but nothing branches on them, and
            // they are NOT verified against physical silicon in this repo.
            mode: MODE_STANDBY,
            rtp_input: 0x00,
            library: 0x00,
            sequencer: [0; SEQUENCER_SLOTS],
            go: 0x00,
            odt: 0x00,
            spt: 0x00,
            snt: 0x00,
            brt: 0x00,
            feedback: 0x36,
            control1: 0x93,
            control2: 0xF5,
            control3: 0xA0,

            seq_index: None,
            slot: None,
            remaining_us: 0,
            total_drive_us: 0,
            sequences_played: 0,
        }
    }

    /// True while the part is in standby (MODE bit 6). No drive in standby.
    pub fn is_standby(&self) -> bool {
        self.mode & MODE_STANDBY != 0
    }

    /// MODE[2:0].
    pub fn mode(&self) -> u8 {
        self.mode & 0x07
    }

    /// **The observable that justifies modelling this part**: is the motor
    /// being driven right now?
    pub fn is_driven(&self) -> bool {
        self.amplitude() > 0
    }

    /// Current drive amplitude, 0..=255 (0 = motor off).
    pub fn amplitude(&self) -> u8 {
        if self.is_standby() {
            return 0;
        }
        if self.mode() == MODE_RTP {
            // Real-time playback drives straight from the RTP register; there
            // is no GO handshake in this mode.
            return self.rtp_input;
        }
        match self.slot {
            Some(Slot::Effect { amplitude, .. }) => amplitude,
            _ => 0,
        }
    }

    /// Cumulative microseconds the motor has been driven since power-on. The
    /// oracle-friendly "did the haptic actually fire, and for how long" signal.
    pub fn total_drive_time_us(&self) -> u64 {
        self.total_drive_us
    }

    /// Waveform sequences played to completion since power-on.
    pub fn sequences_played(&self) -> u32 {
        self.sequences_played
    }

    /// True while a waveform sequence is in flight (GO still set).
    pub fn is_playing(&self) -> bool {
        self.seq_index.is_some()
    }

    /// The effect (or wait) queued in sequencer slot `n` (0-based).
    pub fn sequencer_slot(&self, n: usize) -> u8 {
        self.sequencer.get(n).copied().unwrap_or(0)
    }

    /// Advance playback by `us` microseconds, walking the waveform sequencer
    /// and clearing GO when the sequence completes.
    pub fn advance_us(&mut self, us: u64) {
        let mut left = us;
        while left > 0 {
            // RTP has no timed structure — the motor simply follows the RTP
            // register — so the whole interval is drive time and we are done.
            if !self.is_standby() && self.mode() == MODE_RTP {
                if self.rtp_input > 0 {
                    self.total_drive_us += left;
                }
                return;
            }
            let Some(slot) = self.slot else {
                return; // nothing playing
            };
            let step = left.min(self.remaining_us);
            if matches!(slot, Slot::Effect { .. }) && !self.is_standby() {
                self.total_drive_us += step;
            }
            self.remaining_us -= step;
            left -= step;
            if self.remaining_us == 0 {
                self.next_slot();
            }
            if self.slot.is_none() {
                return; // sequence finished; the rest of the interval is idle
            }
        }
    }

    /// GO was written to 1: start the sequence from slot 0.
    fn start_sequence(&mut self) {
        if self.is_standby() || self.mode() != MODE_INTERNAL_TRIGGER {
            // Standby (or a non-internal-trigger mode) → GO is ignored and
            // self-clears, exactly like silicon that never leaves standby.
            self.go = 0;
            return;
        }
        self.go = GO_BIT;
        self.seq_index = Some(0);
        self.load_slot(0);
    }

    /// Decode sequencer slot `n` and make it current. A zero slot terminates
    /// the sequence (datasheet: writing 0 ends the waveform sequence).
    fn load_slot(&mut self, n: usize) {
        if n >= SEQUENCER_SLOTS {
            self.finish_sequence();
            return;
        }
        let raw = self.sequencer[n];
        if raw == 0 {
            self.finish_sequence();
            return;
        }
        let slot = if raw & SEQ_WAIT != 0 {
            Slot::Wait {
                duration_us: (raw & 0x7F) as u64 * WAIT_STEP_US,
            }
        } else {
            Slot::Effect {
                amplitude: effect_amplitude(raw),
                duration_us: effect_duration_us(raw),
            }
        };
        self.remaining_us = match slot {
            Slot::Effect { duration_us, .. } | Slot::Wait { duration_us } => duration_us,
        };
        self.slot = Some(slot);
        self.seq_index = Some(n);
        if self.remaining_us == 0 {
            // A wait of zero ticks: skip straight on rather than stalling.
            self.next_slot();
        }
    }

    fn next_slot(&mut self) {
        match self.seq_index {
            Some(n) => self.load_slot(n + 1),
            None => self.finish_sequence(),
        }
    }

    /// Sequence complete: GO clears itself, exactly as the part does — which is
    /// how firmware knows the effect finished.
    fn finish_sequence(&mut self) {
        if self.seq_index.is_some() {
            self.sequences_played += 1;
        }
        self.seq_index = None;
        self.slot = None;
        self.remaining_us = 0;
        self.go = 0;
    }

    /// MODE.DEV_RESET: back to power-on state.
    fn device_reset(&mut self) {
        let addr = self.address;
        *self = Self::new(addr);
    }

    fn read_register(&self, reg: u8) -> u8 {
        match reg {
            REG_STATUS => self.status,
            REG_MODE => self.mode,
            REG_RTP_INPUT => self.rtp_input,
            REG_LIBRARY => self.library,
            REG_WAVEFORM_SEQ_BASE..=0x0B => self.sequencer[(reg - REG_WAVEFORM_SEQ_BASE) as usize],
            REG_GO => self.go,
            REG_ODT => self.odt,
            REG_SPT => self.spt,
            REG_SNT => self.snt,
            REG_BRT => self.brt,
            REG_FEEDBACK => self.feedback,
            REG_CONTROL1 => self.control1,
            REG_CONTROL2 => self.control2,
            REG_CONTROL3 => self.control3,
            _ => 0,
        }
    }

    fn write_register(&mut self, reg: u8, value: u8) {
        match reg {
            REG_MODE => {
                if value & MODE_DEV_RESET != 0 {
                    self.device_reset();
                    return;
                }
                self.mode = value;
                if self.is_standby() {
                    // Entering standby aborts any sequence in flight.
                    self.seq_index = None;
                    self.slot = None;
                    self.remaining_us = 0;
                    self.go = 0;
                }
            }
            REG_RTP_INPUT => self.rtp_input = value,
            REG_LIBRARY => self.library = value,
            REG_WAVEFORM_SEQ_BASE..=0x0B => {
                self.sequencer[(reg - REG_WAVEFORM_SEQ_BASE) as usize] = value
            }
            REG_GO => {
                if value & GO_BIT != 0 {
                    if self.go & GO_BIT == 0 {
                        self.start_sequence();
                    }
                } else if self.go & GO_BIT != 0 {
                    // Writing GO = 0 aborts playback (datasheet: clearing GO
                    // stops the current waveform).
                    self.finish_sequence();
                }
            }
            REG_ODT => self.odt = value,
            REG_SPT => self.spt = value,
            REG_SNT => self.snt = value,
            REG_BRT => self.brt = value,
            REG_FEEDBACK => self.feedback = value,
            REG_CONTROL1 => self.control1 = value,
            REG_CONTROL2 => self.control2 = value,
            REG_CONTROL3 => self.control3 = value,
            // Status is read-only (its diagnostic bits are latched by the part).
            _ => {}
        }
    }
}

impl I2cDevice for Drv2605 {
    fn address(&self) -> u8 {
        self.address
    }

    fn read(&mut self) -> u8 {
        let val = self.read_register(self.pointer);
        self.pointer = self.pointer.wrapping_add(1);
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

    /// Read `len` bytes starting at `reg` through the real I²C register
    /// interface — the same path firmware takes.
    fn read_regs(dev: &mut Drv2605, reg: u8, len: usize) -> Vec<u8> {
        dev.stop();
        dev.write(reg);
        let out = (0..len).map(|_| dev.read()).collect();
        dev.stop();
        out
    }

    fn read_reg(dev: &mut Drv2605, reg: u8) -> u8 {
        read_regs(dev, reg, 1)[0]
    }

    fn write_reg(dev: &mut Drv2605, reg: u8, value: u8) {
        dev.stop();
        dev.write(reg);
        dev.write(value);
        dev.stop();
    }

    /// Bring the part up the way a driver does: out of standby, internal
    /// trigger mode, library 1 selected.
    fn ready() -> Drv2605 {
        let mut d = Drv2605::default();
        write_reg(&mut d, REG_MODE, MODE_INTERNAL_TRIGGER);
        write_reg(&mut d, REG_LIBRARY, 0x01);
        d
    }

    #[test]
    fn status_reports_the_drv2605l_device_id() {
        let mut d = Drv2605::default();
        assert_eq!(d.address(), 0x5A);
        let status = read_reg(&mut d, REG_STATUS);
        assert_eq!(status >> 5, DEVICE_ID_DRV2605L, "DEVICE_ID = 7 (DRV2605L)");
        assert_eq!(status, 0xE0);
    }

    #[test]
    fn powers_up_in_standby() {
        let mut d = Drv2605::default();
        assert_eq!(read_reg(&mut d, REG_MODE), 0x40);
        assert!(d.is_standby());
        assert!(!d.is_driven());
    }

    #[test]
    fn register_write_then_read_back_and_pointer_auto_increments() {
        let mut d = ready();
        write_reg(&mut d, REG_FEEDBACK, 0xB6);
        assert_eq!(read_reg(&mut d, REG_FEEDBACK), 0xB6);

        // The classic driver init: one block write filling the 8 sequencer
        // slots off the auto-incrementing pointer.
        d.stop();
        d.write(REG_WAVEFORM_SEQ_BASE);
        for v in 1..=8u8 {
            d.write(v);
        }
        d.stop();
        assert_eq!(
            read_regs(&mut d, REG_WAVEFORM_SEQ_BASE, 8),
            (1..=8).collect::<Vec<u8>>()
        );

        // Control registers read back their datasheet reset values in a burst.
        assert_eq!(read_regs(&mut d, REG_CONTROL1, 3), vec![0x93, 0xF5, 0xA0]);
    }

    #[test]
    fn go_drives_the_motor_and_clears_when_the_sequence_completes() {
        let mut d = ready();
        write_reg(&mut d, REG_WAVEFORM_SEQ_BASE, 1); // one effect
        write_reg(&mut d, REG_WAVEFORM_SEQ_BASE + 1, 0); // terminator
        assert!(!d.is_driven());

        write_reg(&mut d, REG_GO, 1);
        assert_eq!(read_reg(&mut d, REG_GO), 1, "GO stays set while playing");
        assert!(d.is_driven(), "the motor is commanded on");
        assert!(d.amplitude() > 0);
        assert!(d.is_playing());

        let dur = effect_duration_us(1);
        d.advance_us(dur / 2);
        assert!(d.is_driven(), "still mid-effect");
        assert_eq!(read_reg(&mut d, REG_GO), 1);

        d.advance_us(dur / 2);
        assert!(!d.is_driven(), "effect finished → motor off");
        assert_eq!(read_reg(&mut d, REG_GO), 0, "GO self-clears");
        assert!(!d.is_playing());
        assert_eq!(d.sequences_played(), 1);
        assert_eq!(d.total_drive_time_us(), dur);
    }

    #[test]
    fn sequencer_plays_every_queued_effect() {
        let mut d = ready();
        for (i, effect) in [1u8, 2, 3].iter().enumerate() {
            write_reg(&mut d, REG_WAVEFORM_SEQ_BASE + i as u8, *effect);
        }
        write_reg(&mut d, REG_GO, 1);
        let total: u64 = [1u8, 2, 3].iter().map(|&e| effect_duration_us(e)).sum();
        d.advance_us(total - 1);
        assert!(d.is_playing(), "sequence not finished yet");
        d.advance_us(1);
        assert!(!d.is_playing());
        assert_eq!(d.total_drive_time_us(), total);
        assert_eq!(read_reg(&mut d, REG_GO), 0);
    }

    #[test]
    fn wait_slots_pause_without_driving() {
        let mut d = ready();
        write_reg(&mut d, REG_WAVEFORM_SEQ_BASE, 1); // effect
        write_reg(&mut d, REG_WAVEFORM_SEQ_BASE + 1, SEQ_WAIT | 5); // 50 ms wait
        write_reg(&mut d, REG_WAVEFORM_SEQ_BASE + 2, 1); // effect again
        write_reg(&mut d, REG_GO, 1);

        let dur = effect_duration_us(1);
        d.advance_us(dur);
        assert!(d.is_playing(), "now in the wait slot");
        assert!(!d.is_driven(), "a wait slot does not drive the motor");
        d.advance_us(50_000);
        assert!(d.is_driven(), "second effect started");
        d.advance_us(dur);
        assert!(!d.is_playing());
        assert_eq!(
            d.total_drive_time_us(),
            2 * dur,
            "the 50 ms wait is not drive time"
        );
    }

    #[test]
    fn go_is_ignored_while_in_standby() {
        let mut d = Drv2605::default(); // still in standby
        write_reg(&mut d, REG_WAVEFORM_SEQ_BASE, 1);
        write_reg(&mut d, REG_GO, 1);
        assert_eq!(read_reg(&mut d, REG_GO), 0, "GO self-clears in standby");
        assert!(!d.is_driven());
        d.advance_us(1_000_000);
        assert_eq!(d.total_drive_time_us(), 0);
    }

    #[test]
    fn entering_standby_aborts_playback() {
        let mut d = ready();
        write_reg(&mut d, REG_WAVEFORM_SEQ_BASE, 4);
        write_reg(&mut d, REG_GO, 1);
        assert!(d.is_driven());
        write_reg(&mut d, REG_MODE, MODE_STANDBY);
        assert!(!d.is_driven());
        assert_eq!(read_reg(&mut d, REG_GO), 0);
        assert!(!d.is_playing());
    }

    #[test]
    fn writing_go_zero_aborts_the_sequence() {
        let mut d = ready();
        write_reg(&mut d, REG_WAVEFORM_SEQ_BASE, 7);
        write_reg(&mut d, REG_GO, 1);
        d.advance_us(5_000);
        write_reg(&mut d, REG_GO, 0);
        assert!(!d.is_driven());
        assert!(!d.is_playing());
        assert_eq!(d.total_drive_time_us(), 5_000, "only what actually played");
    }

    #[test]
    fn empty_sequence_never_drives() {
        let mut d = ready(); // all sequencer slots still 0
        write_reg(&mut d, REG_GO, 1);
        assert!(!d.is_driven());
        assert_eq!(read_reg(&mut d, REG_GO), 0, "nothing queued → GO clears");
        assert_eq!(d.total_drive_time_us(), 0);
    }

    #[test]
    fn real_time_playback_drives_straight_off_the_rtp_register() {
        let mut d = Drv2605::default();
        write_reg(&mut d, REG_MODE, MODE_RTP);
        assert!(!d.is_driven(), "RTP register still 0");
        write_reg(&mut d, REG_RTP_INPUT, 0x7F);
        assert!(d.is_driven(), "RTP drives with no GO handshake");
        assert_eq!(d.amplitude(), 0x7F);
        d.advance_us(20_000);
        assert_eq!(d.total_drive_time_us(), 20_000);
        write_reg(&mut d, REG_RTP_INPUT, 0x00);
        assert!(!d.is_driven());
        d.advance_us(20_000);
        assert_eq!(d.total_drive_time_us(), 20_000, "off means off");
    }

    #[test]
    fn dev_reset_restores_power_on_state() {
        let mut d = ready();
        write_reg(&mut d, REG_WAVEFORM_SEQ_BASE, 9);
        write_reg(&mut d, REG_LIBRARY, 0x06);
        write_reg(&mut d, REG_MODE, MODE_DEV_RESET);
        assert_eq!(read_reg(&mut d, REG_MODE), MODE_STANDBY);
        assert_eq!(read_reg(&mut d, REG_LIBRARY), 0x00);
        assert_eq!(read_reg(&mut d, REG_WAVEFORM_SEQ_BASE), 0x00);
        assert_eq!(read_reg(&mut d, REG_STATUS), 0xE0);
    }

    #[test]
    fn status_register_is_read_only() {
        let mut d = ready();
        write_reg(&mut d, REG_STATUS, 0x00);
        assert_eq!(read_reg(&mut d, REG_STATUS), 0xE0);
    }

    #[test]
    fn amplitude_is_deterministic_per_effect() {
        assert_eq!(effect_amplitude(1), effect_amplitude(1));
        assert!(effect_amplitude(1) >= 128);
        assert_ne!(effect_amplitude(1), effect_amplitude(2));
        assert_eq!(effect_duration_us(1), 40_000);
    }
}
