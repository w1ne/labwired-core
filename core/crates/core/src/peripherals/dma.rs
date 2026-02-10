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
                                self.channels[chan_idx].active = true;
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

    fn tick(&mut self) -> PeripheralTickResult {
        let mut dma_requests = Vec::new();
        let mut irq = false;

        for (i, chan) in self.channels.iter_mut().enumerate() {
            if chan.active && chan.cndtr > 0 {
                // Determine direction based on CCR bit 4 (DIR)
                // 0: Read from peripheral, write to memory
                // 1: Read from memory, write to peripheral
                let dir_bit = (chan.ccr >> 4) & 1;

                // For a single tick, we transfer one item?
                // STM32 DMA can be very fast, but 1 byte per tick is a good start.

                // Note: Simplified logic assumes 8-bit transfers for now.
                // In reality, PSIZE/MSIZE determine 8/16/32 bit.

                if dir_bit == 1 {
                    // Memory to Peripheral
                    dma_requests.push(DmaRequest {
                        addr: chan.cmar as u64,
                        val: 0, // Value will be populated by SystemBus during Read
                        direction: DmaDirection::Read,
                    });
                    // Wait, our DmaRequest doesn't support "Read then Write".
                    // It's either Read OR Write.
                    // For a real DMA, it's a two-stage process.
                    // To keep it simple in one tick, we'll assume we know the value?
                    // No, that's what buses are for.

                    // Let's refine DmaRequest: maybe we need a way to store the read value.
                    // For now, let's just implement Memory-to-Memory manually for testing.
                } else {
                    // Peripheral to Memory
                    // In a real system, the peripheral would trigger the DMA.
                    // For now, let's just implement a simple memory-to-memory copy if MEM2MEM (bit 14) is set.
                    if (chan.ccr & (1 << 14)) != 0 {
                        // MEM2MEM mode
                        // We need to read from CPAR and write to CMAR (or vice versa depending on DIR?)
                        // Spec: MEM2MEM=1, DIR=1 (Memory to Memory).
                        // Actually, in MEM2MEM, DIR is usually ignored or determines src/dst.

                        // Let's just implement a fake write for now to verify the plumbing.
                        dma_requests.push(DmaRequest {
                            addr: chan.cmar as u64,
                            val: 0x42, // Dummy value
                            direction: DmaDirection::Write,
                        });

                        chan.cndtr -= 1;
                        if (chan.ccr & (1 << 7)) != 0 {
                            chan.cmar += 1;
                        } // MINC
                        if (chan.ccr & (1 << 6)) != 0 {
                            chan.cpar += 1;
                        } // PINC

                        if chan.cndtr == 0 {
                            chan.active = false;
                            // Set TCIF (Transfer Complete Interrupt Flag) in ISR
                            self.isr |= 1 << (i * 4 + 1);
                            if (chan.ccr & (1 << 1)) != 0 {
                                // TCIE
                                irq = true;
                            }
                        }
                    }
                }
            }
        }

        PeripheralTickResult {
            irq,
            cycles: if dma_requests.is_empty() { 0 } else { 1 },
            dma_requests,
            explicit_irqs: Vec::new(),
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
