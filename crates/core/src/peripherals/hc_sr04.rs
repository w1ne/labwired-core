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
//! list of [`HcSr04`] links. A TRIG **write-hook** ([`observe_trig`]) arms the
//! echo window on the GPIO write that toggles TRIG, and a cheap per-tick pass
//! ([`echo_high_at`]) drives the computed ECHO level back into the input
//! register, touching the bus only when that level changes.
//! Timing is **window-based**: a TRIG rising edge arms an echo window measured
//! in simulated CPU cycles, so the pulse the firmware times matches real
//! hardware in simulated time regardless of how fast the host runs.
//!
//! [`observe_trig`]: HcSr04::observe_trig
//! [`echo_high_at`]: HcSr04::echo_high_at
//!
//! `distance_cm` is host-controlled (e.g. a distance control in the
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
    /// Last ECHO level this sensor drove onto the input register. The per-cycle
    /// pass only touches the bus when the computed level differs from this, so a
    /// steady ECHO costs zero bus accesses.
    last_echo_high: bool,
    /// Cached peripheral index of the GPIO that hosts `trig_odr_addr`, resolved
    /// lazily on the first TRIG write so the write-hook can tell in O(1) whether
    /// a given peripheral write touches this sensor's TRIG line. `None` until
    /// resolved.
    trig_peripheral_idx: Option<usize>,
    /// Event-scheduler path only: set true by [`observe_trig`](Self::observe_trig)
    /// on a TRIG rising edge (a fresh echo window was just computed) and cleared
    /// by [`take_edge_schedule`](Self::take_edge_schedule) once the bus has
    /// scheduled the window's rise/fall edges as scheduler events. Unused by the
    /// legacy per-tick service path.
    #[cfg_attr(not(feature = "event-scheduler"), allow(dead_code))]
    edge_reschedule_pending: bool,
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
            last_echo_high: false,
            trig_peripheral_idx: None,
            edge_reschedule_pending: false,
        }
    }

    /// Cached peripheral index of the GPIO hosting `trig_odr_addr`, if resolved.
    pub(crate) fn trig_peripheral_idx(&self) -> Option<usize> {
        self.trig_peripheral_idx
    }

    /// Cache the resolved peripheral index of the TRIG GPIO (called once, lazily,
    /// by the bus write-hook the first time this sensor's TRIG line is written).
    pub(crate) fn set_trig_peripheral_idx(&mut self, idx: usize) {
        self.trig_peripheral_idx = Some(idx);
    }

    /// The last ECHO level driven onto the input register.
    pub(crate) fn last_echo_high(&self) -> bool {
        self.last_echo_high
    }

    /// Record the ECHO level just driven onto the input register.
    pub(crate) fn set_last_echo_high(&mut self, high: bool) {
        self.last_echo_high = high;
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
}

/// Drivable target distance, in cm (the sensor's physical 2–400 cm window).
/// HC-SR04 sensors live directly on the bus (`SystemBus::hcsr04`), not behind
/// a transport trait, so the bus input walk reaches this impl directly and
/// reports each sensor under its `id`.
impl crate::sim_input::SimInput for HcSr04 {
    fn input_channels(&self) -> &'static [crate::sim_input::InputChannel] {
        use crate::sim_input::InputChannel;
        const CH: &[InputChannel] = &[InputChannel {
            key: "distance",
            label: "Distance",
            unit: "cm",
            min: MIN_CM as f64,
            max: MAX_CM as f64,
        }];
        CH
    }

    fn set_input(&mut self, key: &str, value: f64) -> Result<(), crate::sim_input::SimInputError> {
        self.require_channel(key, value)?;
        self.set_distance_cm(value as f32);
        Ok(())
    }

    fn component_id(&self) -> Option<&str> {
        // Constructed with its system.yaml id (from_config) — already identity.
        Some(&self.id)
    }
}

impl HcSr04 {
    /// Service the sensor for the current tick: `trig_high` is the live TRIG
    /// output level and `now` the current simulated cycle count. Returns the
    /// level the bus should drive onto the ECHO input pin.
    ///
    /// A rising edge on TRIG arms the echo window
    /// `[now + delay, now + delay + echo_µs]` (in cycles); ECHO reads high
    /// while `now` is inside it.
    pub fn service(&mut self, trig_high: bool, now: u64) -> bool {
        self.observe_trig(trig_high, now);
        self.echo_high_at(now)
    }

    /// Observe the live TRIG output level and, on a rising edge, arm the echo
    /// window `[now + delay, now + delay + echo_µs]` (in cycles). Updates the
    /// stored `last_trig` so the next call detects the next edge.
    ///
    /// This is the "arm" half of [`service`](Self::service): with TRIG only ever
    /// changing via a GPIO write, calling this from the bus write-hook (with the
    /// same `now` the per-cycle poll would have used — see the cycle-exactness
    /// note in `bus::SystemBus`) is exactly equivalent to polling every cycle.
    pub fn observe_trig(&mut self, trig_high: bool, now: u64) {
        if trig_high && !self.last_trig {
            let cpu = self.cycles_per_us();
            let delay = (TRIG_TO_ECHO_US as f64 * cpu) as u64;
            let pulse = ((self.distance_cm * US_PER_CM) as f64 * cpu) as u64;
            self.echo_start_cycle = now + delay;
            self.echo_end_cycle = self.echo_start_cycle + pulse.max(1);
            // Event-scheduler path: flag the fresh window so the bus reschedules
            // this sensor's ECHO rise/fall as scheduler events. Harmless (unread)
            // on the legacy per-tick path.
            self.edge_reschedule_pending = true;
        }
        self.last_trig = trig_high;
    }

    /// Event-scheduler path: if a fresh echo window was armed since the last
    /// call, return the absolute CPU cycles at which ECHO should rise and
    /// fall, quantised UP to the peripheral-tick grid
    /// (`ceil(c / interval) * interval` — the first tick boundary at or after
    /// the exact cycle), and clear the pending flag. That boundary is
    /// precisely when the legacy per-tick `service_hcsr04` (running only at
    /// tick boundaries) would first observe the level change, so the scheduled
    /// path stays byte-identical to the per-tick reference (the differential
    /// gate in `tests/hcsr04_event_tick_differential.rs`). At `interval == 1`
    /// this is the exact cycle. Returns `None` when no new window has been
    /// armed.
    #[cfg(feature = "event-scheduler")]
    pub(crate) fn take_edge_schedule(&mut self, interval: u64) -> Option<(u64, u64)> {
        if !self.edge_reschedule_pending {
            return None;
        }
        self.edge_reschedule_pending = false;
        let interval = interval.max(1);
        Some((
            self.echo_start_cycle.div_ceil(interval) * interval,
            self.echo_end_cycle.div_ceil(interval) * interval,
        ))
    }

    /// Event-scheduler path: the cycle of this sensor's next ECHO transition
    /// strictly after `now`, quantised to the `interval` tick boundary its
    /// scheduled event fires on (`ceil(edge / interval) * interval`) so the run
    /// loop can end a batch exactly there. Returns `None` once the armed window
    /// has fully elapsed (no pending transition) — matching the two edges
    /// scheduled by [`take_edge_schedule`](Self::take_edge_schedule). Cheap: two
    /// divides, no allocation.
    #[cfg(feature = "event-scheduler")]
    pub(crate) fn next_edge_deadline_cycle(&self, now: u64, interval: u64) -> Option<u64> {
        let interval = interval.max(1);
        let rise = self.echo_start_cycle.div_ceil(interval) * interval;
        let fall = self.echo_end_cycle.div_ceil(interval) * interval;
        [rise, fall].into_iter().filter(|&c| c > now).min()
    }

    /// The ECHO input level for the current cycle: high while `now` is inside the
    /// armed window `[echo_start, echo_end)`. Pure query — no state change — used
    /// by the cheap per-cycle ECHO drive. Same `>=`/`<` rule as the legacy poll.
    pub fn echo_high_at(&self, now: u64) -> bool {
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
        bus.set_current_cycle(0);
        bus.service_hcsr04();
        assert_eq!(echo(&bus), 0, "echo still low during trig→echo delay");

        // Mid-window: ECHO driven high.
        bus.set_current_cycle(3000);
        bus.service_hcsr04();
        assert_eq!(echo(&bus), 1, "echo high mid-pulse");

        // Past the window: ECHO back low.
        bus.set_current_cycle(7000);
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
