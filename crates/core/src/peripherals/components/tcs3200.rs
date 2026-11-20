// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! ams-OSRAM TCS3200 programmable color light-to-frequency converter.
//!
//! Electrical model of the 8-pin module / SOIC-8 die in DS000107 [v1-01]
//! (2018-Mar-14):
//!   https://look.ams-osram.com/m/664723bdb31f55db/original/TCS3200-DS000107.pdf
//! Product page:
//!   https://ams-osram.com/products/sensor-solutions/ambient-light-color-spectral-proximity-sensors/ams-tcs3200-color-sensor
//!
//! OUT is a 50 % duty square wave whose frequency is proportional to the
//! irradiance on the photodiode type selected by S2/S3, scaled by S0/S1.
//! OE is active-low; a high OE or S0/S1 both low (power-down) hold OUT
//! high-impedance, so this model stops scheduling edges and stops driving
//! the pad.
//!
//! Pin functions (top view, DS000107 terminal functions):
//!   S0=1, S1=2, OE=3 (active low), GND=4, VDD=5, OUT=6, S2=7, S3=8.
//!
//! Fig. 5 — output frequency scaling and photodiode selection:
//!   S0 L, S1 L → power down (OUT high-Z)
//!   S0 L, S1 H →  2 %
//!   S0 H, S1 L → 20 %
//!   S0 H, S1 H → 100 %
//!   S2 L, S3 L → red    S2 L, S3 H → blue
//!   S2 H, S3 L → clear  S2 H, S3 H → green
//!
//! Full-scale typ fo is 600 kHz at 100 % (12 kHz typ at 2 %, 120 kHz typ at
//! 20 %). Dark frequency typ is 2 Hz (max 10 Hz) at 100 %. A 50 ms
//! `pulseIn` timeout, as used by the Nano color-detector firmware, cannot
//! observe typ dark: the LOW half-period would be 250 ms at 100 % and 1.25 s
//! at 20 %. The no-object / black floor in this model is therefore 100 Hz at
//! 100 % (scaled with S0/S1), which keeps the LOW pulse inside (400 µs, 50 ms)
//! at 2 / 20 / 100 % so a pulseIn-based sketch can classify black as NONE
//! instead of a timeout of 0. That floor is 0.017 % of typ full-scale and is
//! named here rather than presented as the datasheet typ dark current.
//!
//! Target colour is three host channels `red`, `green`, `blue` in 0..=255.
//! Clear-filter fo uses the mean of the three. (0, 0, 0) is no object.

use std::borrow::Cow;

use anyhow::{Context, Result};

use crate::bus::{BusResidentDevice, DevicePins, SystemBus};
use crate::peripherals::kit::{
    AttachCtx, Category, ConfigKey, ConfigType, KitMetadata, PeripheralKit, Transport,
};
use crate::sim_input::{InputChannel, SimInput, SimInputError};

/// Typ full-scale output frequency at S0=H, S1=H, from DS000107 (600 kHz typ).
const FO_FULL_100_HZ: f64 = 600_000.0;

/// PulseIn-observable no-object floor at 100 % scale. See the module docs.
const FO_DARK_100_HZ: f64 = 100.0;

const CHANNELS: [InputChannel; 3] = [
    InputChannel {
        key: Cow::Borrowed("red"),
        label: Cow::Borrowed("Red"),
        unit: Cow::Borrowed("level"),
        min: 0.0,
        max: 255.0,
        default: Some(0.0),
    },
    InputChannel {
        key: Cow::Borrowed("green"),
        label: Cow::Borrowed("Green"),
        unit: Cow::Borrowed("level"),
        min: 0.0,
        max: 255.0,
        default: Some(0.0),
    },
    InputChannel {
        key: Cow::Borrowed("blue"),
        label: Cow::Borrowed("Blue"),
        unit: Cow::Borrowed("level"),
        min: 0.0,
        max: 255.0,
        default: Some(0.0),
    },
];

/// S0/S1 scale factor, or `None` for the power-down (high-Z) encoding.
pub fn scale_factor(s0: bool, s1: bool) -> Option<f64> {
    match (s0, s1) {
        (false, false) => None,
        (false, true) => Some(0.02),
        (true, false) => Some(0.20),
        (true, true) => Some(1.0),
    }
}

/// Frequency of the selected filter at a given scale and 0..=255 channel level.
pub fn output_frequency_hz(scale: f64, level: f64) -> f64 {
    let level = level.clamp(0.0, 255.0);
    let dark = FO_DARK_100_HZ * scale;
    let full = FO_FULL_100_HZ * scale;
    dark + (full - dark) * (level / 255.0)
}

/// Photodiode level for the S2/S3 encoding. Clear is the mean of R/G/B.
pub fn filter_level(s2: bool, s3: bool, red: f64, green: f64, blue: f64) -> f64 {
    match (s2, s3) {
        (false, false) => red,
        (false, true) => blue,
        (true, false) => (red + green + blue) / 3.0,
        (true, true) => green,
    }
}

#[derive(Debug)]
pub struct Tcs3200 {
    id: String,
    cpu_hz: u64,
    s0: (u64, u8),
    s1: (u64, u8),
    s2: (u64, u8),
    s3: (u64, u8),
    /// `None` when OE is tied (typically to GND) and therefore always enabled.
    oe: Option<(u64, u8)>,
    out: (u64, u8),
    /// Unique output-register addresses of S0/S1/S2/S3/OE, for the write hook.
    control_odr_addrs: Vec<u64>,
    red: f64,
    green: f64,
    blue: f64,
    out_high: bool,
    next_toggle: Option<u64>,
    half_period_cycles: Option<u64>,
    schedule_revision: u64,
}

/// Resolved GPIO pads (ODR address, bit) the sensor is wired to.
struct Tcs3200Pads {
    s0: (u64, u8),
    s1: (u64, u8),
    s2: (u64, u8),
    s3: (u64, u8),
    /// `None` when OE is tied off and the output is always enabled.
    oe: Option<(u64, u8)>,
    out: (u64, u8),
}

impl Tcs3200 {
    fn new(id: String, cpu_hz: u64, pads: Tcs3200Pads) -> Self {
        let Tcs3200Pads {
            s0,
            s1,
            s2,
            s3,
            oe,
            out,
        } = pads;
        let mut addrs: Vec<u64> = [s0.0, s1.0, s2.0, s3.0]
            .into_iter()
            .chain(oe.map(|p| p.0))
            .collect();
        addrs.sort_unstable();
        addrs.dedup();
        Self {
            id,
            cpu_hz: cpu_hz.max(1),
            s0,
            s1,
            s2,
            s3,
            oe,
            out,
            control_odr_addrs: addrs,
            red: 0.0,
            green: 0.0,
            blue: 0.0,
            out_high: true,
            next_toggle: None,
            half_period_cycles: None,
            schedule_revision: 0,
        }
    }

    fn pin_high(pins: &dyn DevicePins, pad: (u64, u8)) -> bool {
        pins.pad_bit(pad.0, pad.1)
            .or_else(|| pins.output_bit(pad.0, pad.1))
            .unwrap_or(false)
    }

    fn oe_enabled(&self, pins: &dyn DevicePins) -> bool {
        match self.oe {
            // Active-low enable. Tied-off (no pin) is the usual module wiring
            // with OE to GND, so the output is enabled.
            None => true,
            Some(pad) => !Self::pin_high(pins, pad),
        }
    }

    fn computed_half_period(&self, pins: &dyn DevicePins) -> Option<u64> {
        if !self.oe_enabled(pins) {
            return None;
        }
        let scale = scale_factor(Self::pin_high(pins, self.s0), Self::pin_high(pins, self.s1))?;
        let level = filter_level(
            Self::pin_high(pins, self.s2),
            Self::pin_high(pins, self.s3),
            self.red,
            self.green,
            self.blue,
        );
        let fo = output_frequency_hz(scale, level);
        if !(fo.is_finite() && fo > 0.0) {
            return None;
        }
        let half = (self.cpu_hz as f64) / (2.0 * fo);
        Some(half.round().max(1.0) as u64)
    }

    fn drive_out(&self, pins: &mut dyn DevicePins) {
        pins.drive_input_bit(self.out.0, self.out.1, self.out_high);
        pins.drive_idr_bit(self.out.0, self.out.1, self.out_high);
    }

    /// Re-sample S0–S3 / OE and (re)arm the square wave from `now`.
    fn sync_from_pins(&mut self, pins: &mut dyn DevicePins, now: u64) {
        let half = self.computed_half_period(pins);
        if half != self.half_period_cycles {
            self.half_period_cycles = half;
            self.schedule_revision = self.schedule_revision.wrapping_add(1);
            match half {
                Some(period) => {
                    self.out_high = true;
                    self.next_toggle = Some(now.saturating_add(period));
                    self.drive_out(pins);
                }
                None => {
                    self.next_toggle = None;
                }
            }
        }
    }
}

pub struct Tcs3200Kit;
pub static TCS3200_KIT: Tcs3200Kit = Tcs3200Kit;

const CONFIG_KEYS: &[ConfigKey] = &[
    ConfigKey {
        name: Cow::Borrowed("s0_pin"),
        ty: ConfigType::Str,
        doc: Cow::Borrowed("MCU output wired to S0 (output frequency scaling)."),
    },
    ConfigKey {
        name: Cow::Borrowed("s1_pin"),
        ty: ConfigType::Str,
        doc: Cow::Borrowed("MCU output wired to S1 (output frequency scaling)."),
    },
    ConfigKey {
        name: Cow::Borrowed("s2_pin"),
        ty: ConfigType::Str,
        doc: Cow::Borrowed("MCU output wired to S2 (photodiode / filter select)."),
    },
    ConfigKey {
        name: Cow::Borrowed("s3_pin"),
        ty: ConfigType::Str,
        doc: Cow::Borrowed("MCU output wired to S3 (photodiode / filter select)."),
    },
    ConfigKey {
        name: Cow::Borrowed("out_pin"),
        ty: ConfigType::Str,
        doc: Cow::Borrowed("MCU input wired to OUT (50 % duty light-to-frequency square wave)."),
    },
    ConfigKey {
        name: Cow::Borrowed("oe_pin"),
        ty: ConfigType::Str,
        doc: Cow::Borrowed(
            "Optional MCU output wired to OE (active low). Omit when OE is tied to GND.",
        ),
    },
    ConfigKey {
        name: Cow::Borrowed("cpu_hz"),
        ty: ConfigType::Int,
        doc: Cow::Borrowed("Simulated CPU clock in Hz, used to convert fo into cycle periods."),
    },
];

const METADATA: KitMetadata = KitMetadata {
    device_type: Cow::Borrowed("tcs3200"),
    label: Cow::Borrowed("TCS3200 color sensor"),
    summary: Cow::Borrowed(
        "Programmable RGB light-to-frequency converter (S0/S1 scale, S2/S3 filter, OE).",
    ),
    detail: Cow::Borrowed(
        "ams-OSRAM TCS3200 (DS000107): OUT is a 50 % duty square wave. S0/S1 select \
         power-down / 2 % / 20 % / 100 % scaling; S2/S3 select red / blue / clear / green \
         photodiodes; OE is an active-low output enable. Host channels red, green and blue \
         (0–255) set the target colour; (0, 0, 0) is no object. Datasheet: \
         https://look.ams-osram.com/m/664723bdb31f55db/original/TCS3200-DS000107.pdf",
    ),
    transport: Transport::GpioGroup,
    category: Category::Gpio,
    config_keys: Cow::Borrowed(CONFIG_KEYS),
    labs: Cow::Borrowed(&[]),
    inputs: Cow::Borrowed(&CHANNELS),
};

impl PeripheralKit for Tcs3200Kit {
    fn metadata(&self) -> &KitMetadata {
        &METADATA
    }

    fn attach(&self, ctx: &mut AttachCtx<'_>) -> Result<()> {
        let id = ctx.device_id().to_string();
        let resolve_odr = |key: &str| -> Result<(u64, u8)> {
            let pin = ctx
                .config_str(key)
                .with_context(|| format!("tcs3200 '{id}': missing {key}"))?;
            SystemBus::resolve_pin_odr_pub(ctx.bus, pin)
                .with_context(|| format!("tcs3200 '{id}': unresolved {key} '{pin}'"))
        };
        let s0 = resolve_odr("s0_pin")?;
        let s1 = resolve_odr("s1_pin")?;
        let s2 = resolve_odr("s2_pin")?;
        let s3 = resolve_odr("s3_pin")?;
        let out_pin = ctx
            .config_str("out_pin")
            .with_context(|| format!("tcs3200 '{id}': missing out_pin"))?;
        let out = SystemBus::resolve_pin_idr_pub(ctx.bus, out_pin)
            .with_context(|| format!("tcs3200 '{id}': unresolved out_pin '{out_pin}'"))?;
        let oe = if let Some(pin) = ctx.config_str("oe_pin") {
            Some(
                SystemBus::resolve_pin_odr_pub(ctx.bus, pin)
                    .with_context(|| format!("tcs3200 '{id}': unresolved oe_pin '{pin}'"))?,
            )
        } else {
            None
        };
        let cpu_hz = ctx
            .config_f64("cpu_hz")
            .map(|hz| hz.round() as u64)
            .filter(|&hz| hz > 0)
            .unwrap_or(ctx.bus.cpu_hz)
            .max(1);
        ctx.bus.gpio_devices.push(Box::new(Tcs3200::new(
            id,
            cpu_hz,
            Tcs3200Pads {
                s0,
                s1,
                s2,
                s3,
                oe,
                out,
            },
        )));
        Ok(())
    }
}

impl SimInput for Tcs3200 {
    fn input_channels(&self) -> &'static [InputChannel] {
        &CHANNELS
    }

    fn set_input(&mut self, key: &str, value: f64) -> Result<(), SimInputError> {
        self.require_channel(key, value)?;
        match key {
            "red" => self.red = value,
            "green" => self.green = value,
            "blue" => self.blue = value,
            _ => return Err(SimInputError::UnknownChannel(key.to_string())),
        }
        // Period is recomputed on the next service / edge, which also bumps
        // schedule_revision so the bus re-anchors the running wave.
        self.half_period_cycles = None;
        self.schedule_revision = self.schedule_revision.wrapping_add(1);
        Ok(())
    }

    fn component_id(&self) -> Option<&str> {
        Some(&self.id)
    }
}

impl BusResidentDevice for Tcs3200 {
    fn service(&mut self, pins: &mut dyn DevicePins, now: u64) {
        self.sync_from_pins(pins, now);
        if self.next_toggle.is_some() {
            self.drive_out(pins);
        }
    }

    fn next_edge_deadline_cycle(&self, now: u64, interval: u64) -> Option<u64> {
        let interval = interval.max(1);
        let edge = self.next_toggle?;
        let quantized = edge.div_ceil(interval) * interval;
        if quantized > now {
            Some(quantized)
        } else if let Some(half) = self.half_period_cycles {
            let next = quantized.saturating_add(half).div_ceil(interval) * interval;
            (next > now).then_some(next)
        } else {
            None
        }
    }

    fn service_scheduled_edges(&mut self, pins: &mut dyn DevicePins, now: u64, _interval: u64) {
        let Some(mut half) = self.half_period_cycles else {
            return;
        };
        let mut toggled = false;
        while let Some(edge) = self.next_toggle {
            if edge > now {
                break;
            }
            self.out_high = !self.out_high;
            // Re-read in case a mid-wave stimulus changed the period after
            // the last sync; half_period_cycles is the latched value for this
            // edge train and is refreshed by service() / service_edge().
            half = self.half_period_cycles.unwrap_or(half);
            self.next_toggle = Some(edge.saturating_add(half.max(1)));
            toggled = true;
        }
        if toggled {
            self.drive_out(pins);
        }
    }

    fn schedule_revision(&self) -> u64 {
        self.schedule_revision
    }

    fn as_sim_input(&mut self) -> &mut dyn SimInput {
        self
    }

    fn id(&self) -> &str {
        &self.id
    }

    fn needs_per_cycle_service(&self) -> bool {
        // The square wave is a scheduled-edge train; S0–S3/OE changes arrive
        // through the write hook. A per-cycle walk would pin the bus to a
        // 1-cycle tick for a 120 kHz (at 20 %) output.
        false
    }

    fn idle_skip_safe(&self) -> bool {
        self.next_toggle.is_none()
    }

    fn edge_service_addrs(&self) -> &[u64] {
        &self.control_odr_addrs
    }

    fn service_edge(&mut self, pins: &mut dyn DevicePins, now: u64) {
        self.sync_from_pins(pins, now);
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    const S0: (u64, u8) = (0x1000, 0);
    const S1: (u64, u8) = (0x1000, 1);
    const S2: (u64, u8) = (0x1000, 2);
    const S3: (u64, u8) = (0x1000, 3);
    const OE: (u64, u8) = (0x1000, 4);
    const OUT: (u64, u8) = (0x2000, 0);
    const CPU: u64 = 16_000_000;

    #[derive(Default)]
    struct FakePads {
        words: BTreeMap<u64, u32>,
    }

    impl FakePads {
        fn set(&mut self, pad: (u64, u8), high: bool) {
            let v = self.words.entry(pad.0).or_insert(0);
            if high {
                *v |= 1 << pad.1;
            } else {
                *v &= !(1 << pad.1);
            }
        }

        fn get(&self, pad: (u64, u8)) -> bool {
            self.words
                .get(&pad.0)
                .is_some_and(|v| (v >> pad.1) & 1 != 0)
        }
    }

    impl DevicePins for FakePads {
        fn output_bit(&self, addr: u64, bit: u8) -> Option<bool> {
            self.words.get(&addr).map(|v| (v >> bit) & 1 != 0)
        }
        fn drive_idr_bit(&mut self, addr: u64, bit: u8, high: bool) {
            self.set((addr, bit), high);
        }
        fn drive_input_bit(&mut self, addr: u64, bit: u8, high: bool) -> bool {
            self.set((addr, bit), high);
            true
        }
    }

    fn sensor() -> (Tcs3200, FakePads) {
        let dev = Tcs3200::new(
            "color".into(),
            CPU,
            Tcs3200Pads {
                s0: S0,
                s1: S1,
                s2: S2,
                s3: S3,
                oe: Some(OE),
                out: OUT,
            },
        );
        let mut pins = FakePads::default();
        // Firmware default: 20 % scale, red filter, OE enabled (low).
        pins.set(S0, true);
        pins.set(S1, false);
        pins.set(S2, false);
        pins.set(S3, false);
        pins.set(OE, false);
        (dev, pins)
    }

    fn half_cycles(scale: f64, level: f64) -> u64 {
        let fo = output_frequency_hz(scale, level);
        ((CPU as f64) / (2.0 * fo)).round().max(1.0) as u64
    }

    /// Drive until `count` complete LOW pulses have been observed; return
    /// their widths in cycles. Starts from a service() at cycle 0.
    fn low_pulses(dev: &mut Tcs3200, pins: &mut FakePads, count: usize) -> Vec<u64> {
        dev.service(pins, 0);
        assert!(
            pins.get(OUT),
            "wave starts HIGH so pulseIn(LOW) sees a falling edge"
        );
        let mut widths = Vec::new();
        let mut last_high = true;
        let mut low_start = 0u64;
        let mut now = 0u64;
        while widths.len() < count {
            let edge = dev
                .next_edge_deadline_cycle(now, 1)
                .expect("oscillator must keep a deadline");
            now = edge;
            dev.service_scheduled_edges(pins, now, 1);
            let high = pins.get(OUT);
            if last_high && !high {
                low_start = now;
            } else if !last_high && high {
                widths.push(now - low_start);
            }
            last_high = high;
        }
        widths
    }

    #[test]
    fn typ_full_scale_frequencies_match_the_datasheet() {
        assert_eq!(output_frequency_hz(1.0, 255.0), 600_000.0);
        assert_eq!(output_frequency_hz(0.20, 255.0), 120_000.0);
        assert_eq!(output_frequency_hz(0.02, 255.0), 12_000.0);
    }

    #[test]
    fn red_at_twenty_percent_has_the_typ_120_khz_half_period() {
        let (mut dev, mut pins) = sensor();
        dev.set_input("red", 255.0).unwrap();
        let widths = low_pulses(&mut dev, &mut pins, 3);
        let expected = half_cycles(0.20, 255.0);
        assert!(
            widths.iter().all(|&w| w == expected),
            "LOW widths {widths:?} vs {expected}"
        );
    }

    #[test]
    fn green_and_blue_filters_select_only_their_own_channel() {
        let (mut dev, mut pins) = sensor();
        dev.set_input("red", 255.0).unwrap();
        dev.set_input("green", 0.0).unwrap();
        dev.set_input("blue", 0.0).unwrap();
        // Green filter (S2 H, S3 H) must see the dark floor, not the red channel.
        pins.set(S2, true);
        pins.set(S3, true);
        let green_width = low_pulses(&mut dev, &mut pins, 1)[0];
        assert_eq!(green_width, half_cycles(0.20, 0.0));

        let (mut dev, mut pins) = sensor();
        dev.set_input("red", 0.0).unwrap();
        dev.set_input("blue", 255.0).unwrap();
        pins.set(S2, false);
        pins.set(S3, true);
        let blue_width = low_pulses(&mut dev, &mut pins, 1)[0];
        assert_eq!(blue_width, half_cycles(0.20, 255.0));
    }

    #[test]
    fn no_object_low_pulse_is_inside_the_pulsein_window() {
        let (mut dev, mut pins) = sensor();
        // Defaults are (0,0,0).
        let width = low_pulses(&mut dev, &mut pins, 1)[0];
        let expected = half_cycles(0.20, 0.0);
        assert_eq!(width, expected);
        let us = width as f64 * 1_000_000.0 / CPU as f64;
        assert!(
            us > 400.0,
            "no-object LOW {us} µs must exceed the 400 µs NONE floor"
        );
        assert!(
            us < 50_000.0,
            "no-object LOW {us} µs must fit in a 50 ms pulseIn timeout"
        );
    }

    #[test]
    fn scale_select_multiplies_frequency() {
        assert_eq!(scale_factor(true, false), Some(0.20));
        assert_eq!(scale_factor(true, true), Some(1.0));
        assert_eq!(scale_factor(false, true), Some(0.02));
        assert_eq!(scale_factor(false, false), None);

        let (mut dev, mut pins) = sensor();
        dev.set_input("red", 255.0).unwrap();
        assert_eq!(
            low_pulses(&mut dev, &mut pins, 1)[0],
            half_cycles(0.20, 255.0)
        );

        pins.set(S0, true);
        pins.set(S1, true);
        assert_eq!(
            low_pulses(&mut dev, &mut pins, 1)[0],
            half_cycles(1.0, 255.0)
        );

        pins.set(S0, false);
        pins.set(S1, true);
        assert_eq!(
            low_pulses(&mut dev, &mut pins, 1)[0],
            half_cycles(0.02, 255.0)
        );
    }

    #[test]
    fn power_down_and_oe_high_stop_the_wave() {
        let (mut dev, mut pins) = sensor();
        dev.set_input("red", 255.0).unwrap();
        dev.service(&mut pins, 0);
        assert!(dev.next_edge_deadline_cycle(0, 1).is_some());

        pins.set(S0, false);
        pins.set(S1, false);
        dev.service_edge(&mut pins, 100);
        assert!(dev.next_edge_deadline_cycle(100, 1).is_none());

        // Restore 20 % scale, then disable via OE.
        pins.set(S0, true);
        pins.set(S1, false);
        pins.set(OE, true);
        dev.service_edge(&mut pins, 200);
        assert!(dev.next_edge_deadline_cycle(200, 1).is_none());
        assert!(!dev.oe_enabled(&pins));
    }

    #[test]
    fn runtime_colour_change_moves_the_period() {
        let (mut dev, mut pins) = sensor();
        dev.set_input("red", 255.0).unwrap();
        let red_width = low_pulses(&mut dev, &mut pins, 1)[0];
        // Switch the photodiodes to blue and the stimulus to blue.
        dev.set_input("red", 0.0).unwrap();
        dev.set_input("blue", 255.0).unwrap();
        pins.set(S2, false);
        pins.set(S3, true);
        let blue_width = low_pulses(&mut dev, &mut pins, 1)[0];
        assert_eq!(red_width, half_cycles(0.20, 255.0));
        assert_eq!(blue_width, half_cycles(0.20, 255.0));
        // Confirm the red filter now sees the dark floor after the change.
        pins.set(S2, false);
        pins.set(S3, false);
        let red_after = low_pulses(&mut dev, &mut pins, 1)[0];
        assert_eq!(red_after, half_cycles(0.20, 0.0));
        assert!(red_after > red_width);
    }

    #[test]
    fn all_three_filters_are_sampled() {
        let (mut dev, mut pins) = sensor();
        dev.set_input("red", 255.0).unwrap();
        dev.set_input("green", 128.0).unwrap();
        dev.set_input("blue", 64.0).unwrap();

        pins.set(S2, false);
        pins.set(S3, false);
        assert_eq!(
            low_pulses(&mut dev, &mut pins, 1)[0],
            half_cycles(0.20, 255.0)
        );

        pins.set(S2, true);
        pins.set(S3, true);
        assert_eq!(
            low_pulses(&mut dev, &mut pins, 1)[0],
            half_cycles(0.20, 128.0)
        );

        pins.set(S2, false);
        pins.set(S3, true);
        assert_eq!(
            low_pulses(&mut dev, &mut pins, 1)[0],
            half_cycles(0.20, 64.0)
        );
    }

    #[test]
    fn duty_is_fifty_percent() {
        let (mut dev, mut pins) = sensor();
        dev.set_input("red", 255.0).unwrap();
        dev.service(&mut pins, 0);
        let mut high = 0u64;
        let mut low = 0u64;
        let mut last = pins.get(OUT);
        let mut prev = 0u64;
        // Accumulate a handful of half-periods.
        for _ in 0..8 {
            let edge = dev.next_edge_deadline_cycle(prev, 1).unwrap();
            dev.service_scheduled_edges(&mut pins, edge, 1);
            let dt = edge - prev;
            if last {
                high += dt;
            } else {
                low += dt;
            }
            last = pins.get(OUT);
            prev = edge;
        }
        assert_eq!(high, low);
    }

    #[test]
    fn stimulus_rejects_an_unknown_or_out_of_range_channel() {
        let (mut dev, _) = sensor();
        assert!(dev.set_input("clear", 1.0).is_err());
        assert!(dev.set_input("red", 256.0).is_err());
        assert!(dev.set_input("green", -1.0).is_err());
        dev.set_input("blue", 255.0).unwrap();
    }
}
