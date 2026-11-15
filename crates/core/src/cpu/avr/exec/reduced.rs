// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.
//
// AVRrc reduced-core decode (ATtiny4/5/9/10, Atmel-8127H; AVR Instruction
// Set Manual DS40002198). Runs before the shared decode chain, and only on a
// reduced-core model. It owns the opcodes whose meaning differs on AVRrc and
// refuses the ones the core does not implement, so the shared chain never
// runs a classic instruction on silicon that lacks it.

use crate::cpu::avr::Avr;
use crate::{Bus, SimResult, SimulationError};

/// `true` when `op` is not an AVRrc instruction: an opcode the reduced core
/// does not implement, or a register operand below r16 (AVRrc has r16..r31
/// only). One-word LDS/STS (0xA000..=0xAFFF) is handled before this check.
fn avrrc_illegal(op: u16) -> bool {
    let rd = (op >> 4) & 0x1F;
    match op {
        // MOVW, MULS, MULSU/FMUL/FMULS/FMULSU.
        0x0100..=0x03FF => true,
        // Two-register ALU and CPSE/MOV: Rd and Rr must be r16..r31.
        0x0400..=0x2FFF => {
            let rr = (op & 0x0F) | ((op >> 5) & 0x10);
            rd < 16 || rr < 16
        }
        // LDD/STD Y+q / Z+q with q != 0 do not exist; q = 0 is LD/ST Y/Z.
        0x8000..=0x8FFF => {
            let q = (op & 0x07) | ((op >> 7) & 0x18) | ((op >> 8) & 0x20);
            q != 0 || rd < 16
        }
        // Low nibble 0: two-word LDS/STS. 4..=7: LPM/ELPM Z and Z+ (loads),
        // XCH/LAS/LAC/LAT (stores). The rest are LD/ST X/Y/Z and PUSH/POP.
        0x9000..=0x93FF => matches!(op & 0x000F, 0x0 | 0x4..=0x7) || rd < 16,
        // LPM, ELPM, SPM (both forms), EIJMP, EICALL.
        0x95C8 | 0x95D8 | 0x95E8 | 0x95F8 | 0x9419 | 0x9519 => true,
        // JMP, CALL.
        _ if op & 0xFE0E == 0x940C || op & 0xFE0E == 0x940E => true,
        // One-register ops (COM NEG SWAP INC ASR LSR ROR DEC).
        0x9400..=0x95FF if matches!(op & 0x0F, 0x0..=0x7 | 0xA) => rd < 16,
        // ADIW, SBIW, MUL.
        0x9600..=0x97FF | 0x9C00..=0x9FFF => true,
        // IN/OUT, BLD/BST/SBRC/SBRS.
        0xB000..=0xBFFF | 0xF800..=0xFFFF => rd < 16,
        _ => false,
    }
}

/// Data address of a one-word AVRrc LDS/STS: (~b8, b8, b10, b9, b3..b0),
/// reaching 0x40..=0xBF.
#[inline]
fn avrrc_direct_address(op: u16) -> u16 {
    let b8 = (op >> 8) & 1;
    ((b8 ^ 1) << 7) | (b8 << 6) | (((op >> 10) & 1) << 5) | (((op >> 9) & 1) << 4) | (op & 0x0F)
}

impl Avr {
    /// AVRrc-only decode. `Ok(Some(()))` when handled, `Ok(None)` to fall
    /// through to the shared chain, `Err(DecodeError)` for an instruction the
    /// reduced core does not implement.
    #[inline(always)]
    pub(in crate::cpu::avr) fn exec_avrrc(
        &mut self,
        bus: &mut dyn Bus,
        op: u16,
        pc: u32,
        next: u32,
    ) -> SimResult<Option<()>> {
        // One-word LDS Rd,k: 1010 0kkk dddd kkkk, d = 16..31.
        // One-word STS k,Rr: 1010 1kkk dddd kkkk.
        if op & 0xF000 == 0xA000 {
            let reg = 16 + usize::from((op >> 4) & 0x0F);
            let k = avrrc_direct_address(op);
            if op & 0x0800 == 0 {
                self.r[reg] = self.data_read(k, bus)?;
            } else {
                self.data_write(k, self.r[reg], bus)?;
            }
            self.pc = next;
            self.cycles += 1;
            return Ok(Some(()));
        }
        if avrrc_illegal(op) {
            return Err(SimulationError::DecodeError(u64::from(pc)));
        }
        Ok(None)
    }
}

#[cfg(test)]
mod tests {
    use super::{avrrc_direct_address, avrrc_illegal};

    #[test]
    fn direct_address_matches_binutils_encodings() {
        // avr-as -mmcu=attiny10 output.
        assert_eq!(avrrc_direct_address(0xA100), 0x40); // lds r16, 0x40
        assert_eq!(avrrc_direct_address(0xA30F), 0x5F); // lds r16, 0x5f
        assert_eq!(avrrc_direct_address(0xA6FF), 0xBF); // lds r31, 0xbf
        assert_eq!(avrrc_direct_address(0xA910), 0x40); // sts 0x40, r17
        assert_eq!(avrrc_direct_address(0xAF0F), 0x7F); // sts 0x7f, r16
        assert_eq!(avrrc_direct_address(0xA800), 0x80); // sts 0x80, r16
    }

    #[test]
    fn refuses_instructions_the_reduced_core_lacks() {
        for op in [
            0x0189, // movw r16, r18
            0x9601, // adiw r24, 1
            0x9701, // sbiw r24, 1
            0x9F01, // mul r16, r17
            0x95C8, // lpm
            0x9104, // lpm r16, Z
            0x9105, // lpm r16, Z+
            0x95E8, // spm
            0x9100, // lds r16, k (two-word)
            0x9300, // sts k, r16 (two-word)
            0x940C, // jmp
            0x940E, // call
            0x8101, // ldd r16, Z+1
            0x8309, // std Y+1, r16
            0x0C01, // add r0, r1
            0x2F01, // mov r16, r17 is legal: checked below
        ] {
            if op == 0x2F01 {
                assert!(!avrrc_illegal(op));
            } else {
                assert!(avrrc_illegal(op), "{op:#06x}");
            }
        }
    }

    #[test]
    fn accepts_reduced_core_instructions() {
        for op in [
            0x0000, // nop
            0x0F01, // add r16, r17
            0xE50A, // ldi r16, 0x5A
            0x8100, // ld r16, Z
            0x9101, // ld r16, Z+
            0x9102, // ld r16, -Z
            0x9309, // st Y+, r16
            0x910C, // ld r16, X
            0x930F, // push r16
            0x910F, // pop r16
            0x9503, // inc r16
            0x9508, // ret
            0xB901, // out 0x01, r16
            0xB100, // in r16, 0x00
            0xFD00, // sbrc r16, 0
            0x9A10, // sbi 0x02, 0
            0xCFFF, // rjmp .-2
        ] {
            assert!(!avrrc_illegal(op), "{op:#06x}");
        }
    }
}
