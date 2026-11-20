// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.
//
// ── Architectural separation ────────────────────────────────────────────────
// Family-specific register files live in the `AdcRegs` enum. F1/F4 share the
// legacy CR/SQR storage, while only F4 currently models JSQR/JDR injected
// sequences; L4/H7/U5 have distinct ISR/IER/CR/CFGR/… register files. The data
// register `dr`, legacy status `sr` (also poked by the WASM injection bridge),
// conversion engine and per-channel analog inputs are shared where semantics
// match.

use crate::{CycleClock, Peripheral, PeripheralTickResult, SimResult};
use std::any::Any;
use std::cell::Cell;
use std::str::FromStr;

// ── Modeled internal source (L4/V2 path) ────────────────────────────────────
// An unbound L4 channel retains the fixed internal fallback source:
// V(IN) = 3.0 V against V(REF+) = 3.3 V. An injected channel instead converts
// its held millivolts against the current reference. The fallback's 12-bit
// code is round-down(3.0/3.3 * 4096) = 3723; narrower resolutions drop LSBs
// (>> (12 - bits)), exactly as the SAR core truncates. The code is therefore
// derived + deterministic and CHANGES with CFGR.RES — a real conversion, not a
// constant.
const STM32_ADC_REF12: u32 = (3000 * 4096) / 3300; // 3723

// STM32G0/C0 ADC bits (ST `stm32g071xx.h`, identical in `stm32c031xx.h`).
const G0_ISR_ADRDY: u32 = 1 << 0;
const G0_ISR_EOC: u32 = 1 << 2;
const G0_ISR_EOS: u32 = 1 << 3;
const G0_ISR_EOCAL: u32 = 1 << 11;
const G0_ISR_CCRDY: u32 = 1 << 13;
const G0_CR_ADEN: u32 = 1 << 0;
const G0_CR_ADDIS: u32 = 1 << 1;
const G0_CR_ADSTART: u32 = 1 << 2;
const G0_CR_ADSTP: u32 = 1 << 4;
const G0_CR_ADVREGEN: u32 = 1 << 28;
const G0_CR_ADCAL: u32 = 1 << 31;
const G0_CFGR1_SCANDIR: u32 = 1 << 2;
const G0_CFGR1_ALIGN: u32 = 1 << 5;
const G0_CFGR1_CONT: u32 = 1 << 13;
const G0_CFGR1_CHSELRMOD: u32 = 1 << 21;

/// CFGR.RES (bits [4:3]) → conversion bit-width. 0b00=12, 01=10, 10=8, 11=6.
fn l4_resolution_bits(cfgr: u32) -> u32 {
    match (cfgr >> 3) & 0x3 {
        0 => 12,
        1 => 10,
        2 => 8,
        _ => 6,
    }
}

/// F2/F4 `ADC_CR1.RES` (bits [25:24]) → conversion bit-width, RM0383 §11.12.2:
/// 0b00=12, 01=10, 10=8, 11=6. The F1 has no RES field (always 12 bits).
fn f4_resolution_bits(cr1: u32) -> u32 {
    match (cr1 >> 24) & 0x3 {
        0 => 12,
        1 => 10,
        2 => 8,
        _ => 6,
    }
}

/// Converted code for the fixed internal source at the given bit-width.
fn l4_adc_code(bits: u32) -> u32 {
    STM32_ADC_REF12 >> (12 - bits)
}

/// H7 `ADC_CFGR.RES[4:2]` → bit width, per the STM32H735 SVD's own enumerated
/// values (RM0468 §26.4.24). This is a THREE-bit field with a sparse encoding
/// and is not the L4's two-bit one at a different offset:
///
/// | RES | width |                     | RES | width |
/// |-----|-------|                     |-----|-------|
/// | 000 | 16    |                     | 011 | 10    |
/// | 001 | 14 (legacy, unoptimised)    | 101 | 14    |
/// | 010 | 12 (legacy, unoptimised)    | 110 | 12    |
/// |     |       |                     | 111 | 8     |
///
/// `100` is NOT defined by the SVD. Silicon behaviour for it is unspecified, so
/// it falls to the 16-bit reset width rather than being invented.
fn h7_resolution_bits(cfgr: u32) -> u32 {
    match (cfgr >> 2) & 0x7 {
        0b000 => 16,
        0b001 | 0b101 => 14,
        0b010 | 0b110 => 12,
        0b011 => 10,
        0b111 => 8,
        _ => 16,
    }
}

/// Converted code for the fixed internal source at a width wider or narrower
/// than the 12-bit reference count. Shared by the H7 (native 16 bits) and U5
/// (native 14 bits) engines; the reference constant is a 12-bit count, so
/// widen rather than narrow above 12.
fn scaled_adc_code(bits: u32) -> u32 {
    if bits >= 12 {
        (STM32_ADC_REF12 << (bits - 12)) & ((1 << bits) - 1)
    } else {
        STM32_ADC_REF12 >> (12 - bits)
    }
}

/// Scale an externally held 12-bit input to a wider or narrower resolution.
fn scaled_input_code(count12: u32, bits: u32) -> u32 {
    if bits >= 12 {
        (count12 << (bits - 12)) & ((1 << bits) - 1)
    } else {
        count12 >> (12 - bits)
    }
}

/// U5 `ADC_CFGR1.RES[3:2]` → bit width, straight from the vendored stm32u575
/// SVD's enumerated values: 0b00=14, 0b01=12, 0b10=10, 0b11=8. The U5 is a
/// 14-bit converter, so the reset encoding is 14, not the H7's 16 or the
/// L4's 12.
fn u5_resolution_bits(cfgr: u32) -> u32 {
    match (cfgr >> 2) & 0x3 {
        0 => 14,
        1 => 12,
        2 => 10,
        _ => 8,
    }
}

/// Analog input channels on the widest modelled family (STM32H7 ADC1, 0..=19).
const MAX_CHANNELS: usize = 20;
const NO_ADC_INPUT: u16 = u16::MAX;

/// The input voltage and reference latched at sample-and-hold acquisition.
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize)]
struct HeldAdcInput {
    millivolts: f64,
    reference_millivolts: f64,
}

// ── STM32F1 ADC_CR2 bits (RM0008 §11.12.3 / stm32f1xx.h) ────────────────────
// Independently checked against ST headers — NOT against this model's older
// (wrong) bit-30 SWSTART constant. F2/F4 reuse the same SR/CR1/CR2/SQR *offsets*
// but place SWSTART at bit 30; see `F4_CR2_SWSTART` and `AdcRegisterLayout::Stm32F4`.
const F1_CR2_ADON: u32 = 1 << 0;
const F1_CR2_CONT: u32 = 1 << 1;
const F1_CR2_CAL: u32 = 1 << 2;
const F1_CR2_RSTCAL: u32 = 1 << 3;
/// Software trigger selected when EXTSEL[2:0] == 0b111 (bits 19:17).
const F1_CR2_EXTSEL: u32 = 0b111 << 17;
const F1_CR2_EXTTRIG: u32 = 1 << 20;
const F1_CR2_SWSTART: u32 = 1 << 22;
const F1_CR1_EOCIE: u32 = 1 << 5;
/// ADC_CR2.TSVREFE (bit 23, RM0008 §11.12.3): connects the temperature sensor
/// to ADC1_IN16 and V_REFINT to ADC1_IN17. Clear (the reset state), both
/// internal channels are disconnected.
const F1_CR2_TSVREFE: u32 = 1 << 23;

// ── STM32F1 internal channels (DS5319 Rev 20) ───────────────────────────────
/// Temperature sensor V_SENSE at 25 °C, typical: 1.43 V (DS5319 Table 50
/// "TS characteristics", V25 min 1.34 / typ 1.43 / max 1.52 V).
const F1_TS_V25_MV: f64 = 1430.0;
/// Temperature sensor average slope, typical: 4.3 mV/°C (DS5319 Table 50,
/// Avg_Slope min 4.0 / typ 4.3 / max 4.6 mV/°C). V_SENSE FALLS as temperature
/// rises: RM0008 §11.10 gives T = (V25 − V_SENSE) / Avg_Slope + 25.
const F1_TS_AVG_SLOPE_MV_PER_C: f64 = 4.3;
/// Internal reference voltage, typical: 1.20 V (DS5319 Table 13 "Embedded
/// internal reference voltage", V_REFINT min 1.16 / typ 1.20 / max 1.24 V).
const F1_VREFINT_MV: f64 = 1200.0;
/// Default positive full-scale reference until a board route drives VDDA.
const ADC_VREF_MV: f64 = 3300.0;
/// Ambient temperature an F1 temperature sensor reads before anything sets
/// one, in °C.
pub const DEFAULT_AMBIENT_TEMPERATURE_C: f64 = 25.0;

/// STM32F2/F4 ADC_CR2.SWSTART (RM0090) — same legacy register *block* as F1,
/// different bit. Kept so F4 board bindings are not silently retargeted to F1.
const F4_CR2_SWSTART: u32 = 1 << 30;
/// F4 `ADC_CR2.JSWSTART`: software-start the injected group.
const F4_CR2_JSWSTART: u32 = 1 << 22;
/// F4 `ADC_CR2.EOCS`: regular overrun detection is enabled when set.
const F4_CR2_EOCS: u32 = 1 << 10;
/// F4 `ADC_CR1.SCAN`: advance through the regular sequence after each conversion.
const F4_CR1_SCAN: u32 = 1 << 8;
/// F4 `ADC_CR1.JAUTO`: automatic injected conversion after a regular group.
const F4_CR1_JAUTO: u32 = 1 << 10;
const F4_SR_EOC: u32 = 1 << 1;
const F4_SR_OVR: u32 = 1 << 5;
#[cfg(test)]
const F4_CR1_EOCIE: u32 = 1 << 5;
const F4_CR1_JEOCIE: u32 = 1 << 7;
const F4_CR1_OVRIE: u32 = 1 << 26;
const F4_SR_JEOC: u32 = 1 << 2;

/// Approximate F1 RSTCAL completion latency in model cycles (not host time).
/// Silicon clears RSTCAL after a short internal reset; a fixed 2-cycle delay is
/// enough for HAL spin-loops (`while (CR2 & RSTCAL)`) to observe completion.
/// Explicitly an approximation — not cycle-accurate ADC-clock timing.
const F1_RSTCAL_CYCLES: u32 = 2;
/// Approximate F1 CAL completion latency in model cycles. ST calibration takes
/// many ADC clocks; a short fixed delay unblocks HAL polls without claiming
/// silicon-accurate duration.
const F1_CAL_CYCLES: u32 = 14;

/// Scheduler / walk event tokens for the F1 side-band. A conversion wake is
/// `F1_EVT_CONVERT | generation`: the generation changes whenever a write
/// starts or cancels a conversion, so a wake armed for a superseded conversion
/// fires stale and does nothing.
const F1_EVT_CONVERT: u32 = 0x8000_0000;
const F1_EVT_GEN_MASK: u32 = 0x00FF_FFFF;
const F1_EVT_RSTCAL: u32 = 1;
const F1_EVT_CAL: u32 = 2;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AdcRegisterLayout {
    #[default]
    Stm32F1,
    /// STM32F2/F4 legacy ADC block: same SR/CR1/CR2/SQR offsets as F1, but
    /// `CR2.SWSTART` is bit 30 (RM0090) and there is no F1-style CAL/RSTCAL.
    /// Kept as a distinct layout so fixing F1 bit 22 does not retarget F4 boards
    /// that share the old `"adc"` default profile.
    Stm32F4,
    Stm32L4,
    /// STM32H7 (RM0468). A genuinely different block from the L4 — see
    /// [`H7AdcRegs`] for what diverges and why the alias that used to point
    /// `"h7"` at [`Self::Stm32L4`] was wrong.
    Stm32H7,
    /// STM32H5 (RM0481). Register-for-register the L4 map — every offset
    /// identical, `CFGR.RES` two bits at [4:3], no `LDORDY` — plus one extra
    /// `ADC_OR`. It is NOT the H7 block, whose map differs in eight registers
    /// and encodes `RES` in three bits at [4:2].
    ///
    /// It differs from the L4 in exactly one modelled respect: the H5 firmware
    /// this simulates configures the ADC kernel clock, so a calibration
    /// request COMPLETES and `ADCAL` self-clears. The plain L4 profile keeps
    /// the latched behaviour, which is what NUCLEO-L476RG silicon shows when
    /// only `AHB2ENR.ADCEN` is set and `CCIPR` is left alone — calibration
    /// cannot run without a clock, so the bit never clears. Two different
    /// firmware situations, not two different silicon behaviours; when the
    /// kernel clock is modelled these collapse back into one layout.
    Stm32H5,
    /// STM32U5 (RM0456). A 14-bit ADC with its own register map, taken from the
    /// vendored `tests/fixtures/real_world/stm32u575.svd`: `CFGR1.RES` is two
    /// bits at [3:2] encoding 14/12/10/8; `PCSEL` @ 0x1C; the LTR/HTR watchdog
    /// pairs at 0xA8..0xBC (NOT the H7's 0x20/0x24); `HTRx` reset to the 25-bit
    /// 0x01FF_FFFF; `GCOMP` @ 0x70 and `CALFACT2` @ 0xC8. The H7 map differs in
    /// all of those, so the alias is a separate layout rather than a flavour.
    Stm32U5,
    /// STM32G0/C0 (RM0444 / RM0490; offsets from ST's `stm32g071xx.h` and
    /// `stm32c031xx.h`, which agree register for register). The L4 offsets
    /// for ISR/IER/CR/CFGR1/CFGR2/DR, but NOT the L4 sequencer: there are no
    /// SQRx, the regular group is `CHSELR` @ 0x28 (a channel bitmap, or eight
    /// 4-bit ranks under `CFGR1.CHSELRMOD`), one `SMPR`, `AWDxTR` pairs at
    /// 0x20/0x24/0x2C, `CALFACT` @ 0xB4, and the `ISR.EOCAL`/`ISR.CCRDY`
    /// flags. No `DEEPPWD`: CR resets to 0.
    ///
    /// The STM32L0 block (RM0367 §14.12; `stm32l073` debug schema) is a
    /// subset at the same offsets: ISR/IER/CR/CFGR1/CFGR2/SMPR, one `TR` at
    /// 0x20 (reset 0x0FFF_0000), the `CHSELR` bitmap at 0x28, `DR`, `CALFACT`
    /// and `CCR` at 0x308. It has no `CHSELRMOD`, `CCRDY` or second and
    /// third watchdogs, so L0 firmware never touches them, and `stm32l0`
    /// shares this layout.
    Stm32G0,
}

impl FromStr for AdcRegisterLayout {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let v = value.trim().to_ascii_lowercase();
        match v.as_str() {
            "stm32f1" | "f1" => Ok(Self::Stm32F1),
            // "legacy" kept as F2/F4 bit-30 SWSTART: the pre-fix shared layout
            // answered software-start on bit 30. F1 callers must name `stm32f1`.
            "stm32f4" | "f4" | "stm32f2" | "f2" | "legacy" => Ok(Self::Stm32F4),
            // `stm32h7`/`h7` used to land here. It no longer does: the H7 block
            // has PCSEL, LTR/HTR watchdog pairs instead of TR1..TR3, CALFACT2,
            // a 3-bit RES at a different offset with a different encoding, and
            // a 16-bit DR. Answering an H7 on the L4 map returned plausible
            // values from the wrong registers.
            "stm32l4" | "l4" | "stm32f7" | "f7" => Ok(Self::Stm32L4),
            // `stm32g0`/`g0` used to alias the L4. Converting on the L4 map
            // read the G0 sequencer from SQR1 (which the G0 does not have),
            // never completed ADCAL or ADDIS, and stock analogRead() on the
            // NUCLEO-G071RB returned 0.
            "stm32g0" | "g0" | "stm32c0" | "c0" | "stm32l0" | "l0" => Ok(Self::Stm32G0),
            "stm32h7" | "h7" => Ok(Self::Stm32H7),
            "stm32h5" | "h5" => Ok(Self::Stm32H5),
            "stm32u5" | "u5" => Ok(Self::Stm32U5),
            _ => Err(format!(
                "unsupported ADC register layout '{}'; supported: stm32f1, stm32f4, stm32l4, stm32g0, stm32l0, stm32h5, stm32h7, stm32u5",
                value
            )),
        }
    }
}

/// STM32F1/F4 legacy ADC registers (status `sr` + regular data `dr` are shared
/// on `Adc`; JSQR/JDR semantics are implemented only for F4).
#[derive(Debug, Default, serde::Serialize)]
pub struct F1AdcRegs {
    cr1: u32, // 0x04
    cr2: u32, // 0x08
    /// ADC_SQR3 @ 0x34 (RM0008 §11.12.14): SQ1..SQ6. Reset value 0 selects
    /// channel 0, which is what firmware that never programs the sequence
    /// converts on silicon.
    sqr3: u32,
    /// ADC_SMPR1/2 @ 0x0C/0x10: per-channel sample times. Stored for
    /// read-back; the conversion takes the model's fixed `conversion_time`.
    #[serde(default)]
    smpr1: u32,
    #[serde(default)]
    smpr2: u32,
    /// ADC_SQR1 @ 0x2C: L[23:20] (sequence length - 1) and SQ13..SQ16.
    #[serde(default)]
    sqr1: u32,
    /// ADC_SQR2 @ 0x30: SQ7..SQ12.
    #[serde(default)]
    sqr2: u32,
    /// ADC_JSQR @ 0x38 and ADC_JDR1..4 @ 0x3C..0x48. The bounded F4
    /// injected-sequence path uses these; F1 leaves them unmodelled.
    jsqr: u32,
    jdr: [u32; 4],
}

/// ADC_CR1.SCAN (RM0008 §11.12.2; AT32 ADC_CTRL1.SQEN): convert the whole
/// regular sequence on one trigger.
const F1_CR1_SCAN: u32 = 1 << 8;
/// ADC_CR2.DMA (RM0008 §11.12.3; AT32 ADC_CTRL2.OCDMAEN): a DMA request each
/// time a regular conversion updates DR.
const F1_CR2_DMA: u32 = 1 << 8;

/// STM32L4 ADC register file (data `dr` is shared on `Adc`).
#[derive(Debug, Default, serde::Serialize)]
pub struct L4AdcRegs {
    isr: u32,        // 0x00
    ier: u32,        // 0x04
    cr: u32,         // 0x08
    cfgr: u32,       // 0x0C
    cfgr2: u32,      // 0x10
    smpr1: u32,      // 0x14
    smpr2: u32,      // 0x18
    sqr1: u32,       // 0x30
    sqr2: u32,       // 0x34
    sqr3: u32,       // 0x38
    sqr4: u32,       // 0x3C
    common_ccr: u32, // 0x308
}

/// STM32H7 ADC control registers (RM0468 §26.4; offsets and reset values taken
/// from the vendored `tests/fixtures/real_world/stm32h735.svd`).
///
/// What makes this NOT the L4 block, and why the old `"h7" => Stm32L4` alias
/// was a fidelity bug rather than a shortcut:
///
/// * `PCSEL` @ 0x1C — channel preselection. Has no L4 counterpart at all, and
///   H7 firmware must set a channel's bit before that channel converts.
/// * `LTR1`/`HTR1` @ 0x20/0x24 as a 26-bit PAIR, where the L4 has `TR1`/`TR2`/
///   `TR3` packing both thresholds into one word. `HTR1` resets to 0x03FF_FFFF.
///   Reading an L4 `TR2` at 0x24 returns an H7 `HTR1`.
/// * `CALFACT2` @ 0xC8 and the linearity-calibration `LINCALRDYWn` bits in
///   `CR` — H7-only.
/// * `RES` is `CFGR[4:2]`, three bits, sparsely encoded (see
///   [`h7_resolution_bits`]). On the L4 it is `CFGR[4:3]`, two bits. The same
///   firmware write therefore selects a different width on each.
/// * `DR` is 16-bit. Every other family here is 12-bit.
///
/// Reset values that matter for bring-up: `CR` = 0x2000_0000 (`DEEPPWD` set —
/// the converter is in deep power-down out of reset and firmware MUST clear it
/// before anything else responds) and `CFGR` = 0x8000_0000 (`JQDIS`).
#[derive(Debug, Default, serde::Serialize)]
pub struct H7AdcRegs {
    isr: u32,      // 0x00
    ier: u32,      // 0x04
    cr: u32,       // 0x08  reset 0x2000_0000 (DEEPPWD)
    cfgr: u32,     // 0x0C  reset 0x8000_0000 (JQDIS)
    cfgr2: u32,    // 0x10
    smpr1: u32,    // 0x14
    smpr2: u32,    // 0x18
    pcsel: u32,    // 0x1C  H7-only
    ltr1: u32,     // 0x20
    htr1: u32,     // 0x24  reset 0x03FF_FFFF
    sqr1: u32,     // 0x30
    sqr2: u32,     // 0x34
    sqr3: u32,     // 0x38
    sqr4: u32,     // 0x3C
    jsqr: u32,     // 0x4C
    ofr: [u32; 4], // 0x60..0x6C
    jdr: [u32; 4], // 0x80..0x8C
    awd2cr: u32,   // 0xA0
    awd3cr: u32,   // 0xA4
    ltr2: u32,     // 0xB0
    htr2: u32,     // 0xB4  reset 0x03FF_FFFF
    ltr3: u32,     // 0xB8
    htr3: u32,     // 0xBC  reset 0x03FF_FFFF
    difsel: u32,   // 0xC0
    calfact: u32,  // 0xC4
    calfact2: u32, // 0xC8  H7-only
    /// ADCx_CCR in the common block (+0x300 from the pair's base).
    common_ccr: u32,
}

/// STM32U5 ADC control registers (RM0456 §30; offsets and reset values from
/// the vendored `tests/fixtures/real_world/stm32u575.svd` ADC1 block).
///
/// The U5 is the family that is neither L4 nor H7:
///
/// * `CFGR1` (not `CFGR`) names the configuration register at 0x0C, and its
///   `RES[3:2]` is the L4-style two-bit field but with the 14-bit converter's
///   encoding (14/12/10/8) — the H7's is three bits at [4:2] and starts at 16.
/// * `PCSEL` @ 0x1C, `AWD2CR`/`AWD3CR` @ 0xA0/0xA4, `CALFACT2` @ 0xC8 are
///   present like the H7's, plus a U5-only `GCOMP` @ 0x70.
/// * The watchdog pairs live at 0xA8..0xBC (`LTR1`/`HTR1` @ 0xA8/0xAC), where
///   the H7 puts `LTR1`/`HTR1` at 0x20/0x24, and the U5 `HTRx` reset to the
///   25-bit `0x01FF_FFFF` rather than the H7's 26-bit `0x03FF_FFFF`.
///
/// Same bring-up contract as the H7: `DEEPPWD` resets set (CR = 0x2000_0000)
/// and blocks `ADVREGEN`/`ADEN`; `CFGR1.JQDIS` resets set (0x8000_0000).
#[derive(Debug, Default, serde::Serialize)]
pub struct U5AdcRegs {
    isr: u32,      // 0x00
    ier: u32,      // 0x04
    cr: u32,       // 0x08  reset 0x2000_0000 (DEEPPWD)
    cfgr: u32,     // 0x0C  CFGR1, reset 0x8000_0000 (JQDIS)
    cfgr2: u32,    // 0x10
    smpr1: u32,    // 0x14
    smpr2: u32,    // 0x18
    pcsel: u32,    // 0x1C
    sqr1: u32,     // 0x30
    sqr2: u32,     // 0x34
    sqr3: u32,     // 0x38
    sqr4: u32,     // 0x3C
    jsqr: u32,     // 0x4C
    ofr: [u32; 4], // 0x60..0x6C
    gcomp: u32,    // 0x70  U5-only
    jdr: [u32; 4], // 0x80..0x8C
    awd2cr: u32,   // 0xA0
    awd3cr: u32,   // 0xA4
    ltr1: u32,     // 0xA8
    htr1: u32,     // 0xAC  reset 0x01FF_FFFF
    ltr2: u32,     // 0xB0
    htr2: u32,     // 0xB4  reset 0x01FF_FFFF
    ltr3: u32,     // 0xB8
    htr3: u32,     // 0xBC  reset 0x01FF_FFFF
    difsel: u32,   // 0xC0
    calfact: u32,  // 0xC4
    calfact2: u32, // 0xC8
    /// ADC12 common `ADC12_CCR` @ 0x308 (ADC12 @ 0x42028300, inside the ADC1
    /// 1 KiB window) — firmware configures the shared clock/prescaler there.
    common_ccr: u32,
}

/// STM32G0/C0 ADC control registers (see [`AdcRegisterLayout::Stm32G0`]).
#[derive(Debug, Default, serde::Serialize)]
pub struct G0AdcRegs {
    isr: u32,        // 0x00
    ier: u32,        // 0x04
    cr: u32,         // 0x08  reset 0 (no DEEPPWD on this block)
    cfgr1: u32,      // 0x0C
    cfgr2: u32,      // 0x10
    smpr: u32,       // 0x14
    awd1tr: u32,     // 0x20  reset 0x0FFF_0000
    awd2tr: u32,     // 0x24  reset 0x0FFF_0000
    chselr: u32,     // 0x28
    awd3tr: u32,     // 0x2C  reset 0x0FFF_0000
    awd2cr: u32,     // 0xA0
    awd3cr: u32,     // 0xA4
    calfact: u32,    // 0xB4  CALFACT[6:0]
    common_ccr: u32, // 0x308 ADC_CCR (PRESC, VREFEN, TSEN, VBATEN)
}

/// Family-isolated ADC control registers.
#[derive(Debug, serde::Serialize)]
enum AdcRegs {
    Stm32F1(F1AdcRegs),
    /// F2/F4: same `F1AdcRegs` storage, F4 SWSTART/EXTEN semantics in the write path.
    Stm32F4(F1AdcRegs),
    Stm32L4(L4AdcRegs),
    Stm32H7(H7AdcRegs),
    Stm32U5(U5AdcRegs),
    Stm32G0(G0AdcRegs),
}

#[derive(Debug, serde::Serialize)]
pub struct Adc {
    regs: AdcRegs,
    /// Legacy status register (F1 SR). Shared because the WASM bridge pokes it
    /// directly to inject an EOC; also the conversion engine's EOC flag.
    pub sr: u32,
    /// Conversion data register — shared result path for both families.
    pub dr: u32,
    /// STM32F1/F4 clear EOC when ADC_DR is read. `Peripheral::read` takes
    /// `&self`, so this records the acknowledgement until the next EOC event
    /// without mutating the public bridge-visible status word.
    #[serde(skip)]
    dr_read_since_eoc: Cell<bool>,

    // Shared conversion engine.
    converting: bool,
    cycles_remaining: u32,
    conversion_time: u32,
    /// True while the bounded F4 injected group is converting. Injected
    /// conversions share the conversion countdown but write JDRx, not DR.
    f4_injected_active: bool,
    f4_injected_sequence_rank: u8,
    /// Per-channel injected levels, held in millivolts. `None` means no
    /// external input; conversion waits until the ADC samples the mux.
    channel_inputs_mv: [Option<u16>; MAX_CHANNELS],
    /// Positive full-scale reference voltage. Defaults to the common 3.3 V
    /// rail; `board.power.vdda_volts` can drive it explicitly.
    reference_millivolts: f64,
    /// F1 sample-and-hold value captured at SWSTART. A later pad, mux or Vdda
    /// change applies to the next sample.
    sampled_input: Option<HeldAdcInput>,
    /// Position in the regular sequence of the conversion in progress
    /// (0 = SQ1). Always 0 outside scan mode.
    #[serde(skip)]
    seq_index: u8,
    /// A DR update with CR2.DMA set requested a DMA transfer that has not been
    /// handed to the bus yet.
    #[serde(skip)]
    dma_pending: bool,

    /// Bus-published cycle clock (walk-free campaign). `Some` once attached →
    /// event-schedulable; `None` keeps the legacy walk.
    #[serde(skip)]
    clock: Option<CycleClock>,
    /// Scheduler mode: `true` while the conversion-countdown event is live.
    #[serde(skip)]
    chain_live: bool,
    /// Scheduler mode: generation of the live conversion wake (see
    /// [`F1_EVT_CONVERT`]).
    #[serde(skip)]
    conv_gen: u32,
    /// Scheduler mode: the conversion wake is asleep until the completion
    /// cycle. `(cycle, remaining)` = the bus cycle of the last executed
    /// countdown tick and the value `cycles_remaining` had then; the ticks
    /// since only decremented it, so [`Adc::projected_cycles_remaining`]
    /// derives the live value.
    #[serde(skip)]
    conv_sleep: Option<(u64, u32)>,
    /// F1 RSTCAL countdown remaining (model cycles). `None` = idle.
    #[serde(skip)]
    f1_rstcal_remaining: Option<u32>,
    /// F1 CAL countdown remaining (model cycles). `None` = idle.
    #[serde(skip)]
    f1_cal_remaining: Option<u32>,
    /// Scheduler: RSTCAL event chain armed.
    #[serde(skip)]
    f1_rstcal_live: bool,
    /// Scheduler: CAL event chain armed.
    #[serde(skip)]
    f1_cal_live: bool,
    /// Does a calibration request complete, so `CR.ADCAL` self-clears?
    ///
    /// `ADCAL` is a self-clearing COMMAND bit, but only once calibration can
    /// actually run — which needs an ADC kernel clock. NUCLEO-L476RG silicon,
    /// captured in `firmware_survival`'s `nucleo_l476rg_adc` case, shows the
    /// bit STAYING SET when the firmware enables only `AHB2ENR.ADCEN` and
    /// never touches `CCIPR`. The H5 firmware modelled here does configure the
    /// clock, so its calibration finishes. On the L4 layout, `ADC_CCR.CKMODE`
    /// != 00 (the synchronous HCLK clock) also completes calibration; that
    /// is read from the register, not from this flag.
    ///
    /// This is a stand-in for the kernel clock the RCC model does not yet
    /// expose to the ADC. When it does, this flag should die and both layouts
    /// should ask the clock instead.
    calibration_completes: bool,
    /// Die/ambient temperature the F1 temperature sensor (ADC1_IN16) reports,
    /// in °C. Set by [`Self::set_ambient_temperature_c`]; not a register, so
    /// not serialized (keeps snapshot shapes identical).
    #[serde(skip)]
    ambient_temp_c: f64,
}

impl Adc {
    pub fn new() -> Self {
        Self::new_with_layout(AdcRegisterLayout::Stm32F1)
    }

    pub fn new_with_layout(layout: AdcRegisterLayout) -> Self {
        // Per RM0351 §16.7 the L4 ADC powers up with DEEPPWD set in CR
        // (bit 29) and JQDIS set in CFGR (bit 31) — verified on NUCLEO-L476RG.
        // F1 reset is all-zeros.
        let regs = match layout {
            AdcRegisterLayout::Stm32F1 => AdcRegs::Stm32F1(F1AdcRegs::default()),
            AdcRegisterLayout::Stm32F4 => AdcRegs::Stm32F4(F1AdcRegs::default()),
            AdcRegisterLayout::Stm32L4 | AdcRegisterLayout::Stm32H5 => {
                AdcRegs::Stm32L4(L4AdcRegs {
                    cr: 0x2000_0000,
                    cfgr: 0x8000_0000,
                    ..Default::default()
                })
            }
            // Same DEEPPWD/JQDIS story as the L4, plus the three analog-watchdog
            // high thresholds, which reset to the 26-bit all-ones 0x03FF_FFFF
            // rather than 0.
            AdcRegisterLayout::Stm32H7 => AdcRegs::Stm32H7(H7AdcRegs {
                cr: 0x2000_0000,
                cfgr: 0x8000_0000,
                htr1: 0x03FF_FFFF,
                htr2: 0x03FF_FFFF,
                htr3: 0x03FF_FFFF,
                ..Default::default()
            }),
            // Same DEEPPWD/JQDIS story as the H7, but the three watchdog high
            // thresholds reset to the U5's 25-bit all-ones.
            AdcRegisterLayout::Stm32U5 => AdcRegs::Stm32U5(U5AdcRegs {
                cr: 0x2000_0000,
                cfgr: 0x8000_0000,
                htr1: 0x01FF_FFFF,
                htr2: 0x01FF_FFFF,
                htr3: 0x01FF_FFFF,
                ..Default::default()
            }),
            AdcRegisterLayout::Stm32G0 => AdcRegs::Stm32G0(G0AdcRegs {
                awd1tr: 0x0FFF_0000,
                awd2tr: 0x0FFF_0000,
                awd3tr: 0x0FFF_0000,
                ..Default::default()
            }),
        };
        Self {
            // H5, H7 and U5 firmware configure the ADC kernel clock; the plain
            // L4 case captured on NUCLEO-L476RG does not. See the field docs.
            calibration_completes: matches!(
                layout,
                AdcRegisterLayout::Stm32H5
                    | AdcRegisterLayout::Stm32H7
                    | AdcRegisterLayout::Stm32U5
            ),
            regs,
            sr: 0,
            dr: 0,
            dr_read_since_eoc: Cell::new(false),
            converting: false,
            cycles_remaining: 0,
            conversion_time: 14,
            f4_injected_active: false,
            f4_injected_sequence_rank: 0,
            channel_inputs_mv: [None; MAX_CHANNELS],
            reference_millivolts: ADC_VREF_MV,
            sampled_input: None,
            seq_index: 0,
            dma_pending: false,
            clock: None,
            chain_live: false,
            conv_gen: 0,
            conv_sleep: None,
            f1_rstcal_remaining: None,
            f1_cal_remaining: None,
            f1_rstcal_live: false,
            f1_cal_live: false,
            ambient_temp_c: DEFAULT_AMBIENT_TEMPERATURE_C,
        }
    }

    /// Set the temperature the F1 internal temperature sensor reports, in °C.
    /// Non-finite values are refused (returns `false`) and leave it unchanged.
    pub fn set_ambient_temperature_c(&mut self, celsius: f64) -> bool {
        if !celsius.is_finite() {
            return false;
        }
        self.ambient_temp_c = celsius;
        true
    }

    /// The temperature the F1 internal sensor currently reports, in °C.
    pub fn ambient_temperature_c(&self) -> f64 {
        self.ambient_temp_c
    }

    /// Millivolts on an F1 INTERNAL channel, or `None` for a pad channel.
    ///
    /// * IN16 — temperature sensor: V_SENSE = V25 − (T − 25) · Avg_Slope
    ///   (RM0008 §11.10; DS5319 Table 50 typicals 1.43 V and 4.3 mV/°C).
    ///   V_SENSE decreases with temperature: 1.43 V at 25 °C, 1.215 V at 75 °C.
    /// * IN17 — V_REFINT, 1.20 V typical (DS5319 Table 13).
    ///
    /// Both read 0 V while `CR2.TSVREFE` is clear: the sensor and the
    /// reference are disconnected from the converter (RM0008 §11.10, "The
    /// TSVREFE bit must be set to enable both internal channels").
    fn f1_internal_channel_mv(&self, ch: usize, cr2: u32) -> Option<f64> {
        if !matches!(self.regs, AdcRegs::Stm32F1(_)) || !(16..=17).contains(&ch) {
            return None;
        }
        if cr2 & F1_CR2_TSVREFE == 0 {
            return Some(0.0);
        }
        Some(if ch == 16 {
            F1_TS_V25_MV - (self.ambient_temp_c - 25.0) * F1_TS_AVG_SLOPE_MV_PER_C
        } else {
            F1_VREFINT_MV
        })
    }

    /// 12-bit code for `mv` against the reference latched at acquisition.
    fn code_for_mv(mv: f64, reference_millivolts: f64) -> u32 {
        ((mv * 4095.0 / reference_millivolts).floor()).clamp(0.0, 4095.0) as u32
    }

    /// Current positive full-scale ADC reference, in volts.
    pub fn reference_voltage_v(&self) -> f64 {
        self.reference_millivolts / 1000.0
    }

    /// Set the positive full-scale ADC reference. Invalid values leave it
    /// unchanged; a conversion already in progress keeps its held reference.
    pub fn set_reference_voltage_v(&mut self, volts: f64) -> bool {
        if !volts.is_finite() || volts <= 0.0 {
            return false;
        }
        self.reference_millivolts = volts * 1000.0;
        true
    }

    fn sample_f1_input(&self) -> Option<HeldAdcInput> {
        let channel = if matches!(self.regs, AdcRegs::Stm32F4(_)) && self.f4_injected_active {
            self.f4_injected_channel()
        } else {
            self.f1_regular_channel()
        };
        let injected = self
            .channel_inputs_mv
            .get(channel)
            .copied()
            .flatten()
            .map(f64::from);
        let (_, cr2) = self.f1_ctrl();
        let millivolts = injected.or_else(|| self.f1_internal_channel_mv(channel, cr2))?;
        Some(HeldAdcInput {
            millivolts,
            reference_millivolts: self.reference_millivolts,
        })
    }

    crate::cycle_clock::scheduler_mode!();

    /// Test/differential knob: detach the clock, pinning the model to the legacy
    /// walk (the walk-on reference for the differential gate).
    pub fn force_legacy_walk(&mut self) {
        self.clock = None;
    }

    /// One cycle of the F1 conversion countdown. Returns whether an EOC IRQ
    /// and/or DMA request should be raised this cycle. Shared verbatim by the
    /// legacy walk `tick()` and the scheduler `on_event`, so the two routes are
    /// identical by construction.
    fn advance_conversion(&mut self) -> bool {
        let mut irq = false;
        if self.converting {
            if self.cycles_remaining > 0 {
                self.cycles_remaining -= 1;
            } else {
                self.converting = false;
                let (cr1, cr2) = self.f1_ctrl();
                let is_f4 = matches!(self.regs, AdcRegs::Stm32F4(_));
                if is_f4 && self.f4_injected_active {
                    // RM0090 §13.3.9: injected results go to JDRx and JEOC
                    // is raised only after the last channel in the group.
                    let rank = usize::from(self.f4_injected_sequence_rank);
                    let previous = match &self.regs {
                        AdcRegs::Stm32F4(r) => r.jdr[rank],
                        _ => 0,
                    };
                    let result = self
                        .sampled_input
                        .take()
                        .map(|sample| {
                            Self::code_for_mv(sample.millivolts, sample.reference_millivolts)
                        })
                        .unwrap_or_else(|| (previous + 1) & 0x0FFF);
                    if let AdcRegs::Stm32F4(r) = &mut self.regs {
                        r.jdr[rank] = result;
                    }
                    if rank + 1 >= self.f4_injected_sequence_length() {
                        self.sr |= F4_SR_JEOC;
                        irq = (cr1 & F4_CR1_JEOCIE) != 0;
                        self.f4_injected_active = false;
                        self.f4_injected_sequence_rank = 0;
                    } else {
                        self.f4_injected_sequence_rank += 1;
                        // In-chain continuation: the running wake carries on.
                        self.begin_conversion();
                    }
                    return irq;
                }
                let last_rank = self.seq_index + 1 >= self.f1_sequence_len();
                // The F4 raises OVR when EOCS is set and the previous result
                // was not read before this one arrives (RM0090 §13.8.2).
                let f4_overrun = is_f4
                    && (cr2 & F4_CR2_EOCS) != 0
                    && (self.sr & F4_SR_EOC) != 0
                    && !self.dr_read_since_eoc.get();

                // The selected SQR3 channel, its input voltage and VREF+ were
                // latched at SWSTART. Changes while SAR conversion is running
                // are visible only to the next sample-and-hold acquisition.
                // A channel with no external or modeled internal source keeps
                // the old incrementing fallback for visual feedback.
                if let Some(sample) = self.sampled_input.take() {
                    let count12 = Self::code_for_mv(sample.millivolts, sample.reference_millivolts);
                    self.dr = match &self.regs {
                        AdcRegs::Stm32F4(_) => scaled_input_code(count12, f4_resolution_bits(cr1)),
                        _ => count12,
                    };
                } else {
                    self.dr = (self.dr + 1) & 0xFFF;
                }
                // CR2.DMA: every DR update requests a transfer (RM0008
                // §11.12.3; RM_AT32F421 §18.4.4.2).
                if cr2 & F1_CR2_DMA != 0 {
                    self.dma_pending = true;
                }
                // EOC marks the end of the regular group (RM0008 §11.12.1 /
                // AT32 ADC_STS.OCCE). F4 `CR2.EOCS` raises it after each
                // conversion instead (RM0090 §13.13.3).
                let eoc = last_rank || (is_f4 && (cr2 & F4_CR2_EOCS) != 0);
                if eoc {
                    self.sr |= F4_SR_EOC;
                    self.dr_read_since_eoc.set(false);
                }
                if f4_overrun {
                    self.sr |= F4_SR_OVR;
                }
                if (eoc && (cr1 & F1_CR1_EOCIE) != 0) || (f4_overrun && (cr1 & F4_CR1_OVRIE) != 0) {
                    irq = true;
                }
                if !last_rank {
                    // Scan mode: the next channel of the sequence follows at
                    // once. In-chain continuation, like CONT: the running
                    // wake carries on (no new generation).
                    self.seq_index += 1;
                    self.begin_conversion();
                } else {
                    self.seq_index = 0;
                    // Continuous mode (CONT + ADON) repeats the full group.
                    if (cr2 & F1_CR2_CONT) != 0 && (cr2 & F1_CR2_ADON) != 0 {
                        // The chain that is running this restarts itself:
                        // same wake, same generation.
                        self.begin_conversion();
                    }
                }
            }
        }
        irq
    }

    /// Read back the injected 12-bit count for a channel (`0xFFFF` = nothing
    /// injected). Conversion is evaluated against the current reference when
    /// the ADC acquires the selected input.
    pub fn channel_input_count(&self, channel: u8) -> u16 {
        self.channel_inputs_mv
            .get(channel as usize)
            .copied()
            .flatten()
            .map(|mv| Self::code_for_mv(f64::from(mv), self.reference_millivolts) as u16)
            .unwrap_or(NO_ADC_INPUT)
    }

    /// Analog input channels this register layout has, `0..count`.
    ///
    /// * F1 layout — the SR/CR1/CR2/SMPRx/SQRx block shared by F1, F2 and F4.
    ///   Regular channels 0..=18: IN0..IN15 on pads, then the internal
    ///   temperature sensor, V_REFINT and (on F4) V_BAT on IN16..IN18.
    /// * L4 layout (L4, H5, F7, G0) — 0..=18, where ADC1 IN0 is V_REFINT and
    ///   IN17/IN18 are the temperature sensor and V_BAT.
    /// * H7 — 0..=19, the `PCSEL` bitmap's width.
    /// * U5 — 0..=19, the `PCSEL` bitmap's width.
    pub fn channel_count(&self) -> u8 {
        match &self.regs {
            AdcRegs::Stm32F1(_)
            | AdcRegs::Stm32F4(_)
            | AdcRegs::Stm32L4(_)
            | AdcRegs::Stm32G0(_) => 19,
            AdcRegs::Stm32H7(_) | AdcRegs::Stm32U5(_) => 20,
        }
    }

    pub fn set_channel_input(&mut self, channel: u8, millivolts: u16) {
        if channel < self.channel_count() {
            self.channel_inputs_mv[channel as usize] = Some(millivolts);
        }
    }

    /// Remove the injected level from a channel, returning it to the modeled
    /// internal source. `set_channel_input` cannot express this itself: its
    /// millivolt input preserves 0xFFFF as its "no injection" sentinel.
    pub fn clear_channel_input(&mut self, channel: u8) {
        if (channel as usize) < self.channel_inputs_mv.len() {
            self.channel_inputs_mv[channel as usize] = None;
        }
    }

    /// A write starts a conversion: any conversion wake already armed (asleep
    /// until the old completion, or awake) is superseded, and
    /// `take_scheduled_events` arms a fresh one for the new countdown.
    fn start_conversion(&mut self) {
        self.conv_gen = self.conv_gen.wrapping_add(1) & F1_EVT_GEN_MASK;
        self.conv_sleep = None;
        self.chain_live = false;
        self.begin_conversion();
    }

    fn conv_token(&self) -> u32 {
        F1_EVT_CONVERT | (self.conv_gen & F1_EVT_GEN_MASK)
    }

    /// `cycles_remaining` as the countdown stands at the published bus cycle:
    /// an asleep chain skipped ticks that only decremented it.
    fn projected_cycles_remaining(&self) -> u32 {
        match (&self.conv_sleep, &self.clock) {
            (Some((anchor, remaining)), Some(clock)) => {
                let elapsed = clock.now().saturating_sub(*anchor);
                remaining.saturating_sub(elapsed.min(u64::from(u32::MAX)) as u32)
            }
            _ => self.cycles_remaining,
        }
    }

    fn begin_conversion(&mut self) {
        self.converting = true;
        self.cycles_remaining = self.conversion_time;
        self.sampled_input = self.sample_f1_input();
        // RM0008 clears F1 EOC by reading DR or writing zero to SR. F4 also
        // keeps an unread result visible across conversion start, where it is
        // used by the bounded EOCS overrun check.
    }

    /// (cr1, cr2) for the F1 control registers; (0, 0) on L4 (no conversion
    /// engine runs there, so these are only consulted on the F1 path).
    fn f1_ctrl(&self) -> (u32, u32) {
        match &self.regs {
            AdcRegs::Stm32F1(r) | AdcRegs::Stm32F4(r) => (r.cr1, r.cr2),
            // Neither the L4, the H7 nor the U5 runs the F1 countdown engine.
            AdcRegs::Stm32L4(_)
            | AdcRegs::Stm32H7(_)
            | AdcRegs::Stm32U5(_)
            | AdcRegs::Stm32G0(_) => (0, 0),
        }
    }

    /// Number of F4 injected conversions. JSQR.JL stores the count minus one.
    fn f4_injected_sequence_length(&self) -> usize {
        match &self.regs {
            AdcRegs::Stm32F4(r) => ((r.jsqr >> 20) & 0x3) as usize + 1,
            _ => 1,
        }
    }

    /// Injected rank channel from JSQR. F4 right-aligns short sequences in
    /// JSQ3/JSQ4 (two ranks) or JSQ2..JSQ4 (three ranks).
    fn f4_injected_channel(&self) -> usize {
        match &self.regs {
            AdcRegs::Stm32F4(r) => {
                let first_slot = 4 - self.f4_injected_sequence_length();
                let slot = first_slot + usize::from(self.f4_injected_sequence_rank);
                ((r.jsqr >> (slot * 5)) & 0x1F) as usize
            }
            _ => 0,
        }
    }

    /// Regular channel being converted: SQ(n+1) of SQR3/SQR2/SQR1 for
    /// sequence position n on the F1 layout; 0 on the other families (their
    /// engines select from their own SQR1/PCSEL).
    fn f1_regular_channel(&self) -> usize {
        match &self.regs {
            AdcRegs::Stm32F1(r) | AdcRegs::Stm32F4(r) => {
                let n = u32::from(self.seq_index);
                let (word, slot) = match n {
                    0..=5 => (r.sqr3, n),
                    6..=11 => (r.sqr2, n - 6),
                    _ => (r.sqr1, n - 12),
                };
                ((word >> (5 * slot)) & 0x1F) as usize
            }
            AdcRegs::Stm32L4(_)
            | AdcRegs::Stm32H7(_)
            | AdcRegs::Stm32U5(_)
            | AdcRegs::Stm32G0(_) => 0,
        }
    }

    /// Regular-sequence length: SQR1.L + 1 with CR1.SCAN set, else one.
    fn f1_sequence_len(&self) -> u8 {
        match &self.regs {
            AdcRegs::Stm32F1(r) | AdcRegs::Stm32F4(r) if r.cr1 & F1_CR1_SCAN != 0 => {
                ((r.sqr1 >> 20) & 0xF) as u8 + 1
            }
            _ => 1,
        }
    }

    /// A trigger converts the regular sequence from SQ1.
    fn start_sequence(&mut self) {
        self.seq_index = 0;
        self.start_conversion();
    }

    fn read_reg_l4(r: &L4AdcRegs, dr: u32, reg: u64) -> u32 {
        match reg {
            0x00 => r.isr,
            0x04 => r.ier,
            0x08 => r.cr,
            0x0C => r.cfgr,
            0x10 => r.cfgr2,
            0x14 => r.smpr1,
            0x18 => r.smpr2,
            0x30 => r.sqr1,
            0x34 => r.sqr2,
            0x38 => r.sqr3,
            0x3C => r.sqr4,
            0x40 => dr,
            0x308 => r.common_ccr,
            _ => {
                crate::census_reg!("adc:Adc", reg, "read");
                0
            }
        }
    }

    /// L4 single-conversion engine. ADSTART (CR bit 2) with ADEN (bit 0) and a
    /// ready converter (ISR.ADRDY) acquires the SQR1.SQ1 channel. An injected
    /// voltage uses the current reference; an unbound channel keeps the fixed
    /// internal source. CFGR.RES selects the width. Conversion raises EOC + EOS
    /// and auto-clears ADSTART immediately (no analog settling to model).
    fn maybe_start_l4_conversion(&mut self, cr: u32) {
        let aden = cr & 0x1 != 0;
        let adstart = cr & (1 << 2) != 0;
        let (adrdy, cfgr, channel) = match &self.regs {
            AdcRegs::Stm32L4(r) => (r.isr & 0x1 != 0, r.cfgr, ((r.sqr1 >> 6) & 0x1F) as usize),
            // H7 and U5 have their own engines (`maybe_start_h7_conversion`,
            // `maybe_start_u5_conversion`).
            AdcRegs::Stm32F1(_)
            | AdcRegs::Stm32F4(_)
            | AdcRegs::Stm32H7(_)
            | AdcRegs::Stm32U5(_)
            | AdcRegs::Stm32G0(_) => return,
        };
        if !(aden && adstart && adrdy) {
            return;
        }
        let bits = l4_resolution_bits(cfgr);
        self.dr = match self.channel_inputs_mv.get(channel).copied().flatten() {
            Some(millivolts) => scaled_input_code(
                Self::code_for_mv(f64::from(millivolts), self.reference_millivolts),
                bits,
            ),
            None => l4_adc_code(bits),
        };
        let cont = cfgr & (1 << 13) != 0;
        if let AdcRegs::Stm32L4(r) = &mut self.regs {
            if !cont {
                r.cr &= !(1 << 2); // ADSTART auto-clears after a single conversion
            }
            r.isr |= (1 << 2) | (1 << 3); // EOC | EOS
        }
    }

    fn read_reg_h7(r: &H7AdcRegs, dr: u32, reg: u64) -> u32 {
        match reg {
            0x00 => r.isr,
            0x04 => r.ier,
            0x08 => r.cr,
            0x0C => r.cfgr,
            0x10 => r.cfgr2,
            0x14 => r.smpr1,
            0x18 => r.smpr2,
            0x1C => r.pcsel,
            0x20 => r.ltr1,
            0x24 => r.htr1,
            0x30 => r.sqr1,
            0x34 => r.sqr2,
            0x38 => r.sqr3,
            0x3C => r.sqr4,
            0x40 => dr,
            0x4C => r.jsqr,
            0x60..=0x6C if reg % 4 == 0 => r.ofr[((reg - 0x60) / 4) as usize],
            0x80..=0x8C if reg % 4 == 0 => r.jdr[((reg - 0x80) / 4) as usize],
            0xA0 => r.awd2cr,
            0xA4 => r.awd3cr,
            0xB0 => r.ltr2,
            0xB4 => r.htr2,
            0xB8 => r.ltr3,
            0xBC => r.htr3,
            0xC0 => r.difsel,
            0xC4 => r.calfact,
            0xC8 => r.calfact2,
            0x308 => r.common_ccr,
            _ => 0,
        }
    }

    /// H7 power-up and conversion engine.
    ///
    /// The order here is the part that matters, and it is the order RM0468
    /// §26.4.6 imposes on firmware: out of reset `DEEPPWD` is SET, and while it
    /// is set the analog voltage regulator cannot come up. So
    ///
    ///   1. clear `DEEPPWD`, then set `ADVREGEN` -> `ISR.LDORDY` rises;
    ///   2. `ADCAL` starts a calibration that self-clears when done;
    ///   3. `ADEN` with the regulator up -> `ISR.ADRDY`;
    ///   4. `ADSTART` converts, loads `DR`, raises `EOC`|`EOS`, self-clears.
    ///
    /// Writing `ADEN` while `DEEPPWD` is still set does NOT make the converter
    /// ready — that is the whole reason a HAL that skips the wake-up hangs on
    /// silicon, and modelling it is the difference between reproducing that
    /// hang and silently succeeding where the real part would stall.
    fn h7_apply_cr(&mut self, value: u32) {
        let AdcRegs::Stm32H7(r) = &mut self.regs else {
            return;
        };
        // ADCAL is a self-clearing command: calibration is instantaneous here
        // (no analog settling to model), so it reads back as already complete
        // rather than latching set forever.
        let calibrating = value & (1 << 31) != 0;
        r.cr = value & !(1 << 31);

        let deeppwd = r.cr & (1 << 29) != 0;
        let advregen = r.cr & (1 << 28) != 0;

        // Deep power-down overrides everything: the regulator drops and the
        // converter cannot be ready.
        if deeppwd {
            r.isr &= !((1 << 12) | 0x1); // LDORDY | ADRDY
            r.cr &= !0x1; // ADEN cannot take effect
            return;
        }
        if advregen {
            r.isr |= 1 << 12; // LDORDY
        }
        if calibrating {
            // A calibration only runs with the regulator up; record a factor so
            // firmware polling CALFACT sees something coherent.
            if advregen {
                r.calfact = 0x0000_2000;
            }
        }
        if r.cr & 0x1 != 0 && advregen {
            r.isr |= 0x1; // ADRDY
        }
        if r.cr & (1 << 1) != 0 {
            // ADDIS: disable request clears ADEN and readiness.
            r.cr &= !((1 << 1) | 0x1);
            r.isr &= !0x1;
        }
    }

    /// One H7 regular conversion. Requires `ADEN` + `ADRDY` + `ADSTART`.
    fn maybe_start_h7_conversion(&mut self) {
        let AdcRegs::Stm32H7(r) = &self.regs else {
            return;
        };
        if r.cr & 0x1 == 0 || r.isr & 0x1 == 0 || r.cr & (1 << 2) == 0 {
            return;
        }
        let cfgr = r.cfgr;
        let bits = h7_resolution_bits(cfgr);
        // SQR1[10:6] is the first conversion in the regular sequence (SQ1).
        let ch = ((r.sqr1 >> 6) & 0x1F) as usize;
        let injected = self.channel_inputs_mv.get(ch).copied().flatten();
        // Preserve the existing 12-bit mV conversion, then scale to the
        // configured width rather than truncating a raw millivolt value.
        let code = if let Some(injected) = injected {
            scaled_input_code(
                Self::code_for_mv(f64::from(injected), self.reference_millivolts),
                bits,
            )
        } else {
            scaled_adc_code(bits)
        };
        self.dr = code;
        let cont = cfgr & (1 << 13) != 0;
        if let AdcRegs::Stm32H7(r) = &mut self.regs {
            r.isr |= (1 << 2) | (1 << 3); // EOC | EOS
            if !cont {
                r.cr &= !(1 << 2); // ADSTART self-clears after a single conversion
            }
        }
    }

    fn write_reg_h7(&mut self, reg: u64, value: u32) {
        // CR carries the power-up state machine, so it routes through
        // `h7_apply_cr` rather than being latched verbatim.
        if reg == 0x08 {
            self.h7_apply_cr(value);
            self.maybe_start_h7_conversion();
            return;
        }
        if let AdcRegs::Stm32H7(r) = &mut self.regs {
            match reg {
                0x00 => r.isr &= !value, // rc_w1
                0x04 => r.ier = value,
                0x0C => r.cfgr = value,
                0x10 => r.cfgr2 = value,
                0x14 => r.smpr1 = value,
                0x18 => r.smpr2 = value,
                0x1C => r.pcsel = value,
                0x20 => r.ltr1 = value,
                0x24 => r.htr1 = value,
                0x30 => r.sqr1 = value,
                0x34 => r.sqr2 = value,
                0x38 => r.sqr3 = value,
                0x3C => r.sqr4 = value,
                0x40 => {} // DR read-only
                0x4C => r.jsqr = value,
                0x60..=0x6C if reg % 4 == 0 => r.ofr[((reg - 0x60) / 4) as usize] = value,
                0x80..=0x8C if reg % 4 == 0 => {} // JDRn read-only
                0xA0 => r.awd2cr = value,
                0xA4 => r.awd3cr = value,
                0xB0 => r.ltr2 = value,
                0xB4 => r.htr2 = value,
                0xB8 => r.ltr3 = value,
                0xBC => r.htr3 = value,
                0xC0 => r.difsel = value,
                0xC4 => r.calfact = value,
                0xC8 => r.calfact2 = value,
                0x308 => r.common_ccr = value,
                _ => {}
            }
        }
    }

    fn read_reg_u5(r: &U5AdcRegs, dr: u32, reg: u64) -> u32 {
        match reg {
            0x00 => r.isr,
            0x04 => r.ier,
            0x08 => r.cr,
            0x0C => r.cfgr,
            0x10 => r.cfgr2,
            0x14 => r.smpr1,
            0x18 => r.smpr2,
            0x1C => r.pcsel,
            0x30 => r.sqr1,
            0x34 => r.sqr2,
            0x38 => r.sqr3,
            0x3C => r.sqr4,
            0x40 => dr,
            0x4C => r.jsqr,
            0x60..=0x6C if reg % 4 == 0 => r.ofr[((reg - 0x60) / 4) as usize],
            0x70 => r.gcomp,
            0x80..=0x8C if reg % 4 == 0 => r.jdr[((reg - 0x80) / 4) as usize],
            0xA0 => r.awd2cr,
            0xA4 => r.awd3cr,
            0xA8 => r.ltr1,
            0xAC => r.htr1,
            0xB0 => r.ltr2,
            0xB4 => r.htr2,
            0xB8 => r.ltr3,
            0xBC => r.htr3,
            0xC0 => r.difsel,
            0xC4 => r.calfact,
            0xC8 => r.calfact2,
            0x308 => r.common_ccr,
            _ => 0,
        }
    }

    /// U5 power-up and conversion engine. The order is the same one RM0456
    /// §30.4.7 imposes on firmware as the H7's: clear `DEEPPWD`, raise
    /// `ADVREGEN` -> `ISR.LDORDY`, `ADCAL` self-clears, `ADEN` -> `ADRDY`, then
    /// `ADSTART` loads `DR` for `CFGR1.RES` (14-bit by default) and raises
    /// `EOC`|`EOS`.
    fn u5_apply_cr(&mut self, value: u32) {
        let AdcRegs::Stm32U5(r) = &mut self.regs else {
            return;
        };
        let calibrating = value & (1 << 31) != 0;
        r.cr = value & !(1 << 31);

        let deeppwd = r.cr & (1 << 29) != 0;
        let advregen = r.cr & (1 << 28) != 0;

        if deeppwd {
            r.isr &= !((1 << 12) | 0x1); // LDORDY | ADRDY
            r.cr &= !0x1; // ADEN cannot take effect
            return;
        }
        if advregen {
            r.isr |= 1 << 12; // LDORDY
        }
        if calibrating && advregen {
            r.calfact = 0x0000_2000;
        }
        if r.cr & 0x1 != 0 && advregen {
            r.isr |= 0x1; // ADRDY
        }
        if r.cr & (1 << 1) != 0 {
            r.cr &= !((1 << 1) | 0x1);
            r.isr &= !0x1;
        }
    }

    /// One U5 regular conversion. Requires `ADEN` + `ADRDY` + `ADSTART`.
    fn maybe_start_u5_conversion(&mut self) {
        let AdcRegs::Stm32U5(r) = &self.regs else {
            return;
        };
        if r.cr & 0x1 == 0 || r.isr & 0x1 == 0 || r.cr & (1 << 2) == 0 {
            return;
        }
        let cfgr = r.cfgr;
        let bits = u5_resolution_bits(cfgr);
        let ch = ((r.sqr1 >> 6) & 0x1F) as usize;
        let injected = self.channel_inputs_mv.get(ch).copied().flatten();
        let code = if let Some(injected) = injected {
            scaled_input_code(
                Self::code_for_mv(f64::from(injected), self.reference_millivolts),
                bits,
            )
        } else {
            scaled_adc_code(bits)
        };
        self.dr = code;
        let cont = cfgr & (1 << 13) != 0;
        if let AdcRegs::Stm32U5(r) = &mut self.regs {
            r.isr |= (1 << 2) | (1 << 3); // EOC | EOS
            if !cont {
                r.cr &= !(1 << 2); // ADSTART self-clears after a single conversion
            }
        }
    }

    fn write_reg_u5(&mut self, reg: u64, value: u32) {
        if reg == 0x08 {
            self.u5_apply_cr(value);
            self.maybe_start_u5_conversion();
            return;
        }
        if let AdcRegs::Stm32U5(r) = &mut self.regs {
            match reg {
                0x00 => r.isr &= !value, // rc_w1
                0x04 => r.ier = value,
                0x0C => r.cfgr = value,
                0x10 => r.cfgr2 = value,
                0x14 => r.smpr1 = value,
                0x18 => r.smpr2 = value,
                0x1C => r.pcsel = value,
                0x30 => r.sqr1 = value,
                0x34 => r.sqr2 = value,
                0x38 => r.sqr3 = value,
                0x3C => r.sqr4 = value,
                0x40 => {} // DR read-only
                0x4C => r.jsqr = value,
                0x60..=0x6C if reg % 4 == 0 => r.ofr[((reg - 0x60) / 4) as usize] = value,
                0x70 => r.gcomp = value,
                0x80..=0x8C if reg % 4 == 0 => {} // JDRn read-only
                0xA0 => r.awd2cr = value,
                0xA4 => r.awd3cr = value,
                0xA8 => r.ltr1 = value,
                0xAC => r.htr1 = value,
                0xB0 => r.ltr2 = value,
                0xB4 => r.htr2 = value,
                0xB8 => r.ltr3 = value,
                0xBC => r.htr3 = value,
                0xC0 => r.difsel = value,
                0xC4 => r.calfact = value,
                0xC8 => r.calfact2 = value,
                0x308 => r.common_ccr = value,
                _ => {}
            }
        }
    }

    fn read_reg_g0(r: &G0AdcRegs, dr: u32, reg: u64) -> u32 {
        match reg {
            0x00 => r.isr,
            0x04 => r.ier,
            0x08 => r.cr,
            0x0C => r.cfgr1,
            0x10 => r.cfgr2,
            0x14 => r.smpr,
            0x20 => r.awd1tr,
            0x24 => r.awd2tr,
            0x28 => r.chselr,
            0x2C => r.awd3tr,
            0x40 => dr,
            0xA0 => r.awd2cr,
            0xA4 => r.awd3cr,
            0xB4 => r.calfact,
            0x308 => r.common_ccr,
            _ => {
                crate::census_reg!("adc:Adc", reg, "read");
                0
            }
        }
    }

    /// Apply a CR write on the G0/C0 block (RM0444 §15.12.3).
    ///
    /// `ADCAL`, `ADSTP`, `ADSTART`, `ADDIS` and `ADEN` are `rs`: software can
    /// only set them and hardware clears them, so a 0 written there (which
    /// every LL `MODIFY_REG(CR, ADC_CR_BITS_PROPERTY_RS, ...)` does) has no
    /// effect. `ADVREGEN` is plain read/write. Each command completes at once:
    ///
    /// * `ADCAL` (ADEN = 0) — calibration ends, `ADCAL` clears and
    ///   `ISR.EOCAL` sets. `CALFACT` keeps its value: the model has no offset
    ///   error to calibrate out.
    /// * `ADEN` with `ADVREGEN` — `ISR.ADRDY` sets.
    /// * `ADDIS` — the ADC disables: `ADEN` and `ADDIS` clear.
    /// * `ADSTP` — any conversion stops: `ADSTART` and `ADSTP` clear.
    ///
    /// Without the last two, `HAL_ADC_Stop()`/`HAL_ADC_DeInit()` time out and
    /// stock analogRead() returns 0 after a good conversion.
    fn g0_apply_cr(&mut self, value: u32) {
        const RS: u32 = G0_CR_ADCAL | G0_CR_ADSTP | G0_CR_ADSTART | G0_CR_ADDIS | G0_CR_ADEN;
        let AdcRegs::Stm32G0(r) = &mut self.regs else {
            return;
        };
        let mut cr = (r.cr & RS) | (value & (RS | G0_CR_ADVREGEN));
        if cr & G0_CR_ADCAL != 0 && cr & G0_CR_ADEN == 0 {
            cr &= !G0_CR_ADCAL;
            r.isr |= G0_ISR_EOCAL;
        }
        if cr & G0_CR_ADSTP != 0 {
            cr &= !(G0_CR_ADSTP | G0_CR_ADSTART);
        }
        if cr & G0_CR_ADDIS != 0 {
            cr &= !(G0_CR_ADDIS | G0_CR_ADEN | G0_CR_ADSTART);
        }
        if cr & G0_CR_ADEN != 0 && cr & G0_CR_ADVREGEN != 0 {
            r.isr |= G0_ISR_ADRDY;
        }
        r.cr = cr;
    }

    /// Channels of the G0/C0 regular sequence, in conversion order.
    ///
    /// * `CFGR1.CHSELRMOD` = 0 — `CHSELR[18:0]` is a bitmap, converted from
    ///   the lowest channel up, or from the highest down with `SCANDIR`.
    /// * `CHSELRMOD` = 1 — `CHSELR` holds ranks SQ1..SQ8, four bits each from
    ///   bit 0; channel code 0xF ends the sequence.
    fn g0_sequence(r: &G0AdcRegs) -> Vec<usize> {
        if r.cfgr1 & G0_CFGR1_CHSELRMOD == 0 {
            let mut chans: Vec<usize> = (0..19).filter(|ch| r.chselr & (1 << ch) != 0).collect();
            if r.cfgr1 & G0_CFGR1_SCANDIR != 0 {
                chans.reverse();
            }
            chans
        } else {
            (0..8)
                .map(|rank| ((r.chselr >> (4 * rank)) & 0xF) as usize)
                .take_while(|&ch| ch != 0xF)
                .collect()
        }
    }

    /// G0/C0 regular conversion. `ADSTART` with `ADEN` and `ISR.ADRDY`
    /// converts the first channel of the sequence: its held input against
    /// VREF+, at `CFGR1.RES` (same [4:3] encoding as the L4), right- or
    /// left-aligned per `CFGR1.ALIGN`. A channel with no held input converts
    /// the model's fixed source, as the L4/H7/U5 engines do. Sets `EOC`, and
    /// `EOS` when the sequence is that one channel; `ADSTART` clears unless
    /// `CONT` is set.
    ///
    /// Limits: one conversion per start (a multi-channel group stops after
    /// its first channel, without `EOS`); no conversion IRQ, DMA request,
    /// overrun, analog watchdog or internal channel (VREFINT, TS, VBAT).
    fn maybe_start_g0_conversion(&mut self) {
        let AdcRegs::Stm32G0(r) = &self.regs else {
            return;
        };
        if r.cr & G0_CR_ADEN == 0 || r.isr & G0_ISR_ADRDY == 0 || r.cr & G0_CR_ADSTART == 0 {
            return;
        }
        let cfgr1 = r.cfgr1;
        let bits = l4_resolution_bits(cfgr1);
        let sequence = Self::g0_sequence(r);
        let Some(&ch) = sequence.first() else {
            return;
        };
        let code = match self.channel_inputs_mv.get(ch).copied().flatten() {
            Some(mv) => scaled_input_code(
                Self::code_for_mv(f64::from(mv), self.reference_millivolts),
                bits,
            ),
            None => scaled_adc_code(bits),
        };
        self.dr = if cfgr1 & G0_CFGR1_ALIGN != 0 {
            // Left alignment: the code's MSB lands on DR bit 15, except at
            // 6 bits, where it lands on bit 7 (RM0444 §15.6.2, Figure 79).
            if bits == 6 {
                code << 2
            } else {
                code << (16 - bits)
            }
        } else {
            code
        };
        if let AdcRegs::Stm32G0(r) = &mut self.regs {
            r.isr |= G0_ISR_EOC;
            if sequence.len() == 1 {
                r.isr |= G0_ISR_EOS;
            }
            if cfgr1 & G0_CFGR1_CONT == 0 {
                r.cr &= !G0_CR_ADSTART;
            }
        }
    }

    fn write_reg_g0(&mut self, reg: u64, value: u32) {
        if reg == 0x08 {
            self.g0_apply_cr(value);
            self.maybe_start_g0_conversion();
            return;
        }
        if let AdcRegs::Stm32G0(r) = &mut self.regs {
            match reg {
                0x00 => r.isr &= !value, // rc_w1
                0x04 => r.ier = value,
                0x0C => {
                    r.cfgr1 = value;
                    // A new CHSELRMOD/SCANDIR is a channel-configuration change
                    // too (RM0444 §15.12.1, CCRDY).
                    r.isr |= G0_ISR_CCRDY;
                }
                0x10 => r.cfgr2 = value,
                0x14 => r.smpr = value,
                0x20 => r.awd1tr = value,
                0x24 => r.awd2tr = value,
                0x28 => {
                    r.chselr = value;
                    // The channel configuration applies at once: CCRDY sets.
                    r.isr |= G0_ISR_CCRDY;
                }
                0x2C => r.awd3tr = value,
                0x40 => {} // DR read-only
                0xA0 => r.awd2cr = value,
                0xA4 => r.awd3cr = value,
                0xB4 => r.calfact = value & 0x7F,
                0x308 => r.common_ccr = value,
                _ => {
                    crate::census_reg!("adc:Adc", reg, "write");
                }
            }
        }
    }

    fn write_reg_l4(r: &mut L4AdcRegs, reg: u64, value: u32, calibration_completes: bool) {
        match reg {
            // ISR is rc_w1 — a write clears matched flags; firmware can't SET it.
            0x00 => r.isr &= !value,
            0x04 => r.ier = value,
            0x08 => {
                // ADCAL (bit 31) is a self-clearing COMMAND — but only once
                // calibration can actually RUN, which needs an ADC kernel
                // clock. Both halves are silicon:
                //
                //   * NUCLEO-L476RG, captured in firmware_survival's
                //     `nucleo_l476rg_adc` case: with only AHB2ENR.ADCEN
                //     enabled and CCIPR untouched, CR reads back 0x9000_0000 —
                //     ADCAL STILL SET. No clock, no calibration, no clear.
                //   * A firmware that does configure the kernel clock sees it
                //     complete and the bit clear, which is what the H5 Arduino
                //     path needs; a HAL running the documented
                //     `while (ADC->CR & ADC_CR_ADCAL);` otherwise spins forever.
                //
                // ⚠️ Do not "simplify" this to always-clear. That breaks the
                // L476 capture, and always-latch is what made the H5 HAL hang —
                // the hang that moved stm32h563 onto the `stm32h7` profile,
                // whose 3-bit CFGR.RES[4:2] then read the fixture's 2-bit
                // RES[4:3] write as 16-bit and turned `TIER1 adc` red.
                //
                // ADC_CCR.CKMODE != 00 selects the synchronous HCLK/1,
                //     /2 or /4 clock, which needs no CCIPR.ADCSEL source, so
                //     calibration runs. Stock STM32duino on the L4 sets
                //     CKMODE = 11 before ADCAL; the L476 capture above never
                //     writes CCR (CKMODE stays 00).
                //
                // The command bits ADCAL, JADSTP, ADSTP, JADSTART, ADSTART,
                // ADDIS and ADEN are "rs" (ST: software can read and set
                // them, writing 0 has no effect; hardware clears them). The
                // HAL relies on it: LL_ADC_REG_StartConversion does
                // MODIFY_REG(CR, ADC_CR_BITS_PROPERTY_RS, ADSTART), which
                // writes ADEN = 0. Storing that write raw disabled the ADC,
                // so no conversion started and analogRead() returned 0.
                const RS: u32 = (1 << 31) | 0x3F;
                let calibrating = value & (1 << 31) != 0;
                let sync_clock = (r.common_ccr >> 16) & 0b11 != 0;
                let mut cr = (value & !RS) | ((r.cr | value) & RS);
                if calibration_completes || sync_clock {
                    cr &= !(1 << 31);
                }
                // ADSTP / JADSTP: the stop completes at once (no conversion
                // is in flight in this model) and clears its start bit.
                if cr & (1 << 4) != 0 {
                    cr &= !((1 << 4) | (1 << 2));
                }
                if cr & (1 << 5) != 0 {
                    cr &= !((1 << 5) | (1 << 3));
                }
                // ADDIS: the disable completes at once and clears ADEN.
                // ISR.ADRDY stays set until software clears it.
                if cr & (1 << 1) != 0 {
                    cr &= !((1 << 1) | 0x1);
                }
                r.cr = cr;
                // ADEN with the voltage regulator up (ADVREGEN set, DEEPPWD
                // clear) raises ISR.ADRDY. Silicon-verified on STM32H563
                // ADC1 (2026-06-11): DEEPPWD=0 -> ADVREGEN=1 -> ADEN=1 reads
                // back CR=0x10000001 with ISR=0x00000001.
                let aden = r.cr & 0x1 != 0;
                let advregen = r.cr & (1 << 28) != 0;
                let deeppwd = r.cr & (1 << 29) != 0;
                if aden && advregen && !deeppwd {
                    r.isr |= 0x1;
                }
                // Nothing else to record: this block has no CALFACT model yet.
                let _ = calibrating;
            }
            0x0C => r.cfgr = value,
            0x10 => r.cfgr2 = value,
            0x14 => r.smpr1 = value,
            0x18 => r.smpr2 = value,
            0x30 => r.sqr1 = value,
            0x34 => r.sqr2 = value,
            0x38 => r.sqr3 = value,
            0x3C => r.sqr4 = value,
            0x40 => {} // DR read-only
            0x308 => r.common_ccr = value,
            _ => {
                crate::census_reg!("adc:Adc", reg, "write");
            }
        }
    }
    /// Cancel F1 RSTCAL/CAL countdowns and the conversion engine. Used when
    /// ADON falls or the block is reset — pending scheduler events become no-ops
    /// once the remaining counters are cleared.
    fn cancel_f1_pending(&mut self) {
        self.converting = false;
        self.cycles_remaining = 0;
        self.seq_index = 0;
        self.chain_live = false;
        self.conv_gen = self.conv_gen.wrapping_add(1) & F1_EVT_GEN_MASK;
        self.conv_sleep = None;
        self.f1_rstcal_remaining = None;
        self.f1_cal_remaining = None;
        self.f1_rstcal_live = false;
        self.f1_cal_live = false;
    }

    /// Apply a full CR2 word on the STM32F1 layout after a byte merge.
    ///
    /// Software start (RM0008 §11.3.1): ADON set, EXTSEL[2:0] == 111 (software
    /// event selected), rising edge of SWSTART (bit 22). EXTTRIG enables the
    /// external-trigger path; when EXTSEL selects software, SWSTART is the
    /// trigger and EXTTRIG is not required — but if EXTTRIG is clear *and*
    /// EXTSEL is not software, SWSTART alone must not convert (external event
    /// not armed). Bit 30 is ignored on F1 (that is the F2/F4 SWSTART).
    ///
    /// Calibration (ST F1 HAL): rising RSTCAL with ADON schedules reset
    /// completion (clears RSTCAL); rising CAL with ADON and RSTCAL idle
    /// schedules cal completion (clears CAL). Bits are NOT cleared on every
    /// write — only when the scheduled completion fires, or when ADON drops
    /// (pending work is cancelled and the command bits are dropped so a HAL
    /// poll cannot hang on a cancelled request).
    fn apply_f1_cr2(&mut self, old_cr2: u32, new_cr2: u32) -> bool {
        let mut cr2 = new_cr2;
        let adon = (cr2 & F1_CR2_ADON) != 0;
        if !adon {
            self.cancel_f1_pending();
            // Drop sticky command bits so a cancelled cal cannot trap a poll.
            cr2 &= !(F1_CR2_RSTCAL | F1_CR2_CAL | F1_CR2_SWSTART);
            if let AdcRegs::Stm32F1(r) = &mut self.regs {
                r.cr2 = cr2;
            }
            return false;
        }

        let rstcal_rise = (cr2 & F1_CR2_RSTCAL) != 0 && (old_cr2 & F1_CR2_RSTCAL) == 0;
        if rstcal_rise {
            self.f1_rstcal_remaining = Some(F1_RSTCAL_CYCLES);
            self.f1_rstcal_live = false; // re-arm take_scheduled_events
        }

        let cal_rise = (cr2 & F1_CR2_CAL) != 0 && (old_cr2 & F1_CR2_CAL) == 0;
        let rstcal_busy = (cr2 & F1_CR2_RSTCAL) != 0 || self.f1_rstcal_remaining.is_some();
        if cal_rise && !rstcal_busy {
            self.f1_cal_remaining = Some(F1_CAL_CYCLES);
            self.f1_cal_live = false;
        }

        // RM0008: SWSTART starts a regular conversion only when EXTSEL[2:0]=111
        // (software trigger). EXTTRIG arms *external* events for other EXTSEL
        // codes — those fire from the selected timer/EXTI edge, not from
        // SWSTART. Bit 30 (F2/F4 SWSTART) is ignored here.
        let sw_selected = (cr2 & F1_CR2_EXTSEL) == F1_CR2_EXTSEL;
        let swstart_rise = (cr2 & F1_CR2_SWSTART) != 0 && (old_cr2 & F1_CR2_SWSTART) == 0;
        let mut trigger = false;
        if swstart_rise {
            // Self-clearing command bit whether or not the selection is valid.
            cr2 &= !F1_CR2_SWSTART;
            if sw_selected {
                trigger = true;
            }
        }
        // EXTTRIG is part of the F1 trigger contract (external edges not modelled).
        // Touch the constant so a rename/removal cannot silently drop the bit.
        debug_assert_eq!(F1_CR2_EXTTRIG, 1 << 20);

        if let AdcRegs::Stm32F1(r) = &mut self.regs {
            r.cr2 = cr2;
        }
        trigger
    }

    /// F2/F4 CR2 writes: SWSTART (regular) at bit 30, JSWSTART (injected) at
    /// bit 22. Injected start is supported only while idle and with JAUTO
    /// clear; external injected triggers and regular-group preemption remain
    /// outside this bounded model.
    fn apply_f4_cr2(&mut self, old_cr2: u32, new_cr2: u32) -> bool {
        let mut cr2 = new_cr2;
        let adon = (cr2 & F1_CR2_ADON) != 0;
        if !adon {
            self.cancel_f1_pending();
            self.seq_index = 0;
            self.f4_injected_active = false;
            self.f4_injected_sequence_rank = 0;
            cr2 &= !(F4_CR2_SWSTART | F4_CR2_JSWSTART);
            if let AdcRegs::Stm32F4(r) = &mut self.regs {
                r.cr2 = cr2;
            }
            return false;
        }
        let swstart = (cr2 & F4_CR2_SWSTART) != 0;
        let old_swstart = (old_cr2 & F4_CR2_SWSTART) != 0;
        let jswstart = (cr2 & F4_CR2_JSWSTART) != 0;
        let old_jswstart = (old_cr2 & F4_CR2_JSWSTART) != 0;
        let swstart_rise = swstart && !old_swstart;
        let jswstart_rise = jswstart && !old_jswstart;
        if swstart_rise {
            cr2 &= !F4_CR2_SWSTART;
        }
        if jswstart_rise {
            cr2 &= !F4_CR2_JSWSTART;
        }
        let both_started = swstart_rise && jswstart_rise;
        let mut trigger = false;
        if !both_started && swstart_rise && !self.converting {
            self.f4_injected_active = false;
            self.f4_injected_sequence_rank = 0;
            self.seq_index = 0;
            trigger = true;
        } else if !both_started
            && jswstart_rise
            && !self.converting
            && matches!(&self.regs, AdcRegs::Stm32F4(r) if r.cr1 & (F4_CR1_JAUTO | F4_CR1_SCAN) == F4_CR1_SCAN)
        {
            self.f4_injected_active = true;
            self.f4_injected_sequence_rank = 0;
            trigger = true;
        }
        if let AdcRegs::Stm32F4(r) = &mut self.regs {
            r.cr2 = cr2;
        }
        trigger
    }

    /// Advance one model cycle of F1 RSTCAL/CAL countdowns (walk path).
    fn advance_f1_calibration(&mut self) {
        self.advance_f1_rstcal();
        self.advance_f1_cal();
    }

    fn advance_f1_rstcal(&mut self) {
        let Some(left) = self.f1_rstcal_remaining.as_mut() else {
            return;
        };
        if *left > 0 {
            *left -= 1;
        }
        if *left == 0 {
            self.f1_rstcal_remaining = None;
            self.f1_rstcal_live = false;
            if let AdcRegs::Stm32F1(r) = &mut self.regs {
                r.cr2 &= !F1_CR2_RSTCAL;
            }
        }
    }

    fn advance_f1_cal(&mut self) {
        let Some(left) = self.f1_cal_remaining.as_mut() else {
            return;
        };
        if *left > 0 {
            *left -= 1;
        }
        if *left == 0 {
            self.f1_cal_remaining = None;
            self.f1_cal_live = false;
            if let AdcRegs::Stm32F1(r) = &mut self.regs {
                r.cr2 &= !F1_CR2_CAL;
            }
        }
    }
}

impl Default for Adc {
    fn default() -> Self {
        Self::new()
    }
}

impl Peripheral for Adc {
    fn adc_channel_count(&self) -> Option<u8> {
        Some(self.channel_count())
    }

    fn supports_adc_reference_voltage(&self) -> bool {
        true
    }

    fn set_adc_reference_voltage_v(&mut self, volts: f64) -> bool {
        Adc::set_reference_voltage_v(self, volts)
    }

    fn set_ambient_temperature_c(&mut self, celsius: f64) -> bool {
        // Only the F1 layout models its temperature sensor; the other
        // families' internal channels are not this sensor and keep their own
        // documented behaviour.
        matches!(self.regs, AdcRegs::Stm32F1(_)) && Adc::set_ambient_temperature_c(self, celsius)
    }

    fn read(&self, offset: u64) -> SimResult<u8> {
        let val = match &self.regs {
            AdcRegs::Stm32F1(r) => match offset {
                0x00..=0x03 => {
                    if self.dr_read_since_eoc.get() {
                        self.sr & !F4_SR_EOC
                    } else {
                        self.sr
                    }
                }
                0x04..=0x07 => r.cr1,
                0x08..=0x0B => r.cr2,
                0x0C..=0x0F => r.smpr1,
                0x10..=0x13 => r.smpr2,
                0x2C..=0x2F => r.sqr1,
                0x30..=0x33 => r.sqr2,
                0x34..=0x37 => r.sqr3,
                0x4C..=0x4F => {
                    self.dr_read_since_eoc.set(true);
                    self.dr
                }
                _ => {
                    crate::census_reg!("adc:Adc", offset, "read");
                    0
                }
            },
            AdcRegs::Stm32F4(r) => match offset {
                0x00..=0x03 => {
                    if self.dr_read_since_eoc.get() {
                        self.sr & !F4_SR_EOC
                    } else {
                        self.sr
                    }
                }
                0x04..=0x07 => r.cr1,
                0x08..=0x0B => r.cr2,
                0x0C..=0x0F => r.smpr1,
                0x10..=0x13 => r.smpr2,
                0x2C..=0x2F => r.sqr1,
                0x30..=0x33 => r.sqr2,
                0x34..=0x37 => r.sqr3,
                0x38..=0x3B => r.jsqr,
                0x3C..=0x3F => r.jdr[0],
                0x40..=0x43 => r.jdr[1],
                0x44..=0x47 => r.jdr[2],
                0x48..=0x4B => r.jdr[3],
                0x4C..=0x4F => {
                    self.dr_read_since_eoc.set(true);
                    self.dr
                }
                _ => {
                    crate::census_reg!("adc:Adc", offset, "read");
                    0
                }
            },
            AdcRegs::Stm32L4(r) => Self::read_reg_l4(r, self.dr, offset & !3),
            AdcRegs::Stm32H7(r) => Self::read_reg_h7(r, self.dr, offset & !3),
            AdcRegs::Stm32U5(r) => Self::read_reg_u5(r, self.dr, offset & !3),
            AdcRegs::Stm32G0(r) => Self::read_reg_g0(r, self.dr, offset & !3),
        };
        let shift = (offset % 4) * 8;
        Ok(((val >> shift) & 0xFF) as u8)
    }

    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        let shift = (offset % 4) * 8;
        let mask: u32 = 0xFF << shift;
        let val_shifted = (value as u32) << shift;

        match self.regs {
            AdcRegs::Stm32F1(_) | AdcRegs::Stm32F4(_) => match offset {
                0x00..=0x03 => {
                    // RM0008/RM0090 ADC_SR flags are rc_w0: write zero to
                    // clear; writing one leaves the current flag unchanged.
                    self.sr &= !(mask & !val_shifted);
                    if mask & F4_SR_EOC != 0 && val_shifted & F4_SR_EOC == 0 {
                        self.dr_read_since_eoc.set(false);
                    }
                }
                0x04..=0x07 => match &mut self.regs {
                    AdcRegs::Stm32F1(r) | AdcRegs::Stm32F4(r) => {
                        r.cr1 = (r.cr1 & !mask) | val_shifted;
                    }
                    _ => {}
                },
                0x08..=0x0B => {
                    // Byte-merge CR2, then apply family-specific side effects
                    // (F1: bit-22 SWSTART + EXTSEL/EXTTRIG + CAL/RSTCAL;
                    //  F4: bit-30 SWSTART). Release `regs` before start_conversion.
                    let (old_cr2, merged, is_f1) = match &self.regs {
                        AdcRegs::Stm32F1(r) => (r.cr2, (r.cr2 & !mask) | val_shifted, true),
                        AdcRegs::Stm32F4(r) => (r.cr2, (r.cr2 & !mask) | val_shifted, false),
                        _ => (0, 0, false),
                    };
                    let trigger = if is_f1 {
                        self.apply_f1_cr2(old_cr2, merged)
                    } else {
                        self.apply_f4_cr2(old_cr2, merged)
                    };
                    if trigger {
                        self.start_sequence();
                    }
                }
                0x0C..=0x0F | 0x10..=0x13 | 0x2C..=0x2F | 0x30..=0x33 | 0x34..=0x37 => {
                    if let AdcRegs::Stm32F1(r) | AdcRegs::Stm32F4(r) = &mut self.regs {
                        let (reg, writable) = match offset & !3 {
                            // SMP17..10 / SMP9..0, three bits each.
                            0x0C => (&mut r.smpr1, 0x00FF_FFFF),
                            0x10 => (&mut r.smpr2, 0x3FFF_FFFF),
                            // L[23:20] + SQ16..13.
                            0x2C => (&mut r.sqr1, 0x00FF_FFFF),
                            // SQ12..7 / SQ6..1.
                            0x30 => (&mut r.sqr2, 0x3FFF_FFFF),
                            _ => (&mut r.sqr3, 0x3FFF_FFFF),
                        };
                        *reg = ((*reg & !mask) | val_shifted) & writable;
                    }
                }
                0x38..=0x3B => match &mut self.regs {
                    AdcRegs::Stm32F4(r) => {
                        r.jsqr = (r.jsqr & !mask) | val_shifted;
                    }
                    _ => {
                        crate::census_reg!("adc:Adc", offset, "write");
                    }
                },
                _ => {
                    crate::census_reg!("adc:Adc", offset, "write");
                }
            },
            AdcRegs::Stm32L4(_) => {
                let reg = offset & !3;
                let dr = self.dr;
                let calibration_completes = self.calibration_completes;
                let mut cr = 0;
                if let AdcRegs::Stm32L4(r) = &mut self.regs {
                    let full = (Self::read_reg_l4(r, dr, reg) & !mask) | val_shifted;
                    Self::write_reg_l4(r, reg, full, calibration_completes);
                    cr = r.cr;
                }
                // A write touching CR may have set ADSTART — try to convert.
                // Use the stored CR: its rs bits keep ADEN through a write
                // that carries ADEN = 0.
                if reg == 0x08 {
                    self.maybe_start_l4_conversion(cr);
                }
            }
            AdcRegs::Stm32H7(_) => {
                // Same byte-merge discipline as the L4 arm: read the current
                // word, splice in this byte, write the whole word back, so a
                // `strb` to one byte of CR cannot clear the rest of the
                // power-up state.
                let reg = offset & !3;
                let dr = self.dr;
                let full = if let AdcRegs::Stm32H7(r) = &self.regs {
                    (Self::read_reg_h7(r, dr, reg) & !mask) | val_shifted
                } else {
                    0
                };
                self.write_reg_h7(reg, full);
            }
            AdcRegs::Stm32U5(_) => {
                // Same byte-merge discipline as the H7 arm: a `strb` to one
                // byte of CR cannot clear the rest of the power-up state.
                let reg = offset & !3;
                let dr = self.dr;
                let full = if let AdcRegs::Stm32U5(r) = &self.regs {
                    (Self::read_reg_u5(r, dr, reg) & !mask) | val_shifted
                } else {
                    0
                };
                self.write_reg_u5(reg, full);
            }
            AdcRegs::Stm32G0(_) => {
                // Same byte-merge discipline as the H7 and U5 arms; ISR is
                // rc_w1, so a byte write clears only its own bits.
                let reg = offset & !3;
                let dr = self.dr;
                let full = if reg == 0x00 {
                    val_shifted
                } else if let AdcRegs::Stm32G0(r) = &self.regs {
                    (Self::read_reg_g0(r, dr, reg) & !mask) | val_shifted
                } else {
                    0
                };
                self.write_reg_g0(reg, full);
            }
        }
        Ok(())
    }

    fn tick(&mut self) -> PeripheralTickResult {
        // Scheduler-mode instances are walk-skipped; the event chain owns the
        // conversion countdown. Guard against a stray direct call.
        if self.scheduler_mode() {
            return PeripheralTickResult::default();
        }
        self.advance_f1_calibration();
        let irq = self.advance_conversion();
        // Tick-cost normalization (mirrors SysTick B1): the legacy model charged
        // `cycles: 1` per converting tick into `total_cycles` — a sim artifact (a
        // real ADC conversion runs on the ADC clock and consumes zero *core*
        // cycles) that is structurally incompatible with deleting the walk (the
        // scheduler never runs a per-cycle tick to charge it). Both modes now
        // charge zero, so the walk-on reference and the scheduler path agree
        // cycle-for-cycle.
        let dma_signals =
            std::mem::take(&mut self.dma_pending).then(|| vec![crate::DMA_SIGNAL_DATA]);
        PeripheralTickResult {
            irq,
            cycles: 0,
            dma_signals,
            ..Default::default()
        }
    }

    fn uses_scheduler(&self) -> bool {
        self.scheduler_mode()
    }

    fn needs_legacy_walk(&self) -> bool {
        !self.scheduler_mode()
    }

    fn attach_cycle_clock(&mut self, clock: CycleClock) {
        self.clock = Some(clock);
    }

    fn sync_to(&mut self, _now_cycle: u64) {
        // No lazily-accumulated state: the conversion countdown is advanced
        // cycle-by-cycle by the event chain (drained up to the current cycle by
        // `Machine::step` before any MMIO access observes DR/SR).
    }

    fn take_scheduled_events(&mut self) -> Vec<(u64, u32)> {
        // Arm conversion / F1 CAL / F1 RSTCAL chains when work is pending.
        // L4 converts synchronously in `write` and never sets `converting`.
        // delay-0 → deadline `current_cycle + 1` = the walk's next tick.
        if !self.scheduler_mode() {
            return Vec::new();
        }
        let mut out = Vec::new();
        if self.converting && !self.chain_live {
            self.chain_live = true;
            out.push((0u64, self.conv_token()));
        }
        if self.f1_rstcal_remaining.is_some() && !self.f1_rstcal_live {
            self.f1_rstcal_live = true;
            out.push((0u64, F1_EVT_RSTCAL));
        }
        if self.f1_cal_remaining.is_some() && !self.f1_cal_live {
            self.f1_cal_live = true;
            out.push((0u64, F1_EVT_CAL));
        }
        out
    }

    fn on_event(
        &mut self,
        event_token: u32,
        sched: &mut crate::sched::EventScheduler,
        _bus: &mut dyn crate::Bus,
    ) -> crate::sched::EventResult {
        if !self.scheduler_mode() {
            return crate::sched::EventResult::default();
        }
        match event_token {
            F1_EVT_RSTCAL => {
                self.advance_f1_rstcal();
                let still = self.f1_rstcal_remaining.is_some();
                self.f1_rstcal_live = still;
                crate::sched::EventResult {
                    reschedule_delay: still.then_some(1),
                    ..Default::default()
                }
            }
            F1_EVT_CAL => {
                self.advance_f1_cal();
                let still = self.f1_cal_remaining.is_some();
                self.f1_cal_live = still;
                crate::sched::EventResult {
                    reschedule_delay: still.then_some(1),
                    ..Default::default()
                }
            }
            token if token == self.conv_token() => {
                // Conversion countdown — same engine as the walk. The ticks
                // since the last event only decremented `cycles_remaining`
                // (the chain sleeps through them, see below); replay them, then
                // run this cycle's tick.
                if let Some((anchor, _)) = self.conv_sleep.take() {
                    let skipped = sched.now().saturating_sub(anchor).saturating_sub(1);
                    let skipped = skipped.min(u64::from(u32::MAX)) as u32;
                    self.cycles_remaining = self.cycles_remaining.saturating_sub(skipped);
                }
                let irq = self.advance_conversion();
                self.chain_live = self.converting;
                // While `cycles_remaining` ticks down nothing observable
                // changes: SR/DR/EOC and the IRQ move only on the completion
                // tick, which is `cycles_remaining + 1` cycles away. Sleep
                // until then; a write that restarts or cancels the conversion
                // bumps the generation so this wake goes stale.
                let reschedule_delay = self.converting.then(|| {
                    let remaining = self.cycles_remaining;
                    if remaining > 0 {
                        self.conv_sleep = Some((sched.now(), remaining));
                    }
                    u64::from(remaining) + 1
                });
                crate::sched::EventResult {
                    raise_own_irq: irq,
                    reschedule_delay,
                    dma_signals: if std::mem::take(&mut self.dma_pending) {
                        vec![crate::DMA_SIGNAL_DATA]
                    } else {
                        Vec::new()
                    },
                    ..Default::default()
                }
            }
            // A conversion wake of a superseded generation: the write that
            // restarted or cancelled the conversion armed its own.
            _ => crate::sched::EventResult::default(),
        }
    }

    fn as_any(&self) -> Option<&dyn Any> {
        Some(self)
    }
    fn as_any_mut(&mut self) -> Option<&mut dyn Any> {
        Some(self)
    }

    fn snapshot(&self) -> serde_json::Value {
        let mut value = serde_json::to_value(self).unwrap_or(serde_json::Value::Null);
        // An asleep conversion wake has not decremented the countdown yet.
        if self.conv_sleep.is_some() {
            if let Some(slot) = value.get_mut("cycles_remaining") {
                *slot = serde_json::Value::from(self.projected_cycles_remaining());
            }
        }
        value
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// ST-correct F1 software-start sequence: ADON | EXTSEL=111, then SWSTART
    /// at bit 22. Expected masks are the named F1 constants (from ST headers),
    /// never the old production bit-30 value.
    fn f1_software_start(adc: &mut Adc) {
        let cr2 = F1_CR2_ADON | F1_CR2_EXTSEL | F1_CR2_SWSTART;
        adc.write_u32(0x08, F1_CR2_ADON | F1_CR2_EXTSEL).unwrap();
        adc.write_u32(0x08, cr2).unwrap();
    }

    fn f1_software_start_with(adc: &mut Adc, extra_cr2: u32) {
        let base = F1_CR2_ADON | F1_CR2_EXTSEL | extra_cr2;
        adc.write_u32(0x08, base).unwrap();
        adc.write_u32(0x08, base | F1_CR2_SWSTART).unwrap();
    }

    #[test]
    fn test_adc_basic_conversion() {
        let mut adc = Adc::new();
        f1_software_start(&mut adc);

        assert!(adc.converting);
        assert_eq!(adc.cycles_remaining, 14);

        for _ in 0..14 {
            let res = adc.tick();
            assert!(adc.converting);
            assert!(!res.irq);
        }

        let _res = adc.tick();
        assert!(!adc.converting);
        assert_eq!(adc.dr, 1);
        assert!((adc.sr & (1 << 1)) != 0); // EOC
    }

    /// RM0008 §11.12.14: the regular channel comes from SQR3 SQ1[4:0], not from
    /// any CR2 bit field. A seeded channel 5 must land in DR only when SQR3
    /// selects it; selecting channel 0 must return that channel's seed instead.
    #[test]
    fn f1_conversion_channel_comes_from_sqr3_sq1() {
        let mut adc = Adc::new();
        adc.set_channel_input(0, 1650); // ch0 ≈ half scale
        adc.set_channel_input(5, 3300); // ch5 full scale

        // SQR3 read/write round-trips at 0x34.
        adc.write_u32(0x34, 5).unwrap();
        assert_eq!(adc.read_u32(0x34).unwrap(), 5);

        let convert = |adc: &mut Adc| {
            f1_software_start(adc);
            for _ in 0..15 {
                adc.tick();
            }
            assert!((adc.sr & (1 << 1)) != 0, "EOC");
            adc.dr
        };

        let ch5 = adc.channel_input_count(5) as u32;
        assert_eq!(convert(&mut adc), ch5, "SQR3 SQ1 = 5 converts channel 5");

        // Re-select channel 0: the next conversion returns the ch0 seed, not a
        // stale ch5 result. (ADON stays set; SWSTART rising edge retriggers.)
        adc.write_u32(0x34, 0).unwrap();
        let ch0 = adc.channel_input_count(0) as u32;
        assert_eq!(convert(&mut adc), ch0, "SQR3 SQ1 = 0 converts channel 0");
    }

    /// RM0008 §§11.3.8, 11.12.1: SCAN converts the configured SQR3 ranks in
    /// order. EOC/EOCIE mark the end of the regular group, not each rank (the
    /// F1 HAL replaces per-rank polling with a timed wait for this reason),
    /// and reading DR clears EOC.
    #[test]
    fn f1_scan_converts_ranks_in_order_and_dr_read_clears_group_eoc() {
        let mut adc = Adc::new();
        adc.set_channel_input(0, 825);
        adc.set_channel_input(1, 1650);
        adc.write_u32(0x04, F1_CR1_SCAN | F1_CR1_EOCIE).unwrap();
        adc.write_u32(0x2C, 1 << 20).unwrap(); // L=1 -> two ranks
        adc.write_u32(0x34, 1 << 5).unwrap(); // SQ1=IN0, SQ2=IN1

        f1_software_start(&mut adc);
        let first: Vec<_> = (0..15).map(|_| adc.tick().irq).collect();
        assert!(first.iter().all(|irq| !irq), "no EOC IRQ inside the group");
        assert_eq!(adc.read_u32(0x00).unwrap() & F4_SR_EOC, 0);
        assert_eq!(adc.dr, 1023, "rank 1 result is in DR");
        assert_eq!(adc.seq_index, 1);
        assert!(
            adc.converting,
            "the second sequence rank starts automatically"
        );

        let second: Vec<_> = (0..15).map(|_| adc.tick().irq).collect();
        assert!(second[..14].iter().all(|irq| !irq));
        assert!(second[14], "EOCIE reports the end of the regular group");
        assert_ne!(adc.read_u32(0x00).unwrap() & F4_SR_EOC, 0);
        assert_eq!(adc.read_u32(0x4C).unwrap(), 2047);
        assert_eq!(adc.read_u32(0x00).unwrap() & F4_SR_EOC, 0);
        assert!(!adc.converting, "single-shot scan stops at the last rank");
        assert_eq!(adc.seq_index, 0);
    }

    /// RM0008 §11.12.3: DMA requests are generated on each completed F1
    /// regular conversion, allowing DMA1 to read DR before the next scan rank
    /// overwrites it.
    #[test]
    fn f1_scan_emits_dma_request_for_each_completed_rank() {
        let mut adc = Adc::new();
        adc.set_channel_input(0, 825);
        adc.set_channel_input(1, 1650);
        adc.write_u32(0x04, F1_CR1_SCAN).unwrap();
        adc.write_u32(0x2C, 1 << 20).unwrap(); // L=1 -> two ranks
        adc.write_u32(0x34, 1 << 5).unwrap(); // SQ1=IN0, SQ2=IN1

        f1_software_start_with(&mut adc, F1_CR2_DMA);
        for _ in 0..14 {
            assert!(adc.tick().dma_signals.is_none());
        }
        assert_eq!(adc.tick().dma_signals, Some(vec![crate::DMA_SIGNAL_DATA]));
        assert_eq!(adc.read_u32(0x4C).unwrap(), 1023);

        for _ in 0..14 {
            assert!(adc.tick().dma_signals.is_none());
        }
        assert_eq!(adc.tick().dma_signals, Some(vec![crate::DMA_SIGNAL_DATA]));
        assert_eq!(adc.read_u32(0x4C).unwrap(), 2047);
        assert!(!adc.converting);
    }

    /// The F1 ADC samples the selected input and VREF+ when SWSTART begins
    /// acquisition. Changes while the conversion countdown is running belong
    /// to the next sample.
    #[test]
    fn f1_sample_hold_latches_mux_input_and_reference() {
        let mut adc = Adc::new();
        adc.set_channel_input(0, 1650);
        adc.set_channel_input(3, 3300);
        assert!(adc.set_reference_voltage_v(3.3));

        f1_software_start(&mut adc); // channel 0, 1.65 V, 3.3 V VREF+
        adc.set_channel_input(0, 3300);
        assert!(adc.set_reference_voltage_v(1.65));
        adc.write_u32(0x34, 3).unwrap(); // change mux while conversion is live
        for _ in 0..15 {
            adc.tick();
        }
        assert_eq!(
            adc.dr, 2047,
            "the first sample keeps its acquired 1.65 V / 3.3 V"
        );

        f1_software_start(&mut adc);
        for _ in 0..15 {
            adc.tick();
        }
        assert_eq!(
            adc.dr, 4095,
            "the next sample sees channel 3 and 1.65 V VREF+"
        );
    }

    /// `clear_channel_input` releases the held level and restores the no-input
    /// sentinel. After the clear, the same conversion must return
    /// the engine's own value again (here: the counter fallback the basic test
    /// pins at 1), not a stale injected count.
    #[test]
    fn clear_channel_input_returns_channel_to_the_modeled_source() {
        let mut adc = Adc::new();
        let convert = |adc: &mut Adc| {
            f1_software_start(adc);
            for _ in 0..15 {
                adc.tick();
            }
            assert!((adc.sr & (1 << 1)) != 0, "EOC");
            adc.dr
        };

        adc.set_channel_input(0, 1650); // ≈ half scale
        let injected = adc.channel_input_count(0) as u32;
        assert_eq!(injected, (1650 * 4095) / 3300, "mV→count arithmetic");
        assert_eq!(convert(&mut adc), injected, "injected level wins");

        adc.clear_channel_input(0);
        assert_eq!(adc.channel_input_count(0), 0xFFFF, "sentinel restored");
        let freed = convert(&mut adc);
        assert_ne!(freed, injected, "the injection no longer answers");

        // Out-of-range channels are ignored on both paths, not a panic.
        adc.set_channel_input(200, 1650);
        adc.clear_channel_input(200);
    }

    /// Firmware that never programs the sequence converts channel 0 — SQR3
    /// resets to 0 on silicon. This is the ntc-thermistor-lab shape: the demo
    /// firmware triggers conversions with SQR3 untouched, and the NTC seed on
    /// channel 0 must reach DR. (The legacy CR2 low-bits fallback read 1 here
    /// because ADON is CR2 bit 0, so the ch0 seed was never returned.)
    #[test]
    fn f1_unwritten_sqr3_converts_channel_zero() {
        let mut adc = Adc::new();
        adc.set_channel_input(0, 1650);
        f1_software_start(&mut adc);
        for _ in 0..15 {
            adc.tick();
        }
        assert_eq!(adc.dr, adc.channel_input_count(0) as u32);
    }

    /// CR2 low bits must NOT influence the converted channel: with SQR3 = 0,
    /// garbage in CR2[4:0] (here CONT|ADON) still converts channel 0.
    #[test]
    fn f1_cr2_low_bits_do_not_select_the_channel() {
        let mut adc = Adc::new();
        adc.set_channel_input(0, 1650);
        adc.set_channel_input(3, 3300);
        f1_software_start_with(&mut adc, F1_CR2_CONT);
        for _ in 0..15 {
            adc.tick();
        }
        assert_eq!(adc.dr, adc.channel_input_count(0) as u32);
    }

    #[test]
    fn test_adc_interrupt() {
        let mut adc = Adc::new();
        adc.write(0x04, 1 << 5).unwrap(); // EOCIE
        f1_software_start(&mut adc);

        for _ in 0..15 {
            let res = adc.tick();
            if !adc.converting {
                assert!(res.irq);
                return;
            }
        }
        panic!("ADC failed to complete conversion");
    }

    #[test]
    fn test_adc_l4_converts_selected_injected_channel() {
        let mut adc = Adc::new_with_layout(AdcRegisterLayout::Stm32L4);
        adc.set_channel_input(1, 3300);
        adc.write_u32(0x08, 1 << 28).unwrap(); // ADVREGEN, leave DEEPPWD
        adc.write_u32(0x08, (1 << 28) | 1).unwrap(); // ADEN -> ADRDY
        adc.write_u32(0x30, 5 << 6).unwrap(); // SQR1.SQ1 = channel 5

        // Known divider voltages, with literal 12-bit expectations. These do
        // not use the conversion helper to manufacture their own oracle.
        for (millivolts, expected) in [(1650, 2047), (0, 0), (3294, 4087), (548, 680), (2751, 3413)]
        {
            adc.set_channel_input(5, millivolts);
            adc.write(0x00, (1 << 2) | (1 << 3)).unwrap(); // clear EOC/EOS only
            assert_eq!(adc.read_u32(0x00).unwrap() & 0xC, 0);
            adc.write_u32(0x08, (1 << 28) | 1 | (1 << 2)).unwrap();
            assert_eq!(adc.read_u32(0x00).unwrap() & 0xD, 0xD, "ADRDY/EOC/EOS");
            assert_eq!(adc.read_u32(0x08).unwrap() & (1 << 2), 0, "ADSTART clears");
            assert_eq!(adc.read_u32(0x40).unwrap(), expected, "{millivolts} mV");
        }
        adc.write_u32(0x30, 1 << 6).unwrap();
        adc.write_u32(0x08, (1 << 28) | 1 | (1 << 2)).unwrap();
        assert_eq!(adc.read_u32(0x40).unwrap(), 4095, "SQ1 selects channel 1");
        adc.write_u32(0x30, 5 << 6).unwrap();
        adc.write_u32(0x08, (1 << 28) | 1 | (1 << 2)).unwrap();
        assert_eq!(adc.read_u32(0x40).unwrap(), 3413, "channel 5 was preserved");
    }

    #[test]
    fn test_adc_l4_injected_input_uses_resolution_and_current_reference() {
        let mut adc = Adc::new_with_layout(AdcRegisterLayout::Stm32L4);
        adc.set_channel_input(5, 1650);
        adc.write_u32(0x08, 1 << 28).unwrap();
        adc.write_u32(0x08, (1 << 28) | 1).unwrap();
        adc.write_u32(0x30, 5 << 6).unwrap();
        for (res, expected) in [(0, 2047), (1, 511), (2, 127), (3, 31)] {
            adc.write_u32(0x0C, res << 3).unwrap();
            adc.write_u32(0x08, (1 << 28) | 1 | (1 << 2)).unwrap();
            assert_eq!(adc.read_u32(0x40).unwrap(), expected, "CFGR.RES={res}");
        }
        adc.write_u32(0x0C, 0).unwrap();
        // Change VREF after the voltage was injected: conversion must acquire
        // the current reference, not reuse a count cached at injection time.
        for (reference, expected) in [(1.65, 4095), (3.0, 2252), (3.3, 2047)] {
            assert!(adc.set_reference_voltage_v(reference));
            adc.write_u32(0x08, (1 << 28) | 1 | (1 << 2)).unwrap();
            assert_eq!(adc.read_u32(0x40).unwrap(), expected, "VREF={reference}");
        }
    }

    #[test]
    fn test_adc_l4_unbound_channel_retains_internal_source() {
        let mut adc = Adc::new_with_layout(AdcRegisterLayout::Stm32L4);
        adc.set_channel_input(5, 1650);
        adc.write_u32(0x08, 1 << 28).unwrap();
        adc.write_u32(0x08, (1 << 28) | 1).unwrap();
        adc.write_u32(0x30, 6 << 6).unwrap(); // unbound, despite channel 5 injection
        adc.write_u32(0x08, (1 << 28) | 1 | (1 << 2)).unwrap();
        assert_eq!(adc.read_u32(0x40).unwrap(), 3723);
        adc.write_u32(0x30, 5 << 6).unwrap();
        adc.write_u32(0x08, (1 << 28) | 1 | (1 << 2)).unwrap();
        assert_eq!(adc.read_u32(0x40).unwrap(), 2047);
        adc.clear_channel_input(5);
        for (res, expected) in [(0, 3723), (1, 930), (2, 232), (3, 58)] {
            adc.write_u32(0x0C, res << 3).unwrap();
            adc.write_u32(0x08, (1 << 28) | 1 | (1 << 2)).unwrap();
            assert_eq!(adc.read_u32(0x40).unwrap(), expected, "unbound RES={res}");
        }
    }

    #[test]
    fn test_adc_l4_reset_values() {
        let adc = Adc::new_with_layout(AdcRegisterLayout::Stm32L4);
        // CR (0x08) DEEPPWD=bit29, CFGR (0x0C) JQDIS=bit31 — silicon-verified.
        let cr = (adc.read(0x08).unwrap() as u32)
            | (adc.read(0x09).unwrap() as u32) << 8
            | (adc.read(0x0A).unwrap() as u32) << 16
            | (adc.read(0x0B).unwrap() as u32) << 24;
        assert_eq!(cr, 0x2000_0000);
        let cfgr = (adc.read(0x0C).unwrap() as u32) | (adc.read(0x0F).unwrap() as u32) << 24;
        assert_eq!(cfgr & 0x8000_0000, 0x8000_0000);
    }

    #[test]
    fn test_adc_l4_aden_raises_adrdy() {
        // Power-up sequence, silicon-verified on STM32H563 ADC1 (2026-06-11):
        // clear DEEPPWD, set ADVREGEN, then ADEN -> ISR.ADRDY rises.
        let mut adc = Adc::new_with_layout(AdcRegisterLayout::Stm32L4);
        adc.write_u32(0x08, 0).unwrap(); // DEEPPWD = 0
        adc.write_u32(0x08, 1 << 28).unwrap(); // ADVREGEN
        assert_eq!(adc.read_u32(0x00).unwrap() & 0x1, 0, "no ADRDY before ADEN");
        adc.write_u32(0x08, (1 << 28) | 1).unwrap(); // ADEN
        assert_eq!(adc.read_u32(0x08).unwrap(), 0x1000_0001);
        assert_eq!(adc.read_u32(0x00).unwrap() & 0x1, 0x1, "ADRDY after ADEN");

        // ADEN while still in deep power-down must NOT ready the ADC.
        let mut cold = Adc::new_with_layout(AdcRegisterLayout::Stm32L4);
        cold.write_u32(0x08, (1 << 29) | 1).unwrap();
        assert_eq!(cold.read_u32(0x00).unwrap() & 0x1, 0);
    }

    /// ADCAL is a self-clearing command bit, and a HAL is entitled to spin on
    /// it. `while (ADC->CR & ADC_CR_ADCAL);` is the sequence ST's own driver
    /// runs, so a model that latches bit 31 hangs the firmware rather than
    /// failing it — the worst shape of defect, because it looks like a stall
    /// in the CPU rather than a wrong value in a peripheral.
    ///
    /// Regression: this bit staying set is what moved stm32h563 onto the
    /// `stm32h7` ADC profile, whose 3-bit `CFGR.RES[4:2]` then read the L4's
    /// 2-bit `RES[4:3]` write as 16-bit and turned `TIER1 adc` red.
    ///
    /// The mirror of this is [`test_adc_l4_adcal_latches_without_a_kernel_clock`]:
    /// the plain L4 case must NOT clear it. Both are silicon; they differ in
    /// what the firmware clocked, not in what the hardware does.
    #[test]
    fn test_adc_h5_adcal_self_clears() {
        let mut adc = Adc::new_with_layout(AdcRegisterLayout::Stm32H5);
        adc.write_u32(0x08, 0).unwrap(); // leave deep power-down
        adc.write_u32(0x08, 1 << 28).unwrap(); // ADVREGEN

        adc.write_u32(0x08, (1 << 28) | (1 << 31)).unwrap(); // ADVREGEN | ADCAL
        assert_eq!(
            adc.read_u32(0x08).unwrap() & (1 << 31),
            0,
            "ADCAL must read back clear — a HAL polling it would spin forever"
        );
        assert_eq!(
            adc.read_u32(0x08).unwrap(),
            1 << 28,
            "clearing ADCAL must not disturb the rest of CR"
        );

        // And calibration must not be a back door to readiness: ADRDY still
        // requires ADEN.
        assert_eq!(
            adc.read_u32(0x00).unwrap() & 0x1,
            0,
            "no ADRDY without ADEN"
        );
        adc.write_u32(0x08, (1 << 28) | 1).unwrap();
        assert_eq!(adc.read_u32(0x00).unwrap() & 0x1, 0x1);
    }

    /// The L476 keeps ADCAL SET, and that is not a bug to be tidied away.
    ///
    /// Captured on NUCLEO-L476RG (see `firmware_survival`'s `nucleo_l476rg_adc`
    /// case, which reads `CR=90000000` after a calibration request): the smoke
    /// firmware enables only `AHB2ENR.ADCEN` and never configures `CCIPR`, so
    /// the converter has no kernel clock, calibration cannot run, and the
    /// command bit never clears.
    ///
    /// This test exists because the first fix for the H5 hang made ADCAL
    /// always self-clear and silently contradicted this capture. The survival
    /// fixture caught it; nothing in this file did.
    #[test]
    fn test_adc_l4_adcal_latches_without_a_kernel_clock() {
        let mut adc = Adc::new_with_layout(AdcRegisterLayout::Stm32L4);
        adc.write_u32(0x08, 0).unwrap(); // leave deep power-down
        adc.write_u32(0x08, 1 << 28).unwrap(); // ADVREGEN
        adc.write_u32(0x08, (1 << 28) | (1 << 31)).unwrap(); // ADCAL
        assert_eq!(
            adc.read_u32(0x08).unwrap(),
            0x9000_0000,
            "NUCLEO-L476RG silicon: ADCAL stays set with no ADC kernel clock"
        );
    }

    /// The synchronous clock is a kernel clock too. `ADC_CCR.CKMODE` = 01/10/11
    /// clocks the converter from HCLK/1, /2 or /4 with no `CCIPR.ADCSEL`
    /// source, so calibration runs and ADCAL clears. Stock STM32duino on the
    /// L4 (NUCLEO-L432KC, NUCLEO-L476RG) writes CKMODE = 11 before ADCAL;
    /// before this, its `analogRead()` timed out and returned 0.
    #[test]
    fn test_adc_l4_adcal_clears_on_the_synchronous_clock() {
        let mut adc = Adc::new_with_layout(AdcRegisterLayout::Stm32L4);
        adc.write_u32(0x308, 0b11 << 16).unwrap(); // CCR.CKMODE = HCLK/4
        adc.write_u32(0x08, 0).unwrap(); // leave deep power-down
        adc.write_u32(0x08, 1 << 28).unwrap(); // ADVREGEN
        adc.write_u32(0x08, (1 << 28) | (1 << 31)).unwrap(); // ADCAL
        assert_eq!(
            adc.read_u32(0x08).unwrap(),
            1 << 28,
            "ADCAL must clear when CKMODE selects the synchronous clock"
        );

        // CKMODE back to 00 (asynchronous, CCIPR-sourced) latches it again.
        adc.write_u32(0x308, 0).unwrap();
        adc.write_u32(0x08, (1 << 28) | (1 << 31)).unwrap();
        assert_eq!(adc.read_u32(0x08).unwrap(), 0x9000_0000);
    }

    /// The CR command bits are "rs": writing 0 leaves them alone. The L4 HAL
    /// starts a conversion with MODIFY_REG(CR, ADC_CR_BITS_PROPERTY_RS,
    /// ADSTART), a write that carries ADEN = 0; on silicon the ADC stays
    /// enabled and converts. ADSTP and ADDIS complete and clear their bits.
    #[test]
    fn test_adc_l4_cr_command_bits_are_read_set() {
        let mut adc = Adc::new_with_layout(AdcRegisterLayout::Stm32L4);
        adc.set_channel_input(5, 1650);
        adc.write_u32(0x08, 0).unwrap(); // leave deep power-down
        adc.write_u32(0x08, 1 << 28).unwrap(); // ADVREGEN
        adc.write_u32(0x08, (1 << 28) | 1).unwrap(); // ADEN
        assert_eq!(adc.read_u32(0x00).unwrap() & 0x1, 0x1, "ADRDY");
        adc.write_u32(0x30, 5 << 6).unwrap(); // SQ1 = IN5, L = 0

        // The HAL's start: every rs bit written 0 except ADSTART.
        adc.write_u32(0x08, (1 << 28) | (1 << 2)).unwrap();
        assert_eq!(adc.read_u32(0x08).unwrap() & 0x1, 0x1, "ADEN must survive");
        assert_eq!(adc.read_u32(0x00).unwrap() & (1 << 2), 1 << 2, "EOC");
        // 1650 mV of 3300 mV at 12 bits (CFGR.RES = 00).
        assert_eq!(adc.read_u32(0x40).unwrap(), 2047);

        // ADSTP clears ADSTART; ADDIS clears ADEN.
        adc.write_u32(0x08, (1 << 28) | (1 << 4)).unwrap();
        assert_eq!(adc.read_u32(0x08).unwrap() & ((1 << 4) | (1 << 2)), 0);
        adc.write_u32(0x08, (1 << 28) | (1 << 1)).unwrap();
        assert_eq!(adc.read_u32(0x08).unwrap(), 1 << 28, "ADDIS clears ADEN");
    }

    /// The H7 clears ADCAL too, and nothing asserted it. Found by accident:
    /// a mis-aimed negative control deleted the H7's `& !(1 << 31)` and the
    /// whole ADC suite stayed green. Same defect as the L4 had, one layout
    /// over, and the same hang on any HAL that polls the bit.
    #[test]
    fn test_adc_h7_adcal_self_clears() {
        let mut adc = Adc::new_with_layout(AdcRegisterLayout::Stm32H7);
        adc.write_u32(0x08, 0).unwrap(); // leave deep power-down
        adc.write_u32(0x08, 1 << 28).unwrap(); // ADVREGEN
        adc.write_u32(0x08, (1 << 28) | (1 << 31)).unwrap(); // ADCAL
        assert_eq!(
            adc.read_u32(0x08).unwrap() & (1 << 31),
            0,
            "ADCAL must read back clear on the H7 as well"
        );
        assert_eq!(adc.read_u32(0x08).unwrap(), 1 << 28);
    }

    /// The H563 is an L4-class ADC: `CFGR.RES` is two bits at [4:3], so the
    /// same firmware write means 12-bit here and 16-bit on the H7. This is the
    /// exact divergence that produced `stm32h563/adc: pass -> blocked`, and it
    /// is asserted on both layouts so neither can drift onto the other again.
    #[test]
    fn test_res_field_placement_differs_between_l4_and_h7() {
        // RES = 0 written at the L4's [4:3] is 12-bit on the L4 …
        let mut l4 = Adc::new_with_layout(AdcRegisterLayout::Stm32L4);
        l4.write_u32(0x08, 0).unwrap();
        l4.write_u32(0x08, 1 << 28).unwrap();
        l4.write_u32(0x08, (1 << 28) | 1).unwrap();
        l4.write_u32(0x0C, 0).unwrap();
        l4.write_u32(0x00, 1 << 2).unwrap();
        l4.write_u32(0x08, (1 << 28) | 1 | (1 << 2)).unwrap();
        assert_eq!(l4.read_u32(0x40).unwrap() & 0xFFFF, 3723, "L4 12-bit code");

        // … and 16-bit on the H7, from the identical CFGR write.
        assert_eq!(h7_resolution_bits(0), 16);
        assert_eq!(l4_adc_code(12), 3723);
        assert_eq!(scaled_adc_code(16), 3723 << 4);
    }

    /// L4 ADSTART converts the fixed internal source: DR holds a derived code,
    /// ISR.EOC rises, and the code scales when firmware narrows CFGR.RES.
    #[test]
    fn test_adc_l4_conversion_scales_with_resolution() {
        let convert = |res: u32| -> (u32, u32) {
            let mut adc = Adc::new_with_layout(AdcRegisterLayout::Stm32L4);
            adc.write_u32(0x08, 0).unwrap(); // DEEPPWD = 0
            adc.write_u32(0x08, 1 << 28).unwrap(); // ADVREGEN
            adc.write_u32(0x08, (1 << 28) | 1).unwrap(); // ADEN -> ADRDY
            assert_eq!(adc.read_u32(0x00).unwrap() & 0x1, 1, "ADRDY");
            adc.write_u32(0x0C, res << 3).unwrap(); // CFGR.RES
            adc.write_u32(0x08, adc.read_u32(0x08).unwrap() | (1 << 2))
                .unwrap(); // ADSTART
            let isr = adc.read_u32(0x00).unwrap();
            (adc.read_u32(0x40).unwrap() & 0xFFFF, isr)
        };

        // No conversion without ADSTART.
        let mut idle = Adc::new_with_layout(AdcRegisterLayout::Stm32L4);
        idle.write_u32(0x08, 0).unwrap();
        idle.write_u32(0x08, (1 << 28) | 1).unwrap();
        assert_eq!(idle.read_u32(0x40).unwrap(), 0, "DR stays 0 until ADSTART");
        assert_eq!(idle.read_u32(0x00).unwrap() & (1 << 2), 0, "no EOC");

        let (dr12, isr12) = convert(0); // 12-bit
        let (dr10, _) = convert(1); // 10-bit
        let (dr8, _) = convert(2); // 8-bit

        assert_ne!(isr12 & (1 << 2), 0, "EOC set after conversion");
        assert_eq!(dr12, 3723, "12-bit code = (3.0/3.3) * 4096");
        assert_eq!(dr10, 930, "10-bit code = 3723 >> 2");
        assert_eq!(dr8, 232, "8-bit code = 3723 >> 4");
        assert!(
            dr10 < dr12 && dr8 < dr10,
            "code scales down with resolution"
        );
    }

    #[test]
    fn l4_converts_the_sq1_channel_held_input() {
        // NUCLEO-L476RG A0 = PA0 = ADC1_IN5 (stm32duino PeripheralPins.c).
        let start = |adc: &mut Adc, sq1: u32, cfgr: u32| -> u32 {
            adc.write_u32(0x08, 0).unwrap(); // DEEPPWD = 0
            adc.write_u32(0x08, (1 << 28) | 1).unwrap(); // ADVREGEN | ADEN
            adc.write_u32(0x0C, cfgr).unwrap();
            adc.write_u32(0x30, sq1 << 6).unwrap(); // SQR1.SQ1, L = 0
                                                    // HAL LL_ADC_REG_StartConversion: MODIFY_REG(CR,
                                                    // ADC_CR_BITS_PROPERTY_RS, ADSTART) writes ADEN as 0.
            let cr = adc.read_u32(0x08).unwrap();
            adc.write_u32(0x08, (cr & !0x8000_003F) | (1 << 2)).unwrap();
            adc.read_u32(0x40).unwrap()
        };
        let mut adc = Adc::new_with_layout(AdcRegisterLayout::Stm32L4);
        adc.set_channel_input(5, 1651);
        adc.set_channel_input(1, 825);
        assert_eq!(start(&mut adc, 5, 0), 2048, "ch5 1651 mV at 12 bits");
        // 825 mV sits on the 1024 code edge; the shared quantizer lands below it.
        assert_eq!(start(&mut adc, 1, 0), 1023, "ch1 825 mV: SQ1 selects");
        assert_eq!(start(&mut adc, 5, 1 << 3), 512, "ch5 at 10 bits");
        assert_eq!(start(&mut adc, 7, 0), 3723, "unheld ch7: fixed source");
        assert_eq!(adc.read_u32(0x08).unwrap() & (1 << 2), 0, "ADSTART clears");

        start(&mut adc, 5, 1 << 13); // CONT
        assert_ne!(
            adc.read_u32(0x08).unwrap() & (1 << 2),
            0,
            "ADSTART stays set in continuous mode"
        );
        assert_ne!(adc.read_u32(0x08).unwrap() & 0x1, 0, "ADEN survives");

        // HAL_ADC_Stop: ADSTP completes, then ADDIS clears ADEN.
        let cr = adc.read_u32(0x08).unwrap();
        adc.write_u32(0x08, (cr & !0x8000_003F) | (1 << 4)).unwrap();
        assert_eq!(
            adc.read_u32(0x08).unwrap() & 0x14,
            0,
            "ADSTP, ADSTART clear"
        );
        let cr = adc.read_u32(0x08).unwrap();
        adc.write_u32(0x08, (cr & !0x8000_003F) | (1 << 1)).unwrap();
        assert_eq!(adc.read_u32(0x08).unwrap() & 0x3, 0, "ADDIS, ADEN clear");
    }

    #[test]
    fn tick_cost_is_normalized_to_zero_while_converting() {
        // The legacy `cycles: 1` per converting tick is gone in BOTH modes.
        let mut adc = Adc::new();
        f1_software_start(&mut adc);
        assert!(adc.converting);
        for _ in 0..14 {
            assert_eq!(
                adc.tick().cycles,
                0,
                "converting tick must charge zero cost"
            );
        }
    }

    #[test]
    fn test_l4_adc_code_helpers() {
        assert_eq!(l4_resolution_bits(0), 12);
        assert_eq!(l4_resolution_bits(1 << 3), 10);
        assert_eq!(l4_resolution_bits(2 << 3), 8);
        assert_eq!(l4_resolution_bits(3 << 3), 6);
        assert_eq!(l4_adc_code(12), 3723);
        assert_eq!(l4_adc_code(10), 930);
        assert_ne!(l4_adc_code(12), l4_adc_code(10));
    }

    // ── STM32U5 ADC1 (RM0456; vendored tests/fixtures/real_world/stm32u575.svd) ──

    #[test]
    fn u5_reset_values_match_the_svd() {
        let adc = Adc::new_with_layout(AdcRegisterLayout::Stm32U5);
        assert_eq!(adc.read_u32(0x08).unwrap(), 0x2000_0000, "CR: DEEPPWD set");
        assert_eq!(adc.read_u32(0x0C).unwrap(), 0x8000_0000, "CFGR1: JQDIS set");
        assert_eq!(adc.read_u32(0x1C).unwrap(), 0, "PCSEL");
        assert_eq!(adc.read_u32(0xA8).unwrap(), 0, "LTR1");
        assert_eq!(adc.read_u32(0xAC).unwrap(), 0x01FF_FFFF, "HTR1");
        assert_eq!(adc.read_u32(0xB0).unwrap(), 0, "LTR2");
        assert_eq!(adc.read_u32(0xB4).unwrap(), 0x01FF_FFFF, "HTR2");
        assert_eq!(adc.read_u32(0xB8).unwrap(), 0, "LTR3");
        assert_eq!(adc.read_u32(0xBC).unwrap(), 0x01FF_FFFF, "HTR3");
        assert_eq!(adc.read_u32(0xC8).unwrap(), 0, "CALFACT2");
        // The H7 puts LTR1/HTR1 at 0x20/0x24; on the U5 those slots are unmapped.
        assert_eq!(adc.read_u32(0x20).unwrap(), 0, "no H7 LTR1 at 0x20");
        assert_eq!(adc.read_u32(0x24).unwrap(), 0, "no H7 HTR1 at 0x24");
    }

    #[test]
    fn u5_cfgr1_res_pcsel_and_watchdogs_round_trip() {
        let mut adc = Adc::new_with_layout(AdcRegisterLayout::Stm32U5);
        adc.write_u32(0x0C, 0x8000_0000 | (0b10 << 2)).unwrap();
        assert_eq!(
            adc.read_u32(0x0C).unwrap() & 0xC,
            0b10 << 2,
            "CFGR1.RES is [3:2]"
        );
        adc.write_u32(0x1C, 0x000F_FFFF).unwrap();
        assert_eq!(adc.read_u32(0x1C).unwrap(), 0x000F_FFFF, "PCSEL20");
        adc.write_u32(0xA8, 0x0000_0ABC).unwrap();
        adc.write_u32(0xAC, 0x0000_1234).unwrap();
        adc.write_u32(0xB4, 0x0000_0DEF).unwrap();
        adc.write_u32(0xBC, 0x0000_5678).unwrap();
        assert_eq!(adc.read_u32(0xA8).unwrap(), 0x0000_0ABC, "LTR1");
        assert_eq!(adc.read_u32(0xAC).unwrap(), 0x0000_1234, "HTR1");
        assert_eq!(adc.read_u32(0xB4).unwrap(), 0x0000_0DEF, "HTR2");
        assert_eq!(adc.read_u32(0xBC).unwrap(), 0x0000_5678, "HTR3");
        adc.write_u32(0xC8, 0x0000_2A00).unwrap();
        assert_eq!(adc.read_u32(0xC8).unwrap(), 0x0000_2A00, "CALFACT2");
        // ADC12 common CCR @ 0x308 (ADC12 @ 0x42028300) stays addressable.
        adc.write_u32(0x308, 0x0003_0000).unwrap();
        assert_eq!(adc.read_u32(0x308).unwrap(), 0x0003_0000, "ADC12_CCR");
    }

    #[test]
    fn u5_converts_at_14_bit_default_and_scales_with_res() {
        let convert = |res: u32| -> (u32, u32) {
            let mut adc = Adc::new_with_layout(AdcRegisterLayout::Stm32U5);
            adc.write_u32(0x08, 0).unwrap(); // leave deep power-down
            adc.write_u32(0x08, 1 << 28).unwrap(); // ADVREGEN
            adc.write_u32(0x08, (1 << 28) | 1).unwrap(); // ADEN -> ADRDY
            assert_eq!(adc.read_u32(0x00).unwrap() & 0x1, 0x1, "ADRDY");
            adc.write_u32(0x0C, 0x8000_0000 | (res << 2)).unwrap();
            adc.write_u32(0x08, (1 << 28) | 1 | (1 << 2)).unwrap(); // ADSTART
            (adc.read_u32(0x40).unwrap(), adc.read_u32(0x00).unwrap())
        };

        let (dr14, isr14) = convert(0b00);
        assert_eq!(dr14, 3723 << 2, "14-bit default code = 12-bit << 2");
        assert_ne!(isr14 & (1 << 2), 0, "EOC");
        assert_ne!(isr14 & (1 << 3), 0, "EOS");
        assert_eq!(convert(0b01).0, 3723, "RES=01 -> 12 bits");
        assert_eq!(convert(0b10).0, 930, "RES=10 -> 10 bits");
        assert_eq!(convert(0b11).0, 232, "RES=11 -> 8 bits");

        // ADSTART self-clears after a single conversion.
        let mut adc = Adc::new_with_layout(AdcRegisterLayout::Stm32U5);
        adc.write_u32(0x08, 0).unwrap();
        adc.write_u32(0x08, (1 << 28) | 1 | (1 << 2)).unwrap();
        assert_eq!(adc.read_u32(0x08).unwrap() & (1 << 2), 0, "ADSTART clears");
    }

    #[test]
    fn u5_adcal_self_clears() {
        let mut adc = Adc::new_with_layout(AdcRegisterLayout::Stm32U5);
        adc.write_u32(0x08, 0).unwrap();
        adc.write_u32(0x08, 1 << 28).unwrap();
        adc.write_u32(0x08, (1 << 28) | (1 << 31)).unwrap(); // ADCAL
        assert_eq!(
            adc.read_u32(0x08).unwrap(),
            1 << 28,
            "ADCAL must self-clear or a HAL polling it spins forever"
        );
    }

    #[test]
    fn u5_layout_is_selected_by_name() {
        for name in ["stm32u5", "u5"] {
            assert_eq!(
                name.parse::<AdcRegisterLayout>().unwrap(),
                AdcRegisterLayout::Stm32U5,
                "{name}"
            );
        }
        assert_ne!(
            "u5".parse::<AdcRegisterLayout>().unwrap(),
            AdcRegisterLayout::Stm32H7
        );
    }

    #[test]
    fn g0_layout_is_selected_by_name_and_is_not_the_l4() {
        for name in ["stm32g0", "g0", "stm32c0", "c0", "stm32l0", "l0"] {
            assert_eq!(
                name.parse::<AdcRegisterLayout>().unwrap(),
                AdcRegisterLayout::Stm32G0,
                "{name}"
            );
        }
        let adc = Adc::new_with_layout(AdcRegisterLayout::Stm32G0);
        assert_eq!(
            adc.read_u32(0x08).unwrap(),
            0,
            "G0 CR resets to 0 (no DEEPPWD)"
        );
        assert_eq!(
            adc.read_u32(0x0C).unwrap(),
            0,
            "CFGR1 resets to 0 (no JQDIS)"
        );
        for awdtr in [0x20, 0x24, 0x2C] {
            assert_eq!(adc.read_u32(awdtr).unwrap(), 0x0FFF_0000, "{awdtr:#x}");
        }
    }

    /// The register sequence stock STM32duino `analogRead()` runs on a G0/C0
    /// (HAL_ADC_Init, ConfigChannel, Calibration_Start, Start, PollForConversion,
    /// GetValue, Stop, DeInit). Every wait in it must end.
    #[test]
    fn g0_stock_hal_sequence_converts_the_chselr_channel() {
        let mut adc = Adc::new_with_layout(AdcRegisterLayout::Stm32G0);
        adc.set_channel_input(0, 1650);
        adc.set_channel_input(1, 3000);

        adc.write_u32(0x08, G0_CR_ADVREGEN).unwrap();
        adc.write_u32(0x28, 1 << 0).unwrap(); // CHSELR: IN0 (fixed sequencer)
        assert_ne!(adc.read_u32(0x00).unwrap() & G0_ISR_CCRDY, 0, "CCRDY");

        adc.write_u32(0x08, G0_CR_ADVREGEN | G0_CR_ADCAL).unwrap();
        assert_eq!(adc.read_u32(0x08).unwrap() & G0_CR_ADCAL, 0, "ADCAL clears");
        assert_ne!(adc.read_u32(0x00).unwrap() & G0_ISR_EOCAL, 0, "EOCAL");

        adc.write_u32(0x08, G0_CR_ADVREGEN | G0_CR_ADEN).unwrap();
        assert_ne!(adc.read_u32(0x00).unwrap() & G0_ISR_ADRDY, 0, "ADRDY");

        adc.write_u32(0x08, G0_CR_ADVREGEN | G0_CR_ADSTART).unwrap();
        let isr = adc.read_u32(0x00).unwrap();
        assert_eq!(isr & (G0_ISR_EOC | G0_ISR_EOS), G0_ISR_EOC | G0_ISR_EOS);
        assert_eq!(
            adc.read_u32(0x40).unwrap(),
            2047,
            "1650 mV of 3300 at 12 bits"
        );
        let cr = adc.read_u32(0x08).unwrap();
        assert_eq!(cr & G0_CR_ADSTART, 0, "single mode: ADSTART clears");
        assert_ne!(cr & G0_CR_ADEN, 0, "a 0 written to rs ADEN is ignored");

        adc.write_u32(0x08, G0_CR_ADVREGEN | G0_CR_ADDIS).unwrap();
        assert_eq!(
            adc.read_u32(0x08).unwrap(),
            G0_CR_ADVREGEN,
            "ADDIS disables"
        );
    }

    #[test]
    fn g0_sequence_order_and_resolution() {
        let mut adc = Adc::new_with_layout(AdcRegisterLayout::Stm32G0);
        adc.set_channel_input(1, 1650);
        adc.set_channel_input(4, 825);
        let convert = |adc: &mut Adc, cfgr1: u32, chselr: u32| {
            adc.write_u32(0x08, 0).unwrap();
            adc.write_u32(0x08, G0_CR_ADVREGEN | G0_CR_ADDIS).unwrap();
            adc.write_u32(0x0C, cfgr1).unwrap();
            adc.write_u32(0x28, chselr).unwrap();
            adc.write_u32(0x08, G0_CR_ADVREGEN | G0_CR_ADEN).unwrap();
            adc.write_u32(0x08, G0_CR_ADVREGEN | G0_CR_ADEN | G0_CR_ADSTART)
                .unwrap();
            adc.read_u32(0x40).unwrap()
        };
        // Bitmap: lowest channel first, highest first under SCANDIR.
        assert_eq!(convert(&mut adc, 0, 0b1_0010), 2047);
        assert_eq!(convert(&mut adc, G0_CFGR1_SCANDIR, 0b1_0010), 1023);
        // CHSELRMOD ranks: SQ1 = IN4, then end of sequence.
        assert_eq!(convert(&mut adc, G0_CFGR1_CHSELRMOD, 0xFFFF_FFF4), 1023);
        // RES = 10 bits, then left-aligned 12 bits.
        assert_eq!(convert(&mut adc, 1 << 3, 0b10), 511);
        assert_eq!(convert(&mut adc, G0_CFGR1_ALIGN, 0b10), 2047 << 4);
    }

    /// ST headers (`ADC_CR2_SWSTART = 0x00400000`) — bit 22, not bit 30.
    #[test]
    fn f1_swstart_constant_matches_st_headers() {
        assert_eq!(F1_CR2_SWSTART, 0x0040_0000);
        assert_eq!(F1_CR2_EXTSEL, 0x000E_0000);
        assert_eq!(F1_CR2_EXTTRIG, 0x0010_0000);
        assert_eq!(F1_CR2_CAL, 0x0000_0004);
        assert_eq!(F1_CR2_RSTCAL, 0x0000_0008);
        assert_ne!(F1_CR2_SWSTART, F4_CR2_SWSTART);
        assert_eq!(F4_CR2_SWSTART, 0x4000_0000);
        assert_eq!(
            "stm32f1".parse::<AdcRegisterLayout>().unwrap(),
            AdcRegisterLayout::Stm32F1
        );
        assert_eq!(
            "legacy".parse::<AdcRegisterLayout>().unwrap(),
            AdcRegisterLayout::Stm32F4
        );
    }

    /// F103 ST HAL path: RSTCAL then CAL complete (bits self-clear) with ADON.
    #[test]
    fn f103_calibration_rstcal_and_cal_complete() {
        let mut adc = Adc::new();
        adc.write_u32(0x08, F1_CR2_ADON).unwrap();

        adc.write_u32(0x08, F1_CR2_ADON | F1_CR2_RSTCAL).unwrap();
        assert_ne!(
            adc.read_u32(0x08).unwrap() & F1_CR2_RSTCAL,
            0,
            "RSTCAL stays set until scheduled completion"
        );
        for _ in 0..(F1_RSTCAL_CYCLES + 1) {
            adc.tick();
        }
        assert_eq!(
            adc.read_u32(0x08).unwrap() & F1_CR2_RSTCAL,
            0,
            "RSTCAL must clear — HAL spins while it is set"
        );

        adc.write_u32(0x08, F1_CR2_ADON | F1_CR2_CAL).unwrap();
        assert_ne!(adc.read_u32(0x08).unwrap() & F1_CR2_CAL, 0);
        for _ in 0..(F1_CAL_CYCLES + 1) {
            adc.tick();
        }
        assert_eq!(
            adc.read_u32(0x08).unwrap() & F1_CR2_CAL,
            0,
            "CAL must clear — HAL spins while it is set"
        );
        assert_eq!(
            adc.read_u32(0x08).unwrap() & F1_CR2_ADON,
            F1_CR2_ADON,
            "calibration must not clear ADON"
        );
    }

    /// Bit 22 with EXTSEL=111 starts conversion; bit 30 alone does not on F1.
    #[test]
    fn f103_software_start_bit22_not_bit30() {
        let mut adc = Adc::new();
        adc.set_channel_input(0, 1650);

        // Bit 30 alone (the old buggy constant) must NOT start an F1 conversion.
        adc.write_u32(0x08, F1_CR2_ADON | F4_CR2_SWSTART).unwrap();
        assert!(!adc.converting, "F1 must ignore bit 30 (F2/F4 SWSTART)");
        assert_eq!(
            adc.read_u32(0x08).unwrap() & F4_CR2_SWSTART,
            F4_CR2_SWSTART,
            "ignored bit 30 is just a stored CR2 bit on F1"
        );

        // EXTSEL not software → SWSTART bit 22 clears but does not convert.
        adc.write_u32(0x08, F1_CR2_ADON).unwrap();
        adc.write_u32(0x08, F1_CR2_ADON | F1_CR2_SWSTART).unwrap();
        assert!(
            !adc.converting,
            "SWSTART without EXTSEL=111 must not convert"
        );
        assert_eq!(adc.read_u32(0x08).unwrap() & F1_CR2_SWSTART, 0);

        // ST-correct path.
        f1_software_start(&mut adc);
        assert!(adc.converting);
        for _ in 0..15 {
            adc.tick();
        }
        assert_eq!(adc.dr, adc.channel_input_count(0) as u32);
    }

    /// Two independently seeded channels convert to their own counts.
    #[test]
    fn f103_two_seeded_channels_convert_independently() {
        let mut adc = Adc::new();
        adc.set_channel_input(1, 1100);
        adc.set_channel_input(4, 2750);
        let c1 = adc.channel_input_count(1) as u32;
        let c4 = adc.channel_input_count(4) as u32;
        assert_ne!(c1, c4);

        adc.write_u32(0x34, 1).unwrap(); // SQR3 SQ1 = ch1
        f1_software_start(&mut adc);
        for _ in 0..15 {
            adc.tick();
        }
        assert_eq!(adc.dr, c1);

        adc.write_u32(0x34, 4).unwrap();
        f1_software_start(&mut adc);
        for _ in 0..15 {
            adc.tick();
        }
        assert_eq!(adc.dr, c4);
    }

    /// ADON clear cancels an in-flight CAL so a HAL cannot hang on a dead bit.
    #[test]
    fn f103_adon_clear_cancels_pending_calibration() {
        let mut adc = Adc::new();
        adc.write_u32(0x08, F1_CR2_ADON | F1_CR2_CAL).unwrap();
        assert!(adc.f1_cal_remaining.is_some());
        adc.write_u32(0x08, 0).unwrap(); // ADON off
        assert!(adc.f1_cal_remaining.is_none());
        assert_eq!(adc.read_u32(0x08).unwrap() & F1_CR2_CAL, 0);
    }

    /// F4 `CR1.RES[25:24]` narrows a held input the way the SAR truncates:
    /// 1650 mV of 3.3 V is 2047 at 12 bits, 511 at 10, 127 at 8, 31 at 6.
    /// STM32duino programs 10 bits in hardware and does not re-map.
    #[test]
    fn f4_cr1_res_narrows_held_input() {
        for (res, want) in [(0u32, 2047u32), (1, 511), (2, 127), (3, 31)] {
            let mut adc = Adc::new_with_layout(AdcRegisterLayout::Stm32F4);
            adc.set_channel_input(0, 1650);
            adc.write_u32(0x04, res << 24).unwrap();
            adc.write_u32(0x08, F1_CR2_ADON | F4_CR2_SWSTART).unwrap();
            for _ in 0..15 {
                adc.tick();
            }
            assert_eq!(adc.dr, want, "CR1.RES={res}");
        }
    }

    /// The F1 layout has no RES field: CR1 bit 24 is DUALMOD there and must
    /// not narrow the 12-bit result.
    #[test]
    fn f1_cr1_bit24_does_not_narrow() {
        let mut adc = Adc::new_with_layout(AdcRegisterLayout::Stm32F1);
        adc.set_channel_input(0, 1650);
        adc.write_u32(0x04, 1 << 24).unwrap();
        f1_software_start(&mut adc);
        for _ in 0..15 {
            adc.tick();
        }
        assert_eq!(adc.dr, 2047);
    }

    /// F4 layout still starts on bit 30 (board bindings share the legacy block).
    #[test]
    fn f4_software_start_uses_bit30() {
        let mut adc = Adc::new_with_layout(AdcRegisterLayout::Stm32F4);
        adc.set_channel_input(0, 1650);
        adc.write_u32(0x08, F1_CR2_ADON | F4_CR2_SWSTART).unwrap();
        assert!(adc.converting, "F4 SWSTART is bit 30");
        for _ in 0..15 {
            adc.tick();
        }
        assert_eq!(adc.dr, adc.channel_input_count(0) as u32);

        // F1 bit 22 alone must not start on the F4 layout.
        let mut adc = Adc::new_with_layout(AdcRegisterLayout::Stm32F4);
        adc.write_u32(0x08, F1_CR2_ADON | F1_CR2_EXTSEL | F1_CR2_SWSTART)
            .unwrap();
        assert!(
            !adc.converting,
            "F4 layout must not treat bit 22 as SWSTART"
        );
    }

    /// RM0090 §13.8.2/§13.13: with EOCS set, an unread result at the next
    /// conversion raises OVR; OVRIE shares the ADC interrupt line. Reading DR
    /// clears EOC, and ADC_SR is rc_w0 for these flags.
    #[test]
    fn f4_eocs_overrun_sets_flag_irq_and_recovers() {
        let mut adc = Adc::new_with_layout(AdcRegisterLayout::Stm32F4);
        let cr2 = F1_CR2_ADON | F4_CR2_EOCS;
        adc.write_u32(0x04, F4_CR1_OVRIE).unwrap();
        adc.write_u32(0x08, cr2).unwrap();
        adc.write_u32(0x08, cr2 | F4_CR2_SWSTART).unwrap();
        let first: Vec<_> = (0..15).map(|_| adc.tick().irq).collect();
        assert!(adc.read_u32(0x00).unwrap() & F4_SR_EOC != 0);
        assert_eq!(adc.read_u32(0x00).unwrap() & F4_SR_OVR, 0);
        assert!(first.iter().all(|irq| !irq), "OVRIE alone ignores EOC");

        // EOC remains pending across SWSTART until DR is read or ADC_SR is
        // explicitly cleared, so this second completion loses the old result.
        adc.write_u32(0x08, cr2 | F4_CR2_SWSTART).unwrap();
        let second: Vec<_> = (0..15).map(|_| adc.tick().irq).collect();
        assert_eq!(
            adc.read_u32(0x00).unwrap() & (F4_SR_EOC | F4_SR_OVR),
            F4_SR_EOC | F4_SR_OVR
        );
        assert!(second.last().copied().unwrap_or(false), "OVRIE raises IRQ");

        // Writing one preserves EOC and zero clears OVR (rc_w0).
        adc.write_u32(0x00, !F4_SR_OVR).unwrap();
        assert_eq!(adc.read_u32(0x00).unwrap() & F4_SR_OVR, 0);
        assert_ne!(adc.read_u32(0x00).unwrap() & F4_SR_EOC, 0);
        let _ = adc.read_u32(0x4C).unwrap();
        assert_eq!(adc.read_u32(0x00).unwrap() & F4_SR_EOC, 0);

        // Reading DR before the next completion acknowledges the prior result,
        // so the resumed single conversion must not raise another overrun.
        adc.write_u32(0x08, cr2 | F4_CR2_SWSTART).unwrap();
        let third: Vec<_> = (0..15).map(|_| adc.tick().irq).collect();
        assert_eq!(adc.read_u32(0x00).unwrap() & F4_SR_OVR, 0);
        assert_ne!(adc.read_u32(0x00).unwrap() & F4_SR_EOC, 0);
        assert!(!third.last().copied().unwrap_or(true));
    }

    /// RM0090 §13.3.8: SCAN consumes the programmed regular ranks in order;
    /// EOCS raises EOC/IRQ for each conversion, and DR reads acknowledge each
    /// result before the next one arrives.
    #[test]
    fn f4_scan_sequence_converts_selected_ranks_with_per_conversion_eoc() {
        let mut adc = Adc::new_with_layout(AdcRegisterLayout::Stm32F4);
        adc.set_channel_input(1, 825);
        adc.set_channel_input(2, 1650);
        adc.write_u32(0x04, F4_CR1_SCAN | F4_CR1_EOCIE).unwrap();
        adc.write_u32(0x2C, 1 << 20).unwrap(); // L=1 → two conversions
        adc.write_u32(0x34, 1 | (2 << 5)).unwrap(); // SQ1=IN1, SQ2=IN2
        adc.write_u32(0x08, F1_CR2_ADON | F4_CR2_EOCS).unwrap();
        adc.write_u32(0x08, F1_CR2_ADON | F4_CR2_EOCS | F4_CR2_SWSTART)
            .unwrap();

        let first: Vec<_> = (0..15).map(|_| adc.tick().irq).collect();
        assert!(first[..14].iter().all(|irq| !irq));
        assert!(first[14], "EOCIE must report the first converted rank");
        assert_ne!(adc.read_u32(0x00).unwrap() & F4_SR_EOC, 0);
        assert_eq!(adc.read_u32(0x4C).unwrap(), 1023);
        assert_eq!(adc.read_u32(0x00).unwrap() & F4_SR_EOC, 0);
        assert_eq!(adc.seq_index, 1);
        assert!(
            adc.converting,
            "the second sequence rank starts automatically"
        );

        let second: Vec<_> = (0..15).map(|_| adc.tick().irq).collect();
        assert!(second[..14].iter().all(|irq| !irq));
        assert!(second[14], "EOCIE must report the second converted rank");
        assert_ne!(adc.read_u32(0x00).unwrap() & F4_SR_EOC, 0);
        assert_eq!(adc.read_u32(0x4C).unwrap(), 2047);
        assert!(!adc.converting, "single-shot scan stops at the last rank");
        assert_eq!(adc.seq_index, 0);
    }

    /// With SCAN set and EOCS clear, RM0090 reports completion only at the end
    /// of the regular group, not after each channel.
    #[test]
    fn f4_scan_without_eocs_interrupts_only_after_the_group() {
        let mut adc = Adc::new_with_layout(AdcRegisterLayout::Stm32F4);
        adc.set_channel_input(1, 825);
        adc.set_channel_input(2, 1650);
        adc.write_u32(0x04, F4_CR1_SCAN | F4_CR1_EOCIE).unwrap();
        adc.write_u32(0x2C, 1 << 20).unwrap();
        adc.write_u32(0x34, 1 | (2 << 5)).unwrap();
        adc.write_u32(0x08, F1_CR2_ADON).unwrap();
        adc.write_u32(0x08, F1_CR2_ADON | F4_CR2_SWSTART).unwrap();

        let first: Vec<_> = (0..15).map(|_| adc.tick().irq).collect();
        assert!(first.iter().all(|irq| !irq));
        assert_eq!(adc.read_u32(0x00).unwrap() & F4_SR_EOC, 0);
        assert!(adc.converting);

        let second: Vec<_> = (0..15).map(|_| adc.tick().irq).collect();
        assert!(second[..14].iter().all(|irq| !irq));
        assert!(second[14], "EOCIE is raised at regular-group completion");
        assert_ne!(adc.read_u32(0x00).unwrap() & F4_SR_EOC, 0);
        assert_eq!(adc.read_u32(0x4C).unwrap(), 2047);
        assert!(!adc.converting);
    }

    /// RM0090 §13.3.5/§13.3.8: CONT keeps a scanned regular group running and
    /// restarts it at rank 1 after the last selected channel.
    #[test]
    fn f4_continuous_scan_restarts_at_first_rank() {
        let mut adc = Adc::new_with_layout(AdcRegisterLayout::Stm32F4);
        adc.set_channel_input(1, 825);
        adc.set_channel_input(2, 1650);
        adc.write_u32(0x04, F4_CR1_SCAN | F4_CR1_EOCIE).unwrap();
        adc.write_u32(0x2C, 1 << 20).unwrap(); // L=1 → two conversions
        adc.write_u32(0x34, 1 | (2 << 5)).unwrap(); // SQ1=IN1, SQ2=IN2
        let cr2 = F1_CR2_ADON | F1_CR2_CONT | F4_CR2_EOCS;
        adc.write_u32(0x08, cr2).unwrap();
        adc.write_u32(0x08, cr2 | F4_CR2_SWSTART).unwrap();

        for (index, expected) in [1023, 2047, 1023, 2047].into_iter().enumerate() {
            let irq: Vec<_> = (0..15).map(|_| adc.tick().irq).collect();
            assert!(irq[..14].iter().all(|pending| !pending));
            assert!(irq[14], "EOCIE must report conversion {}", index + 1);
            assert_eq!(adc.read_u32(0x4C).unwrap(), expected);
            assert_eq!(adc.read_u32(0x00).unwrap() & F4_SR_EOC, 0);
            assert!(adc.converting, "CONT must keep the scan active");
            assert_eq!(adc.seq_index, (index as u8 + 1) % 2);
        }
    }

    /// With EOCS clear, reading DR clears group-end EOC through the next
    /// non-final rank; it must not reappear before the following group ends.
    #[test]
    fn f4_group_eoc_read_stays_clear_until_next_group_end() {
        let mut adc = Adc::new_with_layout(AdcRegisterLayout::Stm32F4);
        adc.set_channel_input(1, 825);
        adc.set_channel_input(2, 1650);
        adc.write_u32(0x04, F4_CR1_SCAN | F4_CR1_EOCIE).unwrap();
        adc.write_u32(0x2C, 1 << 20).unwrap();
        adc.write_u32(0x34, 1 | (2 << 5)).unwrap();
        let cr2 = F1_CR2_ADON | F1_CR2_CONT;
        adc.write_u32(0x08, cr2).unwrap();
        adc.write_u32(0x08, cr2 | F4_CR2_SWSTART).unwrap();

        let first_rank: Vec<_> = (0..15).map(|_| adc.tick().irq).collect();
        assert!(first_rank.iter().all(|irq| !irq));
        assert_eq!(adc.read_u32(0x00).unwrap() & F4_SR_EOC, 0);

        let first_group_end: Vec<_> = (0..15).map(|_| adc.tick().irq).collect();
        assert!(first_group_end[14]);
        assert_ne!(adc.read_u32(0x00).unwrap() & F4_SR_EOC, 0);
        assert_eq!(adc.read_u32(0x4C).unwrap(), 2047);
        assert_eq!(adc.read_u32(0x00).unwrap() & F4_SR_EOC, 0);

        let next_first_rank: Vec<_> = (0..15).map(|_| adc.tick().irq).collect();
        assert!(next_first_rank.iter().all(|irq| !irq));
        assert_eq!(adc.read_u32(0x00).unwrap() & F4_SR_EOC, 0);

        let next_group_end: Vec<_> = (0..15).map(|_| adc.tick().irq).collect();
        assert!(next_group_end[14]);
        assert_ne!(adc.read_u32(0x00).unwrap() & F4_SR_EOC, 0);
        assert_eq!(adc.read_u32(0x4C).unwrap(), 2047);
    }

    /// RM0090 §§13.3.9, 13.13.12: a two-rank software-started injected scan
    /// uses JSQ3/JSQ4, stores each result in JDR1/JDR2, and raises JEOC/JEOCIE
    /// only when the final injected channel completes.
    #[test]
    fn f4_injected_scan_writes_jdr_and_raises_group_end_jeoc() {
        let mut adc = Adc::new_with_layout(AdcRegisterLayout::Stm32F4);
        adc.set_channel_input(1, 825);
        adc.set_channel_input(9, 1650);
        adc.write_u32(0x04, F4_CR1_SCAN | F4_CR1_JEOCIE).unwrap();
        // JL=1 selects two ranks; for two ranks, JSQ3 then JSQ4 are active.
        adc.write_u32(0x38, (1 << 20) | (1 << 10) | (9 << 15))
            .unwrap();
        adc.write_u32(0x08, F1_CR2_ADON).unwrap();
        adc.write_u32(0x08, F1_CR2_ADON | F4_CR2_JSWSTART).unwrap();
        assert_eq!(adc.read_u32(0x08).unwrap() & F4_CR2_JSWSTART, 0);

        let first_rank: Vec<_> = (0..15).map(|_| adc.tick().irq).collect();
        assert!(first_rank.iter().all(|pending| !pending));
        assert_eq!(adc.read_u32(0x3C).unwrap(), 1023);
        assert_eq!(adc.read_u32(0x00).unwrap() & F4_SR_JEOC, 0);
        assert_eq!(adc.read_u32(0x00).unwrap() & F4_SR_EOC, 0);
        assert_eq!(
            adc.read_u32(0x4C).unwrap(),
            0,
            "injected data must not overwrite DR"
        );
        assert!(
            adc.converting,
            "the second injected rank starts automatically"
        );

        let second_rank: Vec<_> = (0..15).map(|_| adc.tick().irq).collect();
        assert!(second_rank[..14].iter().all(|pending| !pending));
        assert!(
            second_rank[14],
            "JEOCIE is raised at injected group completion"
        );
        assert_eq!(adc.read_u32(0x40).unwrap(), 2047);
        assert_ne!(adc.read_u32(0x00).unwrap() & F4_SR_JEOC, 0);
        assert_eq!(adc.read_u32(0x00).unwrap() & F4_SR_EOC, 0);
        assert!(!adc.converting);

        adc.write_u32(0x00, !F4_SR_JEOC).unwrap();
        assert_eq!(adc.read_u32(0x00).unwrap() & F4_SR_JEOC, 0);
    }
}

// ── Walk-free differential: F1 ADC conversion engine walk vs scheduler ────────
#[cfg(all(test, feature = "event-scheduler"))]
mod scheduler_diff {
    use super::*;

    #[derive(Clone, Copy)]
    enum Op {
        Write(u64, u8),
    }

    fn build(scheduler: bool) -> Adc {
        let mut adc = Adc::new(); // F1
        adc.set_channel_input(3, 1650); // deterministic conversion result on ch3
        if scheduler {
            adc.attach_cycle_clock(CycleClock::default());
        }
        adc
    }

    /// Drive the SAME op script against (a) the per-cycle walk and (b) the event
    /// path; assert the register snapshot AND the EOC-IRQ pend cycles are
    /// identical every cycle. The conversion countdown, DR latch, EOC flag and
    /// continuous-mode restart must all line up cycle-for-cycle.
    fn assert_walk_identical(script: &[(u64, Op)], cycles: u64) -> usize {
        let mut walk = build(false);
        let mut delivered = 0usize;
        let mut sched = build(true);
        let clock = sched.clock.clone().unwrap();

        let mut events: Vec<(u64, u32)> = Vec::new();
        let bus = &mut crate::bus::SystemBus::new();
        let mut walk_pends = Vec::new();
        let mut sched_pends = Vec::new();

        for c in 1..=cycles {
            for (sc, Op::Write(off, val)) in script.iter().copied() {
                if sc == c {
                    walk.write(off, val).unwrap();
                    sched.write(off, val).unwrap();
                    for (delay, token) in sched.take_scheduled_events() {
                        events.push((c - 1 + 1 + delay, token));
                    }
                }
            }

            if walk.tick().irq {
                walk_pends.push(c);
            }

            clock.publish(c);
            let due: Vec<(u64, u32)> = events.iter().copied().filter(|(d, _)| *d <= c).collect();
            events.retain(|(d, _)| *d > c);
            let mut esched = crate::sched::EventScheduler::new();
            esched.advance_to(c);
            for (_, token) in due {
                delivered += 1;
                let res = sched.on_event(token, &mut esched, bus);
                if res.raise_own_irq {
                    sched_pends.push(c);
                }
                if let Some(delay) = res.reschedule_delay {
                    events.push((c + delay, token));
                }
            }

            assert_eq!(
                walk.snapshot(),
                sched.snapshot(),
                "register snapshot diverged at cycle {c}"
            );
        }
        assert_eq!(walk_pends, sched_pends, "EOC-IRQ pend cycles diverged");
        delivered
    }

    #[test]
    fn f1_single_conversion_walk_identity() {
        // EOCIE + ADON + SWSTART on channel 3 (SQR3 SQ1) → 14-cycle countdown →
        // EOC + DR. The channel is programmed through SQR3 @ 0x34, the register
        // the model consults since the CR2 low-bits fallback was removed.
        // F1 SWSTART is bit 22 (byte 0x0A bit 6); EXTSEL=111 required (byte 0x0A bits 3:1).
        let script = [
            (1u64, Op::Write(0x04, 1 << 5)), // CR1.EOCIE
            (1, Op::Write(0x34, 3)),         // SQR3.SQ1 = channel 3
            (1, Op::Write(0x08, 1)),         // CR2.ADON
            (1, Op::Write(0x0A, 0x0E)),      // CR2.EXTSEL = 111 (software)
            (2, Op::Write(0x0A, 0x4E)),      // EXTSEL | SWSTART (bit 22) → convert
            (20, Op::Write(0x00, 0)),        // read-back settle (no-op SR write)
        ];
        assert_walk_identical(&script, 26);
    }

    #[test]
    fn f1_continuous_conversion_walk_identity() {
        // CONT + ADON: after the first EOC the engine re-arms and converts again,
        // so the chain must perpetuate across multiple completions.
        let script = [
            (1u64, Op::Write(0x04, 1 << 5)),    // CR1.EOCIE
            (1, Op::Write(0x34, 3)),            // SQR3.SQ1 = channel 3
            (1, Op::Write(0x08, 1 | (1 << 1))), // CR2.ADON | CONT
            (1, Op::Write(0x0A, 0x0E)),         // EXTSEL = 111
            (2, Op::Write(0x0A, 0x4E)),         // EXTSEL | SWSTART (bit 22)
        ];
        // 2 + 15 + 15 + 15 ≈ 47 cycles covers three back-to-back conversions.
        assert_walk_identical(&script, 50);
    }

    /// Continuous mode used to tick the countdown every cycle; it now wakes
    /// once per completion. Snapshot (including the live countdown) and EOC
    /// pend cycles stay identical to the per-cycle walk for many conversions.
    #[test]
    fn f1_continuous_conversion_wakes_once_per_completion() {
        let script = [
            (1u64, Op::Write(0x04, 1 << 5)),
            (1, Op::Write(0x34, 3)),
            (1, Op::Write(0x08, 1 | (1 << 1))),
            (1, Op::Write(0x0A, 0x0E)),
            (2, Op::Write(0x0A, 0x4E)),
        ];
        let cycles = 2 + 15 * 40;
        let delivered = assert_walk_identical(&script, cycles);
        // One arming wake plus one wake per completion, not one per cycle.
        assert!(delivered <= 2 + 40, "delivered {delivered} events");
    }

    /// A restart (SWSTART again) and a cancel (ADON cleared) while the wake is
    /// asleep supersede it: the stale wake must not complete the old countdown.
    #[test]
    fn f1_restart_and_cancel_supersede_a_sleeping_wake() {
        let script = [
            (1u64, Op::Write(0x04, 1 << 5)),
            (1, Op::Write(0x34, 3)),
            (1, Op::Write(0x08, 1 | (1 << 1))), // ADON | CONT
            (1, Op::Write(0x0A, 0x0E)),
            (2, Op::Write(0x0A, 0x4E)),          // SWSTART
            (9, Op::Write(0x0A, 0x0E)),          // SWSTART low
            (10, Op::Write(0x0A, 0x4E)),         // SWSTART again mid-conversion: restart
            (31, Op::Write(0x08, 0)),            // ADON cleared mid-conversion: cancel
            (40, Op::Write(0x08, 1 | (1 << 1))), // ADON again
            (41, Op::Write(0x0A, 0x0E)),
            (42, Op::Write(0x0A, 0x4E)),
        ];
        assert_walk_identical(&script, 90);
    }

    // ── STM32H7 ADC (RM0468) ────────────────────────────────────────────────
    //
    // Every expectation below is the SVD's, not mine: offsets, reset values and
    // the RES encoding all come from tests/fixtures/real_world/stm32h735.svd.

    #[test]
    fn h7_reset_values_match_the_svd() {
        let adc = Adc::new_with_layout(AdcRegisterLayout::Stm32H7);
        assert_eq!(adc.read_u32(0x08).unwrap(), 0x2000_0000, "CR: DEEPPWD set");
        assert_eq!(adc.read_u32(0x0C).unwrap(), 0x8000_0000, "CFGR: JQDIS set");
        // The three analog-watchdog HIGH thresholds reset to 26-bit all-ones.
        // An L4 answering here would return 0 — this is the cheapest single
        // discriminator between the two layouts.
        assert_eq!(adc.read_u32(0x24).unwrap(), 0x03FF_FFFF, "HTR1");
        assert_eq!(adc.read_u32(0xB4).unwrap(), 0x03FF_FFFF, "HTR2");
        assert_eq!(adc.read_u32(0xBC).unwrap(), 0x03FF_FFFF, "HTR3");
        // ...and their LOW counterparts reset to 0.
        assert_eq!(adc.read_u32(0x20).unwrap(), 0, "LTR1");
    }

    #[test]
    fn h7_registers_the_l4_does_not_have_are_addressable() {
        let mut adc = Adc::new_with_layout(AdcRegisterLayout::Stm32H7);
        // PCSEL @ 0x1C and CALFACT2 @ 0xC8 exist only on the H7. On the L4
        // layout both are unmapped and read back 0 whatever you write.
        adc.write_u32(0x1C, 0x000F_FFFF).unwrap();
        assert_eq!(adc.read_u32(0x1C).unwrap(), 0x000F_FFFF, "PCSEL");
        adc.write_u32(0xC8, 0xDEAD_BEEF).unwrap();
        assert_eq!(adc.read_u32(0xC8).unwrap(), 0xDEAD_BEEF, "CALFACT2");

        let mut l4 = Adc::new_with_layout(AdcRegisterLayout::Stm32L4);
        l4.write_u32(0x1C, 0x000F_FFFF).unwrap();
        assert_eq!(l4.read_u32(0x1C).unwrap(), 0, "L4 has no PCSEL");
    }

    #[test]
    fn h7_deep_power_down_gates_the_whole_bring_up() {
        // RM0468 §26.4.6: out of reset the converter is in deep power-down.
        // Writing ADEN without clearing DEEPPWD must NOT make it ready — this
        // is exactly the stall a HAL that skips the wake-up hits on silicon.
        let mut cold = Adc::new_with_layout(AdcRegisterLayout::Stm32H7);
        cold.write_u32(0x08, (1 << 29) | (1 << 28) | 1).unwrap();
        assert_eq!(cold.read_u32(0x00).unwrap() & 0x1, 0, "no ADRDY in DEEPPWD");
        assert_eq!(cold.read_u32(0x08).unwrap() & 0x1, 0, "ADEN cannot latch");

        // The documented order: clear DEEPPWD, raise the LDO, then enable.
        let mut adc = Adc::new_with_layout(AdcRegisterLayout::Stm32H7);
        adc.write_u32(0x08, 0).unwrap();
        adc.write_u32(0x08, 1 << 28).unwrap();
        assert_eq!(
            adc.read_u32(0x00).unwrap() & (1 << 12),
            1 << 12,
            "LDORDY after ADVREGEN"
        );
        assert_eq!(adc.read_u32(0x00).unwrap() & 0x1, 0, "no ADRDY before ADEN");
        adc.write_u32(0x08, (1 << 28) | 1).unwrap();
        assert_eq!(adc.read_u32(0x00).unwrap() & 0x1, 0x1, "ADRDY after ADEN");
    }

    #[test]
    fn h7_adcal_self_clears() {
        let mut adc = Adc::new_with_layout(AdcRegisterLayout::Stm32H7);
        adc.write_u32(0x08, 0).unwrap();
        adc.write_u32(0x08, 1 << 28).unwrap();
        adc.write_u32(0x08, (1 << 28) | (1 << 31)).unwrap(); // ADCAL
        assert_eq!(
            adc.read_u32(0x08).unwrap() & (1 << 31),
            0,
            "ADCAL must self-clear; firmware polls it to 0 and would spin forever"
        );
        assert_ne!(adc.read_u32(0xC4).unwrap(), 0, "CALFACT populated");
    }

    #[test]
    fn h7_converts_at_the_configured_resolution() {
        // Default RES=000 is SIXTEEN bits on this part. The same CFGR write on
        // an L4 selects a different width entirely, which is what made the old
        // alias wrong in a way firmware could observe.
        let mut adc = Adc::new_with_layout(AdcRegisterLayout::Stm32H7);
        adc.write_u32(0x08, 0).unwrap();
        adc.write_u32(0x08, 1 << 28).unwrap();
        adc.write_u32(0x08, (1 << 28) | 1).unwrap(); // ADEN -> ADRDY
        adc.write_u32(0x08, (1 << 28) | 1 | (1 << 2)).unwrap(); // ADSTART

        let dr16 = adc.read_u32(0x40).unwrap();
        assert_eq!(dr16, scaled_adc_code(16));
        assert!(dr16 > 0xFFF, "16-bit code must exceed a 12-bit full scale");
        let isr = adc.read_u32(0x00).unwrap();
        assert_eq!(isr & (1 << 2), 1 << 2, "EOC");
        assert_eq!(isr & (1 << 3), 1 << 3, "EOS");
        assert_eq!(adc.read_u32(0x08).unwrap() & (1 << 2), 0, "ADSTART clears");

        // RES=110 selects 12 bits (NOT the L4's encoding, where 0b10 is 8).
        adc.write_u32(0x0C, 0x8000_0000 | (0b110 << 2)).unwrap();
        adc.write_u32(0x08, (1 << 28) | 1 | (1 << 2)).unwrap();
        assert_eq!(adc.read_u32(0x40).unwrap(), scaled_adc_code(12));

        // RES=111 is 8 bits.
        adc.write_u32(0x0C, 0x8000_0000 | (0b111 << 2)).unwrap();
        adc.write_u32(0x08, (1 << 28) | 1 | (1 << 2)).unwrap();
        assert_eq!(adc.read_u32(0x40).unwrap(), scaled_adc_code(8));
    }

    #[test]
    fn h7_res_encoding_follows_the_svd_not_the_l4() {
        // The two encodings disagree on every value they share, so a firmware
        // write of RES=0b010 means 12 bits here and 8 bits on an L4.
        assert_eq!(h7_resolution_bits(0b000 << 2), 16);
        assert_eq!(h7_resolution_bits(0b001 << 2), 14);
        assert_eq!(h7_resolution_bits(0b010 << 2), 12);
        assert_eq!(h7_resolution_bits(0b011 << 2), 10);
        assert_eq!(h7_resolution_bits(0b101 << 2), 14);
        assert_eq!(h7_resolution_bits(0b110 << 2), 12);
        assert_eq!(h7_resolution_bits(0b111 << 2), 8);
        // 0b100 is undefined in the SVD; fall back to the reset width rather
        // than inventing a behaviour.
        assert_eq!(h7_resolution_bits(0b100 << 2), 16);
    }

    #[test]
    fn h7_layout_is_selected_by_name() {
        assert_eq!(
            "h7".parse::<AdcRegisterLayout>().unwrap(),
            AdcRegisterLayout::Stm32H7
        );
        assert_eq!(
            "stm32h7".parse::<AdcRegisterLayout>().unwrap(),
            AdcRegisterLayout::Stm32H7
        );
        // Regression guard: these used to resolve to Stm32L4.
        assert_ne!(
            "h7".parse::<AdcRegisterLayout>().unwrap(),
            AdcRegisterLayout::Stm32L4
        );
        // The families that genuinely DO share the L4 block still do. G0 has
        // its own CHSELR block (`g0_layout_is_selected_by_name_and_is_not_the_l4`).
        for name in ["l4", "stm32l4", "f7"] {
            assert_eq!(
                name.parse::<AdcRegisterLayout>().unwrap(),
                AdcRegisterLayout::Stm32L4,
                "{name}"
            );
        }
    }

    #[test]
    fn h7_stimulus_channel_scales_to_the_configured_width() {
        // set_channel_input stores a 12-bit count; a 16-bit conversion must
        // widen it rather than return a 12-bit value in a 16-bit register.
        let mut adc = Adc::new_with_layout(AdcRegisterLayout::Stm32H7);
        adc.set_channel_input(5, 1650); // ~half scale
        let count12 = adc.channel_input_count(5) as u32;
        adc.write_u32(0x08, 0).unwrap();
        adc.write_u32(0x08, 1 << 28).unwrap();
        adc.write_u32(0x08, (1 << 28) | 1).unwrap();
        adc.write_u32(0x30, 5 << 6).unwrap(); // SQR1.SQ1 = channel 5
        adc.write_u32(0x08, (1 << 28) | 1 | (1 << 2)).unwrap();
        assert_eq!(adc.read_u32(0x40).unwrap(), count12 << 4);
    }
}
