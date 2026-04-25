// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

use crate::{DmaDirection, DmaRequest, Peripheral, PeripheralTickResult, SimResult};
use std::any::Any;

#[derive(Debug, Default, serde::Serialize)]
struct DmaChannel {
    ccr: u32,
    cndtr: u32,
    cpar: u32,
    cmar: u32,
    active: bool,
    /// Internal pointers used during transfer. Real STM32 silicon does
    /// NOT modify the user-facing CPAR / CMAR registers as a transfer
    /// runs — it uses internal next-address registers and leaves the
    /// configured base addresses readable for the firmware. Modelling
    /// the increment as a separate field preserves that contract.
    cpar_ptr: u32,
    cmar_ptr: u32,
    /// Initial CNDTR value. Used to fire HTIF when the transfer crosses
    /// half-way (CNDTR == cndtr_initial / 2).
    cndtr_initial: u32,
}

/// STM32F1 DMA1 Controller (7 channels)
#[derive(Debug, Default, serde::Serialize)]
pub struct Dma1 {
    isr: u32,
    ifcr: u32,
    channels: [DmaChannel; 7],
}

impl Dma1 {
    pub fn new() -> Self {
        Self::default()
    }

    fn read_reg(&self, offset: u64) -> u32 {
        match offset {
            0x00 => self.isr,
            _ => {
                let chan_idx = ((offset - 0x08) / 20) as usize;
                let reg_off = (offset - 0x08) % 20;
                if chan_idx < 7 {
                    match reg_off {
                        0x00 => self.channels[chan_idx].ccr,
                        0x04 => self.channels[chan_idx].cndtr,
                        0x08 => self.channels[chan_idx].cpar,
                        0x0C => self.channels[chan_idx].cmar,
                        _ => 0,
                    }
                } else {
                    0
                }
            }
        }
    }

    fn write_reg(&mut self, offset: u64, value: u32) {
        match offset {
            0x04 => {
                // IFCR: Write 1 to clear corresponding ISR bits
                self.isr &= !value;
            }
            _ => {
                let chan_idx = ((offset - 0x08) / 20) as usize;
                let reg_off = (offset - 0x08) % 20;
                if chan_idx < 7 {
                    match reg_off {
                        0x00 => {
                            let old_en = (self.channels[chan_idx].ccr & 1) != 0;
                            self.channels[chan_idx].ccr = value;
                            let new_en = (value & 1) != 0;
                            if !old_en && new_en {
                                let chan = &mut self.channels[chan_idx];
                                chan.active = true;
                                chan.cpar_ptr = chan.cpar;
                                chan.cmar_ptr = chan.cmar;
                                chan.cndtr_initial = chan.cndtr;
                            }
                        }
                        0x04 => self.channels[chan_idx].cndtr = value & 0xFFFF,
                        0x08 => self.channels[chan_idx].cpar = value,
                        0x0C => self.channels[chan_idx].cmar = value,
                        _ => {}
                    }
                }
            }
        }
    }
}

impl Peripheral for Dma1 {
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

    fn dma_request(&mut self, request_id: u32) {
        // request_id usually corresponds to the channel (1-7) or a mapping
        let chan_idx = (request_id.saturating_sub(1)) as usize;
        if chan_idx < 7 {
            let chan = &mut self.channels[chan_idx];
            if (chan.ccr & 1) != 0 {
                // Channel is enabled, mark as active for the next tick
                chan.active = true;
            }
        }
    }

    fn tick(&mut self) -> PeripheralTickResult {
        let mut dma_requests = None;
        let mut irq = false;

        for (i, chan) in self.channels.iter_mut().enumerate() {
            if chan.active && chan.cndtr > 0 {
                let dir_bit = (chan.ccr >> 4) & 1;
                let mem2mem = (chan.ccr >> 14) & 1;

                // Use internal pointers for the actual transfer; leave
                // the user-facing CPAR / CMAR registers untouched so
                // firmware reads them at the configured base, matching
                // real STM32 hardware.
                let (src, dst, direction) = if mem2mem == 1 {
                    // STM32 mem-to-mem mode (RM0351 §11.4.7): MEM2MEM=1
                    // requires DIR=1, and the data flows CMAR -> CPAR.
                    // CMAR is "memory side" (source), CPAR is "peripheral
                    // side" (destination). Earlier code had this swapped,
                    // which only mattered once a self-test actually
                    // verified the destination word post-transfer.
                    (chan.cmar_ptr, chan.cpar_ptr, DmaDirection::Copy)
                } else if dir_bit == 1 {
                    // Memory -> peripheral: read from CMAR, write to CPAR.
                    (chan.cmar_ptr, chan.cpar_ptr, DmaDirection::Write)
                } else {
                    // Peripheral -> memory: read from CPAR, write to CMAR.
                    (chan.cpar_ptr, chan.cmar_ptr, DmaDirection::Read)
                };

                dma_requests.get_or_insert_with(Vec::new).push(DmaRequest {
                    src_addr: src as u64,
                    addr: dst as u64,
                    val: 0,
                    direction,
                });

                chan.cndtr -= 1;
                // Increment internal memory/peripheral pointers if MINC/PINC
                // is set. The CCR PSIZE/MSIZE bits select 1/2/4 byte width;
                // we treat each tick as one element so the increment matches.
                if (chan.ccr & (1 << 7)) != 0 {
                    chan.cmar_ptr = chan.cmar_ptr.wrapping_add(
                        if (chan.ccr & (1 << 10)) != 0 { 4 }
                        else if (chan.ccr & (1 << 8)) != 0 { 2 }
                        else { 1 },
                    );
                }
                if (chan.ccr & (1 << 6)) != 0 {
                    chan.cpar_ptr = chan.cpar_ptr.wrapping_add(
                        if (chan.ccr & (1 << 11)) != 0 { 4 }
                        else if (chan.ccr & (1 << 8)) != 0 { 2 }
                        else { 1 },
                    );
                }

                // HTIF: set when transfer crosses the halfway mark.
                // Matches what real silicon does for any non-trivial CNDTR.
                if chan.cndtr_initial >= 2
                    && chan.cndtr <= chan.cndtr_initial / 2
                    && (self.isr & (1 << (i * 4 + 2))) == 0
                {
                    self.isr |= 1 << (i * 4 + 2); // HTIF_x
                    self.isr |= 1 << (i * 4); // GIF_x
                    if (chan.ccr & (1 << 2)) != 0 {
                        // HTIE
                        irq = true;
                    }
                }

                if chan.cndtr == 0 {
                    chan.active = false;
                    self.isr |= 1 << (i * 4 + 1); // TCIF_x
                    self.isr |= 1 << (i * 4); // GIF_x — global IF tracks
                                              // logical-OR of TCIF/HTIF/TEIF.
                    if (chan.ccr & (1 << 1)) != 0 {
                        // TCIE
                        irq = true;
                    }
                } else if mem2mem == 0 {
                    chan.active = false;
                }
            }
        }

        PeripheralTickResult {
            irq,
            cycles: if dma_requests.is_none() { 0 } else { 1 },
            dma_requests,
            explicit_irqs: None,
            system_exception: None,
            dma_signals: None,
            ticks_until_next: None,
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
    fn test_dma_channel_completes_and_sets_irq_on_tcie() {
        let mut dma = Dma1::new();
        // CH1: CCR=EN|TCIE|DIR|MINC|PINC, one byte transfer.
        dma.write_reg(0x10, 0x2000_0010); // CH1 CPAR
        dma.write_reg(0x0C, 1); // CH1 CNDTR
        dma.write_reg(0x14, 0x2000_0020); // CH1 CMAR
        dma.write_reg(0x08, (1 << 0) | (1 << 1) | (1 << 4) | (1 << 6) | (1 << 7));

        let res = dma.tick();
        assert!(res.irq);
        assert!(res.dma_requests.is_some());
        let reqs = res.dma_requests.unwrap();
        assert_eq!(reqs.len(), 1);
        assert_eq!(reqs[0].direction, DmaDirection::Write);
        assert_eq!(reqs[0].src_addr, 0x2000_0020);
        assert_eq!(reqs[0].addr, 0x2000_0010);
        // CH1 TCIF is bit 1.
        assert_ne!(dma.read_reg(0x00) & (1 << 1), 0);
    }
}
