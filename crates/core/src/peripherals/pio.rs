// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

use crate::Peripheral;
use crate::PeripheralTickResult;
use crate::SimResult;
use anyhow::Result;

#[derive(Debug, Default, Clone)]
pub struct StateMachine {
    pub pc: u8,
    pub x: u32,
    pub y: u32,
    pub isr: u32,
    pub osr: u32,
    pub isr_count: u8,
    pub osr_count: u8,

    pub clkdiv_int: u16,
    pub clkdiv_frac: u8,
    pub clk_counter: u32,
    pub delay_cycles: u8,

    pub wrap_top: u8,
    pub wrap_bottom: u8,

    pub exec_ctrl: u32,
    pub shift_ctrl: u32,
    pub pin_ctrl: u32,

    pub enabled: bool,
    pub stalled: bool,
}

#[derive(Debug)]
pub struct Pio {
    pub instruction_mem: [u16; 32],
    pub sm: [StateMachine; 4],

    pub ctrl: u32,
    pub fstat: u32,
    pub fdebug: u32,
    pub flevel: u32,

    pub irq: u8,
    pub irq_force: u8,

    pub input_sync_bypass: u32,

    pub tx_fifo: [Vec<u32>; 4],
    pub rx_fifo: [Vec<u32>; 4],

    // Write accumulator: buffers byte writes until a complete 32-bit word
    write_acc: u32,
    write_acc_mask: u8, // bitmask of which bytes have been written (0b1111 = complete)
    write_acc_offset: u64, // register offset being accumulated
}

impl Default for Pio {
    fn default() -> Self {
        Self::new()
    }
}

impl Pio {
    pub fn new() -> Self {
        let mut sm: [StateMachine; 4] = Default::default();
        for item in &mut sm {
            item.clkdiv_int = 1;
            item.exec_ctrl = 0x0001f000; // wrap_top=31, wrap_bottom=0
        }
        Self {
            instruction_mem: [0; 32],
            sm,
            ctrl: 0,
            fstat: 0x0f000f00, // TXEMPTY set for all 4
            fdebug: 0,
            flevel: 0,
            irq: 0,
            irq_force: 0,
            input_sync_bypass: 0,
            tx_fifo: Default::default(),
            rx_fifo: Default::default(),
            write_acc: 0,
            write_acc_mask: 0,
            write_acc_offset: 0xFFFF_FFFF,
        }
    }

    pub fn load_program_asm(&mut self, asm: &str) -> Result<()> {
        let programs = pio_parser::Parser::<32>::parse_file(asm)
            .map_err(|e| anyhow::anyhow!("PIO Parse Error: {:?}", e))?;

        // Extract instructions from the first program
        let (name, program) = programs
            .iter()
            .next()
            .ok_or_else(|| anyhow::anyhow!("No PIO programs found in assembly"))?;

        tracing::debug!("Loading PIO program: {}", name);

        for (i, &instr) in program.program.code.iter().enumerate() {
            if i < 32 {
                self.instruction_mem[i] = instr;
            }
        }

        // Apply program settings to all state machines
        for i in 0..4 {
            self.sm[i].wrap_bottom = program.program.wrap.target;
            self.sm[i].wrap_top = program.program.wrap.source;

            // Update exec_ctrl with wrap settings
            // RP2040 EXEC_CTRL bits: 11:7 is wrap_bottom, 16:12 is wrap_top
            self.sm[i].exec_ctrl &= !(0x1F << 7);
            self.sm[i].exec_ctrl |= (program.program.wrap.target as u32) << 7;
            self.sm[i].exec_ctrl &= !(0x1F << 12);
            self.sm[i].exec_ctrl |= (program.program.wrap.source as u32) << 12;
        }

        Ok(())
    }

    pub fn read_reg(&self, addr: u64) -> u32 {
        let offset = addr & 0x1FF;
        match offset {
            0x000 => self.ctrl,
            0x004 => self.fstat,
            0x008 => self.fdebug,
            0x00c => self.flevel,
            0x020..=0x02c => {
                let idx = ((offset - 0x020) / 4) as usize;
                if idx < 4 && !self.rx_fifo[idx].is_empty() {
                    self.rx_fifo[idx][0]
                } else {
                    0
                }
            }
            0x030 => self.irq as u32,
            0x038 => self.input_sync_bypass,
            0x048..=0x0c4 => {
                let idx = ((offset - 0x048) / 4) as usize;
                if idx < 32 {
                    self.instruction_mem[idx] as u32
                } else {
                    0
                }
            }
            0x0c8..=0x124 => {
                let sm_idx = ((offset - 0x0c8) / 24) as usize;
                let reg_off = (offset - 0x0c8) % 24;
                if sm_idx < 4 {
                    let sm = &self.sm[sm_idx];
                    match reg_off {
                        0 => ((sm.clkdiv_int as u32) << 16) | ((sm.clkdiv_frac as u32) << 8),
                        4 => sm.exec_ctrl,
                        8 => sm.shift_ctrl,
                        12 => sm.pc as u32,
                        _ => 0,
                    }
                } else {
                    0
                }
            }
            _ => 0,
        }
    }

    pub fn write_reg(&mut self, addr: u64, value: u32) {
        let offset = addr & 0x1FF;
        match offset {
            0x000 => {
                self.ctrl = value;
                for i in 0..4 {
                    self.sm[i].enabled = (value & (1 << i)) != 0;
                }
            }
            0x008 => self.fdebug &= !value, // W1C
            0x010..=0x01c => {
                let sm_idx = ((offset - 0x010) / 4) as usize;
                if self.tx_fifo[sm_idx].len() < 4 {
                    self.tx_fifo[sm_idx].push(value);
                } else {
                    self.fdebug |= 1 << (16 + sm_idx); // TXOVER
                }
            }
            0x030 => self.irq &= !(value as u8), // W1C
            0x034 => self.irq |= value as u8,    // IRQ_FORCE
            0x038 => self.input_sync_bypass = value,
            0x048..=0x0c4 => {
                let idx = ((offset - 0x048) / 4) as usize;
                if idx < 32 {
                    self.instruction_mem[idx] = value as u16;
                }
            }
            0x0c8..=0x124 => {
                let sm_idx = ((offset - 0x0c8) / 24) as usize;
                let reg_off = (offset - 0x0c8) % 24;
                if sm_idx < 4 {
                    match reg_off {
                        0 => {
                            self.sm[sm_idx].clkdiv_int = (value >> 16) as u16;
                            self.sm[sm_idx].clkdiv_frac = (value >> 8) as u8;
                        }
                        4 => self.sm[sm_idx].exec_ctrl = value,
                        8 => self.sm[sm_idx].shift_ctrl = value,
                        12 => self.sm[sm_idx].pc = (value & 0x1F) as u8,
                        _ => {}
                    }
                }
            }
            _ => {}
        }
    }
}

impl Peripheral for Pio {
    fn read(&self, offset: u64) -> SimResult<u8> {
        let reg_offset = offset & !3;
        let byte_offset = (offset % 4) as u32;
        let reg_val = self.read_reg(reg_offset);
        Ok(((reg_val >> (byte_offset * 8)) & 0xFF) as u8)
    }

    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        let reg_offset = offset & !3;
        let byte_offset = (offset % 4) as u32;

        // Start new accumulation if register changed
        if reg_offset != self.write_acc_offset {
            self.write_acc = 0;
            self.write_acc_mask = 0;
            self.write_acc_offset = reg_offset;
        }

        // Accumulate byte
        let mask = 0xFFu32 << (byte_offset * 8);
        self.write_acc = (self.write_acc & !mask) | ((value as u32) << (byte_offset * 8));
        self.write_acc_mask |= 1 << byte_offset;

        // Once all 4 bytes are written, commit the full 32-bit value
        if self.write_acc_mask == 0x0F {
            self.write_reg(reg_offset, self.write_acc);
            self.write_acc = 0;
            self.write_acc_mask = 0;
            self.write_acc_offset = 0xFFFF_FFFF;
        }
        Ok(())
    }

    fn tick(&mut self) -> PeripheralTickResult {
        for i in 0..4 {
            let mut sm = self.sm[i].clone();
            if !sm.enabled {
                continue;
            }

            // Clock divider
            let div_int = if sm.clkdiv_int == 0 {
                65536
            } else {
                sm.clkdiv_int as u32
            };
            let div_total = (div_int << 8) | (sm.clkdiv_frac as u32);

            sm.clk_counter += 256;
            if sm.clk_counter < div_total {
                self.sm[i] = sm;
                continue;
            }
            sm.clk_counter -= div_total;

            if sm.delay_cycles > 0 {
                sm.delay_cycles -= 1;
                self.sm[i] = sm;
                continue;
            }

            let instr = self.instruction_mem[sm.pc as usize];
            let opcode = (instr >> 13) & 0x7;
            let delay_side = (instr >> 8) & 0x1F;

            let mut pc_overridden = false;
            let mut stalled = false;

            // Simple Delay logic (for now, assuming all 5 bits are Delay)
            sm.delay_cycles = delay_side as u8;

            match opcode {
                0 => {
                    // JMP
                    let cond = (instr >> 5) & 0x7;
                    let target = instr & 0x1F;
                    let should_jump = match cond {
                        0 => true,
                        1 => sm.x == 0,
                        2 => {
                            let tmp = sm.x != 0;
                            sm.x = sm.x.saturating_sub(1);
                            tmp
                        }
                        3 => sm.y == 0,
                        4 => {
                            let tmp = sm.y != 0;
                            sm.y = sm.y.saturating_sub(1);
                            tmp
                        }
                        5 => sm.x != sm.y,
                        7 => sm.osr_count == 0,
                        _ => false,
                    };
                    if should_jump {
                        sm.pc = target as u8;
                        pc_overridden = true;
                    }
                }
                1 => {
                    // WAIT
                    let pol = (instr >> 7) & 0x1 != 0;
                    let source = (instr >> 5) & 0x3;
                    let index = instr & 0x1F;

                    let val = match source {
                        2 => (self.irq & (1 << index)) != 0,
                        _ => false, // TODO: GPIO/PIN
                    };

                    if val != pol {
                        stalled = true;
                    }
                }
                2 => {
                    // IN
                    let src = (instr >> 5) & 0x7;
                    let count = if (instr & 0x1F) == 0 {
                        32
                    } else {
                        instr & 0x1F
                    };
                    let val = match src {
                        1 => sm.x,
                        2 => sm.y,
                        6 => sm.isr,
                        7 => sm.osr,
                        _ => 0,
                    };
                    let mask = if count == 32 {
                        0xFFFFFFFF
                    } else {
                        (1 << count) - 1
                    };
                    if count == 32 {
                        sm.isr = val;
                    } else {
                        sm.isr = (sm.isr >> count) | ((val & mask) << (32 - count));
                    }
                    sm.isr_count = (sm.isr_count + count as u8).min(32);
                }
                3 => {
                    // OUT
                    let dest = (instr >> 5) & 0x7;
                    let count = if (instr & 0x1F) == 0 {
                        32
                    } else {
                        instr & 0x1F
                    };
                    let val = if sm.osr_count >= count as u8 {
                        let mask = if count == 32 {
                            0xFFFFFFFF
                        } else {
                            (1 << count) - 1
                        };
                        let tmp = sm.osr & mask;
                        if count == 32 {
                            sm.osr = 0;
                        } else {
                            sm.osr >>= count;
                        }
                        sm.osr_count -= count as u8;
                        tmp
                    } else {
                        stalled = true;
                        0
                    };
                    if !stalled {
                        match dest {
                            1 => sm.x = val,
                            2 => sm.y = val,
                            5 => {
                                sm.pc = (val & 0x1F) as u8;
                                pc_overridden = true;
                            }
                            6 => {
                                sm.isr = val;
                                sm.isr_count = count as u8;
                            }
                            _ => {}
                        }
                    }
                }
                4 => {
                    // PUSH / PULL
                    let block = (instr >> 5) & 0x1 != 0;
                    let is_pull = (instr >> 7) & 0x1 != 0;
                    if is_pull {
                        if !self.tx_fifo[i].is_empty() {
                            sm.osr = self.tx_fifo[i].remove(0);
                            sm.osr_count = 32;
                        } else if block {
                            stalled = true;
                        } else {
                            sm.osr = sm.x;
                            sm.osr_count = 32;
                        }
                    } else if self.rx_fifo[i].len() < 4 {
                        self.rx_fifo[i].push(sm.isr);
                        sm.isr = 0;
                        sm.isr_count = 0;
                    } else if block {
                        stalled = true;
                    }
                }
                5 => {
                    // MOV
                    let dest = (instr >> 5) & 0x7;
                    let op = (instr >> 3) & 0x3;
                    let src = instr & 0x7;
                    let mut val = match src {
                        1 => sm.x,
                        2 => sm.y,
                        6 => sm.isr,
                        7 => sm.osr,
                        _ => 0,
                    };
                    match op {
                        1 => val = !val,
                        2 => val = val.reverse_bits(),
                        _ => {}
                    }
                    match dest {
                        1 => sm.x = val,
                        2 => sm.y = val,
                        4 => {
                            // EXEC (MOV only): execute val as instruction
                            // Simplified placeholder
                        }
                        5 => {
                            sm.pc = (val & 0x1F) as u8;
                            pc_overridden = true;
                        }
                        6 => sm.isr = val,
                        7 => sm.osr = val,
                        _ => {}
                    }
                }
                6 => {
                    // IRQ
                    let set_clear = (instr >> 6) & 0x1;
                    let index = instr & 0x7;
                    if set_clear == 0 {
                        self.irq |= 1 << index;
                    } else {
                        self.irq &= !(1 << index);
                    }
                }
                7 => {
                    // SET
                    let dest = (instr >> 5) & 0x7;
                    let data = instr & 0x1F;
                    match dest {
                        1 => sm.x = data as u32,
                        2 => sm.y = data as u32,
                        _ => {}
                    }
                }
                _ => {}
            }

            sm.stalled = stalled;
            if !pc_overridden && !stalled {
                let wrap_top = ((sm.exec_ctrl >> 12) & 0x1F) as u8;
                let wrap_bottom = ((sm.exec_ctrl >> 7) & 0x1F) as u8;
                if sm.pc == wrap_top {
                    sm.pc = wrap_bottom;
                } else {
                    sm.pc = (sm.pc + 1) & 0x1F;
                }
            }
            self.sm[i] = sm;
        }

        self.fstat = 0;
        for i in 0..4 {
            if self.tx_fifo[i].is_empty() {
                self.fstat |= 1 << (24 + i);
            }
            if self.tx_fifo[i].len() == 4 {
                self.fstat |= 1 << (16 + i);
            }
            if self.rx_fifo[i].is_empty() {
                self.fstat |= 1 << (8 + i);
            }
            if self.rx_fifo[i].len() == 4 {
                self.fstat |= 1 << i;
            }
        }

        PeripheralTickResult {
            irq: false,
            cycles: 1,
            ..Default::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pio_set_jmp() {
        let mut pio = Pio::new();
        // SET X, 10 -> 0xE02A
        pio.instruction_mem[0] = 0xE02A;
        // JMP 0
        pio.instruction_mem[1] = 0x0000;

        pio.write_reg(0, 1); // Enable SM0

        pio.tick(); // Execute SET X, 10
        assert_eq!(pio.sm[0].x, 10);
        assert_eq!(pio.sm[0].pc, 1);

        pio.tick(); // Execute JMP 0
        assert_eq!(pio.sm[0].pc, 0);
    }

    #[test]
    fn test_pio_fifo() {
        let mut pio = Pio::new();
        // PULL blocking -> 0x80A0
        pio.instruction_mem[0] = 0x80A0;
        // SET X, 31 -> 0xE03F (Dest X=1, Data=31)
        pio.instruction_mem[1] = 0xE03F;
        // PUSH blocking -> 0x8020
        pio.instruction_mem[2] = 0x8020;

        pio.write_reg(0, 1); // Enable SM0

        // Execute PULL - should stall
        pio.tick();
        assert!(pio.sm[0].stalled);

        // Feed FIFO
        pio.write_reg(0x10, 123);

        pio.tick(); // Execute PULL
        assert_eq!(pio.sm[0].osr, 123);
        assert!(!pio.sm[0].stalled);
        assert_eq!(pio.sm[0].pc, 1);

        pio.tick(); // SET X, 31
        assert_eq!(pio.sm[0].x, 31);
        assert_eq!(pio.sm[0].pc, 2);

        pio.tick(); // PUSH
        assert_eq!(pio.rx_fifo[0][0], 0);
        assert_eq!(pio.sm[0].pc, 3);
    }

    #[test]
    fn test_pio_delay() {
        let mut pio = Pio::new();
        // SET X, 10 with Delay 2 -> 0xE02A | (2 << 8) = 0xE22A
        pio.instruction_mem[0] = 0xE22A;
        // SET Y, 5
        pio.instruction_mem[1] = 0xE045;

        pio.write_reg(0, 1); // Enable SM0

        pio.tick(); // Execute SET X, 10, set delay=2
        assert_eq!(pio.sm[0].x, 10);
        assert_eq!(pio.sm[0].pc, 1);
        assert_eq!(pio.sm[0].delay_cycles, 2);

        pio.tick(); // Delay 1
        assert_eq!(pio.sm[0].delay_cycles, 1);
        assert_eq!(pio.sm[0].pc, 1);

        pio.tick(); // Delay 2
        assert_eq!(pio.sm[0].delay_cycles, 0);
        assert_eq!(pio.sm[0].pc, 1);

        pio.tick(); // Execute SET Y, 5
        assert_eq!(pio.sm[0].y, 5);
        assert_eq!(pio.sm[0].pc, 2);
    }

    #[test]
    fn test_pio_assembly() {
        let mut pio = Pio::new();
        let asm = ".program test\nset x, 10\n";
        pio.load_program_asm(asm).unwrap();

        // 0xE02A is SET X, 10
        assert_eq!(pio.instruction_mem[0], 0xE02A);
    }

    #[test]
    fn test_pio_mov() {
        let mut pio = Pio::new();
        pio.sm[0].x = 0xAAAAAAAA;
        // MOV Y, X -> 0xA041
        pio.instruction_mem[0] = 0xA041;
        // MOV X, !X -> 0xA029 (Dest X=1, Op Not=1, Src X=1)
        pio.instruction_mem[1] = 0xA029;
        // MOV Y, ::X -> 0xA051 (Dest Y=2, Op Rev=2, Src X=1)
        pio.instruction_mem[2] = 0xA051;

        pio.write_reg(0, 1); // Enable SM0

        pio.tick();
        assert_eq!(pio.sm[0].y, 0xAAAAAAAA);
        assert_eq!(pio.sm[0].pc, 1);

        pio.tick();
        assert_eq!(pio.sm[0].x, 0x55555555);
        assert_eq!(pio.sm[0].pc, 2);

        pio.sm[0].x = 0x80000000;
        pio.tick();
        assert_eq!(pio.sm[0].y, 0x00000001);
    }

    #[test]
    fn test_pio_irq() {
        let mut pio = Pio::new();
        // IRQ set 7 -> 0xC007
        pio.instruction_mem[0] = 0xC007;
        // IRQ clear 7 -> 0xC047
        pio.instruction_mem[1] = 0xC047;

        pio.write_reg(0, 1); // Enable SM0

        pio.tick();
        assert_eq!(pio.irq, 0x80);

        pio.tick();
        assert_eq!(pio.irq, 0x00);
    }

    #[test]
    fn test_pio_wait_irq() {
        let mut pio = Pio::new();
        // WAIT 1, IRQ, 3 -> 0x20C3 (Pol=1, Src=IRQ=2, Idx=3)
        // 0x2000 (WAIT) | 0x80 (Pol=1) | 0x40 (Src=2) | 0x03 (Idx=3) = 0x20C3
        pio.instruction_mem[0] = 0x20C3;
        
        pio.write_reg(0, 1); // Enable SM0

        pio.tick();
        assert!(pio.sm[0].stalled);
        assert_eq!(pio.sm[0].pc, 0);

        pio.irq |= 1 << 3;
        pio.tick();
        assert!(!pio.sm[0].stalled);
        assert_eq!(pio.sm[0].pc, 1);
    }

    #[test]
    fn test_pio_wrap() {
        let mut pio = Pio::new();
        // SET X, 1 -> 0xE021
        pio.instruction_mem[0] = 0xE021;
        // SET Y, 2 -> 0xE042
        pio.instruction_mem[1] = 0xE042;
        
        // Wrap at PC=1 back to 0
        pio.sm[0].exec_ctrl = (1 << 12) | (0 << 7); 
        pio.write_reg(0, 1); // Enable SM0

        pio.tick(); // Execute PC=0
        assert_eq!(pio.sm[0].pc, 1);
        
        pio.tick(); // Execute PC=1, should wrap
        assert_eq!(pio.sm[0].pc, 0);
    }

    #[test]
    fn test_pio_in_out_full() {
        let mut pio = Pio::new();
        pio.sm[0].x = 0x12345678;
        // IN X, 16 -> 0x4030 (0x4000 | Src=1(0x20) | Count=16(0x10)) = 0x4030
        pio.instruction_mem[0] = 0x4030;
        // PUSH blocking -> 0x8020
        pio.instruction_mem[1] = 0x8020;
        // OUT Y, 32 -> 0x6040
        pio.instruction_mem[2] = 0x6040;

        pio.write_reg(0, 1); // Enable SM0

        pio.tick(); // IN X, 16
        assert_eq!(pio.sm[0].isr, 0x56780000); // 16 LSBs of X (0x5678) shifted into ISR MSB
        assert_eq!(pio.sm[0].isr_count, 16);

        pio.tick(); // PUSH
        assert_eq!(pio.rx_fifo[0][0], 0x56780000);

        pio.write_reg(0x10, 0xDEADBEEF); // TX FIFO
        pio.sm[0].osr = 0xDEADBEEF;
        pio.sm[0].osr_count = 32;
        
        pio.tick(); // OUT Y, 32
        assert_eq!(pio.sm[0].y, 0xDEADBEEF);
        assert_eq!(pio.sm[0].osr_count, 0);
    }
}
