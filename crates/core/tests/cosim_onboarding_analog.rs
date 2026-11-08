// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Generic MNA devices route through every imported onboarding profile
//! (`configs/systems/onboarding/*.yaml`) whose chip descriptor declares an ADC
//! LabWired can convert, into EVERY such converter instance.
//!
//! Nothing here is per-board. The converter family is read from each
//! descriptor (peripheral type plus `config.profile`), the instance base from
//! its `base_address`, and the same two netlists — a Beta NTC divider and an
//! unrelated Shockley diode load line — are routed to `adc.<id>.<ch>_volts`
//! and read back through the register sequence firmware uses for that family.
//! A new onboarding descriptor whose ADC is convertible is covered the moment
//! it lands; one whose ADC changes family moves between the lists below and
//! fails here until the coverage doc is updated.

mod common;
use common::root;
use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::bus::SystemBus;
use labwired_core::cosim::{CosimSession, CosimSignalValue, RoutingError};
use labwired_core::Bus;
use std::path::{Path, PathBuf};

/// Channels used on every converter: low enough to exist on all four
/// families (nRF52 SAADC has AIN0..7), clear of channel 0.
const NTC_CHANNEL: u8 = 5;
const DIODE_CHANNEL: u8 = 6;
const IDLE_CHANNEL: u8 = 7;

/// Onboarding profiles whose ADC is converted end to end here.
const CONVERTED: &[&str] = &[
    "ambiq-apollo4",
    "arduino_nano_33_ble",
    "atsamd21j17d-aft",
    "brd4116a",
    "brd4117a",
    "brd4118a",
    "brd4120a",
    "brd4121a",
    "brd4186c",
    "brd4402a",
    "cc2538",
    "efr32mg24",
    "efr32mg26",
    "efr32xg22",
    "eos-s3-qomu",
    "eos-s3-quickfeather",
    "eos-s3",
    "imxrt1064",
    "mars_zx3",
    "max32652-evkit",
    "max32652",
    "mimxrt1064_evk",
    "nrf52840",
    "nrf52840dk_nrf52840",
    "nucleo_f767zi",
    "nucleo_h753zi",
    "nucleo_wba52cg",
    "ramn",
    "renesas-da14592",
    "sam4s",
    "sam4s16c",
    "sam4s8b",
    "sam4s_xplained",
    "stm32f042",
    "stm32f072",
    "stm32f072b_discovery",
    "stm32f4",
    "stm32f401cdu6-blackpill",
    "stm32f412",
    "stm32f429",
    "stm32f4_discovery-kit",
    "stm32f4_discovery",
    "stm32f746",
    "stm32f777",
    "stm32f7_discovery-bb",
    "stm32h7",
    "stm32h743",
    "stm32h747-disco",
    "stm32h747",
    "stm32h753",
    "stm32l071",
    "stm32l072",
    "stm32wba52",
    "ut32m0r500",
    "zedboard",
    "zolertia-firefly",
    "zynq-7000",
];

/// Onboarding profiles that declare an ADC whose IP LabWired does not yet
/// convert from a routed input. Empty: the last two, the STM32WBA52 ADC4
/// boards, now convert on the `stm32wba` profile. A descriptor that lands
/// with an unconvertible ADC fails here until it is converted or listed.
const DECLARED_NOT_CONVERTIBLE: &[&str] = &[];

fn ntc_resistance_ohms(temperature_c: f64) -> f64 {
    10_000.0 * (3950.0 * (1.0 / (temperature_c + 273.15) - 1.0 / 298.15)).exp()
}

/// Independent Shockley load line, including the diode's series resistance.
fn diode_load_line_voltage() -> f64 {
    const SUPPLY_V: f64 = 3.3;
    const LIMIT_OHMS: f64 = 1_000.0;
    const DIODE_IS_A: f64 = 2.52e-9;
    const DIODE_N: f64 = 1.752;
    const DIODE_RS_OHMS: f64 = 0.568;
    const THERMAL_V: f64 = 1.380_649e-23 * 298.15 / 1.602_176_634e-19;
    let current_at_junction_voltage =
        |junction_v: f64| DIODE_IS_A * ((junction_v / (DIODE_N * THERMAL_V)).exp() - 1.0);
    let residual = |junction_v: f64| {
        let current = current_at_junction_voltage(junction_v);
        (SUPPLY_V - junction_v - current * DIODE_RS_OHMS) / LIMIT_OHMS - current
    };
    let (mut low, mut high) = (0.0, 1.2);
    for _ in 0..100 {
        let mid = (low + high) / 2.0;
        if residual(mid) > 0.0 {
            low = mid;
        } else {
            high = mid;
        }
    }
    let junction_v = (low + high) / 2.0;
    junction_v + current_at_junction_voltage(junction_v) * DIODE_RS_OHMS
}

fn signal_voltage(session: &CosimSession, path: &str) -> f64 {
    match session.signals().get(path) {
        Some(CosimSignalValue::F64(volts)) => *volts,
        other => panic!("expected solved voltage at {path}, got {other:?}"),
    }
}

fn millivolts(volts: f64) -> u32 {
    (volts * 1000.0).round() as u32
}

/// The converter families an onboarding descriptor can name.
#[derive(Debug, Clone, Copy)]
enum Family {
    /// Shared `adc` model, F2/F4/F7 map (RM0090/RM0410).
    Stm32F4,
    /// Shared `adc` model, H7 map (RM0433).
    Stm32H7,
    /// Shared `adc` model, L4 map, which the L5 keeps register for register
    /// (RM0351 §18.6, RM0438 §21.6): `SQR1.SQ1` @0x30, `DR` @0x40.
    Stm32L4,
    /// Shared `adc` model, F0/L0 v2 map with the CHSELR bitmap (RM0091 §13,
    /// RM0367 §14).
    Stm32V2,
    /// Shared `adc` model, STM32WBA ADC4 (RM0493 §21): the CHSELR block with
    /// an LDO (`CR.ADVREGEN` → `ISR.LDORDY`) and `CFGR1.RES` at [3:2].
    Stm32Adc4,
    /// i.MX RT 12-bit SAR ADC (IMXRT1060RM §66).
    Imxrt,
    /// nRF52840 SAADC with EasyDMA (nRF52840 PS §6.23).
    Saadc { ram: u64 },
    /// SAM4S 12-bit ADC: `CHER` channel set, `CR.START`, `CDRx`/`LCDR`
    /// (SAM4S datasheet DS11100 §42).
    Sam4s,
    /// Ambiq Apollo4 ADC: SLOTnCFG slots, software trigger `SWT` = 0x37,
    /// slot-tagged results in the FIFO with six fractional bits, against the
    /// 1.19 V internal reference, so the 3.3 V netlists saturate above it.
    Apollo4,
    /// CC2538 SoC ADC: an `ADCCON3` write runs one extra conversion, EOC in
    /// `ADCCON1` clears on the `ADCH` read, and the 2's-complement result
    /// sits MSB-aligned in `ADCH:ADCL` (SWRU319C §15, Contiki-NG adc.c).
    Cc2538,
    /// QuickLogic EOS S3 ADC: two channels only, `CONTROL.SEL` picks one,
    /// `CONTROL.SOC` starts it, `STATUS.EOC` drops then rises, 12 bits in
    /// `OUT` against 1.4 V (qorc-sdk eoss3_hal_adc.c, Renode EOSS3_ADC.cs).
    Eoss3,
    /// MAX32650/MAX32652 10-bit ADC: `CTRL.CH_SEL` picks AIN0..AIN3 or
    /// AIN0/AIN1 through the divide-by-5, `CTRL.START` converts against the
    /// 1.22 V bandgap, `INTR.DONE_IF` is write 1 to clear (msdk adc_reva.c,
    /// Renode MAX32650_ADC.cs).
    Max32650,
    /// Renesas DA1459x GPADC: `GP_ADC_SEL.SEL_P` picks the mux input,
    /// `GP_ADC_START` converts and self-clears, 10 bits MSB-aligned in the
    /// 16-bit `RESULT` against 0.9 V x (`GP_ADC_ATTN` + 1) (Renode
    /// RenesasDA14_GPADC.cs, SDK10 hw_gpadc_adc_measure/get_value).
    Da14Gpadc,
    /// CAES UT32M0R500 delta-sigma ADC: one `ADC_TRIGGER` sweeps every
    /// enabled `SECHAN_CFG`, 12 bits against 1.5 V / PGA gain, bit 11
    /// flipped in `DATA_OUT` (Renode CAES_ADC.cs, CAES ADC app notes).
    Caes,
    /// Zynq-7000 XADC through the PS-XADC interface: DRP read commands into
    /// `CMDFIFO`, results one command later in `RDFIFO`, 12 bits
    /// MSB-justified against 1 V on VP/VN and VAUXn (Renode Xilinx_XADC.cs,
    /// embeddedsw xadcps.c), so the 3.3 V NTC divider saturates above 1 V.
    Xadc,
    /// SAM D21 ADC: `INPUTCTRL.MUXPOS` against GND, `SWTRIG.START`,
    /// `INTFLAG.RESRDY` cleared by the `RESULT` read, and a pad converts
    /// only once PORT hands it to the ADC (PINCFG.PMUXEN, PMUX function B)
    /// (DS40001882 §33, ASF adc_start_conversion/adc_read).
    Samd21,
    /// EFR32 Series-2 IADC (xG22/MG24/MG26): `SINGLE.PORTPOS`/`PINPOS` mux a
    /// pad, `CMD.SINGLESTART` converts only while `EN.EN` is set,
    /// `STATUS.SINGLEFIFODV` and `IF.SINGLEDONE` rise, `SINGLEFIFODATA` pops
    /// 12 bits against AVDD (simplicity_sdk efr32mg2x_iadc.h, em_iadc.c).
    Efr32Iadc,
}

/// `SINGLE.PORTPOS` value naming GPIO port A (`_IADC_SINGLE_PORTPOS_PORTA`).
const IADC_PORTPOS_PORTA: u32 = 0x8;

/// SAM D21 PORT GROUP0 (PA). The routed AIN5..7 are pads PA05..PA07.
const SAMD21_PORT_A: u64 = 0x4100_4400;

/// XADC DRP channel of VAUXn (xadcps_hw.h `XADCPS_CH_AUX_MIN`).
const XADC_AUX0: u8 = 0x10;

struct Converter {
    id: String,
    base: u64,
    family: Family,
}

/// Every non-stub ADC the descriptor declares, with the family it converts
/// through. `Err` names an ADC this test cannot convert and why.
fn converters(chip: &ChipDescriptor) -> Result<Vec<Converter>, String> {
    let mut found = Vec::new();
    for p in &chip.peripherals {
        let kind = p.r#type.to_ascii_lowercase();
        if kind == "stub" || !p.id.to_ascii_lowercase().contains("adc") {
            continue;
        }
        let family = if kind == "imxrt_adc" {
            Family::Imxrt
        } else if kind.contains("saadc") {
            Family::Saadc { ram: chip.ram.base }
        } else if kind == "sam4s_adc" {
            Family::Sam4s
        } else if kind == "apollo4_adc" {
            Family::Apollo4
        } else if kind == "cc2538_soc_adc" {
            Family::Cc2538
        } else if kind == "eoss3_adc" {
            Family::Eoss3
        } else if kind == "max32650_adc" {
            Family::Max32650
        } else if kind == "renesasda14_gpadc" {
            Family::Da14Gpadc
        } else if kind == "caes_adc" {
            Family::Caes
        } else if kind == "samd21_adc" {
            Family::Samd21
        } else if kind == "xilinx_xadc" {
            Family::Xadc
        } else if kind == "efr32s2_iadc" {
            Family::Efr32Iadc
        } else {
            match p.config.get("profile").and_then(|v| v.as_str()) {
                Some("stm32f4") => Family::Stm32F4,
                Some("stm32h7") => Family::Stm32H7,
                Some("stm32l4") => Family::Stm32L4,
                Some("stm32f0" | "stm32l0") => Family::Stm32V2,
                Some("stm32wba") => Family::Stm32Adc4,
                other => {
                    return Err(format!(
                        "{}:{} ({kind}, profile {other:?})",
                        chip.name, p.id
                    ))
                }
            }
        };
        found.push(Converter {
            id: p.id.clone(),
            base: p.base_address,
            family,
        });
    }
    Ok(found)
}

impl Converter {
    fn expected_code(&self, volts: f64) -> u32 {
        let mv = millivolts(volts);
        match self.family {
            // 12-bit, mV * 4095 / VREF (3.3 V) truncated.
            Family::Stm32F4
            | Family::Stm32H7
            | Family::Stm32L4
            | Family::Stm32V2
            | Family::Stm32Adc4
            | Family::Sam4s => (mv * 4095 / 3300).min(4095),
            // 12-bit, rounded to nearest.
            Family::Imxrt => (mv.min(3300) * 4095 + 1650) / 3300,
            // Gain 1/4 against VDD/4: full scale is VDD, 12-bit, 2^12 / VDD.
            Family::Saadc { .. } => (mv * 4096 / 3300).min(4095),
            // 12-bit against VREF 1.19 V, truncated, clamped to full scale.
            Family::Apollo4 => (mv * 4095 / 1190).min(4095),
            // Decimation 512 against AVDD5 (3.3 V): 12 bits including the
            // sign, so single-ended full scale is 2047.
            Family::Cc2538 => (mv * 2047 / 3300).min(2047),
            // 12-bit against 1.4 V, truncated, clamped (Renode ToADCRawValue).
            Family::Eoss3 => mv.min(1400) * 4095 / 1400,
            // 10-bit against 1.22 V, truncated, clamped (Renode ToADCRawValue).
            Family::Max32650 => mv.min(1220) * 1023 / 1220,
            // 10-bit with the attenuator at 3.6 V full scale, truncated.
            Family::Da14Gpadc => mv.min(3600) * 1023 / 3600,
            // 12-bit against 1.5 V at PGA 1 V/V, truncated, clamped.
            Family::Caes => mv.min(1500) * 4095 / 1500,
            // Unipolar, 1 LSB = 1 V / 4096, clamped at 0xFFF.
            Family::Xadc => (mv * 4096 / 1000).min(4095),
            // 12-bit against REFCTRL.REFSEL INTVCC0 = VDDANA / 1.48
            // (2229 mV at 3.3 V), truncated, clamped.
            Family::Samd21 => (mv * 4095 / (3300 * 100 / 148)).min(4095),
            // 12-bit against AVDD (3.3 V), 2^12 / VREF, truncated, clamped.
            Family::Efr32Iadc => (mv * 4096 / 3300).min(4095),
        }
    }

    /// The NTC code. The MAX32650 converts the NTC pad through
    /// `AIN0_DIV5`, so the 3.3 V divider stays inside 5 x 1.22 V.
    fn expected_ntc_code(&self, volts: f64) -> u32 {
        match self.family {
            Family::Max32650 => millivolts(volts).min(6100) * 1023 / 6100,
            // PGA 0.5 V/V doubles the 1.5 V range to 3 V.
            Family::Caes => millivolts(volts).min(3000) * 4095 / 3000,
            _ => self.expected_code(volts),
        }
    }

    /// The channels the NTC and diode netlists route to. The EOS S3 ADC has
    /// only channels 0 and 1, so it takes both; every other family has
    /// AIN0..7 and uses NTC_CHANNEL/DIODE_CHANNEL, clear of channel 0.
    fn routed_channels(&self) -> [u8; 2] {
        match self.family {
            // AIN0 and AIN1: the two pads with a divide-by-5 channel.
            // DA1459x: SEL_P 0/1 are pads P1_0/P1_1.
            Family::Eoss3 | Family::Max32650 | Family::Da14Gpadc => [0, 1],
            // VAUX5 and VAUX6: the DRP channel is 0x10 + n.
            Family::Xadc => [XADC_AUX0 + NTC_CHANNEL, XADC_AUX0 + DIODE_CHANNEL],
            _ => [NTC_CHANNEL, DIODE_CHANNEL],
        }
    }

    /// Bring the converter up the way firmware does, before any conversion.
    fn power_up(&self, bus: &mut SystemBus) {
        match self.family {
            Family::Stm32F4 | Family::Sam4s => {}
            Family::Efr32Iadc => {
                // A SINGLESTART before EN.EN is accepted and dropped: no
                // result, no flag, which is how a missing IADC_init hangs on
                // the bench. Then IADC_init's EN = 1.
                bus.write_u32(self.base + 0x98, (IADC_PORTPOS_PORTA << 12) | (5 << 8))
                    .expect("SINGLE PA05");
                bus.write_u32(self.base + 0x0C, 1).expect("SINGLESTART");
                bus.tick_peripherals_fully();
                assert_eq!(
                    bus.read_u32(self.base + 0x14).unwrap() & (1 << 8),
                    0,
                    "{}: a disabled IADC converts nothing",
                    self.id
                );
                assert_eq!(
                    bus.read_u32(self.base + 0x24).unwrap(),
                    0,
                    "{}: IF",
                    self.id
                );
                bus.write_u32(self.base + 0x04, 1).expect("EN.EN");
            }
            Family::Xadc => {
                // xadcps.c XAdcPs_CfgInitialize: CFG |= ENABLE | CFIFOTH |
                // DFIFOTH, then MCTL = 0 to release the XADC from reset.
                // Until then a command is dropped.
                const CFG_ENABLE_FIFOTH: u32 = (1 << 31) | (0xF << 20) | (0xF << 16);
                let (cfg, msts, cmdfifo, mctl) = (
                    self.base,
                    self.base + 0x0C,
                    self.base + 0x10,
                    self.base + 0x18,
                );
                assert_eq!(
                    bus.read_u32(mctl).unwrap(),
                    0x10,
                    "{}: MCTL.RESET at reset",
                    self.id
                );
                bus.write_u32(cmdfifo, 0).expect("no-op before release");
                assert_eq!(
                    bus.read_u32(msts).unwrap(),
                    0x500,
                    "{}: no command runs before bring-up (DFIFOE | CFIFOE)",
                    self.id
                );
                let value = bus.read_u32(cfg).unwrap();
                assert_eq!(value, 0x1114, "{}: CFG reset value", self.id);
                bus.write_u32(cfg, value | CFG_ENABLE_FIFOTH)
                    .expect("CFG.ENABLE");
                bus.write_u32(mctl, 0).expect("release MCTL.RESET");
            }
            Family::Samd21 => {
                // ASF port_pin_set_config + adc_init: hand PA05..PA07 to
                // function B (PMUX[2] odd nibble, PMUX[3] both nibbles,
                // PINCFG.PMUXEN), then 12-bit right-adjusted, INTVCC0, and
                // CTRLA.ENABLE.
                bus.write_u8(SAMD21_PORT_A + 0x32, 0x10)
                    .expect("PMUX[2] PA05 = B");
                bus.write_u8(SAMD21_PORT_A + 0x33, 0x11)
                    .expect("PMUX[3] PA06/PA07 = B");
                for pad in [NTC_CHANNEL, DIODE_CHANNEL, IDLE_CHANNEL] {
                    bus.write_u8(SAMD21_PORT_A + 0x40 + u64::from(pad), 1)
                        .expect("PINCFG.PMUXEN");
                }
                bus.write_u16(self.base + 0x04, 0).expect("CTRLB 12-bit");
                bus.write_u8(self.base + 0x01, 1).expect("REFCTRL INTVCC0");
                bus.write_u8(self.base, 1 << 1).expect("CTRLA.ENABLE");
            }
            Family::Caes => {
                // Users Guide Example 1: ADC_REGDEF, then single sweep with
                // ADC_EN, plus EN_PGA for the per-channel gains.
                const REGDEF: u32 = 1 << 31;
                const ADC_EN: u32 = 1;
                const EN_PGA: u32 = 1 << 1;
                const SINGLESWEEP: u32 = 1 << 13;
                bus.write_u32(self.base + 0x04, REGDEF).expect("ADC_REGDEF");
                assert_eq!(
                    bus.read_u32(self.base + 0x10).unwrap(),
                    0x10,
                    "{}: SECHAN_CFG resets to GAIN 1 V/V, disabled",
                    self.id
                );
                bus.write_u32(self.base, SINGLESWEEP | EN_PGA | ADC_EN)
                    .expect("SPB_CFG_0");
            }
            Family::Da14Gpadc => {
                // hw_gpadc_configure: single-ended, GP_ADC_ATTN = 3 (up to
                // 3.6 V), CONV_NRS = 4 (16 samples, the SDK minimum for
                // the die sensor), then GP_ADC_EN.
                const SE: u32 = 1 << 6;
                let ctrl = self.base;
                let ctrl2 = self.base + 0x04;
                bus.write_u32(ctrl, SE).expect("GP_ADC_SE");
                let value = bus.read_u32(ctrl2).unwrap() & !(3 | (7 << 6));
                bus.write_u32(ctrl2, value | 3 | (4 << 6))
                    .expect("GP_ADC_ATTN/CONV_NRS");
                bus.write_u32(ctrl, SE | 1).expect("GP_ADC_EN");
            }
            Family::Max32650 => {
                // msdk MXC_ADC_RevA_Init: clear REF_READY_IF, CTRL.PWR, then
                // CTRL.REFBUF_PWR, spin on REF_READY_IF, clear it.
                const REF_READY_IF: u32 = 1 << 17;
                let ctrl = self.base;
                let intr = self.base + 0x0C;
                bus.write_u32(intr, REF_READY_IF)
                    .expect("clear REF_READY_IF");
                bus.write_u32(ctrl, 1 << 1).expect("CTRL.PWR");
                bus.write_u32(ctrl, (1 << 1) | (1 << 3))
                    .expect("CTRL.REFBUF_PWR");
                assert_ne!(
                    bus.read_u32(intr).unwrap() & REF_READY_IF,
                    0,
                    "{}: REFBUF_PWR raises INTR.REF_READY_IF",
                    self.id
                );
                bus.write_u32(intr, REF_READY_IF)
                    .expect("clear REF_READY_IF");
            }
            Family::Eoss3 => {
                // HAL_ADC_Init(): CONTROL = 0, then SEL/MEAS_EN without SOC.
                bus.write_u32(self.base + 0x08, 0).expect("CONTROL reset");
                assert_eq!(
                    bus.read_u32(self.base + 0x04).unwrap(),
                    1,
                    "{}: idle STATUS.EOC reads 1",
                    self.id
                );
            }
            Family::Cc2538 => {
                // adc_init(): ADCCON1.STSEL = 11, manual starts only.
                bus.write_u32(self.base, 0x30).expect("ADCCON1.STSEL");
            }
            Family::Apollo4 => {
                // CFG.ADCEN with destructive FIFO reads (DFIFORDEN).
                bus.write_u32(self.base, 1 | (1 << 12))
                    .expect("enable Apollo4 ADC");
            }
            Family::Stm32H7 => {
                let cr = self.base + 0x08;
                bus.write_u32(cr, 0).expect("leave H7 ADC deep power-down");
                bus.write_u32(cr, 1 << 28).expect("enable ADC regulator");
                bus.write_u32(cr, (1 << 28) | (1 << 31))
                    .expect("calibrate ADC");
                bus.write_u32(cr, (1 << 28) | 1).expect("enable ADC");
                assert_ne!(
                    bus.read_u32(self.base).unwrap() & 1,
                    0,
                    "{}: H7 ADC reports ADRDY",
                    self.id
                );
                bus.write_u32(self.base + 0x0C, 2 << 2)
                    .expect("select 12-bit H7 resolution");
                bus.write_u32(self.base + 0x1C, (1 << NTC_CHANNEL) | (1 << DIODE_CHANNEL))
                    .expect("preselect routed channels");
            }
            Family::Stm32L4 => {
                let cr = self.base + 0x08;
                assert_eq!(
                    bus.read_u32(cr).unwrap(),
                    1 << 29,
                    "{}: L4 CR resets in deep power-down",
                    self.id
                );
                bus.write_u32(cr, 0).expect("leave L4 ADC deep power-down");
                bus.write_u32(cr, 1 << 28).expect("enable ADC regulator");
                bus.write_u32(cr, (1 << 28) | 1).expect("enable ADC");
                assert_ne!(
                    bus.read_u32(self.base).unwrap() & 1,
                    0,
                    "{}: L4 ADC reports ADRDY",
                    self.id
                );
                // CFGR RES[4:3] = 00 selects 12 bits, right-aligned.
                bus.write_u32(self.base + 0x0C, 0).expect("select 12-bit");
            }
            Family::Stm32V2 => {
                const ADEN: u32 = 1;
                const ADCAL: u32 = 1 << 31;
                const EOCAL: u32 = 1 << 11;
                let cr = self.base + 0x08;
                assert_eq!(
                    bus.read_u32(cr).unwrap(),
                    0,
                    "{}: v2 CR resets to 0, no DEEPPWD",
                    self.id
                );
                bus.write_u32(cr, ADCAL).expect("calibrate ADC");
                assert_eq!(bus.read_u32(cr).unwrap() & ADCAL, 0, "ADCAL self-clears");
                assert_ne!(
                    bus.read_u32(self.base).unwrap() & EOCAL,
                    0,
                    "{}: calibration raises EOCAL",
                    self.id
                );
                bus.write_u32(self.base, EOCAL).expect("clear EOCAL");
                bus.write_u32(cr, ADEN).expect("enable ADC");
                assert_ne!(
                    bus.read_u32(self.base).unwrap() & 1,
                    0,
                    "{}: ADEN raises ADRDY",
                    self.id
                );
                // CFGR1 RES[4:3] = 00 selects 12 bits, right-aligned.
                bus.write_u32(self.base + 0x0C, 0).expect("select 12-bit");
            }
            Family::Stm32Adc4 => {
                const ADEN: u32 = 1;
                const ADVREGEN: u32 = 1 << 28;
                const ADCAL: u32 = 1 << 31;
                const LDORDY: u32 = 1 << 12;
                const EOCAL: u32 = 1 << 11;
                let cr = self.base + 0x08;
                assert_eq!(
                    bus.read_u32(cr).unwrap(),
                    0,
                    "{}: ADC4 CR resets to 0",
                    self.id
                );
                bus.write_u32(cr, ADEN).expect("ADEN before the LDO");
                assert_eq!(
                    bus.read_u32(self.base).unwrap() & 1,
                    0,
                    "{}: ADC4 stays not-ready until ADVREGEN",
                    self.id
                );
                bus.write_u32(cr, ADVREGEN).expect("enable ADC4 LDO");
                assert_ne!(
                    bus.read_u32(self.base).unwrap() & LDORDY,
                    0,
                    "{}: ADVREGEN raises LDORDY",
                    self.id
                );
                bus.write_u32(cr, ADVREGEN | ADCAL).expect("calibrate ADC4");
                assert_ne!(
                    bus.read_u32(self.base).unwrap() & EOCAL,
                    0,
                    "{}: calibration raises EOCAL",
                    self.id
                );
                bus.write_u32(self.base, EOCAL | LDORDY)
                    .expect("clear EOCAL/LDORDY");
                bus.write_u32(cr, ADVREGEN | ADEN).expect("enable ADC4");
                assert_ne!(
                    bus.read_u32(self.base).unwrap() & 1,
                    0,
                    "{}: ADEN raises ADRDY",
                    self.id
                );
                // CFGR1 RES[3:2] = 00 selects 12 bits, right-aligned.
                bus.write_u32(self.base + 0x0C, 0).expect("select 12-bit");
            }
            Family::Imxrt => {
                let cfg = bus.read_u32(self.base + 0x44).unwrap();
                bus.write_u32(self.base + 0x44, (cfg & !(3 << 2)) | (2 << 2))
                    .expect("select 12-bit i.MX RT ADC mode");
            }
            Family::Saadc { ram } => {
                for (slot, channel) in [(0u64, NTC_CHANNEL), (1, DIODE_CHANNEL), (2, IDLE_CHANNEL)]
                {
                    let ch = self.base + 0x510 + slot * 0x10;
                    bus.write_u32(ch, u32::from(channel) + 1)
                        .expect("select SAADC PSELP = AIN<channel>");
                    bus.write_u32(ch + 8, 0x1200)
                        .expect("select gain 1/4 and VDD/4 reference");
                }
                bus.write_u32(self.base + 0x5f0, 2)
                    .expect("select 12-bit SAADC resolution");
                bus.write_u32(self.base + 0x62c, ram as u32)
                    .expect("set EasyDMA result pointer");
                bus.write_u32(self.base + 0x630, 3)
                    .expect("reserve three results");
                bus.write_u32(self.base + 0x500, 1).expect("enable SAADC");
            }
        }
    }

    /// One firmware-style conversion per routed channel: `[ntc, diode]`.
    fn convert_routed(&self, bus: &mut SystemBus) -> [u32; 2] {
        match self.family {
            Family::Xadc => {
                // Three pipelined DRP reads and a no-op: each command shifts
                // the previous result into RDFIFO, so the first word is the
                // no-op latch and the reads follow in order.
                let (msts, cmdfifo, rdfifo) =
                    (self.base + 0x0C, self.base + 0x10, self.base + 0x14);
                let read = |channel: u8| (1 << 26) | (u32::from(channel) << 16);
                let [ntc, diode] = self.routed_channels();
                for channel in [ntc, diode, XADC_AUX0 + IDLE_CHANNEL] {
                    bus.write_u32(cmdfifo, read(channel)).expect("DRP read");
                }
                bus.write_u32(cmdfifo, 0).expect("DRP no-op");
                assert_eq!(
                    bus.read_u32(msts).unwrap() >> 12 & 0xF,
                    4,
                    "{}: four commands leave four RDFIFO words",
                    self.id
                );
                let words: Vec<u32> = (0..4).map(|_| bus.read_u32(rdfifo).unwrap()).collect();
                assert_eq!(
                    words[0], 0,
                    "{}: the first word is the no-op latch",
                    self.id
                );
                assert_eq!(
                    words[3], 0,
                    "{}: unrouted VAUX{IDLE_CHANNEL} converts to zero",
                    self.id
                );
                for word in &words {
                    assert_eq!(word & 0xF, 0, "{}: 12 bits MSB-justified", self.id);
                }
                // The single-read path (XAdcPs_GetAdcData) agrees.
                assert_eq!(
                    self.convert(bus, ntc),
                    words[1] >> 4,
                    "{}: GetAdcData",
                    self.id
                );
                [words[1] >> 4, words[2] >> 4]
            }
            Family::Saadc { ram } => {
                bus.write_u32(self.base + 0x104, 0).expect("clear END");
                bus.write_u32(self.base, 1).expect("TASKS_START");
                bus.write_u32(self.base + 4, 1).expect("TASKS_SAMPLE");
                bus.tick_peripherals_fully();
                assert_eq!(bus.read_u32(self.base + 0x104).unwrap(), 1, "scan sets END");
                assert_eq!(bus.read_u32(self.base + 0x634).unwrap(), 3, "AMOUNT");
                assert_eq!(
                    bus.read_u16(ram + 4).unwrap(),
                    0,
                    "{}: unrouted AIN{IDLE_CHANNEL} converts to zero",
                    self.id
                );
                [
                    u32::from(bus.read_u16(ram).unwrap()),
                    u32::from(bus.read_u16(ram + 2).unwrap()),
                ]
            }
            Family::Sam4s => {
                // One START converts the whole enabled set in channel
                // order; EMR.TAG stamps LCDR with the last channel.
                const EMR_TAG: u32 = 1 << 24;
                const DRDY: u32 = 1 << 24;
                let set = (1 << NTC_CHANNEL) | (1 << DIODE_CHANNEL) | (1 << IDLE_CHANNEL);
                bus.write_u32(self.base + 0x40, EMR_TAG).expect("EMR.TAG");
                bus.write_u32(self.base + 0x10, set).expect("CHER");
                bus.write_u32(self.base, 1 << 1).expect("CR.START");
                assert_eq!(
                    bus.read_u32(self.base + 0x30).unwrap() & (set | DRDY),
                    set | DRDY,
                    "{}: START raises EOC for every enabled channel and DRDY",
                    self.id
                );
                assert_eq!(
                    bus.read_u32(self.base + 0x20).unwrap(),
                    u32::from(IDLE_CHANNEL) << 12,
                    "{}: LCDR tags the last channel, unrouted AD{IDLE_CHANNEL}, with code 0",
                    self.id
                );
                let codes = [
                    bus.read_u32(self.base + 0x50 + 4 * u64::from(NTC_CHANNEL))
                        .unwrap(),
                    bus.read_u32(self.base + 0x50 + 4 * u64::from(DIODE_CHANNEL))
                        .unwrap(),
                ];
                assert_eq!(
                    bus.read_u32(self.base + 0x30).unwrap() & (set | DRDY),
                    1 << IDLE_CHANNEL,
                    "{}: CDR reads clear their EOC, the LCDR read cleared DRDY",
                    self.id
                );
                bus.write_u32(self.base + 0x14, set).expect("CHDR");
                bus.read_u32(self.base + 0x50 + 4 * u64::from(IDLE_CHANNEL))
                    .expect("drain the idle CDR");
                codes
            }
            Family::Eoss3 => self
                .routed_channels()
                .map(|channel| self.convert(bus, channel)),
            Family::Max32650 => {
                // CH_SEL 4 is AIN0_DIV5; AIN2 is unrouted and converts to 0.
                const AIN0_DIV5: u8 = 4;
                assert_eq!(
                    self.convert(bus, 2),
                    0,
                    "{}: unrouted AIN2 converts to zero",
                    self.id
                );
                [self.convert(bus, AIN0_DIV5), self.convert(bus, 1)]
            }
            Family::Caes => {
                // One sweep of three SECHAN_CFG channels: the NTC through
                // PGA 0.5 V/V (GAIN 000), the diode and the unrouted AIN7 at
                // 1 V/V (GAIN 001). Poll CONV_COMPL_COMB plus the highest
                // enabled channel's pending bit (errata 8.2), then read
                // DATA_OUT and flip bit 11 back to offset binary (Code 4).
                const EN: u32 = 1;
                let chan_cfg = |channel: u8| self.base + 0x10 + 4 * u64::from(channel);
                let data = |channel: u8| self.base + 0x90 + 4 * u64::from(channel);
                bus.write_u32(chan_cfg(NTC_CHANNEL), EN)
                    .expect("NTC SECHAN_CFG");
                bus.write_u32(chan_cfg(DIODE_CHANNEL), (1 << 4) | EN)
                    .expect("diode SECHAN_CFG");
                bus.write_u32(chan_cfg(IDLE_CHANNEL), (1 << 4) | EN)
                    .expect("idle SECHAN_CFG");
                bus.write_u32(self.base + 0x04, 1).expect("ADC_TRIGGER");
                let done = (1 << 31) | (1 << IDLE_CHANNEL);
                assert_eq!(
                    bus.read_u32(self.base + 0x8C).unwrap() & done,
                    done,
                    "{}: the sweep sets CONV_COMPL_COMB and the last channel's pending bit",
                    self.id
                );
                assert_eq!(
                    bus.read_u32(self.base + 0x8C).unwrap(),
                    0,
                    "{}: INT_STATUS clears on read",
                    self.id
                );
                let code = |bus: &mut SystemBus, channel: u8| {
                    (bus.read_u32(data(channel)).expect("DATA") & 0xFFF) ^ 0x800
                };
                assert_eq!(
                    code(bus, IDLE_CHANNEL),
                    0,
                    "{}: unrouted AIN7 converts to zero",
                    self.id
                );
                let codes = [code(bus, NTC_CHANNEL), code(bus, DIODE_CHANNEL)];
                for channel in [NTC_CHANNEL, DIODE_CHANNEL, IDLE_CHANNEL] {
                    bus.write_u32(chan_cfg(channel), 0x10).expect("disable");
                }
                codes
            }
            Family::Samd21 => {
                let codes = [
                    self.convert(bus, NTC_CHANNEL),
                    self.convert(bus, DIODE_CHANNEL),
                ];
                // The ADC reads the pad only while PORT hands it over:
                // with PINCFG.PMUXEN cleared the driven NTC pad converts to
                // zero, and handing it back restores the code.
                let pincfg = SAMD21_PORT_A + 0x40 + u64::from(NTC_CHANNEL);
                bus.write_u8(pincfg, 0).expect("PA05 back to GPIO");
                assert_eq!(
                    self.convert(bus, NTC_CHANNEL),
                    0,
                    "{}: a pad PORT has not handed to the ADC carries no signal",
                    self.id
                );
                bus.write_u8(pincfg, 1).expect("PA05 to the ADC");
                assert_eq!(self.convert(bus, NTC_CHANNEL), codes[0], "{}", self.id);
                codes
            }
            Family::Da14Gpadc => {
                // SEL_P 2 is pad P1_2, unrouted, so it converts to 0.
                assert_eq!(
                    self.convert(bus, 2),
                    0,
                    "{}: unrouted P1_2 converts to zero",
                    self.id
                );
                self.routed_channels()
                    .map(|channel| self.convert(bus, channel))
            }
            Family::Cc2538 => {
                // One extra conversion per channel; EOC rises at the ADCCON3
                // write and the ADCH read clears it before the next.
                [NTC_CHANNEL, DIODE_CHANNEL].map(|channel| {
                    let code = self.convert(bus, channel);
                    assert_eq!(
                        bus.read_u32(self.base).unwrap(),
                        0x30,
                        "{}: EOC clear after the ADCH read, STSEL kept",
                        self.id
                    );
                    code
                })
            }
            Family::Apollo4 => {
                // Three slots, one SWT: each slot pushes one FIFO entry
                // tagged with its slot number, in slot order.
                for (slot, channel) in [(0u64, NTC_CHANNEL), (1, DIODE_CHANNEL), (2, IDLE_CHANNEL)]
                {
                    bus.write_u32(self.base + 0x0C + 4 * slot, 1 | (u32::from(channel) << 8))
                        .expect("enable 12-bit slot");
                }
                bus.write_u32(self.base + 0x08, 0x37).expect("SWT");
                assert_eq!(
                    bus.read_u32(self.base + 0x204).unwrap() & 3,
                    3,
                    "{}: the scan raises CNVCMP and SCNCMP",
                    self.id
                );
                bus.write_u32(self.base + 0x208, 3).expect("INTCLR");
                let mut codes = [0; 3];
                for (slot, code) in codes.iter_mut().enumerate() {
                    let word = bus.read_u32(self.base + 0x3C).expect("pop FIFOPR");
                    assert_eq!(
                        (word >> 28) & 7,
                        slot as u32,
                        "{}: FIFO entry {slot} carries its slot number",
                        self.id
                    );
                    assert_eq!(
                        (word >> 20) & 0xFF,
                        3 - slot as u32,
                        "{}: FIFO count before pop {slot}",
                        self.id
                    );
                    *code = (word & 0xF_FFFF) >> 6;
                }
                assert_eq!(
                    codes[2], 0,
                    "{}: unrouted SE{IDLE_CHANNEL} converts to zero",
                    self.id
                );
                for slot in 0..3u64 {
                    bus.write_u32(self.base + 0x0C + 4 * slot, 0)
                        .expect("disable slot");
                }
                [codes[0], codes[1]]
            }
            Family::Stm32V2 | Family::Stm32Adc4 => {
                let single = [
                    self.convert(bus, NTC_CHANNEL),
                    self.convert(bus, DIODE_CHANNEL),
                ];
                let idle = self.convert(bus, IDLE_CHANNEL);
                let [ntc, diode] = single;
                for (cfgr1, chselr, order) in self.v2_scans() {
                    let expected: Vec<u32> = order
                        .iter()
                        .map(|&ch| match ch {
                            NTC_CHANNEL => ntc,
                            DIODE_CHANNEL => diode,
                            _ => idle,
                        })
                        .collect();
                    assert_eq!(
                        self.scan_v2(bus, cfgr1, chselr, order.len()),
                        expected,
                        "{}: one ADSTART scans channels {order:?} into DR in turn",
                        self.id
                    );
                }
                single
            }
            Family::Stm32F4 => {
                let single = [
                    self.convert(bus, NTC_CHANNEL),
                    self.convert(bus, DIODE_CHANNEL),
                ];
                let [ntc, diode] = single;
                for (order, expected) in [
                    ([NTC_CHANNEL, DIODE_CHANNEL], [ntc, diode]),
                    ([DIODE_CHANNEL, NTC_CHANNEL], [diode, ntc]),
                ] {
                    assert_eq!(
                        self.scan_f4(bus, &order),
                        expected,
                        "{}: one SWSTART scans CR1.SCAN group {order:?} into DR in turn",
                        self.id
                    );
                }
                // CR1.DISCEN over [NTC, diode, NTC]: one sensor per trigger,
                // then two, each subgroup stopping short at SQ3.
                let group = [NTC_CHANNEL, DIODE_CHANNEL, NTC_CHANNEL];
                assert_eq!(
                    self.discen_f4(bus, &group, 1, 4),
                    vec![vec![ntc], vec![diode], vec![ntc], vec![ntc]],
                    "{}: DISCNUM=0 converts one routed sensor per SWSTART",
                    self.id
                );
                assert_eq!(
                    self.discen_f4(bus, &group, 2, 3),
                    vec![vec![ntc, diode], vec![ntc], vec![ntc, diode]],
                    "{}: DISCNUM=1 converts the next two routed sensors per SWSTART",
                    self.id
                );
                // Injected group: one JSWSTART converts both routed sensors
                // into JDR1..JDRn in rank order.
                for (ranks, expected) in [
                    (vec![DIODE_CHANNEL], vec![diode]),
                    (
                        vec![NTC_CHANNEL, DIODE_CHANNEL, NTC_CHANNEL],
                        vec![ntc, diode, ntc],
                    ),
                ] {
                    assert_eq!(
                        self.injected_f4(bus, &ranks),
                        expected,
                        "{}: one JSWSTART converts injected ranks {ranks:?}",
                        self.id
                    );
                }
                single
            }
            Family::Stm32H7 | Family::Stm32L4 => {
                let single = [
                    self.convert(bus, NTC_CHANNEL),
                    self.convert(bus, DIODE_CHANNEL),
                ];
                let idle = self.convert(bus, IDLE_CHANNEL);
                let [ntc, diode] = single;
                for (sqr, order) in Self::sqr_scans() {
                    let expected: Vec<u32> = order
                        .iter()
                        .map(|&ch| match ch {
                            NTC_CHANNEL => ntc,
                            DIODE_CHANNEL => diode,
                            _ => idle,
                        })
                        .collect();
                    assert_eq!(
                        self.scan_sqr(bus, sqr, order.len()),
                        expected,
                        "{}: one ADSTART scans SQR sequence {order:?} into DR in turn",
                        self.id
                    );
                }
                // DISCEN, DISCNUM = 1: NTC, diode, idle, NTC, diode in
                // subsequences of two; the third stops at the sequence end.
                assert_eq!(
                    self.scan_sqr_discontinuous(bus, 1, 5),
                    vec![
                        vec![ntc, diode],
                        vec![idle, ntc],
                        vec![diode],
                        vec![ntc, diode]
                    ],
                    "{}: each ADSTART converts the next DISCNUM + 1 channels",
                    self.id
                );
                // Injected group: one JADSTART converts the routed sensors
                // into JDR1..JDRn in JSQ order, leaving DR alone.
                let dr = bus.read_u32(self.base + 0x40).expect("DR");
                for (ranks, expected) in [
                    (vec![DIODE_CHANNEL], vec![diode]),
                    (
                        vec![NTC_CHANNEL, DIODE_CHANNEL, IDLE_CHANNEL, NTC_CHANNEL],
                        vec![ntc, diode, idle, ntc],
                    ),
                ] {
                    assert_eq!(
                        self.injected_sqr(bus, &ranks),
                        expected,
                        "{}: one JADSTART converts injected ranks {ranks:?}",
                        self.id
                    );
                }
                assert_eq!(
                    bus.read_u32(self.base + 0x40).unwrap(),
                    dr,
                    "{}: the injected group leaves DR alone",
                    self.id
                );
                single
            }
            _ => [
                self.convert(bus, NTC_CHANNEL),
                self.convert(bus, DIODE_CHANNEL),
            ],
        }
    }

    /// `(CFGR1, CHSELR, channel order)` for the multi-channel scans each v2
    /// map runs. The bitmap scans upward, or downward with SCANDIR; ADC4's
    /// CHSELRMOD sequence runs in SQx order whatever the channel numbers.
    fn v2_scans(&self) -> Vec<(u32, u32, Vec<u8>)> {
        let bitmap = (1 << NTC_CHANNEL) | (1 << DIODE_CHANNEL) | (1 << IDLE_CHANNEL);
        let upward = vec![NTC_CHANNEL, DIODE_CHANNEL, IDLE_CHANNEL];
        let downward = vec![IDLE_CHANNEL, DIODE_CHANNEL, NTC_CHANNEL];
        match self.family {
            Family::Stm32V2 => vec![(0, bitmap, upward), (1 << 2, bitmap, downward)],
            Family::Stm32Adc4 => {
                // SQ1 = diode, SQ2 = NTC, SQ3 = 0xF ends the sequence.
                let sq = 0xFFFF_FF00 | (u32::from(NTC_CHANNEL) << 4) | u32::from(DIODE_CHANNEL);
                vec![
                    (0, bitmap, upward),
                    (1 << 4, bitmap, downward),
                    (1 << 21, sq, vec![DIODE_CHANNEL, NTC_CHANNEL]),
                ]
            }
            _ => Vec::new(),
        }
    }

    /// `([SQR1..SQR4], channel order)` for the multi-channel scans the L4 and
    /// H7 maps run. `SQR1.L` is the length minus one; SQ1..SQ4 follow it in
    /// SQR1, SQ5.. continue five to a word in SQR2. The order is the SQx order
    /// whatever the channel numbers, so the reversed pair converts the diode
    /// first, and the six-long one reaches into SQR2.
    fn sqr_scans() -> Vec<([u32; 4], Vec<u8>)> {
        let [ntc, diode, idle] = [NTC_CHANNEL, DIODE_CHANNEL, IDLE_CHANNEL].map(u32::from);
        vec![
            (
                [2 | (ntc << 6) | (diode << 12) | (idle << 18), 0, 0, 0],
                vec![NTC_CHANNEL, DIODE_CHANNEL, IDLE_CHANNEL],
            ),
            (
                [1 | (diode << 6) | (ntc << 12), 0, 0, 0],
                vec![DIODE_CHANNEL, NTC_CHANNEL],
            ),
            (
                [
                    5 | (idle << 6) | (idle << 12) | (idle << 18) | (idle << 24),
                    ntc | (diode << 6),
                    0,
                    0,
                ],
                vec![
                    IDLE_CHANNEL,
                    IDLE_CHANNEL,
                    IDLE_CHANNEL,
                    IDLE_CHANNEL,
                    NTC_CHANNEL,
                    DIODE_CHANNEL,
                ],
            ),
        ]
    }

    /// One software-started L4/H7 `SQRx` scan read by polling. Leaves SQR1..4
    /// at a one-channel sequence.
    fn scan_sqr(&self, bus: &mut SystemBus, sqr: [u32; 4], len: usize) -> Vec<u32> {
        const EOC: u32 = 1 << 2;
        const EOS: u32 = 1 << 3;
        for (k, word) in sqr.into_iter().enumerate() {
            bus.write_u32(self.base + 0x30 + 4 * k as u64, word)
                .expect("write SQRx");
        }
        bus.write_u32(self.base, EOC | EOS).expect("clear EOC/EOS");
        bus.write_u32(self.base + 0x08, (1 << 28) | 1 | (1 << 2))
            .expect("ADSTART");
        let codes = self.poll_scan(bus, len);
        for k in 0..4u64 {
            bus.write_u32(self.base + 0x30 + 4 * k, 0)
                .expect("restore SQRx");
        }
        codes
    }

    /// Four software triggers of a discontinuous (`CFGR.DISCEN`) L4/H7
    /// `SQRx` sequence of `len` channels cycling NTC, diode, idle, read by
    /// polling. Each converts the next `discnum` + 1 channels; only the one
    /// reaching the sequence end raises `EOS`. Restores CFGR and SQR1..4.
    fn scan_sqr_discontinuous(
        &self,
        bus: &mut SystemBus,
        discnum: u32,
        len: usize,
    ) -> Vec<Vec<u32>> {
        const EOC: u32 = 1 << 2;
        const EOS: u32 = 1 << 3;
        let cycle = [NTC_CHANNEL, DIODE_CHANNEL, IDLE_CHANNEL].map(u32::from);
        let mut sqr = [len as u32 - 1, 0, 0, 0];
        for slot in 0..len {
            let (word, field) = match slot {
                0..=3 => (0, slot + 1),
                _ => (1, slot - 4),
            };
            sqr[word] |= cycle[slot % 3] << (6 * field);
        }
        for (k, word) in sqr.into_iter().enumerate() {
            bus.write_u32(self.base + 0x30 + 4 * k as u64, word)
                .expect("write SQRx");
        }
        let cfgr = bus.read_u32(self.base + 0x0C).expect("CFGR");
        bus.write_u32(self.base + 0x0C, cfgr | (1 << 16) | (discnum << 17))
            .expect("DISCEN");
        let per_trigger = discnum as usize + 1;
        let mut position = 0;
        let mut triggers = Vec::new();
        for _ in 0..4 {
            let n = per_trigger.min(len - position);
            bus.write_u32(self.base, EOC | EOS).expect("clear EOC/EOS");
            bus.write_u32(self.base + 0x08, (1 << 28) | 1 | (1 << 2))
                .expect("ADSTART");
            let codes: Vec<u32> = (0..n)
                .map(|_| {
                    let isr = bus.read_u32(self.base).unwrap();
                    assert_ne!(isr & EOC, 0, "{}: EOC", self.id);
                    assert_eq!(
                        isr & EOS != 0,
                        position + n == len,
                        "{}: EOS only on the subsequence reaching SQ{len}",
                        self.id
                    );
                    bus.write_u32(self.base, EOC).expect("clear EOC");
                    bus.read_u32(self.base + 0x40).expect("read DR")
                })
                .collect();
            assert_eq!(
                bus.read_u32(self.base).unwrap() & EOC,
                0,
                "{}: the subsequence stops after {n}",
                self.id
            );
            triggers.push(codes);
            position = (position + n) % len;
        }
        bus.write_u32(self.base + 0x0C, cfgr).expect("restore CFGR");
        for k in 0..4u64 {
            bus.write_u32(self.base + 0x30 + 4 * k, 0)
                .expect("restore SQRx");
        }
        triggers
    }

    /// One software-started F2/F4/F7 `CR1.SCAN` group with `CR2.EOCS`, read
    /// by polling: wait for EOC, clear it, read DR. Leaves CR1/SQR1 at 0.
    fn scan_f4(&self, bus: &mut SystemBus, channels: &[u8]) -> Vec<u32> {
        const ADON: u32 = 1;
        const EOCS: u32 = 1 << 10;
        let sqr3 = channels
            .iter()
            .enumerate()
            .fold(0, |w, (k, &ch)| w | (u32::from(ch) << (5 * k)));
        bus.write_u32(self.base + 0x34, sqr3).expect("SQR3 SQ1..");
        bus.write_u32(self.base + 0x2c, (channels.len() as u32 - 1) << 20)
            .expect("SQR1.L");
        bus.write_u32(self.base + 0x04, 1 << 8).expect("CR1.SCAN");
        bus.write_u32(self.base, 0).expect("clear SR");
        bus.write_u32(self.base + 0x08, ADON | EOCS).expect("ADON");
        bus.write_u32(self.base + 0x08, ADON | EOCS | (1 << 30))
            .expect("SWSTART");
        let codes = (0..channels.len())
            .map(|slot| {
                let mut ticks = 0;
                while bus.read_u32(self.base).unwrap() & (1 << 1) == 0 {
                    assert!(ticks < 64, "{}: EOC for conversion {slot}", self.id);
                    bus.tick_peripherals_fully();
                    ticks += 1;
                }
                bus.write_u32(self.base, 0).expect("clear EOC");
                bus.read_u32(self.base + 0x4c).expect("read DR")
            })
            .collect();
        bus.write_u32(self.base + 0x04, 0).expect("restore CR1");
        bus.write_u32(self.base + 0x2c, 0).expect("restore SQR1");
        codes
    }

    /// F2/F4/F7 injected group over `channels` (rank 1 first), placed the
    /// way the ST HAL's `ADC_JSQR_RK_JL` does — rank r of n in `JSQ(4 - n +
    /// r)` — and started by `CR2.JSWSTART` (bit 22, RM0090 §13.13.3). Waits
    /// for `JEOC` and returns `JDR1..JDRn`.
    fn injected_f4(&self, bus: &mut SystemBus, channels: &[u8]) -> Vec<u32> {
        const ADON: u32 = 1;
        let n = channels.len();
        let jsqr = channels
            .iter()
            .enumerate()
            .fold(((n as u32) - 1) << 20, |w, (r, &ch)| {
                w | (u32::from(ch) << (5 * (4 - n + r)))
            });
        bus.write_u32(self.base + 0x38, jsqr).expect("JSQR");
        bus.write_u32(self.base, 0).expect("clear SR");
        bus.write_u32(self.base + 0x08, ADON).expect("ADON");
        bus.write_u32(self.base + 0x08, ADON | (1 << 22))
            .expect("JSWSTART");
        let mut ticks = 0;
        while bus.read_u32(self.base).unwrap() & (1 << 2) == 0 {
            assert!(ticks < 64 * n, "{}: JEOC", self.id);
            bus.tick_peripherals_fully();
            ticks += 1;
        }
        (0..n)
            .map(|k| bus.read_u32(self.base + 0x3c + 4 * k as u64).expect("JDRx"))
            .collect()
    }

    /// L4/L5/H7 injected group over `channels` (`JSQ1` first), started by
    /// `CR.JADSTART` (bit 3) with a software trigger. The L4 map (L4, L5)
    /// packs `JSQR` with `JSQ1` at bit 8 (`stm32l476.svd`); the H7 with
    /// `JSQ1` at bit 9 (`stm32h735.svd`). Expects `JEOC`|`JEOS` and
    /// `JADSTART` cleared, and returns `JDR1..JDRn`.
    fn injected_sqr(&self, bus: &mut SystemBus, channels: &[u8]) -> Vec<u32> {
        const JEOC_JEOS: u32 = (1 << 5) | (1 << 6);
        const JADSTART: u32 = 1 << 3;
        let jsq1 = if matches!(self.family, Family::Stm32H7) {
            9
        } else {
            8
        };
        let n = channels.len();
        let jsqr = channels
            .iter()
            .enumerate()
            .fold((n as u32) - 1, |w, (k, &ch)| {
                w | (u32::from(ch) << (jsq1 + 6 * k))
            });
        bus.write_u32(self.base + 0x4C, jsqr).expect("JSQR");
        bus.write_u32(self.base, JEOC_JEOS)
            .expect("clear JEOC/JEOS");
        bus.write_u32(self.base + 0x08, (1 << 28) | 1 | JADSTART)
            .expect("JADSTART");
        assert_eq!(
            bus.read_u32(self.base).unwrap() & JEOC_JEOS,
            JEOC_JEOS,
            "{}: the injected group sets JEOC and JEOS",
            self.id
        );
        assert_eq!(
            bus.read_u32(self.base + 0x08).unwrap() & JADSTART,
            0,
            "{}: JADSTART self-clears",
            self.id
        );
        (0..n)
            .map(|k| bus.read_u32(self.base + 0x80 + 4 * k as u64).expect("JDRx"))
            .collect()
    }

    /// F2/F4/F7 `CR1.DISCEN` with `DISCNUM` = `per_trigger` - 1 and
    /// `CR2.EOCS`, read by polling: each SWSTART lands the next
    /// `per_trigger` channels (fewer at the group end), one `EOC` each, and
    /// then the ADC stays idle. Leaves CR1/SQR1 at 0.
    fn discen_f4(
        &self,
        bus: &mut SystemBus,
        channels: &[u8],
        per_trigger: usize,
        triggers: usize,
    ) -> Vec<Vec<u32>> {
        const ADON: u32 = 1;
        const EOCS: u32 = 1 << 10;
        let sqr3 = channels
            .iter()
            .enumerate()
            .fold(0, |w, (k, &ch)| w | (u32::from(ch) << (5 * k)));
        bus.write_u32(self.base + 0x34, sqr3).expect("SQR3 SQ1..");
        bus.write_u32(self.base + 0x2c, (channels.len() as u32 - 1) << 20)
            .expect("SQR1.L");
        let cr1 = (1 << 8) | (1 << 11) | ((per_trigger as u32 - 1) << 13);
        bus.write_u32(self.base + 0x04, cr1).expect("CR1.DISCEN");
        bus.write_u32(self.base, 0).expect("clear SR");
        bus.write_u32(self.base + 0x08, ADON | EOCS).expect("ADON");
        let mut next = 0;
        let out = (0..triggers)
            .map(|t| {
                bus.write_u32(self.base + 0x08, ADON | EOCS | (1 << 30))
                    .expect("SWSTART");
                let n = per_trigger.min(channels.len() - next);
                next = (next + n) % channels.len();
                let codes = (0..n)
                    .map(|slot| {
                        let mut ticks = 0;
                        while bus.read_u32(self.base).unwrap() & (1 << 1) == 0 {
                            assert!(ticks < 64, "{}: trigger {t} EOC {slot}", self.id);
                            bus.tick_peripherals_fully();
                            ticks += 1;
                        }
                        bus.write_u32(self.base, 0).expect("clear EOC");
                        bus.read_u32(self.base + 0x4c).expect("read DR")
                    })
                    .collect();
                for _ in 0..64 {
                    bus.tick_peripherals_fully();
                }
                assert_eq!(
                    bus.read_u32(self.base).unwrap() & (1 << 1),
                    0,
                    "{}: trigger {t} stops after its {n}-channel subgroup",
                    self.id
                );
                codes
            })
            .collect();
        bus.write_u32(self.base + 0x04, 0).expect("restore CR1");
        bus.write_u32(self.base + 0x2c, 0).expect("restore SQR1");
        out
    }

    /// One software-started v2 scan read by polling: wait for EOC, read DR.
    /// EOS must stay low until the last conversion lands. Leaves CFGR1 at 0.
    fn scan_v2(&self, bus: &mut SystemBus, cfgr1: u32, chselr: u32, len: usize) -> Vec<u32> {
        const EOC: u32 = 1 << 2;
        const EOS: u32 = 1 << 3;
        let ldo = if matches!(self.family, Family::Stm32Adc4) {
            1 << 28
        } else {
            0
        };
        bus.write_u32(self.base + 0x0C, cfgr1).expect("write CFGR1");
        bus.write_u32(self.base + 0x28, chselr)
            .expect("write CHSELR");
        bus.write_u32(self.base, EOC | EOS).expect("clear EOC/EOS");
        bus.write_u32(self.base + 0x08, ldo | 1 | (1 << 2))
            .expect("ADSTART");
        let codes = self.poll_scan(bus, len);
        bus.write_u32(self.base + 0x0C, 0).expect("restore CFGR1");
        codes
    }

    /// Read a running `len`-conversion scan the way polling firmware does:
    /// wait for EOC, clear it, read DR. EOS must stay low until the last
    /// conversion lands.
    fn poll_scan(&self, bus: &mut SystemBus, len: usize) -> Vec<u32> {
        const EOC: u32 = 1 << 2;
        const EOS: u32 = 1 << 3;
        (0..len)
            .map(|slot| {
                let isr = bus.read_u32(self.base).unwrap();
                assert_ne!(isr & EOC, 0, "{}: EOC for conversion {slot}", self.id);
                assert_eq!(
                    isr & EOS != 0,
                    slot == len - 1,
                    "{}: EOS only when the last of {len} conversions lands (at {slot})",
                    self.id
                );
                bus.write_u32(self.base, EOC).expect("clear EOC");
                bus.read_u32(self.base + 0x40).expect("read DR")
            })
            .collect()
    }

    fn convert(&self, bus: &mut SystemBus, channel: u8) -> u32 {
        match self.family {
            Family::Xadc => {
                // XAdcPs_ReadInternalReg: read command, dummy RDFIFO pop,
                // write the dummy back (a no-op), pop the result.
                let (cmdfifo, rdfifo) = (self.base + 0x10, self.base + 0x14);
                bus.write_u32(cmdfifo, (1 << 26) | (u32::from(channel) << 16))
                    .expect("DRP read");
                let dummy = bus.read_u32(rdfifo).expect("dummy RDFIFO");
                bus.write_u32(cmdfifo, dummy).expect("dummy command");
                bus.read_u32(rdfifo).expect("RDFIFO") >> 4
            }
            Family::Stm32F4 => {
                const ADON: u32 = 1;
                bus.write_u32(self.base + 0x34, u32::from(channel))
                    .expect("select SQR3 SQ1");
                bus.write_u32(self.base + 0x08, ADON).expect("ADON");
                bus.write_u32(self.base + 0x08, ADON | (1 << 30))
                    .expect("SWSTART");
                for _ in 0..64 {
                    bus.tick_peripherals_fully();
                }
                assert_ne!(
                    bus.read_u32(self.base).unwrap() & (1 << 1),
                    0,
                    "{}: conversion sets EOC",
                    self.id
                );
                bus.read_u32(self.base + 0x4c).expect("read DR")
            }
            Family::Stm32H7 | Family::Stm32L4 => {
                const EOC_EOS: u32 = (1 << 2) | (1 << 3);
                bus.write_u32(self.base + 0x30, u32::from(channel) << 6)
                    .expect("select SQR1 SQ1");
                bus.write_u32(self.base + 0x08, (1 << 28) | 1 | (1 << 2))
                    .expect("ADSTART");
                assert_eq!(
                    bus.read_u32(self.base).unwrap() & EOC_EOS,
                    EOC_EOS,
                    "{}: SQ1 conversion sets EOC and EOS",
                    self.id
                );
                let code = bus.read_u32(self.base + 0x40).expect("read DR");
                bus.write_u32(self.base, EOC_EOS).expect("clear EOC/EOS");
                code
            }
            Family::Stm32V2 | Family::Stm32Adc4 => {
                const EOC_EOS: u32 = (1 << 2) | (1 << 3);
                // ADC4 keeps ADVREGEN on while it converts.
                let ldo = if matches!(self.family, Family::Stm32Adc4) {
                    1 << 28
                } else {
                    0
                };
                bus.write_u32(self.base + 0x28, 1 << channel)
                    .expect("select CHSELR channel");
                bus.write_u32(self.base, EOC_EOS).expect("clear EOC/EOS");
                bus.write_u32(self.base + 0x08, ldo | 1 | (1 << 2))
                    .expect("ADSTART");
                assert_eq!(
                    bus.read_u32(self.base).unwrap() & EOC_EOS,
                    EOC_EOS,
                    "{}: single-channel CHSELR conversion sets EOC and EOS",
                    self.id
                );
                bus.read_u32(self.base + 0x40).expect("read DR")
            }
            Family::Imxrt => {
                bus.write_u32(self.base, u32::from(channel))
                    .expect("HC0 software trigger");
                assert_eq!(
                    bus.read_u32(self.base + 0x20).unwrap() & 1,
                    0,
                    "{}: HS.COCO0 clears when HC0 starts a conversion",
                    self.id
                );
                // A conversion takes 3 us; publish 10 us at the RT1064's
                // 600 MHz core (the model's default without a Machine) on
                // the bus clock the model reads.
                let now = bus.current_cycle;
                bus.set_current_cycle(now + 6_000);
                assert_ne!(
                    bus.read_u32(self.base + 0x20).unwrap() & 1,
                    0,
                    "{}: HS.COCO0 sets once the conversion time has elapsed",
                    self.id
                );
                bus.read_u32(self.base + 0x24).expect("read R0")
            }
            Family::Sam4s => {
                bus.write_u32(self.base + 0x10, 1 << channel).expect("CHER");
                bus.write_u32(self.base, 1 << 1).expect("CR.START");
                let code = bus
                    .read_u32(self.base + 0x50 + 4 * u64::from(channel))
                    .expect("read CDRx");
                bus.write_u32(self.base + 0x14, 1 << channel).expect("CHDR");
                code
            }
            Family::Apollo4 => {
                bus.write_u32(self.base + 0x0C, 1 | (u32::from(channel) << 8))
                    .expect("enable slot 0");
                bus.write_u32(self.base + 0x08, 0x37).expect("SWT");
                let word = bus.read_u32(self.base + 0x3C).expect("pop FIFOPR");
                bus.write_u32(self.base + 0x0C, 0).expect("disable slot 0");
                (word & 0xF_FFFF) >> 6
            }
            Family::Cc2538 => {
                // Contiki-NG adc_get(): EREF = AVDD5, EDIV = 512, ECH = AINn.
                bus.write_u32(self.base + 0x08, (2 << 6) | (3 << 4) | u32::from(channel))
                    .expect("ADCCON3 extra conversion");
                assert_ne!(
                    bus.read_u32(self.base).unwrap() & 0x80,
                    0,
                    "{}: ADCCON1.EOC",
                    self.id
                );
                let low = bus.read_u32(self.base + 0x0C).expect("ADCL") & 0xFC;
                let high = bus.read_u32(self.base + 0x10).expect("ADCH");
                let raw = (low | (high << 8)) as u16 as i16;
                assert!(raw >= 0, "{}: single-ended results are positive", self.id);
                assert_eq!(raw & 0xF, 0, "{}: 12 significant bits", self.id);
                (raw >> 4) as u32
            }
            Family::Eoss3 => {
                // qorc-sdk HAL_ADC_Init / HAL_ADC_StartConversion /
                // HAL_ADC_GetData.
                let control = self.base + 0x08;
                let status = self.base + 0x04;
                bus.write_u32(control, 0).expect("CONTROL = 0");
                bus.write_u32(control, u32::from(channel) << 1)
                    .expect("CONTROL.SEL");
                let temp = bus.read_u32(control).unwrap();
                bus.write_u32(control, temp | 1).expect("CONTROL.SOC");
                assert_eq!(
                    bus.read_u32(status).unwrap(),
                    0,
                    "{}: STATUS.EOC goes low after SOC",
                    self.id
                );
                assert_eq!(
                    bus.read_u32(status).unwrap(),
                    1,
                    "{}: STATUS.EOC rises when the conversion is done",
                    self.id
                );
                let code = bus.read_u32(self.base).expect("OUT") & 0xFFF;
                let temp = bus.read_u32(control).unwrap();
                bus.write_u32(control, temp & !1).expect("clear SOC");
                code
            }
            Family::Max32650 => {
                // msdk MXC_ADC_RevA_StartConversion + MXC_ADC_RevA_GetData.
                const START: u32 = 1;
                const CH_SEL: u32 = 0xF << 12;
                const DONE_IF: u32 = 1 << 16;
                let ctrl = self.base;
                let intr = self.base + 0x0C;
                let value = bus.read_u32(ctrl).unwrap() & !CH_SEL;
                bus.write_u32(ctrl, value | (u32::from(channel) << 12))
                    .expect("CTRL.CH_SEL");
                let ie = bus.read_u32(intr).unwrap() & 0x1F;
                bus.write_u32(intr, ie | DONE_IF).expect("clear DONE_IF");
                assert_eq!(
                    bus.read_u32(intr).unwrap() & DONE_IF,
                    0,
                    "{}: DONE_IF W1C",
                    self.id
                );
                let value = bus.read_u32(ctrl).unwrap();
                bus.write_u32(ctrl, value | START).expect("CTRL.START");
                assert_eq!(
                    bus.read_u32(self.base + 0x04).unwrap() & 1,
                    0,
                    "{}: STATUS.ACTIVE clear when done",
                    self.id
                );
                assert_ne!(
                    bus.read_u32(intr).unwrap() & DONE_IF,
                    0,
                    "{}: the conversion raises INTR.DONE_IF",
                    self.id
                );
                // CTRL.DATA_ALIGN = 0: LSB justified.
                bus.read_u32(self.base + 0x08).expect("DATA") & 0xFFFF
            }
            Family::Da14Gpadc => {
                // SDK hw_gpadc_set_positive, hw_gpadc_clear_interrupt,
                // hw_gpadc_adc_measure (START, spin until it reads 0),
                // hw_gpadc_get_value (RESULT >> (6 - CONV_NRS)).
                const START: u32 = 1 << 1;
                const INT: u32 = 1 << 4;
                let ctrl = self.base;
                bus.write_u32(self.base + 0x0C, u32::from(channel) << 4)
                    .expect("GP_ADC_SEL_P");
                bus.write_u32(self.base + 0x1C, 1).expect("GP_ADC_CLR_INT");
                assert_eq!(
                    bus.read_u32(ctrl).unwrap() & INT,
                    0,
                    "{}: CLEAR_INT clears GP_ADC_INT",
                    self.id
                );
                let value = bus.read_u32(ctrl).unwrap();
                bus.write_u32(ctrl, value | START).expect("GP_ADC_START");
                let value = bus.read_u32(ctrl).unwrap();
                assert_eq!(value & START, 0, "{}: GP_ADC_START clears", self.id);
                assert_ne!(value & INT, 0, "{}: the conversion sets INT", self.id);
                let raw = bus.read_u32(self.base + 0x20).expect("RESULT") & 0xFFFF;
                let conv_nrs = (bus.read_u32(self.base + 0x04).unwrap() >> 6) & 7;
                assert_eq!(conv_nrs, 4, "{}: 16 samples", self.id);
                // 10 + 4 bits; the noiseless samples carry no extra bits.
                let value = raw >> (6 - conv_nrs);
                assert_eq!(value & 0xF, 0, "{}: noiseless oversampling", self.id);
                value >> conv_nrs
            }
            Family::Caes => {
                // Single channel at 1 V/V, one sweep, DATA_OUT bit 11 flipped.
                let chan_cfg = self.base + 0x10 + 4 * u64::from(channel);
                bus.write_u32(chan_cfg, (1 << 4) | 1).expect("SECHAN_CFG");
                bus.write_u32(self.base + 0x04, 1).expect("ADC_TRIGGER");
                assert_ne!(
                    bus.read_u32(self.base + 0x8C).unwrap() & (1 << channel),
                    0,
                    "{}: channel {channel} pends",
                    self.id
                );
                let raw = bus
                    .read_u32(self.base + 0x90 + 4 * u64::from(channel))
                    .expect("DATA");
                bus.write_u32(chan_cfg, 0x10).expect("disable");
                (raw & 0xFFF) ^ 0x800
            }
            Family::Samd21 => {
                // ASF adc_start_conversion + adc_read: MUXNEG = GND (0x18),
                // SWTRIG.START, RESRDY on the next tick, RESULT clears it.
                const RESRDY: u8 = 1;
                bus.write_u32(self.base + 0x10, 0x1800 | u32::from(channel))
                    .expect("INPUTCTRL");
                bus.write_u8(self.base + 0x0C, 1 << 1)
                    .expect("SWTRIG.START");
                assert_eq!(
                    bus.read_u8(self.base + 0x18).unwrap() & RESRDY,
                    0,
                    "{}: START clears RESRDY until the conversion ends",
                    self.id
                );
                bus.tick_peripherals_fully();
                assert_ne!(
                    bus.read_u8(self.base + 0x18).unwrap() & RESRDY,
                    0,
                    "{}: the conversion raises INTFLAG.RESRDY",
                    self.id
                );
                let code = u32::from(bus.read_u16(self.base + 0x1A).expect("RESULT"));
                assert_eq!(
                    bus.read_u8(self.base + 0x18).unwrap() & RESRDY,
                    0,
                    "{}: the RESULT read clears RESRDY",
                    self.id
                );
                code
            }
            Family::Efr32Iadc => {
                // em_iadc IADC_initSingle + IADC_command(SINGLESTART), poll
                // STATUS.SINGLEFIFODV, IADC_pullSingleFifoResult. Channel
                // n is PA<n>: PORTPOS = PORTA, PINPOS = n.
                const SINGLEFIFODV: u32 = 1 << 8;
                const SINGLEDONE: u32 = 1 << 9;
                let (cmd, status, iflag) = (self.base + 0x0C, self.base + 0x14, self.base + 0x24);
                bus.write_u32(
                    self.base + 0x98,
                    (IADC_PORTPOS_PORTA << 12) | (u32::from(channel) << 8),
                )
                .expect("SINGLE");
                bus.write_u32(iflag, SINGLEDONE).expect("clear SINGLEDONE");
                bus.write_u32(cmd, 1).expect("SINGLESTART");
                bus.tick_peripherals_fully();
                assert_ne!(
                    bus.read_u32(status).unwrap() & SINGLEFIFODV,
                    0,
                    "{}: the conversion raises SINGLEFIFODV",
                    self.id
                );
                assert_ne!(
                    bus.read_u32(iflag).unwrap() & SINGLEDONE,
                    0,
                    "{}: and IF.SINGLEDONE",
                    self.id
                );
                let code = bus.read_u32(self.base + 0x74).expect("SINGLEFIFODATA");
                assert_eq!(
                    bus.read_u32(status).unwrap() & SINGLEFIFODV,
                    0,
                    "{}: popping the only result empties the FIFO",
                    self.id
                );
                assert_eq!(
                    bus.read_u32(self.base + 0x7C).unwrap(),
                    code,
                    "{}: SINGLEDATA keeps the newest result",
                    self.id
                );
                code
            }
            Family::Saadc { .. } => unreachable!("SAADC converts as a scan"),
        }
    }

    /// The shared STM32 model also exposes what it holds per channel, so an
    /// unrouted channel can be shown undriven rather than merely "some code".
    fn held_count(&self, bus: &mut SystemBus, channel: u8) -> Option<u16> {
        if !matches!(
            self.family,
            Family::Stm32F4
                | Family::Stm32H7
                | Family::Stm32L4
                | Family::Stm32V2
                | Family::Stm32Adc4
        ) {
            return None;
        }
        let index = bus.find_peripheral_index_by_name(&self.id)?;
        Some(
            bus.peripherals[index]
                .dev
                .as_any_mut()?
                .downcast_mut::<labwired_core::peripherals::adc::Adc>()
                .expect("STM32 ADC uses the shared model")
                .channel_input_count(channel),
        )
    }
}

fn generic_netlists(adc_id: &str, [ntc_channel, diode_channel]: [u8; 2]) -> SystemManifest {
    let mut analog = SystemManifest::from_file(root(
        "examples/ci-fixture-unknown-arch/analog-ntc-diode-system.yaml",
    ))
    .expect("load generic NTC divider and diode netlists");
    for model in &mut analog.cosim_models {
        let (key, channel) = match model.id.as_str() {
            "ntc_divider" => ("v_sense", ntc_channel),
            "diode_load_line" => ("v_diode", diode_channel),
            other => panic!("unexpected fixture model {other}"),
        };
        model
            .outputs
            .insert(key.into(), format!("adc.{adc_id}.{channel}_volts"));
    }
    analog
}

/// The sweep drives whatever map a descriptor names, so a wrong but
/// self-consistent `config.profile` would still convert. For profiles named
/// `stm32<series>...`, pin the map to the one that series' reference manual
/// gives its ADC: F0/L0 `CHSELR` (RM0091, RM0367), F2/F4/F7 (RM0033, RM0090,
/// RM0410), H7 (RM0433), L4/L5 (RM0351, RM0438), WBA ADC4 (RM0493).
fn assert_stm32_series_map(board: &str, converter: &Converter) {
    let Some(series) = board.strip_prefix("stm32") else {
        return;
    };
    let expected = match series.get(..2) {
        Some("f0" | "l0") => "Stm32V2",
        Some("f2" | "f4" | "f7") => "Stm32F4",
        Some("h7") => "Stm32H7",
        Some("l4" | "l5") => "Stm32L4",
        Some("wb") if series.starts_with("wba") => "Stm32Adc4",
        _ => panic!("{board}: no reference-manual ADC map recorded for this STM32 series"),
    };
    assert_eq!(
        format!("{:?}", converter.family),
        expected,
        "{board}:{} declares the wrong ADC register map for its series",
        converter.id
    );
}

fn onboarding_profiles() -> Vec<PathBuf> {
    let mut paths: Vec<_> = std::fs::read_dir(root("configs/systems/onboarding"))
        .expect("read onboarding systems")
        .map(|entry| entry.expect("onboarding entry").path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "yaml"))
        .collect();
    paths.sort();
    paths
}

fn assert_converter_routes_generic_analog(
    board: &str,
    chip: &ChipDescriptor,
    base_profile: &SystemManifest,
    converter: &Converter,
) {
    let tag = format!("{board}:{}", converter.id);
    let mut profile = base_profile.clone();
    let [ntc_channel, diode_channel] = converter.routed_channels();
    profile.cosim_models =
        generic_netlists(&converter.id, [ntc_channel, diode_channel]).cosim_models;
    let mut bus = SystemBus::from_config(chip, &profile).expect("build onboarding bus");
    converter.power_up(&mut bus);
    let mut session = CosimSession::new(&profile.cosim_models, Path::new("."), &bus)
        .expect("build analog co-simulation")
        .expect("profile carries both generic devices");
    assert_eq!(
        session.binding_errors(),
        &[] as &[RoutingError],
        "{tag}: both netlists bind to ADC channels {ntc_channel}/{diode_channel}"
    );

    let expected_diode_v = diode_load_line_voltage();
    let ntc_path = format!("adc.{}.{ntc_channel}_volts", converter.id);
    let diode_path = format!("adc.{}.{diode_channel}_volts", converter.id);
    let mut cycles = 0;
    let mut ntc_codes = Vec::new();
    for temperature_c in [-10.0_f64, 25.0, 80.0] {
        session
            .set_signal_number("ui.ntc.temperature_c", temperature_c)
            .expect("set NTC temperature");
        cycles += session.cycles_until_boundary(cycles);
        let (steps, errors) = session
            .advance_to(cycles, &mut bus)
            .expect("solve and route both netlists");
        assert_eq!(steps.len(), 2, "{tag}: both netlists solve each step");
        assert!(errors.is_empty(), "{tag}: route errors: {errors:?}");

        let ntc_v = signal_voltage(&session, &ntc_path);
        let r = ntc_resistance_ohms(temperature_c);
        let expected_ntc_v = 3.3 * r / (10_000.0 + r);
        assert!(
            (ntc_v - expected_ntc_v).abs() < 1e-9,
            "{tag}: at {temperature_c} °C the Beta divider should be {expected_ntc_v} V, got {ntc_v} V"
        );
        let diode_v = signal_voltage(&session, &diode_path);
        assert!(
            (diode_v - expected_diode_v).abs() < 2e-5,
            "{tag}: Shockley load line should be {expected_diode_v} V, got {diode_v} V"
        );

        let [ntc_code, diode_code] = converter.convert_routed(&mut bus);
        assert_eq!(
            ntc_code,
            converter.expected_ntc_code(ntc_v),
            "{tag}: firmware-visible NTC conversion at {temperature_c} °C"
        );
        assert_eq!(
            diode_code,
            converter.expected_code(diode_v),
            "{tag}: firmware-visible diode conversion"
        );
        if let Some(idle) = converter.held_count(&mut bus, IDLE_CHANNEL) {
            assert_eq!(idle, 0xFFFF, "{tag}: unrouted channel stays undriven");
        }
        if matches!(
            converter.family,
            Family::Imxrt
                | Family::Sam4s
                | Family::Apollo4
                | Family::Cc2538
                | Family::Caes
                | Family::Samd21
                | Family::Efr32Iadc
        ) {
            assert_eq!(
                converter.convert(&mut bus, IDLE_CHANNEL),
                0,
                "{tag}: unrouted channel converts to zero"
            );
        }
        ntc_codes.push(ntc_code);
    }
    // The NTC sits on the low side: hotter means lower resistance, lower volts.
    if matches!(
        converter.family,
        Family::Apollo4 | Family::Eoss3 | Family::Xadc
    ) {
        // Above the 1.19 V (Apollo4), 1.4 V (EOS S3) or 1 V (XADC) full
        // scale the -10 and 25 °C dividers (2.79 V, 1.65 V) both saturate;
        // only 80 °C (0.37 V) is in range.
        assert_eq!(
            ntc_codes[..2],
            [4095, 4095],
            "{tag}: dividers above VREF saturate"
        );
        assert!(
            ntc_codes[2] < 4095,
            "{tag}: in-range divider resolves: {ntc_codes:?}"
        );
    } else {
        assert!(
            ntc_codes[0] > ntc_codes[1] && ntc_codes[1] > ntc_codes[2],
            "{tag}: codes track the NTC temperature: {ntc_codes:?}"
        );
    }
}

#[test]
fn every_onboarding_profile_with_a_convertible_adc_routes_ntc_and_diode_into_each_instance() {
    let mut converted = Vec::new();
    let mut not_convertible = Vec::new();
    let mut without_adc = 0;
    let mut cpu_owned = 0;
    let mut instances = 0;
    for system_path in onboarding_profiles() {
        let board = system_path
            .file_stem()
            .unwrap()
            .to_string_lossy()
            .to_string();
        let mut profile =
            SystemManifest::from_file(&system_path).unwrap_or_else(|e| panic!("load {board}: {e}"));
        // Anchor the chip path at the system file, as the CLI does, so a
        // declarative peripheral's `path` (EOS S3 `io_in`) resolves next to
        // the chip when the bus is built.
        let chip_path = system_path.parent().unwrap().join(&profile.chip);
        profile.chip = chip_path.to_string_lossy().into_owned();
        let chip = ChipDescriptor::from_file(&chip_path)
            .unwrap_or_else(|e| panic!("load {board} descriptor: {e}"));
        // The AVR ADC registers live on the CPU, not the bus; firmware
        // conversions for these are in `cosim_onboarding_avr_analog.rs`.
        if chip.peripherals.iter().any(|p| p.r#type == "avr_adc") {
            cpu_owned += 1;
            continue;
        }
        match converters(&chip) {
            Ok(list) if list.is_empty() => without_adc += 1,
            Ok(list) => {
                for converter in &list {
                    assert_stm32_series_map(&board, converter);
                    assert_converter_routes_generic_analog(&board, &chip, &profile, converter);
                }
                instances += list.len();
                converted.push(board);
            }
            Err(_) => not_convertible.push(board),
        }
    }
    assert_eq!(
        converted, CONVERTED,
        "onboarding profiles converted end to end"
    );
    assert_eq!(
        not_convertible, DECLARED_NOT_CONVERTIBLE,
        "onboarding profiles with an ADC LabWired cannot convert yet"
    );
    assert_eq!(
        instances, 81,
        "ADC instances exercised; update docs/mixed_signal_board_coverage.md"
    );
    assert_eq!(
        cpu_owned, 34,
        "simavr profiles with a CPU-owned AVR ADC or comparator, driven in cosim_onboarding_avr_analog.rs"
    );
    assert_eq!(
        without_adc, 154,
        "onboarding profiles that declare no modelled ADC"
    );
}
