// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! Call-stack unwinding from DWARF call frame information (`.debug_frame`,
//! then `.eh_frame`).
//!
//! The unwinder starts from the live registers of the stopped core and
//! recovers, frame by frame, the canonical frame address (CFA) and the
//! registers of each caller. It stops where there is no CFI for the pc, the
//! return address is 0 or a Cortex-M `EXC_RETURN` value (it does not unwind
//! through an exception entry), the CFA does not grow, or at the frame limit.

use crate::variables::{RegisterSet, TargetMemory};
use crate::{DwarfReader, SymbolProvider};
use gimli::{
    BaseAddresses, CfaRule, DebugFrame, EhFrame, RegisterRule, UnwindContext, UnwindSection,
    UnwindTableRow,
};
use std::sync::Arc;

/// The call frame information of one ELF.
#[derive(Default)]
pub(crate) struct CfiSections {
    debug_frame: Option<DebugFrame<DwarfReader>>,
    eh_frame: Option<(EhFrame<DwarfReader>, BaseAddresses)>,
    /// The ELF's address size in bytes.
    pub(crate) address_size: u8,
}

impl CfiSections {
    pub(crate) fn from_object(object: &object::File<'_>) -> Self {
        use object::{Object, ObjectSection};
        let address_size: u8 = if object.is_64() { 8 } else { 4 };
        let reader = |name: &str| {
            let section = object.section_by_name(name)?;
            let data = section.uncompressed_data().ok()?;
            if data.is_empty() {
                return None;
            }
            Some((
                DwarfReader::new(Arc::from(&data[..]), gimli::RunTimeEndian::Little),
                section.address(),
            ))
        };
        let debug_frame = reader(".debug_frame").map(|(r, _)| {
            let mut s = DebugFrame::from(r);
            s.set_address_size(address_size);
            s
        });
        let eh_frame = reader(".eh_frame").map(|(r, addr)| {
            let mut s = EhFrame::from(r);
            s.set_address_size(address_size);
            let mut bases = BaseAddresses::default().set_eh_frame(addr);
            if let Some(text) = object.section_by_name(".text") {
                bases = bases.set_text(text.address());
            }
            (s, bases)
        });
        Self {
            debug_frame,
            eh_frame,
            address_size,
        }
    }

    /// The CFI row for `addr` and the return-address register of its CIE.
    fn row(&self, addr: u64) -> Option<(UnwindTableRow<DwarfReader>, u16)> {
        let mut ctx = UnwindContext::new();
        if let Some(section) = &self.debug_frame {
            let bases = BaseAddresses::default();
            if let Ok(fde) = section.fde_for_address(&bases, addr, DebugFrame::cie_from_offset) {
                if let Ok(row) = fde.unwind_info_for_address(section, &bases, &mut ctx, addr) {
                    return Some((row.clone(), fde.cie().return_address_register().0));
                }
            }
        }
        if let Some((section, bases)) = &self.eh_frame {
            if let Ok(fde) = section.fde_for_address(bases, addr, EhFrame::cie_from_offset) {
                if let Ok(row) = fde.unwind_info_for_address(section, bases, &mut ctx, addr) {
                    return Some((row.clone(), fde.cie().return_address_register().0));
                }
            }
        }
        None
    }
}

/// What the unwinder needs to know about the architecture.
#[derive(Debug, Clone)]
pub struct Abi {
    /// DWARF number of the stack pointer.
    pub sp: u16,
    /// Registers a callee preserves. In a caller's frame, a register the CFI
    /// does not describe keeps its value when it is one of these; any other
    /// is unknown.
    pub callee_saved: Vec<u16>,
    /// Mask applied to a return address to get the code address (`!1` for
    /// Arm Thumb).
    pub code_mask: u64,
    /// Width of a register in bytes.
    pub register_size: u8,
    /// Whether to unwind past frame 0 at all.
    pub unwind: bool,
    /// Stop at a Cortex-M `EXC_RETURN` return address (0xFFFF_FFFx).
    pub exc_return: bool,
}

/// One physical (not inlined) frame of the call stack.
#[derive(Debug, Clone)]
pub struct PhysicalFrame {
    /// The frame's pc: the live pc for frame 0, the return address (code
    /// address) for callers.
    pub pc: u64,
    /// The address to look up line, scope and CFI information with. For a
    /// caller this is inside the call instruction, not after it.
    pub lookup_pc: u64,
    /// The frame's registers, by DWARF number; `None` where not recovered.
    pub regs: RegisterSet,
    /// The canonical frame address, when the CFI gives one.
    pub cfa: Option<u64>,
}

impl SymbolProvider {
    /// Unwinds the stack from the live registers `regs` at `pc`, innermost
    /// frame first, at most `max_frames` frames. Frame 0 is always present.
    pub fn unwind(
        &self,
        pc: u64,
        regs: RegisterSet,
        mem: &dyn TargetMemory,
        abi: &Abi,
        max_frames: usize,
    ) -> Vec<PhysicalFrame> {
        let mut frames = vec![PhysicalFrame {
            pc,
            lookup_pc: pc,
            regs,
            cfa: None,
        }];
        let ctx_size = self.cfi.address_size;
        loop {
            let index = frames.len() - 1;
            let frame = &frames[index];
            let Some((row, ra_reg)) = self.cfi.row(frame.lookup_pc) else {
                break;
            };
            let Some(cfa) = self.cfa_of(&row, &frame.regs, mem, ctx_size) else {
                break;
            };
            frames[index].cfa = Some(cfa);
            if !abi.unwind || frames.len() >= max_frames {
                break;
            }
            // The CFA must grow towards the caller, else the stack is corrupt
            // or the unwind loops.
            if index > 0 && frames[index - 1].cfa.is_some_and(|prev| cfa <= prev) {
                break;
            }
            let frame = &frames[index];
            let mut caller = RegisterSet::default();
            for reg in 0..frame.regs.len() as u16 {
                let value = match row.register(gimli::Register(reg)) {
                    RegisterRule::Undefined => {
                        // Undescribed: a callee-saved register keeps its
                        // value. In frame 0 the return-address register still
                        // holds the return address (a leaf, or before the
                        // prologue saved it).
                        if abi.callee_saved.contains(&reg) || (index == 0 && reg == ra_reg) {
                            frame.regs.get(reg)
                        } else {
                            None
                        }
                    }
                    RegisterRule::SameValue => frame.regs.get(reg),
                    rule => self.apply_rule(&rule, cfa, &frame.regs, mem, abi),
                };
                caller.set(reg, value);
            }
            caller.set(abi.sp, Some(cfa));
            let Some(ra) = caller.get(ra_reg) else { break };
            if ra == 0 || (abi.exc_return && ra & 0xFFFF_FFF0 == 0xFFFF_FFF0) {
                break;
            }
            let pc = ra & abi.code_mask;
            frames.push(PhysicalFrame {
                pc,
                lookup_pc: pc.wrapping_sub(1) & abi.code_mask,
                regs: caller,
                cfa: None,
            });
        }
        frames
    }

    fn cfa_of(
        &self,
        row: &UnwindTableRow<DwarfReader>,
        regs: &RegisterSet,
        mem: &dyn TargetMemory,
        address_size: u8,
    ) -> Option<u64> {
        match row.cfa() {
            CfaRule::RegisterAndOffset { register, offset } => {
                Some(regs.get(register.0)?.wrapping_add(*offset as u64) & mask(address_size))
            }
            CfaRule::Expression(expr) => {
                let value =
                    crate::variables::eval_cfi(expr.clone(), None, regs, mem, address_size)?;
                Some(value)
            }
        }
    }

    fn apply_rule(
        &self,
        rule: &RegisterRule<DwarfReader>,
        cfa: u64,
        regs: &RegisterSet,
        mem: &dyn TargetMemory,
        abi: &Abi,
    ) -> Option<u64> {
        let size = abi.register_size;
        let read = |addr: u64| -> Option<u64> {
            let mut buf = [0u8; 8];
            mem.read_memory(addr, &mut buf[..size as usize])
                .then(|| u64::from_le_bytes(buf))
        };
        let address_size = self.cfi.address_size;
        match rule {
            RegisterRule::Undefined | RegisterRule::Architectural | RegisterRule::SameValue => None,
            RegisterRule::Offset(o) => read(cfa.wrapping_add(*o as u64) & mask(address_size)),
            RegisterRule::ValOffset(o) => Some(cfa.wrapping_add(*o as u64) & mask(address_size)),
            RegisterRule::Register(r) => regs.get(r.0),
            RegisterRule::Expression(e) => read(crate::variables::eval_cfi(
                e.clone(),
                Some(cfa),
                regs,
                mem,
                address_size,
            )?),
            RegisterRule::ValExpression(e) => {
                crate::variables::eval_cfi(e.clone(), Some(cfa), regs, mem, address_size)
            }
            RegisterRule::Constant(c) => Some(*c),
            _ => None,
        }
    }
}

fn mask(address_size: u8) -> u64 {
    if address_size >= 8 {
        !0
    } else {
        (1u64 << (u32::from(address_size) * 8)) - 1
    }
}
