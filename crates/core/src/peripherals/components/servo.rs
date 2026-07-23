// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Hobby PWM servo (SG90 / MG996R-class) — a digital-twin actuator.
//!
//! A hobby servo is commanded by the **high-pulse width** of a ~50 Hz PWM
//! signal on its single control wire: a ~1 ms pulse parks it at one end of
//! travel, ~2 ms at the other, ~1.5 ms at center. This model reproduces
//! that transfer function and exposes the resulting shaft `angle`.
//!
//! ## How it reads the command
//!
//! Real silicon drives the control pin from a PWM source (ESP32 LEDC /
//! MCPWM, or a bit-banged GPIO). What the servo physically keys off is the
//! **duty ratio** of that signal — `high_time / frame_period`. This model
//! is therefore driven by a duty fraction, obtained two ways:
//!
//!   * [`Servo::on_gpio_edge`] — fed from real pin transitions (the
//!     [`GpioObserver`](crate::peripherals::esp32s3::gpio::GpioObserver)
//!     path). It measures `high_cycles / period_cycles` across frames, so
//!     it is robust to the simulator's instruction-paced (non-wall-clock)
//!     cycle counter: only the *ratio* matters, not absolute cycle timing.
//!   * [`Servo::apply_duty_fraction`] — fed directly from a PWM
//!     peripheral's introspected duty (e.g. `Ledc::channel_duty_fraction`),
//!     for sources that don't toggle a GPIO pin in the model.
//!
//! The duty fraction is converted to an equivalent pulse width using the
//! nominal frame rate (`pulse_us = fraction * 1e6 / frame_hz`), then mapped
//! to an angle through the servo's pulse↔angle calibration. Because servos
//! run a fixed ~50 Hz frame, the nominal rate is a faithful conversion.
//!
//! Interior mutability: the GPIO observer hook is `&self`, so measured
//! state lives behind a `Mutex`. Hold the servo as `Arc<Servo>` — register
//! a clone as a GPIO observer and keep a clone to read the angle back.

use std::sync::{Arc, Mutex};

/// Pulse-width ↔ angle calibration for a hobby servo.
#[derive(Debug, Clone, Copy)]
pub struct ServoCal {
    /// Pulse width (µs) commanding `min_angle`.
    pub min_us: f32,
    /// Pulse width (µs) commanding `max_angle`.
    pub max_us: f32,
    /// Shaft angle (degrees) at `min_us`.
    pub min_angle: f32,
    /// Shaft angle (degrees) at `max_us`.
    pub max_angle: f32,
    /// Nominal PWM frame rate (Hz) — used to convert a measured duty
    /// fraction into a pulse width. Hobby servos expect ~50 Hz.
    pub frame_hz: f32,
}

impl ServoCal {
    /// Generic "standard" servo: 1.0–2.0 ms → 0–180°, 50 Hz. The textbook
    /// RC pulse range; matches `ESP32Servo`/`Servo.h` defaults.
    pub const fn standard() -> Self {
        Self {
            min_us: 1000.0,
            max_us: 2000.0,
            min_angle: 0.0,
            max_angle: 180.0,
            frame_hz: 50.0,
        }
    }

    /// SG90 micro servo: ~0.5–2.4 ms → 0–180°, 50 Hz (TowerPro SG90 sheet).
    pub const fn sg90() -> Self {
        Self {
            min_us: 500.0,
            max_us: 2400.0,
            min_angle: 0.0,
            max_angle: 180.0,
            frame_hz: 50.0,
        }
    }

    /// MG996R metal-gear servo: ~1.0–2.0 ms → 0–180°, 50 Hz.
    pub const fn mg996r() -> Self {
        Self {
            min_us: 1000.0,
            max_us: 2000.0,
            min_angle: 0.0,
            max_angle: 180.0,
            frame_hz: 50.0,
        }
    }

    /// Map a pulse width (µs) to a shaft angle, clamped to the travel range.
    fn pulse_to_angle(&self, pulse_us: f32) -> f32 {
        let span_us = self.max_us - self.min_us;
        let angle = if span_us.abs() < f32::EPSILON {
            self.min_angle
        } else {
            let t = (pulse_us - self.min_us) / span_us;
            self.min_angle + t * (self.max_angle - self.min_angle)
        };
        let (lo, hi) = if self.min_angle <= self.max_angle {
            (self.min_angle, self.max_angle)
        } else {
            (self.max_angle, self.min_angle)
        };
        angle.clamp(lo, hi)
    }
}

#[derive(Debug, Default)]
struct EdgeState {
    /// sim_cycle of the most recent rising edge.
    last_rise: Option<u64>,
    /// Most recently measured full frame length (rise→rise), in sim cycles.
    period_cycles: Option<u64>,
    /// Current pin level (true = high).
    level: bool,
}

#[derive(Debug, Default)]
struct ServoState {
    edge: EdgeState,
    /// Last commanded pulse width (µs), as interpreted by the servo.
    pulse_us: f32,
    /// Current shaft angle (degrees).
    angle: f32,
    /// True once at least one valid duty measurement has been applied.
    commanded: bool,
}

/// A hobby PWM servo digital twin. See module docs.
#[derive(Debug)]
pub struct Servo {
    /// Canvas / board_io part id — used by `get_actuator_states` so the UI can
    /// map shaft angle back onto the diagram part.
    id: String,
    cal: ServoCal,
    /// GPIO pin this servo's control wire is connected to (for the
    /// observer path). Edges on other pins are ignored.
    pin: u8,
    state: Mutex<ServoState>,
}

impl Servo {
    /// Create a servo with the given calibration, listening on `pin` for
    /// the GPIO-edge drive path. Prefer [`Self::with_id`] when the angle must
    /// be exported to the UI under a known part id.
    pub fn new(cal: ServoCal, pin: u8) -> Self {
        Self::with_id(String::new(), cal, pin)
    }

    /// Create a servo bound to a diagram part id for UI readback.
    pub fn with_id(id: impl Into<String>, cal: ServoCal, pin: u8) -> Self {
        Self {
            id: id.into(),
            cal,
            pin,
            state: Mutex::new(ServoState::default()),
        }
    }

    /// Convenience: a standard 1–2 ms / 0–180° servo on `pin`.
    pub fn standard(pin: u8) -> Self {
        Self::new(ServoCal::standard(), pin)
    }

    /// Convenience: an SG90 micro servo on `pin`.
    pub fn sg90(pin: u8) -> Self {
        Self::new(ServoCal::sg90(), pin)
    }

    /// Diagram / board_io part id (empty when constructed without one).
    pub fn id(&self) -> &str {
        &self.id
    }

    /// The GPIO pin this servo is wired to.
    pub fn pin(&self) -> u8 {
        self.pin
    }

    /// Current shaft angle in degrees. Returns the calibration's
    /// `min_angle` until the first valid command is measured.
    pub fn angle_degrees(&self) -> f32 {
        let s = self.state.lock().unwrap();
        if s.commanded {
            s.angle
        } else {
            self.cal.min_angle
        }
    }

    /// Last interpreted control pulse width in microseconds (0 until
    /// commanded).
    pub fn pulse_us(&self) -> f32 {
        self.state.lock().unwrap().pulse_us
    }

    /// True once the servo has seen at least one valid PWM command.
    pub fn is_commanded(&self) -> bool {
        self.state.lock().unwrap().commanded
    }

    /// Drive the servo from a PWM **duty fraction** (`high / period`, in
    /// `0.0..=1.0`) — the direct path for PWM sources that expose their
    /// duty without toggling a GPIO pin (e.g. `Ledc::channel_duty_fraction`).
    pub fn apply_duty_fraction(&self, fraction: f64) {
        let pulse_us = (fraction.clamp(0.0, 1.0) as f32) * (1.0e6 / self.cal.frame_hz);
        let angle = self.cal.pulse_to_angle(pulse_us);
        let mut s = self.state.lock().unwrap();
        s.pulse_us = pulse_us;
        s.angle = angle;
        s.commanded = true;
    }

    /// Feed a single GPIO pin transition (the
    /// [`GpioObserver`](crate::peripherals::esp32s3::gpio::GpioObserver)
    /// hook). Measures the duty ratio across frames and updates the angle
    /// on each falling edge once a frame period is known. No-op for edges
    /// on other pins.
    pub fn on_gpio_edge(&self, pin: u8, to: bool, sim_cycle: u64) {
        if pin != self.pin {
            return;
        }
        let mut s = self.state.lock().unwrap();
        let was = s.edge.level;
        if to == was {
            return; // not a transition on this pin
        }
        s.edge.level = to;
        if to {
            // Rising edge: a rise→rise interval is one full PWM frame.
            if let Some(prev) = s.edge.last_rise {
                if sim_cycle > prev {
                    s.edge.period_cycles = Some(sim_cycle - prev);
                }
            }
            s.edge.last_rise = Some(sim_cycle);
        } else {
            // Falling edge: high time = fall - last_rise. Convert to a duty
            // fraction using the most recent measured frame period.
            if let (Some(rise), Some(period)) = (s.edge.last_rise, s.edge.period_cycles) {
                if sim_cycle >= rise && period > 0 {
                    let high = sim_cycle - rise;
                    let fraction = (high as f64 / period as f64).clamp(0.0, 1.0);
                    let pulse_us = (fraction as f32) * (1.0e6 / self.cal.frame_hz);
                    s.pulse_us = pulse_us;
                    s.angle = self.cal.pulse_to_angle(pulse_us);
                    s.commanded = true;
                }
            }
        }
    }
}

// Bridge the servo into each chip's GPIO observer protocol. Both traits
// share the same `(pin, from, to, sim_cycle)` signature; the servo only
// needs the `to` level and the cycle.
impl crate::peripherals::esp32s3::gpio::GpioObserver for Servo {
    fn on_pin_change(&self, pin: u8, _from: bool, to: bool, sim_cycle: u64) {
        self.on_gpio_edge(pin, to, sim_cycle);
    }
}

impl crate::peripherals::esp32::gpio::GpioObserver for Servo {
    fn on_pin_change(&self, pin: u8, _from: bool, to: bool, sim_cycle: u64) {
        self.on_gpio_edge(pin, to, sim_cycle);
    }
}

/// Binds an [`Ledc`](crate::peripherals::esp32::ledc::Ledc) PWM channel to a
/// [`Servo`] so each `ledcWrite` drives the servo's angle with no polling.
/// Register with `ledc.add_duty_observer(Arc::new(LedcServoDriver::new(ch,
/// servo)))` and keep an `Arc<Servo>` clone to read the angle back.
#[derive(Debug)]
pub struct LedcServoDriver {
    channel: u64,
    servo: Arc<Servo>,
}

impl LedcServoDriver {
    /// Drive `servo` from LEDC `channel`'s duty.
    pub fn new(channel: u64, servo: Arc<Servo>) -> Self {
        Self { channel, servo }
    }
}

impl crate::peripherals::esp32::ledc::LedcDutyObserver for LedcServoDriver {
    fn on_duty_change(&self, channel: u64, duty_fraction: f64) {
        if channel == self.channel {
            self.servo.apply_duty_fraction(duty_fraction);
        }
    }
}

/// Binds an [`Mcpwm`](crate::peripherals::esp32::mcpwm::Mcpwm) operator to a
/// [`Servo`] so each `mcpwm_set_duty` drives the servo's angle — the
/// motor-control PWM peripheral driving the same actuator as LEDC. Register
/// with `mcpwm.add_duty_observer(Arc::new(McpwmServoDriver::new(op, servo)))`.
#[derive(Debug)]
pub struct McpwmServoDriver {
    operator: u64,
    servo: Arc<Servo>,
}

impl McpwmServoDriver {
    /// Drive `servo` from MCPWM `operator`'s duty.
    pub fn new(operator: u64, servo: Arc<Servo>) -> Self {
        Self { operator, servo }
    }
}

impl crate::peripherals::esp32::mcpwm::McpwmDutyObserver for McpwmServoDriver {
    fn on_duty_change(&self, operator: u64, duty_fraction: f64) {
        if operator == self.operator {
            self.servo.apply_duty_fraction(duty_fraction);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Feed `frames` PWM frames with `high`/`period` sim-cycle widths on
    /// `pin`, starting at `t0`. Returns the cycle just past the last frame.
    fn drive_pwm(servo: &Servo, pin: u8, t0: u64, high: u64, period: u64, frames: u64) -> u64 {
        let mut t = t0;
        for _ in 0..frames {
            servo.on_gpio_edge(pin, true, t); // rising
            servo.on_gpio_edge(pin, false, t + high); // falling
            t += period;
        }
        t
    }

    #[test]
    fn duty_fraction_maps_center_to_90_degrees() {
        let servo = Servo::standard(5);
        // 1.5 ms / 20 ms frame = 0.075 duty -> 1500 µs -> mid travel.
        servo.apply_duty_fraction(0.075);
        assert!(
            (servo.angle_degrees() - 90.0).abs() < 0.5,
            "{}",
            servo.angle_degrees()
        );
        assert!((servo.pulse_us() - 1500.0).abs() < 1.0);
    }

    #[test]
    fn duty_fraction_endpoints() {
        let servo = Servo::standard(5);
        servo.apply_duty_fraction(0.05); // 1.0 ms -> 0°
        assert!((servo.angle_degrees() - 0.0).abs() < 0.5);
        servo.apply_duty_fraction(0.10); // 2.0 ms -> 180°
        assert!((servo.angle_degrees() - 180.0).abs() < 0.5);
    }

    #[test]
    fn duty_fraction_clamps_out_of_range() {
        let servo = Servo::standard(5);
        servo.apply_duty_fraction(0.20); // 4 ms — well past max
        assert_eq!(servo.angle_degrees(), 180.0);
        servo.apply_duty_fraction(0.01); // 0.2 ms — well below min
        assert_eq!(servo.angle_degrees(), 0.0);
    }

    #[test]
    fn gpio_edges_drive_angle_via_duty_ratio() {
        let servo = Servo::standard(5);
        // 50 Hz frame modeled as 20_000 sim-cycle period; 1500-cycle high
        // = 0.075 duty -> 1500 µs -> 90°. Needs ≥2 frames to learn period.
        drive_pwm(&servo, 5, 1000, 1500, 20_000, 3);
        assert!(servo.is_commanded());
        assert!(
            (servo.angle_degrees() - 90.0).abs() < 1.0,
            "angle={}",
            servo.angle_degrees()
        );
    }

    #[test]
    fn gpio_edges_track_a_sweep() {
        let servo = Servo::standard(9);
        // Min end: 1000/20000 = 0.05 -> 0°.
        let t = drive_pwm(&servo, 9, 0, 1000, 20_000, 2);
        assert!(
            servo.angle_degrees().abs() < 1.0,
            "min={}",
            servo.angle_degrees()
        );
        // Max end: 2000/20000 = 0.10 -> 180°.
        drive_pwm(&servo, 9, t, 2000, 20_000, 2);
        assert!(
            (servo.angle_degrees() - 180.0).abs() < 1.0,
            "max={}",
            servo.angle_degrees()
        );
    }

    #[test]
    fn ignores_other_pins() {
        let servo = Servo::standard(5);
        drive_pwm(&servo, 7, 0, 1500, 20_000, 3); // wrong pin
        assert!(!servo.is_commanded());
        assert_eq!(servo.angle_degrees(), 0.0); // parks at min_angle
    }

    #[test]
    fn sg90_calibration_widens_range() {
        // SG90: 0.5–2.4 ms. At 1.5 ms the angle is below center because the
        // range is asymmetric ( (1500-500)/(2400-500) = 0.526 -> ~94.7° ).
        let servo = Servo::sg90(5);
        servo.apply_duty_fraction(0.075); // 1500 µs
        let a = servo.angle_degrees();
        assert!((a - 94.7).abs() < 1.0, "sg90 mid={a}");
    }
}
