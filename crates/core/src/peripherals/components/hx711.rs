// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! HX711 24-bit load-cell ADC (Avia Semiconductor).
//!
//! Bit-banged two-wire protocol (DT data, SCK clock):
//! * DT low = conversion ready
//! * MCU clocks SCK 24 times; each rising edge shifts out one data bit (MSB first)
//! * 1–3 extra SCK pulses select gain for the next conversion
//!
//! Host sets `weight` (grams) or raw counts via SimInput. The pure protocol
//! state machine is unit-tested here; bus attach wires SCK observe + DT drive
//! like the TM1637/HC-SR04 GPIO models.

use crate::sim_input::SimInput;

/// 24-bit two's complement raw value corresponding to `weight` grams at a
/// fixed scale (1 gram ≈ 100 counts) for demo firmware.
fn grams_to_raw(grams: f64) -> i32 {
    (grams * 100.0).round().clamp(-(1 << 23) as f64, ((1 << 23) - 1) as f64) as i32
}

pub struct Hx711 {
    component_id: Option<String>,
    /// Live sample as 24-bit two's complement stored in the low 24 bits.
    raw24: u32,
    /// Bit index into the current 24-bit frame (0 = MSB pending).
    bit_index: u8,
    /// Extra pulses after bit 24 (gain select); ignored for value.
    extra_clocks: u8,
    /// Current DT output level (true = high / not ready or bit high).
    dt_high: bool,
    prev_sck: bool,
    /// True while a conversion frame is being shifted out.
    shifting: bool,
    // Bus wiring (resolved at attach)
    pub sck_odr_addr: u64,
    pub sck_bit: u8,
    pub dt_idr_addr: u64,
    pub dt_bit: u8,
    sck_peripheral_idx: Option<usize>,
    dt_peripheral_idx: Option<usize>,
    last_dt_high: Option<bool>,
}

impl Hx711 {
    pub fn new(sck_odr_addr: u64, sck_bit: u8, dt_idr_addr: u64, dt_bit: u8) -> Self {
        Self {
            component_id: None,
            raw24: 0,
            bit_index: 0,
            extra_clocks: 0,
            dt_high: false, // ready
            prev_sck: false,
            shifting: false,
            sck_odr_addr,
            sck_bit,
            dt_idr_addr,
            dt_bit,
            sck_peripheral_idx: None,
            dt_peripheral_idx: None,
            last_dt_high: None,
        }
    }

    pub fn set_raw(&mut self, raw: i32) {
        self.raw24 = (raw as u32) & 0x00FF_FFFF;
        if !self.shifting {
            self.dt_high = false; // data ready
        }
    }

    pub fn set_weight_g(&mut self, grams: f64) {
        self.set_raw(grams_to_raw(grams));
    }

    pub fn raw(&self) -> i32 {
        let mut v = self.raw24 as i32;
        if self.raw24 & 0x0080_0000 != 0 {
            v |= !0x00FF_FFFF_i32;
        }
        v
    }

    pub fn dt_high(&self) -> bool {
        self.dt_high
    }

    pub fn sck_peripheral_idx(&self) -> Option<usize> {
        self.sck_peripheral_idx
    }
    pub fn set_sck_peripheral_idx(&mut self, idx: usize) {
        self.sck_peripheral_idx = Some(idx);
    }
    pub fn dt_peripheral_idx(&self) -> Option<usize> {
        self.dt_peripheral_idx
    }
    pub fn set_dt_peripheral_idx(&mut self, idx: usize) {
        self.dt_peripheral_idx = Some(idx);
    }
    pub fn last_dt_high(&self) -> Option<bool> {
        self.last_dt_high
    }
    pub fn set_last_dt_high(&mut self, v: bool) {
        self.last_dt_high = Some(v);
    }

    /// Observe SCK level from the MCU. Rising edges clock out data bits.
    pub fn observe_sck(&mut self, sck: bool) {
        let rising = sck && !self.prev_sck;
        self.prev_sck = sck;
        if !rising {
            return;
        }

        if !self.shifting {
            // First clock after ready starts the frame.
            if self.dt_high {
                return; // not ready — ignore clocks (busy)
            }
            self.shifting = true;
            self.bit_index = 0;
            self.extra_clocks = 0;
        }

        if self.bit_index < 24 {
            // MSB first: bit 23 down to 0
            let shift = 23 - self.bit_index;
            let bit = ((self.raw24 >> shift) & 1) != 0;
            self.dt_high = bit;
            self.bit_index += 1;
        } else {
            // Gain pulses: hold DT high (end of data)
            self.dt_high = true;
            self.extra_clocks += 1;
            if self.extra_clocks >= 1 {
                // End of conversion; next sample ready again
                self.shifting = false;
                self.bit_index = 0;
                self.extra_clocks = 0;
                self.dt_high = false;
            }
        }
    }
}

impl crate::sim_input::SimInput for Hx711 {
    fn input_channels(&self) -> &'static [crate::sim_input::InputChannel] {
        INPUT_CHANNELS
    }

    fn set_input(&mut self, key: &str, value: f64) -> Result<(), crate::sim_input::SimInputError> {
        self.require_channel(key, value)?;
        match key {
            "weight" => self.set_weight_g(value),
            "raw" => self.set_raw(value.round() as i32),
            _ => unreachable!(),
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

pub const INPUT_CHANNELS: &[crate::sim_input::InputChannel] = &[
    crate::sim_input::InputChannel {
        key: "weight",
        label: "Weight",
        unit: "g",
        min: -50_000.0,
        max: 50_000.0,
    },
    crate::sim_input::InputChannel {
        key: "raw",
        label: "Raw counts",
        unit: "counts",
        min: -8_388_608.0,
        max: 8_388_607.0,
    },
];

use crate::peripherals::kit::{
    AttachCtx, Category, ConfigKey, ConfigType, KitMetadata, PeripheralKit, Transport,
};

pub struct Hx711Kit;
pub static HX711_KIT: Hx711Kit = Hx711Kit;

static HX711_METADATA: KitMetadata = KitMetadata {
    inputs: INPUT_CHANNELS,
    device_type: "hx711",
    label: "HX711 Load Cell",
    summary: "24-bit load-cell amplifier (DT/SCK bit-bang).",
    detail: "Avia HX711. Firmware bit-bangs SCK while reading DT; host sets weight (g) or raw \
             counts via SimInput. Gain/channel selection pulses after bit 24 are accepted.",
    transport: Transport::GpioGroup,
    category: Category::Gpio,
    config_keys: &[
        ConfigKey {
            name: "sck_pin",
            ty: ConfigType::Str,
            doc: "SCK GPIO pin (e.g. \"PA8\"). Defaults to PA8.",
        },
        ConfigKey {
            name: "dt_pin",
            ty: ConfigType::Str,
            doc: "DT (DOUT) GPIO pin (e.g. \"PA9\"). Defaults to PA9.",
        },
    ],
    labs: &[],
};

impl PeripheralKit for Hx711Kit {
    fn metadata(&self) -> &'static KitMetadata {
        &HX711_METADATA
    }
    fn attach(&self, ctx: &mut AttachCtx<'_>) -> anyhow::Result<()> {
        let sck = ctx.config_str("sck_pin").unwrap_or("PA8").to_string();
        let dt = ctx.config_str("dt_pin").unwrap_or("PA9").to_string();
        let (sck_addr, sck_bit) = ctx.resolve_pin_odr(&sck).ok_or_else(|| {
            anyhow::anyhow!(
                "HX711 '{}' sck_pin '{}' could not be resolved to a GPIO",
                ctx.device_id(),
                sck
            )
        })?;
        // DT is an MCU *input* — resolve IDR for driving ready/data bits.
        let (dt_addr, dt_bit) = crate::bus::SystemBus::resolve_pin_idr_pub(ctx.bus, &dt)
            .or_else(|| ctx.resolve_pin_odr(&dt))
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "HX711 '{}' dt_pin '{}' could not be resolved to a GPIO",
                    ctx.device_id(),
                    dt
                )
            })?;
        let mut dev = Hx711::new(sck_addr, sck_bit, dt_addr, dt_bit);
        dev.set_component_id(ctx.device_id().to_string());
        ctx.bus.hx711.push(dev);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clocks_out_24_bit_sample_msb_first() {
        let mut dev = Hx711::new(0, 0, 0, 0);
        dev.set_raw(0x00AB_CDEF & 0x00FF_FFFF); // 24-bit
        assert!(!dev.dt_high()); // ready
        let mut bits = 0u32;
        for i in 0..24 {
            dev.observe_sck(false);
            dev.observe_sck(true);
            bits = (bits << 1) | if dev.dt_high() { 1 } else { 0 };
            let _ = i;
        }
        assert_eq!(bits, 0x00AB_CDEF & 0x00FF_FFFF);
        // one extra clock ends frame
        dev.observe_sck(false);
        dev.observe_sck(true);
        assert!(!dev.dt_high()); // ready again
    }

    #[test]
    fn weight_sim_input_scales() {
        let mut dev = Hx711::new(0, 0, 0, 0);
        dev.set_weight_g(10.0);
        assert_eq!(dev.raw(), 1000);
    }
}
