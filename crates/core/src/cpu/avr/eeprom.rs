// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! The data EEPROM of the classic ATmega parts: what Arduino `EEPROM.h`
//! (`EEPROM.put/get/update`) and avr-libc `eeprom_*_byte` run on.
//!
//! Registers (data-space addresses, ATmega328P datasheet DS40002061B
//! section 8.6; the same addresses and bits on the ATmega1281/2560):
//!
//! | Register | Addr | Kept |
//! |----------|------|------|
//! | `EECR`   | 0x3F | EEPM1:0, EERIE, EEMPE, EEPE, EERE |
//! | `EEDR`   | 0x40 | data byte |
//! | `EEARL`  | 0x41 | address low |
//! | `EEARH`  | 0x42 | address high (masked to the array size) |
//!
//! `EEMPE` reads 1 for four clocks after it is written to one, then the
//! hardware clears it. `EEPE` written to one while `EEMPE` is still set
//! starts the operation `EEPM` selects (erase+write, erase only, write only)
//! and reads 1 until it completes: 3.4 ms for erase+write, 1.8 ms for the
//! other two (Table 8-2), timed from the chip's `cpu_hz`. A write of `EEPE`
//! without `EEMPE` is ignored, as on silicon. `EERE` copies the byte at
//! `EEAR` to `EEDR` and halts the CPU four clocks; it is ignored while a
//! write is in progress. `EEPM` only changes while `EEPE` is clear.
//!
//! `EE_READY` is a level interrupt: pending while `EERIE` is set and no
//! write is in progress.
//!
//! The array is non-volatile: a reset keeps it. An ELF `.eeprom` section
//! (avr-gcc `EEMEM`, VMA 0x81_0000) is loaded into it; the rest is erased
//! (0xFF), like a part fresh from the factory.
//!
//! Not modelled: the classic tinyAVR EEPROM (different addresses), the
//! SPM-in-progress interlock, and the 2 halted clocks after `EEPE`.

use super::{Avr, AvrModel};

/// `EE_READY_vect` (`__vector_22`, byte 0x58), 1-based like the rest.
pub const VEC_EE_READY: u32 = 23;

pub(super) const ADDR_EECR: u16 = 0x3F;
pub(super) const ADDR_EEDR: u16 = 0x40;
pub(super) const ADDR_EEARL: u16 = 0x41;
pub(super) const ADDR_EEARH: u16 = 0x42;

const EERE: u8 = 1 << 0;
const EEPE: u8 = 1 << 1;
const EEMPE: u8 = 1 << 2;
const EERIE: u8 = 1 << 3;
const EEPM_MASK: u8 = 0x30;

/// Clocks `EEMPE` stays set after software writes it.
const EEMPE_WINDOW: u64 = 4;
/// Clocks the CPU is halted by an `EERE` read.
const EERE_HALT: u64 = 4;
/// Erase+write time, Table 8-2 (typical).
const ERASE_WRITE_US: u64 = 3_400;
/// Erase-only and write-only time, Table 8-2 (typical).
const SPLIT_OP_US: u64 = 1_800;

/// The default clock of the boards these parts are modelled for.
const DEFAULT_CPU_HZ: u64 = 16_000_000;

impl AvrModel {
    /// Data EEPROM bytes, for the parts whose EEPROM this module models.
    pub fn eeprom_size(self) -> usize {
        match self {
            AvrModel::Atmega328p | AvrModel::Atmega328pb => 1024,
            AvrModel::Atmega1281 | AvrModel::Atmega2560 => 4096,
            _ => 0,
        }
    }
}

#[derive(Debug)]
pub struct Eeprom {
    pub data: Vec<u8>,
    eedr: u8,
    eear: u16,
    /// EEPM1:0 and EERIE as last written.
    eecr: u8,
    /// `EEMPE` reads 1 while `cycles < eempe_until`.
    eempe_until: u64,
    /// `EEPE` reads 1 while `cycles < busy_until`.
    busy_until: u64,
    cpu_hz: u64,
}

impl Eeprom {
    pub fn new(size: usize) -> Self {
        Self {
            data: vec![0xFF; size],
            eedr: 0,
            eear: 0,
            eecr: 0,
            eempe_until: 0,
            busy_until: 0,
            cpu_hz: DEFAULT_CPU_HZ,
        }
    }

    pub(super) fn handles(&self, addr: u16) -> bool {
        !self.data.is_empty() && (ADDR_EECR..=ADDR_EEARH).contains(&addr)
    }

    /// Back to the reset state of the registers. The array is kept.
    pub(super) fn reset(&mut self) {
        self.eedr = 0;
        self.eear = 0;
        self.eecr = 0;
        self.eempe_until = 0;
        self.busy_until = 0;
    }

    fn busy(&self, now: u64) -> bool {
        now < self.busy_until
    }

    fn mask(&self) -> u16 {
        (self.data.len() - 1) as u16
    }

    /// `EERIE` is set: the `EE_READY` level has to be followed.
    #[inline]
    pub(super) fn irq_watched(&self) -> bool {
        self.eecr & EERIE != 0
    }

    /// Clocks until the write in progress ends, if `EE_READY` waits on it.
    pub(super) fn ready_wake_cycles(&self, now: u64) -> Option<u64> {
        (self.irq_watched() && self.busy(now)).then(|| self.busy_until - now)
    }

    /// Whether `EE_READY` is pending at `now`.
    pub(super) fn ready_irq(&self, now: u64) -> bool {
        self.eecr & EERIE != 0 && !self.busy(now)
    }

    pub(super) fn read(&self, addr: u16, now: u64) -> u8 {
        match addr {
            ADDR_EECR => {
                let mut v = self.eecr & (EEPM_MASK | EERIE);
                if now < self.eempe_until {
                    v |= EEMPE;
                }
                if self.busy(now) {
                    v |= EEPE;
                }
                v
            }
            ADDR_EEDR => self.eedr,
            ADDR_EEARL => self.eear as u8,
            _ => (self.eear >> 8) as u8,
        }
    }

    /// Returns the clocks the CPU is halted by this write.
    pub(super) fn write(&mut self, addr: u16, value: u8, now: u64) -> u64 {
        match addr {
            ADDR_EECR => self.write_eecr(value, now),
            ADDR_EEDR => {
                self.eedr = value;
                0
            }
            ADDR_EEARL => {
                self.eear = ((self.eear & 0xFF00) | u16::from(value)) & self.mask();
                0
            }
            _ => {
                self.eear = ((u16::from(value) << 8) | (self.eear & 0x00FF)) & self.mask();
                0
            }
        }
    }

    fn write_eecr(&mut self, value: u8, now: u64) -> u64 {
        let busy = self.busy(now);
        // EEMPE as it was before this write: the one EEPE checks.
        let master = now < self.eempe_until;
        let keep_pm = if busy {
            self.eecr & EEPM_MASK
        } else {
            value & EEPM_MASK
        };
        self.eecr = keep_pm | (value & EERIE);
        self.eempe_until = if value & EEMPE == 0 {
            0
        } else if master {
            self.eempe_until
        } else {
            now + EEMPE_WINDOW
        };
        if value & EEPE != 0 && master && !busy {
            self.eempe_until = 0;
            let a = usize::from(self.eear);
            let us = match self.eecr & EEPM_MASK {
                0x00 => {
                    self.data[a] = self.eedr;
                    ERASE_WRITE_US
                }
                0x10 => {
                    self.data[a] = 0xFF;
                    SPLIT_OP_US
                }
                // Write only: bits can go 1 -> 0, never back.
                _ => {
                    self.data[a] &= self.eedr;
                    SPLIT_OP_US
                }
            };
            self.busy_until = now + us * self.cpu_hz / 1_000_000;
            return 0;
        }
        if value & EERE != 0 && !busy {
            self.eedr = self.data[usize::from(self.eear)];
            return EERE_HALT;
        }
        0
    }
}

impl Avr {
    /// The core clock the EEPROM programming time is counted in.
    pub fn set_cpu_hz(&mut self, hz: u64) {
        if hz != 0 {
            self.eeprom.cpu_hz = hz;
        }
    }

    /// The data EEPROM, as firmware last left it.
    pub fn eeprom(&self) -> &[u8] {
        &self.eeprom.data
    }

    /// Re-raise or drop the `EE_READY` level at an instruction boundary.
    #[inline]
    pub(super) fn sync_eeprom_irq(&mut self) {
        if self.eeprom.ready_irq(self.cycles) {
            self.pending_irq |= 1u64 << VEC_EE_READY;
        } else {
            self.pending_irq &= !(1u64 << VEC_EE_READY);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cpu::avr::AVR_EEPROM_VMA_BIAS;
    use crate::memory::{ProgramImage, Segment};
    use crate::{Arch, Bus, Cpu, DmaRequest, SimResult, SimulationConfig};

    struct NullBus(SimulationConfig);

    impl Bus for NullBus {
        fn read_u8(&self, _addr: u64) -> SimResult<u8> {
            Ok(0)
        }
        fn write_u8(&mut self, _addr: u64, _value: u8) -> SimResult<()> {
            Ok(())
        }
        fn tick_peripherals(&mut self) -> Vec<u32> {
            Vec::new()
        }
        fn execute_dma(&mut self, _requests: &[DmaRequest]) -> SimResult<()> {
            Ok(())
        }
        fn config(&self) -> &SimulationConfig {
            &self.0
        }
    }

    const IO_EECR: u16 = 0x1F;
    const IO_EEDR: u16 = 0x20;
    const IO_EEARL: u16 = 0x21;
    const IO_EEARH: u16 = 0x22;

    fn ldi(d: u16, k: u16) -> u16 {
        0xE000 | ((k & 0xF0) << 4) | ((d - 16) << 4) | (k & 0x0F)
    }
    fn out(a: u16, r: u16) -> u16 {
        0xB800 | ((a & 0x30) << 5) | (r << 4) | (a & 0x0F)
    }
    fn inp(r: u16, a: u16) -> u16 {
        0xB000 | ((a & 0x30) << 5) | (r << 4) | (a & 0x0F)
    }
    fn sbi(a: u16, b: u16) -> u16 {
        0x9A00 | (a << 3) | b
    }
    fn sbic(a: u16, b: u16) -> u16 {
        0x9900 | (a << 3) | b
    }
    /// `RJMP` from word `at` back to word `to`.
    fn rjmp(at: usize, to: usize) -> u16 {
        0xC000 | ((to as i32 - at as i32 - 1) as u16 & 0x0FFF)
    }

    /// avr-libc `eeprom_write_byte` (wait on EEPE, then EEMPE, EEPE),
    /// starting at word `at`: address in r24, data in r22.
    fn write_byte(at: usize, addr: u16, data: u16) -> Vec<u16> {
        vec![
            ldi(24, addr),
            ldi(22, data),
            sbic(IO_EECR, 1),
            rjmp(at + 3, at + 2),
            out(IO_EEARH, 1),
            out(IO_EEARL, 24),
            out(IO_EEDR, 22),
            sbi(IO_EECR, 2),
            sbi(IO_EECR, 1),
        ]
    }

    fn run_to(cpu: &mut Avr, bus: &mut NullBus, pc: u32, max: usize) {
        let config = SimulationConfig::default();
        for _ in 0..max {
            if cpu.pc == pc {
                return;
            }
            cpu.step(bus, &[], &config).unwrap();
        }
        panic!("pc {:#x} not reached, stuck at {:#x}", pc, cpu.pc);
    }

    /// The Arduino `EEPROM.write` loop that hung: the second write waits on
    /// EEPE, which a plain stored register never cleared.
    #[test]
    fn back_to_back_writes_complete_and_read_back() {
        let mut cpu = Avr::for_chip_name("atmega328p");
        let mut bus = NullBus(SimulationConfig::default());
        let mut prog = write_byte(0, 5, 0x42);
        prog.extend(write_byte(prog.len(), 6, 0x43));
        let wait = prog.len();
        prog.extend([sbic(IO_EECR, 1), rjmp(wait + 1, wait)]);
        prog.extend([
            ldi(24, 5),
            out(IO_EEARL, 24),
            sbi(IO_EECR, 0),
            inp(20, IO_EEDR),
        ]);
        let done = prog.len();
        prog.push(rjmp(done, done));
        cpu.load_words(0, &prog);

        run_to(&mut cpu, &mut bus, (done * 2) as u32, 400_000);
        assert_eq!(&cpu.eeprom()[5..7], &[0x42, 0x43]);
        assert_eq!(cpu.r[20], 0x42, "EERE copies EEPROM[EEAR] to EEDR");
        assert_eq!(cpu.eeprom()[7], 0xFF, "untouched cells stay erased");
        // Two 3.4 ms erase+write cycles at 16 MHz.
        assert!(cpu.cycles >= 2 * 54_400, "cycles {}", cpu.cycles);
        assert!(cpu.cycles < 2 * 54_400 + 1_000, "cycles {}", cpu.cycles);
    }

    #[test]
    fn eepe_without_eempe_is_ignored_and_eempe_expires() {
        let mut ee = Eeprom::new(1024);
        ee.write(ADDR_EEARL, 3, 0);
        ee.write(ADDR_EEDR, 0x11, 0);
        ee.write(ADDR_EECR, EEPE, 0);
        assert_eq!(ee.data[3], 0xFF);
        assert_eq!(ee.read(ADDR_EECR, 1), 0);

        ee.write(ADDR_EECR, EEMPE, 10);
        assert_eq!(ee.read(ADDR_EECR, 13), EEMPE);
        assert_eq!(ee.read(ADDR_EECR, 14), 0, "EEMPE clears after four clocks");
        ee.write(ADDR_EECR, EEMPE | EEPE, 14);
        assert_eq!(ee.data[3], 0xFF, "EEPE after the window is ignored");
    }

    #[test]
    fn eepm_modes_and_busy_interlock() {
        let mut ee = Eeprom::new(1024);
        ee.data[0] = 0xF0;
        ee.write(ADDR_EEDR, 0x3C, 0);
        // Write only (EEPM = 10): bits only clear.
        ee.write(ADDR_EECR, 0x20 | EEMPE, 0);
        ee.write(ADDR_EECR, 0x20 | EEMPE | EEPE, 1);
        assert_eq!(ee.data[0], 0x30);
        let busy = 1 + SPLIT_OP_US * 16;
        assert_eq!(ee.read(ADDR_EECR, busy - 1) & EEPE, EEPE);
        // EERE and a mode change are ignored while the write runs.
        ee.write(ADDR_EEDR, 0, busy - 1);
        assert_eq!(ee.write(ADDR_EECR, EERE, busy - 1), 0);
        assert_eq!(ee.read(ADDR_EEDR, busy - 1), 0);
        assert_eq!(ee.read(ADDR_EECR, busy) & (EEPE | EEPM_MASK), 0x20);
        // Erase only (EEPM = 01).
        ee.write(ADDR_EECR, 0x10 | EEMPE, busy);
        ee.write(ADDR_EECR, 0x10 | EEMPE | EEPE, busy + 1);
        assert_eq!(ee.data[0], 0xFF);
        assert_eq!(
            ee.write(ADDR_EECR, EERE, busy + 1 + SPLIT_OP_US * 16),
            EERE_HALT
        );
        assert_eq!(ee.read(ADDR_EEDR, 0), 0xFF);
    }

    #[test]
    fn address_is_masked_to_the_array() {
        let mut ee = Eeprom::new(1024);
        ee.write(ADDR_EEARH, 0xFF, 0);
        ee.write(ADDR_EEARL, 0xFF, 0);
        assert_eq!(ee.read(ADDR_EEARH, 0), 0x03);
        let mut big = Eeprom::new(4096);
        big.write(ADDR_EEARH, 0xFF, 0);
        assert_eq!(big.read(ADDR_EEARH, 0), 0x0F);
    }

    #[test]
    fn ee_ready_is_a_level_on_eerie_and_idle() {
        let mut cpu = Avr::for_chip_name("atmega328p");
        cpu.eeprom.write(ADDR_EECR, EERIE, 0);
        cpu.sync_eeprom_irq();
        assert_ne!(cpu.pending_irq & (1 << VEC_EE_READY), 0);
        cpu.eeprom.write(ADDR_EECR, EERIE | EEMPE, 0);
        cpu.eeprom.write(ADDR_EECR, EERIE | EEMPE | EEPE, 1);
        cpu.cycles = 2;
        cpu.sync_eeprom_irq();
        assert_eq!(cpu.pending_irq & (1 << VEC_EE_READY), 0, "busy");
        assert_eq!(
            cpu.eeprom.ready_wake_cycles(2),
            Some(ERASE_WRITE_US * 16 - 1)
        );
        cpu.cycles = 1 + ERASE_WRITE_US * 16;
        cpu.sync_eeprom_irq();
        assert_ne!(cpu.pending_irq & (1 << VEC_EE_READY), 0);
        assert_eq!(super::super::mega2560_vector(VEC_EE_READY), 31);
    }

    /// `EEMEM` used to be copied over the I/O file at data addresses
    /// 0x20..0xFF (and SRAM above), and EEPROM.get then read 0xFF.
    #[test]
    fn elf_eeprom_section_loads_into_the_eeprom_and_survives_reset() {
        let mut cpu = Avr::for_chip_name("atmega328p");
        let mut image = ProgramImage::new(0, Arch::Avr);
        image.segments.push(Segment {
            start_addr: AVR_EEPROM_VMA_BIAS + 0x3E,
            data: vec![0xA5, 0x5A, 0x77, 0x01, 0x02, 0x03],
        });
        cpu.load_program_image(&image);
        assert_eq!(
            &cpu.eeprom()[0x3E..0x44],
            &[0xA5, 0x5A, 0x77, 0x01, 0x02, 0x03]
        );
        assert_eq!(cpu.io[0x3F - 0x20], 0, "EECR is not clobbered");
        assert!(cpu.sram.iter().all(|b| *b == 0));

        let mut bus = NullBus(SimulationConfig::default());
        cpu.reset(&mut bus).unwrap();
        assert_eq!(cpu.eeprom()[0x3E], 0xA5, "non-volatile across reset");
    }

    #[test]
    fn unmodelled_parts_have_no_eeprom_window() {
        let cpu = Avr::for_chip_name("attiny85");
        assert!(cpu.eeprom().is_empty());
        assert!(!cpu.eeprom.handles(ADDR_EECR));
    }
}
