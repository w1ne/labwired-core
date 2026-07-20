// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! DHT22 / AM2302 single-wire temperature + humidity sensor.
//!
//! Unlike every I²C/SPI sensor in this tree, the DHT22 is not a bus slave that
//! answers register reads — it **drives the MCU's GPIO line itself**. One wire
//! carries both directions, open-drain with an external pull-up:
//!
//! 1. **Start.** The MCU pulls the line LOW for ≥ 1 ms, then releases it. The
//!    pull-up takes the line high for 20–40 µs while the sensor wakes.
//! 2. **Response.** The sensor pulls LOW for 80 µs, then HIGH for 80 µs.
//! 3. **40 data bits.** Each bit is a 50 µs LOW slot followed by a HIGH pulse
//!    whose *width* is the value: 26–28 µs = `0`, 70 µs = `1`.
//! 4. **Frame.** 16-bit humidity, 16-bit temperature, 8-bit checksum, MSB
//!    first. Temperature is **sign-magnitude** — bit 15 marks a negative
//!    reading, it is *not* two's complement.
//! 5. The sensor then releases the line and it idles high until the next start
//!    pulse. There is no free-running output: a read with no valid start pulse
//!    sees nothing but idle high, which is exactly why a firmware bug in the
//!    start pulse shows up as a read timeout on real hardware.
//!
//! ## How it is wired into the simulator
//!
//! This mirrors [`HcSr04`](crate::peripherals::hc_sr04::HcSr04) exactly, which
//! is the one existing model that also drives a pin the MCU samples as an
//! input:
//!
//! * **Observing the host** — a GPIO **write-hook**
//!   ([`SystemBus::maybe_start_dht22`](crate::bus::SystemBus)) re-reads the data
//!   pin's ODR bit after any MMIO write to that GPIO and feeds it to
//!   [`observe_line`](Dht22::observe_line). The host line only ever changes via
//!   a GPIO write, so edge detection on the write is exactly equivalent to
//!   polling every cycle — and cycle-exact, because `current_cycle` at the
//!   write equals what the following tick would have seen.
//! * **Driving the pad** — a cheap per-tick pass
//!   ([`SystemBus::service_dht22`](crate::bus::SystemBus)) computes
//!   [`pad_high_at`](Dht22::pad_high_at) and writes it into the pin's **IDR**
//!   bit, touching the bus only when the level actually changes. Same
//!   mechanism `HcSr04` uses to drive ECHO.
//! * **Advancing in time** — arming is window-based, in simulated CPU cycles.
//!   Where `HcSr04` arms a single `[start, end)` echo window,
//!   [`observe_line`](Dht22::observe_line) precomputes the sensor's *entire*
//!   transition schedule for one frame (83 edges) as absolute cycle counts, and
//!   [`sensor_high_at`](Dht22::sensor_high_at) is a pure binary search over it.
//!   No wall-clock, no host timing: the pulse widths the firmware measures match
//!   the datasheet in simulated time however fast (or slow) the host runs.
//!
//! Because one wire is shared, the pad level is the **wired-AND** of the two
//! drivers: the MCU pulling low wins over the pull-up, so the firmware reads
//! back its own start pulse, then the sensor's reply once it has released.
//!
//! Modelled from the Aosong AM2302 / DHT22 datasheet:
//!   - Range −40…80 °C (±0.5 °C), 0…100 %RH (±2 %RH).
//!   - Both values are transmitted ×10 as 16-bit integers.
//!   - Checksum = low 8 bits of the sum of the four data bytes.
//!   - ≥ 2 s between reads (sampling period); not enforced here — the model
//!     answers every valid start pulse.

/// Minimum LOW the MCU must hold to be a valid start pulse (µs). The datasheet
/// asks for ≥ 1 ms; anything shorter the sensor ignores, which is the failure
/// mode this model reproduces rather than papers over.
const MIN_START_LOW_US: f64 = 1_000.0;

/// Pull-up interval between the MCU releasing the line and the sensor answering
/// (datasheet 20–40 µs).
const RESPONSE_DELAY_US: f64 = 30.0;

/// Sensor's response pulse: LOW then HIGH, 80 µs each.
const RESPONSE_LOW_US: f64 = 80.0;
const RESPONSE_HIGH_US: f64 = 80.0;

/// Every data bit opens with a 50 µs LOW slot.
const BIT_LOW_US: f64 = 50.0;
/// HIGH width that encodes a `0` (datasheet 26–28 µs).
const BIT_0_HIGH_US: f64 = 27.0;
/// HIGH width that encodes a `1` (datasheet 70 µs).
const BIT_1_HIGH_US: f64 = 70.0;

/// Bits in one frame: 16 humidity + 16 temperature + 8 checksum.
pub const FRAME_BITS: usize = 40;

/// Sensor range, from the datasheet.
const MIN_TEMP_C: f32 = -40.0;
const MAX_TEMP_C: f32 = 80.0;
const MIN_HUMIDITY_PCT: f32 = 0.0;
const MAX_HUMIDITY_PCT: f32 = 100.0;

/// One DHT22 hanging off a single bidirectional GPIO line.
#[derive(Debug, Clone)]
pub struct Dht22 {
    /// board_io / external-device id, used to target the stimulus setters.
    pub id: String,
    /// Absolute address + bit of the data pin's GPIO **output** register (ODR),
    /// which is how the model sees the host driving the line.
    pub data_odr_addr: u64,
    /// Absolute address + bit of the same pin's GPIO **input** register (IDR),
    /// which is where the model drives the pad level back.
    pub data_idr_addr: u64,
    pub data_bit: u8,
    /// CPU clock used to convert microseconds → simulated cycles.
    pub cpu_hz: u64,

    temperature_c: f32,
    humidity_pct: f32,

    /// Last host-driven level observed on the ODR bit. The line idles high
    /// (external pull-up), so this starts `true`.
    host_high: bool,
    /// Cycle of the most recent host falling edge, if the line is currently
    /// held low by the MCU.
    low_start_cycle: Option<u64>,
    /// Sensor-driven transition schedule for the armed frame: `(cycle, level)`
    /// pairs in strictly increasing cycle order, each meaning "from this cycle
    /// on, the sensor drives `level`". Empty = nothing armed, line idles high.
    transitions: Vec<(u64, bool)>,
    /// The 40 bits the armed frame carries, MSB first in the low 40 bits.
    /// Latched at arm time so a mid-frame `set_input` cannot corrupt a
    /// transmission already on the wire.
    frame: u64,
    /// Last pad level this sensor drove onto the input register. The per-tick
    /// pass only touches the bus when the computed level differs, so an idle
    /// line costs zero bus accesses.
    last_pad_high: bool,
    /// Cached peripheral index of the GPIO hosting the data pin, resolved
    /// lazily on the first write so the write-hook can tell in O(1) whether a
    /// given peripheral write touches this sensor. `None` until resolved.
    data_peripheral_idx: Option<usize>,
}

impl Dht22 {
    pub fn new(
        id: String,
        data_odr_addr: u64,
        data_idr_addr: u64,
        data_bit: u8,
        cpu_hz: u64,
        initial_temperature_c: f32,
        initial_humidity_pct: f32,
    ) -> Self {
        Self {
            id,
            data_odr_addr,
            data_idr_addr,
            data_bit,
            cpu_hz: cpu_hz.max(1),
            temperature_c: initial_temperature_c.clamp(MIN_TEMP_C, MAX_TEMP_C),
            humidity_pct: initial_humidity_pct.clamp(MIN_HUMIDITY_PCT, MAX_HUMIDITY_PCT),
            host_high: true,
            low_start_cycle: None,
            transitions: Vec::new(),
            frame: 0,
            // Start "low" so the very first service tick drives the idle-high
            // pull-up level onto the IDR bit (which resets to 0).
            last_pad_high: false,
            data_peripheral_idx: None,
        }
    }

    /// Cached peripheral index of the GPIO hosting the data pin, if resolved.
    pub(crate) fn data_peripheral_idx(&self) -> Option<usize> {
        self.data_peripheral_idx
    }

    /// Cache the resolved peripheral index of the data GPIO (called once,
    /// lazily, by the bus write-hook on the first write to that port).
    pub(crate) fn set_data_peripheral_idx(&mut self, idx: usize) {
        self.data_peripheral_idx = Some(idx);
    }

    /// The last pad level driven onto the input register.
    pub(crate) fn last_pad_high(&self) -> bool {
        self.last_pad_high
    }

    /// Record the pad level just driven onto the input register.
    pub(crate) fn set_last_pad_high(&mut self, high: bool) {
        self.last_pad_high = high;
    }

    fn cycles_per_us(&self) -> f64 {
        self.cpu_hz as f64 / 1_000_000.0
    }

    fn cycles(&self, us: f64) -> u64 {
        // Match HcSr04: truncating µs→cycle conversion off the CPU clock.
        (us * self.cycles_per_us()) as u64
    }

    // ─── Readback (tests / debug / UI) ────────────────────────────────────

    /// Current temperature the sensor will report, in °C.
    pub fn temperature_c(&self) -> f32 {
        self.temperature_c
    }

    /// Current relative humidity the sensor will report, in %RH.
    pub fn humidity_pct(&self) -> f32 {
        self.humidity_pct
    }

    /// Set the reported temperature (°C), clamped to the datasheet range.
    pub fn set_temperature_c(&mut self, c: f32) {
        self.temperature_c = c.clamp(MIN_TEMP_C, MAX_TEMP_C);
    }

    /// Set the reported relative humidity (%RH), clamped to 0–100.
    pub fn set_humidity_pct(&mut self, pct: f32) {
        self.humidity_pct = pct.clamp(MIN_HUMIDITY_PCT, MAX_HUMIDITY_PCT);
    }

    /// The five frame bytes for the *current* readings: `[h_hi, h_lo, t_hi,
    /// t_lo, checksum]`. Temperature is sign-magnitude — bit 15 of the 16-bit
    /// temperature word marks a negative reading.
    pub fn frame_bytes(&self) -> [u8; 5] {
        let h_raw = (self.humidity_pct * 10.0).round().clamp(0.0, 1000.0) as u16;
        let magnitude = (self.temperature_c.abs() * 10.0).round().clamp(0.0, 800.0) as u16;
        let t_raw = if self.temperature_c < 0.0 {
            magnitude | 0x8000
        } else {
            magnitude
        };
        let b = [
            (h_raw >> 8) as u8,
            h_raw as u8,
            (t_raw >> 8) as u8,
            t_raw as u8,
        ];
        let checksum = b.iter().fold(0u8, |acc, &byte| acc.wrapping_add(byte));
        [b[0], b[1], b[2], b[3], checksum]
    }

    /// The 40-bit frame for the current readings, MSB first in the low 40 bits.
    pub fn frame_bits(&self) -> u64 {
        self.frame_bytes()
            .iter()
            .fold(0u64, |acc, &byte| (acc << 8) | byte as u64)
    }

    /// The 40-bit frame currently *latched on the wire* — what an armed
    /// transmission is actually sending, which may lag
    /// [`frame_bits`](Self::frame_bits) if the readings changed mid-frame.
    /// Zero when nothing is armed.
    pub fn armed_frame_bits(&self) -> u64 {
        self.frame
    }

    /// The sensor's transition schedule for the armed frame: `(cycle, level)`
    /// pairs, each "from this cycle on the sensor drives `level`". Empty when
    /// idle. Exposed for tests and waveform debugging.
    pub fn transitions(&self) -> &[(u64, bool)] {
        &self.transitions
    }

    // ─── Protocol ─────────────────────────────────────────────────────────

    /// Observe the live host-driven level on the data pin at simulated cycle
    /// `now`. A falling edge starts timing the start pulse; a rising edge that
    /// closes a ≥ 1 ms LOW arms one full frame beginning at `now`. A rising
    /// edge closing a shorter LOW is ignored — the real sensor does not answer
    /// it.
    ///
    /// This is the DHT22's counterpart to
    /// [`HcSr04::observe_trig`](crate::peripherals::hc_sr04::HcSr04::observe_trig)
    /// and is driven from the same GPIO write-hook.
    pub fn observe_line(&mut self, host_high: bool, now: u64) {
        if !host_high && self.host_high {
            // Falling edge: the MCU has taken the line low. Start timing.
            self.low_start_cycle = Some(now);
            // Anything still on the wire is aborted by the host pulling low.
            self.transitions.clear();
        } else if host_high && !self.host_high {
            // Rising edge: the MCU released the line. Answer only if the LOW
            // it just ended was long enough to be a start pulse.
            if let Some(start) = self.low_start_cycle.take() {
                let held = now.saturating_sub(start);
                if held >= self.cycles(MIN_START_LOW_US) {
                    self.arm_frame(now);
                }
            }
        }
        self.host_high = host_high;
    }

    /// Precompute the sensor's whole transition schedule for one frame,
    /// starting from the host's release at cycle `release`. 1 + 2 + 80 edges:
    /// the response low/high pair, then a low/high pair per data bit, then the
    /// final release back to idle high.
    fn arm_frame(&mut self, release: u64) {
        let frame = self.frame_bits();
        self.frame = frame;

        let mut t = release + self.cycles(RESPONSE_DELAY_US);
        let mut edges: Vec<(u64, bool)> = Vec::with_capacity(2 * FRAME_BITS + 3);

        // Response: 80 µs LOW then 80 µs HIGH.
        edges.push((t, false));
        t += self.cycles(RESPONSE_LOW_US);
        edges.push((t, true));
        t += self.cycles(RESPONSE_HIGH_US);

        // 40 data bits, MSB first: 50 µs LOW slot, then a value-width HIGH.
        for i in (0..FRAME_BITS).rev() {
            let bit = (frame >> i) & 1 != 0;
            edges.push((t, false));
            t += self.cycles(BIT_LOW_US);
            edges.push((t, true));
            t += self.cycles(if bit { BIT_1_HIGH_US } else { BIT_0_HIGH_US });
        }

        // The last bit's HIGH runs straight into the idle-high release, so the
        // closing edge is a no-op level-wise; push it anyway so the schedule
        // carries an explicit end-of-frame cycle.
        edges.push((t, true));
        self.transitions = edges;
    }

    /// The level the *sensor* drives at cycle `now`: idle high (external
    /// pull-up) outside an armed frame, otherwise the level the precomputed
    /// schedule implies. Pure query — no state change — so the per-tick pad
    /// drive stays cheap. Binary search, `O(log 83)`.
    pub fn sensor_high_at(&self, now: u64) -> bool {
        if self.transitions.is_empty() {
            return true;
        }
        let after = self.transitions.partition_point(|&(c, _)| c <= now);
        if after == 0 {
            // Still in the pull-up gap before the sensor answers.
            return true;
        }
        self.transitions[after - 1].1
    }

    /// The level on the pad at cycle `now`: the **wired-AND** of the two
    /// drivers on this open-drain line. The MCU pulling LOW wins over the
    /// pull-up and over the sensor, so firmware reads back its own start pulse;
    /// once the MCU releases, the sensor owns the line.
    pub fn pad_high_at(&self, now: u64) -> bool {
        self.host_high && self.sensor_high_at(now)
    }

    /// Whether a frame is currently armed (a valid start pulse was answered and
    /// the transmission has not been aborted).
    pub fn is_transmitting(&self) -> bool {
        !self.transitions.is_empty()
    }

    /// Convenience for unit tests and headless drivers: feed the live host
    /// level at cycle `now` and get back the resulting pad level, exactly what
    /// the write-hook + per-tick pair does through the bus.
    pub fn service(&mut self, host_high: bool, now: u64) -> bool {
        self.observe_line(host_high, now);
        self.pad_high_at(now)
    }
}

/// Drivable temperature / humidity, matching the channel vocabulary the BME280
/// kit already uses (`temperature` in °C, `humidity` in %RH) so an agent driving
/// a weather demo does not have to learn a second spelling per part.
///
/// DHT22 sensors live directly on the bus (`SystemBus::dht22`), not behind a
/// transport trait, so the bus input walk reaches this impl directly and reports
/// each sensor under its `id` — same as [`HcSr04`](crate::peripherals::hc_sr04::HcSr04).
pub const INPUT_CHANNELS: &[crate::sim_input::InputChannel] = &[
    crate::sim_input::InputChannel {
        key: "temperature",
        label: "Temperature",
        unit: "°C",
        min: MIN_TEMP_C as f64,
        max: MAX_TEMP_C as f64,
    },
    crate::sim_input::InputChannel {
        key: "humidity",
        label: "Humidity",
        unit: "%RH",
        min: MIN_HUMIDITY_PCT as f64,
        max: MAX_HUMIDITY_PCT as f64,
    },
];

impl crate::sim_input::SimInput for Dht22 {
    fn input_channels(&self) -> &'static [crate::sim_input::InputChannel] {
        INPUT_CHANNELS
    }

    fn set_input(&mut self, key: &str, value: f64) -> Result<(), crate::sim_input::SimInputError> {
        let ch = self.require_channel(key, value)?;
        match ch.key {
            "temperature" => self.set_temperature_c(value as f32),
            "humidity" => self.set_humidity_pct(value as f32),
            _ => unreachable!("require_channel only returns declared channels"),
        }
        Ok(())
    }

    fn component_id(&self) -> Option<&str> {
        // Constructed with its system.yaml id (from_config) — already identity.
        Some(&self.id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sim_input::SimInput;

    /// 1 MHz CPU → 1 cycle per µs, so cycles == µs and the datasheet numbers
    /// read straight out of the assertions.
    const HZ: u64 = 1_000_000;

    fn sensor(t: f32, h: f32) -> Dht22 {
        // stm32v2 layout: IDR @ 0x10, ODR @ 0x14 — same port, same bit.
        Dht22::new("env".into(), 0x4800_0014, 0x4800_0010, 4, HZ, t, h)
    }

    /// Drive a valid start pulse: LOW for `low_us`, released at `release`.
    /// Returns the release cycle.
    fn start_pulse(s: &mut Dht22, low_us: u64) -> u64 {
        s.observe_line(true, 0); // idle high
        s.observe_line(false, 10); // MCU pulls low at cycle 10
        let release = 10 + low_us;
        s.observe_line(true, release);
        release
    }

    /// Walk the armed schedule and decode it the way real firmware does:
    /// measure each HIGH pulse width, `> 40 µs` = 1. Returns
    /// `(response_low_us, response_high_us, bits)`.
    fn decode(s: &Dht22) -> (u64, u64, u64) {
        let tr = s.transitions();
        assert!(!tr.is_empty(), "nothing armed to decode");
        // tr[0] = response LOW, tr[1] = response HIGH, tr[2] = first bit LOW…
        let response_low = tr[1].0 - tr[0].0;
        let response_high = tr[2].0 - tr[1].0;
        let mut bits = 0u64;
        for i in 0..FRAME_BITS {
            let low_edge = tr[2 + 2 * i];
            let high_edge = tr[3 + 2 * i];
            let end = tr[4 + 2 * i];
            assert!(!low_edge.1, "bit {i} slot must open LOW");
            assert!(high_edge.1, "bit {i} must pulse HIGH");
            assert_eq!(
                high_edge.0 - low_edge.0,
                BIT_LOW_US as u64,
                "bit {i} LOW slot must be 50 µs"
            );
            let width = end.0 - high_edge.0;
            bits = (bits << 1) | u64::from(width > 40);
        }
        (response_low, response_high, bits)
    }

    #[test]
    fn start_pulse_produces_response_then_forty_bits() {
        let mut s = sensor(23.4, 65.3);
        let release = start_pulse(&mut s, 1_100);
        assert!(s.is_transmitting(), "a ≥1 ms start pulse must be answered");

        let tr = s.transitions().to_vec();
        // 2 response edges + 2 per data bit + the closing release.
        assert_eq!(tr.len(), 2 * FRAME_BITS + 3);

        // Sensor waits out the pull-up gap, then pulls low.
        assert_eq!(tr[0].0, release + RESPONSE_DELAY_US as u64);
        assert!(!tr[0].1, "response starts LOW");
        assert!(s.sensor_high_at(release), "pull-up gap is high");
        assert!(!s.sensor_high_at(tr[0].0), "sensor pulls low at t0");

        let (resp_low, resp_high, _) = decode(&s);
        assert_eq!(resp_low, RESPONSE_LOW_US as u64, "80 µs response LOW");
        assert_eq!(resp_high, RESPONSE_HIGH_US as u64, "80 µs response HIGH");
    }

    #[test]
    fn bit_widths_encode_zero_and_one() {
        // 0.0 %RH / 0.0 °C is an all-zero frame except the checksum: every data
        // bit of the first 32 is a 0. 100.0 %RH = 1000 = 0x03E8 gives 1 bits.
        let mut s = sensor(0.0, 0.0);
        start_pulse(&mut s, 1_100);
        let tr = s.transitions().to_vec();
        // First data bit (MSB of humidity) is 0 → 50 µs LOW then 27 µs HIGH.
        assert_eq!(tr[3].0 - tr[2].0, BIT_LOW_US as u64);
        assert_eq!(tr[4].0 - tr[3].0, BIT_0_HIGH_US as u64, "a 0 bit is ~27 µs");

        // 100 %RH → humidity 1000 = 0b0000_0011_1110_1000; bit 9 (0-indexed
        // from the MSB) is the first set bit.
        let mut s = sensor(0.0, 100.0);
        start_pulse(&mut s, 1_100);
        let tr = s.transitions().to_vec();
        let first_one = 6usize; // 0x03E8: bits 0..5 are 0, bit 6 is the first 1
        let low_edge = tr[2 + 2 * first_one];
        let high_edge = tr[3 + 2 * first_one];
        let end = tr[4 + 2 * first_one];
        assert_eq!(
            high_edge.0 - low_edge.0,
            BIT_LOW_US as u64,
            "50 µs LOW slot"
        );
        assert_eq!(
            end.0 - high_edge.0,
            BIT_1_HIGH_US as u64,
            "a 1 bit is 70 µs"
        );

        // And the decode round-trips to the encoder's own frame.
        let (_, _, bits) = decode(&s);
        assert_eq!(bits, s.frame_bits());
    }

    #[test]
    fn known_reading_encodes_expected_frame_and_checksum() {
        // 65.3 %RH → 653 = 0x028D; 23.4 °C → 234 = 0x00EA.
        // checksum = (0x02 + 0x8D + 0x00 + 0xEA) & 0xFF = 0x179 & 0xFF = 0x79.
        let s = sensor(23.4, 65.3);
        assert_eq!(s.frame_bytes(), [0x02, 0x8D, 0x00, 0xEA, 0x79]);
        assert_eq!(s.frame_bits(), 0x02_8D_00_EA_79);

        // Checksum is genuinely the low byte of the sum of the four data bytes.
        let b = s.frame_bytes();
        let sum: u32 = b[..4].iter().map(|&x| x as u32).sum();
        assert_eq!(b[4] as u32, sum & 0xFF);

        // …and the wire carries exactly those bits.
        let mut s = s;
        start_pulse(&mut s, 1_100);
        let (_, _, bits) = decode(&s);
        assert_eq!(bits, 0x02_8D_00_EA_79);
    }

    #[test]
    fn negative_temperature_uses_sign_magnitude_bit15() {
        // −10.1 °C → magnitude 101 = 0x0065, sign bit set → 0x8065.
        // 40.0 %RH → 400 = 0x0190.
        // checksum = (0x01 + 0x90 + 0x80 + 0x65) & 0xFF = 0x176 & 0xFF = 0x76.
        let mut s = sensor(-10.1, 40.0);
        assert_eq!(s.frame_bytes(), [0x01, 0x90, 0x80, 0x65, 0x76]);

        let t_word = u16::from_be_bytes([s.frame_bytes()[2], s.frame_bytes()[3]]);
        assert_eq!(t_word & 0x8000, 0x8000, "bit 15 marks negative");
        assert_eq!(
            t_word & 0x7FFF,
            101,
            "magnitude, NOT two's complement (which would be 0xFF9B)"
        );

        // Checksum still consistent with the sign-magnitude bytes.
        let b = s.frame_bytes();
        let sum: u32 = b[..4].iter().map(|&x| x as u32).sum();
        assert_eq!(b[4] as u32, sum & 0xFF);

        start_pulse(&mut s, 1_100);
        let (_, _, bits) = decode(&s);
        assert_eq!(bits, 0x01_90_80_65_76);
    }

    #[test]
    fn set_input_changes_the_next_frame() {
        let mut s = sensor(20.0, 50.0);
        let before = s.frame_bits();

        s.set_input("temperature", -12.5).unwrap();
        s.set_input("humidity", 88.8).unwrap();
        assert_eq!(s.temperature_c(), -12.5);
        assert_eq!(s.humidity_pct(), 88.8);

        let after = s.frame_bits();
        assert_ne!(before, after, "set_input must change the encoded frame");

        // 88.8 %RH → 888 = 0x0378; −12.5 °C → 125 = 0x007D | 0x8000 = 0x807D.
        assert_eq!(s.frame_bytes()[..4], [0x03, 0x78, 0x80, 0x7D]);

        // And the next transmission carries the NEW values.
        start_pulse(&mut s, 1_100);
        let (_, _, bits) = decode(&s);
        assert_eq!(bits, after);

        // Unknown channels are rejected, not silently swallowed.
        assert!(s.set_input("pressure", 1013.0).is_err());
    }

    #[test]
    fn no_start_pulse_means_the_line_stays_idle_high() {
        let mut s = sensor(23.4, 65.3);
        // Never touch the line: sample far into the future.
        for now in [0u64, 1_000, 100_000, 10_000_000] {
            assert!(s.sensor_high_at(now), "idle high at {now}");
            assert!(s.pad_high_at(now), "pad idle high at {now}");
        }
        assert!(!s.is_transmitting(), "no spontaneous bit stream");

        // Even the host merely wiggling high (no LOW at all) arms nothing.
        s.observe_line(true, 10);
        s.observe_line(true, 20);
        assert!(!s.is_transmitting());
    }

    #[test]
    fn too_short_start_pulse_is_ignored() {
        let mut s = sensor(23.4, 65.3);
        // 999 µs < the 1 ms the datasheet requires.
        start_pulse(&mut s, 999);
        assert!(!s.is_transmitting(), "sub-1 ms start pulse must be ignored");
        assert!(s.sensor_high_at(5_000), "line stays idle high");

        // Exactly 1 ms is accepted (boundary is inclusive).
        let mut s = sensor(23.4, 65.3);
        start_pulse(&mut s, 1_000);
        assert!(s.is_transmitting(), "exactly 1 ms is a valid start pulse");
    }

    #[test]
    fn host_pulling_low_wins_the_wired_and() {
        let mut s = sensor(23.4, 65.3);
        s.observe_line(true, 0);
        s.observe_line(false, 10);
        // While the MCU holds the line low, the pad reads low even though the
        // sensor is not driving anything.
        assert!(s.sensor_high_at(500), "sensor itself is idle-high");
        assert!(!s.pad_high_at(500), "but the host is pulling the pad low");
        // Release → pad follows the sensor again.
        s.observe_line(true, 1_100);
        assert!(s.pad_high_at(1_100));
    }

    #[test]
    fn a_new_start_pulse_aborts_and_re_arms() {
        let mut s = sensor(23.4, 65.3);
        start_pulse(&mut s, 1_100);
        assert!(s.is_transmitting());
        // Host yanks the line low again mid-frame: transmission is aborted.
        s.observe_line(false, 1_200);
        assert!(!s.is_transmitting(), "host low aborts the frame");
        // …and a fresh valid pulse re-arms with the current readings.
        s.set_temperature_c(1.0);
        s.observe_line(true, 3_000);
        assert!(s.is_transmitting());
        let (_, _, bits) = decode(&s);
        assert_eq!(bits, s.frame_bits());
    }

    #[test]
    fn cpu_clock_scales_the_schedule() {
        // 80 MHz → 80 cycles per µs; every datasheet interval scales.
        let mut s = Dht22::new("env".into(), 0x14, 0x10, 4, 80_000_000, 23.4, 65.3);
        s.observe_line(true, 0);
        s.observe_line(false, 0);
        // 1 ms at 80 MHz = 80_000 cycles.
        s.observe_line(true, 79_999);
        assert!(!s.is_transmitting(), "one cycle short of 1 ms");

        let mut s = Dht22::new("env".into(), 0x14, 0x10, 4, 80_000_000, 23.4, 65.3);
        s.observe_line(true, 0);
        s.observe_line(false, 0);
        s.observe_line(true, 80_000);
        assert!(s.is_transmitting());
        let tr = s.transitions();
        assert_eq!(tr[1].0 - tr[0].0, 80 * 80, "80 µs response LOW at 80 MHz");
    }

    #[test]
    fn readings_are_clamped_to_the_datasheet_range() {
        let mut s = sensor(0.0, 0.0);
        s.set_temperature_c(200.0);
        assert_eq!(s.temperature_c(), MAX_TEMP_C);
        s.set_temperature_c(-200.0);
        assert_eq!(s.temperature_c(), MIN_TEMP_C);
        s.set_humidity_pct(500.0);
        assert_eq!(s.humidity_pct(), MAX_HUMIDITY_PCT);
        s.set_humidity_pct(-5.0);
        assert_eq!(s.humidity_pct(), MIN_HUMIDITY_PCT);
    }

    /// End-to-end through the bus: bit-bang a real start pulse on a GPIO ODR
    /// bit, then let the per-tick service pass drive the same pin's IDR bit
    /// with the sensor's reply — the mechanism a real firmware read uses.
    #[test]
    fn frame_driven_through_bus() {
        use crate::bus::SystemBus;
        use crate::peripherals::gpio::{GpioPort, GpioRegisterLayout};
        use crate::Bus;

        const GPIOA: u64 = 0x4800_0000; // stm32v2: IDR @ 0x10, ODR @ 0x14, BSRR @ 0x18
        let bit = 4u8;

        let mut bus = SystemBus::empty();
        bus.add_peripheral(
            "gpioa",
            GPIOA,
            0x400,
            None,
            Box::new(GpioPort::new_with_layout(GpioRegisterLayout::Stm32V2)),
        );
        bus.dht22.push(Dht22::new(
            "env".into(),
            GPIOA + 0x14,
            GPIOA + 0x10,
            bit,
            HZ,
            23.4,
            65.3,
        ));

        let pad = |bus: &SystemBus| (bus.read_u32(GPIOA + 0x10).unwrap() >> bit) & 1;

        // Idle: the first service tick drives the pull-up level high.
        bus.set_current_cycle(0);
        bus.write_u32(GPIOA + 0x18, 1 << bit).unwrap(); // release (BSRR set)
        bus.service_dht22();
        assert_eq!(pad(&bus), 1, "line idles high via the pull-up");

        // MCU pulls the line low for 1.1 ms (BSRR reset bit).
        bus.set_current_cycle(100);
        bus.write_u32(GPIOA + 0x18, 1 << (bit + 16)).unwrap();
        bus.service_dht22();
        assert_eq!(pad(&bus), 0, "host start pulse reads back low");

        // Release at cycle 1_200 → frame armed.
        bus.set_current_cycle(1_200);
        bus.write_u32(GPIOA + 0x18, 1 << bit).unwrap();
        bus.service_dht22();
        assert_eq!(pad(&bus), 1, "pull-up gap before the sensor answers");
        assert!(bus.dht22[0].is_transmitting(), "write-hook armed the frame");

        // 30 µs later the sensor pulls low for its 80 µs response.
        bus.set_current_cycle(1_200 + 30);
        bus.service_dht22();
        assert_eq!(pad(&bus), 0, "sensor response LOW");

        bus.set_current_cycle(1_200 + 30 + 80);
        bus.service_dht22();
        assert_eq!(pad(&bus), 1, "sensor response HIGH");

        // Sample the whole frame at 1-cycle (=1 µs) resolution and decode it
        // the way firmware does, by timing HIGH pulses after each LOW slot.
        let end = bus.dht22[0].transitions().last().unwrap().0;
        let mut levels = Vec::new();
        for c in (1_200 + 30)..=end {
            bus.set_current_cycle(c);
            bus.service_dht22();
            levels.push(pad(&bus) == 1);
        }
        // Walk the sampled waveform: skip the response pair, then each
        // LOW→HIGH run pair is one bit.
        let mut runs: Vec<(bool, usize)> = Vec::new();
        for lvl in levels {
            match runs.last_mut() {
                Some((l, n)) if *l == lvl => *n += 1,
                _ => runs.push((lvl, 1)),
            }
        }
        // runs[0] = 80 µs response low, runs[1] = 80 µs response high.
        assert_eq!(runs[0], (false, 80));
        assert_eq!(runs[1], (true, 80));
        let mut bits = 0u64;
        for i in 0..FRAME_BITS {
            let (low_lvl, low_n) = runs[2 + 2 * i];
            let (high_lvl, high_n) = runs[3 + 2 * i];
            assert!(!low_lvl && low_n == 50, "bit {i} 50 µs LOW slot");
            assert!(high_lvl, "bit {i} HIGH pulse");
            bits = (bits << 1) | u64::from(high_n > 40);
        }
        assert_eq!(bits, 0x02_8D_00_EA_79, "decoded frame matches the encoder");
    }

    /// The sensor is reachable from the ONE bus stimulus walk, so `set_input`
    /// / `list_inputs` (test-script `stimuli:`, MCP, wasm) all see it.
    #[test]
    fn reachable_from_the_bus_input_walk() {
        use crate::bus::SystemBus;

        let mut bus = SystemBus::empty();
        bus.dht22.push(Dht22::new(
            "env".into(),
            0x4800_0014,
            0x4800_0010,
            4,
            HZ,
            23.4,
            65.3,
        ));

        let channels = bus.list_inputs();
        assert!(
            channels
                .iter()
                .any(|(owner, ch)| owner == "env" && ch.key == "temperature"),
            "temperature channel not discovered: {channels:?}"
        );
        assert!(
            channels
                .iter()
                .any(|(owner, ch)| owner == "env" && ch.key == "humidity"),
            "humidity channel not discovered: {channels:?}"
        );
    }
}
