// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

use crate::peripherals::i2c::I2cDevice;

/// Bosch BME280 factory calibration coefficients.
///
/// ONE source of truth for the part's calibration: the calib registers the
/// device serves (0x88..0xA1, 0xE1..0xE7) are *serialized from these fields*
/// (see [`Bme280::read_register`]), and the compensation math below reads the
/// same fields. A driver that slurps the calib blob and a stimulus that
/// inverts the compensation therefore cannot disagree.
///
/// Values are the Bosch reference coefficients previously hard-coded as raw
/// register bytes.
#[derive(Debug, Clone, Copy, serde::Serialize)]
pub struct Bme280Calib {
    pub dig_t1: u16,
    pub dig_t2: i16,
    pub dig_t3: i16,
    pub dig_p1: u16,
    pub dig_p2: i16,
    pub dig_p3: i16,
    pub dig_p4: i16,
    pub dig_p5: i16,
    pub dig_p6: i16,
    pub dig_p7: i16,
    pub dig_p8: i16,
    pub dig_p9: i16,
    pub dig_h1: u8,
    pub dig_h2: i16,
    pub dig_h3: u8,
    pub dig_h4: i16,
    pub dig_h5: i16,
    pub dig_h6: i8,
}

impl Default for Bme280Calib {
    fn default() -> Self {
        Self {
            dig_t1: 28528,
            dig_t2: 26435,
            dig_t3: -1000,
            dig_p1: 38221,
            dig_p2: -10577,
            dig_p3: 3024,
            dig_p4: 7421,
            dig_p5: -185,
            dig_p6: -7,
            dig_p7: 9900,
            dig_p8: -10230,
            dig_p9: 4285,
            dig_h1: 75,
            dig_h2: 350,
            dig_h3: 0,
            dig_h4: 309,
            dig_h5: 0,
            dig_h6: 30,
        }
    }
}

/// Widest raw ADC words the part can report: 20-bit for T/P, 16-bit for H.
const ADC_TP_MAX: i32 = 0x000F_FFFF;
const ADC_H_MAX: i32 = 0x0000_FFFF;

impl Bme280Calib {
    // ─── Bosch reference compensation (forward) ────────────────────────────
    //
    // Transcribed from the BME280 datasheet's fixed-point reference code
    // (`BME280_compensate_T_int32` / `_P_int64` / `_H_int32`). This is the
    // exact arithmetic a real driver runs on the host side; the inverse
    // functions below invert *these*, so a driver reading this model gets the
    // engineering value that was set.

    /// `t_fine`, the shared temperature fine-resolution intermediate.
    pub fn t_fine(&self, adc_t: i32) -> i32 {
        let dig_t1 = self.dig_t1 as i32;
        let dig_t2 = self.dig_t2 as i32;
        let dig_t3 = self.dig_t3 as i32;
        let var1 = (((adc_t >> 3) - (dig_t1 << 1)) * dig_t2) >> 11;
        let var2 = (((((adc_t >> 4) - dig_t1) * ((adc_t >> 4) - dig_t1)) >> 12) * dig_t3) >> 14;
        var1 + var2
    }

    /// Compensated temperature in hundredths of °C (Bosch `T` output).
    pub fn compensate_t(&self, adc_t: i32) -> i32 {
        (self.t_fine(adc_t) * 5 + 128) >> 8
    }

    /// Compensated pressure in Q24.8 Pa (Bosch `_P_int64` output; Pa = ret/256).
    pub fn compensate_p(&self, adc_p: i32, t_fine: i32) -> u32 {
        let mut var1: i64 = t_fine as i64 - 128_000;
        let mut var2: i64 = var1 * var1 * self.dig_p6 as i64;
        var2 += (var1 * self.dig_p5 as i64) << 17;
        var2 += (self.dig_p4 as i64) << 35;
        var1 = ((var1 * var1 * self.dig_p3 as i64) >> 8) + ((var1 * self.dig_p2 as i64) << 12);
        var1 = (((1i64 << 47) + var1) * self.dig_p1 as i64) >> 33;
        if var1 == 0 {
            return 0; // avoid division by zero, exactly as the reference does
        }
        let mut p: i64 = 1_048_576 - adc_p as i64;
        p = (((p << 31) - var2) * 3125) / var1;
        var1 = ((self.dig_p9 as i64) * (p >> 13) * (p >> 13)) >> 25;
        var2 = ((self.dig_p8 as i64) * p) >> 19;
        p = ((p + var1 + var2) >> 8) + ((self.dig_p7 as i64) << 4);
        p.clamp(0, u32::MAX as i64) as u32
    }

    /// Compensated humidity in Q22.10 %RH (Bosch `_H_int32` output; %RH = ret/1024).
    pub fn compensate_h(&self, adc_h: i32, t_fine: i32) -> u32 {
        let mut v: i32 = t_fine - 76_800;
        v = ((((adc_h << 14) - ((self.dig_h4 as i32) << 20) - ((self.dig_h5 as i32) * v))
            + 16_384)
            >> 15)
            * (((((((v * (self.dig_h6 as i32)) >> 10)
                * (((v * (self.dig_h3 as i32)) >> 11) + 32_768))
                >> 10)
                + 2_097_152)
                * (self.dig_h2 as i32)
                + 8_192)
                >> 14);
        v -= ((((v >> 15) * (v >> 15)) >> 7) * (self.dig_h1 as i32)) >> 4;
        v = v.clamp(0, 419_430_400);
        (v >> 12) as u32
    }

    // ─── Inverse compensation (engineering unit → raw ADC word) ────────────
    //
    // The compensation functions are monotone in their raw argument over the
    // part's operating range, so each inverse is a binary search over the
    // *forward* function above. That is deliberate: there is no second,
    // approximate transcription of the polynomials to drift out of sync — the
    // inverse is defined in terms of the exact code a driver will run.

    /// Raw 20-bit `adc_T` that compensates to `temp_c` degrees Celsius.
    pub fn invert_t(&self, temp_c: f64) -> i32 {
        let target = (temp_c * 100.0).round() as i64;
        invert_monotone(0, ADC_TP_MAX, target, |adc| self.compensate_t(adc) as i64)
    }

    /// Raw 16-bit `adc_H` that compensates to `humidity_pct` %RH at `t_fine`.
    pub fn invert_h(&self, humidity_pct: f64, t_fine: i32) -> i32 {
        let target = (humidity_pct * 1024.0).round() as i64;
        invert_monotone(0, ADC_H_MAX, target, |adc| {
            self.compensate_h(adc, t_fine) as i64
        })
    }

    /// Raw 20-bit `adc_P` that compensates to `pressure_hpa` at `t_fine`.
    ///
    /// Pressure is monotonically *decreasing* in `adc_P` (the reference code
    /// starts from `1048576 - adc_P`), so the search runs on the negated
    /// output to keep one monotone-increasing helper.
    pub fn invert_p(&self, pressure_hpa: f64, t_fine: i32) -> i32 {
        // hPa → Q24.8 Pa
        let target = (pressure_hpa * 100.0 * 256.0).round() as i64;
        invert_monotone(0, ADC_TP_MAX, -target, |adc| {
            -(self.compensate_p(adc, t_fine) as i64)
        })
    }
}

/// Smallest `x` in `[lo, hi]` whose `f(x)` is nearest `target`, for a
/// monotonically non-decreasing `f`. Plain bisection plus a neighbour compare
/// so the result is the closest attainable raw code, not merely a bracket.
fn invert_monotone(lo: i32, hi: i32, target: i64, f: impl Fn(i32) -> i64) -> i32 {
    let (mut lo, mut hi) = (lo, hi);
    while lo < hi {
        let mid = lo + (hi - lo) / 2;
        if f(mid) < target {
            lo = mid + 1;
        } else {
            hi = mid;
        }
    }
    // `lo` is the first code reaching `target`; its predecessor may be nearer.
    if lo > 0 {
        let here = (f(lo) - target).abs();
        let prev = (f(lo - 1) - target).abs();
        if prev <= here {
            return lo - 1;
        }
    }
    lo
}

/// BME280 Environmental Sensor I²C Component.
///
/// Serves factory calibration coefficients plus RAW ADC words, exactly like the
/// silicon — the firmware-side driver runs Bosch's compensation formulas over
/// them to recover engineering units. Stimulus ([`crate::sim_input::SimInput`])
/// takes °C / %RH / hPa and *inverts* that same compensation against the same
/// coefficients to produce the raw words, so a real driver reading this part
/// through the real compensation path lands on the value that was set.
#[derive(Debug, serde::Serialize)]
pub struct Bme280 {
    address: u8,
    current_register: u8,
    register_address_written: bool,

    /// Factory calibration; also the source of the calib register bytes.
    calib: Bme280Calib,

    /// Requested engineering values (the stimulus-facing state of record).
    temperature_c: f64,
    humidity_pct: f64,
    pressure_hpa: f64,

    /// Raw ADC words derived from the above by inverse compensation.
    adc_t: i32,
    adc_p: i32,
    adc_h: i32,

    // Writable control registers
    pub ctrl_hum: u8,
    pub ctrl_meas: u8,
    pub config: u8,

    /// system.yaml `external_devices` id, stamped at attach (see
    /// [`crate::sim_input::SimInput::component_id`]).
    component_id: Option<String>,
}

impl Default for Bme280 {
    fn default() -> Self {
        Self::new(0x76) // Default I²C address for BME280
    }
}

impl Bme280 {
    pub fn new(address: u8) -> Self {
        let mut dev = Self {
            address,
            current_register: 0,
            register_address_written: false,
            calib: Bme280Calib::default(),
            // Standard-atmosphere defaults; the same ≈25 °C / 50 %RH /
            // 1013.25 hPa the part used to report, now actually derived.
            temperature_c: 25.0,
            humidity_pct: 50.0,
            pressure_hpa: 1013.25,
            adc_t: 0,
            adc_p: 0,
            adc_h: 0,
            ctrl_hum: 0,
            ctrl_meas: 0,
            config: 0,
            component_id: None,
        };
        dev.resync_raw();
        dev
    }

    pub fn calib(&self) -> Bme280Calib {
        self.calib
    }

    /// Current engineering values (°C, %RH, hPa) as last requested.
    pub fn readings(&self) -> (f64, f64, f64) {
        (self.temperature_c, self.humidity_pct, self.pressure_hpa)
    }

    /// Raw ADC words the part currently reports (`adc_T`, `adc_P`, `adc_H`).
    pub fn raw(&self) -> (i32, i32, i32) {
        (self.adc_t, self.adc_p, self.adc_h)
    }

    /// Recompute every raw ADC word from the requested engineering values.
    ///
    /// Order matters: pressure and humidity compensation both consume `t_fine`,
    /// which is a function of `adc_T`, so temperature is inverted first and the
    /// resulting `t_fine` feeds the other two. That mirrors the driver-side
    /// requirement to read temperature before pressure/humidity.
    fn resync_raw(&mut self) {
        self.adc_t = self.calib.invert_t(self.temperature_c);
        let t_fine = self.calib.t_fine(self.adc_t);
        self.adc_p = self.calib.invert_p(self.pressure_hpa, t_fine);
        self.adc_h = self.calib.invert_h(self.humidity_pct, t_fine);
    }

    fn read_register(&self, reg: u8) -> u8 {
        // Little-endian 16-bit calib words, exactly as the silicon lays them
        // out: T1..T3 at 0x88, P1..P9 at 0x8E, each low byte first.
        let t_words: [u16; 3] = [
            self.calib.dig_t1,
            self.calib.dig_t2 as u16,
            self.calib.dig_t3 as u16,
        ];
        let p_words: [u16; 9] = [
            self.calib.dig_p1,
            self.calib.dig_p2 as u16,
            self.calib.dig_p3 as u16,
            self.calib.dig_p4 as u16,
            self.calib.dig_p5 as u16,
            self.calib.dig_p6 as u16,
            self.calib.dig_p7 as u16,
            self.calib.dig_p8 as u16,
            self.calib.dig_p9 as u16,
        ];
        // Little-endian byte `n` of `word`.
        let byte_of = |word: u16, n: u8| {
            if n == 0 {
                word as u8
            } else {
                (word >> 8) as u8
            }
        };

        match reg {
            // Calibration coefficients (T): 0x88..0x8D
            0x88..=0x8D => byte_of(t_words[((reg - 0x88) / 2) as usize], (reg - 0x88) % 2),

            // Calibration (P): 0x8E..0x9F
            0x8E..=0x9F => byte_of(p_words[((reg - 0x8E) / 2) as usize], (reg - 0x8E) % 2),

            // Chip ID
            0xD0 => 0x60, // BME280

            // Calibration (H)
            0xA1 => self.calib.dig_h1,
            0xE1 => byte_of(self.calib.dig_h2 as u16, 0),
            0xE2 => byte_of(self.calib.dig_h2 as u16, 1),
            0xE3 => self.calib.dig_h3,
            // dig_H4 = [E4]<<4 | [E5]&0x0F ; dig_H5 = [E6]<<4 | [E5]>>4
            0xE4 => ((self.calib.dig_h4 >> 4) & 0xFF) as u8,
            0xE5 => (((self.calib.dig_h5 & 0x0F) << 4) as u8) | ((self.calib.dig_h4 & 0x0F) as u8),
            0xE6 => ((self.calib.dig_h5 >> 4) & 0xFF) as u8,
            0xE7 => self.calib.dig_h6 as u8,

            // Status: never measuring, data always available
            0xF3 => 0x00,

            // Control registers return what was last written
            0xF2 => self.ctrl_hum,
            0xF4 => self.ctrl_meas,
            0xF5 => self.config,

            // Raw measurement registers (msb, lsb, xlsb) — 20-bit P and T are
            // left-aligned across three bytes; H is a plain 16-bit word.
            0xF7 => ((self.adc_p >> 12) & 0xFF) as u8,
            0xF8 => ((self.adc_p >> 4) & 0xFF) as u8,
            0xF9 => ((self.adc_p << 4) & 0xF0) as u8,
            0xFA => ((self.adc_t >> 12) & 0xFF) as u8,
            0xFB => ((self.adc_t >> 4) & 0xFF) as u8,
            0xFC => ((self.adc_t << 4) & 0xF0) as u8,
            0xFD => ((self.adc_h >> 8) & 0xFF) as u8,
            0xFE => (self.adc_h & 0xFF) as u8,

            _ => 0,
        }
    }

    fn write_register(&mut self, reg: u8, value: u8) {
        match reg {
            0xF2 => self.ctrl_hum = value,
            0xF4 => self.ctrl_meas = value,
            0xF5 => self.config = value,
            0xE0 => { /* soft reset 0xB6 — ignore */ }
            _ => {}
        }
    }
}

impl I2cDevice for Bme280 {
    fn address(&self) -> u8 {
        self.address
    }

    fn read(&mut self) -> u8 {
        let val = self.read_register(self.current_register);
        self.current_register = self.current_register.wrapping_add(1);
        val
    }

    fn write(&mut self, data: u8) {
        if !self.register_address_written {
            self.current_register = data;
            self.register_address_written = true;
        } else {
            self.write_register(self.current_register, data);
            self.current_register = self.current_register.wrapping_add(1);
        }
    }

    fn stop(&mut self) {
        self.register_address_written = false;
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

/// Drivable environmental channels. Ranges are the BME280 datasheet's
/// operating envelope. One table backs BOTH the `SimInput` impl and the kit
/// metadata, so the device schema and the runtime API cannot drift.
pub const INPUT_CHANNELS: &[crate::sim_input::InputChannel] = &[
    crate::sim_input::InputChannel {
        key: "temperature",
        label: "Temperature",
        unit: "°C",
        min: -40.0,
        max: 85.0,
    },
    crate::sim_input::InputChannel {
        key: "humidity",
        label: "Humidity",
        unit: "%RH",
        min: 0.0,
        max: 100.0,
    },
    crate::sim_input::InputChannel {
        key: "pressure",
        label: "Pressure",
        unit: "hPa",
        min: 300.0,
        max: 1100.0,
    },
];

impl crate::sim_input::SimInput for Bme280 {
    fn input_channels(&self) -> &'static [crate::sim_input::InputChannel] {
        INPUT_CHANNELS
    }

    fn set_input(&mut self, key: &str, value: f64) -> Result<(), crate::sim_input::SimInputError> {
        self.require_channel(key, value)?;
        match key {
            "temperature" => self.temperature_c = value,
            "humidity" => self.humidity_pct = value,
            "pressure" => self.pressure_hpa = value,
            _ => unreachable!("require_channel validated the key"),
        }
        // Temperature moves `t_fine`, which the pressure and humidity
        // polynomials consume — so every channel re-derives all three raw
        // words rather than patching one in isolation.
        self.resync_raw();
        Ok(())
    }

    fn component_id(&self) -> Option<&str> {
        self.component_id.as_deref()
    }

    fn set_component_id(&mut self, id: String) {
        self.component_id = Some(id);
    }
}

// ─── PeripheralKit registration ────────────────────────────────────────────

use crate::peripherals::kit::{
    AttachCtx, Category, ConfigKey, ConfigType, KitMetadata, LabRef, PeripheralKit, Transport,
};

pub struct Bme280Kit;
pub static BME280_KIT: Bme280Kit = Bme280Kit;

static BME280_METADATA: KitMetadata = KitMetadata {
    inputs: INPUT_CHANNELS,
    device_type: "bme280",
    label: "BME280 Weather",
    summary: "Bosch BME280 temp + humidity + pressure sensor over I2C.",
    detail: "Serves factory calibration coefficients plus raw ADC words, exactly like the \
             silicon; the firmware driver recovers engineering units by running Bosch's \
             compensation formulas. Stimulus takes °C / %RH / hPa and inverts that same \
             compensation against the same coefficients, so a real driver reads back what \
             was set. Defaults to 25 °C / 50 %RH / 1013.25 hPa.",
    transport: Transport::I2c,
    category: Category::I2c,
    config_keys: &[ConfigKey {
        name: "i2c_address",
        ty: ConfigType::Int,
        doc: "7-bit slave address. Defaults to 0x76; 0x77 selects the SDO=VDDIO variant.",
    }],
    labs: &[LabRef {
        board_id: "bme280-weather-lab",
        chip: "stm32f103",
        example_dir: "bme280-weather-lab",
        demo_elf: "demo-bme280-weather-lab.elf",
    }],
};

impl PeripheralKit for Bme280Kit {
    fn metadata(&self) -> &'static KitMetadata {
        &BME280_METADATA
    }
    fn attach(&self, ctx: &mut AttachCtx<'_>) -> anyhow::Result<()> {
        let address = ctx.i2c_address_or(0x76)?;
        ctx.attach_i2c_device(Box::new(Bme280::new(address)))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::peripherals::i2c::I2cDevice;
    use crate::sim_input::SimInput;

    /// Read `len` bytes starting at `reg` through the real I²C register
    /// interface — the same path firmware takes.
    fn read_regs(dev: &mut Bme280, reg: u8, len: usize) -> Vec<u8> {
        dev.stop();
        dev.write(reg);
        let out = (0..len).map(|_| dev.read()).collect();
        dev.stop();
        out
    }

    /// An INDEPENDENT transcription of the Bosch BME280 fixed-point reference
    /// compensation, driven purely off bytes read out of the device over I²C.
    /// Deliberately does not call into `Bme280Calib` — this is what a real
    /// driver does, so it checks the register serialization AND the inversion,
    /// not just self-consistency of the model's own helpers.
    struct DriverSide {
        dig_t1: u16,
        dig_t2: i16,
        dig_t3: i16,
        dig_p1: u16,
        dig_p: [i16; 8],
        dig_h1: u8,
        dig_h2: i16,
        dig_h3: u8,
        dig_h4: i16,
        dig_h5: i16,
        dig_h6: i8,
    }

    impl DriverSide {
        fn slurp(dev: &mut Bme280) -> Self {
            let c = read_regs(dev, 0x88, 26); // 0x88..0xA1
            let h = read_regs(dev, 0xE1, 7); // 0xE1..0xE7
            let u16le = |b: &[u8], i: usize| u16::from_le_bytes([b[i], b[i + 1]]);
            let i16le = |b: &[u8], i: usize| i16::from_le_bytes([b[i], b[i + 1]]);
            Self {
                dig_t1: u16le(&c, 0),
                dig_t2: i16le(&c, 2),
                dig_t3: i16le(&c, 4),
                dig_p1: u16le(&c, 6),
                dig_p: [
                    i16le(&c, 8),
                    i16le(&c, 10),
                    i16le(&c, 12),
                    i16le(&c, 14),
                    i16le(&c, 16),
                    i16le(&c, 18),
                    i16le(&c, 20),
                    i16le(&c, 22),
                ],
                dig_h1: c[25], // 0xA1
                dig_h2: i16::from_le_bytes([h[0], h[1]]),
                dig_h3: h[2],
                // dig_H4 = [E4]<<4 | [E5]&0x0F ; dig_H5 = [E6]<<4 | [E5]>>4
                dig_h4: ((h[3] as i8 as i16) << 4) | ((h[4] & 0x0F) as i16),
                dig_h5: ((h[5] as i8 as i16) << 4) | ((h[4] >> 4) as i16),
                dig_h6: h[6] as i8,
            }
        }

        fn t_fine(&self, adc_t: i32) -> i32 {
            let t1 = self.dig_t1 as i32;
            let var1 = (((adc_t >> 3) - (t1 << 1)) * self.dig_t2 as i32) >> 11;
            let var2 =
                (((((adc_t >> 4) - t1) * ((adc_t >> 4) - t1)) >> 12) * self.dig_t3 as i32) >> 14;
            var1 + var2
        }

        /// °C
        fn temperature(&self, adc_t: i32) -> f64 {
            ((self.t_fine(adc_t) * 5 + 128) >> 8) as f64 / 100.0
        }

        /// hPa
        fn pressure(&self, adc_p: i32, t_fine: i32) -> f64 {
            let mut var1: i64 = t_fine as i64 - 128_000;
            let mut var2: i64 = var1 * var1 * self.dig_p[4] as i64; // dig_P6
            var2 += (var1 * self.dig_p[3] as i64) << 17; // dig_P5
            var2 += (self.dig_p[2] as i64) << 35; // dig_P4
            var1 = ((var1 * var1 * self.dig_p[1] as i64) >> 8) // dig_P3
                + ((var1 * self.dig_p[0] as i64) << 12); // dig_P2
            var1 = (((1i64 << 47) + var1) * self.dig_p1 as i64) >> 33;
            assert!(var1 != 0, "reference code divide-by-zero guard");
            let mut p: i64 = 1_048_576 - adc_p as i64;
            p = (((p << 31) - var2) * 3125) / var1;
            var1 = ((self.dig_p[7] as i64) * (p >> 13) * (p >> 13)) >> 25; // dig_P9
            var2 = ((self.dig_p[6] as i64) * p) >> 19; // dig_P8
            p = ((p + var1 + var2) >> 8) + ((self.dig_p[5] as i64) << 4); // dig_P7
            (p as f64 / 256.0) / 100.0 // Q24.8 Pa → hPa
        }

        /// %RH
        fn humidity(&self, adc_h: i32, t_fine: i32) -> f64 {
            let mut v: i32 = t_fine - 76_800;
            v = ((((adc_h << 14) - ((self.dig_h4 as i32) << 20) - ((self.dig_h5 as i32) * v))
                + 16_384)
                >> 15)
                * (((((((v * (self.dig_h6 as i32)) >> 10)
                    * (((v * (self.dig_h3 as i32)) >> 11) + 32_768))
                    >> 10)
                    + 2_097_152)
                    * (self.dig_h2 as i32)
                    + 8_192)
                    >> 14);
            v -= ((((v >> 15) * (v >> 15)) >> 7) * (self.dig_h1 as i32)) >> 4;
            v = v.clamp(0, 419_430_400);
            (v >> 12) as f64 / 1024.0
        }
    }

    /// Read the raw ADC words out of the measurement registers the way a
    /// driver does: one 8-byte burst from 0xF7.
    fn read_raw_via_i2c(dev: &mut Bme280) -> (i32, i32, i32) {
        let d = read_regs(dev, 0xF7, 8);
        let adc_p = ((d[0] as i32) << 12) | ((d[1] as i32) << 4) | ((d[2] as i32) >> 4);
        let adc_t = ((d[3] as i32) << 12) | ((d[4] as i32) << 4) | ((d[5] as i32) >> 4);
        let adc_h = ((d[6] as i32) << 8) | (d[7] as i32);
        (adc_t, adc_p, adc_h)
    }

    #[test]
    fn chip_id_is_bme280() {
        let mut dev = Bme280::new(0x76);
        assert_eq!(read_regs(&mut dev, 0xD0, 1)[0], 0x60);
    }

    /// The calib bytes the device serves must decode back to the coefficients
    /// it computes with — otherwise the driver and the model disagree.
    #[test]
    fn calib_registers_round_trip_to_coefficients() {
        let mut dev = Bme280::new(0x76);
        let want = dev.calib();
        let got = DriverSide::slurp(&mut dev);
        assert_eq!(got.dig_t1, want.dig_t1);
        assert_eq!(got.dig_t2, want.dig_t2);
        assert_eq!(got.dig_t3, want.dig_t3);
        assert_eq!(got.dig_p1, want.dig_p1);
        assert_eq!(got.dig_p[0], want.dig_p2);
        assert_eq!(got.dig_p[1], want.dig_p3);
        assert_eq!(got.dig_p[2], want.dig_p4);
        assert_eq!(got.dig_p[3], want.dig_p5);
        assert_eq!(got.dig_p[4], want.dig_p6);
        assert_eq!(got.dig_p[5], want.dig_p7);
        assert_eq!(got.dig_p[6], want.dig_p8);
        assert_eq!(got.dig_p[7], want.dig_p9);
        assert_eq!(got.dig_h1, want.dig_h1);
        assert_eq!(got.dig_h2, want.dig_h2);
        assert_eq!(got.dig_h3, want.dig_h3);
        assert_eq!(got.dig_h4, want.dig_h4);
        assert_eq!(got.dig_h5, want.dig_h5);
        assert_eq!(got.dig_h6, want.dig_h6);
    }

    /// THE round-trip that matters: drive an engineering value in, then run
    /// the real Bosch compensation over the device's reported raw + calib and
    /// recover it. This is exactly what firmware sees.
    #[test]
    fn set_temperature_recovers_through_bosch_compensation() {
        for target in [-40.0, -10.0, 0.0, 21.5, 25.0, 37.0, 60.0, 85.0] {
            let mut dev = Bme280::new(0x76);
            dev.set_input("temperature", target).unwrap();
            let drv = DriverSide::slurp(&mut dev);
            let (adc_t, _, _) = read_raw_via_i2c(&mut dev);
            let got = drv.temperature(adc_t);
            assert!(
                (got - target).abs() <= 0.01,
                "temperature {target} °C → adc_T {adc_t} → compensated {got} °C"
            );
        }
    }

    #[test]
    fn set_humidity_recovers_through_bosch_compensation() {
        for target in [0.0, 12.5, 30.0, 50.0, 78.0, 100.0] {
            let mut dev = Bme280::new(0x76);
            dev.set_input("humidity", target).unwrap();
            let drv = DriverSide::slurp(&mut dev);
            let (adc_t, _, adc_h) = read_raw_via_i2c(&mut dev);
            let t_fine = drv.t_fine(adc_t);
            let got = drv.humidity(adc_h, t_fine);
            assert!(
                (got - target).abs() <= 0.2,
                "humidity {target} %RH → adc_H {adc_h} → compensated {got} %RH"
            );
        }
    }

    #[test]
    fn set_pressure_recovers_through_bosch_compensation() {
        for target in [300.0, 700.0, 950.0, 1013.25, 1100.0] {
            let mut dev = Bme280::new(0x76);
            dev.set_input("pressure", target).unwrap();
            let drv = DriverSide::slurp(&mut dev);
            let (adc_t, adc_p, _) = read_raw_via_i2c(&mut dev);
            let t_fine = drv.t_fine(adc_t);
            let got = drv.pressure(adc_p, t_fine);
            assert!(
                (got - target).abs() <= 0.1,
                "pressure {target} hPa → adc_P {adc_p} → compensated {got} hPa"
            );
        }
    }

    /// Temperature shifts `t_fine`, so P and H must be re-derived with it —
    /// otherwise driving temperature silently corrupts the other two readings.
    #[test]
    fn temperature_change_preserves_humidity_and_pressure() {
        let mut dev = Bme280::new(0x76);
        dev.set_input("humidity", 65.0).unwrap();
        dev.set_input("pressure", 880.0).unwrap();
        dev.set_input("temperature", 70.0).unwrap();

        let drv = DriverSide::slurp(&mut dev);
        let (adc_t, adc_p, adc_h) = read_raw_via_i2c(&mut dev);
        let t_fine = drv.t_fine(adc_t);
        assert!((drv.temperature(adc_t) - 70.0).abs() <= 0.01);
        assert!(
            (drv.humidity(adc_h, t_fine) - 65.0).abs() <= 0.2,
            "humidity drifted to {}",
            drv.humidity(adc_h, t_fine)
        );
        assert!(
            (drv.pressure(adc_p, t_fine) - 880.0).abs() <= 0.1,
            "pressure drifted to {}",
            drv.pressure(adc_p, t_fine)
        );
    }

    /// Defaults must themselves be honest: the untouched part reads back as
    /// standard atmosphere through the compensation path.
    #[test]
    fn defaults_compensate_to_standard_atmosphere() {
        let mut dev = Bme280::default();
        let drv = DriverSide::slurp(&mut dev);
        let (adc_t, adc_p, adc_h) = read_raw_via_i2c(&mut dev);
        let t_fine = drv.t_fine(adc_t);
        assert!((drv.temperature(adc_t) - 25.0).abs() <= 0.01);
        assert!((drv.humidity(adc_h, t_fine) - 50.0).abs() <= 0.2);
        assert!((drv.pressure(adc_p, t_fine) - 1013.25).abs() <= 0.1);
    }

    #[test]
    fn out_of_range_is_rejected() {
        let mut dev = Bme280::new(0x76);
        assert!(dev.set_input("temperature", 200.0).is_err());
        assert!(dev.set_input("humidity", -1.0).is_err());
        assert!(dev.set_input("pressure", 5000.0).is_err());
        assert!(dev.set_input("altitude", 1.0).is_err());
    }

    #[test]
    fn control_registers_still_read_back() {
        let mut dev = Bme280::new(0x76);
        dev.stop();
        dev.write(0xF4);
        dev.write(0x27);
        dev.stop();
        assert_eq!(read_regs(&mut dev, 0xF4, 1)[0], 0x27);
    }
}
