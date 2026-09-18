// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Arm bodies of `XtensaLx7::execute` for the misc instruction class,
//! moved here verbatim. `execute` keeps the single `match ins`; each arm
//! calls one of these `#[inline(always)]` methods.

use crate::cpu::xtensa_lx7::XtensaLx7;
use crate::{Bus, SimResult};
impl XtensaLx7 {
    #[inline(always)]
    pub(in crate::cpu::xtensa_lx7) fn exec_nop_fence(
        &mut self,
        _bus: &mut dyn Bus,
        len: u32,
    ) -> SimResult<()> {
        self.pc = self.pc.wrapping_add(len);
        Ok(())
    }

    #[inline(always)]
    pub(in crate::cpu::xtensa_lx7) fn exec_wer(
        &mut self,
        _bus: &mut dyn Bus,
        len: u32,
    ) -> SimResult<()> {
        self.pc = self.pc.wrapping_add(len);
        Ok(())
    }
}
