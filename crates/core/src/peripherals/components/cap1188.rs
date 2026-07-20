// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Microchip CAP1188 8-channel capacitive touch controller as an
//! [`I2cDevice`].
//!
//! A driver's whole interaction with the part is: probe the Product /
//! Manufacturer ID registers, enable the channels it uses (Sensor Input
//! Enable), set a sensitivity, then either poll the Sensor Input Status
//! register or wait on the ALERT pin and read status. Touch is reported two
//! ways — one status bit per channel, and a signed per-channel *delta count*
//! (how far the measured capacitance moved from the calibrated base) — and
//! both are modelled here.
//!
//! ## Latching semantics (the part firmware most often gets wrong)
//!
//! Per the datasheet, Sensor Input Status bits and the Main Control `INT` bit
//! are **latched**: they do not clear when read. Firmware clears them by
//! writing Main Control back with `INT = 0`, and a status bit only actually
//! clears if the touch has been *released* by then — a still-held pad
//! immediately re-asserts `INT`. That is what this model implements, so a
//! driver that forgets the write-0 sees the same stuck interrupt it would see
//! on hardware.
//!
//! Touch is driven at runtime through [`crate::sim_input::SimInput`]
//! (`touch1`..`touch8`), because on the smart-ring platform a pad press is the
//! primary UI event and a lab has to be able to inject it.

use crate::peripherals::i2c::I2cDevice;

/// Default 7-bit I²C address (ADDR_COMM floating).
pub const CAP1188_ADDR: u8 = 0x29;

// ── Register map (CAP1188 datasheet §5, "Register Descriptions") ────────────
const REG_MAIN_CONTROL: u8 = 0x00;
const REG_GENERAL_STATUS: u8 = 0x02;
const REG_SENSOR_INPUT_STATUS: u8 = 0x03;
const REG_NOISE_FLAGS: u8 = 0x0A;
/// Sensor Input 1..8 Delta Count occupy 0x10..=0x17.
const REG_DELTA_BASE: u8 = 0x10;
const REG_SENSITIVITY: u8 = 0x1F;
const REG_CONFIGURATION: u8 = 0x20;
const REG_SENSOR_INPUT_ENABLE: u8 = 0x21;
const REG_INTERRUPT_ENABLE: u8 = 0x27;
const REG_CONFIGURATION_2: u8 = 0x44;
const REG_PRODUCT_ID: u8 = 0xFD;
const REG_MANUFACTURER_ID: u8 = 0xFE;
const REG_REVISION: u8 = 0xFF;

/// Product ID — 0x50 on the CAP1188; the value drivers probe for.
pub const PRODUCT_ID_VALUE: u8 = 0x50;
/// Manufacturer ID — 0x5D (Microchip/SMSC) on every part in the family.
pub const MANUFACTURER_ID_VALUE: u8 = 0x5D;
/// Revision register. 0x83 is the value the datasheet lists as default, but it
/// is silicon-revision dependent and NOT independently verified here. No
/// driver should branch on it; the probe registers are Product/Manufacturer ID.
const REVISION_VALUE: u8 = 0x83;

/// Main Control bit 0: interrupt asserted. Write-0 to clear.
const MAIN_INT: u8 = 0x01;
/// General Status bit 0: at least one channel is currently touched.
const GENERAL_TOUCH: u8 = 0x01;

/// Number of capacitive channels.
pub const CHANNELS: usize = 8;

/// Touch strength (percent of full-scale delta) at or above which a channel
/// counts as touched. A single fixed threshold, deliberately: the model does
/// not simulate the analogue calibration loop, so the threshold is a MODELLING
/// CHOICE rather than a datasheet-derived level.
const TOUCH_THRESHOLD_PERCENT: u32 = 50;

/// Delta count reported at 100 % touch strength. The delta registers are
/// signed 8-bit and the datasheet's own examples sit in the tens-to-low-
/// hundreds range for a firm press; 127 is full scale here.
const DELTA_FULL_SCALE: u32 = 127;

/// Microchip CAP1188 capacitive touch controller.
#[derive(Debug)]
pub struct Cap1188 {
    address: u8,
    /// I²C register pointer.
    pointer: u8,
    /// False until the first byte after a START has selected the register.
    pointer_set: bool,

    // ── Register file ───────────────────────────────────────────────────────
    /// Main Control. Bit 0 is INT and is maintained by the model; the gain /
    /// standby / deep-sleep bits are stored so a read-modify-write round-trips.
    main_control: u8,
    noise_flags: u8,
    sensitivity: u8,
    configuration: u8,
    configuration_2: u8,
    sensor_input_enable: u8,
    interrupt_enable: u8,

    // ── Touch state ─────────────────────────────────────────────────────────
    /// Live pad state, one bit per channel — what a finger is doing RIGHT NOW.
    touched: u8,
    /// Latched Sensor Input Status, one bit per channel. Set when a touch is
    /// detected, cleared only by a write-0 to Main Control INT while the pad is
    /// released.
    status_latch: u8,
    /// Per-channel touch strength in percent, as driven through `SimInput`.
    strength_percent: [u8; CHANNELS],

    /// system.yaml `external_devices` id, stamped at attach (see
    /// [`crate::sim_input::SimInput::component_id`]).
    component_id: Option<String>,
}

impl Default for Cap1188 {
    fn default() -> Self {
        Self::new(CAP1188_ADDR)
    }
}

impl Cap1188 {
    pub fn new(address: u8) -> Self {
        Self {
            address,
            pointer: 0,
            pointer_set: false,

            main_control: 0x00,
            noise_flags: 0x00,
            // Datasheet defaults: Sensitivity Control 0x2F (32× gain, base
            // shift 0x0F), Configuration 0x20, Configuration 2 0x40, all eight
            // inputs enabled (0xFF), all interrupts enabled (0xFF).
            sensitivity: 0x2F,
            configuration: 0x20,
            configuration_2: 0x40,
            sensor_input_enable: 0xFF,
            interrupt_enable: 0xFF,

            touched: 0,
            status_latch: 0,
            strength_percent: [0; CHANNELS],

            component_id: None,
        }
    }

    /// Live pad state, one bit per channel (bit 0 = CS1).
    pub fn touched_mask(&self) -> u8 {
        self.touched
    }

    /// Latched Sensor Input Status, one bit per channel.
    pub fn status_mask(&self) -> u8 {
        self.status_latch
    }

    /// True while the ALERT/INT line would be asserted.
    pub fn interrupt_asserted(&self) -> bool {
        self.main_control & MAIN_INT != 0
    }

    /// Delta count the part reports for `channel` (0-based), as the signed
    /// value in the 0x10..0x17 registers.
    pub fn delta_count(&self, channel: usize) -> i8 {
        if channel >= CHANNELS || self.sensor_input_enable & (1 << channel) == 0 {
            return 0;
        }
        let pct = self.strength_percent[channel] as u32;
        ((pct * DELTA_FULL_SCALE / 100) as i32).clamp(0, 127) as i8
    }

    /// Set the touch strength on `channel` (0-based) as a percentage of
    /// full-scale delta. Crossing [`TOUCH_THRESHOLD_PERCENT`] asserts the
    /// touch — which latches the status bit and raises INT (subject to Sensor
    /// Input Enable and Interrupt Enable, as on hardware).
    pub fn set_touch_strength(&mut self, channel: usize, percent: f64) {
        if channel >= CHANNELS {
            return;
        }
        let pct = percent.clamp(0.0, 100.0).round() as u32;
        self.strength_percent[channel] = pct as u8;

        let bit = 1u8 << channel;
        let enabled = self.sensor_input_enable & bit != 0;
        if pct >= TOUCH_THRESHOLD_PERCENT && enabled {
            self.touched |= bit;
            self.status_latch |= bit;
            if self.interrupt_enable & bit != 0 {
                self.main_control |= MAIN_INT;
            }
        } else {
            self.touched &= !bit;
            // The STATUS bit stays latched until firmware clears INT — the
            // release alone does not clear it.
        }
    }

    /// Convenience: assert or release a pad at full / zero strength.
    pub fn set_touch(&mut self, channel: usize, pressed: bool) {
        self.set_touch_strength(channel, if pressed { 100.0 } else { 0.0 });
    }

    /// Firmware wrote Main Control. Bit 0 is write-0-to-clear: clearing it
    /// deasserts the interrupt and drops the latched status of every channel
    /// that has since been released. Channels still held immediately
    /// re-assert, exactly as the silicon does.
    fn write_main_control(&mut self, value: u8) {
        // The other bits (gain, standby, deep sleep) are stored verbatim.
        self.main_control = (self.main_control & MAIN_INT) | (value & !MAIN_INT);
        if value & MAIN_INT == 0 {
            self.main_control &= !MAIN_INT;
            // Only released pads drop out of the latch.
            self.status_latch &= self.touched;
            if self.status_latch != 0 && self.interrupt_enable & self.status_latch != 0 {
                self.main_control |= MAIN_INT;
            }
        }
    }

    fn read_register(&self, reg: u8) -> u8 {
        match reg {
            REG_MAIN_CONTROL => self.main_control,
            // GENERAL_TOUCH is asserted while any pad reads as touched.
            REG_GENERAL_STATUS if self.touched != 0 => GENERAL_TOUCH,
            REG_GENERAL_STATUS => 0,
            REG_SENSOR_INPUT_STATUS => self.status_latch,
            REG_NOISE_FLAGS => self.noise_flags,
            REG_DELTA_BASE..=0x17 => self.delta_count((reg - REG_DELTA_BASE) as usize) as u8,
            REG_SENSITIVITY => self.sensitivity,
            REG_CONFIGURATION => self.configuration,
            REG_SENSOR_INPUT_ENABLE => self.sensor_input_enable,
            REG_INTERRUPT_ENABLE => self.interrupt_enable,
            REG_CONFIGURATION_2 => self.configuration_2,
            REG_PRODUCT_ID => PRODUCT_ID_VALUE,
            REG_MANUFACTURER_ID => MANUFACTURER_ID_VALUE,
            REG_REVISION => REVISION_VALUE,
            _ => 0,
        }
    }

    fn write_register(&mut self, reg: u8, value: u8) {
        match reg {
            REG_MAIN_CONTROL => self.write_main_control(value),
            REG_NOISE_FLAGS => self.noise_flags &= !value,
            REG_SENSITIVITY => self.sensitivity = value,
            REG_CONFIGURATION => self.configuration = value,
            REG_SENSOR_INPUT_ENABLE => {
                self.sensor_input_enable = value;
                // Disabled channels stop reporting immediately.
                self.touched &= value;
                self.status_latch &= value;
            }
            REG_INTERRUPT_ENABLE => self.interrupt_enable = value,
            REG_CONFIGURATION_2 => self.configuration_2 = value,
            // Status, delta counts and the ID registers are read-only.
            _ => {}
        }
    }
}

impl I2cDevice for Cap1188 {
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

    fn as_sim_input_mut(&mut self) -> Option<&mut dyn crate::sim_input::SimInput> {
        Some(self)
    }
}

/// Drivable touch channels, one per capacitive pad. The value is the touch
/// strength as a percentage of full-scale delta count: 0 % is "not touching",
/// 100 % is a firm press. Anything at or above
/// [`TOUCH_THRESHOLD_PERCENT`] asserts the channel's status bit; below it the
/// pad is released but the delta count still moves, so a lab can sweep a
/// near-threshold approach and check the driver's debounce. ONE table backs
/// both the `SimInput` impl and any kit metadata.
pub const INPUT_CHANNELS: &[crate::sim_input::InputChannel] = &[
    crate::sim_input::InputChannel {
        key: "touch1",
        label: "Touch CS1",
        unit: "%",
        min: 0.0,
        max: 100.0,
    },
    crate::sim_input::InputChannel {
        key: "touch2",
        label: "Touch CS2",
        unit: "%",
        min: 0.0,
        max: 100.0,
    },
    crate::sim_input::InputChannel {
        key: "touch3",
        label: "Touch CS3",
        unit: "%",
        min: 0.0,
        max: 100.0,
    },
    crate::sim_input::InputChannel {
        key: "touch4",
        label: "Touch CS4",
        unit: "%",
        min: 0.0,
        max: 100.0,
    },
    crate::sim_input::InputChannel {
        key: "touch5",
        label: "Touch CS5",
        unit: "%",
        min: 0.0,
        max: 100.0,
    },
    crate::sim_input::InputChannel {
        key: "touch6",
        label: "Touch CS6",
        unit: "%",
        min: 0.0,
        max: 100.0,
    },
    crate::sim_input::InputChannel {
        key: "touch7",
        label: "Touch CS7",
        unit: "%",
        min: 0.0,
        max: 100.0,
    },
    crate::sim_input::InputChannel {
        key: "touch8",
        label: "Touch CS8",
        unit: "%",
        min: 0.0,
        max: 100.0,
    },
];

impl crate::sim_input::SimInput for Cap1188 {
    fn input_channels(&self) -> &'static [crate::sim_input::InputChannel] {
        INPUT_CHANNELS
    }

    fn set_input(&mut self, key: &str, value: f64) -> Result<(), crate::sim_input::SimInputError> {
        self.require_channel(key, value)?;
        let channel = key
            .strip_prefix("touch")
            .and_then(|n| n.parse::<usize>().ok())
            .expect("require_channel validated the key");
        self.set_touch_strength(channel - 1, value);
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

    /// Read `len` bytes starting at `reg` through the real I²C register
    /// interface — the same path firmware takes.
    fn read_regs(dev: &mut Cap1188, reg: u8, len: usize) -> Vec<u8> {
        dev.stop();
        dev.write(reg);
        let out = (0..len).map(|_| dev.read()).collect();
        dev.stop();
        out
    }

    fn read_reg(dev: &mut Cap1188, reg: u8) -> u8 {
        read_regs(dev, reg, 1)[0]
    }

    fn write_reg(dev: &mut Cap1188, reg: u8, value: u8) {
        dev.stop();
        dev.write(reg);
        dev.write(value);
        dev.stop();
    }

    #[test]
    fn id_registers_identify_a_cap1188() {
        let mut d = Cap1188::default();
        assert_eq!(d.address(), 0x29);
        assert_eq!(read_reg(&mut d, REG_PRODUCT_ID), 0x50);
        assert_eq!(read_reg(&mut d, REG_MANUFACTURER_ID), 0x5D);
        // The three ID registers are contiguous, so a driver reads them in one
        // burst off the auto-incrementing pointer.
        let ids = read_regs(&mut d, REG_PRODUCT_ID, 3);
        assert_eq!(ids, vec![0x50, 0x5D, REVISION_VALUE]);
    }

    #[test]
    fn register_write_then_read_back_and_pointer_auto_increments() {
        let mut d = Cap1188::default();
        write_reg(&mut d, REG_SENSITIVITY, 0x3F);
        assert_eq!(read_reg(&mut d, REG_SENSITIVITY), 0x3F);

        // Block write across 0x20/0x21 walks the pointer.
        d.stop();
        d.write(REG_CONFIGURATION);
        d.write(0x38);
        d.write(0x0F);
        d.stop();
        assert_eq!(read_reg(&mut d, REG_CONFIGURATION), 0x38);
        assert_eq!(read_reg(&mut d, REG_SENSOR_INPUT_ENABLE), 0x0F);

        // And a block read does too.
        let block = read_regs(&mut d, REG_CONFIGURATION, 2);
        assert_eq!(block, vec![0x38, 0x0F]);
    }

    #[test]
    fn read_only_registers_ignore_writes() {
        let mut d = Cap1188::default();
        write_reg(&mut d, REG_PRODUCT_ID, 0x00);
        write_reg(&mut d, REG_SENSOR_INPUT_STATUS, 0xFF);
        assert_eq!(read_reg(&mut d, REG_PRODUCT_ID), 0x50);
        assert_eq!(read_reg(&mut d, REG_SENSOR_INPUT_STATUS), 0x00);
    }

    #[test]
    fn touch_sets_status_bit_delta_and_int() {
        let mut d = Cap1188::default();
        assert_eq!(read_reg(&mut d, REG_SENSOR_INPUT_STATUS), 0x00);
        assert_eq!(read_reg(&mut d, REG_MAIN_CONTROL) & MAIN_INT, 0);

        d.set_touch(2, true); // CS3
        assert_eq!(read_reg(&mut d, REG_SENSOR_INPUT_STATUS), 0b0000_0100);
        assert_ne!(read_reg(&mut d, REG_MAIN_CONTROL) & MAIN_INT, 0);
        assert_eq!(read_reg(&mut d, REG_GENERAL_STATUS) & GENERAL_TOUCH, 1);
        // Delta count on the touched channel only.
        let deltas = read_regs(&mut d, REG_DELTA_BASE, 8);
        assert_eq!(deltas[2], 127, "full-strength press → full-scale delta");
        assert!(deltas.iter().enumerate().all(|(i, &v)| i == 2 || v == 0));
    }

    #[test]
    fn status_and_int_are_latched_and_do_not_clear_on_read() {
        let mut d = Cap1188::default();
        d.set_touch(0, true);
        assert_eq!(read_reg(&mut d, REG_SENSOR_INPUT_STATUS), 0x01);
        assert_eq!(
            read_reg(&mut d, REG_SENSOR_INPUT_STATUS),
            0x01,
            "reading does NOT clear the latch"
        );
        assert_ne!(read_reg(&mut d, REG_MAIN_CONTROL) & MAIN_INT, 0);
    }

    #[test]
    fn write_zero_to_int_clears_only_after_release() {
        let mut d = Cap1188::default();
        d.set_touch(0, true);

        // Still held: clearing INT immediately re-asserts, and the status bit
        // survives — the stuck-interrupt behaviour real firmware must handle.
        let mc = read_reg(&mut d, REG_MAIN_CONTROL);
        write_reg(&mut d, REG_MAIN_CONTROL, mc & !MAIN_INT);
        assert_ne!(read_reg(&mut d, REG_MAIN_CONTROL) & MAIN_INT, 0);
        assert_eq!(read_reg(&mut d, REG_SENSOR_INPUT_STATUS), 0x01);

        // Release, then clear: both drop.
        d.set_touch(0, false);
        assert_eq!(
            read_reg(&mut d, REG_SENSOR_INPUT_STATUS),
            0x01,
            "release alone leaves the latch set"
        );
        let mc = read_reg(&mut d, REG_MAIN_CONTROL);
        write_reg(&mut d, REG_MAIN_CONTROL, mc & !MAIN_INT);
        assert_eq!(read_reg(&mut d, REG_SENSOR_INPUT_STATUS), 0x00);
        assert_eq!(read_reg(&mut d, REG_MAIN_CONTROL) & MAIN_INT, 0);
        assert_eq!(read_reg(&mut d, REG_GENERAL_STATUS) & GENERAL_TOUCH, 0);
        assert_eq!(read_reg(&mut d, REG_DELTA_BASE), 0);
    }

    #[test]
    fn main_control_write_preserves_the_other_bits() {
        let mut d = Cap1188::default();
        // Gain = 3 (bits 7:6) plus STBY (bit 5), INT written as 0.
        write_reg(&mut d, REG_MAIN_CONTROL, 0xE0);
        assert_eq!(read_reg(&mut d, REG_MAIN_CONTROL), 0xE0);
    }

    #[test]
    fn multiple_channels_report_independently() {
        let mut d = Cap1188::default();
        d.set_touch(1, true);
        d.set_touch(7, true);
        assert_eq!(read_reg(&mut d, REG_SENSOR_INPUT_STATUS), 0b1000_0010);
        d.set_touch(1, false);
        let mc = read_reg(&mut d, REG_MAIN_CONTROL);
        write_reg(&mut d, REG_MAIN_CONTROL, mc & !MAIN_INT);
        assert_eq!(
            read_reg(&mut d, REG_SENSOR_INPUT_STATUS),
            0b1000_0000,
            "only the released channel drops out of the latch"
        );
        assert_ne!(
            read_reg(&mut d, REG_MAIN_CONTROL) & MAIN_INT,
            0,
            "CS8 still held → INT re-asserts"
        );
    }

    #[test]
    fn disabled_channels_do_not_report() {
        let mut d = Cap1188::default();
        write_reg(&mut d, REG_SENSOR_INPUT_ENABLE, 0x01); // CS1 only
        d.set_touch(3, true);
        assert_eq!(read_reg(&mut d, REG_SENSOR_INPUT_STATUS), 0x00);
        assert_eq!(read_reg(&mut d, REG_MAIN_CONTROL) & MAIN_INT, 0);
        assert_eq!(read_regs(&mut d, REG_DELTA_BASE, 8)[3], 0);
    }

    #[test]
    fn masked_interrupt_still_latches_status() {
        let mut d = Cap1188::default();
        write_reg(&mut d, REG_INTERRUPT_ENABLE, 0x00);
        d.set_touch(0, true);
        assert_eq!(
            read_reg(&mut d, REG_MAIN_CONTROL) & MAIN_INT,
            0,
            "interrupt masked"
        );
        assert_eq!(
            read_reg(&mut d, REG_SENSOR_INPUT_STATUS),
            0x01,
            "status still latches — firmware can poll it"
        );
    }

    #[test]
    fn sub_threshold_strength_moves_delta_without_asserting_touch() {
        let mut d = Cap1188::default();
        d.set_touch_strength(0, 30.0);
        assert_eq!(read_reg(&mut d, REG_SENSOR_INPUT_STATUS), 0x00);
        assert_eq!(read_reg(&mut d, REG_DELTA_BASE), 38, "30 % of 127");
        d.set_touch_strength(0, 60.0);
        assert_eq!(read_reg(&mut d, REG_SENSOR_INPUT_STATUS), 0x01);
    }

    #[test]
    fn sim_input_drives_every_pad() {
        let mut d = Cap1188::default();
        for ch in 1..=CHANNELS {
            d.set_input(&format!("touch{ch}"), 100.0).unwrap();
        }
        assert_eq!(d.status_mask(), 0xFF);
        assert!(d.interrupt_asserted());
        assert_eq!(d.input_channels().len(), CHANNELS);
    }

    #[test]
    fn sim_input_rejects_out_of_range_and_unknown_channels() {
        let mut d = Cap1188::default();
        assert!(d.set_input("touch1", 150.0).is_err());
        assert!(d.set_input("touch9", 100.0).is_err());
        assert!(d.set_input("pressure", 1.0).is_err());
        assert!(d.set_input("touch1", 100.0).is_ok());
    }

    #[test]
    fn component_id_round_trips() {
        let mut d = Cap1188::default();
        assert!(SimInput::component_id(&d).is_none());
        d.set_component_id("touchpad".to_string());
        assert_eq!(SimInput::component_id(&d), Some("touchpad"));
    }
}
