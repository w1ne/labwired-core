// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! AVR8 CPU (ATmega328P-class) — Harvard flash + byte data space.
//!
//! Public PC is a **byte** address (ELF/DWARF). Fetch uses word index
//! `pc_byte / 2`. Data space is separate from program flash.

use crate::snapshot::{AvrCpuSnapshot, CpuSnapshot};
use crate::{Bus, Cpu, SimResult, SimulationConfig, SimulationError, SimulationObserver};
use std::sync::Arc;

pub const FLASH_SIZE: usize = 32 * 1024;
pub const RAMEND: u16 = 0x08FF;
pub const SRAM_START: u16 = 0x0100;
pub const AVR_DATA_VMA_BIAS: u64 = 0x0080_0000;
pub const AVR_EEPROM_VMA_BIAS: u64 = 0x0081_0000;

pub fn strip_avr_data_bias(vma: u64) -> Option<u64> {
    if (AVR_EEPROM_VMA_BIAS..AVR_EEPROM_VMA_BIAS + 0x1_0000).contains(&vma) {
        return Some(vma - AVR_EEPROM_VMA_BIAS);
    }
    if (AVR_DATA_VMA_BIAS..AVR_DATA_VMA_BIAS + 0x1_0000).contains(&vma) {
        return Some(vma - AVR_DATA_VMA_BIAS);
    }
    None
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AvrLoadSpace {
    Flash,
    Data,
    Eeprom,
}

pub fn classify_avr_vma(vma: u64) -> (AvrLoadSpace, u64) {
    if let Some(d) = strip_avr_data_bias(vma) {
        if vma >= AVR_EEPROM_VMA_BIAS {
            (AvrLoadSpace::Eeprom, d)
        } else {
            (AvrLoadSpace::Data, d)
        }
    } else {
        (AvrLoadSpace::Flash, vma)
    }
}

#[derive(Debug, Clone)]
pub struct Avr {
    pub r: [u8; 32],
    pub pc: u32,
    pub sp: u16,
    pub sreg: u8,
    pub flash: Vec<u8>,
    pub sram: Vec<u8>,
    pub io: [u8; 0xE0],
    pub pending_irq: u64,
    pub cycles: u64,
    pub tcnt0: u8,
    pub tccr0a: u8,
    pub tccr0b: u8,
    pub timsk0: u8,
    pub tifr0: u8,
    pub ocr0a: u8,
    pub ocr0b: u8,
    pub t0_prescale_acc: u32,
    pub serial_tx: Vec<u8>,
    pub ucsr0a: u8,
    pub ucsr0b: u8,
    pub ucsr0c: u8,
    pub ubrr0: u16,
}

pub const VEC_TIMER0_OVF: u32 = 22;
pub const UCSRA_UDRE: u8 = 1 << 5;
pub const UCSRA_TXC: u8 = 1 << 6;
pub const TIMSK_TOIE0: u8 = 1 << 0;
pub const TIFR_TOV0: u8 = 1 << 0;


impl Default for Avr {
    fn default() -> Self {
        Self::new()
    }
}

impl Avr {
    pub fn new() -> Self {
        Self {
            r: [0; 32],
            pc: 0,
            sp: RAMEND,
            sreg: 0,
            flash: vec![0xFF; FLASH_SIZE],
            sram: vec![0; (RAMEND as usize + 1) - SRAM_START as usize],
            io: [0; 0xE0],
            pending_irq: 0,
            cycles: 0,
            tcnt0: 0, tccr0a: 0, tccr0b: 0, timsk0: 0, tifr0: 0,
            ocr0a: 0, ocr0b: 0, t0_prescale_acc: 0,
            serial_tx: Vec::new(),
            ucsr0a: UCSRA_UDRE, ucsr0b: 0, ucsr0c: 0, ubrr0: 0,
        }
    }

    pub fn load_flash(&mut self, addr: u32, data: &[u8]) {
        let start = addr as usize;
        let end = (start + data.len()).min(self.flash.len());
        if start < end {
            let n = end - start;
            self.flash[start..end].copy_from_slice(&data[..n]);
        }
    }

    pub fn load_words(&mut self, addr: u32, words: &[u16]) {
        for (i, w) in words.iter().enumerate() {
            let b = addr as usize + i * 2;
            if b + 1 < self.flash.len() {
                self.flash[b] = (*w & 0xFF) as u8;
                self.flash[b + 1] = (*w >> 8) as u8;
            }
        }
    }

    #[inline]
    fn flag_i(&self) -> bool {
        self.sreg & 0x80 != 0
    }

    #[inline]
    fn set_flag_i(&mut self, on: bool) {
        if on {
            self.sreg |= 0x80;
        } else {
            self.sreg &= !0x80;
        }
    }

    #[inline]
    fn set_z(&mut self, v: u8) {
        if v == 0 {
            self.sreg |= 0x02;
        } else {
            self.sreg &= !0x02;
        }
    }

    #[inline]
    fn set_n(&mut self, v: u8) {
        if v & 0x80 != 0 {
            self.sreg |= 0x04;
        } else {
            self.sreg &= !0x04;
        }
    }

    #[inline]
    fn set_c(&mut self, on: bool) {
        if on {
            self.sreg |= 0x01;
        } else {
            self.sreg &= !0x01;
        }
    }

    #[inline]
    fn set_v(&mut self, on: bool) {
        if on {
            self.sreg |= 0x08;
        } else {
            self.sreg &= !0x08;
        }
    }

    #[inline]
    fn update_s_from_nv(&mut self) {
        let n = (self.sreg >> 2) & 1;
        let v = (self.sreg >> 3) & 1;
        if n ^ v != 0 {
            self.sreg |= 0x10;
        } else {
            self.sreg &= !0x10;
        }
    }

    fn fetch_word(&self, pc_byte: u32) -> SimResult<u16> {
        if pc_byte % 2 != 0 {
            return Err(SimulationError::DecodeError(pc_byte as u64));
        }
        let i = pc_byte as usize;
        if i + 1 >= self.flash.len() {
            return Err(SimulationError::MemoryViolation(pc_byte as u64));
        }
        Ok(u16::from_le_bytes([self.flash[i], self.flash[i + 1]]))
    }

    fn data_read(&self, addr: u16, _bus: &dyn Bus) -> SimResult<u8> {
        match addr {
            0x0000..=0x001F => Ok(self.r[addr as usize]),
            0x005D => Ok((self.sp & 0xFF) as u8),
            0x005E => Ok((self.sp >> 8) as u8),
            0x005F => Ok(self.sreg),
            0x0035 => Ok(self.tifr0),
            0x0044 => Ok(self.tccr0a),
            0x0045 => Ok(self.tccr0b),
            0x0046 => Ok(self.tcnt0),
            0x0047 => Ok(self.ocr0a),
            0x0048 => Ok(self.ocr0b),
            0x006E => Ok(self.timsk0),
            0x00C0 => Ok(self.ucsr0a | UCSRA_UDRE),
            0x00C1 => Ok(self.ucsr0b),
            0x00C2 => Ok(self.ucsr0c),
            0x00C4 => Ok((self.ubrr0 & 0xFF) as u8),
            0x00C5 => Ok((self.ubrr0 >> 8) as u8),
            0x00C6 => Ok(0),
            0x0020..=0x00FF => Ok(self.io[(addr - 0x20) as usize]),
            a if a >= SRAM_START && a <= RAMEND => Ok(self.sram[(a - SRAM_START) as usize]),
            _ => Err(SimulationError::MemoryViolation(addr as u64)),
        }
    }

    fn data_write(&mut self, addr: u16, value: u8, bus: &mut dyn Bus) -> SimResult<()> {
        match addr {
            0x0000..=0x001F => { self.r[addr as usize] = value; Ok(()) }
            0x005D => { self.sp = (self.sp & 0xFF00) | value as u16; Ok(()) }
            0x005E => { self.sp = (self.sp & 0x00FF) | ((value as u16) << 8); Ok(()) }
            0x005F => { self.sreg = value; Ok(()) }
            0x0035 => { self.tifr0 &= !value; Ok(()) }
            0x0044 => { self.tccr0a = value; Ok(()) }
            0x0045 => { self.tccr0b = value; Ok(()) }
            0x0046 => { self.tcnt0 = value; Ok(()) }
            0x0047 => { self.ocr0a = value; Ok(()) }
            0x0048 => { self.ocr0b = value; Ok(()) }
            0x006E => { self.timsk0 = value; Ok(()) }
            0x00C0 => {
                if value & UCSRA_TXC != 0 { self.ucsr0a &= !UCSRA_TXC; }
                self.ucsr0a |= UCSRA_UDRE;
                Ok(())
            }
            0x00C1 => { self.ucsr0b = value; Ok(()) }
            0x00C2 => { self.ucsr0c = value; Ok(()) }
            0x00C4 => { self.ubrr0 = (self.ubrr0 & 0xFF00) | value as u16; Ok(()) }
            0x00C5 => { self.ubrr0 = (self.ubrr0 & 0x00FF) | ((value as u16) << 8); Ok(()) }
            0x00C6 => {
                self.serial_tx.push(value);
                self.ucsr0a |= UCSRA_UDRE | UCSRA_TXC;
                let _ = bus.write_u8(addr as u64, value);
                Ok(())
            }
            0x0020..=0x00FF => {
                self.io[(addr - 0x20) as usize] = value;
                let _ = bus.write_u8(addr as u64, value);
                Ok(())
            }
            a if a >= SRAM_START && a <= RAMEND => {
                self.sram[(a - SRAM_START) as usize] = value;
                let _ = bus.write_u8(a as u64, value);
                Ok(())
            }
            _ => Err(SimulationError::MemoryViolation(addr as u64)),
        }
    }

    fn t0_prescaler(&self) -> u32 {
        match self.tccr0b & 0x07 {
            0 => 0, 1 => 1, 2 => 8, 3 => 64, 4 => 256, 5 => 1024, _ => 0,
        }
    }

    pub fn tick_timer0(&mut self, cpu_cycles: u32) {
        let div = self.t0_prescaler();
        if div == 0 || cpu_cycles == 0 { return; }
        self.t0_prescale_acc = self.t0_prescale_acc.saturating_add(cpu_cycles);
        while self.t0_prescale_acc >= div {
            self.t0_prescale_acc -= div;
            let (next, overflowed) = self.tcnt0.overflowing_add(1);
            self.tcnt0 = next;
            if overflowed {
                self.tifr0 |= TIFR_TOV0;
                if self.timsk0 & TIMSK_TOIE0 != 0 {
                    self.pending_irq |= 1u64 << VEC_TIMER0_OVF;
                }
            }
        }
    }

    pub fn portb(&self) -> u8 {
        self.io[(0x25 - 0x20) as usize]
    }

    pub fn serial_as_str(&self) -> String {
        String::from_utf8_lossy(&self.serial_tx).into_owned()
    }

    fn push_byte(&mut self, value: u8, bus: &mut dyn Bus) -> SimResult<()> {
        self.data_write(self.sp, value, bus)?;
        self.sp = self.sp.wrapping_sub(1);
        Ok(())
    }

    fn pop_byte(&mut self, bus: &dyn Bus) -> SimResult<u8> {
        self.sp = self.sp.wrapping_add(1);
        self.data_read(self.sp, bus)
    }

    fn push_pc(&mut self, bus: &mut dyn Bus) -> SimResult<()> {
        let word = (self.pc / 2) as u16;
        self.push_byte((word >> 8) as u8, bus)?;
        self.push_byte((word & 0xFF) as u8, bus)?;
        Ok(())
    }

    fn pop_pc(&mut self, bus: &dyn Bus) -> SimResult<()> {
        let lo = self.pop_byte(bus)? as u16;
        let hi = self.pop_byte(bus)? as u16;
        let word = (hi << 8) | lo;
        self.pc = (word as u32) * 2;
        Ok(())
    }

    fn word_size_bytes(op: u16) -> u32 {
        let top = op & 0xFE0E;
        if top == 0x940C || top == 0x940E {
            return 4;
        }
        if (op & 0xFE0F) == 0x9000 || (op & 0xFE0F) == 0x9200 {
            return 4;
        }
        2
    }

    fn try_take_irq(&mut self, bus: &mut dyn Bus) -> SimResult<bool> {
        if !self.flag_i() || self.pending_irq == 0 {
            return Ok(false);
        }
        let vec = self.pending_irq.trailing_zeros();
        if vec == 0 || vec > 31 {
            return Ok(false);
        }
        self.pending_irq &= !(1u64 << vec);
        self.set_flag_i(false);
        self.push_pc(bus)?;
        self.pc = vec.saturating_sub(1) * 4;
        Ok(true)
    }

    fn step_inner(
        &mut self,
        bus: &mut dyn Bus,
        _observers: &[Arc<dyn SimulationObserver>],
        _config: &SimulationConfig,
    ) -> SimResult<()> {
        if self.try_take_irq(bus)? {
            self.cycles += 4;
            return Ok(());
        }

        let pc = self.pc;
        let op = self.fetch_word(pc)?;
        let mut next = pc.wrapping_add(2);

        if op == 0x0000 {
            self.pc = next;
            self.cycles += 1;
            return Ok(());
        }
        if op == 0x9478 {
            self.set_flag_i(true);
            self.pc = next;
            self.cycles += 1;
            return Ok(());
        }
        if op == 0x94F8 {
            self.set_flag_i(false);
            self.pc = next;
            self.cycles += 1;
            return Ok(());
        }
        if op == 0x9508 {
            self.pop_pc(bus)?;
            self.cycles += 4;
            return Ok(());
        }
        if op == 0x9518 {
            self.pop_pc(bus)?;
            self.set_flag_i(true);
            self.cycles += 4;
            return Ok(());
        }
        if op == 0x9588 {
            self.pc = next;
            self.cycles += 1;
            return Ok(());
        }
        if op == 0x9598 {
            return Err(SimulationError::Halt);
        }

        // RJMP
        if (op & 0xF000) == 0xC000 {
            let k = op & 0x0FFF;
            let offset = if k & 0x0800 != 0 {
                (k | 0xF000) as i16
            } else {
                k as i16
            };
            let pc_word = (pc / 2) as i32 + 1 + offset as i32;
            self.pc = (pc_word as u32) * 2;
            self.cycles += 2;
            return Ok(());
        }

        // RCALL
        if (op & 0xF000) == 0xD000 {
            let k = op & 0x0FFF;
            let offset = if k & 0x0800 != 0 {
                (k | 0xF000) as i16
            } else {
                k as i16
            };
            self.pc = next;
            self.push_pc(bus)?;
            let pc_word = (pc / 2) as i32 + 1 + offset as i32;
            self.pc = (pc_word as u32) * 2;
            self.cycles += 3;
            return Ok(());
        }

        // LDI
        if (op & 0xF000) == 0xE000 {
            let rd = 16 + ((op >> 4) & 0x0F) as usize;
            let k = ((op & 0x0F00) >> 4) as u8 | (op & 0x0F) as u8;
            self.r[rd] = k;
            self.pc = next;
            self.cycles += 1;
            return Ok(());
        }

        // OUT
        if (op & 0xF800) == 0xB800 {
            let a = (((op >> 5) & 0x30) | (op & 0x0F)) as u8;
            let rr = ((op >> 4) & 0x1F) as usize;
            let data_addr = 0x20u16 + a as u16;
            self.data_write(data_addr, self.r[rr], bus)?;
            self.pc = next;
            self.cycles += 1;
            return Ok(());
        }

        // IN
        if (op & 0xF800) == 0xB000 {
            let a = (((op >> 5) & 0x30) | (op & 0x0F)) as u8;
            let rd = ((op >> 4) & 0x1F) as usize;
            let data_addr = 0x20u16 + a as u16;
            self.r[rd] = self.data_read(data_addr, bus)?;
            self.pc = next;
            self.cycles += 1;
            return Ok(());
        }

        // SBI
        if (op & 0xFF00) == 0x9A00 {
            let a = ((op >> 3) & 0x1F) as u8;
            let b = (op & 0x07) as u8;
            let data_addr = 0x20u16 + a as u16;
            let v = self.data_read(data_addr, bus)? | (1 << b);
            self.data_write(data_addr, v, bus)?;
            self.pc = next;
            self.cycles += 2;
            return Ok(());
        }

        // CBI
        if (op & 0xFF00) == 0x9800 {
            let a = ((op >> 3) & 0x1F) as u8;
            let b = (op & 0x07) as u8;
            let data_addr = 0x20u16 + a as u16;
            let v = self.data_read(data_addr, bus)? & !(1 << b);
            self.data_write(data_addr, v, bus)?;
            self.pc = next;
            self.cycles += 2;
            return Ok(());
        }

        // LDS
        if (op & 0xFE0F) == 0x9000 {
            let rd = ((op >> 4) & 0x1F) as usize;
            let k = self.fetch_word(next)?;
            next = next.wrapping_add(2);
            self.r[rd] = self.data_read(k, bus)?;
            self.pc = next;
            self.cycles += 2;
            return Ok(());
        }

        // STS
        if (op & 0xFE0F) == 0x9200 {
            let rr = ((op >> 4) & 0x1F) as usize;
            let k = self.fetch_word(next)?;
            next = next.wrapping_add(2);
            self.data_write(k, self.r[rr], bus)?;
            self.pc = next;
            self.cycles += 2;
            return Ok(());
        }

        // MOV
        if (op & 0xFC00) == 0x2C00 {
            let rd = ((op >> 4) & 0x1F) as usize;
            let rr = (((op >> 5) & 0x10) | (op & 0x0F)) as usize;
            self.r[rd] = self.r[rr];
            self.pc = next;
            self.cycles += 1;
            return Ok(());
        }

        // ADD
        if (op & 0xFC00) == 0x0C00 {
            let rd = ((op >> 4) & 0x1F) as usize;
            let rr = (((op >> 5) & 0x10) | (op & 0x0F)) as usize;
            let a = self.r[rd];
            let b = self.r[rr];
            let (res, c) = a.overflowing_add(b);
            let v = (!(a ^ b) & (a ^ res)) & 0x80 != 0;
            self.r[rd] = res;
            self.set_c(c);
            self.set_z(res);
            self.set_n(res);
            self.set_v(v);
            self.update_s_from_nv();
            self.pc = next;
            self.cycles += 1;
            return Ok(());
        }

        // EOR
        if (op & 0xFC00) == 0x2400 {
            let rd = ((op >> 4) & 0x1F) as usize;
            let rr = (((op >> 5) & 0x10) | (op & 0x0F)) as usize;
            let res = self.r[rd] ^ self.r[rr];
            self.r[rd] = res;
            self.set_v(false);
            self.set_z(res);
            self.set_n(res);
            self.update_s_from_nv();
            self.pc = next;
            self.cycles += 1;
            return Ok(());
        }

        // AND
        if (op & 0xFC00) == 0x2000 {
            let rd = ((op >> 4) & 0x1F) as usize;
            let rr = (((op >> 5) & 0x10) | (op & 0x0F)) as usize;
            let res = self.r[rd] & self.r[rr];
            self.r[rd] = res;
            self.set_v(false);
            self.set_z(res);
            self.set_n(res);
            self.update_s_from_nv();
            self.pc = next;
            self.cycles += 1;
            return Ok(());
        }

        // OR
        if (op & 0xFC00) == 0x2800 {
            let rd = ((op >> 4) & 0x1F) as usize;
            let rr = (((op >> 5) & 0x10) | (op & 0x0F)) as usize;
            let res = self.r[rd] | self.r[rr];
            self.r[rd] = res;
            self.set_v(false);
            self.set_z(res);
            self.set_n(res);
            self.update_s_from_nv();
            self.pc = next;
            self.cycles += 1;
            return Ok(());
        }

        // CP
        if (op & 0xFC00) == 0x1400 {
            let rd = ((op >> 4) & 0x1F) as usize;
            let rr = (((op >> 5) & 0x10) | (op & 0x0F)) as usize;
            let a = self.r[rd];
            let b = self.r[rr];
            let (res, c) = a.overflowing_sub(b);
            let v = ((a ^ b) & (a ^ res)) & 0x80 != 0;
            self.set_c(c);
            self.set_z(res);
            self.set_n(res);
            self.set_v(v);
            self.update_s_from_nv();
            self.pc = next;
            self.cycles += 1;
            return Ok(());
        }

        // CPI
        if (op & 0xF000) == 0x3000 {
            let rd = 16 + ((op >> 4) & 0x0F) as usize;
            let k = ((op & 0x0F00) >> 4) as u8 | (op & 0x0F) as u8;
            let a = self.r[rd];
            let (res, c) = a.overflowing_sub(k);
            let v = ((a ^ k) & (a ^ res)) & 0x80 != 0;
            self.set_c(c);
            self.set_z(res);
            self.set_n(res);
            self.set_v(v);
            self.update_s_from_nv();
            self.pc = next;
            self.cycles += 1;
            return Ok(());
        }

        // BRcc
        if (op & 0xF800) == 0xF000 || (op & 0xF800) == 0xF400 {
            let k = ((op >> 3) & 0x7F) as i8;
            let offset = if k & 0x40 != 0 { k | !0x7F } else { k };
            let bit = (op & 0x07) as u8;
            let complement = (op & 0x0400) != 0;
            let flag = (self.sreg >> bit) & 1 != 0;
            let take = if complement { !flag } else { flag };
            if take {
                let pc_word = (pc / 2) as i32 + 1 + offset as i32;
                self.pc = (pc_word as u32) * 2;
                self.cycles += 2;
            } else {
                self.pc = next;
                self.cycles += 1;
            }
            return Ok(());
        }

        // PUSH
        if (op & 0xFE0F) == 0x920F {
            let rr = ((op >> 4) & 0x1F) as usize;
            self.push_byte(self.r[rr], bus)?;
            self.pc = next;
            self.cycles += 2;
            return Ok(());
        }

        // POP
        if (op & 0xFE0F) == 0x900F {
            let rd = ((op >> 4) & 0x1F) as usize;
            self.r[rd] = self.pop_byte(bus)?;
            self.pc = next;
            self.cycles += 2;
            return Ok(());
        }

        // ADIW
        if (op & 0xFF00) == 0x9600 {
            let d = ((op >> 4) & 0x03) as usize;
            let rd = 24 + d * 2;
            let k = (((op >> 6) & 0x03) << 4) as u16 | (op & 0x0F) as u16;
            let val = u16::from_le_bytes([self.r[rd], self.r[rd + 1]]).wrapping_add(k);
            self.r[rd] = (val & 0xFF) as u8;
            self.r[rd + 1] = (val >> 8) as u8;
            self.set_z(if val == 0 { 0 } else { 1 });
            self.set_n((val >> 8) as u8);
            self.pc = next;
            self.cycles += 2;
            return Ok(());
        }

        // SBIW
        if (op & 0xFF00) == 0x9700 {
            let d = ((op >> 4) & 0x03) as usize;
            let rd = 24 + d * 2;
            let k = (((op >> 6) & 0x03) << 4) as u16 | (op & 0x0F) as u16;
            let val = u16::from_le_bytes([self.r[rd], self.r[rd + 1]]).wrapping_sub(k);
            self.r[rd] = (val & 0xFF) as u8;
            self.r[rd + 1] = (val >> 8) as u8;
            self.set_z(if val == 0 { 0 } else { 1 });
            self.set_n((val >> 8) as u8);
            self.pc = next;
            self.cycles += 2;
            return Ok(());
        }

        // MOVW
        if (op & 0xFF00) == 0x0100 {
            let rd = ((op >> 4) & 0x0F) as usize * 2;
            let rr = (op & 0x0F) as usize * 2;
            self.r[rd] = self.r[rr];
            self.r[rd + 1] = self.r[rr + 1];
            self.pc = next;
            self.cycles += 1;
            return Ok(());
        }

        // IJMP
        if op == 0x9409 {
            let z = u16::from_le_bytes([self.r[30], self.r[31]]);
            self.pc = (z as u32) * 2;
            self.cycles += 2;
            return Ok(());
        }

        // LPM
        if op == 0x95C8 {
            let z = u16::from_le_bytes([self.r[30], self.r[31]]) as usize;
            self.r[0] = self.flash.get(z).copied().unwrap_or(0xFF);
            self.pc = next;
            self.cycles += 3;
            return Ok(());
        }

        // LPM Rd,Z
        if (op & 0xFE0F) == 0x9004 {
            let rd = ((op >> 4) & 0x1F) as usize;
            let z = u16::from_le_bytes([self.r[30], self.r[31]]) as usize;
            self.r[rd] = self.flash.get(z).copied().unwrap_or(0xFF);
            self.pc = next;
            self.cycles += 3;
            return Ok(());
        }

        // LPM Rd,Z+
        if (op & 0xFE0F) == 0x9005 {
            let rd = ((op >> 4) & 0x1F) as usize;
            let z = u16::from_le_bytes([self.r[30], self.r[31]]);
            self.r[rd] = self.flash.get(z as usize).copied().unwrap_or(0xFF);
            let z2 = z.wrapping_add(1);
            self.r[30] = (z2 & 0xFF) as u8;
            self.r[31] = (z2 >> 8) as u8;
            self.pc = next;
            self.cycles += 3;
            return Ok(());
        }

        // LD Rd,X
        if (op & 0xFE0F) == 0x900C {
            let rd = ((op >> 4) & 0x1F) as usize;
            let x = u16::from_le_bytes([self.r[26], self.r[27]]);
            self.r[rd] = self.data_read(x, bus)?;
            self.pc = next;
            self.cycles += 2;
            return Ok(());
        }

        // ST X,Rr
        if (op & 0xFE0F) == 0x920C {
            let rr = ((op >> 4) & 0x1F) as usize;
            let x = u16::from_le_bytes([self.r[26], self.r[27]]);
            self.data_write(x, self.r[rr], bus)?;
            self.pc = next;
            self.cycles += 2;
            return Ok(());
        }

        // JMP
        if (op & 0xFE0E) == 0x940C {
            let k_hi = ((op >> 3) & 0x3E) | (op & 0x01);
            let k_lo = self.fetch_word(next)?;
            let k = ((k_hi as u32) << 16) | k_lo as u32;
            self.pc = k * 2;
            self.cycles += 3;
            return Ok(());
        }

        // CALL
        if (op & 0xFE0E) == 0x940E {
            let k_hi = ((op >> 3) & 0x3E) | (op & 0x01);
            let k_lo = self.fetch_word(next)?;
            let k = ((k_hi as u32) << 16) | k_lo as u32;
            self.pc = next.wrapping_add(2);
            self.push_pc(bus)?;
            self.pc = k * 2;
            self.cycles += 4;
            return Ok(());
        }

        // SBIS
        if (op & 0xFF00) == 0x9B00 {
            let a = ((op >> 3) & 0x1F) as u8;
            let b = (op & 0x07) as u8;
            let v = self.data_read(0x20 + a as u16, bus)?;
            if v & (1 << b) != 0 {
                let following = self.fetch_word(next)?;
                next = next.wrapping_add(Self::word_size_bytes(following));
            }
            self.pc = next;
            self.cycles += 1;
            return Ok(());
        }

        // SBIC
        if (op & 0xFF00) == 0x9900 {
            let a = ((op >> 3) & 0x1F) as u8;
            let b = (op & 0x07) as u8;
            let v = self.data_read(0x20 + a as u16, bus)?;
            if v & (1 << b) == 0 {
                let following = self.fetch_word(next)?;
                next = next.wrapping_add(Self::word_size_bytes(following));
            }
            self.pc = next;
            self.cycles += 1;
            return Ok(());
        }

        // SBRS
        if (op & 0xFE08) == 0xFE00 {
            let rr = ((op >> 4) & 0x1F) as usize;
            let b = (op & 0x07) as u8;
            if self.r[rr] & (1 << b) != 0 {
                let following = self.fetch_word(next)?;
                next = next.wrapping_add(Self::word_size_bytes(following));
            }
            self.pc = next;
            self.cycles += 1;
            return Ok(());
        }

        // SBRC
        if (op & 0xFE08) == 0xFC00 {
            let rr = ((op >> 4) & 0x1F) as usize;
            let b = (op & 0x07) as u8;
            if self.r[rr] & (1 << b) == 0 {
                let following = self.fetch_word(next)?;
                next = next.wrapping_add(Self::word_size_bytes(following));
            }
            self.pc = next;
            self.cycles += 1;
            return Ok(());
        }

        // CPSE
        if (op & 0xFC00) == 0x1000 {
            let rd = ((op >> 4) & 0x1F) as usize;
            let rr = (((op >> 5) & 0x10) | (op & 0x0F)) as usize;
            if self.r[rd] == self.r[rr] {
                let following = self.fetch_word(next)?;
                next = next.wrapping_add(Self::word_size_bytes(following));
            }
            self.pc = next;
            self.cycles += 1;
            return Ok(());
        }

        // INC
        if (op & 0xFE0F) == 0x9403 {
            let rd = ((op >> 4) & 0x1F) as usize;
            let res = self.r[rd].wrapping_add(1);
            self.r[rd] = res;
            self.set_v(res == 0x80);
            self.set_z(res);
            self.set_n(res);
            self.update_s_from_nv();
            self.pc = next;
            self.cycles += 1;
            return Ok(());
        }

        // DEC
        if (op & 0xFE0F) == 0x940A {
            let rd = ((op >> 4) & 0x1F) as usize;
            let res = self.r[rd].wrapping_sub(1);
            self.r[rd] = res;
            self.set_v(res == 0x7F);
            self.set_z(res);
            self.set_n(res);
            self.update_s_from_nv();
            self.pc = next;
            self.cycles += 1;
            return Ok(());
        }

        Err(SimulationError::DecodeError(pc as u64))
    }
}

impl Cpu for Avr {
    fn reset(&mut self, _bus: &mut dyn Bus) -> SimResult<()> {
        self.r = [0; 32];
        self.pc = 0;
        self.sp = RAMEND;
        self.sreg = 0;
        self.pending_irq = 0;
        self.cycles = 0;
        self.tcnt0 = 0; self.tccr0a = 0; self.tccr0b = 0;
        self.timsk0 = 0; self.tifr0 = 0; self.t0_prescale_acc = 0;
        self.serial_tx.clear();
        self.ucsr0a = UCSRA_UDRE; self.ucsr0b = 0; self.ucsr0c = 0; self.ubrr0 = 0;
        Ok(())
    }

    fn step(
        &mut self,
        bus: &mut dyn Bus,
        observers: &[Arc<dyn SimulationObserver>],
        config: &SimulationConfig,
    ) -> SimResult<()> {
        let before = self.cycles;
        self.step_inner(bus, observers, config)?;
        let delta = self.cycles.saturating_sub(before) as u32;
        self.tick_timer0(delta.max(1));
        Ok(())
    }

    fn set_pc(&mut self, val: u32) {
        self.pc = val & !1;
    }
    fn get_pc(&self) -> u32 {
        self.pc
    }
    fn set_sp(&mut self, val: u32) {
        self.sp = val as u16;
    }
    fn set_exception_pending(&mut self, exception_num: u32) {
        if exception_num > 0 && exception_num < 64 {
            self.pending_irq |= 1u64 << exception_num;
        }
    }

    fn get_register(&self, id: u8) -> u32 {
        match id {
            0..=31 => self.r[id as usize] as u32,
            32 => self.sp as u32,
            33 => self.sreg as u32,
            34 => self.pc,
            _ => 0,
        }
    }

    fn set_register(&mut self, id: u8, val: u32) {
        match id {
            0..=31 => self.r[id as usize] = val as u8,
            32 => self.sp = val as u16,
            33 => self.sreg = val as u8,
            34 => self.pc = val & !1,
            _ => {}
        }
    }

    fn snapshot(&self) -> CpuSnapshot {
        CpuSnapshot::Avr(AvrCpuSnapshot {
            registers: self.r.to_vec(),
            pc: self.pc,
            sp: self.sp,
            sreg: self.sreg,
        })
    }

    fn apply_snapshot(&mut self, snapshot: &CpuSnapshot) {
        if let CpuSnapshot::Avr(s) = snapshot {
            for (i, v) in s.registers.iter().take(32).enumerate() {
                self.r[i] = *v;
            }
            self.pc = s.pc & !1;
            self.sp = s.sp;
            self.sreg = s.sreg;
        }
    }

    fn get_register_names(&self) -> Vec<String> {
        let mut names: Vec<String> = (0..32).map(|i| format!("R{i}")).collect();
        names.push("SP".into());
        names.push("SREG".into());
        names.push("PC".into());
        names
    }

    fn index_of_register(&self, name: &str) -> Option<u8> {
        let u = name.to_uppercase();
        if let Some(rest) = u.strip_prefix('R') {
            if let Ok(n) = rest.parse::<u8>() {
                if n < 32 {
                    return Some(n);
                }
            }
        }
        match u.as_str() {
            "SP" => Some(32),
            "SREG" => Some(33),
            "PC" => Some(34),
            "X" => Some(26),
            "Y" => Some(28),
            "Z" => Some(30),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{DmaRequest, SimulationConfig};
    use std::collections::HashMap;

    struct MockBus {
        mem: HashMap<u64, u8>,
        config: SimulationConfig,
    }

    impl MockBus {
        fn new() -> Self {
            Self {
                mem: HashMap::new(),
                config: SimulationConfig::default(),
            }
        }
    }

    impl Bus for MockBus {
        fn read_u8(&self, addr: u64) -> SimResult<u8> {
            Ok(*self.mem.get(&addr).unwrap_or(&0))
        }
        fn write_u8(&mut self, addr: u64, value: u8) -> SimResult<()> {
            self.mem.insert(addr, value);
            Ok(())
        }
        fn tick_peripherals(&mut self) -> Vec<u32> {
            Vec::new()
        }
        fn execute_dma(&mut self, _requests: &[DmaRequest]) -> SimResult<()> {
            Ok(())
        }
        fn config(&self) -> &SimulationConfig {
            &self.config
        }
    }

    #[test]
    fn rjmp_self_retires_10000_steps() {
        let mut cpu = Avr::new();
        cpu.load_words(0, &[0xCFFF]);
        cpu.set_pc(0);
        let mut bus = MockBus::new();
        let cfg = SimulationConfig::default();
        for _ in 0..10_000 {
            cpu.step(&mut bus, &[], &cfg).unwrap();
        }
        assert_eq!(cpu.get_pc(), 0);
    }

    #[test]
    fn pc_rejects_odd_fetch() {
        let mut cpu = Avr::new();
        cpu.load_words(0, &[0x0000]);
        cpu.pc = 1;
        let mut bus = MockBus::new();
        let err = cpu
            .step(&mut bus, &[], &SimulationConfig::default())
            .unwrap_err();
        assert!(matches!(err, SimulationError::DecodeError(1)));
    }

    #[test]
    fn in_out_and_lds_alias_same_io_register() {
        let mut cpu = Avr::new();
        // LDI R16,0xA5; OUT PORTB(io5),R16; LDS R17,0x25
        cpu.load_words(0, &[0xEA05, 0xB905, 0x9110, 0x0025, 0xCFFF]);
        let mut bus = MockBus::new();
        let cfg = SimulationConfig::default();
        cpu.set_pc(0);
        cpu.step(&mut bus, &[], &cfg).unwrap();
        assert_eq!(cpu.r[16], 0xA5);
        cpu.step(&mut bus, &[], &cfg).unwrap();
        assert_eq!(cpu.io[(0x25 - 0x20) as usize], 0xA5);
        cpu.step(&mut bus, &[], &cfg).unwrap();
        assert_eq!(cpu.r[17], 0xA5);
    }

    #[test]
    fn data_bias_strip() {
        assert_eq!(strip_avr_data_bias(0x0080_0100), Some(0x100));
        assert_eq!(strip_avr_data_bias(0x0081_0010), Some(0x10));
        assert_eq!(strip_avr_data_bias(0x0000_0100), None);
        assert_eq!(classify_avr_vma(0x0080_0200), (AvrLoadSpace::Data, 0x200));
        assert_eq!(classify_avr_vma(0x0000_0040), (AvrLoadSpace::Flash, 0x40));
    }

    #[test]
    fn sbi_sets_portb_bit() {
        let mut cpu = Avr::new();
        cpu.load_words(0, &[0x9A2D, 0xCFFF]);
        let mut bus = MockBus::new();
        cpu.step(&mut bus, &[], &SimulationConfig::default())
            .unwrap();
        assert_eq!(cpu.io[0x05], 1 << 5);
    }

    #[test]
    fn unknown_opcode_decode_error() {
        let mut cpu = Avr::new();
        cpu.load_words(0, &[0xFFFF]);
        let mut bus = MockBus::new();
        let err = cpu
            .step(&mut bus, &[], &SimulationConfig::default())
            .unwrap_err();
        assert!(matches!(err, SimulationError::DecodeError(0)));
    }
    #[test]
    fn timer0_overflow_pends_with_arduino_prescale() {
        let mut cpu = Avr::new();
        cpu.tccr0a = 0x03; cpu.tccr0b = 0x03; cpu.timsk0 = TIMSK_TOIE0;
        cpu.tick_timer0(16384);
        assert!(cpu.pending_irq & (1 << VEC_TIMER0_OVF) != 0);
    }

    #[test]
    fn usart_udre_tx_never_blocks() {
        let mut cpu = Avr::new();
        let mut bus = MockBus::new();
        for b in b"Hi" {
            cpu.data_write(0xC6, *b, &mut bus).unwrap();
            assert_ne!(cpu.data_read(0xC0, &bus).unwrap() & UCSRA_UDRE, 0);
        }
        assert_eq!(cpu.serial_tx, b"Hi");
    }

    #[test]
    fn hand_blink_toggles_portb5_and_serial() {
        let mut cpu = Avr::new();
        cpu.load_words(0, &[0x9A2D, 0x982D, 0xE508, 0x9300, 0x00C6, 0xCFFA]);
        let mut bus = MockBus::new();
        let cfg = SimulationConfig::default();
        let mut hi = false; let mut lo = false;
        for _ in 0..200 {
            cpu.step(&mut bus, &[], &cfg).unwrap();
            if cpu.portb() & 0x20 != 0 { hi = true; } else { lo = true; }
            if hi && lo && cpu.serial_tx.contains(&b'X') { break; }
        }
        assert!(hi && lo);
        assert!(cpu.serial_tx.contains(&b'X'));
    }

}
