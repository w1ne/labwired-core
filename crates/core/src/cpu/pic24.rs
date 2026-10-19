// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! PIC24 / dsPIC33F CPU — **M0 subset** for the dsPIC33FJ32MC204 class.
//!
//! Scope (M0): reset/boot, the MOV/NOP/BRA instruction subset needed for the
//! UART smoke, CPU-internal UART1 transmit, deterministic stepping. Timers,
//! interrupts (beyond a pending flag), PWM, QEI, ADC, SPI, I2C and the DSP
//! MAC/accumulator unit are **not** modelled yet — unsupported opcodes return
//! [`SimulationError::DecodeError`] rather than guessing.
//!
//! Conventions (provisional until verified against an XC16 ELF):
//! * `pc` is a **word address**; each 24-bit instruction occupies one 4-byte
//!   little-endian slot in flash (`INSTR_SLOT_BYTES`). The mapping of real
//!   XC16 ELF code sections onto this layout is an open item (see
//!   `docs/pic24-m0-reference.md`).
//! * Data space: `0x0000–0x07FF` SFR (UART1 registers modelled, the rest is a
//!   shadow latch), `0x0800–0x0FFF` 2 KB SRAM. Anything else is a
//!   [`SimulationError::MemoryViolation`] — never silently dropped.
//! * Instruction encodings are transcribed from DS70000157G (16-Bit MCU and
//!   DSC Programmer's Reference Manual); see the reference doc for the table.
//!
//! ## Modelled instructions
//!
//! | Instruction | Encoding source |
//! | :--- | :--- |
//! | `NOP` | DS70000157G |
//! | `MOV #lit16, Wnd` | DS70000157G |
//! | `MOV Wns, f` | DS70000157G |
//! | `MOV f, Wnd` | DS70000157G |
//! | `BRA Expr` (slit16) | DS70000157G |
//!
//! UART1 registers (DS70283H): `U1MODE 0x0220`, `U1STA 0x0222`, `U1TXREG 0x0224`.

use crate::snapshot::{CpuSnapshot, Pic24CpuSnapshot};
use crate::{Bus, Cpu, SimResult, SimulationConfig, SimulationError, SimulationObserver};
use std::sync::{Arc, Mutex};

/// Flash size of the dsPIC33FJ32MC204 (DS70283H): 32 KB.
pub const FLASH_BYTES: usize = 32 * 1024;
/// One instruction slot: 24-bit word in a 4-byte little-endian slot (provisional).
pub const INSTR_SLOT_BYTES: usize = 4;
/// Instruction slots available in flash.
pub const INSTR_SLOTS: usize = FLASH_BYTES / INSTR_SLOT_BYTES;
/// Data-space SRAM base (X RAM), DS70283H Figure 4-3.
pub const RAM_BASE: u32 = 0x0800;
/// 2 KB SRAM.
pub const RAM_BYTES: usize = 2 * 1024;
/// SFR space size (0x0000–0x07FF).
pub const SFR_BYTES: usize = 0x0800;

/// UART1 mode register.
pub const SFR_U1MODE: u16 = 0x0220;
/// UART1 status register.
pub const SFR_U1STA: u16 = 0x0222;
/// UART1 transmit register (write a byte here).
pub const SFR_U1TXREG: u16 = 0x0224;

/// U1STA.TRMT — transmit shift register empty (DS70283H U1STA bit 8).
pub const U1STA_TRMT: u16 = 0x0100;
/// U1STA.UTXBF — transmit buffer full (DS70283H U1STA bit 9).
pub const U1STA_UTXBF: u16 = 0x0200;

/// dsPIC33F CPU core (M0 subset).
#[derive(Debug, Clone)]
pub struct Pic24 {
    /// W0–W15. W15 is the stack pointer.
    pub w: [u16; 16],
    /// Program counter — **word address** (see module docs).
    pub pc: u32,
    /// Status register (SR). Flags are not yet evaluated by the M0 subset.
    pub sr: u16,
    /// Retired instruction cycles (1 per single-cycle instruction).
    pub cycles: u64,
    flash: Vec<u8>,
    sfr: Vec<u8>,
    ram: Vec<u8>,
    u1mode: u16,
    u1sta: u16,
    serial_sink: Option<Arc<Mutex<Vec<u8>>>>,
    exception_pending: Option<u32>,
}

impl Default for Pic24 {
    fn default() -> Self {
        Self::new()
    }
}

impl Pic24 {
    pub fn new() -> Self {
        Self {
            w: [0; 16],
            pc: 0,
            sr: 0,
            cycles: 0,
            flash: vec![0xFF; FLASH_BYTES],
            sfr: vec![0; SFR_BYTES],
            ram: vec![0; RAM_BYTES],
            u1mode: 0,
            u1sta: 0,
            serial_sink: None,
            exception_pending: None,
        }
    }

    /// Load instruction words into flash from slot 0 (tests / fixtures).
    ///
    /// Words are 24-bit; the top byte of each slot is ignored.
    pub fn load_words(&mut self, words: &[u32]) {
        assert!(
            words.len() <= INSTR_SLOTS,
            "fixture exceeds flash instruction slots"
        );
        for (i, word) in words.iter().enumerate() {
            let off = i * INSTR_SLOT_BYTES;
            self.flash[off..off + 3].copy_from_slice(&word.to_le_bytes()[..3]);
        }
    }

    /// Load raw instruction-slot bytes at a word address (Harvard flash), the
    /// PIC24 counterpart of `Avr::load_flash` for fixture harnesses.
    pub fn load_flash(&mut self, addr_word: u32, bytes: &[u8]) {
        let off = addr_word as usize * INSTR_SLOT_BYTES;
        assert!(off + bytes.len() <= FLASH_BYTES, "fixture exceeds flash");
        self.flash[off..off + bytes.len()].copy_from_slice(bytes);
    }

    /// Load a program image (engine path). Segments are copied into flash at
    /// their start address; the entry point becomes the reset PC.
    pub fn load_program_image(&mut self, image: &crate::memory::ProgramImage) {
        for seg in &image.segments {
            let start = seg.start_addr as usize;
            let end = start.saturating_add(seg.data.len()).min(FLASH_BYTES);
            if start < FLASH_BYTES {
                self.flash[start..end].copy_from_slice(&seg.data[..end - start]);
            }
        }
        self.pc = image.entry_point as u32;
    }

    /// Attach the console sink that receives UART1 TX bytes (same type the AVR
    /// core and the session builders use).
    pub fn set_serial_sink(&mut self, sink: Arc<Mutex<Vec<u8>>>) {
        self.serial_sink = Some(sink);
    }

    fn fetch(&self) -> SimResult<u32> {
        let off = self.pc as usize * INSTR_SLOT_BYTES;
        if off + INSTR_SLOT_BYTES > FLASH_BYTES {
            return Err(SimulationError::MemoryViolation(off as u64));
        }
        Ok(u32::from_le_bytes([
            self.flash[off],
            self.flash[off + 1],
            self.flash[off + 2],
            0,
        ]))
    }

    /// Data-space read (word). DS70283H Figure 4-3 layout.
    pub fn data_read(&self, addr: u16) -> SimResult<u16> {
        match addr {
            a if a < SFR_BYTES as u16 => match a {
                SFR_U1MODE => Ok(self.u1mode),
                // TRMT reads set once the (instant) shift completes; UTXBF never sticks.
                SFR_U1STA => Ok((self.u1sta | U1STA_TRMT) & !U1STA_UTXBF),
                SFR_U1TXREG => Ok(0),
                _ => {
                    let o = a as usize;
                    Ok(u16::from_le_bytes([self.sfr[o], self.sfr[o + 1]]))
                }
            },
            a if (RAM_BASE as u16..(RAM_BASE as u16 + RAM_BYTES as u16)).contains(&a) => {
                let o = a as usize - RAM_BASE as usize;
                Ok(u16::from_le_bytes([self.ram[o], self.ram[o + 1]]))
            }
            a => Err(SimulationError::MemoryViolation(a as u64)),
        }
    }

    /// Data-space write (word). UART1 TX latches into the console sink.
    pub fn data_write(&mut self, addr: u16, val: u16) -> SimResult<()> {
        match addr {
            a if a < SFR_BYTES as u16 => {
                match a {
                    SFR_U1MODE => self.u1mode = val,
                    SFR_U1STA => self.u1sta = val,
                    SFR_U1TXREG => {
                        if let Some(sink) = &self.serial_sink {
                            if let Ok(mut buf) = sink.lock() {
                                buf.push((val & 0x00FF) as u8);
                            }
                        }
                    }
                    _ => {
                        let o = a as usize;
                        self.sfr[o..o + 2].copy_from_slice(&val.to_le_bytes());
                    }
                }
                Ok(())
            }
            a if (RAM_BASE as u16..(RAM_BASE as u16 + RAM_BYTES as u16)).contains(&a) => {
                let o = a as usize - RAM_BASE as usize;
                self.ram[o..o + 2].copy_from_slice(&val.to_le_bytes());
                Ok(())
            }
            a => Err(SimulationError::MemoryViolation(a as u64)),
        }
    }

    /// Execute one decoded word. Returns the instruction cycles consumed.
    fn execute(&mut self, word: u32) -> SimResult<u32> {
        // NOP — DS70000157G: 0000 0000 xxxx xxxx xxxx xxxx.
        if word & 0x00FFFF == 0 && word >> 16 == 0 {
            self.pc = self.pc.wrapping_add(1);
            return Ok(1);
        }

        // MOV #lit16, Wnd — 0010 kkkk kkkk kkkk kkkk dddd.
        if word & 0x00F0_0000 == 0x0020_0000 {
            let k = ((word >> 4) & 0xFFFF) as u16;
            let d = (word & 0xF) as usize;
            self.w[d] = k;
            self.pc = self.pc.wrapping_add(1);
            return Ok(1);
        }

        // MOV Wns, f — 1000 1fff ffff ffff ffff ssss (f is a 16-bit even address:
        // 15 stored bits, LSB implied 0 — DS70000157G).
        if word >> 19 == 0b1_0001 {
            let f = (((word >> 4) & 0x7FFF) << 1) as u16;
            let s = (word & 0xF) as usize;
            self.data_write(f, self.w[s])?;
            self.pc = self.pc.wrapping_add(1);
            return Ok(1);
        }

        // MOV f, Wnd — 1000 0fff ffff ffff ffff dddd.
        if word >> 19 == 0b1_0000 {
            let f = (((word >> 4) & 0x7FFF) << 1) as u16;
            let d = (word & 0xF) as usize;
            self.w[d] = self.data_read(f)?;
            self.pc = self.pc.wrapping_add(1);
            return Ok(1);
        }

        // BRA Expr — 0011 0111 nnnn nnnn nnnn nnnn (slit16 words: PC = PC + 2 + 2*offset).
        if word >> 16 == 0x0037 {
            let off = (word & 0xFFFF) as u16 as i16;
            let next = self.pc as i64 + 2 + 2 * i64::from(off);
            if next < 0 {
                return Err(SimulationError::DecodeError(self.pc as u64));
            }
            self.pc = next as u32;
            return Ok(2);
        }

        // Everything else is deliberately unimplemented at M0.
        Err(SimulationError::DecodeError(
            (self.pc as u64) * INSTR_SLOT_BYTES as u64,
        ))
    }
}

impl Cpu for Pic24 {
    fn reset(&mut self, _bus: &mut dyn Bus) -> SimResult<()> {
        self.w = [0; 16];
        self.pc = 0;
        self.sr = 0;
        self.cycles = 0;
        self.u1mode = 0;
        self.u1sta = 0;
        self.exception_pending = None;
        Ok(())
    }

    fn step(
        &mut self,
        bus: &mut dyn Bus,
        observers: &[Arc<dyn SimulationObserver>],
        _config: &SimulationConfig,
    ) -> SimResult<()> {
        // The per-instruction trace contract (see `SimulationObserver`): all
        // three notifications, in order, gated on anyone observing.
        let observed = !observers.is_empty();
        let pc = self.pc;
        let word = self.fetch()?;
        if observed {
            for obs in observers {
                obs.on_step_start(pc, word);
            }
        }

        let used = self.execute(word)?;
        self.cycles += u64::from(used);
        crate::advance_batch_cycle(bus, u64::from(used));

        if observed {
            // W0-W15, then the standard trailer: SP (W15), then PC.
            let mut registers = [0u32; 18];
            for (slot, value) in registers.iter_mut().take(16).zip(self.w.iter()) {
                *slot = u32::from(*value);
            }
            registers[16] = u32::from(self.w[15]);
            registers[17] = self.pc;

            crate::emit_trace_event(
                observers,
                labwired_hw_trace::TraceEvent::InstructionRetired { pc, opcode: word },
            );
            for obs in observers {
                obs.on_step_end(used, &registers);
            }
        }
        Ok(())
    }

    fn clock_cycles(&self) -> u64 {
        self.cycles
    }

    /// `BRA` takes two instruction cycles; every modelled single-word
    /// instruction takes one.
    fn max_step_cycles(&self) -> u32 {
        2
    }

    fn set_pc(&mut self, val: u32) {
        self.pc = val;
    }

    fn get_pc(&self) -> u32 {
        self.pc
    }

    fn set_sp(&mut self, val: u32) {
        self.w[15] = val as u16;
    }

    fn set_exception_pending(&mut self, exception_num: u32) {
        self.exception_pending = Some(exception_num);
    }

    fn get_register(&self, id: u8) -> u32 {
        match id {
            0..=15 => u32::from(self.w[id as usize]),
            16 => self.pc,
            17 => u32::from(self.sr),
            _ => 0,
        }
    }

    fn set_register(&mut self, id: u8, val: u32) {
        match id {
            0..=15 => self.w[id as usize] = val as u16,
            16 => self.pc = val,
            17 => self.sr = val as u16,
            _ => {}
        }
    }

    fn get_register_names(&self) -> Vec<String> {
        let mut names: Vec<String> = (0..16).map(|i| format!("W{i}")).collect();
        names.push("PC".to_string());
        names.push("SR".to_string());
        names
    }

    fn index_of_register(&self, name: &str) -> Option<u8> {
        let n = name.trim().to_ascii_uppercase();
        if let Some(rest) = n.strip_prefix('W') {
            if let Ok(i) = rest.parse::<u8>() {
                if i < 16 {
                    return Some(i);
                }
            }
        }
        match n.as_str() {
            "PC" => Some(16),
            "SR" => Some(17),
            _ => None,
        }
    }

    fn snapshot(&self) -> CpuSnapshot {
        CpuSnapshot::Pic24(Pic24CpuSnapshot {
            registers: self.w.to_vec(),
            pc: self.pc,
            sr: self.sr,
            cycles: self.cycles,
            u1mode: self.u1mode,
            u1sta: self.u1sta,
        })
    }

    fn apply_snapshot(&mut self, snapshot: &CpuSnapshot) {
        if let CpuSnapshot::Pic24(s) = snapshot {
            if s.registers.len() == 16 {
                self.w.copy_from_slice(&s.registers);
            }
            self.pc = s.pc;
            self.sr = s.sr;
            self.cycles = s.cycles;
            self.u1mode = s.u1mode;
            self.u1sta = s.u1sta;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bus::SystemBus;

    fn cpu_with(words: &[u32]) -> Pic24 {
        let mut cpu = Pic24::new();
        cpu.load_words(words);
        cpu
    }

    fn mov_lit_w0(val: u16) -> u32 {
        0x0020_0000 | (u32::from(val) << 4)
    }

    fn mov_w0_to_f(f: u16) -> u32 {
        debug_assert!(f % 2 == 0);
        0b1_0001 << 19 | (u32::from(f >> 1) << 4)
    }

    fn step(cpu: &mut Pic24, bus: &mut SystemBus) {
        cpu.step(bus, &[], &SimulationConfig::default()).unwrap();
    }

    #[test]
    fn reset_starts_at_word_zero() {
        let mut cpu = Pic24::new();
        let mut bus = SystemBus::new();
        cpu.reset(&mut bus).unwrap();
        assert_eq!(cpu.get_pc(), 0);
        assert_eq!(cpu.clock_cycles(), 0);
    }

    #[test]
    fn mov_literal_loads_w0() {
        let mut cpu = cpu_with(&[mov_lit_w0(0x48)]);
        let mut bus = SystemBus::new();
        step(&mut cpu, &mut bus);
        assert_eq!(cpu.w[0], 0x48);
        assert_eq!(cpu.get_pc(), 1);
        assert_eq!(cpu.clock_cycles(), 1);
    }

    #[test]
    fn mov_w0_to_u1txreg_reaches_sink() {
        let sink: Arc<Mutex<Vec<u8>>> = Arc::new(Mutex::new(Vec::new()));
        let mut cpu = cpu_with(&[mov_lit_w0(0x48), mov_w0_to_f(SFR_U1TXREG)]);
        cpu.set_serial_sink(sink.clone());
        let mut bus = SystemBus::new();
        step(&mut cpu, &mut bus);
        step(&mut cpu, &mut bus);
        assert_eq!(*sink.lock().unwrap(), vec![b'H']);
        assert_eq!(cpu.get_pc(), 2);
    }

    #[test]
    fn nop_advances_pc() {
        let mut cpu = cpu_with(&[0x0000_0000]);
        let mut bus = SystemBus::new();
        step(&mut cpu, &mut bus);
        assert_eq!(cpu.get_pc(), 1);
    }

    #[test]
    fn bra_minus_one_stays_put() {
        // BRA $-1 → PC = PC + 2 + 2*(-1) = PC.
        let mut cpu = cpu_with(&[0x0037_FFFF]);
        let mut bus = SystemBus::new();
        step(&mut cpu, &mut bus);
        assert_eq!(cpu.get_pc(), 0);
        assert_eq!(cpu.clock_cycles(), 2);
    }

    #[test]
    fn u1sta_read_reports_transmit_empty() {
        let cpu = Pic24::new();
        assert_eq!(cpu.data_read(SFR_U1STA).unwrap() & U1STA_TRMT, U1STA_TRMT);
        assert_eq!(cpu.data_read(SFR_U1STA).unwrap() & U1STA_UTXBF, 0);
    }

    #[test]
    fn ram_round_trip() {
        let mut cpu = Pic24::new();
        cpu.data_write(RAM_BASE as u16, 0xBEEF).unwrap();
        assert_eq!(cpu.data_read(RAM_BASE as u16).unwrap(), 0xBEEF);
    }

    #[test]
    fn unsupported_opcode_is_a_decode_error() {
        let mut cpu = cpu_with(&[0x00FF_FFFF]);
        let mut bus = SystemBus::new();
        let err = cpu.step(&mut bus, &[], &SimulationConfig::default());
        assert!(matches!(err, Err(SimulationError::DecodeError(_))));
    }
}
