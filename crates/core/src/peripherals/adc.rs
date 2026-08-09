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

/// Converted code for the fixed internal source at an H7 width. The reference
/// constant is a 12-bit count, so widen rather than narrow: the H7's native
/// 16 bits is the *widest* case here, unlike every other family.
fn h7_adc_code(bits: u32) -> u32 {
    if bits >= 12 {
        (STM32_ADC_REF12 << (bits - 12)) & ((1 << bits) - 1)
    } else {
        STM32_ADC_REF12 >> (12 - bits)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AdcRegisterLayout {
    #[default]
    Stm32F1,
    Stm32L4,
    /// STM32H7 (RM0468). A genuinely different block from the L4 — see
    /// [`H7AdcRegs`] for what diverges and why the alias that used to point
    /// `"h7"` at [`Self::Stm32L4`] was wrong.
    Stm32H7,
}

impl FromStr for AdcRegisterLayout {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let v = value.trim().to_ascii_lowercase();
        match v.as_str() {
            "stm32f1" | "f1" | "legacy" => Ok(Self::Stm32F1),
            // `stm32h7`/`h7` used to land here. It no longer does: the H7 block
            // has PCSEL, LTR/HTR watchdog pairs instead of TR1..TR3, CALFACT2,
            // a 3-bit RES at a different offset with a different encoding, and
            // a 16-bit DR. Answering an H7 on the L4 map returned plausible
            // values from the wrong registers.
            "stm32l4" | "l4" | "stm32f7" | "f7" | "stm32g0" | "g0" => Ok(Self::Stm32L4),
            "stm32h7" | "h7" => Ok(Self::Stm32H7),
            _ => Err(format!(
                "unsupported ADC register layout '{}'; supported: stm32f1, stm32l4, stm32h7",
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

/// Family-isolated ADC control registers.
#[derive(Debug, serde::Serialize)]
enum AdcRegs {
    Stm32F1(F1AdcRegs),
    Stm32L4(L4AdcRegs),
    Stm32H7(H7AdcRegs),
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
    channel_inputs: [u16; 18],

    /// Bus-published cycle clock (walk-free campaign). `Some` once attached →
    /// event-schedulable; `None` keeps the legacy walk.
    #[serde(skip)]
    clock: Option<CycleClock>,
    /// Scheduler mode: `true` while the conversion-countdown event is live.
    #[serde(skip)]
    chain_live: bool,
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
            AdcRegisterLayout::Stm32L4 => AdcRegs::Stm32L4(L4AdcRegs {
                cr: 0x2000_0000,
                cfgr: 0x8000_0000,
                ..Default::default()
            }),
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
        };
        Self {
            regs,
            sr: 0,
            dr: 0,
            converting: false,
            cycles_remaining: 0,
            conversion_time: 14,
            channel_inputs: [0xFFFF; 18],
            clock: None,
            chain_live: false,
        }
    }

    #[inline]
    fn scheduler_mode(&self) -> bool {
        cfg!(feature = "event-scheduler") && self.clock.is_some()
    }

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
                // visual feedback. SQR3 ch fallback uses CR2 low bits (legacy).
                let ch = (cr2 & 0x1F) as usize;
                if ch < self.channel_inputs.len() && self.channel_inputs[ch] != 0xFFFF {
                    self.dr = self.channel_inputs[ch] as u32;
                } else {
                    self.dr = (self.dr + 1) & 0xFFF;
                }

                self.sr |= 1 << 1; // EOC

                if (cr1 & (1 << 5)) != 0 {
                    irq = true; // EOCIE
                }
                // Continuous mode (CONT bit1 + ADON bit0).
                if (cr2 & (1 << 1)) != 0 && (cr2 & 1) != 0 {
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

    pub fn set_channel_input(&mut self, channel: u8, millivolts: u16) {
        if (channel as usize) < self.channel_inputs.len() {
            let count = ((millivolts as u32 * 4095) / 3300).min(4095) as u16;
            self.channel_inputs[channel as usize] = count;
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
            AdcRegs::Stm32F1(r) => (r.cr1, r.cr2),
            // Neither the L4 nor the H7 runs the F1 countdown engine.
            AdcRegs::Stm32L4(_) | AdcRegs::Stm32H7(_) => (0, 0),
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
            // H7 has its own engine (`maybe_start_h7_conversion`).
            AdcRegs::Stm32F1(_) | AdcRegs::Stm32H7(_) => return,
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
            h7_adc_code(bits)
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

    fn write_reg_l4(r: &mut L4AdcRegs, reg: u64, value: u32) {
        match reg {
            // ISR is rc_w1 — a write clears matched flags; firmware can't SET it.
            0x00 => r.isr &= !value,
            0x04 => r.ier = value,
            0x08 => {
                r.cr = value; // latch verbatim (ADCAL self-clear not modelled)
                              // ADEN with the voltage regulator up (ADVREGEN set, DEEPPWD
                              // clear) raises ISR.ADRDY. Silicon-verified on STM32H563
                              // ADC1 (2026-06-11): DEEPPWD=0 -> ADVREGEN=1 -> ADEN=1 reads
                              // back CR=0x10000001 with ISR=0x00000001.
                let aden = value & 0x1 != 0;
                let advregen = value & (1 << 28) != 0;
                let deeppwd = value & (1 << 29) != 0;
                if aden && advregen && !deeppwd {
                    r.isr |= 0x1;
                }
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
}

impl Default for Adc {
    fn default() -> Self {
        Self::new()
    }
}

impl Peripheral for Adc {
    fn read(&self, offset: u64) -> SimResult<u8> {
        let val = match &self.regs {
            AdcRegs::Stm32F1(r) => match offset {
                0x00..=0x03 => self.sr,
                0x04..=0x07 => r.cr1,
                0x08..=0x0B => r.cr2,
                0x4C..=0x4F => self.dr,
                _ => {
                    crate::census_reg!("adc:Adc", offset, "read");
                    0
                }
            },
            AdcRegs::Stm32L4(r) => Self::read_reg_l4(r, self.dr, offset & !3),
            AdcRegs::Stm32H7(r) => Self::read_reg_h7(r, self.dr, offset & !3),
        };
        let shift = (offset % 4) * 8;
        Ok(((val >> shift) & 0xFF) as u8)
    }

    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        let shift = (offset % 4) * 8;
        let mask: u32 = 0xFF << shift;
        let val_shifted = (value as u32) << shift;

        match self.regs {
            AdcRegs::Stm32F1(_) => match offset {
                0x00..=0x03 => self.sr = (self.sr & !mask) | val_shifted,
                0x04..=0x07 => {
                    if let AdcRegs::Stm32F1(r) = &mut self.regs {
                        r.cr1 = (r.cr1 & !mask) | val_shifted;
                    }
                }
                0x08..=0x0B => {
                    // Update CR2; decide whether to kick off a conversion, then
                    // release the `regs` borrow before calling start_conversion
                    // (which mutates the shared engine fields).
                    let mut trigger = false;
                    if let AdcRegs::Stm32F1(r) = &mut self.regs {
                        let old_cr2 = r.cr2;
                        r.cr2 = (r.cr2 & !mask) | val_shifted;
                        let adon = (r.cr2 & 1) != 0;
                        let swstart = (r.cr2 & (1 << 30)) != 0;
                        let old_swstart = (old_cr2 & (1 << 30)) != 0;
                        if adon && swstart && !old_swstart {
                            r.cr2 &= !(1 << 30);
                            trigger = true;
                        }
                    }
                    if trigger {
                        self.start_conversion();
                    }
                }
                _ => {
                    crate::census_reg!("adc:Adc", offset, "write");
                }
            },
            AdcRegs::Stm32L4(_) => {
                let reg = offset & !3;
                let dr = self.dr;
                let mut full = 0;
                if let AdcRegs::Stm32L4(r) = &mut self.regs {
                    full = (Self::read_reg_l4(r, dr, reg) & !mask) | val_shifted;
                    Self::write_reg_l4(r, reg, full);
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
        }
        Ok(())
    }

    fn tick(&mut self) -> PeripheralTickResult {
        // Scheduler-mode instances are walk-skipped; the event chain owns the
        // conversion countdown. Guard against a stray direct call.
        if self.scheduler_mode() {
            return PeripheralTickResult::default();
        }
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
        // Arm the conversion-countdown chain the moment a write starts a
        // conversion (`converting` set by SWSTART on F1). L4 converts
        // synchronously in `write` and never sets `converting`, so it arms
        // nothing. delay-0 → deadline `current_cycle + 1` = the walk's next tick.
        if self.scheduler_mode() && self.converting && !self.chain_live {
            self.chain_live = true;
            vec![(0u64, 0u32)]
        } else {
            Vec::new()
        }
    }

    fn on_event(
        &mut self,
        _event_token: u32,
        _sched: &mut crate::sched::EventScheduler,
        _bus: &mut dyn crate::Bus,
    ) -> crate::sched::EventResult {
        if !self.scheduler_mode() {
            return crate::sched::EventResult::default();
        }
        // Run one cycle of the SAME conversion countdown the walk runs and pend
        // the EOC line on its verdict. Continuous mode re-arms `converting` in
        // `advance_conversion`, so re-check it AFTER and perpetuate at delay 1
        // while still converting; stop when the conversion completes (single
        // shot) so idle fast-forward engages.
        let irq = self.advance_conversion();
        self.chain_live = self.converting;
        crate::sched::EventResult {
            raise_own_irq: irq,
            reschedule_delay: self.converting.then_some(1),
            ..Default::default()
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

    #[test]
    fn test_adc_basic_conversion() {
        let mut adc = Adc::new();
        adc.write(0x08, 1).unwrap(); // ADON
        adc.write(0x0B, 1 << 6).unwrap(); // SWSTART (bit 30)

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

    #[test]
    fn test_adc_interrupt() {
        let mut adc = Adc::new();
        adc.write(0x04, 1 << 5).unwrap(); // EOCIE
        adc.write(0x08, 1).unwrap(); // ADON
        adc.write(0x0B, 1 << 6).unwrap(); // SWSTART

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
        adc.write(0x08, 1).unwrap(); // ADON
        adc.write(0x0B, 1 << 6).unwrap(); // SWSTART
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
        // EOCIE + ADON + SWSTART on channel 3 → 14-cycle countdown → EOC + DR.
        let script = [
            (1u64, Op::Write(0x04, 1 << 5)), // CR1.EOCIE
            (1, Op::Write(0x08, 1 | 3)),     // CR2.ADON, SQR-fallback channel 3
            (2, Op::Write(0x0B, 1 << 6)),    // CR2.SWSTART (bit 30) → convert
            (20, Op::Write(0x00, 0)),        // read-back settle (no-op SR write)
        ];
        assert_walk_identical(&script, 26);
    }

    #[test]
    fn f1_continuous_conversion_walk_identity() {
        // CONT + ADON: after the first EOC the engine re-arms and converts again,
        // so the chain must perpetuate across multiple completions.
        let script = [
            (1u64, Op::Write(0x04, 1 << 5)),        // CR1.EOCIE
            (1, Op::Write(0x08, 1 | (1 << 1) | 3)), // CR2.ADON | CONT | channel 3
            (2, Op::Write(0x0B, 1 << 6)),           // SWSTART → first conversion
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
        assert_eq!(dr16, h7_adc_code(16));
        assert!(dr16 > 0xFFF, "16-bit code must exceed a 12-bit full scale");
        let isr = adc.read_u32(0x00).unwrap();
        assert_eq!(isr & (1 << 2), 1 << 2, "EOC");
        assert_eq!(isr & (1 << 3), 1 << 3, "EOS");
        assert_eq!(adc.read_u32(0x08).unwrap() & (1 << 2), 0, "ADSTART clears");

        // RES=110 selects 12 bits (NOT the L4's encoding, where 0b10 is 8).
        adc.write_u32(0x0C, 0x8000_0000 | (0b110 << 2)).unwrap();
        adc.write_u32(0x08, (1 << 28) | 1 | (1 << 2)).unwrap();
        assert_eq!(adc.read_u32(0x40).unwrap(), h7_adc_code(12));

        // RES=111 is 8 bits.
        adc.write_u32(0x0C, 0x8000_0000 | (0b111 << 2)).unwrap();
        adc.write_u32(0x08, (1 << 28) | 1 | (1 << 2)).unwrap();
        assert_eq!(adc.read_u32(0x40).unwrap(), h7_adc_code(8));
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
