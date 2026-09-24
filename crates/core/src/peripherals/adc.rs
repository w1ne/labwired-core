// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.
//
// ── Architectural separation ────────────────────────────────────────────────
// The family-specific CONTROL registers live in the `AdcRegs` enum: an F1 ADC
// carries only CR1/CR2, an L4 ADC carries only ISR/IER/CR/CFGR/…/CCR — neither
// holds the other's. The data register `dr`, the legacy status `sr` (both poked
// directly by the WASM value-injection bridge), the conversion engine and the
// per-channel injected inputs are architecture-independent and stay shared.

use crate::{CycleClock, Peripheral, PeripheralTickResult, SimResult};
use std::any::Any;
use std::str::FromStr;

// ── Modeled internal source (L4/V2 path) ────────────────────────────────────
// There is no live analog input in simulation, so the L4 conversion engine
// converts a *fixed* source: V(IN) = 3.0 V against V(REF+) = 3.3 V. The 12-bit
// code is round-down(3.0/3.3 * 4096) = 3723; narrower resolutions drop LSBs
// (>> (12 - bits)), exactly as the SAR core truncates. The code is therefore
// derived + deterministic and CHANGES with CFGR.RES — a real conversion, not a
// constant.
const STM32_ADC_REF12: u32 = (3000 * 4096) / 3300; // 3723

/// CFGR.RES (bits [4:3]) → conversion bit-width. 0b00=12, 01=10, 10=8, 11=6.
fn l4_resolution_bits(cfgr: u32) -> u32 {
    match (cfgr >> 3) & 0x3 {
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

/// STM32F2/F4 ADC_CR2.SWSTART (RM0090) — same legacy register *block* as F1,
/// different bit. Kept so F4 board bindings are not silently retargeted to F1.
const F4_CR2_SWSTART: u32 = 1 << 30;

/// Approximate F1 RSTCAL completion latency in model cycles (not host time).
/// Silicon clears RSTCAL after a short internal reset; a fixed 2-cycle delay is
/// enough for HAL spin-loops (`while (CR2 & RSTCAL)`) to observe completion.
/// Explicitly an approximation — not cycle-accurate ADC-clock timing.
const F1_RSTCAL_CYCLES: u32 = 2;
/// Approximate F1 CAL completion latency in model cycles. ST calibration takes
/// many ADC clocks; a short fixed delay unblocks HAL polls without claiming
/// silicon-accurate duration.
const F1_CAL_CYCLES: u32 = 14;

/// Scheduler / walk event tokens for the F1 side-band (conversion uses 0).
const F1_EVT_CONVERT: u32 = 0;
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
            "stm32l4" | "l4" | "stm32f7" | "f7" | "stm32g0" | "g0" => Ok(Self::Stm32L4),
            "stm32h7" | "h7" => Ok(Self::Stm32H7),
            "stm32h5" | "h5" => Ok(Self::Stm32H5),
            "stm32u5" | "u5" => Ok(Self::Stm32U5),
            _ => Err(format!(
                "unsupported ADC register layout '{}'; supported: stm32f1, stm32f4, stm32l4, stm32h5, stm32h7, stm32u5",
                value
            )),
        }
    }
}

/// STM32F1 ADC control registers (status `sr` + data `dr` are shared on `Adc`).
#[derive(Debug, Default, serde::Serialize)]
pub struct F1AdcRegs {
    cr1: u32, // 0x04
    cr2: u32, // 0x08
    /// ADC_SQR3 @ 0x34 (RM0008 §11.12.14). Only SQ1[4:0] — the first (and, on
    /// this single-shot model, only) regular-sequence channel — is consumed,
    /// by `advance_conversion`. Reset value 0 selects channel 0, which is what
    /// firmware that never programs the sequence converts on silicon.
    sqr3: u32,
}

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

/// Family-isolated ADC control registers.
#[derive(Debug, serde::Serialize)]
enum AdcRegs {
    Stm32F1(F1AdcRegs),
    /// F2/F4: same `F1AdcRegs` storage, F4 SWSTART/EXTEN semantics in the write path.
    Stm32F4(F1AdcRegs),
    Stm32L4(L4AdcRegs),
    Stm32H7(H7AdcRegs),
    Stm32U5(U5AdcRegs),
}

#[derive(Debug, serde::Serialize)]
pub struct Adc {
    regs: AdcRegs,
    /// Legacy status register (F1 SR). Shared because the WASM bridge pokes it
    /// directly to inject an EOC; also the conversion engine's EOC flag.
    pub sr: u32,
    /// Conversion data register — shared result path for both families.
    pub dr: u32,

    // Shared conversion engine.
    converting: bool,
    cycles_remaining: u32,
    conversion_time: u32,
    /// Per-channel injected values (12-bit counts). 0xFFFF = "no injection".
    /// Sized for the widest family (H7, 20 channels); [`Self::channel_count`]
    /// is how many of them this layout has.
    channel_inputs: [u16; MAX_CHANNELS],

    /// Bus-published cycle clock (walk-free campaign). `Some` once attached →
    /// event-schedulable; `None` keeps the legacy walk.
    #[serde(skip)]
    clock: Option<CycleClock>,
    /// Scheduler mode: `true` while the conversion-countdown event is live.
    #[serde(skip)]
    chain_live: bool,
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
    /// clock, so its calibration finishes.
    ///
    /// This is a stand-in for the kernel clock the RCC model does not yet
    /// expose to the ADC. When it does, this flag should die and both layouts
    /// should ask the clock instead.
    calibration_completes: bool,
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
            converting: false,
            cycles_remaining: 0,
            conversion_time: 14,
            channel_inputs: [0xFFFF; MAX_CHANNELS],
            clock: None,
            chain_live: false,
            f1_rstcal_remaining: None,
            f1_cal_remaining: None,
            f1_rstcal_live: false,
            f1_cal_live: false,
        }
    }

    crate::cycle_clock::scheduler_mode!();

    /// Test/differential knob: detach the clock, pinning the model to the legacy
    /// walk (the walk-on reference for the differential gate).
    pub fn force_legacy_walk(&mut self) {
        self.clock = None;
    }

    /// One cycle of the F1 conversion countdown. Returns whether an EOC IRQ
    /// should be raised this cycle. Shared verbatim by the legacy walk `tick()`
    /// and the scheduler `on_event`, so the two routes are identical by
    /// construction.
    fn advance_conversion(&mut self) -> bool {
        let mut irq = false;
        if self.converting {
            if self.cycles_remaining > 0 {
                self.cycles_remaining -= 1;
            } else {
                self.converting = false;
                let (cr1, cr2) = self.f1_ctrl();

                // Injected channel value if available; else increment DR for
                // visual feedback. The converted channel is SQR3 SQ1[4:0]
                // (RM0008 §11.12.14) — NOT a CR2 bit field: the earlier CR2
                // low-bits fallback always read ≥ 1 for any firmware that
                // holds ADON set, so a seeded channel 0 was never returned.
                // SQR3 resets to 0, so firmware that never programs the
                // sequence converts channel 0, exactly like silicon.
                let ch = self.f1_regular_channel();
                if ch < self.channel_inputs.len() && self.channel_inputs[ch] != 0xFFFF {
                    self.dr = self.channel_inputs[ch] as u32;
                } else {
                    self.dr = (self.dr + 1) & 0xFFF;
                }

                self.sr |= 1 << 1; // EOC

                if (cr1 & (1 << 5)) != 0 {
                    irq = true; // EOCIE
                }
                // Continuous mode (CONT + ADON).
                if (cr2 & F1_CR2_CONT) != 0 && (cr2 & F1_CR2_ADON) != 0 {
                    self.start_conversion();
                }
            }
        }
        irq
    }

    /// Inject a millivolt reading for a specific ADC channel. The next
    /// conversion on this channel returns the equivalent 12-bit count.
    /// Read back the injected 12-bit count for a channel (`0xFFFF` = nothing
    /// injected). The read-back counterpart of [`Self::set_channel_input`], so
    /// a stimulus test can assert what the next conversion will return without
    /// running one.
    pub fn channel_input_count(&self, channel: u8) -> u16 {
        self.channel_inputs
            .get(channel as usize)
            .copied()
            .unwrap_or(0xFFFF)
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
            AdcRegs::Stm32F1(_) | AdcRegs::Stm32F4(_) | AdcRegs::Stm32L4(_) => 19,
            AdcRegs::Stm32H7(_) | AdcRegs::Stm32U5(_) => 20,
        }
    }

    pub fn set_channel_input(&mut self, channel: u8, millivolts: u16) {
        if channel < self.channel_count() {
            let count = ((millivolts as u32 * 4095) / 3300).min(4095) as u16;
            self.channel_inputs[channel as usize] = count;
        }
    }

    /// Remove the injected level from a channel, returning it to the modeled
    /// internal source. `set_channel_input` cannot express this itself: its
    /// mV→count conversion saturates at 4095, so the 0xFFFF "no injection"
    /// sentinel is unreachable through it by construction.
    pub fn clear_channel_input(&mut self, channel: u8) {
        if (channel as usize) < self.channel_inputs.len() {
            self.channel_inputs[channel as usize] = 0xFFFF;
        }
    }

    fn start_conversion(&mut self) {
        self.converting = true;
        self.cycles_remaining = self.conversion_time;
        self.sr &= !0x2; // clear EOC on start
    }

    /// (cr1, cr2) for the F1 control registers; (0, 0) on L4 (no conversion
    /// engine runs there, so these are only consulted on the F1 path).
    fn f1_ctrl(&self) -> (u32, u32) {
        match &self.regs {
            AdcRegs::Stm32F1(r) | AdcRegs::Stm32F4(r) => (r.cr1, r.cr2),
            // Neither the L4, the H7 nor the U5 runs the F1 countdown engine.
            AdcRegs::Stm32L4(_) | AdcRegs::Stm32H7(_) | AdcRegs::Stm32U5(_) => (0, 0),
        }
    }

    /// Regular channel being converted: SQR3 SQ1[4:0] on the F1 layout; 0 on
    /// the other families (their engines select from their own SQR1/PCSEL).
    fn f1_regular_channel(&self) -> usize {
        match &self.regs {
            AdcRegs::Stm32F1(r) | AdcRegs::Stm32F4(r) => (r.sqr3 & 0x1F) as usize,
            AdcRegs::Stm32L4(_) | AdcRegs::Stm32H7(_) | AdcRegs::Stm32U5(_) => 0,
        }
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
    /// ready converter (ISR.ADRDY) converts the fixed internal source: loads DR
    /// with the code for CFGR.RES, raises ISR.EOC + EOS, and auto-clears
    /// ADSTART. Deterministic and immediate (no analog settling to model).
    fn maybe_start_l4_conversion(&mut self, cr: u32) {
        let aden = cr & 0x1 != 0;
        let adstart = cr & (1 << 2) != 0;
        let (adrdy, cfgr) = match &self.regs {
            AdcRegs::Stm32L4(r) => (r.isr & 0x1 != 0, r.cfgr),
            // H7 and U5 have their own engines (`maybe_start_h7_conversion`,
            // `maybe_start_u5_conversion`).
            AdcRegs::Stm32F1(_)
            | AdcRegs::Stm32F4(_)
            | AdcRegs::Stm32H7(_)
            | AdcRegs::Stm32U5(_) => return,
        };
        if !(aden && adstart && adrdy) {
            return;
        }
        self.dr = l4_adc_code(l4_resolution_bits(cfgr));
        if let AdcRegs::Stm32L4(r) = &mut self.regs {
            r.cr &= !(1 << 2); // ADSTART auto-clears after a single conversion
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
        let injected = self.channel_inputs.get(ch).copied().unwrap_or(0xFFFF);
        // Injected stimuli are held as 12-bit counts (set_channel_input), so
        // rescale to the configured width rather than truncating to 12.
        let code = if injected != 0xFFFF {
            let v = injected as u32;
            if bits >= 12 {
                (v << (bits - 12)) & ((1 << bits) - 1)
            } else {
                v >> (12 - bits)
            }
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
        let injected = self.channel_inputs.get(ch).copied().unwrap_or(0xFFFF);
        let code = if injected != 0xFFFF {
            let v = injected as u32;
            if bits >= 12 {
                (v << (bits - 12)) & ((1 << bits) - 1)
            } else {
                v >> (12 - bits)
            }
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
                let calibrating = value & (1 << 31) != 0;
                r.cr = if calibration_completes {
                    value & !(1 << 31)
                } else {
                    value
                };
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
        self.chain_live = false;
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

    /// F2/F4 CR2 write: SWSTART at bit 30, rising edge + ADON starts conversion.
    fn apply_f4_cr2(&mut self, old_cr2: u32, new_cr2: u32) -> bool {
        let mut cr2 = new_cr2;
        let adon = (cr2 & F1_CR2_ADON) != 0;
        if !adon {
            self.cancel_f1_pending();
            cr2 &= !F4_CR2_SWSTART;
            if let AdcRegs::Stm32F4(r) = &mut self.regs {
                r.cr2 = cr2;
            }
            return false;
        }
        let swstart = (cr2 & F4_CR2_SWSTART) != 0;
        let old_swstart = (old_cr2 & F4_CR2_SWSTART) != 0;
        let mut trigger = false;
        if swstart && !old_swstart {
            cr2 &= !F4_CR2_SWSTART;
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

    fn read(&self, offset: u64) -> SimResult<u8> {
        let val = match &self.regs {
            AdcRegs::Stm32F1(r) | AdcRegs::Stm32F4(r) => match offset {
                0x00..=0x03 => self.sr,
                0x04..=0x07 => r.cr1,
                0x08..=0x0B => r.cr2,
                0x34..=0x37 => r.sqr3,
                0x4C..=0x4F => self.dr,
                _ => {
                    crate::census_reg!("adc:Adc", offset, "read");
                    0
                }
            },
            AdcRegs::Stm32L4(r) => Self::read_reg_l4(r, self.dr, offset & !3),
            AdcRegs::Stm32H7(r) => Self::read_reg_h7(r, self.dr, offset & !3),
            AdcRegs::Stm32U5(r) => Self::read_reg_u5(r, self.dr, offset & !3),
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
                0x00..=0x03 => self.sr = (self.sr & !mask) | val_shifted,
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
                        self.start_conversion();
                    }
                }
                0x34..=0x37 => match &mut self.regs {
                    AdcRegs::Stm32F1(r) | AdcRegs::Stm32F4(r) => {
                        r.sqr3 = (r.sqr3 & !mask) | val_shifted;
                    }
                    _ => {}
                },
                _ => {
                    crate::census_reg!("adc:Adc", offset, "write");
                }
            },
            AdcRegs::Stm32L4(_) => {
                let reg = offset & !3;
                let dr = self.dr;
                let calibration_completes = self.calibration_completes;
                let mut full = 0;
                if let AdcRegs::Stm32L4(r) = &mut self.regs {
                    full = (Self::read_reg_l4(r, dr, reg) & !mask) | val_shifted;
                    Self::write_reg_l4(r, reg, full, calibration_completes);
                }
                // A write touching CR may have set ADSTART — try to convert.
                if reg == 0x08 {
                    self.maybe_start_l4_conversion(full);
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
        PeripheralTickResult {
            irq,
            cycles: 0,
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
            out.push((0u64, F1_EVT_CONVERT));
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
        _sched: &mut crate::sched::EventScheduler,
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
            _ => {
                // Conversion countdown (token 0) — same engine as the walk.
                let irq = self.advance_conversion();
                self.chain_live = self.converting;
                crate::sched::EventResult {
                    raise_own_irq: irq,
                    reschedule_delay: self.converting.then_some(1),
                    ..Default::default()
                }
            }
        }
    }

    fn as_any(&self) -> Option<&dyn Any> {
        Some(self)
    }
    fn as_any_mut(&mut self) -> Option<&mut dyn Any> {
        Some(self)
    }

    fn snapshot(&self) -> serde_json::Value {
        serde_json::to_value(self).unwrap_or(serde_json::Value::Null)
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

    /// `clear_channel_input` is the only road back to "no injection": the
    /// mV→count conversion saturates at 4095, so 0xFFFF is unreachable through
    /// `set_channel_input`. After the clear, the same conversion must return
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
    fn assert_walk_identical(script: &[(u64, Op)], cycles: u64) {
        let mut walk = build(false);
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
        // The families that genuinely DO share the L4 block still do.
        for name in ["l4", "stm32l4", "f7", "g0"] {
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
