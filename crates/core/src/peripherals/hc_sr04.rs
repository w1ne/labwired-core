// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! HC-SR04 ultrasonic distance sensor.
//!
//! The HC-SR04 is wired to two GPIO lines, not a bus: the MCU pulses **TRIG**
//! (an MCU output) high for ≥10 µs, and the sensor replies by driving **ECHO**
//! (an MCU input) high for a time proportional to distance —
//! `echo_µs = distance_cm × 58` (sound travels ~58 µs per cm, round trip).
//!
//! Because the sensor observes one GPIO and drives another, it can't be a plain
//! MMIO peripheral. Instead the [`SystemBus`](crate::bus::SystemBus) holds a
//! list of [`HcSr04`] links and, once per peripheral-tick, reads the TRIG
//! output bit and writes the computed ECHO level back into the input register.
//! Timing is **window-based**: a TRIG rising edge arms an echo window measured
//! in simulated CPU cycles, so the pulse the firmware times matches real
//! hardware in simulated time regardless of how fast the host runs.
//!
//! `distance_cm` is host-controlled (e.g. a "hand distance" slider in the
//! playground), which is what makes gesture control possible.
//!
//! Modelled from the ElecFreaks HC-SR04 datasheet:
//!   - Trigger: ≥10 µs TTL high pulse starts a ranging.
//!   - The module then emits an **8-cycle 40 kHz** ultrasonic burst and raises
//!     ECHO; ECHO stays high for the round-trip time.
//!   - `µs / 58 = cm` (equivalently `range = high_time × 340 m/s / 2`).
//!   - Range 2 cm – 400 cm, beam angle 15°, ≥60 ms between measurement cycles.

/// Microseconds of echo-high per centimetre of distance. Datasheet:
/// `cm = echo_µs / 58` (round-trip at 340 m/s).
const US_PER_CM: f32 = 58.0;

/// Minimum TRIG high pulse the datasheet requires to start a ranging (µs).
/// Informational — we arm on the rising edge regardless.
#[allow(dead_code)]
const MIN_TRIG_US: f32 = 10.0;

/// Delay from the TRIG pulse to ECHO going high: the module sends an 8-cycle
/// 40 kHz burst first, so the latency is `8 / 40 kHz = 200 µs`.
const TRIG_TO_ECHO_US: f32 = 8.0 / 40_000.0 * 1_000_000.0;

/// Valid HC-SR04 range, in centimetres.
const MIN_CM: f32 = 2.0;
const MAX_CM: f32 = 400.0;

/// One HC-SR04 sensor wired between a TRIG output pin and an ECHO input pin.
#[derive(Debug, Clone)]
pub struct HcSr04 {
    /// board_io / external-device id, used to target the distance setter.
    pub id: String,
    /// Absolute address + bit of the TRIG GPIO **output** register (ODR).
    pub trig_odr_addr: u64,
    pub trig_bit: u8,
    /// Absolute address + bit of the ECHO GPIO **input** register (IDR).
    pub echo_idr_addr: u64,
    pub echo_bit: u8,
    /// CPU clock used to convert microseconds → simulated cycles.
    pub cpu_hz: u64,

    distance_cm: f32,
    last_trig: bool,
    echo_start_cycle: u64,
    echo_end_cycle: u64,
}

impl HcSr04 {
    pub fn new(
        id: String,
        trig_odr_addr: u64,
        trig_bit: u8,
        echo_idr_addr: u64,
        echo_bit: u8,
        cpu_hz: u64,
        initial_distance_cm: f32,
    ) -> Self {
        Self {
            id,
            trig_odr_addr,
            trig_bit,
            echo_idr_addr,
            echo_bit,
            cpu_hz: cpu_hz.max(1),
            distance_cm: initial_distance_cm.clamp(MIN_CM, MAX_CM),
            last_trig: false,
            echo_start_cycle: 0,
            echo_end_cycle: 0,
        }
    }

    /// Set the measured distance (the "hand position"), clamped to range.
    pub fn set_distance_cm(&mut self, cm: f32) {
        self.distance_cm = cm.clamp(MIN_CM, MAX_CM);
    }

    pub fn distance_cm(&self) -> f32 {
        self.distance_cm
    }

    fn cycles_per_us(&self) -> f64 {
        self.cpu_hz as f64 / 1_000_000.0
    }

    /// Service the sensor for the current tick: `trig_high` is the live TRIG
    /// output level and `now` the current simulated cycle count. Returns the
    /// level the bus should drive onto the ECHO input pin.
    ///
    /// A rising edge on TRIG arms the echo window
    /// `[now + delay, now + delay + echo_µs]` (in cycles); ECHO reads high
    /// while `now` is inside it.
    pub fn service(&mut self, trig_high: bool, now: u64) -> bool {
        if trig_high && !self.last_trig {
            let cpu = self.cycles_per_us();
            let delay = (TRIG_TO_ECHO_US as f64 * cpu) as u64;
            let pulse = ((self.distance_cm * US_PER_CM) as f64 * cpu) as u64;
            self.echo_start_cycle = now + delay;
            self.echo_end_cycle = self.echo_start_cycle + pulse.max(1);
        }
        self.last_trig = trig_high;
        now >= self.echo_start_cycle && now < self.echo_end_cycle
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sensor(cpu_hz: u64, cm: f32) -> HcSr04 {
        // TRIG = some ODR bit, ECHO = some IDR bit — addresses irrelevant here.
        HcSr04::new("dist".into(), 0x4800_0014, 0, 0x4800_0410, 1, cpu_hz, cm)
    }

    #[test]
    fn echo_pulse_width_tracks_distance() {
        // 1 MHz CPU → 1 cycle per µs, so cycles == µs (easy arithmetic).
        let mut s = sensor(1_000_000, 100.0); // 100 cm → 5800 µs echo
        let delay = TRIG_TO_ECHO_US as u64; // 100 cycles
        let pulse = (100.0 * US_PER_CM) as u64; // 5800 cycles

        // Rising edge at cycle 0 arms the window.
        assert!(
            !s.service(true, 0),
            "echo not high immediately (within delay)"
        );
        // Still low during the trig→echo delay.
        assert!(!s.service(true, delay - 1));
        // High once the window opens, through its width.
        assert!(s.service(true, delay));
        assert!(s.service(true, delay + pulse - 1));
        // Low again once the pulse ends.
        assert!(!s.service(true, delay + pulse));
    }

    #[test]
    fn closer_hand_gives_shorter_pulse() {
        let mut near = sensor(1_000_000, 10.0); // 580 µs
        let mut far = sensor(1_000_000, 200.0); // 11600 µs
        near.service(true, 0);
        far.service(true, 0);
        let delay = TRIG_TO_ECHO_US as u64;
        // At a cycle inside the near window's end but the far window still open:
        let probe = delay + (10.0 * US_PER_CM) as u64 + 1; // just past near's pulse
        assert!(!near.service(true, probe), "near echo already ended");
        assert!(far.service(true, probe), "far echo still high");
    }

    #[test]
    fn distance_is_clamped_to_range() {
        let mut s = sensor(1_000_000, 100.0);
        s.set_distance_cm(1000.0);
        assert_eq!(s.distance_cm(), MAX_CM);
        s.set_distance_cm(0.0);
        assert_eq!(s.distance_cm(), MIN_CM);
    }

    /// End-to-end through the bus: pulse a real TRIG GPIO output, then the
    /// per-tick service pass reads it and drives the ECHO GPIO input register
    /// high for the distance-proportional window.
    #[test]
    fn echo_driven_through_bus() {
        use crate::bus::SystemBus;
        use crate::peripherals::gpio::{GpioPort, GpioRegisterLayout};
        use crate::Bus;

        const GPIOA: u64 = 0x4800_0000; // stm32v2: ODR @ 0x14, IDR @ 0x10, BSRR @ 0x18
        let echo_bit = 9u8; // PA9 (ECHO input)

        let mut bus = SystemBus::empty();
        bus.add_peripheral(
            "gpioa",
            GPIOA,
            0x400,
            None,
            Box::new(GpioPort::new_with_layout(GpioRegisterLayout::Stm32V2)),
        );
        // TRIG = PA8 (ODR bit 8), ECHO = PA9 (IDR bit 9), 1 MHz, 100 cm.
        bus.hcsr04.push(HcSr04::new(
            "dist".into(),
            GPIOA + 0x14,
            8,
            GPIOA + 0x10,
            echo_bit,
            1_000_000,
            100.0,
        ));

        let echo = |bus: &SystemBus| (bus.read_u32(GPIOA + 0x10).unwrap() >> echo_bit) & 1;

        // Pulse TRIG high via BSRR, service at cycle 0 → arms window [200, 6000).
        bus.write_u32(GPIOA + 0x18, 1 << 8).unwrap();
        bus.current_cycle = 0;
        bus.service_hcsr04();
        assert_eq!(echo(&bus), 0, "echo still low during trig→echo delay");

        // Mid-window: ECHO driven high.
        bus.current_cycle = 3000;
        bus.service_hcsr04();
        assert_eq!(echo(&bus), 1, "echo high mid-pulse");

        // Past the window: ECHO back low.
        bus.current_cycle = 7000;
        bus.service_hcsr04();
        assert_eq!(echo(&bus), 0, "echo low after pulse");
    }

    #[test]
    fn cpu_clock_scales_cycle_count() {
        // 80 MHz → 80 cycles per µs. A 2 cm (116 µs) pulse = 9280 cycles.
        let mut s = sensor(80_000_000, 2.0);
        s.service(true, 0);
        let delay = (TRIG_TO_ECHO_US as u64) * 80;
        let pulse = (2.0 * US_PER_CM) as u64 * 80; // 116 * 80 = 9280
        assert!(s.service(true, delay + pulse - 1));
        assert!(!s.service(true, delay + pulse));
    }
}
