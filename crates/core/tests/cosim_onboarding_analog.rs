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
    "arduino_uno_r4_minima",
    "atsamd21j17d-aft",
    "atsamd51g19a",
    "brd4116a",
    "brd4117a",
    "brd4118a",
    "brd4120a",
    "brd4121a",
    "brd4162a",
    "brd4186c",
    "brd4402a",
    "cc2538",
    "efm32g210",
    "efm32g222",
    "efm32g232",
    "efm32g842",
    "efm32g890",
    "efm32gg942",
    "efm32gg995",
    "efm32hg350",
    "efm32jg1",
    "efm32jg12",
    "efm32lg942",
    "efm32lg995",
    "efm32pg1",
    "efm32pg12",
    "efm32tg840",
    "efm32wg995",
    "efm32zg222",
    "efr32mg1",
    "efr32mg12",
    "efr32mg13",
    "efr32mg24",
    "efr32mg26",
    "efr32xg22",
    "eos-s3-qomu",
    "eos-s3-quickfeather",
    "eos-s3",
    "ezr32hg320",
    "ezr32lg330",
    "ezr32wg330",
    "imxrt1064",
    "imxrt500",
    "mars_zx3",
    "max32652-evkit",
    "max32652",
    "mimxrt1064_evk",
    "mimxrt700_evk",
    "mimxrt798s",
    "msp430f2619",
    "mspsim-msp430f149",
    "mspsim-msp430f1611",
    "mspsim-msp430f2617",
    "mspsim-msp430f5437",
    "nrf52840",
    "nrf52840dk_nrf52840",
    "nucleo_f767zi",
    "nucleo_h753zi",
    "nucleo_wba52cg",
    "nxp-k6xf",
    "ramn",
    "renesas-ck_ra6m5",
    "renesas-da14592",
    "renesas-ek_ra2e1",
    "renesas-ek_ra8m1",
    "renesas-r7fa2e1a9",
    "renesas-r7fa2l1a",
    "renesas-r7fa4m1a",
    "renesas-r7fa6m5b",
    "renesas-r7fa8m1a",
    "sam4s",
    "sam4s16c",
    "sam4s8b",
    "sam4s_xplained",
    "sam_e70",
    "sltb001a",
    "sltb004a",
    "slwstk6220a",
    "stk3200",
    "stk3600",
    "stk3700",
    "stk3800",
    "stm32f042",
    "stm32f072",
    "stm32f072b_discovery",
    "stm32f103",
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
    "stm32l552",
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
    /// Shared `adc` model, F1 map (RM0008 §11.12): the F2/F4 register block,
    /// but `SWSTART` is `CR2` bit 22 and starts only with `EXTSEL` = 111,
    /// `JSWSTART` is bit 21 with `JEXTSEL` = 111, and `EOC` rises once at
    /// the end of the regular group (no `EOCS`).
    Stm32F1,
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
    /// SAM E70 AFEC: `CHER` channel set, `CR.START`, then `CSELR` picks the
    /// channel whose result the shared `CDR` returns (hal_atmel afec.h,
    /// Zephyr adc_sam_afec.c).
    SamAfec,
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
    /// SAM D5x ADC: the D21 pad hand-over (PMUX function B, PINCFG.PMUXEN)
    /// plus the D5x clock gates, `MCLK.APBDMASK` and the ADC's GCLK
    /// `PCHCTRL.CHEN`; `INPUTCTRL.MUXPOS` against GND, `SWTRIG.START`,
    /// `INTFLAG.RESRDY` on the next tick, 12 bits against `REFCTRL.REFSEL`
    /// INTVCC1 = VDDANA (DS60001507 §45, Arduino SAMD core analogRead).
    Samd51,
    /// EFR32 Series-2 IADC (xG22/MG24/MG26): `SINGLE.PORTPOS`/`PINPOS` mux a
    /// pad, `CMD.SINGLESTART` converts only while `EN.EN` is set,
    /// `STATUS.SINGLEFIFODV` and `IF.SINGLEDONE` rise, `SINGLEFIFODATA` pops
    /// 12 bits against AVDD (simplicity_sdk efr32mg2x_iadc.h, em_iadc.c).
    Efr32Iadc,
    /// EFR32 Series-1 ADC0 (xG1/xG12/xG13): `SINGLECTRL.POSSEL` reaches a
    /// pad through the APORT matrix (EFR32MG12 data sheet Table 6.12; PAn
    /// is CH(8+n) on APORT3 and APORT4), `CMD.SINGLESTART` converts,
    /// `STATUS.SINGLEDV` and `IF.SINGLE` rise, `SINGLEDATA` pops 12 bits
    /// against `REF` = VDD (efr32mg12p_adc.h, em_adc.c, the vendor
    /// series1 adc_single_polled example).
    Efr32S1Adc,
    /// EFM32/EZR32 Series-0 ADC0 (G/GG/LG/WG/TG/ZG/HG): `SINGLECTRL.INPUTSEL`
    /// = n picks CHn from a fixed mux (PD6/PD7 are CH6/CH7 in the vendor
    /// series0/adc examples), `CMD.SINGLESTART` converts, `STATUS.SINGLEDV`
    /// and `IF.SINGLE` rise, the `SINGLEDATA` read clears `SINGLEDV` and the
    /// `SINGLEDATAP` peek does not, 12 bits against `REF` = VDD
    /// (efm32gg_adc.h, em_adc.c, EFM32GG995F1024.svd).
    Efm32S0Adc,
    /// Renesas RA ADC12/ADC14 (`ra_adc12`/`ra_adc14`): group-A channels in
    /// `ADANSA0`, a software `ADCSR.ADST` write runs one scan that takes
    /// time, `ADST` clears at scan end and `ADDRn` holds each result, against
    /// VREFH0 3.3 V (FSP r_adc.c R_ADC_ScanStart/R_ADC_Read). ADC14 runs at
    /// `ADCER.ADPRC` = 11b, 14 bits (FSP ADC_RESOLUTION_14_BIT).
    RaAdc { adc14: bool },
    /// MSP430 ADC12 (F1/F2, `msp430_adc12`, the split map from 0x0080) and
    /// ADC12_A (F5, `msp430_adc12_a`, one block): `ADC12MCTLx.INCH` picks
    /// the input, `ENC` + `ADC12SC` in pulse mode (`SHP`) converts, `IFGx`
    /// rises and the `ADC12MEMx` read clears it, 12 bits against AVCC
    /// (3.3 V) with `SREF_0` (mspsim ADC12.java, TI msp430-gcc headers).
    Msp430Adc12 { adc12_a: bool },
    /// NXP Kinetis ADC16: `ADC16_Init` then `ADC16_DoAutoCalibration`
    /// (`SC3.CAL`, poll `SC1A.COCO`, dummy `R[0]` read, `PG/MG`), then a
    /// write to `SC1A.ADCH` converts in software-trigger mode, `COCO` rises
    /// and the `RA` read clears it. 12-bit single-ended (`CFG1.MODE` 01)
    /// against VREFH 3.3 V (mcux-sdk fsl_adc16.c, K66 RM ch. 39).
    KinetisAdc16,
    /// NXP LPADC (i.MX RT500): `LPADC_Init` (`CTRL.RST` pulse, `RSTFIFOn`,
    /// `CFG`, `FCTRLn`, `ADCEN`), one command per channel chained by
    /// `CMDH.NEXT`, trigger 0 by `SWTRIG`, results popped from `RESFIFO0`
    /// with `VALID` and `CMDSRC`. 12-bit single-ended in `D[14:3]` against
    /// VREFH 1.8 V (mcux-sdk fsl_lpadc.c, Zephyr adc_mcux_lpadc.c).
    /// `ctype_mode` is the i.MX RT700 revision (`layout: ctype_mode`): the
    /// init adds `LPADC_DoOffsetCalibration` and `LPADC_DoAutoCalibration`
    /// (`CAL_RDY`, `GCC[n].RDY`), and the chain converts at `CMDL.MODE` = 1,
    /// 16-bit `D` read directly (mcuxsdk-examples mimxrt700evk lpadc/polling).
    NxpLpadc { ctype_mode: bool },
}

/// NXP LPADC offsets (MIMXRT595S_cm33.h `ADC_Type`) and bits.
const LPADC_CTRL: u64 = 0x10;
const LPADC_STAT: u64 = 0x14;
const LPADC_CFG: u64 = 0x20;
const LPADC_SWTRIG: u64 = 0x34;
const LPADC_TCTRL0: u64 = 0xA0;
const LPADC_FCTRL0: u64 = 0xE0;
const LPADC_CMD1: u64 = 0x100;
const LPADC_RESFIFO0: u64 = 0x300;
const LPADC_ADCEN: u32 = 1 << 0;
const LPADC_RST: u32 = 1 << 1;
const LPADC_RSTFIFO0: u32 = 1 << 8;
const LPADC_RSTFIFO1: u32 = 1 << 9;
const LPADC_RDY0: u32 = 1 << 0;
const LPADC_CSCALE: u32 = 1 << 13;
const LPADC_VALID: u32 = 1 << 31;
/// RT700 additions (MIMXRT798S_cm33_core0.xml).
const LPADC_GCC0: u64 = 0xF0;
const LPADC_GCR0: u64 = 0xF8;
const LPADC_CAL_REQ: u32 = 1 << 3;
const LPADC_CALOFS: u32 = 1 << 4;
const LPADC_CAL_RDY: u32 = 1 << 10;
const LPADC_GCC_RDY: u32 = 1 << 24;
const LPADC_MODE16: u32 = 1 << 7;

/// Kinetis ADC16 offsets (MK64F12.h `ADC_Type`) and bits.
const ADC16_SC1A: u64 = 0x00;
const ADC16_CFG1: u64 = 0x08;
const ADC16_RA: u64 = 0x10;
const ADC16_SC2: u64 = 0x20;
const ADC16_SC3: u64 = 0x24;
const ADC16_OFS: u64 = 0x28;
const ADC16_PG: u64 = 0x2C;
const ADC16_COCO: u32 = 1 << 7;
const ADC16_MODE_12: u32 = 1 << 2;
const ADC16_ADTRG: u32 = 1 << 6;
const ADC16_CAL: u32 = 1 << 7;
const ADC16_CALF: u32 = 1 << 6;

/// MSP430 ADC12 register offsets from the descriptor base, as
/// `(CTL0, CTL1, IFG, MCTL0, MEM0)`: F1/F2 from `ADC12MCTL0` (0x0080), so
/// CTL0 is 0x01A0 - 0x0080; ADC12_A from `ADC12CTL0` (msp430*_symbols.ld).
const fn msp430_adc12_regs(adc12_a: bool) -> (u64, u64, u64, u64, u64) {
    if adc12_a {
        (0x00, 0x02, 0x0A, 0x10, 0x20)
    } else {
        (0x120, 0x122, 0x124, 0x00, 0x0C0)
    }
}

/// `ADC12CTL0`/`ADC12CTL1` bits (msp430f1611.h; ADC12_A names them ADC12*).
const MSP430_ADC12SC: u16 = 1 << 0;
const MSP430_ENC: u16 = 1 << 1;
const MSP430_ADC12ON: u16 = 1 << 4;
const MSP430_MSC: u16 = 1 << 7;
const MSP430_SHT0_2: u16 = 0x0200;
const MSP430_SHP: u16 = 1 << 9;
const MSP430_CONSEQ_1: u16 = 0x0002;
const MSP430_EOS: u8 = 1 << 7;

/// `SINGLE.PORTPOS` value naming GPIO port A (`_IADC_SINGLE_PORTPOS_PORTA`).
const IADC_PORTPOS_PORTA: u32 = 0x8;

/// Series-1 `SINGLECTRL` for `POSSEL` at `REF` = VDD, 12 bits, right
/// adjusted, `NEGSEL` = VSS (`_ADC_SINGLECTRL_*`, efr32mg12p_adc.h).
const fn s1_singlectrl(possel: u32) -> u32 {
    (0xFF << 16) | (possel << 8) | (2 << 5)
}

/// Series-0 `SINGLECTRL` for `INPUTSEL` at `REF` = VDD, 12 bits, right
/// adjusted, single-ended (`_ADC_SINGLECTRL_*`, efm32gg_adc.h).
const fn s0_singlectrl(inputsel: u32) -> u32 {
    (2 << 16) | (inputsel << 8)
}

/// Series-1 POSSEL `APORT<bus><X|Y>CH<8+n>`, the bus channel PAn sits on.
const fn s1_possel_pa(bus: u32, pin: u8) -> u32 {
    bus * 0x20 + 8 + pin as u32
}

/// F1 `ADC_CR2.EXTTRIG` (bit 20, RM0008 §11.12.3).
const F1_EXTTRIG: u32 = 1 << 20;

/// SAM D21 PORT GROUP0 (PA). The routed AIN5..7 are pads PA05..PA07.
const SAMD21_PORT_A: u64 = 0x4100_4400;

/// SAM D5x PORT group 0 (PA); the routed AIN5..7 are pads PA05..PA07.
const SAMD51_PORT_A: u64 = 0x4100_8000;
/// SAM D5x `MCLK.APBDMASK` and GCLK `PCHCTRL[0]` (samd51g19a.h).
const SAMD51_MCLK_APBDMASK: u64 = 0x4000_0820;
const SAMD51_GCLK_PCHCTRL0: u64 = 0x4000_1C80;

/// RA `ADCER` offset (R_ADC0_Type).
const RA_ADCER: u64 = 0x0E;

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
        // SAM E70 names its converters AFEC0/AFEC1, so the type counts too.
        let named_adc = p.id.to_ascii_lowercase().contains("adc") || kind == "sam_afec";
        if kind == "stub" || !named_adc {
            continue;
        }
        let family = if kind == "imxrt_adc" {
            Family::Imxrt
        } else if kind.contains("saadc") {
            Family::Saadc { ram: chip.ram.base }
        } else if kind == "sam4s_adc" {
            Family::Sam4s
        } else if kind == "sam_afec" {
            Family::SamAfec
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
        } else if kind == "samd51_adc" {
            Family::Samd51
        } else if kind == "xilinx_xadc" {
            Family::Xadc
        } else if kind == "efr32s2_iadc" {
            Family::Efr32Iadc
        } else if kind == "efr32s1_adc" {
            Family::Efr32S1Adc
        } else if kind == "efm32s0_adc" {
            Family::Efm32S0Adc
        } else if kind == "msp430_adc12" || kind == "msp430_adc12_a" {
            Family::Msp430Adc12 {
                adc12_a: kind == "msp430_adc12_a",
            }
        } else if kind == "kinetis_adc16" {
            Family::KinetisAdc16
        } else if kind == "nxp_lpadc" {
            Family::NxpLpadc {
                ctype_mode: p.config.get("layout").and_then(|v| v.as_str()) == Some("ctype_mode"),
            }
        } else if kind == "ra_adc12" || kind == "ra_adc14" {
            Family::RaAdc {
                adc14: kind == "ra_adc14",
            }
        } else {
            match p.config.get("profile").and_then(|v| v.as_str()) {
                Some("stm32f1") => Family::Stm32F1,
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
            Family::Stm32F1
            | Family::Stm32F4
            | Family::Stm32H7
            | Family::Stm32L4
            | Family::Stm32V2
            | Family::Stm32Adc4
            | Family::Sam4s
            | Family::SamAfec => (mv * 4095 / 3300).min(4095),
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
            // 12-bit against INTVCC1 = VDDANA (3.3 V), 2^12 / VREF,
            // truncated, clamped.
            Family::Samd51 => (mv * 4096 / 3300).min(4095),
            // 12-bit against AVDD (3.3 V), 2^12 / VREF, truncated, clamped.
            Family::Efr32Iadc => (mv * 4096 / 3300).min(4095),
            // 12-bit against REF = VDD (3.3 V); the vendor example's
            // inverse is mV = code * Vref / 4096. Truncated, clamped.
            Family::Efr32S1Adc => (mv * 4096 / 3300).min(4095),
            // Same transfer function on Series 0 (em_adc.h, efm32gg_adc.h).
            Family::Efm32S0Adc => (mv * 4096 / 3300).min(4095),
            // (2^bits - 1) / VREFH0 (3.3 V), truncated, clamped.
            Family::RaAdc { adc14: false } => (mv * 4095 / 3300).min(4095),
            Family::RaAdc { adc14: true } => (mv * 16383 / 3300).min(16383),
            // N = 4095 (Vin - VR-) / (VR+ - VR-), SREF_0: AVCC 3.3 V / AVSS,
            // truncated, clamped (SLAU049 §17.2.1).
            Family::Msp430Adc12 { .. } => (mv * 4095 / 3300).min(4095),
            // 12-bit single-ended, 1 LSB = VREFH / 2^12, truncated, clamped
            // (K66 RM §39.4.4); calibration leaves the ideal OFS = 0.
            Family::KinetisAdc16 => (mv * 4096 / 3300).min(4095),
            // 12-bit single-ended, 1 LSB = VREFH (1.8 V) / 4096, truncated,
            // clamped; the -10 °C divider (2.79 V) saturates.
            Family::NxpLpadc { .. } => (mv * 4096 / 1800).min(4095),
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
            // PA3 and PA4. emlib em_gpio.h (gsdk_4.4) bonds only PA0..PA5
            // on xG1 and xG13 (`_GPIO_PORT_A_PIN_MASK` 0x003F, SDID 80/89),
            // so PA6/PA7 do not exist there.
            Family::Efr32S1Adc => [3, 4],
            // CH6 on PD6 and CH7 on PD7, the pads the vendor series0/adc
            // examples name on every Series-0 family. emlib em_gpio.h bonds
            // PD4..PD7 on Happy Gecko (`_GPIO_PORT_D_PIN_MASK` 0x00F0).
            Family::Efm32S0Adc => [6, 7],
            // ADxx14/15: PTC0/PTC1 on ADC0, PTB10/PTB11 (Arduino A2/A3) on
            // ADC1 (MK64FN1M0VLL12-pinctrl.h). AD5a..AD7a are not bonded
            // on ADC0.
            Family::KinetisAdc16 => [14, 15],
            // CH0A (P0_5) and CH2A (P0_19), the EVK's J30-1/J30-3 pads
            // (Zephyr mimxrt595_evk pinmux_lpadc0). The MIMXRT595SFFOC bonds
            // only CH0A/B, CH1A/B, CH2A and CH6A.
            Family::NxpLpadc { ctype_mode: false } => [0, 2],
            // CH0A (PIO10_0) and CH1A (PIO10_2), the MIMXRT700-EVK's
            // pinmux_lpadc0 pads (Zephyr mimxrt700_evk-pinctrl.dtsi).
            Family::NxpLpadc { ctype_mode: true } => [0, 1],
            _ => [NTC_CHANNEL, DIODE_CHANNEL],
        }
    }

    /// The channel left unrouted: a real pad with nothing wired to it.
    fn idle_channel(&self) -> u8 {
        match self.family {
            // PA5, the last port-A pad on every Series-1 die.
            Family::Efr32S1Adc => 5,
            // CH5 (PD5): inside the HG PD4..PD7 mask too.
            Family::Efm32S0Adc => 5,
            // AD17: PTE24 on ADC0, PTA17 on ADC1.
            Family::KinetisAdc16 => 17,
            // CH1A (P0_12).
            Family::NxpLpadc { ctype_mode: false } => 1,
            // CH2A (PIO10_4).
            Family::NxpLpadc { ctype_mode: true } => 2,
            _ => IDLE_CHANNEL,
        }
    }

    /// SAM D5x `(MCLK.APBDMASK bit, GCLK PCHCTRL index)` of this ADC
    /// (hal_atmel samd51 mclk.h `MCLK_APBDMASK_ADC0/1_Pos` = 7/8,
    /// instance/adc0.h, adc1.h `ADC0/1_GCLK_ID` = 40/41).
    fn samd51_clocks(&self) -> (u32, u64) {
        match self.base {
            0x4300_1C00 => (7, 40),
            0x4300_2000 => (8, 41),
            other => panic!("{}: no SAM D5x ADC at {other:#x}", self.id),
        }
    }

    /// Bring the converter up the way firmware does, before any conversion.
    fn power_up(&self, bus: &mut SystemBus) {
        match self.family {
            Family::Stm32F4 | Family::Sam4s => {}
            Family::SamAfec => {
                // adc_sam_init: SWRST, MR, COCR = VREFP/2 behind each CSELR.
                bus.write_u32(self.base, 1).expect("CR.SWRST");
                bus.write_u32(self.base + 0x04, (1 << 23) | (5 << 16))
                    .expect("MR");
                for channel in 0..12 {
                    bus.write_u32(self.base + 0x64, channel).expect("CSELR");
                    bus.write_u32(self.base + 0x6C, 512).expect("COCR");
                }
            }
            Family::Stm32F1 => {
                const ADON: u32 = 1;
                const RSTCAL: u32 = 1 << 3;
                const CAL: u32 = 1 << 2;
                // HAL_ADCEx_Calibration_Start: RSTCAL, then CAL, each polled
                // until the hardware clears it.
                bus.write_u32(self.base + 0x08, ADON).expect("ADON");
                for bit in [RSTCAL, CAL] {
                    bus.write_u32(self.base + 0x08, ADON | bit)
                        .expect("calibration step");
                    let mut ticks = 0;
                    while bus.read_u32(self.base + 0x08).unwrap() & bit != 0 {
                        assert!(ticks < 4096, "{}: calibration step {bit:#x} ends", self.id);
                        bus.tick_peripherals_fully();
                        ticks += 1;
                    }
                }
                // The F2/F4 SWSTART (bit 30), and the F1 SWSTART with an
                // external EXTSEL, both convert nothing on the F1 map.
                for cr2 in [ADON | (1 << 30), ADON | F1_EXTTRIG | (1 << 22)] {
                    bus.write_u32(self.base + 0x08, ADON).expect("ADON");
                    bus.write_u32(self.base + 0x08, cr2)
                        .expect("not a software start");
                    for _ in 0..64 {
                        bus.tick_peripherals_fully();
                    }
                    assert_eq!(
                        bus.read_u32(self.base).unwrap() & (1 << 1),
                        0,
                        "{}: CR2 {cr2:#x} is no F1 software start",
                        self.id
                    );
                }
            }
            Family::RaAdc { adc14 } => {
                // R_ADC_Open: ADCER.ADPRC from the resolution, right
                // justified, then ADCSR stored without ADST. Nothing
                // converts until R_ADC_ScanStart.
                let adprc = if adc14 { 3 << 1 } else { 0 };
                bus.write_u16(self.base + RA_ADCER, adprc).expect("ADCER");
                bus.write_u16(self.base, 0).expect("ADCSR single scan");
            }
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
            Family::Efr32S1Adc => {
                // CH13 on APORT1Y has no pin (Table 6.12: APORT1/2 carry
                // PC0..11 and PF0..15). A SINGLESTART there converts nothing:
                // no FIFO entry, no SINGLEDV, no IF.SINGLE.
                bus.write_u32(self.base + 0x10, s1_singlectrl(0x2D))
                    .expect("SINGLECTRL APORT1YCH13");
                bus.write_u32(self.base + 0x08, 1).expect("SINGLESTART");
                bus.tick_peripherals_fully();
                assert_eq!(
                    bus.read_u32(self.base + 0x0C).unwrap() & (1 << 16),
                    0,
                    "{}: an empty APORT cell converts nothing",
                    self.id
                );
                assert_eq!(
                    bus.read_u32(self.base + 0x38).unwrap(),
                    0,
                    "{}: IF",
                    self.id
                );
                assert_eq!(
                    bus.read_u32(self.base + 0x84).unwrap(),
                    0,
                    "{}: SINGLEFIFOCOUNT",
                    self.id
                );
            }
            Family::NxpLpadc { ctype_mode } => {
                // LPADC_Init with the default config; nothing converts
                // before CTRL.ADCEN.
                for ctrl in [LPADC_RST, 0, LPADC_RSTFIFO0, LPADC_RSTFIFO1, 0] {
                    bus.write_u32(self.base + LPADC_CTRL, ctrl).expect("CTRL");
                }
                bus.write_u32(self.base + LPADC_CFG, 1 << 28)
                    .expect("CFG.PWREN");
                bus.write_u32(self.base + LPADC_FCTRL0, 0).expect("FCTRL0");
                bus.write_u32(self.base + LPADC_CMD1, LPADC_CSCALE)
                    .expect("CMDL1");
                bus.write_u32(self.base + LPADC_TCTRL0, 1 << 24)
                    .expect("TCTRL0");
                bus.write_u32(self.base + LPADC_SWTRIG, 1).expect("SWT0");
                assert_eq!(
                    bus.read_u32(self.base + LPADC_FCTRL0).unwrap() & 0x1F,
                    0,
                    "{}: a trigger with ADCEN clear converts nothing",
                    self.id
                );
                bus.write_u32(self.base + LPADC_CTRL, LPADC_ADCEN)
                    .expect("ADCEN");
                if ctype_mode {
                    // LPADC_DoOffsetCalibration, then LPADC_DoAutoCalibration
                    // (fsl_lpadc.c with CTRL[CALOFSMODE]).
                    assert_eq!(
                        bus.read_u32(self.base + LPADC_STAT).unwrap() & LPADC_CAL_RDY,
                        0,
                        "{}: CAL_RDY before any calibration",
                        self.id
                    );
                    let ctrl = bus.read_u32(self.base + LPADC_CTRL).unwrap();
                    bus.write_u32(self.base + LPADC_CTRL, ctrl | LPADC_CALOFS)
                        .expect("CALOFS");
                    assert_ne!(
                        bus.read_u32(self.base + LPADC_STAT).unwrap() & LPADC_CAL_RDY,
                        0,
                        "{}: offset calibration sets CAL_RDY",
                        self.id
                    );
                    let ctrl = bus.read_u32(self.base + LPADC_CTRL).unwrap();
                    bus.write_u32(self.base + LPADC_CTRL, ctrl | LPADC_CAL_REQ)
                        .expect("CAL_REQ");
                    for n in 0..2u64 {
                        let gcc = bus.read_u32(self.base + LPADC_GCC0 + 4 * n).unwrap();
                        assert_ne!(gcc & LPADC_GCC_RDY, 0, "{}: GCC[{n}].RDY", self.id);
                        assert_eq!(gcc & 0x8000, 0, "{}: zero gain error", self.id);
                        let gcr = bus.read_u32(self.base + LPADC_GCR0 + 4 * n).unwrap();
                        bus.write_u32(self.base + LPADC_GCR0 + 4 * n, gcr | (1 << 24))
                            .expect("GCR.RDY");
                    }
                }
            }
            Family::KinetisAdc16 => {
                // ADC16_Init with kADC16_Resolution12or13Bit, REFSEL VREFH.
                bus.write_u32(self.base + ADC16_CFG1, ADC16_MODE_12)
                    .expect("CFG1");
                // Hardware trigger mode: no PDB is modelled, so an SC1A
                // write only stores the channel, and CAL fails with CALF.
                bus.write_u32(self.base + ADC16_SC2, ADC16_ADTRG)
                    .expect("ADTRG");
                bus.write_u32(self.base + ADC16_SC1A, u32::from(NTC_CHANNEL))
                    .expect("SC1A under ADTRG");
                assert_eq!(
                    bus.read_u32(self.base + ADC16_SC1A).unwrap() & ADC16_COCO,
                    0,
                    "{}: ADTRG = 1 needs a hardware trigger",
                    self.id
                );
                bus.write_u32(self.base + ADC16_SC3, ADC16_CAL | ADC16_CALF)
                    .expect("CAL under ADTRG");
                assert_eq!(
                    bus.read_u32(self.base + ADC16_SC3).unwrap() & (ADC16_CAL | ADC16_CALF),
                    ADC16_CALF,
                    "{}: CAL with ADTRG = 1 sets CALF",
                    self.id
                );
                // ADC16_DoAutoCalibration in software-trigger mode.
                bus.write_u32(self.base + ADC16_SC2, 0).expect("SC2");
                bus.write_u32(self.base + ADC16_SC3, ADC16_CAL | ADC16_CALF)
                    .expect("SC3.CAL");
                assert_ne!(
                    bus.read_u32(self.base + ADC16_SC1A).unwrap() & ADC16_COCO,
                    0,
                    "{}: calibration ends with SC1A.COCO",
                    self.id
                );
                assert_eq!(
                    bus.read_u32(self.base + ADC16_SC3).unwrap() & (ADC16_CAL | ADC16_CALF),
                    0,
                    "{}: calibration passed",
                    self.id
                );
                let _ = bus.read_u32(self.base + ADC16_RA).expect("dummy R[0]");
                let plus: u32 = (0x38..=0x4C)
                    .step_by(4)
                    .map(|o| bus.read_u32(self.base + o).unwrap())
                    .sum();
                bus.write_u32(self.base + ADC16_PG, 0x8000 | (plus >> 1))
                    .expect("PG");
                assert_eq!(
                    bus.read_u32(self.base + ADC16_OFS).unwrap(),
                    0,
                    "{}: ideal calibration offset",
                    self.id
                );
            }
            Family::Msp430Adc12 { adc12_a } => {
                // SLAU049 single-conversion example: ADC12ON + SHT0_2, then
                // SHP. Without ENC an ADC12SC converts nothing.
                let (ctl0, ctl1, ifg, _, _) = msp430_adc12_regs(adc12_a);
                bus.write_u16(self.base + ctl0, MSP430_SHT0_2 | MSP430_ADC12ON)
                    .expect("ADC12CTL0");
                bus.write_u16(self.base + ctl1, MSP430_SHP)
                    .expect("ADC12CTL1");
                bus.write_u16(
                    self.base + ctl0,
                    MSP430_SHT0_2 | MSP430_ADC12ON | MSP430_ADC12SC,
                )
                .expect("ADC12SC without ENC");
                assert_eq!(
                    bus.read_u16(self.base + ifg).unwrap(),
                    0,
                    "{}: ADC12SC without ENC converts nothing",
                    self.id
                );
            }
            Family::Efm32S0Adc => {
                // INPUTSEL = TEMP is an input this model does not convert: a
                // SINGLESTART there lands no result, no SINGLEDV, no IF.
                bus.write_u32(self.base + 0x0C, s0_singlectrl(8))
                    .expect("SINGLECTRL TEMP");
                bus.write_u32(self.base + 0x04, 1).expect("SINGLESTART");
                bus.tick_peripherals_fully();
                assert_eq!(
                    bus.read_u32(self.base + 0x08).unwrap() & (1 << 16),
                    0,
                    "{}: the temperature sensor converts nothing",
                    self.id
                );
                assert_eq!(
                    bus.read_u32(self.base + 0x18).unwrap(),
                    0,
                    "{}: IF",
                    self.id
                );
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
            Family::Samd51 => {
                // Arduino SAMD core pinPeripheral + analogRead: hand
                // PA05..PA07 to function B (PMUX[2] odd nibble, PMUX[3] both
                // nibbles, PINCFG.PMUXEN), 12-bit right-adjusted, INTVCC1,
                // MUXPOS against GND, CTRLA.ENABLE.
                bus.write_u8(SAMD51_PORT_A + 0x32, 0x10)
                    .expect("PMUX[2] PA05 = B");
                bus.write_u8(SAMD51_PORT_A + 0x33, 0x11)
                    .expect("PMUX[3] PA06/PA07 = B");
                for pad in [NTC_CHANNEL, DIODE_CHANNEL, IDLE_CHANNEL] {
                    bus.write_u8(SAMD51_PORT_A + 0x40 + u64::from(pad), 1)
                        .expect("PINCFG.PMUXEN");
                }
                bus.write_u16(self.base + 0x06, 0).expect("CTRLB 12-bit");
                bus.write_u8(self.base + 0x08, 3).expect("REFCTRL INTVCC1");
                bus.write_u16(self.base + 0x04, 0x1800 | u16::from(NTC_CHANNEL))
                    .expect("INPUTCTRL");
                bus.write_u16(self.base, 1 << 1).expect("CTRLA.ENABLE");
                // Both clocks are off out of reset. A START now is held, not
                // dropped: it completes only once the APB clock
                // (MCLK.APBDMASK) and the generic clock (GCLK PCHCTRL.CHEN)
                // both run.
                let (apbd_bit, gclk_channel) = self.samd51_clocks();
                let resrdy = |bus: &SystemBus| bus.read_u8(self.base + 0x2E).unwrap() & 1;
                bus.write_u8(self.base + 0x14, 1 << 1)
                    .expect("SWTRIG.START");
                bus.tick_peripherals_fully();
                assert_eq!(resrdy(bus), 0, "{}: no clocks, no conversion", self.id);
                bus.write_u32(SAMD51_GCLK_PCHCTRL0 + 4 * gclk_channel, 1 << 6)
                    .expect("PCHCTRL GEN0 | CHEN");
                bus.tick_peripherals_fully();
                assert_eq!(
                    resrdy(bus),
                    0,
                    "{}: the generic clock alone does not convert",
                    self.id
                );
                bus.write_u32(SAMD51_MCLK_APBDMASK, 1 << apbd_bit)
                    .expect("MCLK.APBDMASK");
                bus.tick_peripherals_fully();
                assert_eq!(
                    resrdy(bus),
                    1,
                    "{}: the held conversion ends once both clocks run",
                    self.id
                );
                bus.write_u8(self.base + 0x2E, 1).expect("clear RESRDY");
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
            Family::NxpLpadc { ctype_mode } => {
                // Zephyr's chain: CMD1 (NTC) -> CMD2 (diode) -> CMD3 (the
                // unrouted pad), started once by trigger 0. The RT700 runs
                // it in 16-bit MODE; CSCALE is not a field there.
                let cmdl_mode = if ctype_mode {
                    LPADC_MODE16
                } else {
                    LPADC_CSCALE
                };
                let [ntc, diode] = self.routed_channels();
                let chain = [ntc, diode, self.idle_channel()];
                for (index, channel) in chain.iter().enumerate() {
                    let at = self.base + LPADC_CMD1 + 8 * index as u64;
                    let next = if index + 1 < chain.len() {
                        index as u32 + 2
                    } else {
                        0
                    };
                    bus.write_u32(at, cmdl_mode | u32::from(*channel))
                        .expect("CMDL");
                    bus.write_u32(at + 4, next << 24).expect("CMDH.NEXT");
                }
                bus.write_u32(self.base + LPADC_TCTRL0, 1 << 24)
                    .expect("TCTRL0");
                bus.write_u32(self.base + LPADC_SWTRIG, 1).expect("SWT0");
                assert_ne!(
                    bus.read_u32(self.base + LPADC_STAT).unwrap() & LPADC_RDY0,
                    0,
                    "{}: RDY0 once FIFO0 holds more than FWMARK",
                    self.id
                );
                assert_eq!(
                    bus.read_u32(self.base + LPADC_FCTRL0).unwrap() & 0x1F,
                    3,
                    "{}: one trigger runs the whole NEXT chain",
                    self.id
                );
                let mut codes = [0; 3];
                for (index, code) in codes.iter_mut().enumerate() {
                    let entry = bus.read_u32(self.base + LPADC_RESFIFO0).expect("RESFIFO0");
                    assert_ne!(entry & LPADC_VALID, 0, "{}: entry {index} valid", self.id);
                    assert_eq!(
                        (entry >> 24) & 0xF,
                        index as u32 + 1,
                        "{}: CMDSRC follows the chain",
                        self.id
                    );
                    *code = if ctype_mode {
                        // 16-bit D; its top 12 bits are the 12-bit code.
                        (entry & 0xFFFF) >> 4
                    } else {
                        assert_eq!(entry & 0x7, 0, "{}: 12-bit result in D[14:3]", self.id);
                        (entry & 0xFFFF) >> 3
                    };
                }
                assert_eq!(
                    codes[2], 0,
                    "{}: the unrouted pad converts to zero",
                    self.id
                );
                assert_eq!(
                    bus.read_u32(self.base + LPADC_RESFIFO0).unwrap(),
                    0,
                    "{}: the chain ends at CMD3",
                    self.id
                );
                [codes[0], codes[1]]
            }
            Family::Msp430Adc12 { adc12_a } => {
                // One CONSEQ_1 sequence with MSC from CSTARTADD 0: MCTL0..2
                // = NTC, diode, the unrouted pad, EOS on MCTL2. MCTL3 points
                // at the NTC too and must stay unconverted.
                let (ctl0, ctl1, ifg, mctl, mem) = msp430_adc12_regs(adc12_a);
                let [ntc, diode] = self.routed_channels();
                let on = MSP430_SHT0_2 | MSP430_ADC12ON | MSP430_MSC;
                bus.write_u16(self.base + ctl0, on).expect("clear ENC");
                bus.write_u16(self.base + ctl1, MSP430_SHP | MSP430_CONSEQ_1)
                    .expect("CONSEQ_1");
                for (slot, inch) in [ntc, diode, IDLE_CHANNEL | MSP430_EOS, ntc]
                    .into_iter()
                    .enumerate()
                {
                    bus.write_u8(self.base + mctl + slot as u64, inch)
                        .expect("ADC12MCTLx");
                }
                bus.write_u16(self.base + ifg, 0).expect("clear IFG");
                bus.write_u16(self.base + ctl0, on | MSP430_ENC | MSP430_ADC12SC)
                    .expect("ENC + ADC12SC");
                assert_eq!(
                    bus.read_u16(self.base + ifg).unwrap(),
                    0b0111,
                    "{}: the sequence stops at EOS on MCTL2",
                    self.id
                );
                let codes: Vec<u32> = (0..3)
                    .map(|slot| u32::from(bus.read_u16(self.base + mem + 2 * slot).unwrap()))
                    .collect();
                assert_eq!(
                    bus.read_u16(self.base + ifg).unwrap(),
                    0,
                    "{}: the ADC12MEMx reads clear IFGx",
                    self.id
                );
                assert_eq!(
                    codes[2], 0,
                    "{}: unrouted A{IDLE_CHANNEL} converts to zero",
                    self.id
                );
                bus.write_u16(self.base + ctl0, on).expect("clear ENC");
                [codes[0], codes[1]]
            }
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
            Family::SamAfec => {
                // One START converts the enabled set; the shared CDR returns
                // whichever channel CSELR names, and EMR.TAG puts the last
                // channel in LCDR.CHNB.
                const EMR_TAG: u32 = 1 << 24;
                const DRDY: u32 = 1 << 24;
                let set = (1 << NTC_CHANNEL) | (1 << DIODE_CHANNEL) | (1 << IDLE_CHANNEL);
                bus.write_u32(self.base + 0x08, EMR_TAG).expect("EMR.TAG");
                bus.write_u32(self.base + 0x14, set).expect("CHER");
                bus.write_u32(self.base, 1 << 1).expect("CR.START");
                assert_eq!(
                    bus.read_u32(self.base + 0x30).unwrap() & (set | DRDY),
                    set | DRDY,
                    "{}: START raises EOC for every enabled channel and DRDY",
                    self.id
                );
                assert_eq!(
                    bus.read_u32(self.base + 0x20).unwrap(),
                    u32::from(IDLE_CHANNEL) << 24,
                    "{}: LCDR.CHNB names the last channel, unrouted AD{IDLE_CHANNEL}, with code 0",
                    self.id
                );
                let mut cdr = |channel: u8| {
                    bus.write_u32(self.base + 0x64, u32::from(channel))
                        .expect("CSELR");
                    bus.read_u32(self.base + 0x68).expect("CDR")
                };
                let codes = [cdr(NTC_CHANNEL), cdr(DIODE_CHANNEL)];
                assert_eq!(
                    bus.read_u32(self.base + 0x30).unwrap() & (set | DRDY),
                    1 << IDLE_CHANNEL,
                    "{}: each CDR read clears only the CSELR channel's EOC",
                    self.id
                );
                bus.write_u32(self.base + 0x64, u32::from(IDLE_CHANNEL))
                    .expect("CSELR idle");
                bus.read_u32(self.base + 0x68).expect("drain the idle CDR");
                bus.write_u32(self.base + 0x18, set).expect("CHDR");
                bus.write_u32(self.base + 0x08, 0).expect("EMR");
                codes
            }
            Family::RaAdc { .. } => {
                let codes = self.ra_scan(bus, &[NTC_CHANNEL, DIODE_CHANNEL, IDLE_CHANNEL]);
                assert_eq!(
                    codes[2], 0,
                    "{}: unrouted AN00{IDLE_CHANNEL} converts to zero",
                    self.id
                );
                [codes[0], codes[1]]
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
            Family::Samd21 | Family::Samd51 => {
                let codes = [
                    self.convert(bus, NTC_CHANNEL),
                    self.convert(bus, DIODE_CHANNEL),
                ];
                // The ADC reads the pad only while PORT hands it over:
                // with PINCFG.PMUXEN cleared the driven NTC pad converts to
                // zero, and handing it back restores the code.
                let port_a = if matches!(self.family, Family::Samd21) {
                    SAMD21_PORT_A
                } else {
                    SAMD51_PORT_A
                };
                let pincfg = port_a + 0x40 + u64::from(NTC_CHANNEL);
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
            Family::Stm32F1 => {
                let single = [
                    self.convert(bus, NTC_CHANNEL),
                    self.convert(bus, DIODE_CHANNEL),
                ];
                let [ntc, diode] = single;
                // CR1.SCAN: both sensors convert, DR passes through the first
                // and EOC rises only once the last one lands.
                for (order, expected) in [
                    ([NTC_CHANNEL, DIODE_CHANNEL], [ntc, diode]),
                    ([DIODE_CHANNEL, NTC_CHANNEL], [diode, ntc]),
                ] {
                    assert_eq!(
                        self.scan_f1(bus, &order),
                        expected,
                        "{}: one SWSTART scans CR1.SCAN group {order:?}, EOC at its end",
                        self.id
                    );
                }
                // CR1.DISCEN over [NTC, diode, NTC], one sensor per trigger:
                // EOC only on the trigger that reaches SQ3 (RM0008 §11.3.10).
                let group = [NTC_CHANNEL, DIODE_CHANNEL, NTC_CHANNEL];
                assert_eq!(
                    self.discen_f1(bus, &group, 4),
                    vec![(ntc, false), (diode, false), (ntc, true), (ntc, false)],
                    "{}: DISCNUM=0 converts one routed sensor per SWSTART",
                    self.id
                );
                for (ranks, expected) in [
                    (vec![DIODE_CHANNEL], vec![diode]),
                    (
                        vec![NTC_CHANNEL, DIODE_CHANNEL, NTC_CHANNEL],
                        vec![ntc, diode, ntc],
                    ),
                ] {
                    assert_eq!(
                        self.injected_f1(bus, &ranks),
                        expected,
                        "{}: one JSWSTART converts injected ranks {ranks:?}",
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
            _ => self
                .routed_channels()
                .map(|channel| self.convert(bus, channel)),
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

    /// F1 `CR2` with software triggers selected for both groups, the way
    /// `HAL_ADC_Start`/`HAL_ADCEx_InjectedStart` leave it: `EXTSEL` and
    /// `JEXTSEL` = 111 (SWSTART/JSWSTART) with `EXTTRIG`/`JEXTTRIG`.
    const F1_CR2_SOFTWARE: u32 = 1 | (0b111 << 17) | F1_EXTTRIG | (0b111 << 12) | (1 << 15);

    /// Rising F1 `CR2.SWSTART` (bit 22).
    fn f1_swstart(&self, bus: &mut SystemBus) {
        bus.write_u32(self.base + 0x08, Self::F1_CR2_SOFTWARE)
            .expect("ADON, software EXTSEL");
        bus.write_u32(self.base + 0x08, Self::F1_CR2_SOFTWARE | (1 << 22))
            .expect("SWSTART");
    }

    fn f1_wait_eoc(&self, bus: &mut SystemBus, what: &str) {
        let mut ticks = 0;
        while bus.read_u32(self.base).unwrap() & (1 << 1) == 0 {
            assert!(ticks < 256, "{}: EOC after {what}", self.id);
            bus.tick_peripherals_fully();
            ticks += 1;
        }
    }

    /// One software-started F1 `CR1.SCAN` group read without DMA: each
    /// conversion lands in DR, but EOC rises only with the last
    /// (RM0008 §11.3.8). Returns DR as each conversion lands. Leaves
    /// CR1/SQR1 at 0.
    fn scan_f1(&self, bus: &mut SystemBus, channels: &[u8]) -> Vec<u32> {
        // Leave a DR that matches neither sensor (the undriven channel's
        // fallback count), so every landing shows as a change.
        self.convert(bus, IDLE_CHANNEL);
        let sqr3 = channels
            .iter()
            .enumerate()
            .fold(0, |w, (k, &ch)| w | (u32::from(ch) << (5 * k)));
        bus.write_u32(self.base + 0x34, sqr3).expect("SQR3 SQ1..");
        bus.write_u32(self.base + 0x2c, (channels.len() as u32 - 1) << 20)
            .expect("SQR1.L");
        bus.write_u32(self.base + 0x04, 1 << 8).expect("CR1.SCAN");
        bus.write_u32(self.base, 0).expect("clear SR");
        let mut landed = Vec::new();
        let mut dr = bus.read_u32(self.base + 0x4c).unwrap();
        self.f1_swstart(bus);
        for _ in 0..256 * channels.len() {
            bus.tick_peripherals_fully();
            let now = bus.read_u32(self.base + 0x4c).unwrap();
            let eoc = bus.read_u32(self.base).unwrap() & (1 << 1) != 0;
            if now != dr || eoc && landed.len() < channels.len() {
                landed.push(now);
                dr = now;
            }
            assert_eq!(
                eoc,
                landed.len() == channels.len(),
                "{}: EOC only once all {} conversions land (after {landed:?})",
                self.id,
                channels.len()
            );
            if eoc {
                break;
            }
        }
        bus.write_u32(self.base, 0).expect("clear EOC");
        bus.write_u32(self.base + 0x04, 0).expect("restore CR1");
        bus.write_u32(self.base + 0x2c, 0).expect("restore SQR1");
        landed
    }

    /// F1 `CR1.DISCEN` with `DISCNUM` = 0 over `channels`: each SWSTART
    /// converts the next one into DR. Returns `(DR, EOC)` per trigger.
    /// Leaves CR1/SQR1 at 0.
    fn discen_f1(&self, bus: &mut SystemBus, channels: &[u8], triggers: usize) -> Vec<(u32, bool)> {
        let sqr3 = channels
            .iter()
            .enumerate()
            .fold(0, |w, (k, &ch)| w | (u32::from(ch) << (5 * k)));
        bus.write_u32(self.base + 0x34, sqr3).expect("SQR3 SQ1..");
        bus.write_u32(self.base + 0x2c, (channels.len() as u32 - 1) << 20)
            .expect("SQR1.L");
        bus.write_u32(self.base + 0x04, (1 << 8) | (1 << 11))
            .expect("CR1.DISCEN");
        let out = (0..triggers)
            .map(|_| {
                bus.write_u32(self.base, 0).expect("clear SR");
                self.f1_swstart(bus);
                for _ in 0..256 {
                    bus.tick_peripherals_fully();
                }
                (
                    bus.read_u32(self.base + 0x4c).expect("read DR"),
                    bus.read_u32(self.base).unwrap() & (1 << 1) != 0,
                )
            })
            .collect();
        bus.write_u32(self.base + 0x04, 0).expect("restore CR1");
        bus.write_u32(self.base + 0x2c, 0).expect("restore SQR1");
        out
    }

    /// F1 injected group: the F4 `JSQR` packing (rank r of n in `JSQ(4 - n +
    /// r)`), started by `CR2.JSWSTART` (bit 21) with `JEXTSEL` = 111
    /// (RM0008 §11.12.3). Waits for `JEOC` and returns `JDR1..JDRn`.
    fn injected_f1(&self, bus: &mut SystemBus, channels: &[u8]) -> Vec<u32> {
        let n = channels.len();
        let jsqr = channels
            .iter()
            .enumerate()
            .fold(((n as u32) - 1) << 20, |w, (r, &ch)| {
                w | (u32::from(ch) << (5 * (4 - n + r)))
            });
        bus.write_u32(self.base + 0x38, jsqr).expect("JSQR");
        bus.write_u32(self.base, 0).expect("clear SR");
        bus.write_u32(self.base + 0x08, Self::F1_CR2_SOFTWARE)
            .expect("ADON");
        bus.write_u32(self.base + 0x08, Self::F1_CR2_SOFTWARE | (1 << 21))
            .expect("JSWSTART");
        let mut ticks = 0;
        while bus.read_u32(self.base).unwrap() & (1 << 2) == 0 {
            assert!(ticks < 256 * n, "{}: JEOC", self.id);
            bus.tick_peripherals_fully();
            ticks += 1;
        }
        (0..n)
            .map(|k| bus.read_u32(self.base + 0x3c + 4 * k as u64).expect("JDRx"))
            .collect()
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
            Family::NxpLpadc { .. } => {
                // fsl_lpadc polling example: one command, trigger 0, one
                // RESFIFO0 read, D >> 3 (standard resolution on both IPs).
                bus.write_u32(self.base + LPADC_CMD1, LPADC_CSCALE | u32::from(channel))
                    .expect("CMDL1");
                bus.write_u32(self.base + LPADC_CMD1 + 4, 0).expect("CMDH1");
                bus.write_u32(self.base + LPADC_TCTRL0, 1 << 24)
                    .expect("TCTRL0");
                bus.write_u32(self.base + LPADC_SWTRIG, 1).expect("SWT0");
                let entry = bus.read_u32(self.base + LPADC_RESFIFO0).expect("RESFIFO0");
                assert_ne!(entry & LPADC_VALID, 0, "{}: CH{channel}A converts", self.id);
                (entry & 0xFFFF) >> 3
            }
            Family::KinetisAdc16 => {
                // ADC16_SetChannelConfig(SC1A), poll COCO,
                // ADC16_GetChannelConversionValue; the RA read clears COCO.
                bus.write_u32(self.base + ADC16_SC1A, u32::from(channel))
                    .expect("SC1A.ADCH");
                assert_ne!(
                    bus.read_u32(self.base + ADC16_SC1A).unwrap() & ADC16_COCO,
                    0,
                    "{}: AD{channel} sets COCO",
                    self.id
                );
                let code = bus.read_u32(self.base + ADC16_RA).expect("RA");
                assert_eq!(
                    bus.read_u32(self.base + ADC16_SC1A).unwrap() & ADC16_COCO,
                    0,
                    "{}: the RA read clears COCO",
                    self.id
                );
                code
            }
            Family::Msp430Adc12 { adc12_a } => {
                // Single channel, single conversion at CSTARTADD = channel,
                // MCTLx = INCH with SREF_0; poll IFGx, read ADC12MEMx.
                let (ctl0, ctl1, ifg, mctl, mem) = msp430_adc12_regs(adc12_a);
                let slot = u64::from(channel);
                let on = MSP430_SHT0_2 | MSP430_ADC12ON;
                bus.write_u16(self.base + ctl0, on).expect("clear ENC");
                bus.write_u16(self.base + ctl1, (u16::from(channel) << 12) | MSP430_SHP)
                    .expect("CSTARTADD");
                bus.write_u8(self.base + mctl + slot, channel)
                    .expect("ADC12MCTLx");
                bus.write_u16(self.base + ctl0, on | MSP430_ENC | MSP430_ADC12SC)
                    .expect("ENC + ADC12SC");
                assert_ne!(
                    bus.read_u16(self.base + ifg).unwrap() & (1 << channel),
                    0,
                    "{}: A{channel} sets ADC12IFG{channel}",
                    self.id
                );
                let code = bus.read_u16(self.base + mem + 2 * slot).expect("ADC12MEMx");
                bus.write_u16(self.base + ctl0, on).expect("clear ENC");
                u32::from(code)
            }
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
            Family::Stm32F1 => {
                bus.write_u32(self.base + 0x34, u32::from(channel))
                    .expect("select SQR3 SQ1");
                bus.write_u32(self.base, 0).expect("clear SR");
                self.f1_swstart(bus);
                self.f1_wait_eoc(bus, "conversion");
                bus.read_u32(self.base + 0x4c).expect("read DR")
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
            Family::SamAfec => {
                // Zephyr adc_sam_start_conversion + adc_sam_isr, polled.
                bus.write_u32(self.base + 0x14, 1 << channel).expect("CHER");
                bus.write_u32(self.base, 1 << 1).expect("CR.START");
                bus.write_u32(self.base + 0x64, u32::from(channel))
                    .expect("CSELR");
                let code = bus.read_u32(self.base + 0x68).expect("CDR");
                bus.write_u32(self.base + 0x18, 1 << channel).expect("CHDR");
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
            Family::Samd51 => {
                // analogRead: MUXPOS against GND (0x18), SWTRIG.START,
                // RESRDY on the next tick, read RESULT, write RESRDY clear.
                const RESRDY: u8 = 1;
                bus.write_u16(self.base + 0x04, 0x1800 | u16::from(channel))
                    .expect("INPUTCTRL");
                bus.write_u8(self.base + 0x14, 1 << 1)
                    .expect("SWTRIG.START");
                assert_eq!(
                    bus.read_u8(self.base + 0x2E).unwrap() & RESRDY,
                    0,
                    "{}: START clears RESRDY until the conversion ends",
                    self.id
                );
                bus.tick_peripherals_fully();
                assert_ne!(
                    bus.read_u8(self.base + 0x2E).unwrap() & RESRDY,
                    0,
                    "{}: the conversion raises INTFLAG.RESRDY",
                    self.id
                );
                let code = u32::from(bus.read_u16(self.base + 0x40).expect("RESULT"));
                bus.write_u8(self.base + 0x2E, RESRDY)
                    .expect("clear RESRDY");
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
            Family::Efr32S1Adc => {
                // em_adc ADC_InitSingle + ADC_Start(adcStartSingle), poll
                // STATUS.SINGLEDV, ADC_DataSingleGet. Channel n is PAn,
                // reached once through APORT3 and once through APORT4: the
                // two buses carry the same pad, so the codes must agree.
                const SINGLEDV: u32 = 1 << 16;
                const IF_SINGLE: u32 = 1;
                let (cmd, status, iflag, ifc) = (
                    self.base + 0x08,
                    self.base + 0x0C,
                    self.base + 0x38,
                    self.base + 0x40,
                );
                let mut codes = [0u32; 2];
                for (slot, bus_no) in [3u32, 4].into_iter().enumerate() {
                    bus.write_u32(
                        self.base + 0x10,
                        s1_singlectrl(s1_possel_pa(bus_no, channel)),
                    )
                    .expect("SINGLECTRL");
                    bus.write_u32(ifc, IF_SINGLE).expect("IFC SINGLE");
                    bus.write_u32(cmd, 1).expect("SINGLESTART");
                    bus.tick_peripherals_fully();
                    assert_ne!(
                        bus.read_u32(status).unwrap() & SINGLEDV,
                        0,
                        "{}: APORT{bus_no} conversion raises SINGLEDV",
                        self.id
                    );
                    assert_ne!(
                        bus.read_u32(iflag).unwrap() & IF_SINGLE,
                        0,
                        "{}: and IF.SINGLE",
                        self.id
                    );
                    codes[slot] = bus.read_u32(self.base + 0x48).expect("SINGLEDATA");
                    assert_eq!(
                        bus.read_u32(self.base + 0x84).unwrap(),
                        0,
                        "{}: SINGLEDATA pops the only result",
                        self.id
                    );
                }
                assert_eq!(
                    codes[0], codes[1],
                    "{}: APORT3 and APORT4 reach PA{channel}",
                    self.id
                );
                codes[0]
            }
            Family::Efm32S0Adc => {
                // em_adc ADC_InitSingle + ADC_Start(adcStartSingle), poll
                // STATUS.SINGLEDV, ADC_DataSingleGet. The SINGLEDATAP peek
                // leaves SINGLEDV set; the SINGLEDATA read clears it (SVD
                // readAction modifyExternal).
                const SINGLEDV: u32 = 1 << 16;
                const IF_SINGLE: u32 = 1;
                let (cmd, status, iflag, ifc) = (
                    self.base + 0x04,
                    self.base + 0x08,
                    self.base + 0x18,
                    self.base + 0x20,
                );
                bus.write_u32(self.base + 0x0C, s0_singlectrl(u32::from(channel)))
                    .expect("SINGLECTRL");
                bus.write_u32(ifc, IF_SINGLE).expect("IFC SINGLE");
                bus.write_u32(cmd, 1).expect("SINGLESTART");
                bus.tick_peripherals_fully();
                assert_ne!(
                    bus.read_u32(status).unwrap() & SINGLEDV,
                    0,
                    "{}: CH{channel} conversion raises SINGLEDV",
                    self.id
                );
                assert_ne!(
                    bus.read_u32(iflag).unwrap() & IF_SINGLE,
                    0,
                    "{}: and IF.SINGLE",
                    self.id
                );
                let peeked = bus.read_u32(self.base + 0x2C).expect("SINGLEDATAP");
                assert_ne!(
                    bus.read_u32(status).unwrap() & SINGLEDV,
                    0,
                    "{}: the SINGLEDATAP peek keeps SINGLEDV",
                    self.id
                );
                let code = bus.read_u32(self.base + 0x24).expect("SINGLEDATA");
                assert_eq!(code, peeked, "{}: peek and read agree", self.id);
                assert_eq!(
                    bus.read_u32(status).unwrap() & SINGLEDV,
                    0,
                    "{}: the SINGLEDATA read clears SINGLEDV",
                    self.id
                );
                code
            }
            Family::RaAdc { .. } => self.ra_scan(bus, &[channel])[0],
            Family::Saadc { .. } => unreachable!("SAADC converts as a scan"),
        }
    }

    /// One FSP software scan: select `channels` in `ADANSA0`, write
    /// `ADCSR.ADST`, and poll `ADST` until the scan ends. The scan takes
    /// one nominal microsecond per channel, so `ADST` must still be set one
    /// cycle in, then the `ADDRn` reads.
    fn ra_scan(&self, bus: &mut SystemBus, channels: &[u8]) -> Vec<u32> {
        const ADST: u16 = 1 << 15;
        let mask = channels.iter().fold(0u16, |m, &ch| m | (1 << ch));
        let addr = |ch: u8| self.base + 0x20 + 2 * u64::from(ch);
        bus.write_u16(self.base + 0x04, mask).expect("ADANSA0");
        bus.write_u16(self.base, ADST).expect("ADCSR.ADST");
        let start = bus.current_cycle;
        bus.set_current_cycle(start + 1);
        bus.tick_peripherals_fully();
        assert_ne!(
            bus.read_u16(self.base).unwrap() & ADST,
            0,
            "{}: ADST stays set while the scan runs",
            self.id
        );
        // A 3-channel scan is 3 us; publish 10 us at the fastest ICLK
        // declared (480 MHz).
        bus.set_current_cycle(start + 4_800);
        bus.tick_peripherals_fully();
        assert_eq!(
            bus.read_u16(self.base).unwrap() & ADST,
            0,
            "{}: ADST clears at scan end",
            self.id
        );
        bus.write_u16(self.base + 0x04, 0).expect("clear ADANSA0");
        channels
            .iter()
            .map(|&ch| u32::from(bus.read_u16(addr(ch)).expect("ADDRn")))
            .collect()
    }

    /// The shared STM32 model also exposes what it holds per channel, so an
    /// unrouted channel can be shown undriven rather than merely "some code".
    fn held_count(&self, bus: &mut SystemBus, channel: u8) -> Option<u16> {
        if !matches!(
            self.family,
            Family::Stm32F1
                | Family::Stm32F4
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
/// gives its ADC: F0/L0 `CHSELR` (RM0091, RM0367), F1 (RM0008), F2/F4/F7 (RM0033, RM0090,
/// RM0410), H7 (RM0433), L4/L5 (RM0351, RM0438), WBA ADC4 (RM0493).
fn assert_stm32_series_map(board: &str, converter: &Converter) {
    let Some(series) = board.strip_prefix("stm32") else {
        return;
    };
    let expected = match series.get(..2) {
        Some("f0" | "l0") => "Stm32V2",
        Some("f1") => "Stm32F1",
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

/// The RA type must match the part's converter: only the RA4M1 has the
/// 14-bit ADC140 (FSP `BSP_FEATURE_ADC_MAX_RESOLUTION_BITS` 14); RA2E1,
/// RA2L1, RA6M5 and RA8M1 report 12. The Arduino UNO R4 Minima carries an
/// R7FA4M1AB3CFM.
fn assert_ra_resolution(board: &str, converter: &Converter) {
    let Family::RaAdc { adc14 } = converter.family else {
        return;
    };
    let ra4m1 =
        board.contains("r7fa4m1") || board.contains("ra4m1") || board == "arduino_uno_r4_minima";
    assert_eq!(
        adc14, ra4m1,
        "{board}:{} declares the wrong RA ADC resolution",
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
        if let Some(idle) = converter.held_count(&mut bus, converter.idle_channel()) {
            assert_eq!(idle, 0xFFFF, "{tag}: unrouted channel stays undriven");
        }
        if matches!(
            converter.family,
            Family::Imxrt
                | Family::Sam4s
                | Family::SamAfec
                | Family::Apollo4
                | Family::Cc2538
                | Family::Caes
                | Family::Samd21
                | Family::Samd51
                | Family::Efr32Iadc
                | Family::Efr32S1Adc
                | Family::Efm32S0Adc
                | Family::Msp430Adc12 { .. }
                | Family::KinetisAdc16
                | Family::NxpLpadc { .. }
                | Family::RaAdc { .. }
        ) {
            assert_eq!(
                converter.convert(&mut bus, converter.idle_channel()),
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
                    assert_ra_resolution(&board, converter);
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
        instances, 138,
        "ADC instances exercised; update docs/mixed_signal_board_coverage.md"
    );
    assert_eq!(
        cpu_owned, 34,
        "simavr profiles with a CPU-owned AVR ADC or comparator, driven in cosim_onboarding_avr_analog.rs"
    );
    assert_eq!(
        without_adc, 101,
        "onboarding profiles that declare no modelled ADC"
    );
}
