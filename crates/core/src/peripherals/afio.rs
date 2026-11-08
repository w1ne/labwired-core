// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

use crate::{Peripheral, SimResult};
use std::any::Any;

/// Which AFIO register file the block implements.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AfioLayout {
    /// STM32F1: EVCR, MAPR, EXTICR1..4 (four 4-bit line fields each), MAPR2.
    #[default]
    Stm32F1,
    /// WCH CH32V003 (`AFIO_TypeDef` in ch32fun's ch32v003hw.h): PCFR1 at
    /// 0x04 and ONE `EXTICR` at 0x08 with a 2-bit port field per line for
    /// lines 0..7 (00 = PA, 10 = PC, 11 = PD). 0x00 is reserved.
    Ch32v003,
    /// WCH CH32V002/V004/V005/V006/V007 (`AFIO_TypeDef` in ch32fun's
    /// ch32x00xhw.h, ch32-data `afio_v00x.yaml`): 0x00/0x04 reserved, then
    /// `EXTICR` at 0x08 with a 2-bit port field per line for lines 0..7
    /// (00 = PA, 01 = PB, 10 = PC, 11 = PD) and `PCFR1` at **0x0C** — after
    /// EXTICR, not before it as on the CH32V003. PCFR1 also has wider
    /// fields: USART1_RM [9:6], TIM1_RM [13:10], USART2_RM [22:20].
    Ch32v00x,
    /// WCH CH32X035 (`AFIO_TypeDef` in ch32fun's ch32x03xhw.h): PCFR1 at
    /// 0x04, then two EXTICRs of 2-bit port fields (00 = PA, 01 = PB,
    /// 10 = PC): `EXTICR1` at 0x08 for lines 0..15 and `EXTICR2` at 0x0C for
    /// lines 16..23. `CTLR` (USB pad pull-ups, PD switches) at 0x18 is
    /// stored only.
    Ch32x03x,
}

/// STM32F1 Alternate Function I/O (AFIO), and the WCH copies of it
/// ([`AfioLayout::Ch32v003`], [`AfioLayout::Ch32x03x`]).
#[derive(Debug, Default, serde::Serialize)]
pub struct Afio {
    pub layout: AfioLayout,
    pub evcr: u32,
    pub mapr: u32,
    pub exticr: [u32; 4], // EXTICR1, EXTICR2, EXTICR3, EXTICR4
    pub mapr2: u32,
    /// CH32X035 `AFIO_CTLR`.
    pub ctlr: u32,
    /// `MAPR` (`PCFR1` on the CH32V003) as last written, shared with the pad
    /// routes whose mapping it selects ([`crate::Peripheral::remap_register`]).
    #[serde(skip)]
    remap: std::sync::Arc<std::sync::atomic::AtomicU32>,
}

impl Afio {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn new_with_layout(layout: AfioLayout) -> Self {
        Self {
            layout,
            ..Self::default()
        }
    }

    fn set_mapr(&mut self, value: u32) {
        self.mapr = value;
        self.remap
            .store(value, std::sync::atomic::Ordering::Relaxed);
    }

    /// Returns the GPIO port index (0=A, 1=B, etc.) for a given EXTI line (0-15)
    pub fn get_exti_mapping(&self, line: u8) -> u8 {
        if self.layout == AfioLayout::Ch32x03x && (16..24).contains(&line) {
            return ((self.exticr[1] >> (u32::from(line - 16) * 2)) & 0x3) as u8;
        }
        if line >= 16 {
            return 0;
        }
        if matches!(self.layout, AfioLayout::Ch32v003 | AfioLayout::Ch32v00x) {
            // Lines 8..15 have no GPIO source on the CH32V003; the caller
            // only asks for lines with a pad (PA1/PA2, PC0..7, PD0..7).
            return ((self.exticr[0] >> (u32::from(line % 8) * 2)) & 0x3) as u8;
        }
        if self.layout == AfioLayout::Ch32x03x {
            return ((self.exticr[0] >> (u32::from(line) * 2)) & 0x3) as u8;
        }
        let reg_idx = (line / 4) as usize;
        let shift = (line % 4) * 4;
        ((self.exticr[reg_idx] >> shift) & 0xF) as u8
    }

    fn read_reg(&self, offset: u64) -> u32 {
        if self.layout == AfioLayout::Ch32v00x {
            return match offset {
                0x08 => self.exticr[0],
                0x0C => self.mapr,
                _ => {
                    crate::census_reg!("afio:Afio", offset, "read");
                    0
                }
            };
        }
        if self.layout == AfioLayout::Ch32v003 {
            return match offset {
                0x04 => self.mapr,
                0x08 => self.exticr[0],
                _ => {
                    crate::census_reg!("afio:Afio", offset, "read");
                    0
                }
            };
        }
        if self.layout == AfioLayout::Ch32x03x {
            return match offset {
                0x04 => self.mapr,
                0x08 => self.exticr[0],
                0x0C => self.exticr[1],
                0x18 => self.ctlr,
                _ => {
                    crate::census_reg!("afio:Afio", offset, "read");
                    0
                }
            };
        }
        match offset {
            0x00 => self.evcr,
            0x04 => self.mapr,
            0x08 => self.exticr[0],
            0x0C => self.exticr[1],
            0x10 => self.exticr[2],
            0x14 => self.exticr[3],
            0x1C => self.mapr2,
            _ => {
                crate::census_reg!("afio:Afio", offset, "read");
                0
            }
        }
    }

    fn write_reg(&mut self, offset: u64, value: u32) {
        if self.layout == AfioLayout::Ch32v00x {
            match offset {
                // EXTICR: eight 2-bit fields in [15:0].
                0x08 => self.exticr[0] = value & 0x0000_FFFF,
                // PCFR1: SPI1_RM [2:0] .. USART2_RM [22:20] and SWCFG
                // [26:24]; [31:27] are reserved. USART1_RM and USART2_RM
                // move the USART TX pads (`SystemBus::wire_stm32_uart_pads`);
                // the other fields are stored only.
                0x0C => self.set_mapr(value & 0x07FF_FFFF),
                _ => {
                    crate::census_reg!("afio:Afio", offset, "write");
                }
            }
            return;
        }
        if self.layout == AfioLayout::Ch32v003 {
            match offset {
                // PCFR1, low 16 bits. TIM1_RM [7:6] moves the TIM1 pads
                // (`SystemBus::wire_stm32_timer_output_pads`); the other
                // remap fields are stored only.
                0x04 => self.set_mapr(value & 0x0000_FFFF),
                // EXTICR: eight 2-bit fields in [15:0].
                0x08 => self.exticr[0] = value & 0x0000_FFFF,
                _ => {
                    crate::census_reg!("afio:Afio", offset, "write");
                }
            }
            return;
        }
        if self.layout == AfioLayout::Ch32x03x {
            match offset {
                // PCFR1: no pad route on this part reads it; stored only.
                0x04 => self.set_mapr(value),
                // EXTICR1: sixteen 2-bit fields; EXTICR2: eight, in [15:0].
                // 0b11 selects no port on a three-port part; it is stored.
                0x08 => self.exticr[0] = value,
                0x0C => self.exticr[1] = value & 0x0000_FFFF,
                0x18 => self.ctlr = value,
                _ => {
                    crate::census_reg!("afio:Afio", offset, "write");
                }
            }
            return;
        }
        // Writable masks silicon-confirmed on the bench F103 via the address
        // sweep (stm32f1_mmio_diff). AFIO is an F1-only peripheral (F4+ use
        // SYSCFG) and F103 is the only F1 chip, so these are exact, not gated.
        match offset {
            // EVCR: PIN[3:0]/PORT[6:4]/EVOE(7) = 0xFF.
            0x00 => self.evcr = value & 0x0000_00FF,
            // MAPR: the remap field readable on F103-medium is the low 16 bits;
            // the ADC-ETRG remap bits [20:16], the reserved [23:21]/[31:27], and
            // the write-only SWJ_CFG [26:24] all read back 0 here.
            0x04 => self.set_mapr(value & 0x0000_FFFF),
            // EXTICR: four 4-bit line-source fields; bit 15 (the 4th port-select
            // bit of the top field) and [31:16] read 0 → 0x7FFF.
            0x08 => self.exticr[0] = value & 0x7FFF,
            0x0C => self.exticr[1] = value & 0x7FFF,
            0x10 => self.exticr[2] = value & 0x7FFF,
            0x14 => self.exticr[3] = value & 0x7FFF,
            // MAPR2 is not implemented on F103 medium-density — reads 0.
            0x1C => self.mapr2 = 0,
            _ => {
                crate::census_reg!("afio:Afio", offset, "write");
            }
        }
    }
}

impl Peripheral for Afio {
    fn read(&self, offset: u64) -> SimResult<u8> {
        let reg_offset = offset & !3;
        let byte_offset = (offset % 4) as u32;
        let reg_val = self.read_reg(reg_offset);
        Ok(((reg_val >> (byte_offset * 8)) & 0xFF) as u8)
    }

    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        let reg_offset = offset & !3;
        let byte_offset = (offset % 4) as u32;

        let mut reg_val = self.read_reg(reg_offset);
        let mask = 0xFF << (byte_offset * 8);
        reg_val &= !mask;
        reg_val |= (value as u32) << (byte_offset * 8);

        self.write_reg(reg_offset, reg_val);
        Ok(())
    }

    /// AFIO_EXTICRx is the F1's EXTI line-source mux (AFIO_EXTICR, lines
    /// 0..7, on the CH32V003; EXTICR1/2, lines 0..23, on the CH32X035).
    fn exti_line_source(&self, line: u8) -> Option<u8> {
        let lines = match self.layout {
            AfioLayout::Stm32F1 => 16,
            AfioLayout::Ch32v003 | AfioLayout::Ch32v00x => 8,
            AfioLayout::Ch32x03x => 24,
        };
        (line < lines).then(|| self.get_exti_mapping(line))
    }

    /// `MAPR` / CH32V003 `PCFR1`, for the
    /// [`crate::peripherals::pad_routing::RemapGate`]s on the pads it moves.
    fn remap_register(&self) -> Option<std::sync::Arc<std::sync::atomic::AtomicU32>> {
        Some(self.remap.clone())
    }

    fn needs_legacy_walk(&self) -> bool {
        // AFIO is a pure configuration register bank (MAPR/EXTICR remap bits).
        // It has no `tick()` override, so its per-cycle walk callback is the
        // default no-op: it never mutates observable state, emits an IRQ/DMA
        // request, or fires an event from the walk. Deleting the walk is
        // therefore byte-identical for every reachable firmware state, so it
        // must not pin the walk on for the rest of the bus.
        false
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
    use super::{Afio, AfioLayout};
    use crate::Peripheral;

    fn rd32(a: &Afio, off: u64) -> u32 {
        (0..4).fold(0u32, |acc, i| {
            acc | ((a.read(off + i).unwrap() as u32) << (i * 8))
        })
    }

    #[test]
    fn mapr_reserved_bits_read_zero() {
        // Implemented remap bits [20:0] stick; reserved bits [23:21]/[31:27] and
        // the write-only SWJ_CFG [26:24] read back 0. Silicon-verified on the
        // bench STM32F103 (stm32f1_exec_oracle::afio_mapr_reserved_bits...).
        let mut a = Afio::new();
        a.write_u32(0x04, 0x0820_0004).unwrap(); // reserved 27,21 + USART1_REMAP
        assert_eq!(rd32(&a, 0x04), 0x0000_0004);

        let mut b = Afio::new();
        b.write_u32(0x04, 0xFFFF_FFFF).unwrap();
        // The address sweep on the bench F103 returned 0xFFFF for the remap
        // field (bits [20:16] read 0 on this part, with SWJ_CFG held 0).
        assert_eq!(rd32(&b, 0x04), 0x0000_FFFF);
    }

    #[test]
    fn exticr_upper_half_reads_zero() {
        // EXTICR1..4 implement four 4-bit fields in [15:0]; [31:16] reserved → 0.
        // Silicon-verified on F103 (stm32f1_exec_oracle::afio_exticr1_upper...).
        let mut a = Afio::new();
        for off in [0x08u64, 0x0C, 0x10, 0x14] {
            a.write_u32(off, 0xFFFF_FFFF).unwrap();
            // Bench F103 sweep: bit 15 (top of the 4th field) also reads 0 → 0x7FFF.
            assert_eq!(rd32(&a, off), 0x0000_7FFF, "EXTICR @ 0x{off:02X}");
        }
    }

    #[test]
    fn exticr_is_the_exti_line_source() {
        let mut a = Afio::new();
        // EXTICR1: line 0 -> port B; EXTICR2: line 5 -> port C.
        a.write_u32(0x08, 0x1).unwrap();
        a.write_u32(0x0C, 0x2 << 4).unwrap();
        assert_eq!(a.exti_line_source(0), Some(1));
        assert_eq!(a.exti_line_source(5), Some(2));
        assert_eq!(a.exti_line_source(4), Some(0), "reset: port A");
        assert_eq!(a.exti_line_source(16), None);
    }

    #[test]
    fn ch32v003_exticr_is_one_register_of_2_bit_fields() {
        let mut a = Afio::new_with_layout(AfioLayout::Ch32v003);
        // ch32fun exti_pin_change_isr: AFIO_EXTICR_EXTI3_PD = 0xC0.
        a.write_u32(0x08, 0xC0).unwrap();
        assert_eq!(a.exti_line_source(3), Some(3), "line 3 -> port D");
        assert_eq!(a.exti_line_source(0), Some(0), "reset: port A");
        // Line 7 -> PC (0b10 in [15:14]); the upper half is not implemented.
        a.write_u32(0x08, 0xFFFF_0000 | (0b10 << 14)).unwrap();
        assert_eq!(rd32(&a, 0x08), 0x8000);
        assert_eq!(a.exti_line_source(7), Some(2));
        assert_eq!(a.exti_line_source(8), None, "no GPIO line 8");
        // The F1 EXTICR2..4 and MAPR2 offsets do not exist here.
        a.write_u32(0x0C, 0xFFFF).unwrap();
        assert_eq!(rd32(&a, 0x0C), 0);
        assert_eq!(rd32(&a, 0x00), 0, "0x00 is reserved");
    }

    #[test]
    fn ch32v00x_exticr_at_0x08_and_pcfr1_at_0x0c() {
        let mut a = Afio::new_with_layout(AfioLayout::Ch32v00x);
        // ch32fun dual_usart: USART2_RM = 3 in [22:20].
        a.write_reg(0x0C, 3 << 20);
        // EXTI line 2 from PD (11), line 1 from PB (01).
        a.write_reg(0x08, (0b11 << 4) | (0b01 << 2));
        assert_eq!(a.read_reg(0x0C), 3 << 20);
        assert_eq!(a.read_reg(0x04), 0, "0x04 is reserved on the V00x");
        assert_eq!(
            a.remap_register()
                .unwrap()
                .load(std::sync::atomic::Ordering::Relaxed),
            3 << 20
        );
        assert_eq!(a.exti_line_source(2), Some(3));
        assert_eq!(a.exti_line_source(1), Some(1));
        assert_eq!(a.exti_line_source(8), None);
        // Reserved [31:27] read back 0.
        a.write_reg(0x0C, 0xFFFF_FFFF);
        assert_eq!(a.read_reg(0x0C), 0x07FF_FFFF);
    }

    #[test]
    fn ch32x03x_exticr1_and_exticr2_cover_lines_0_to_23() {
        let mut a = Afio::new_with_layout(AfioLayout::Ch32x03x);
        // ch32fun examples_x035/exti_pin_change_isr: AFIO_EXTICR1_EXTI7_PC.
        a.write_u32(0x08, 0x0000_8000).unwrap();
        assert_eq!(a.exti_line_source(7), Some(2), "line 7 -> port C");
        assert_eq!(a.exti_line_source(6), Some(0), "reset: port A");
        // Line 15 sits in EXTICR1's top field; line 23 in EXTICR2 [15:14].
        a.write_u32(0x08, 0x4000_0000).unwrap();
        assert_eq!(a.exti_line_source(15), Some(1), "line 15 -> port B");
        a.write_u32(0x0C, 0xFFFF_0000 | (0b10 << 14) | 0b01)
            .unwrap();
        assert_eq!(rd32(&a, 0x0C), 0x8001);
        assert_eq!(a.exti_line_source(16), Some(1));
        assert_eq!(a.exti_line_source(23), Some(2));
        assert_eq!(a.exti_line_source(24), None, "no GPIO line 24");
        // CTLR at 0x18 is storage; the F1 EXTICR3/4 offsets do not exist.
        a.write_u32(0x18, 0x0000_000F).unwrap();
        assert_eq!(rd32(&a, 0x18), 0xF);
        a.write_u32(0x10, 0xFFFF).unwrap();
        assert_eq!(rd32(&a, 0x10), 0);
    }
}
