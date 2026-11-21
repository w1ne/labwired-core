// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Analog Devices (Maxim) MAX32650/MAX32652 10-bit ADC at `0x4003_4000`.
//! Register layout and bit fields come from the msdk CMSIS header
//! `Libraries/CMSIS/Device/Maxim/MAX32650/Include/adc_regs.h` (`CTRL` @0x00,
//! `STATUS` @0x04, `DATA` @0x08, `INTR` @0x0C, `LIMIT0..3` @0x10..0x1C) and
//! `max32650.h` (`MXC_BASE_ADC`, `ADC_IRQn` = 36). The firmware handshake is
//! msdk `PeriphDrivers/Source/ADC/adc_reva.c`: `MXC_ADC_RevA_Init` sets
//! `CTRL.PWR` and `CTRL.REFBUF_PWR`, then spins on `INTR.REF_READY_IF`;
//! `MXC_ADC_RevA_StartConversion` writes `CTRL.CH_SEL`, clears
//! `INTR.DONE_IF` (write 1 to clear), sets `CTRL.START`, waits for
//! `STATUS.ACTIVE` to drop and reads `DATA` (LSB justified, or `>> 6` with
//! `CTRL.DATA_ALIGN`). The 1.22 V internal bandgap reference, the 10-bit
//! resolution and the instant conversion come from Renode
//! `Peripherals/Analog/MAX32650_ADC.cs`.
//!
//! Modelled:
//! - `CH_SEL` 0..3 convert AIN0..AIN3. 4 and 5 (`AIN0_DIV5`, `AIN1_DIV5`)
//!   convert AIN0 and AIN1 through the on-chip divide-by-5, so pads above
//!   the 1.22 V reference resolve. Renode treats 4 and 5 as two more
//!   independent inputs; the msdk enum names them "Channel 0 divided by 5".
//! - `CTRL.INPUT_SCALE` halves the input (Renode, msdk `INPUT_SCALE_DIV2`).
//! - Codes are `V / 1.22 V * 1023` truncated and clamped to `[0, 1023]`
//!   (Renode `ToADCRawValue`). The input is sampled at the `START` write and
//!   held, as a sample-and-hold would; Renode reads it live at `DATA`.
//! - `STATUS.ACTIVE` reads 0: the conversion completes at `START`, which
//!   sets `INTR.DONE_IF`. A `REFBUF_PWR` 0 -> 1 edge sets `REF_READY_IF`.
//! - `INTR` enables (bits 0..4) are read/write, flags (bits 16..20) are
//!   write 1 to clear, and `PENDING` (bit 22) is any enabled flag. The same
//!   level drives the IRQ line.
//!
//! Not modelled: the internal supply channels (`CH_SEL` 6..12, which read 0),
//! `REF_SEL` (VDD/2 reference) and `REF_SCALE` (stored only), the limit
//! monitors (`LIMIT0..3` stored, `HI/LO_LIMIT_IF` never set),
//! `STATUS.OVERFLOW`, and the conversion clock in GCR.

use crate::{Peripheral, PeripheralTickResult, SimResult};

/// External pads AIN0..AIN3.
const PADS: usize = 4;
/// Internal bandgap (Renode `MAX32650_ADC.ReferenceVoltage`, AN6766).
const DEFAULT_VREF_MV: u32 = 1220;
const FULL_SCALE: u32 = (1 << 10) - 1;

// ── Register map (msdk adc_regs.h) ──────────────────────────────────────────
const CTRL: u64 = 0x00;
const STATUS: u64 = 0x04;
const DATA: u64 = 0x08;
const INTR: u64 = 0x0C;
const LIMIT0: u64 = 0x10;
const LIMIT_COUNT: usize = 4;

const CTRL_START: u32 = 1 << 0;
const CTRL_PWR: u32 = 1 << 1;
const CTRL_REFBUF_PWR: u32 = 1 << 3;
const CTRL_REF_SEL: u32 = 1 << 4;
const CTRL_REF_SCALE: u32 = 1 << 8;
const CTRL_INPUT_SCALE: u32 = 1 << 9;
const CTRL_CLK_EN: u32 = 1 << 11;
const CTRL_CH_SEL_POS: u32 = 12;
const CTRL_CH_SEL: u32 = 0xF << CTRL_CH_SEL_POS;
const CTRL_DATA_ALIGN: u32 = 1 << 17;
/// `START` self-clears, so it is not stored.
const CTRL_STORED: u32 = CTRL_PWR
    | CTRL_REFBUF_PWR
    | CTRL_REF_SEL
    | CTRL_REF_SCALE
    | CTRL_INPUT_SCALE
    | CTRL_CLK_EN
    | CTRL_CH_SEL
    | CTRL_DATA_ALIGN;

const INTR_IE_MASK: u32 = 0x1F;
const INTR_DONE_IF: u32 = 1 << 16;
const INTR_REF_READY_IF: u32 = 1 << 17;
const INTR_IF_MASK: u32 = 0x1F << 16;
const INTR_PENDING: u32 = 1 << 22;

/// `LIMITn`: `CH_LO_LIMIT` [9:0], `CH_HI_LIMIT` [21:12], `CH_SEL` [27:24],
/// the two enables at 28/29.
const LIMIT_MASK: u32 = 0x3F3F_F3FF;

/// MAX32650 ADC: one `START` converts the `CH_SEL` channel at 10 bits.
#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct Max32650Adc {
    ctrl: u32,
    data: u32,
    intr: u32,
    limit: [u32; LIMIT_COUNT],
    /// Routed pad levels in millivolts. `u16::MAX` means undriven.
    inputs_mv: [u16; PADS],
    vref_mv: u32,
}

impl Default for Max32650Adc {
    fn default() -> Self {
        Self::new(DEFAULT_VREF_MV)
    }
}

impl Max32650Adc {
    pub fn new(vref_mv: u32) -> Self {
        Self {
            ctrl: 0,
            data: 0,
            intr: 0,
            limit: [0; LIMIT_COUNT],
            inputs_mv: [u16::MAX; PADS],
            vref_mv: vref_mv.max(1),
        }
    }

    fn pad_mv(&self, pad: usize) -> u32 {
        match self.inputs_mv[pad] {
            u16::MAX => 0,
            mv => u32::from(mv),
        }
    }

    /// Sample `CH_SEL` and store the right-aligned 10-bit code.
    fn convert(&mut self) {
        // (millivolts, divider) seen by the converter.
        let (mv, divider) = match (self.ctrl & CTRL_CH_SEL) >> CTRL_CH_SEL_POS {
            ch @ 0..=3 => (self.pad_mv(ch as usize), 1),
            ch @ 4..=5 => (self.pad_mv(ch as usize - 4), 5),
            _ => (0, 1),
        };
        let divider = if self.ctrl & CTRL_INPUT_SCALE != 0 {
            divider * 2
        } else {
            divider
        };
        let full_scale_mv = self.vref_mv * divider;
        self.data = mv.min(full_scale_mv) * FULL_SCALE / full_scale_mv;
        self.intr |= INTR_DONE_IF;
    }

    fn read_word(&self, offset: u64) -> u32 {
        match offset & !3 {
            CTRL => self.ctrl,
            // ACTIVE, PWR_UP_ACTIVE and OVERFLOW all read 0.
            STATUS => 0,
            DATA if self.ctrl & CTRL_DATA_ALIGN != 0 => self.data << 6,
            DATA => self.data,
            INTR => {
                let pending = if self.irq_asserted() { INTR_PENDING } else { 0 };
                self.intr | pending
            }
            o @ LIMIT0..=0x1C => self.limit[((o - LIMIT0) / 4) as usize],
            _ => 0,
        }
    }

    fn write_word(&mut self, offset: u64, value: u32) {
        match offset & !3 {
            CTRL => {
                let refbuf_rising =
                    value & CTRL_REFBUF_PWR != 0 && self.ctrl & CTRL_REFBUF_PWR == 0;
                self.ctrl = value & CTRL_STORED;
                if refbuf_rising {
                    self.intr |= INTR_REF_READY_IF;
                }
                if value & CTRL_START != 0 {
                    self.convert();
                }
            }
            INTR => {
                self.intr = (self.intr & !value & INTR_IF_MASK) | (value & INTR_IE_MASK);
            }
            o @ LIMIT0..=0x1C => self.limit[((o - LIMIT0) / 4) as usize] = value & LIMIT_MASK,
            _ => {}
        }
    }

    fn irq_asserted(&self) -> bool {
        (self.intr >> 16) & self.intr & INTR_IE_MASK != 0
    }
}

impl Peripheral for Max32650Adc {
    fn adc_channel_count(&self) -> Option<u8> {
        Some(PADS as u8)
    }

    fn set_adc_channel_input(&mut self, channel: u8, millivolts: u16) -> bool {
        let Some(input) = self.inputs_mv.get_mut(channel as usize) else {
            return false;
        };
        *input = millivolts;
        true
    }

    fn clear_adc_channel_input(&mut self, channel: u8) -> bool {
        let Some(input) = self.inputs_mv.get_mut(channel as usize) else {
            return false;
        };
        *input = u16::MAX;
        true
    }

    fn irq_line_level(&self) -> Option<bool> {
        Some(self.irq_asserted())
    }

    fn on_system_reset(&mut self) {
        let inputs = self.inputs_mv;
        *self = Self::new(self.vref_mv);
        self.inputs_mv = inputs;
    }

    fn read(&self, offset: u64) -> SimResult<u8> {
        Ok((self.read_word(offset) >> ((offset & 3) * 8)) as u8)
    }

    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        let aligned = offset & !3;
        let shift = (offset & 3) * 8;
        // Merge into the stored word. INTR flags are write 1 to clear, so the
        // other bytes of a byte write must not clear them: merge enables only.
        let current = match aligned {
            INTR => self.intr & INTR_IE_MASK,
            o => self.read_word(o),
        };
        let next = (current & !(0xFF << shift)) | (u32::from(value) << shift);
        self.write_word(aligned, next);
        Ok(())
    }

    fn read_u32(&self, offset: u64) -> SimResult<u32> {
        Ok(self.read_word(offset))
    }

    fn write_u32(&mut self, offset: u64, value: u32) -> SimResult<()> {
        self.write_word(offset, value);
        Ok(())
    }

    fn tick(&mut self) -> PeripheralTickResult {
        PeripheralTickResult {
            irq: self.irq_asserted(),
            ..PeripheralTickResult::default()
        }
    }

    fn as_any(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }

    fn as_any_mut(&mut self) -> Option<&mut dyn std::any::Any> {
        Some(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn adc_with(inputs: &[(u8, u16)]) -> Max32650Adc {
        let mut adc = Max32650Adc::default();
        for &(ch, mv) in inputs {
            assert!(adc.set_adc_channel_input(ch, mv));
        }
        adc
    }

    /// msdk MXC_ADC_RevA_Init: PWR, REFBUF_PWR, spin on REF_READY_IF, clear.
    fn init(adc: &mut Max32650Adc) {
        adc.write_u32(
            INTR,
            (adc.read_u32(INTR).unwrap() & INTR_IE_MASK) | INTR_REF_READY_IF,
        )
        .unwrap();
        let ctrl = adc.read_u32(CTRL).unwrap();
        adc.write_u32(CTRL, ctrl | CTRL_PWR).unwrap();
        let ctrl = adc.read_u32(CTRL).unwrap();
        adc.write_u32(CTRL, ctrl | CTRL_REFBUF_PWR).unwrap();
        assert_ne!(
            adc.read_u32(INTR).unwrap() & INTR_REF_READY_IF,
            0,
            "REF_READY_IF"
        );
        adc.write_u32(
            INTR,
            (adc.read_u32(INTR).unwrap() & INTR_IE_MASK) | INTR_REF_READY_IF,
        )
        .unwrap();
        assert_eq!(adc.read_u32(INTR).unwrap() & INTR_IF_MASK, 0);
    }

    /// msdk MXC_ADC_RevA_StartConversion + MXC_ADC_RevA_GetData.
    fn start_conversion(adc: &mut Max32650Adc, channel: u32) -> u32 {
        let ctrl = adc.read_u32(CTRL).unwrap() & !CTRL_CH_SEL;
        adc.write_u32(CTRL, ctrl | (channel << CTRL_CH_SEL_POS))
            .unwrap();
        adc.write_u32(
            INTR,
            (adc.read_u32(INTR).unwrap() & INTR_IE_MASK) | INTR_DONE_IF,
        )
        .unwrap();
        assert_eq!(
            adc.read_u32(INTR).unwrap() & INTR_DONE_IF,
            0,
            "DONE_IF cleared"
        );
        let ctrl = adc.read_u32(CTRL).unwrap();
        adc.write_u32(CTRL, ctrl | CTRL_START).unwrap();
        assert_eq!(adc.read_u32(STATUS).unwrap() & 1, 0, "STATUS.ACTIVE drops");
        assert_ne!(adc.read_u32(INTR).unwrap() & INTR_DONE_IF, 0, "DONE_IF set");
        assert_eq!(
            adc.read_u32(CTRL).unwrap() & CTRL_START,
            0,
            "START self-clears"
        );
        let data = adc.read_u32(DATA).unwrap();
        if adc.read_u32(CTRL).unwrap() & CTRL_DATA_ALIGN == 0 {
            data & 0xFFFF
        } else {
            (data & 0xFFFF) >> 6
        }
    }

    #[test]
    fn msdk_sequence_converts_each_pad_against_the_1v22_bandgap() {
        let mut adc = adc_with(&[(0, 610), (1, 300), (2, 1220), (3, 2000)]);
        init(&mut adc);
        assert_eq!(start_conversion(&mut adc, 0), 610 * 1023 / 1220);
        assert_eq!(start_conversion(&mut adc, 0), 511, "half scale");
        assert_eq!(start_conversion(&mut adc, 1), 300 * 1023 / 1220);
        assert_eq!(start_conversion(&mut adc, 2), 1023, "full scale");
        assert_eq!(start_conversion(&mut adc, 3), 1023, "clamps above VREF");
        // AIN0_DIV5/AIN1_DIV5 resolve pads above the reference.
        adc.set_adc_channel_input(0, 2790);
        assert_eq!(start_conversion(&mut adc, 4), 2790 * 1023 / 6100);
        assert_eq!(start_conversion(&mut adc, 5), 300 * 1023 / 6100);
        // INPUT_SCALE halves the input on top of the channel divider.
        let ctrl = adc.read_u32(CTRL).unwrap();
        adc.write_u32(CTRL, ctrl | CTRL_INPUT_SCALE).unwrap();
        assert_eq!(start_conversion(&mut adc, 3), 2000 * 1023 / 2440);
        assert_eq!(start_conversion(&mut adc, 4), 2790 * 1023 / 12200);
        // Internal supply channels are not modelled and read 0; so does an
        // undriven pad.
        assert_eq!(start_conversion(&mut adc, 7), 0);
        adc.clear_adc_channel_input(1);
        assert_eq!(start_conversion(&mut adc, 1), 0);
        // Only AIN0..AIN3 take a routed input.
        assert!(!adc.set_adc_channel_input(4, 100));
        assert_eq!(adc.adc_channel_count(), Some(4));
    }

    #[test]
    fn msb_alignment_shifts_data_and_the_sample_is_held_at_start() {
        let mut adc = adc_with(&[(2, 1000)]);
        init(&mut adc);
        let ctrl = adc.read_u32(CTRL).unwrap();
        adc.write_u32(CTRL, ctrl | CTRL_DATA_ALIGN).unwrap();
        let code = 1000 * 1023 / 1220;
        assert_eq!(start_conversion(&mut adc, 2), code);
        assert_eq!(adc.read_u32(DATA).unwrap(), code << 6, "MSB justified");
        // A later input change does not reach DATA until the next START.
        adc.set_adc_channel_input(2, 100);
        assert_eq!(adc.read_u32(DATA).unwrap(), code << 6);
        assert_eq!(start_conversion(&mut adc, 2), 100 * 1023 / 1220);
    }

    #[test]
    fn done_and_ref_ready_flags_drive_pending_and_the_irq_line() {
        let mut adc = adc_with(&[(0, 500)]);
        // REF_READY_IF sets only on the REFBUF_PWR rising edge.
        adc.write_u32(CTRL, CTRL_PWR).unwrap();
        assert_eq!(adc.read_u32(INTR).unwrap(), 0);
        adc.write_u32(CTRL, CTRL_PWR | CTRL_REFBUF_PWR).unwrap();
        assert_eq!(adc.read_u32(INTR).unwrap(), INTR_REF_READY_IF);
        adc.write_u32(INTR, INTR_REF_READY_IF).unwrap();
        adc.write_u32(CTRL, CTRL_PWR | CTRL_REFBUF_PWR).unwrap();
        assert_eq!(adc.read_u32(INTR).unwrap(), 0, "no edge, no flag");
        // A flag without its enable does not pend.
        adc.write_u32(CTRL, CTRL_PWR | CTRL_REFBUF_PWR | CTRL_START)
            .unwrap();
        assert_eq!(adc.read_u32(INTR).unwrap(), INTR_DONE_IF);
        assert_eq!(adc.irq_line_level(), Some(false));
        // DONE_IE: PENDING reads 1 and the line rises.
        adc.write_u32(INTR, 1).unwrap();
        assert_eq!(adc.read_u32(INTR).unwrap(), INTR_PENDING | INTR_DONE_IF | 1);
        assert_eq!(adc.irq_line_level(), Some(true));
        assert!(adc.tick().irq);
        // A byte write to the enable byte keeps the flag (W1C needs a 1).
        adc.write(INTR, 1).unwrap();
        assert_ne!(adc.read_u32(INTR).unwrap() & INTR_DONE_IF, 0);
        // Writing 1 to DONE_IF clears it, PENDING drops with it.
        adc.write(INTR + 2, 1).unwrap();
        assert_eq!(adc.read_u32(INTR).unwrap(), 1);
        assert_eq!(adc.irq_line_level(), Some(false));
        // PENDING is read-only.
        adc.write_u32(INTR, INTR_PENDING).unwrap();
        assert_eq!(adc.read_u32(INTR).unwrap(), 0);
    }

    #[test]
    fn limits_store_their_fields_and_reset_keeps_inputs() {
        let mut adc = adc_with(&[(3, 1220)]);
        adc.write_u32(LIMIT0 + 4, 0xFFFF_FFFF).unwrap();
        assert_eq!(adc.read_u32(LIMIT0 + 4).unwrap(), LIMIT_MASK);
        adc.write_u32(CTRL, 0xFFFF_FFFF).unwrap();
        assert_eq!(adc.read_u32(CTRL).unwrap(), CTRL_STORED);
        adc.on_system_reset();
        assert_eq!(adc.read_u32(CTRL).unwrap(), 0);
        assert_eq!(adc.read_u32(LIMIT0 + 4).unwrap(), 0);
        assert_eq!(adc.read_u32(INTR).unwrap(), 0);
        init(&mut adc);
        assert_eq!(start_conversion(&mut adc, 3), 1023);
    }
}
