// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

use crate::bus::SystemBus;
use crate::decoder::arm::{decode_thumb_16, decode_thumb_32, Instruction};
use crate::{Bus, Cpu, SimResult, SimulationConfig, SimulationObserver};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;

#[derive(Debug, Clone, Copy)]
pub struct DecodeCacheEntry {
    pub tag: u32,
    pub instruction: Instruction,
    pub opcode: u32,
    pub pc_increment: u8,
    pub cycles: u32,
}

#[derive(Debug)]
pub struct CortexM {
    pub r0: u32,
    pub r1: u32,
    pub r2: u32,
    pub r3: u32,
    pub r4: u32,
    pub r5: u32,
    pub r6: u32,
    pub r7: u32,
    pub r8: u32,
    pub r9: u32,
    pub r10: u32,
    pub r11: u32,
    pub r12: u32,
    pub sp: u32, // R13 — the *active* stack pointer (live R13).
    /// Banked Main / Process stack pointers (ARMv7-M). `sp` is always the
    /// live copy of whichever bank is currently selected; the OTHER bank is
    /// held here. Handler mode always uses MSP; Thread mode uses MSP or PSP
    /// per `CONTROL.SPSEL`. The active bank's stored field is treated as
    /// stale — `sp` is authoritative for it until the next stack switch.
    pub msp: u32,
    pub psp: u32,
    /// CONTROL register. Only SPSEL (bit 1) and nPRIV (bit 0) are modelled;
    /// FPCA (bit 2) is not (no lazy FP stacking).
    pub control: u32,
    pub lr: u32, // R14
    pub pc: u32, // R15
    pub xpsr: u32,
    /// Pending-exception bitmask, 4x64 = exceptions 0..255. The H5-class
    /// parts wire external interrupts past IRQ 47 (STM32H563 TIM12 = IRQ
    /// 120 -> exception 136), which a single u64 silently dropped —
    /// caught by foreign firmware whose time driver never ticked.
    pub pending_exceptions: [u64; 4],
    pub primask: bool, // Interrupt mask (true = disabled)
    /// FAULTMASK: when set, masks every exception except NMI (it raises the
    /// effective priority to -1). Set by `CPSID f` / `MSR FAULTMASK`, cleared by
    /// `CPSIE f` and automatically on exception return (except return from NMI).
    /// Zephyr's fault path toggles it; an unmodelled `CPS f` decoded as Unknown.
    pub faultmask: bool,
    /// BASEPRI: when non-zero, masks any exception whose priority value is
    /// numerically >= basepri (i.e. equal or lower priority). Zephyr's Cortex-M
    /// critical sections raise BASEPRI to block the scheduler/timer IRQs; an
    /// unmodelled BASEPRI let those fire mid-critical-section and corrupt kernel
    /// state. NMI/HardFault (negative priority) are never masked by it.
    pub basepri: u8,
    pub vtor: Arc<AtomicU32>, // Shared Vector Table Offset Register
    pub it_state: u8,         // Thumb IT block state
    /// Currently active exception number (0 = thread mode). Used to prevent re-entry
    /// of the same or lower-priority exception while one is already being serviced.
    pub active_exception: u32,
    /// Shared with SCB peripheral: live mirror of `active_exception` so
    /// firmware reading ICSR.VECTACTIVE sees the currently-handling
    /// exception. cortex-m-rt's DefaultHandler depends on this to
    /// route to the right IRQ branch.
    pub vectactive: Arc<AtomicU32>,
    /// Shared with SCB: SHPR1 (MemManage/BusFault/UsageFault priority).
    /// Used by `exception_priority` so dispatch honours real ARM priority
    /// rules rather than picking by exception number.
    pub shpr1: Arc<AtomicU32>,
    /// Shared with SCB: SHPR2 (SVCall priority byte 3).
    pub shpr2: Arc<AtomicU32>,
    /// Shared with SCB: SHPR3 (PendSV byte 2, SysTick byte 3). FreeRTOS
    /// sets PendSV to 0xFF (lowest), which lets the SysTick handler set
    /// PENDSVSET and only have it dispatch on return-to-thread — the
    /// load-bearing semantics for context switching.
    pub shpr3: Arc<AtomicU32>,
    /// Shared NVIC state for IRQ priority lookups via IPR.
    pub nvic_state: Option<Arc<crate::peripherals::nvic::NvicState>>,
    pub decode_cache: Box<[Option<DecodeCacheEntry>; 4096]>,
    /// FPU single-precision register file (VFPv4 single — S0..S31).
    /// Each S register is the IEEE-754 binary32 bit pattern; reads via
    /// `f32::from_bits` and writes via `f32::to_bits`. Double-precision
    /// (D0..D15 = pairs of S regs) is NOT modelled; firmware compiled for
    /// `-mfpu=fpv4-sp-d16` only emits single-precision ops anyway.
    pub fpu_s: [u32; 32],
    /// True while the core is suspended in WFI sleep. Set by the `Wfi`
    /// executor when no wake-up event is pending, cleared at the top of every
    /// `step_internal`. Gates idle fast-forward; transient (not snapshotted),
    /// mirroring the RISC-V `waiting_for_interrupt` flag.
    sleeping: bool,
}

impl Default for CortexM {
    fn default() -> Self {
        Self {
            r0: 0,
            r1: 0,
            r2: 0,
            r3: 0,
            r4: 0,
            r5: 0,
            r6: 0,
            r7: 0,
            r8: 0,
            r9: 0,
            r10: 0,
            r11: 0,
            r12: 0,
            sp: 0,
            msp: 0,
            psp: 0,
            control: 0,
            lr: 0,
            pc: 0,
            xpsr: 0x01000000, // Typical reset state (Thumb bit set)
            pending_exceptions: [0; 4],
            primask: false,
            faultmask: false,
            basepri: 0,
            vtor: Arc::new(AtomicU32::new(0)),
            it_state: 0,
            active_exception: 0,
            vectactive: Arc::new(AtomicU32::new(0)),
            shpr1: Arc::new(AtomicU32::new(0)),
            shpr2: Arc::new(AtomicU32::new(0)),
            shpr3: Arc::new(AtomicU32::new(0)),
            nvic_state: None,
            decode_cache: Box::new([None; 4096]),
            fpu_s: [0u32; 32],
            sleeping: false,
        }
    }
}

impl CortexM {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn get_vtor(&self) -> u32 {
        self.vtor.load(Ordering::SeqCst)
    }

    pub fn set_vtor(&mut self, val: u32) {
        self.vtor.store(val, Ordering::SeqCst);
    }

    pub fn set_shared_vtor(&mut self, vtor: Arc<AtomicU32>) {
        self.vtor = vtor;
    }

    pub fn set_shared_vectactive(&mut self, vectactive: Arc<AtomicU32>) {
        self.vectactive = vectactive;
    }

    /// Wire the three SHPR atomics shared with the SCB peripheral. Once
    /// shared, `exception_priority` can read live priorities and the
    /// dispatch loop honours ARMv7-M priority rules instead of just
    /// picking the lowest-numbered pending exception.
    pub fn set_shared_shpr(
        &mut self,
        shpr1: Arc<AtomicU32>,
        shpr2: Arc<AtomicU32>,
        shpr3: Arc<AtomicU32>,
    ) {
        self.shpr1 = shpr1;
        self.shpr2 = shpr2;
        self.shpr3 = shpr3;
    }

    /// Wire the NVIC's shared state so `exception_priority` can read
    /// IPR bytes for IRQs (exception number ≥ 16).
    pub fn set_shared_nvic_state(&mut self, state: Arc<crate::peripherals::nvic::NvicState>) {
        self.nvic_state = Some(state);
    }

    /// ARMv7-M exception priority. Lower numeric value = higher priority.
    /// Reset(1) = -3, NMI(2) = -2, HardFault(3) = -1 are fixed. Configurable
    /// system exceptions read from SHPR1/2/3. IRQs (≥16) read from the
    /// NVIC IPR byte for that IRQ. Unmapped or unknown exceptions return
    /// 0xFF (lowest configurable priority).
    pub fn exception_priority(&self, exc: u32) -> i32 {
        match exc {
            0 => 256,
            1 => -3,
            2 => -2,
            3 => -1,
            4 => (self.shpr1.load(Ordering::Relaxed) & 0xFF) as i32,
            5 => ((self.shpr1.load(Ordering::Relaxed) >> 8) & 0xFF) as i32,
            6 => ((self.shpr1.load(Ordering::Relaxed) >> 16) & 0xFF) as i32,
            11 => ((self.shpr2.load(Ordering::Relaxed) >> 24) & 0xFF) as i32,
            14 => ((self.shpr3.load(Ordering::Relaxed) >> 16) & 0xFF) as i32,
            15 => ((self.shpr3.load(Ordering::Relaxed) >> 24) & 0xFF) as i32,
            n if n >= 16 => {
                if let Some(nvic) = &self.nvic_state {
                    nvic.ipr_priority((n - 16) as usize) as i32
                } else {
                    0xFF
                }
            }
            _ => 0xFF,
        }
    }

    /// True if BASEPRI masks an exception of the given priority. A non-zero
    /// BASEPRI masks any exception whose priority value is numerically >=
    /// BASEPRI (equal or lower priority). NMI/HardFault (negative priority) are
    /// never masked.
    #[inline]
    fn masked_by_basepri(&self, prio: i32) -> bool {
        self.basepri != 0 && prio >= self.basepri as i32
    }

    /// Among the pending exceptions, return the one with the highest
    /// priority (lowest numeric value). Ties break by exception number
    /// (lower number wins, per ARMv7-M B1.5.4).
    fn highest_priority_pending(&self) -> Option<u32> {
        let mut best: Option<(i32, u32)> = None;
        for (word_idx, &word) in self.pending_exceptions.iter().enumerate() {
            let mut mask = word;
            while mask != 0 {
                let exc = (word_idx as u32) * 64 + mask.trailing_zeros();
                mask &= mask - 1;
                let prio = self.exception_priority(exc);
                best = Some(match best {
                    Some((bp, be)) if bp <= prio => (bp, be),
                    _ => (prio, exc),
                });
            }
        }
        best.map(|(_, e)| e)
    }

    /// Update both the local field and the SCB.ICSR mirror in one go.
    fn set_active_exception(&mut self, exc: u32) {
        self.active_exception = exc;
        self.vectactive
            .store(exc & 0x1FF, std::sync::atomic::Ordering::Relaxed);
    }

    fn xpsr_with_itstate(&self, xpsr: u32) -> u32 {
        let mut out = xpsr & !((0b11 << 25) | (0b11_1111 << 10));
        out |= ((self.it_state as u32) & 0b11) << 25;
        out |= (((self.it_state as u32) >> 2) & 0b11_1111) << 10;
        out
    }

    fn itstate_from_xpsr(xpsr: u32) -> u8 {
        let low = ((xpsr >> 25) & 0b11) as u8;
        let high = ((xpsr >> 10) & 0b11_1111) as u8;
        low | (high << 2)
    }

    fn read_reg(&self, n: u8) -> u32 {
        match n {
            0 => self.r0,
            1 => self.r1,
            2 => self.r2,
            3 => self.r3,
            4 => self.r4,
            5 => self.r5,
            6 => self.r6,
            7 => self.r7,
            8 => self.r8,
            9 => self.r9,
            10 => self.r10,
            11 => self.r11,
            12 => self.r12,
            13 => self.sp,
            14 => self.lr,
            15 => self.pc,
            16 => self.xpsr,
            _ => 0,
        }
    }

    fn write_reg(&mut self, n: u8, val: u32) {
        match n {
            0 => self.r0 = val,
            1 => self.r1 = val,
            2 => self.r2 = val,
            3 => self.r3 = val,
            4 => self.r4 = val,
            5 => self.r5 = val,
            6 => self.r6 = val,
            7 => self.r7 = val,
            8 => self.r8 = val,
            9 => self.r9 = val,
            10 => self.r10 = val,
            11 => self.r11 = val,
            12 => self.r12 = val,
            13 => self.sp = val,
            14 => self.lr = val,
            15 => self.pc = val,
            16 => self.xpsr = val,
            _ => {}
        }
    }

    fn update_nz(&mut self, result: u32) {
        let n = (result >> 31) & 1;
        let z = if result == 0 { 1 } else { 0 };
        // Clear N/Z (bits 31, 30)
        self.xpsr &= !(0xC000_0000);
        self.xpsr |= (n << 31) | (z << 30);
    }

    fn get_carry(&self) -> bool {
        (self.xpsr >> 29) & 1 == 1
    }

    /// APSR.GE[3:0] live in xpsr bits [19:16]; each corresponds to one byte
    /// lane of a SIMD add/sub and is consumed by SEL. `ge` low nibble = GE[3:0].
    fn set_ge(&mut self, ge: u32) {
        self.xpsr &= !(0xF << 16);
        self.xpsr |= (ge & 0xF) << 16;
    }

    fn get_ge(&self) -> u32 {
        (self.xpsr >> 16) & 0xF
    }

    fn get_overflow(&self) -> bool {
        (self.xpsr >> 28) & 1 == 1
    }

    fn update_nzcv(&mut self, result: u32, carry: bool, overflow: bool) {
        let n = (result >> 31) & 1;
        let z = if result == 0 { 1 } else { 0 };
        let c = if carry { 1 } else { 0 };
        let v = if overflow { 1 } else { 0 };

        self.xpsr &= !(0xF000_0000);
        self.xpsr |= (n << 31) | (z << 30) | (c << 29) | (v << 28);
    }

    #[inline(always)]
    fn check_condition(&self, cond: u8) -> bool {
        let n = (self.xpsr >> 31) & 1 == 1;
        let z = (self.xpsr >> 30) & 1 == 1;
        let c = (self.xpsr >> 29) & 1 == 1;
        let v = (self.xpsr >> 28) & 1 == 1;

        match cond {
            0x0 => z,              // EQ (Equal)
            0x1 => !z,             // NE (Not Equal)
            0x2 => c,              // CS/HS (Carry Set)
            0x3 => !c,             // CC/LO (Carry Clear)
            0x4 => n,              // MI (Minus)
            0x5 => !n,             // PL (Plus)
            0x6 => v,              // VS (Overflow)
            0x7 => !v,             // VC (No Overflow)
            0x8 => c && !z,        // HI (Unsigned Higher)
            0x9 => !c || z,        // LS (Unsigned Lower or Same)
            0xA => n == v,         // GE (Signed Greater or Equal)
            0xB => n != v,         // LT (Signed Less Than)
            0xC => !z && (n == v), // GT (Signed Greater Than)
            0xD => z || (n != v),  // LE (Signed Less or Equal)
            0xE => true,           // AL (Always)
            _ => false,            // Undefined/Reserved
        }
    }

    fn branch_to<B: Bus + ?Sized>(&mut self, addr: u32, bus: &mut B) -> SimResult<()> {
        if (addr & 0xFFFFFFF0) == 0xFFFFFFF0 {
            // EXC_RETURN: valid values are 0xFFFFFFF1/F9/FD (and FPU variants E1/E9/ED)
            self.exception_return(addr, bus)?;
        } else {
            self.pc = addr & !1;
        }
        Ok(())
    }

    /// True if FAULTMASK currently blocks taking the given exception. FAULTMASK
    /// masks everything except NMI (exception 2).
    #[inline]
    fn faultmask_blocks(&self, exc: u32) -> bool {
        self.faultmask && exc != 2
    }

    /// ARMv7-M WFI wake-up condition: a pending exception whose priority would
    /// preempt the current execution priority *if it were unmasked*. This
    /// deliberately ignores PRIMASK — the canonical `__disable_irq(); wfi();`
    /// idle pattern must wake on a pend even though PRIMASK blocks the actual
    /// entry (the core then falls through without taking the exception).
    /// BASEPRI/FAULTMASK still gate: an exception they suppress would not
    /// preempt, so it is not a wake event. Mirrors the takeable-exception break
    /// in `step_batch`, minus the `!self.primask` guard.
    fn wfi_wake_pending(&self) -> bool {
        if !self.pending_exceptions.iter().any(|&w| w != 0) {
            return false;
        }
        let Some(exc) = self.highest_priority_pending() else {
            return false;
        };
        let exc_prio = self.exception_priority(exc);
        let active_prio = self.exception_priority(self.active_exception);
        exc_prio < active_prio && !self.masked_by_basepri(exc_prio) && !self.faultmask_blocks(exc)
    }

    /// True when the live `sp` is the Process stack: Thread mode with
    /// CONTROL.SPSEL set. Handler mode always uses MSP.
    #[inline]
    fn use_psp(&self) -> bool {
        self.active_exception == 0 && (self.control & 0x2) != 0
    }

    /// Persist the live `sp` into whichever bank it currently represents.
    /// Call this *before* a transition that changes the selected stack.
    #[inline]
    fn sync_sp_to_bank(&mut self) {
        if self.use_psp() {
            self.psp = self.sp;
        } else {
            self.msp = self.sp;
        }
    }

    /// The stored value of the bank that selection *would* make active now.
    #[inline]
    fn current_stack_value(&self) -> u32 {
        if self.use_psp() {
            self.psp
        } else {
            self.msp
        }
    }

    /// Read MSP regardless of which bank is live.
    #[inline]
    fn read_msp(&self) -> u32 {
        if self.use_psp() {
            self.msp
        } else {
            self.sp
        }
    }

    /// Read PSP regardless of which bank is live.
    #[inline]
    fn read_psp(&self) -> u32 {
        if self.use_psp() {
            self.sp
        } else {
            self.psp
        }
    }

    fn exception_return<B: Bus + ?Sized>(&mut self, exc_return: u32, bus: &mut B) -> SimResult<()> {
        // FAULTMASK is cleared automatically on exception return, except when
        // returning from NMI (exception 2).
        if self.active_exception != 2 {
            self.faultmask = false;
        }

        // We are in Handler mode, so the live `sp` is MSP — capture it.
        self.sync_sp_to_bank();

        // EXC_RETURN bit 2 selects the stack the frame was stacked on:
        // 0 → MSP (returning to a handler or Thread/MSP), 1 → PSP (Thread/PSP).
        let frame_on_psp = (exc_return & 0x4) != 0;
        let frame_ptr = if frame_on_psp { self.psp } else { self.msp };

        self.r0 = bus.read_u32(frame_ptr as u64)?;
        self.r1 = bus.read_u32((frame_ptr + 4) as u64)?;
        self.r2 = bus.read_u32((frame_ptr + 8) as u64)?;
        self.r3 = bus.read_u32((frame_ptr + 12) as u64)?;
        self.r12 = bus.read_u32((frame_ptr + 16) as u64)?;
        self.lr = bus.read_u32((frame_ptr + 20) as u64)?;
        self.pc = bus.read_u32((frame_ptr + 24) as u64)? & !1;
        self.xpsr = bus.read_u32((frame_ptr + 28) as u64)?;
        self.it_state = Self::itstate_from_xpsr(self.xpsr);

        // Advance the bank the frame was popped from.
        let new_sp = frame_ptr + 32;
        if frame_on_psp {
            self.psp = new_sp;
        } else {
            self.msp = new_sp;
        }

        // Restore active exception from stacked xPSR IPSR bits [8:0].
        // When taking an exception, we saved the previous active_exception in IPSR,
        // so restoring it here correctly handles both non-nested and nested cases.
        self.set_active_exception(self.xpsr & 0x1FF);

        // On return to Thread mode, CONTROL.SPSEL takes EXC_RETURN[2].
        if self.active_exception == 0 {
            if frame_on_psp {
                self.control |= 0x2;
            } else {
                self.control &= !0x2;
            }
        }

        // Re-point the live `sp` at whichever bank is now selected.
        self.sp = self.current_stack_value();

        tracing::debug!(
            "EXC_RETURN: frame={:#010x} restored LR={:#010x} PC={:#010x} active_exc={} sp={:#010x}",
            frame_ptr,
            self.lr,
            self.pc,
            self.active_exception,
            self.sp
        );
        Ok(())
    }
}

impl Cpu for CortexM {
    fn reset(&mut self, bus: &mut dyn Bus) -> SimResult<()> {
        self.pc = 0x0000_0000;
        self.sp = 0x2000_0000;
        self.pending_exceptions = [0; 4];
        self.set_active_exception(0);
        self.decode_cache.fill(None);

        // Out of reset the core is in Thread mode using MSP (CONTROL=0); PSP
        // is architecturally UNKNOWN — start it at 0.
        self.control = 0;
        self.psp = 0;

        let vtor = self.vtor.load(Ordering::SeqCst) as u64;
        if let Ok(sp) = bus.read_u32(vtor) {
            self.sp = sp;
        }
        if let Ok(pc) = bus.read_u32(vtor + 4) {
            self.pc = pc & !1;
        }
        self.msp = self.sp;

        Ok(())
    }

    fn get_pc(&self) -> u32 {
        self.pc
    }
    fn set_pc(&mut self, val: u32) {
        self.pc = val & !1;
    }
    fn set_sp(&mut self, val: u32) {
        self.sp = val;
        // Keep the active bank coherent (out-of-reset / external SP loads are
        // on the currently-selected stack).
        self.sync_sp_to_bank();
    }
    fn set_exception_pending(&mut self, exception_num: u32) {
        if std::env::var("LABWIRED_TRACE_EXC").is_ok() {
            eprintln!("EXC pend num={} pc=0x{:08X}", exception_num, self.pc);
        }
        if exception_num < 256 {
            self.pending_exceptions[(exception_num / 64) as usize] |= 1u64 << (exception_num % 64);
        }
    }

    fn get_register(&self, id: u8) -> u32 {
        self.read_reg(id)
    }

    fn set_register(&mut self, id: u8, val: u32) {
        self.write_reg(id, val);
    }

    fn snapshot(&self) -> crate::snapshot::CpuSnapshot {
        crate::snapshot::CpuSnapshot::Arm(crate::snapshot::ArmCpuSnapshot {
            registers: vec![
                self.r0, self.r1, self.r2, self.r3, self.r4, self.r5, self.r6, self.r7, self.r8,
                self.r9, self.r10, self.r11, self.r12, self.sp, self.lr, self.pc,
            ],
            pc: self.pc,
            xpsr: self.xpsr,
            primask: self.primask,
            pending_exceptions: self.pending_exceptions[0],
            pending_exceptions_hi: self.pending_exceptions[1..].to_vec(),
            vtor: self.vtor.load(Ordering::Relaxed),
        })
    }

    fn apply_snapshot(&mut self, snapshot: &crate::snapshot::CpuSnapshot) {
        if let crate::snapshot::CpuSnapshot::Arm(s) = snapshot {
            if s.registers.len() >= 16 {
                self.r0 = s.registers[0];
                self.r1 = s.registers[1];
                self.r2 = s.registers[2];
                self.r3 = s.registers[3];
                self.r4 = s.registers[4];
                self.r5 = s.registers[5];
                self.r6 = s.registers[6];
                self.r7 = s.registers[7];
                self.r8 = s.registers[8];
                self.r9 = s.registers[9];
                self.r10 = s.registers[10];
                self.r11 = s.registers[11];
                self.r12 = s.registers[12];
                self.sp = s.registers[13];
                self.lr = s.registers[14];
                self.pc = s.pc; // Use explicit PC field
            }
            self.xpsr = s.xpsr;
            self.primask = s.primask;
            self.pending_exceptions = [0; 4];
            self.pending_exceptions[0] = s.pending_exceptions;
            for (i, w) in s.pending_exceptions_hi.iter().take(3).enumerate() {
                self.pending_exceptions[i + 1] = *w;
            }
            self.vtor.store(s.vtor, Ordering::Relaxed);
        }
    }

    fn get_register_names(&self) -> Vec<String> {
        let mut names = Vec::new();
        for i in 0..13 {
            names.push(format!("R{}", i));
        }
        names.push("SP".to_string());
        names.push("LR".to_string());
        names.push("PC".to_string());
        names
    }

    fn index_of_register(&self, name: &str) -> Option<u8> {
        match name.to_uppercase().as_str() {
            "R0" => Some(0),
            "R1" => Some(1),
            "R2" => Some(2),
            "R3" => Some(3),
            "R4" => Some(4),
            "R5" => Some(5),
            "R6" => Some(6),
            "R7" => Some(7),
            "R8" => Some(8),
            "R9" => Some(9),
            "R10" => Some(10),
            "R11" => Some(11),
            "R12" => Some(12),
            "SP" | "R13" => Some(13),
            "LR" | "R14" => Some(14),
            "PC" | "R15" => Some(15),
            "XPSR" => Some(16),
            _ => None,
        }
    }

    fn step(
        &mut self,
        bus: &mut dyn Bus,
        observers: &[Arc<dyn SimulationObserver>],
        config: &SimulationConfig,
    ) -> SimResult<()> {
        self.step_internal(bus, observers, config)
    }

    fn step_batch(
        &mut self,
        bus: &mut dyn Bus,
        observers: &[Arc<dyn SimulationObserver>],
        config: &SimulationConfig,
        max_count: u32,
    ) -> SimResult<u32> {
        // Push-mode logic capture: while armed, the tap clock advances once
        // per retired instruction (BEFORE executing it) so MMIO pad writes
        // stamp with the cycle boundary they become observable at. One Arc
        // clone + flag check per batch when disarmed.
        let tap = bus.logic_tap().filter(|t| t.push_armed());

        if !config.batch_mode_enabled {
            for i in 0..max_count {
                if let Some(tap) = &tap {
                    tap.bump_clock();
                }
                self.step(bus, observers, config)?;
                // WFI idle escape: leave the batch once the core is sleeping so
                // `Machine::run` can fast-forward the idle window (mirrors the
                // batch paths below and the RISC-V core).
                if config.idle_fast_forward_enabled && self.idle_fast_forward_budget(bus).is_some()
                {
                    return Ok(i + 1);
                }
            }
            return Ok(max_count);
        }

        let mut executed = 0;

        if let Some(sysbus) = bus.as_any_mut().and_then(|a| a.downcast_mut::<SystemBus>()) {
            while executed < max_count {
                // End the batch early when a takeable exception is pending:
                // its priority must be strictly higher (smaller number) than
                // the currently-active one (or 256 = thread mode baseline).
                // Only ONCE the batch has made progress (`executed > 0`) —
                // at the batch top the pending exception must instead be
                // DISPATCHED by the `step_internal` below (which takes it
                // exactly like the single-step path). Breaking at zero made
                // `Machine::run` return no-progress forever the moment a
                // walk/scheduler-pended IRQ (e.g. SysTick) became takeable
                // between batches, wedging every batched IRQ-driven Cortex-M
                // firmware (walk-free campaign B1 surfaced this — batching is
                // pointless if an armed SysTick freezes the run loop).
                if executed > 0 && self.pending_exceptions.iter().any(|&w| w != 0) && !self.primask
                {
                    if let Some(exc) = self.highest_priority_pending() {
                        let exc_prio = self.exception_priority(exc);
                        let active_prio = self.exception_priority(self.active_exception);
                        if exc_prio < active_prio
                            && !self.masked_by_basepri(exc_prio)
                            && !self.faultmask_blocks(exc)
                        {
                            break;
                        }
                    }
                }
                if let Some(tap) = &tap {
                    tap.bump_clock();
                }
                self.step_internal(sysbus, observers, config)?;
                executed += 1;
                // Taken branches no longer break the batch — the run loop bounds
                // it to the next peripheral tick, so bouncing back through
                // `Machine::run` at every branch was pure overhead. Only WFI
                // sleep leaves the batch, so the machine can fast-forward the
                // idle window.
                if config.idle_fast_forward_enabled
                    && self.idle_fast_forward_budget(sysbus).is_some()
                {
                    break;
                }
            }
        } else {
            while executed < max_count {
                // Same early-out rule as the SystemBus arm above: break only
                // after progress; at the batch top a takeable pending
                // exception is dispatched by `step_internal`, never spun on.
                if executed > 0 && self.pending_exceptions.iter().any(|&w| w != 0) && !self.primask
                {
                    if let Some(exc) = self.highest_priority_pending() {
                        let exc_prio = self.exception_priority(exc);
                        let active_prio = self.exception_priority(self.active_exception);
                        if exc_prio < active_prio
                            && !self.masked_by_basepri(exc_prio)
                            && !self.faultmask_blocks(exc)
                        {
                            break;
                        }
                    }
                }
                if let Some(tap) = &tap {
                    tap.bump_clock();
                }
                self.step_internal(bus, observers, config)?;
                executed += 1;
                if config.idle_fast_forward_enabled && self.idle_fast_forward_budget(bus).is_some()
                {
                    break;
                }
            }
        }

        Ok(executed)
    }

    fn idle_fast_forward_budget(&self, _bus: &dyn Bus) -> Option<u64> {
        // Only fast-forward while the core sleeps in WFI and no wake-up event
        // has arrived. A pending wake exception (evaluated ignoring PRIMASK)
        // resumes normal execution: the machine must re-enter `step` so the
        // core either takes the exception or, under PRIMASK, falls through it.
        if !self.sleeping || self.wfi_wake_pending() {
            return None;
        }
        // Cortex-M has no core-local timer to bound the skip (SysTick lives on
        // the bus). Offer an unbounded budget; `Machine::run` clamps it to the
        // next scheduler deadline, exactly as it does for a RISC-V core whose
        // mtimecmp is disabled.
        Some(u64::MAX)
    }

    fn fast_forward_idle_cycles(&mut self, _cycles: u64) {
        // Cortex-M keeps no core-local cycle counter (unlike RISC-V mtime); the
        // machine owns `total_cycles` and advances it. Nothing to do here.
    }
}

impl CortexM {
    #[inline(always)]
    fn step_internal<B: Bus + ?Sized>(
        &mut self,
        bus: &mut B,
        _observers: &[Arc<dyn SimulationObserver>],
        config: &SimulationConfig,
    ) -> SimResult<()> {
        // Leave WFI sleep before this step commits: the flag is re-armed only if
        // this instruction is itself a WFI with no wake event pending.
        self.sleeping = false;
        // Check for pending exceptions before executing instruction.
        // Use real ARMv7-M priority dispatch: pick the highest-priority
        // pending exception (smallest numeric priority value), and only
        // take it if its priority is strictly higher than the currently
        // active exception's. This is the dispatch path that makes
        // FreeRTOS PendSV-driven context switches behave correctly —
        // PendSV at priority 0xFF only runs when no other ISR is active.
        let exception_num = self.highest_priority_pending().unwrap_or(0);
        if self.pending_exceptions.iter().any(|&w| w != 0) && !self.primask && exception_num != 0 {
            let take_prio = self.exception_priority(exception_num);
            let active_prio = self.exception_priority(self.active_exception);
            let can_take = take_prio < active_prio
                && !self.masked_by_basepri(take_prio)
                && !self.faultmask_blocks(exception_num);

            if can_take {
                // For NVIC-routed exceptions (num >= 16): verify the NVIC ISPR bit is
                // still set before taking the exception.  Firmware may have called
                // NVIC_ClearPendingIRQ (writing NVIC ICPR) while the ISR was active,
                // which clears ISPR but leaves our cpu-side `pending_exceptions` stale.
                // Without this check the stale bit causes a spurious second ISR after the
                // real one returns.  On real ARM Cortex-M the hardware never re-latches a
                // pending bit whose ISPR was cleared by software before ISR exit.
                if exception_num >= 16 && !bus.is_nvic_irq_pending(exception_num) {
                    // Stale pending_exceptions bit — drop it without taking the exception.
                    self.pending_exceptions[(exception_num / 64) as usize] &=
                        !(1u64 << (exception_num % 64));
                    // Fall through to normal instruction execution.
                } else {
                    self.pending_exceptions[(exception_num / 64) as usize] &=
                        !(1u64 << (exception_num % 64));

                    // Clear NVIC ISPR for this exception so it isn't immediately re-pended.
                    // On real ARM hardware this happens automatically when the exception is taken.
                    bus.clear_nvic_pending(exception_num);

                    // Capture the entry context BEFORE switching to Handler mode:
                    // which mode/stack we came from determines EXC_RETURN.
                    let entered_from_handler = self.active_exception != 0;
                    let entry_on_psp = self.use_psp();

                    // Perform Stacking on the CURRENT (preempted) stack.
                    let sp = self.sp;
                    let frame_ptr = sp.wrapping_sub(32);

                    // Save the previous active_exception in xPSR IPSR bits [8:0] so that
                    // exception_return can restore the correct nesting level.
                    let save_xpsr =
                        self.xpsr_with_itstate((self.xpsr & !0x1FF) | self.active_exception);

                    // Stack: R0, R1, R2, R3, R12, LR, PC, xPSR (with previous IPSR)
                    let stacked_lr = self.lr;
                    let stacked_pc = self.pc;
                    let _ = bus.write_u32(frame_ptr as u64, self.r0);
                    let _ = bus.write_u32((frame_ptr + 4) as u64, self.r1);
                    let _ = bus.write_u32((frame_ptr + 8) as u64, self.r2);
                    let _ = bus.write_u32((frame_ptr + 12) as u64, self.r3);
                    let _ = bus.write_u32((frame_ptr + 16) as u64, self.r12);
                    let _ = bus.write_u32((frame_ptr + 20) as u64, self.lr);
                    let _ = bus.write_u32((frame_ptr + 24) as u64, self.pc);
                    let _ = bus.write_u32((frame_ptr + 28) as u64, save_xpsr);

                    // Bank the preempted stack pointer into its bank (PSP or MSP)
                    // BEFORE entering Handler mode, then switch the live `sp` to MSP.
                    if entry_on_psp {
                        self.psp = frame_ptr;
                    } else {
                        self.msp = frame_ptr;
                    }

                    // Update active exception (→ Handler mode) so nested exceptions
                    // see the correct level. Handler always runs on MSP.
                    self.set_active_exception(exception_num);
                    self.sp = self.msp;
                    self.it_state = 0;

                    // EXC_RETURN encodes the mode/stack to restore on return:
                    //   0xFFFFFFF1 → return to Handler mode (nested), frame on MSP
                    //   0xFFFFFFF9 → return to Thread/MSP
                    //   0xFFFFFFFD → return to Thread/PSP
                    self.lr = if entered_from_handler {
                        0xFFFF_FFF1
                    } else if entry_on_psp {
                        0xFFFF_FFFD
                    } else {
                        0xFFFF_FFF9
                    };

                    // Jump to ISR handler
                    let vtor = self.vtor.load(Ordering::SeqCst);
                    let vector_addr = vtor + (exception_num * 4);
                    if std::env::var("LABWIRED_TRACE_EXC").is_ok() {
                        eprintln!(
                            "EXC take num={} vtor=0x{:08X} vec=0x{:08X} fetch={:?}",
                            exception_num,
                            vtor,
                            vector_addr,
                            bus.read_u32(vector_addr as u64)
                        );
                    }
                    if let Ok(handler) = bus.read_u32(vector_addr as u64) {
                        self.pc = handler & !1;
                        tracing::debug!(
                        "EXC_ENTRY: exc={} handler={:#010x} frame={:#010x} stacked_lr={:#010x} stacked_pc={:#010x}",
                        exception_num, self.pc, frame_ptr, stacked_lr, stacked_pc
                    );
                    }

                    return Ok(());
                } // end else (NVIC ISPR still set — take the exception)
            }
            // Can't take this exception right now (lower priority than active).
            // Fall through and execute the current instruction normally.
        }
        // Fetch/Decode with optional Cache
        let cache_idx = ((self.pc >> 1) & 0xFFF) as usize;
        let entry = if config.decode_cache_enabled {
            if let Some(e) = self.decode_cache[cache_idx] {
                if e.tag == self.pc {
                    Some(e)
                } else {
                    None
                }
            } else {
                None
            }
        } else {
            None
        };

        let (instruction, opcode, mut pc_increment, _cycles) = if let Some(e) = entry {
            (e.instruction, e.opcode, e.pc_increment as u32, e.cycles)
        } else {
            let fetch_pc = self.pc & !1;
            let h1 = bus.read_u16(fetch_pc as u64)?;
            let is_32bit = (h1 & 0xE000) == 0xE000 && (h1 & 0x1800) != 0;

            let (instr, op, pincr, cyc) = if is_32bit {
                let h2 = bus.read_u16((fetch_pc + 2) as u64)?;
                let instr = decode_thumb_32(h1, h2);
                let op = ((h1 as u32) << 16) | h2 as u32;
                (instr, op, 4, 2)
            } else {
                let instr = decode_thumb_16(h1);
                (instr, h1 as u32, 2, 1)
            };

            if config.decode_cache_enabled {
                self.decode_cache[cache_idx] = Some(DecodeCacheEntry {
                    tag: self.pc,
                    instruction: instr,
                    opcode: op,
                    pc_increment: pincr as u8,
                    cycles: cyc,
                });
            }

            (instr, op, pincr as u32, cyc)
        };

        // Per-instruction PC trace gated on LABWIRED_TRACE_INSN env var.
        // Use only for short runs — VERY chatty. Format suitable for grepping:
        //   INSN pc=0xPPPPPPPP op=0xOOOOOOOO
        if std::env::var("LABWIRED_TRACE_INSN").is_ok() {
            eprintln!("INSN pc=0x{:08X} op=0x{:08X}", self.pc, opcode);
        }

        let retired_pc = self.pc;
        if !_observers.is_empty() {
            for observer in _observers {
                observer.on_step_start(self.pc, opcode);
            }
        }

        let mut execute = true;
        let mut it_block_instruction = false;

        if self.it_state != 0 {
            it_block_instruction = true;
            let cond = self.it_state >> 4;
            execute = self.check_condition(cond);
        }

        if execute {
            #[cfg(debug_assertions)]
            tracing::debug!(
                "PC={:#x}, Opcode={:#04x}, Instr={:?}",
                self.pc,
                opcode,
                instruction
            );

            // Execute
            match instruction {
                Instruction::Bfi { rd, rn, lsb, width } => {
                    let src = self.read_reg(rn);
                    let dst = self.read_reg(rd);
                    let mask = if width == 32 {
                        !0
                    } else {
                        ((1u32.wrapping_shl(width as u32)).wrapping_sub(1)).wrapping_shl(lsb as u32)
                    };
                    let result = (dst & !mask) | ((src.wrapping_shl(lsb as u32)) & mask);
                    self.write_reg(rd, result);
                    pc_increment = 4;
                }
                Instruction::Bfc { rd, lsb, width } => {
                    let dst = self.read_reg(rd);
                    let mask = if width == 32 {
                        !0
                    } else {
                        ((1u32.wrapping_shl(width as u32)).wrapping_sub(1)).wrapping_shl(lsb as u32)
                    };
                    let result = dst & !mask;
                    self.write_reg(rd, result);
                    pc_increment = 4;
                }
                Instruction::Sbfx { rd, rn, lsb, width } => {
                    let src = self.read_reg(rn);
                    let width_mask = if width == 32 {
                        !0
                    } else {
                        (1u32.wrapping_shl(width as u32)).wrapping_sub(1)
                    };
                    let val = (src.wrapping_shr(lsb as u32)) & width_mask;
                    let result = if width == 32 {
                        val
                    } else {
                        let shift = 32 - width;
                        ((val.wrapping_shl(shift as u32)) as i32).wrapping_shr(shift as u32) as u32
                    };
                    self.write_reg(rd, result);
                    pc_increment = 4;
                }
                Instruction::Ubfx { rd, rn, lsb, width } => {
                    let src = self.read_reg(rn);
                    let width_mask = if width == 32 {
                        !0
                    } else {
                        (1u32.wrapping_shl(width as u32)).wrapping_sub(1)
                    };
                    let result = (src.wrapping_shr(lsb as u32)) & width_mask;
                    self.write_reg(rd, result);
                    pc_increment = 4;
                }
                Instruction::Clz { rd, rm } => {
                    let val = self.read_reg(rm);
                    let result = val.leading_zeros();
                    self.write_reg(rd, result);
                    pc_increment = 4;
                }
                Instruction::Rbit { rd, rm } => {
                    let val = self.read_reg(rm);
                    let result = val.reverse_bits();
                    self.write_reg(rd, result);
                    pc_increment = 4;
                }
                Instruction::SimdAddSub8 { rd, rn, rm, op } => {
                    // Per-byte parallel add/sub; each lane sets one APSR.GE bit.
                    // op: 0=SADD8 1=UADD8 2=SSUB8 3=USUB8 (ARMv7-M A7.7).
                    let n = self.read_reg(rn);
                    let m = self.read_reg(rm);
                    let mut result = 0u32;
                    let mut ge = 0u32;
                    for i in 0..4 {
                        let nb = ((n >> (i * 8)) & 0xFF) as i32;
                        let mb = ((m >> (i * 8)) & 0xFF) as i32;
                        let (byte, ge_bit) = match op {
                            0 => {
                                // SADD8: signed add, GE = sum >= 0
                                let s = (nb as i8 as i32) + (mb as i8 as i32);
                                ((s as u32) & 0xFF, s >= 0)
                            }
                            1 => {
                                // UADD8: unsigned add, GE = carry out (sum >= 0x100)
                                let s = nb + mb;
                                ((s as u32) & 0xFF, s >= 0x100)
                            }
                            2 => {
                                // SSUB8: signed sub, GE = diff >= 0
                                let d = (nb as i8 as i32) - (mb as i8 as i32);
                                ((d as u32) & 0xFF, d >= 0)
                            }
                            _ => {
                                // USUB8: unsigned sub, GE = no borrow (nb >= mb)
                                let d = nb - mb;
                                ((d as u32) & 0xFF, nb >= mb)
                            }
                        };
                        result |= byte << (i * 8);
                        if ge_bit {
                            ge |= 1 << i;
                        }
                    }
                    self.write_reg(rd, result);
                    self.set_ge(ge);
                    pc_increment = 4;
                }
                Instruction::Sel { rd, rn, rm } => {
                    // SEL: pick each byte from Rn if its GE bit is set, else Rm.
                    let n = self.read_reg(rn);
                    let m = self.read_reg(rm);
                    let ge = self.get_ge();
                    let mut result = 0u32;
                    for i in 0..4 {
                        let src = if (ge >> i) & 1 == 1 { n } else { m };
                        result |= ((src >> (i * 8)) & 0xFF) << (i * 8);
                    }
                    self.write_reg(rd, result);
                    pc_increment = 4;
                }
                Instruction::Sdiv { rd, rn, rm } => {
                    let n = self.read_reg(rn) as i32;
                    let m = self.read_reg(rm) as i32;
                    let result = if m == 0 {
                        0
                    } else if n == i32::MIN && m == -1 {
                        i32::MIN as u32
                    } else {
                        (n / m) as u32
                    };
                    self.write_reg(rd, result);
                    pc_increment = 4;
                }
                Instruction::Udiv { rd, rn, rm } => {
                    let n = self.read_reg(rn);
                    let m = self.read_reg(rm);
                    let result = n.checked_div(m).unwrap_or(0);
                    self.write_reg(rd, result);
                    pc_increment = 4;
                }
                Instruction::DataProc32 {
                    op,
                    rn,
                    rd,
                    rm,
                    imm5,
                    shift_type,
                    set_flags,
                } => {
                    let op2_raw = self.read_reg(rm);
                    let mut op2 = op2_raw;
                    match shift_type {
                        0 => op2 = op2.wrapping_shl(imm5 as u32), // LSL
                        1 => {
                            op2 = if imm5 == 0 {
                                0
                            } else {
                                op2.wrapping_shr(imm5 as u32)
                            }
                        } // LSR
                        2 => {
                            op2 = if imm5 == 0 {
                                if (op2 & 0x80000000) != 0 {
                                    0xFFFFFFFF
                                } else {
                                    0
                                }
                            } else {
                                ((op2 as i32) >> (imm5 as u32)) as u32
                            }
                        } // ASR
                        3 if imm5 != 0 => op2 = op2.rotate_right(imm5 as u32), // ROR
                        _ => {}
                    }
                    let op1 = self.read_reg(rn);
                    let carry_in = self.get_carry();
                    // (result, carry-out, overflow). For logical ops C/V are the
                    // preserved current flags; the barrel-shifter carry-out is not
                    // tracked here (NOTE: logical-op C reflects the prior C, not the
                    // shifter carry — only N/Z are meaningful for them). Arithmetic
                    // ops compute true NZCV via the shared add/sub-with-flags helpers.
                    let (result, c, v) = match op {
                        0x0 => (op1 & op2, carry_in, self.get_overflow()), // AND / TST
                        0x1 => (op1 & !op2, carry_in, self.get_overflow()), // BIC
                        0x2 => {
                            let r = if rn == 0xF { op2 } else { op1 | op2 };
                            (r, carry_in, self.get_overflow())
                        } // ORR / MOV
                        0x3 => {
                            let r = if rn == 0xF { !op2 } else { op1 | !op2 };
                            (r, carry_in, self.get_overflow())
                        } // ORN / MVN
                        0x4 => (op1 ^ op2, carry_in, self.get_overflow()), // EOR / TEQ
                        0x6 => {
                            // PKH (PKHBT/PKHTB): pack halfwords. The tb bit lives in
                            // shift_type bit1 (0 => PKHBT keep op1 low / Rm high,
                            // 2 => PKHTB keep op1 high / Rm low). The optional barrel
                            // shift on Rm is not applied here (PKH is off the bignum
                            // path and only the imm5==0 form is exercised); operands
                            // come from the raw Rm. Not flag-setting.
                            let r = if shift_type == 2 {
                                (op1 & 0xFFFF_0000) | (op2_raw & 0x0000_FFFF)
                            } else {
                                (op1 & 0x0000_FFFF) | (op2_raw & 0xFFFF_0000)
                            };
                            (r, carry_in, self.get_overflow())
                        } // PKH
                        0x8 => add_with_flags(op1, op2),                   // ADD / CMN
                        0xA => adc_with_flags(op1, op2, carry_in as u32),  // ADC
                        0xB => sbc_with_flags(op1, op2, carry_in as u32),  // SBC
                        0xD => sub_with_flags(op1, op2),                   // SUB / CMP
                        0xE => sub_with_flags(op2, op1),                   // RSB (op2 - op1)
                        _ => {
                            #[cfg(debug_assertions)]
                            tracing::warn!("Unknown DataProc32 op {:#x}", op);
                            (op2, carry_in, self.get_overflow())
                        }
                    };
                    if rd != 15 {
                        self.write_reg(rd, result);
                    }
                    if set_flags {
                        self.update_nzcv(result, c, v);
                    }
                    pc_increment = 4;
                }
                Instruction::DataProcImm32 {
                    op,
                    rn,
                    rd,
                    imm12,
                    set_flags,
                } => {
                    let imm = thumb_expand_imm(imm12);
                    let val1 = self.read_reg(rn);
                    let (res, c, v) = match op {
                        0x0 => (val1 & imm, self.get_carry(), self.get_overflow()), // AND
                        0x1 => (val1 & !imm, self.get_carry(), self.get_overflow()), // BIC
                        0x2 => {
                            let res = if rn == 0xF { imm } else { val1 | imm };
                            (res, self.get_carry(), self.get_overflow())
                        } // ORR / MOV
                        0x3 => {
                            let res = if rn == 0xF { !imm } else { val1 | !imm };
                            (res, self.get_carry(), self.get_overflow())
                        } // ORN / MVN
                        0x4 => (val1 ^ imm, self.get_carry(), self.get_overflow()), // EOR
                        0x8 => add_with_flags(val1, imm),                           // ADD
                        0xA => adc_with_flags(val1, imm, self.get_carry() as u32),  // ADC
                        0xB => sbc_with_flags(val1, imm, self.get_carry() as u32),  // SBC
                        0xD => sub_with_flags(val1, imm),                           // SUB / CMP
                        0xE => sub_with_flags(imm, val1),                           // RSB
                        _ => {
                            tracing::warn!("Unhandled T32 DataProcImm32 op {:#x}", op);
                            (0, self.get_carry(), self.get_overflow())
                        }
                    };

                    if rd != 15 {
                        self.write_reg(rd, res);
                    }
                    if set_flags {
                        self.update_nzcv(res, c, v);
                    }
                    pc_increment = 4;
                }
                Instruction::ShiftReg32 {
                    rd,
                    rn,
                    rm,
                    shift_type,
                } => {
                    let value = self.read_reg(rn);
                    let shift = self.read_reg(rm) & 0xFF;
                    let result = match shift_type {
                        0 => {
                            if shift >= 32 {
                                0
                            } else {
                                value.wrapping_shl(shift)
                            }
                        }
                        1 => {
                            if shift == 0 {
                                value
                            } else if shift >= 32 {
                                0
                            } else {
                                value.wrapping_shr(shift)
                            }
                        }
                        2 => {
                            if shift == 0 {
                                value
                            } else if shift >= 32 {
                                if (value & 0x8000_0000) != 0 {
                                    0xFFFF_FFFF
                                } else {
                                    0
                                }
                            } else {
                                ((value as i32) >> shift) as u32
                            }
                        }
                        3 => {
                            if shift == 0 {
                                value
                            } else {
                                value.rotate_right(shift % 32)
                            }
                        }
                        _ => value,
                    };
                    self.write_reg(rd, result);
                    pc_increment = 4;
                }
                Instruction::Movw { rd, imm } => {
                    self.write_reg(rd, imm as u32);
                    pc_increment = 4;
                }
                Instruction::Movt { rd, imm } => {
                    let old_val = self.read_reg(rd);
                    let new_val = (old_val & 0x0000FFFF) | ((imm as u32) << 16);
                    self.write_reg(rd, new_val);
                    pc_increment = 4;
                }
                Instruction::LdrImm32 { rt, rn, imm12 } => {
                    // When rn==PC, ARM spec requires Align(PC+4, 4) as base (literal load)
                    let base = if rn == 15 {
                        (self.pc.wrapping_add(4)) & !3
                    } else {
                        self.read_reg(rn)
                    };
                    let addr = base.wrapping_add(imm12 as u32);
                    if let Ok(val) = bus.read_u32(addr as u64) {
                        if rt == 15 {
                            // LDR PC, [...] is an interworking branch — must go through branch_to
                            self.branch_to(val, bus)?;
                            pc_increment = 0;
                        } else {
                            self.write_reg(rt, val);
                        }
                    }
                    // pc_increment stays at 4 (set by decode) unless we took a branch above
                }
                Instruction::StrImm32 { rt, rn, imm12 } => {
                    let base = self.read_reg(rn);
                    let addr = base.wrapping_add(imm12 as u32);
                    let val = self.read_reg(rt);
                    let _ = bus.write_u32(addr as u64, val);
                    pc_increment = 4;
                }
                Instruction::LdrImm32Idx {
                    rt,
                    rn,
                    imm8,
                    pre_index,
                    add,
                    writeback,
                } => {
                    // LDR T4 indexed. Offset address = base ± imm8. The access
                    // uses the offset address when pre_index, else the base
                    // (post-index). Writeback stores the offset address in Rn.
                    let base = self.read_reg(rn);
                    let offset = imm8 as u32;
                    let offset_addr = if add {
                        base.wrapping_add(offset)
                    } else {
                        base.wrapping_sub(offset)
                    };
                    let access_addr = if pre_index { offset_addr } else { base };
                    if let Ok(val) = bus.read_u32(access_addr as u64) {
                        // Commit writeback before branching so a load-to-PC
                        // (function return) leaves Rn=SP correct.
                        if writeback {
                            self.write_reg(rn, offset_addr);
                        }
                        if rt == 15 {
                            // LDR PC, [...] — interworking branch (function return).
                            self.branch_to(val, bus)?;
                            pc_increment = 0;
                        } else {
                            self.write_reg(rt, val);
                            pc_increment = 4;
                        }
                    } else {
                        pc_increment = 4;
                    }
                }
                Instruction::StrImm32Idx {
                    rt,
                    rn,
                    imm8,
                    pre_index,
                    add,
                    writeback,
                } => {
                    let base = self.read_reg(rn);
                    let offset = imm8 as u32;
                    let offset_addr = if add {
                        base.wrapping_add(offset)
                    } else {
                        base.wrapping_sub(offset)
                    };
                    let access_addr = if pre_index { offset_addr } else { base };
                    let val = self.read_reg(rt);
                    let _ = bus.write_u32(access_addr as u64, val);
                    if writeback {
                        self.write_reg(rn, offset_addr);
                    }
                    pc_increment = 4;
                }
                Instruction::Ldrd {
                    rt,
                    rt2,
                    rn,
                    imm8,
                    add_imm,
                    index,
                    writeback,
                } => {
                    // ARMv8-M LDRD (immediate): offset_addr = Rn ± imm32;
                    // access_addr = index ? offset_addr : Rn; if writeback,
                    // Rn = offset_addr.
                    let base = self.read_reg(rn);
                    let offset_addr = if add_imm {
                        base.wrapping_add(imm8 << 2)
                    } else {
                        base.wrapping_sub(imm8 << 2)
                    };
                    let addr = if index { offset_addr } else { base };
                    if let Ok(v1) = bus.read_u32(addr as u64) {
                        self.write_reg(rt, v1);
                    }
                    if let Ok(v2) = bus.read_u32(addr.wrapping_add(4) as u64) {
                        self.write_reg(rt2, v2);
                    }
                    if writeback {
                        self.write_reg(rn, offset_addr);
                    }
                    pc_increment = 4;
                }
                Instruction::Strd {
                    rt,
                    rt2,
                    rn,
                    imm8,
                    add_imm,
                    index,
                    writeback,
                } => {
                    let base = self.read_reg(rn);
                    let offset_addr = if add_imm {
                        base.wrapping_add(imm8 << 2)
                    } else {
                        base.wrapping_sub(imm8 << 2)
                    };
                    let addr = if index { offset_addr } else { base };
                    let v1 = self.read_reg(rt);
                    let v2 = self.read_reg(rt2);
                    let _ = bus.write_u32(addr as u64, v1);
                    let _ = bus.write_u32(addr.wrapping_add(4) as u64, v2);
                    if writeback {
                        self.write_reg(rn, offset_addr);
                    }
                    pc_increment = 4;
                }
                Instruction::Tbb { rn, rm } => {
                    let mut base = self.read_reg(rn);
                    if rn == 15 {
                        // ARMv7-M: the table base is the in-execution PC
                        // (insn address + 4) — NOT word-aligned. The old
                        // Align(PC,4) read the table 2 bytes early whenever
                        // the TBB sat at a 2-mod-4 address; ST's
                        // HAL_DMA_RegisterCallback dispatched every
                        // callback ID into the same slot because of it.
                        base = self.pc.wrapping_add(4);
                    }
                    let index = self.read_reg(rm);
                    let addr = base.wrapping_add(index);
                    if let Ok(byte) = bus.read_u8(addr as u64) {
                        let offset = (byte as u32) << 1;
                        self.pc = self.pc.wrapping_add(4).wrapping_add(offset);
                        pc_increment = 0;
                    }
                }
                Instruction::Tbh { rn, rm } => {
                    let mut base = self.read_reg(rn);
                    if rn == 15 {
                        // Same unaligned PC+4 base rule as TBB above.
                        base = self.pc.wrapping_add(4);
                    }
                    let index = self.read_reg(rm);
                    let addr = base.wrapping_add(index << 1);
                    if let Ok(halfword) = bus.read_u16(addr as u64) {
                        let offset = (halfword as u32) << 1;
                        self.pc = self.pc.wrapping_add(4).wrapping_add(offset);
                        pc_increment = 0;
                    }
                }
                Instruction::Unknown32(h1, h2) => {
                    // Manual fallback for complex bit patterns not yet in Instruction enum.
                    //
                    // LDREX Rt, [Rn, #imm8*4]: T1 = 0xE85F_TFTT where
                    //   h1 = 0xE85_ | Rn, h2 = (Rt << 12) | 0xF00 | imm8
                    // STREX Rd, Rt, [Rn, #imm8*4]: T1 = 0xE840_TDII where
                    //   h1 = 0xE84_ | Rn, h2 = (Rt << 12) | (Rd << 8) | imm8
                    //
                    // Single-threaded sim has no preemption between LDREX and
                    // STREX, so we model the exclusive monitor as always
                    // succeeding. This matches the observable behavior of
                    // atomic ops on real hardware in the uncontended case.
                    if (h1 & 0xFFF0) == 0xE850 {
                        // LDREX
                        let rn = (h1 & 0xF) as u8;
                        let rt = ((h2 >> 12) & 0xF) as u8;
                        let imm8 = (h2 & 0xFF) as u32;
                        let addr = self.get_register(rn).wrapping_add(imm8 * 4);
                        if let Ok(val) = bus.read_u32(addr as u64) {
                            self.set_register(rt, val);
                        }
                        pc_increment = 4;
                    } else if (h1 & 0xFFF0) == 0xE840 {
                        // STREX
                        let rn = (h1 & 0xF) as u8;
                        let rt = ((h2 >> 12) & 0xF) as u8;
                        let rd = ((h2 >> 8) & 0xF) as u8;
                        let imm8 = (h2 & 0xFF) as u32;
                        let addr = self.get_register(rn).wrapping_add(imm8 * 4);
                        let val = self.get_register(rt);
                        let _ = bus.write_u32(addr as u64, val);
                        // Rd = 0 → success.
                        self.set_register(rd, 0);
                        pc_increment = 4;
                    } else if (h1 & 0xFFF0) == 0xE8D0 && (h2 & 0x0F0F) == 0x0F0F {
                        // Load-acquire family (ARMv8-M mainline, also on the
                        // M33 with TrustZone off): LDAB/LDAH/LDA and
                        // LDAEXB/LDAEXH/LDAEX.
                        //   h1 = 0xE8D0 | Rn, h2 = Rt<<12 | 0xF<<8 | sz<<4 | 0xF
                        //   sz: 8=B, 9=H, A=word (acquire), C=EXB, D=EXH, E=EX
                        // Acquire ordering and the exclusive monitor are
                        // no-ops in this single-threaded sim (same rationale
                        // as LDREX above). Rust atomics on thumbv8m compile
                        // to these — embassy's executor run-queue lives on
                        // LDAEX/STLEX.
                        let rn = (h1 & 0xF) as u8;
                        let rt = ((h2 >> 12) & 0xF) as u8;
                        let addr = self.get_register(rn) as u64;
                        let loaded = match (h2 >> 4) & 0xF {
                            0x8 | 0xC => bus.read_u8(addr).ok().map(|v| v as u32),
                            0x9 | 0xD => bus.read_u16(addr).ok().map(|v| v as u32),
                            0xA | 0xE => bus.read_u32(addr).ok(),
                            _ => None,
                        };
                        if let Some(val) = loaded {
                            self.set_register(rt, val);
                        }
                        pc_increment = 4;
                    } else if (h1 & 0xFFF0) == 0xE8C0 && (h2 & 0x0F00) == 0x0F00 {
                        // Store-release family: STLB/STLH/STL ([3:0]=0xF, no
                        // status register) and STLEXB/STLEXH/STLEX ([3:0]=Rd,
                        // always-success monitor → Rd = 0).
                        //   h1 = 0xE8C0 | Rn, h2 = Rt<<12 | 0xF<<8 | sz<<4 | Rd/0xF
                        let rn = (h1 & 0xF) as u8;
                        let rt = ((h2 >> 12) & 0xF) as u8;
                        let addr = self.get_register(rn) as u64;
                        let val = self.get_register(rt);
                        let sz = (h2 >> 4) & 0xF;
                        match sz {
                            0x8 | 0xC => {
                                let _ = bus.write_u8(addr, val as u8);
                            }
                            0x9 | 0xD => {
                                let _ = bus.write_u16(addr, val as u16);
                            }
                            0xA | 0xE => {
                                let _ = bus.write_u32(addr, val);
                            }
                            _ => {}
                        }
                        if matches!(sz, 0xC..=0xE) {
                            let rd = (h2 & 0xF) as u8;
                            self.set_register(rd, 0); // success
                        }
                        pc_increment = 4;
                    } else if (h1 & 0xFE00) == 0xE800 {
                        // Table branch, load/store multiple etc — not yet
                        // modeled in full; advance past the 32-bit insn.
                        pc_increment = 4;
                    } else if (h1 & 0xFE00) == 0xF800 {
                        // LDR/STR (immediate) T3/T4
                        let op1 = (h1 >> 4) & 0xF;
                        let rn = (h1 & 0xF) as u8;
                        let rt = ((h2 >> 12) & 0xF) as u8;
                        let is_t4 = (op1 & 0x8) == 0;
                        // Signed (LDRSB.W/LDRSH.W) vs unsigned (LDRB.W/LDRH.W)
                        // is selected by h1 bit 8 (0x0100), NOT op1 bit 3:
                        // op1 = h1[7:4] excludes bit 8, and op1 bit 3 (=h1 bit 7)
                        // is the imm12-form selector used for is_t4 above. Using
                        // it for the sign made LDRB.W T2 (0xF89x) sign-extend any
                        // byte >= 0x80 (0x85 -> 0xFFFFFF85).
                        let is_signed = (h1 & 0x0100) != 0;
                        // When Rn=PC (rn==15), T4 form is always the PC-literal encoding
                        // (LDR.W Rt, [PC, ±imm12]), never register-offset.
                        let is_reg_offset = is_t4 && rn != 15 && (h2 & 0x0800) == 0;
                        if !is_reg_offset {
                            let mut supported = true;
                            let addr: u32;
                            let mut wb = false;
                            let mut wb_val = 0u32;
                            if !is_t4 {
                                // T3: Rn-relative with unsigned imm12
                                let base = if rn == 15 {
                                    // PC-literal T3: Align(PC+4, 4) + imm12
                                    (self.pc.wrapping_add(4)) & !3
                                } else {
                                    self.read_reg(rn)
                                };
                                let offset = (h2 & 0xFFF) as u32;
                                addr = base.wrapping_add(offset);
                            } else if rn == 15 {
                                // PC-literal T2: LDR.W Rt, [PC, ±imm12]
                                // U bit is bit 7 of h1; imm12 is h2[11:0]
                                let imm12 = (h2 & 0xFFF) as u32;
                                let u = (h1 >> 7) & 1;
                                let base = (self.pc.wrapping_add(4)) & !3;
                                addr = if u != 0 {
                                    base.wrapping_add(imm12)
                                } else {
                                    base.wrapping_sub(imm12)
                                };
                            } else {
                                let p = (h2 >> 10) & 1;
                                let u = (h2 >> 9) & 1;
                                let w = (h2 >> 8) & 1;
                                let imm8 = (h2 & 0xFF) as i32;
                                let offset = if u != 0 { imm8 } else { -imm8 };
                                let base = self.read_reg(rn);
                                if p != 0 {
                                    addr = base.wrapping_add(offset as u32);
                                    if w != 0 {
                                        wb = true;
                                        wb_val = addr;
                                    }
                                } else {
                                    addr = base;
                                    wb = true;
                                    wb_val = base.wrapping_add(offset as u32);
                                }
                            }
                            let mut branch_taken = false;
                            match op1 & 0x7 {
                                0 => {
                                    let val = (self.read_reg(rt) & 0xFF) as u8;
                                    let _ = bus.write_u8(addr as u64, val);
                                }
                                // Rt==15 = PLD/PLI preload hint — NOP (handled by `_`).
                                1 if rt != 15 => {
                                    if let Ok(v) = bus.read_u8(addr as u64) {
                                        let out = if is_signed {
                                            (v as i8) as i32 as u32
                                        } else {
                                            v as u32
                                        };
                                        self.write_reg(rt, out);
                                    }
                                }
                                2 => {
                                    let val = (self.read_reg(rt) & 0xFFFF) as u16;
                                    let _ = bus.write_u16(addr as u64, val);
                                }
                                // Rt==15 = PLDW preload hint — NOP (handled by `_`).
                                3 if rt != 15 => {
                                    if let Ok(v) = bus.read_u16(addr as u64) {
                                        let out = if is_signed {
                                            (v as i16) as i32 as u32
                                        } else {
                                            v as u32
                                        };
                                        self.write_reg(rt, out);
                                    }
                                }
                                4 => {
                                    let val = self.read_reg(rt);
                                    let _ = bus.write_u32(addr as u64, val);
                                }
                                5 => {
                                    if let Ok(v) = bus.read_u32(addr as u64) {
                                        if rt == 15 {
                                            if wb {
                                                self.write_reg(rn, wb_val);
                                                wb = false;
                                            }
                                            self.branch_to(v, bus)?;
                                            branch_taken = true;
                                        } else {
                                            self.write_reg(rt, v);
                                        }
                                    }
                                }
                                _ => {
                                    supported = false;
                                }
                            }
                            if supported {
                                if wb {
                                    self.write_reg(rn, wb_val);
                                }
                                // Rt==15 load is a branch; suppress pc_increment
                                // (see the register-offset path below).
                                if branch_taken {
                                    pc_increment = 0;
                                } else {
                                    pc_increment = 4;
                                }
                            }
                        } else {
                            // Register offset (T2)
                            let rn = (h1 & 0xF) as u8;
                            let rt = ((h2 >> 12) & 0xF) as u8;
                            let rm = (h2 & 0xF) as u8;
                            let imm2 = ((h2 >> 4) & 0x3) as u8;
                            let base = self.read_reg(rn);
                            let offset = self.read_reg(rm).wrapping_shl(imm2 as u32);
                            let addr = base.wrapping_add(offset);
                            let mut branch_taken = false;
                            match op1 & 0x7 {
                                0 => {
                                    let val = (self.read_reg(rt) & 0xFF) as u8;
                                    let _ = bus.write_u8(addr as u64, val);
                                }
                                // Rt==15 = PLD/PLI preload hint — NOP (handled by `_`).
                                1 if rt != 15 => {
                                    if let Ok(v) = bus.read_u8(addr as u64) {
                                        let out = if is_signed {
                                            (v as i8) as i32 as u32
                                        } else {
                                            v as u32
                                        };
                                        self.write_reg(rt, out);
                                    }
                                }
                                2 => {
                                    let val = (self.read_reg(rt) & 0xFFFF) as u16;
                                    let _ = bus.write_u16(addr as u64, val);
                                }
                                // Rt==15 = PLDW preload hint — NOP (handled by `_`).
                                3 if rt != 15 => {
                                    if let Ok(v) = bus.read_u16(addr as u64) {
                                        let out = if is_signed {
                                            (v as i16) as i32 as u32
                                        } else {
                                            v as u32
                                        };
                                        self.write_reg(rt, out);
                                    }
                                }
                                4 => {
                                    let val = self.read_reg(rt);
                                    let _ = bus.write_u32(addr as u64, val);
                                }
                                5 => {
                                    if let Ok(v) = bus.read_u32(addr as u64) {
                                        if rt == 15 {
                                            self.branch_to(v, bus)?;
                                            branch_taken = true;
                                        } else {
                                            self.write_reg(rt, v);
                                        }
                                    }
                                }
                                _ => {}
                            }
                            // A load into PC (Rt==15) is a branch: branch_to
                            // already set PC, so the 32-bit pc_increment must be
                            // suppressed (same contract as Bx). Leaving it at 4
                            // landed PC one halfword past the target — this broke
                            // GCC switch jump tables (`ldr.w pc,[rn,rm,lsl#n]`).
                            if branch_taken {
                                pc_increment = 0;
                            } else {
                                pc_increment = 4;
                            }
                        }
                    } else if (h1 & 0xFB00) == 0xF000 && (h2 & 0x8000) == 0 {
                        // Data-processing (modified immediate) - repeated here for safety but usually handled by DataProcImm32
                        let i = (h1 >> 10) & 0x1;
                        let op = ((h1 >> 5) & 0xF) as u8;
                        let s = ((h1 >> 4) & 0x1) != 0;
                        let rn = (h1 & 0xF) as u8;
                        let imm3 = (h2 >> 12) & 0x7;
                        let rd = ((h2 >> 8) & 0xF) as u8;
                        let imm8 = h2 & 0xFF;
                        let imm12 = (i << 11) | (imm3 << 8) | imm8;
                        let imm32 = thumb_expand_imm(imm12 as u32);
                        let op1 = self.read_reg(rn);
                        let mut result = 0u32;
                        let mut update_rd = true;
                        match op {
                            0x0 => result = op1 & imm32,                                  // AND
                            0x1 => result = op1 & !imm32,                                 // BIC
                            0x2 => result = if rn == 15 { imm32 } else { op1 | imm32 },   // ORR/MOV
                            0x3 => result = if rn == 15 { !imm32 } else { op1 | !imm32 }, // ORN/MVN
                            0x4 => result = op1 ^ imm32,                                  // EOR
                            0x8 => result = op1.wrapping_add(imm32),                      // ADD
                            0xD => result = op1.wrapping_sub(imm32),                      // SUB
                            _ => update_rd = false,
                        }
                        if update_rd {
                            if rd != 15 {
                                self.write_reg(rd, result);
                            }
                            if s {
                                self.update_nz(result);
                            }
                            pc_increment = 4;
                        }
                    } else if (h1 & 0xFB00) == 0xF100 && (h2 & 0x8000) == 0 {
                        // Data-processing (plain binary immediate)
                        let i = (h1 >> 10) & 0x1;
                        let op = ((h1 >> 5) & 0xF) as u8;
                        let rn = (h1 & 0xF) as u8;
                        let imm3 = (h2 >> 12) & 0x7;
                        let rd = ((h2 >> 8) & 0xF) as u8;
                        let imm8 = h2 & 0xFF;
                        let imm12 = (i << 11) | (imm3 << 8) | imm8;
                        let op1 = self.read_reg(rn);
                        match op {
                            0x0 => {
                                self.write_reg(rd, op1.wrapping_add(imm12 as u32));
                                pc_increment = 4;
                            } // ADD
                            0xA => {
                                self.write_reg(rd, op1.wrapping_sub(imm12 as u32));
                                pc_increment = 4;
                            } // SUB
                            _ => {}
                        }
                    } else if (h1 & 0xF000) == 0xF000 && (h2 & 0x8000) == 0x8000 {
                        // B.W / BL (handled elsewhere but just in case)
                        pc_increment = 4;
                    } else {
                        tracing::warn!(
                            "Unknown 32-bit instruction at {:#x}: {:#x} {:#x}",
                            self.pc,
                            h1,
                            h2
                        );
                        crate::fidelity::record_undecoded(
                            self.pc,
                            ((h1 as u64) << 16) | (h2 as u64),
                            "undecoded T32",
                        );
                        pc_increment = 4;
                    }
                }

                Instruction::Nop => { /* Do nothing */ }
                Instruction::Wfi => {
                    // ARMv7-M WFI: complete as a NOP if a wake-up event is
                    // already pending, otherwise suspend the core until one
                    // arrives. Wake-up ignores PRIMASK (see `wfi_wake_pending`);
                    // `Machine::run` fast-forwards the idle window while
                    // `self.sleeping` holds. PC has already advanced past the
                    // WFI like any other 16-bit hint.
                    if !self.wfi_wake_pending() {
                        self.sleeping = true;
                    }
                }
                Instruction::MovImm { rd, imm } => {
                    self.write_reg(rd, imm as u32);
                    if !it_block_instruction {
                        self.update_nz(imm as u32);
                    }
                }
                // Control Flow
                Instruction::Cbz { rn, imm } => {
                    if self.read_reg(rn) == 0 {
                        self.pc = self.pc.wrapping_add(4).wrapping_add(imm as u32);
                        pc_increment = 0;
                    }
                }
                Instruction::Cbnz { rn, imm } => {
                    if self.read_reg(rn) != 0 {
                        self.pc = self.pc.wrapping_add(4).wrapping_add(imm as u32);
                        pc_increment = 0;
                    }
                }
                Instruction::Branch { offset } => {
                    let target = (self.pc as i32 + 4 + offset) as u32;
                    self.pc = target;
                    pc_increment = 0;
                }
                // Arithmetic
                Instruction::AddReg { rd, rn, rm } => {
                    let op1 = self.read_reg(rn);
                    let op2 = self.read_reg(rm);
                    let (res, c, v) = add_with_flags(op1, op2);
                    self.write_reg(rd, res);
                    if !it_block_instruction {
                        self.update_nzcv(res, c, v);
                    }
                }
                Instruction::AddImm3 { rd, rn, imm } => {
                    let op1 = self.read_reg(rn);
                    let (res, c, v) = add_with_flags(op1, imm as u32);
                    self.write_reg(rd, res);
                    if !it_block_instruction {
                        self.update_nzcv(res, c, v);
                    }
                }
                Instruction::AddImm8 { rd, imm } => {
                    let op1 = self.read_reg(rd);
                    let (res, c, v) = add_with_flags(op1, imm as u32);
                    self.write_reg(rd, res);
                    if !it_block_instruction {
                        self.update_nzcv(res, c, v);
                    }
                }
                Instruction::SubReg { rd, rn, rm } => {
                    let op1 = self.read_reg(rn);
                    let op2 = self.read_reg(rm);
                    let (res, c, v) = sub_with_flags(op1, op2);
                    self.write_reg(rd, res);
                    if !it_block_instruction {
                        self.update_nzcv(res, c, v);
                    }
                }
                Instruction::SubImm3 { rd, rn, imm } => {
                    let op1 = self.read_reg(rn);
                    let (res, c, v) = sub_with_flags(op1, imm as u32);
                    self.write_reg(rd, res);
                    if !it_block_instruction {
                        self.update_nzcv(res, c, v);
                    }
                }
                Instruction::SubImm8 { rd, imm } => {
                    let op1 = self.read_reg(rd);
                    let (res, c, v) = sub_with_flags(op1, imm as u32);
                    self.write_reg(rd, res);
                    if !it_block_instruction {
                        self.update_nzcv(res, c, v);
                    }
                }
                Instruction::AddSp { imm } => {
                    let sp = self.read_reg(13).wrapping_add(imm as u32);
                    self.write_reg(13, sp);
                }
                Instruction::SubSp { imm } => {
                    let sp = self.read_reg(13).wrapping_sub(imm as u32);
                    self.write_reg(13, sp);
                }

                Instruction::Uxtb { rd, rm } => {
                    let val = self.read_reg(rm);
                    self.write_reg(rd, val & 0xFF);
                }
                Instruction::Uxth { rd, rm } => {
                    let val = self.read_reg(rm);
                    self.write_reg(rd, val & 0xFFFF);
                }
                Instruction::Sxtb { rd, rm } => {
                    let val = self.read_reg(rm) as u8 as i8 as i32 as u32;
                    self.write_reg(rd, val);
                }
                Instruction::Sxth { rd, rm } => {
                    let val = self.read_reg(rm) as u16 as i16 as i32 as u32;
                    self.write_reg(rd, val);
                }
                Instruction::ExtendW {
                    rd,
                    rn,
                    rm,
                    rotate,
                    op,
                } => {
                    // ROR Rm by `rotate` (0/8/16/24), then extract+extend.
                    let v = self.read_reg(rm).rotate_right(rotate as u32);
                    let ext = match op {
                        0b000 => v as u16 as i16 as i32 as u32, // S*XTH
                        0b001 => v & 0xFFFF,                    // U*XTH
                        0b100 => v as u8 as i8 as i32 as u32,   // S*XTB
                        _ => v & 0xFF,                          // U*XTB (0b101)
                    };
                    // Extend-and-add variants (Rn != 0xF) add Rn; the plain
                    // extends encode Rn = 0xF.
                    let out = if rn == 0xF {
                        ext
                    } else {
                        self.read_reg(rn).wrapping_add(ext)
                    };
                    self.write_reg(rd, out);
                }

                Instruction::It { cond, mask } => {
                    self.it_state = (cond << 4) | mask;
                    it_block_instruction = false; // The IT instruction itself doesn't count towards the block's instructions
                }
                Instruction::AddRegHigh { rd, rm } => {
                    let val1 = self.read_reg(rd);
                    let val2 = self.read_reg(rm);
                    self.write_reg(rd, val1.wrapping_add(val2));
                }
                Instruction::CmpImm { rn, imm } => {
                    let op1 = self.read_reg(rn);
                    let (res, c, v) = sub_with_flags(op1, imm as u32);
                    self.update_nzcv(res, c, v);
                }
                Instruction::CmpReg { rn, rm } => {
                    let op1 = self.read_reg(rn);
                    let op2 = self.read_reg(rm);
                    let (res, c, v) = sub_with_flags(op1, op2);
                    self.update_nzcv(res, c, v);
                }
                Instruction::MovReg { rd, rm } => {
                    let val = self.read_reg(rm);
                    self.write_reg(rd, val);
                }
                // Logic
                Instruction::And { rd, rm } => {
                    let res = self.read_reg(rd) & self.read_reg(rm);
                    self.write_reg(rd, res);
                    self.update_nz(res);
                }
                Instruction::Bic { rd, rm } => {
                    let res = self.read_reg(rd) & !self.read_reg(rm);
                    self.write_reg(rd, res);
                    self.update_nz(res);
                }
                Instruction::Orr { rd, rm } => {
                    let res = self.read_reg(rd) | self.read_reg(rm);
                    self.write_reg(rd, res);
                    self.update_nz(res);
                }
                Instruction::Eor { rd, rm } => {
                    let res = self.read_reg(rd) ^ self.read_reg(rm);
                    self.write_reg(rd, res);
                    self.update_nz(res);
                }
                Instruction::Mvn { rd, rm } => {
                    let res = !self.read_reg(rm);
                    self.write_reg(rd, res);
                    self.update_nz(res);
                }
                Instruction::Mul { rd, rn } => {
                    let op1 = self.read_reg(rd);
                    let op2 = self.read_reg(rn);
                    let res = op1.wrapping_mul(op2);
                    self.write_reg(rd, res);
                    self.update_nz(res);
                }
                Instruction::Mul32 { rd, rn, rm } => {
                    let op1 = self.read_reg(rn);
                    let op2 = self.read_reg(rm);
                    let res = op1.wrapping_mul(op2);
                    self.write_reg(rd, res);
                    pc_increment = 4;
                }

                Instruction::Cpsie { primask, faultmask } => {
                    if primask {
                        self.primask = false;
                    }
                    if faultmask {
                        self.faultmask = false;
                    }
                }
                Instruction::Cpsid { primask, faultmask } => {
                    if primask {
                        self.primask = true;
                    }
                    if faultmask {
                        self.faultmask = true;
                    }
                }

                // Shifts
                Instruction::Lsl { rd, rm, imm } => {
                    let val = self.read_reg(rm);
                    let res = val.wrapping_shl(imm as u32);
                    self.write_reg(rd, res);
                    // T1 shift-immediate: setflags = !InITBlock(). Inside an
                    // IT block this encoding is the flag-preserving LSL, and
                    // leaking flags here would corrupt the remaining block
                    // conditions (Tier-1 H563/WBA52 gpio-check regression).
                    if !it_block_instruction {
                        // LSL #n (n>0) sets C to the last bit shifted out:
                        // Rm[32-n]. LSL #0 is a move and leaves C unchanged.
                        // (Verified against STM32F103 silicon via thumb_oracles.)
                        if imm == 0 {
                            self.update_nz(res);
                        } else {
                            let carry = (val >> (32 - imm as u32)) & 1 == 1;
                            self.update_nzcv(res, carry, self.get_overflow());
                        }
                    }
                }
                Instruction::Lsr { rd, rm, imm } => {
                    let val = self.read_reg(rm);
                    // Thumb T1: imm5 == 0 encodes a shift of 32.
                    let n = if imm == 0 { 32 } else { imm as u32 };
                    let res = if n >= 32 { 0 } else { val.wrapping_shr(n) };
                    self.write_reg(rd, res);
                    // T1 shift-immediate: setflags = !InITBlock(). LSR #n sets C
                    // to Rm[n-1], the last bit shifted out (silicon-verified).
                    if !it_block_instruction {
                        let carry = (val >> (n - 1)) & 1 == 1;
                        self.update_nzcv(res, carry, self.get_overflow());
                    }
                }
                Instruction::Asr { rd, rm, imm } => {
                    let val = self.read_reg(rm);
                    // Thumb T1: imm5 == 0 encodes a shift of 32.
                    let n = if imm == 0 { 32 } else { imm as u32 };
                    let res = ((val as i32) >> n.min(31)) as u32;
                    self.write_reg(rd, res);
                    // T1 shift-immediate: setflags = !InITBlock(). ASR #n sets C
                    // to Rm[n-1], the last bit shifted out (silicon-verified).
                    if !it_block_instruction {
                        let carry = (val >> (n - 1)) & 1 == 1;
                        self.update_nzcv(res, carry, self.get_overflow());
                    }
                }
                Instruction::LslReg { rd, rm } => {
                    // Register-controlled shift: amount = Rm[7:0]. Carry = last
                    // bit shifted out; n==0 leaves C unchanged (ARMv7-M Shift_C).
                    // Silicon-verified on STM32F103 via thumb_oracles.
                    let val = self.read_reg(rd);
                    let shift = self.read_reg(rm) & 0xFF;
                    let (res, carry) = if shift == 0 {
                        (val, self.get_carry())
                    } else if shift < 32 {
                        (val << shift, (val >> (32 - shift)) & 1 == 1)
                    } else if shift == 32 {
                        (0, val & 1 == 1)
                    } else {
                        (0, false)
                    };
                    self.write_reg(rd, res);
                    if !it_block_instruction {
                        self.update_nzcv(res, carry, self.get_overflow());
                    }
                }
                Instruction::LsrReg { rd, rm } => {
                    let val = self.read_reg(rd);
                    let shift = self.read_reg(rm) & 0xFF;
                    let (res, carry) = if shift == 0 {
                        (val, self.get_carry())
                    } else if shift < 32 {
                        (val >> shift, (val >> (shift - 1)) & 1 == 1)
                    } else if shift == 32 {
                        (0, (val >> 31) & 1 == 1)
                    } else {
                        (0, false)
                    };
                    self.write_reg(rd, res);
                    if !it_block_instruction {
                        self.update_nzcv(res, carry, self.get_overflow());
                    }
                }
                Instruction::AsrReg { rd, rm } => {
                    let val = self.read_reg(rd);
                    let vali = val as i32;
                    let shift = self.read_reg(rm) & 0xFF;
                    let (res, carry) = if shift == 0 {
                        (val, self.get_carry())
                    } else if shift < 32 {
                        ((vali >> shift) as u32, (val >> (shift - 1)) & 1 == 1)
                    } else {
                        // shift >= 32: result is all sign bits; C = Rm[31].
                        ((vali >> 31) as u32, (val >> 31) & 1 == 1)
                    };
                    self.write_reg(rd, res);
                    if !it_block_instruction {
                        self.update_nzcv(res, carry, self.get_overflow());
                    }
                }
                Instruction::Adc { rd, rm } => {
                    let op1 = self.read_reg(rd);
                    let op2 = self.read_reg(rm);
                    let carry_in = (self.xpsr >> 29) & 1;
                    let (res, c, v) = adc_with_flags(op1, op2, carry_in);
                    self.write_reg(rd, res);
                    self.update_nzcv(res, c, v);
                }
                Instruction::Sbc { rd, rm } => {
                    let op1 = self.read_reg(rd);
                    let op2 = self.read_reg(rm);
                    let carry_in = (self.xpsr >> 29) & 1;
                    let (res, c, v) = sbc_with_flags(op1, op2, carry_in);
                    self.write_reg(rd, res);
                    self.update_nzcv(res, c, v);
                }
                Instruction::Ror { rd, rm } => {
                    // Register rotate: amount = Rm[7:0]. Carry = the rotated
                    // result's MSB; n==0 leaves C unchanged (ARMv7-M ROR_C).
                    // Silicon-verified on STM32F103 via thumb_oracles.
                    let val = self.read_reg(rd);
                    let n = self.read_reg(rm) & 0xFF;
                    let (res, carry) = if n == 0 {
                        (val, self.get_carry())
                    } else {
                        let r = val.rotate_right(n % 32);
                        (r, (r >> 31) & 1 == 1)
                    };
                    self.write_reg(rd, res);
                    if !it_block_instruction {
                        self.update_nzcv(res, carry, self.get_overflow());
                    }
                }
                Instruction::Rev { rd, rm } => {
                    let val = self.read_reg(rm);
                    self.write_reg(rd, val.swap_bytes());
                }
                Instruction::Rev16 { rd, rm } => {
                    let val = self.read_reg(rm);
                    let low = ((val & 0xFF) << 8) | ((val >> 8) & 0xFF);
                    let high = ((val & 0x00FF0000) << 8) | ((val & 0xFF000000) >> 8);
                    self.write_reg(rd, high | low);
                }
                Instruction::RevSh { rd, rm } => {
                    let val = self.read_reg(rm);
                    let low = ((val & 0xFF) << 8) | ((val >> 8) & 0xFF);
                    self.write_reg(rd, (low as i16) as u32);
                }
                Instruction::Tst { rn, rm } => {
                    let res = self.read_reg(rn) & self.read_reg(rm);
                    self.update_nz(res);
                }
                Instruction::Cmn { rn, rm } => {
                    let op1 = self.read_reg(rn);
                    let op2 = self.read_reg(rm);
                    let (res, c, v) = add_with_flags(op1, op2);
                    self.update_nzcv(res, c, v);
                }
                Instruction::Rsbs { rd, rn } => {
                    let op1 = self.read_reg(rn);
                    let (res, c, v) = sub_with_flags(0, op1);
                    self.write_reg(rd, res);
                    self.update_nzcv(res, c, v);
                }

                // Memory Operations (Word)
                Instruction::LdrImm { rt, rn, imm } => {
                    let base = self.read_reg(rn);
                    let addr = base.wrapping_add(imm as u32);
                    if let Ok(val) = bus.read_u32(addr as u64) {
                        self.write_reg(rt, val);
                        if val == 0x021d0000 {
                            tracing::info!("LDR Literal/Imm SUSPICIOUS: R{} loaded with {:#x} from {:#x} (PC={:#x})", rt, val, addr, self.pc);
                        }
                    } else {
                        tracing::error!(
                            "Bus Read Fault at {:#x} (PC={:#x}, Opcode={:#04x})",
                            addr,
                            self.pc,
                            opcode
                        );
                    }
                }
                Instruction::StrImm { rt, rn, imm } => {
                    let base = self.read_reg(rn);
                    let addr = base.wrapping_add(imm as u32);
                    let val = self.read_reg(rt);
                    if bus.write_u32(addr as u64, val).is_err() {
                        tracing::error!(
                            "Bus Write Fault at {:#x} (PC={:#x}, Opcode={:#04x})",
                            addr,
                            self.pc,
                            opcode
                        );
                    }
                }
                Instruction::LdrReg { rt, rn, rm } => {
                    let addr = self.read_reg(rn).wrapping_add(self.read_reg(rm));
                    if let Ok(val) = bus.read_u32(addr as u64) {
                        self.write_reg(rt, val);
                    } else {
                        tracing::error!("Bus Read Fault (LDR reg) at {:#x}", addr);
                    }
                }
                Instruction::StrReg { rt, rn, rm } => {
                    let addr = self.read_reg(rn).wrapping_add(self.read_reg(rm));
                    let val = self.read_reg(rt);
                    let _ = bus.write_u32(addr as u64, val);
                }

                Instruction::LdrLit { rt, imm } => {
                    let pc_val = (self.pc & !3) + 4;
                    let addr = pc_val.wrapping_add(imm as u32);
                    if let Ok(val) = bus.read_u32(addr as u64) {
                        self.write_reg(rt, val);
                    } else {
                        tracing::error!("Bus Read Fault (LdrLit) at {:#x}", addr);
                    }
                }

                Instruction::LdrSp { rt, imm } => {
                    let addr = self.sp.wrapping_add(imm as u32);
                    if let Ok(val) = bus.read_u32(addr as u64) {
                        self.write_reg(rt, val);
                    } else {
                        tracing::error!("Bus Read Fault (LdrSp) at {:#x}", addr);
                    }
                }
                Instruction::StrSp { rt, imm } => {
                    let addr = self.sp.wrapping_add(imm as u32);
                    let val = self.read_reg(rt);
                    if bus.write_u32(addr as u64, val).is_err() {
                        tracing::error!("Bus Write Fault (StrSp) at {:#x}", addr);
                    }
                }
                Instruction::AddSpReg { rd, imm } => {
                    let res = self.sp.wrapping_add(imm as u32);
                    self.write_reg(rd, res);
                }
                Instruction::Adr { rd, imm } => {
                    let pc_val = (self.pc & !3) + 4;
                    let res = pc_val.wrapping_add(imm as u32);
                    self.write_reg(rd, res);
                }
                Instruction::AddwImm { rd, rn, imm } => {
                    // Plain 12-bit zero-extended immediate (T4). Distinct
                    // from DataProcImm32::ADD which runs imm12 through
                    // ThumbExpandImm.
                    let res = self.read_reg(rn).wrapping_add(imm as u32);
                    self.write_reg(rd, res);
                    pc_increment = 4;
                }
                Instruction::SubwImm { rd, rn, imm } => {
                    let res = self.read_reg(rn).wrapping_sub(imm as u32);
                    self.write_reg(rd, res);
                    pc_increment = 4;
                }

                // Memory Operations (Byte)
                Instruction::LdrbImm { rt, rn, imm } => {
                    let base = self.read_reg(rn);
                    let addr = base.wrapping_add(imm as u32);
                    if let Ok(val) = bus.read_u8(addr as u64) {
                        self.write_reg(rt, val as u32);
                    } else {
                        tracing::error!(
                            "Bus Read Fault (LDRB) at {:#x} (PC={:#x}, Opcode={:#04x})",
                            addr,
                            self.pc,
                            opcode
                        );
                    }
                }
                Instruction::LdrbReg { rt, rn, rm } => {
                    let addr = self.read_reg(rn).wrapping_add(self.read_reg(rm));
                    if let Ok(val) = bus.read_u8(addr as u64) {
                        self.write_reg(rt, val as u32);
                    } else {
                        tracing::error!("Bus Read Fault (LDRB reg) at {:#x}", addr);
                    }
                }
                Instruction::StrbReg { rt, rn, rm } => {
                    let addr = self.read_reg(rn).wrapping_add(self.read_reg(rm));
                    let val = (self.read_reg(rt) & 0xFF) as u8;
                    let _ = bus.write_u8(addr as u64, val);
                }
                Instruction::LdrsbReg { rt, rn, rm } => {
                    let addr = self.read_reg(rn).wrapping_add(self.read_reg(rm));
                    if let Ok(val) = bus.read_u8(addr as u64) {
                        let res = (val as i8) as i32 as u32;
                        self.write_reg(rt, res);
                    } else {
                        tracing::error!("Bus Read Fault (LDRSB reg) at {:#x}", addr);
                    }
                }
                Instruction::LdrhReg { rt, rn, rm } => {
                    let addr = self.read_reg(rn).wrapping_add(self.read_reg(rm));
                    if let Ok(val) = bus.read_u16(addr as u64) {
                        self.write_reg(rt, val as u32);
                    } else {
                        tracing::error!("Bus Read Fault (LDRH reg) at {:#x}", addr);
                    }
                }
                Instruction::StrhReg { rt, rn, rm } => {
                    let addr = self.read_reg(rn).wrapping_add(self.read_reg(rm));
                    let val = (self.read_reg(rt) & 0xFFFF) as u16;
                    let _ = bus.write_u16(addr as u64, val);
                }
                Instruction::LdrshReg { rt, rn, rm } => {
                    let addr = self.read_reg(rn).wrapping_add(self.read_reg(rm));
                    if let Ok(val) = bus.read_u16(addr as u64) {
                        let res = (val as i16) as i32 as u32;
                        self.write_reg(rt, res);
                    } else {
                        tracing::error!("Bus Read Fault (LDRSH reg) at {:#x}", addr);
                    }
                }
                Instruction::StrbImm { rt, rn, imm } => {
                    let base = self.read_reg(rn);
                    let addr = base.wrapping_add(imm as u32);
                    let val = (self.read_reg(rt) & 0xFF) as u8;
                    if bus.write_u8(addr as u64, val).is_err() {
                        tracing::error!(
                            "Bus Write Fault (STRB) at {:#x} (PC={:#x}, Opcode={:#04x})",
                            addr,
                            self.pc,
                            opcode
                        );
                    }
                }
                Instruction::LdrhImm { rt, rn, imm } => {
                    let base = self.read_reg(rn);
                    let addr = base.wrapping_add(imm as u32);
                    if let Ok(val) = bus.read_u16(addr as u64) {
                        self.write_reg(rt, val as u32);
                    } else {
                        tracing::error!("Bus Read Fault (LDRH) at {:#x}", addr);
                    }
                }
                Instruction::StrhImm { rt, rn, imm } => {
                    let base = self.read_reg(rn);
                    let addr = base.wrapping_add(imm as u32);
                    let val = (self.read_reg(rt) & 0xFFFF) as u16;
                    if bus.write_u16(addr as u64, val).is_err() {
                        tracing::error!("Bus Write Fault (STRH) at {:#x}", addr);
                    }
                }
                Instruction::Bkpt { imm8 } => {
                    // ARM semihosting uses `bkpt #0xAB` as the trap into
                    // the debugger. On real silicon openocd intercepts
                    // these and emulates the syscall (WRITEC, WRITE0,
                    // SYS_EXIT, …). The simulator doesn't emulate the
                    // syscalls itself — firmware that wants the same
                    // bytes available on both sides should also emit
                    // them via UART, which our sink already captures.
                    // Treating semihosting BKPT as a no-op here lets
                    // such dual-emit firmware run identically on sim
                    // and silicon. Any other BKPT immediate (typical
                    // for `panic!` traps or debugger breakpoints) is
                    // still a halt.
                    if imm8 != 0xAB {
                        return Err(crate::SimulationError::Halt);
                    }
                }

                Instruction::Svc { imm8: _ } => {
                    // Supervisor call. Pend the SVCall exception (number 11);
                    // the exception-entry path at the top of `step_internal`
                    // stacks the frame and vectors to the handler on the next
                    // step. Zephyr drives its fatal handler, irq_offload, and
                    // userspace syscalls through SVC, so an unmodeled SVC left
                    // the PC stuck on the instruction (ztest hung forever).
                    // The immediate selects the call on the Zephyr side; the
                    // handler recovers it from the stacked instruction, so we
                    // don't branch on it here. pc_increment stays 2 so the
                    // stacked return address points just past the SVC.
                    self.set_exception_pending(11);
                }

                // Stack Operations
                Instruction::Push { registers, m } => {
                    let mut sp = self.read_reg(13);
                    // Cycle through R14(LR), R7..R0 high to low

                    // If M (LR) is set, push LR first (highest address)
                    if m {
                        sp = sp.wrapping_sub(4);
                        let val = self.read_reg(14);
                        if bus.write_u32(sp as u64, val).is_err() {
                            tracing::error!("Stack Overflow (PUSH LR)");
                        }
                    }

                    // Registers R7 down to R0
                    for i in (0..=7).rev() {
                        if (registers & (1 << i)) != 0 {
                            sp = sp.wrapping_sub(4);
                            let val = self.read_reg(i);
                            if bus.write_u32(sp as u64, val).is_err() {
                                tracing::error!("Stack Overflow (PUSH R{})", i);
                            }
                        }
                    }

                    self.write_reg(13, sp);
                }
                Instruction::Pop { registers, p } => {
                    let mut sp = self.read_reg(13);

                    // Registers R0 up to R7
                    for i in 0..=7 {
                        if (registers & (1 << i)) != 0 {
                            if let Ok(val) = bus.read_u32(sp as u64) {
                                self.write_reg(i, val);
                            }
                            sp = sp.wrapping_add(4);
                        }
                    }

                    // If P (PC) is set, pop PC (lowest address?? No, highest)
                    // POP is inverse of PUSH. PUSH pushed LR last (lowest addr) ??
                    // Wait. PUSH stores STMDB (Decrement Before). Highest reg = Highest address.
                    // R0 is lowest register. LR is highest.
                    // PUSH order: LR, R7, ... R0.
                    // Stack grows down.
                    // Low Addr [ R0 | R1 | ... | LR ] High Addr.
                    // So POP (LDMIA) should read: R0, ... R7, PC.
                    // My PUSH loop:
                    // 1. If LR, sub 4, write LR. (Top of stack, highest addr - 4)
                    // 2. Loop 7 down to 0: sub 4, write Rx.
                    // Result: R0 is at current SP. LR is at SP + n*4.

                    // My POP loop:
                    // 1. Loop 0 to 7: read, add 4. (Read R0, R1...)
                    // 2. If PC, read, add 4.

                    if p {
                        if let Ok(val) = bus.read_u32(sp as u64) {
                            // Commit SP before branching so EXC_RETURN unstacking reads the
                            // hardware exception frame, not this function's software save area.
                            sp = sp.wrapping_add(4);
                            self.write_reg(13, sp);
                            self.branch_to(val, bus)?;
                            pc_increment = 0; // Branch taken
                        } else {
                            sp = sp.wrapping_add(4);
                            self.write_reg(13, sp);
                        }
                    } else {
                        self.write_reg(13, sp);
                    }
                }
                Instruction::Ldm { rn, registers } => {
                    let mut base = self.read_reg(rn);
                    for i in 0..=7 {
                        if (registers & (1 << i)) != 0 {
                            if let Ok(val) = bus.read_u32(base as u64) {
                                self.write_reg(i, val);
                            }
                            base = base.wrapping_add(4);
                        }
                    }
                    // LDM (T1) writeback: the base register is written back with
                    // the incremented address ONLY when it is NOT in the register
                    // list. When the base IS in the list (the assembler emits no
                    // `!`, e.g. `ldmia r2, {r0,r1,r2}`), the ARMv6-M architecture
                    // specifies the loaded value wins and no writeback occurs.
                    // Writing back unconditionally clobbered the just-loaded value
                    // — which corrupted the compiler's struct-copy / stacked-arg
                    // reload idiom and silently dropped a loaded argument.
                    if (registers & (1 << rn)) == 0 {
                        self.write_reg(rn, base);
                    }
                }
                Instruction::Stm { rn, registers } => {
                    let mut base = self.read_reg(rn);
                    for i in 0..=7 {
                        if (registers & (1 << i)) != 0 {
                            let val = self.read_reg(i);
                            if bus.write_u32(base as u64, val).is_err() {
                                tracing::error!("Bus Write Fault (STM) at {:#x}", base);
                            }
                            base = base.wrapping_add(4);
                        }
                    }
                    self.write_reg(rn, base);
                }
                // STMDB Rn(!), {reg_list} — 32-bit store multiple, decrement before.
                // Lowest-numbered register stored at lowest address.
                Instruction::StmdbW {
                    rn,
                    reg_list,
                    writeback,
                } => {
                    let count = reg_list.count_ones();
                    let mut addr = self.read_reg(rn).wrapping_sub(count * 4);
                    let start = addr;
                    for i in 0u8..=15 {
                        if (reg_list & (1 << i)) != 0 {
                            let val = self.read_reg(i);
                            if bus.write_u32(addr as u64, val).is_err() {
                                tracing::error!("Bus Write Fault (STMDB) at {:#x}", addr);
                            }
                            addr = addr.wrapping_add(4);
                        }
                    }
                    if writeback {
                        self.write_reg(rn, start);
                    }
                    pc_increment = 4;
                }
                // STMIA.W Rn(!), {reg_list} — 32-bit store multiple, increment
                // after. Lowest-numbered register stored at the base address.
                Instruction::StmiaW {
                    rn,
                    reg_list,
                    writeback,
                } => {
                    let mut addr = self.read_reg(rn);
                    for i in 0u8..=14 {
                        if (reg_list & (1 << i)) != 0 {
                            let val = self.read_reg(i);
                            if bus.write_u32(addr as u64, val).is_err() {
                                tracing::error!("Bus Write Fault (STMIA.W) at {:#x}", addr);
                            }
                            addr = addr.wrapping_add(4);
                        }
                    }
                    if writeback {
                        self.write_reg(rn, addr);
                    }
                    pc_increment = 4;
                }
                // LDMDB.W Rn(!), {reg_list} — 32-bit load multiple, decrement
                // before. Lowest-numbered register loaded from the lowest address.
                Instruction::LdmdbW {
                    rn,
                    reg_list,
                    writeback,
                } => {
                    let count = reg_list.count_ones();
                    let start = self.read_reg(rn).wrapping_sub(count * 4);
                    let mut addr = start;
                    for i in 0u8..=14 {
                        if (reg_list & (1 << i)) != 0 {
                            if let Ok(val) = bus.read_u32(addr as u64) {
                                self.write_reg(i, val);
                            }
                            addr = addr.wrapping_add(4);
                        }
                    }
                    if writeback {
                        self.write_reg(rn, start);
                    }
                    if (reg_list & (1 << 15)) != 0 {
                        if let Ok(pc_val) = bus.read_u32(addr as u64) {
                            self.branch_to(pc_val, bus)?;
                            pc_increment = 0;
                        } else {
                            pc_increment = 4;
                        }
                    } else {
                        pc_increment = 4;
                    }
                }
                // LDMIA.W Rn(!), {reg_list} — 32-bit load multiple, increment after.
                // Lowest-numbered register loaded from lowest address.
                Instruction::LdmiaW {
                    rn,
                    reg_list,
                    writeback,
                } => {
                    let mut addr = self.read_reg(rn);
                    // Load R0-R14 (skip PC; handle separately to commit SP first)
                    for i in 0u8..=14 {
                        if (reg_list & (1 << i)) != 0 {
                            if let Ok(val) = bus.read_u32(addr as u64) {
                                self.write_reg(i, val);
                            }
                            addr = addr.wrapping_add(4);
                        }
                    }
                    // Handle PC (bit 15) — commit writeback before branching
                    if (reg_list & (1 << 15)) != 0 {
                        if let Ok(pc_val) = bus.read_u32(addr as u64) {
                            addr = addr.wrapping_add(4);
                            if writeback {
                                self.write_reg(rn, addr);
                            }
                            self.branch_to(pc_val, bus)?;
                            pc_increment = 0;
                        } else {
                            addr = addr.wrapping_add(4);
                            if writeback {
                                self.write_reg(rn, addr);
                            }
                            pc_increment = 4;
                        }
                    } else {
                        if writeback {
                            self.write_reg(rn, addr);
                        }
                        pc_increment = 4;
                    }
                }

                // Control Flow
                Instruction::Bl { offset } => {
                    // BL: Branch with Link.
                    // LR = Next Instruction Address | 1 (Thumb bit)
                    let _next_pc = self.pc + 4; // 32-bit instruction size for BL?
                                                // Wait. BL is decoded as 32-bit.
                                                // If we assume decode_thumb_16 handled a 32-bit stream, then PC increment should be adjusted?
                                                // Or does `decode_thumb_16` return `BlPrefix` and then we handle it?
                                                // The current `decoder` returns `Bl` with full offset if it sees the pair??
                                                // NO. My decoder implementation for BL (in previous turn) was:
                                                // `Instruction::Bl { offset: offset << 1 }`
                                                // But `decode_thumb_16` ONLY sees 16 bits. It cannot see the second half!
                                                // Real decoding of BL requires fetching 32 bits.

                    // CRITICAL CORRECTION: `decode_thumb_16` is 16-bit.
                    // BL is 32-bit (encoded as two 16-bit halves).
                    // Fetch loop fetches 16 bits.
                    // 1. Fetch High Half (0xF0xx). Returns BlPrefix?
                    // 2. Fetch Low Half (0xF8xx). Combine?

                    // My logic in decoder needs revisit. I put `Bl { offset }` thinking T1/T2 but BL is always 32-bit in Thumb-2.
                    // T1 encoding of BL doesn't exist as single 16-bit.

                    // For now, let's just implement the execution stub assuming the decoder *somehow* gave us the full BL.
                    // But since the decoder only sees 16 bits, we need to handle the prefix state in the CPU loop!

                    self.lr = (self.pc + 4) | 1;
                    let target = (self.pc as i32 + 4 + offset) as u32;
                    self.pc = target;
                    pc_increment = 0;
                }
                Instruction::BranchCond { cond, offset } => {
                    if self.check_condition(cond) {
                        let target = (self.pc as i32 + 4 + offset) as u32;
                        self.pc = target;
                        pc_increment = 0;
                    }
                }
                Instruction::Bx { rm } => {
                    let target = self.read_reg(rm);
                    self.branch_to(target, bus)?;
                    pc_increment = 0;
                }

                // BLX Rm (T1): branch-with-link to register address.
                // Sets LR = (PC_of_blx + 2) | 1 before branching.
                Instruction::BlxReg { rm } => {
                    let target = self.read_reg(rm);
                    self.lr = (self.pc.wrapping_add(2)) | 1;
                    self.branch_to(target, bus)?;
                    pc_increment = 0;
                }

                // --- Thumb-2 ARMv7-M additions ---
                Instruction::Barrier => {
                    // DMB / DSB / ISB — architectural no-ops on a single-threaded
                    // simulator. They're modelled explicitly so they don't raise
                    // DecodeError; startup code and HAL inline-asm emit them
                    // routinely.
                    pc_increment = 4;
                }
                Instruction::Mrs { rd, sysm } => {
                    // IPSR (the active exception number, xPSR[8:0]) is load-bearing
                    // for Zephyr: _isr_wrapper reads it and computes `IRQ = IPSR-16`
                    // to index the software ISR table. Returning 0 made the index
                    // -16 → garbage handler. The xPSR/IPSR-bearing reads all expose
                    // the current exception number; PRIMASK, BASEPRI, FAULTMASK,
                    // the banked SPs and CONTROL are the other modelled special
                    // registers. Anything else still reads as zero.
                    let ipsr = self.active_exception & 0x1FF;
                    let val: u32 = match sysm {
                        0x00 => self.xpsr & 0xF800_0000,          // APSR (condition flags)
                        0x03 => (self.xpsr & 0xF800_0000) | ipsr, // xPSR
                        0x05 => ipsr,                             // IPSR
                        0x08 => self.read_msp(),                  // MSP
                        0x09 => self.read_psp(),                  // PSP
                        0x10 => self.primask as u32,
                        // BASEPRI (0x11) and BASEPRI_MAX (0x12) both read BASEPRI.
                        0x11 | 0x12 => self.basepri as u32,
                        0x13 => self.faultmask as u32, // FAULTMASK
                        0x14 => self.control & 0x3,    // CONTROL
                        _ => 0,
                    };
                    self.write_reg(rd, val);
                    pc_increment = 4;
                }
                Instruction::Msr { sysm, rn } => {
                    let val = self.read_reg(rn);
                    match sysm {
                        0x08 => {
                            // MSP bank. If MSP is the live stack, update `sp` too.
                            self.msp = val;
                            if !self.use_psp() {
                                self.sp = val;
                            }
                        }
                        0x09 => {
                            // PSP bank. If PSP is the live stack, update `sp` too.
                            self.psp = val;
                            if self.use_psp() {
                                self.sp = val;
                            }
                        }
                        0x10 => self.primask = (val & 1) != 0,
                        // BASEPRI: plain write of the priority mask byte.
                        0x11 => self.basepri = (val & 0xFF) as u8,
                        // BASEPRI_MAX: writes BASEPRI only if it raises the
                        // masking level (smaller non-zero value), or BASEPRI is 0.
                        0x12 => {
                            let new = (val & 0xFF) as u8;
                            if new != 0 && (self.basepri == 0 || new < self.basepri) {
                                self.basepri = new;
                            }
                        }
                        0x13 => self.faultmask = (val & 1) != 0, // FAULTMASK
                        0x14 => {
                            // CONTROL.SPSEL can switch the active thread stack.
                            // Persist the live `sp` to its bank, change SPSEL/nPRIV,
                            // then re-point `sp` at the newly-selected bank.
                            self.sync_sp_to_bank();
                            self.control = (self.control & !0x3) | (val & 0x3);
                            self.sp = self.current_stack_value();
                        }
                        _ => {}
                    }
                    pc_increment = 4;
                }
                Instruction::Smull {
                    rd_lo,
                    rd_hi,
                    rn,
                    rm,
                } => {
                    let lhs = self.read_reg(rn) as i32 as i64;
                    let rhs = self.read_reg(rm) as i32 as i64;
                    let prod = lhs.wrapping_mul(rhs) as u64;
                    self.write_reg(rd_lo, prod as u32);
                    self.write_reg(rd_hi, (prod >> 32) as u32);
                    pc_increment = 4;
                }
                Instruction::Umull {
                    rd_lo,
                    rd_hi,
                    rn,
                    rm,
                } => {
                    let prod = (self.read_reg(rn) as u64).wrapping_mul(self.read_reg(rm) as u64);
                    self.write_reg(rd_lo, prod as u32);
                    self.write_reg(rd_hi, (prod >> 32) as u32);
                    pc_increment = 4;
                }
                Instruction::Smlal {
                    rd_lo,
                    rd_hi,
                    rn,
                    rm,
                } => {
                    let acc = ((self.read_reg(rd_hi) as u64) << 32) | (self.read_reg(rd_lo) as u64);
                    let lhs = self.read_reg(rn) as i32 as i64;
                    let rhs = self.read_reg(rm) as i32 as i64;
                    let new = (acc as i64).wrapping_add(lhs.wrapping_mul(rhs)) as u64;
                    self.write_reg(rd_lo, new as u32);
                    self.write_reg(rd_hi, (new >> 32) as u32);
                    pc_increment = 4;
                }
                Instruction::Umlal {
                    rd_lo,
                    rd_hi,
                    rn,
                    rm,
                } => {
                    let acc = ((self.read_reg(rd_hi) as u64) << 32) | (self.read_reg(rd_lo) as u64);
                    let prod = (self.read_reg(rn) as u64).wrapping_mul(self.read_reg(rm) as u64);
                    let new = acc.wrapping_add(prod);
                    self.write_reg(rd_lo, new as u32);
                    self.write_reg(rd_hi, (new >> 32) as u32);
                    pc_increment = 4;
                }
                Instruction::Umaal {
                    rd_lo,
                    rd_hi,
                    rn,
                    rm,
                } => {
                    // (rd_hi:rd_lo) = Rn*Rm + rd_lo + rd_hi. Cannot overflow u64.
                    let prod = (self.read_reg(rn) as u64).wrapping_mul(self.read_reg(rm) as u64);
                    let res = prod
                        .wrapping_add(self.read_reg(rd_lo) as u64)
                        .wrapping_add(self.read_reg(rd_hi) as u64);
                    self.write_reg(rd_lo, res as u32);
                    self.write_reg(rd_hi, (res >> 32) as u32);
                    pc_increment = 4;
                }
                Instruction::Mla { rd, rn, rm, ra } => {
                    let res = self
                        .read_reg(ra)
                        .wrapping_add(self.read_reg(rn).wrapping_mul(self.read_reg(rm)));
                    self.write_reg(rd, res);
                    pc_increment = 4;
                }
                Instruction::Mls { rd, rn, rm, ra } => {
                    let res = self
                        .read_reg(ra)
                        .wrapping_sub(self.read_reg(rn).wrapping_mul(self.read_reg(rm)));
                    self.write_reg(rd, res);
                    pc_increment = 4;
                }

                // -------- VFPv4 single-precision (FPU) --------
                Instruction::Vldr { sd, rn, imm, add } => {
                    let base = self.read_reg(rn);
                    let base = if rn == 15 { base & !3 } else { base };
                    let addr = if add {
                        base.wrapping_add(imm as u32)
                    } else {
                        base.wrapping_sub(imm as u32)
                    };
                    if let Ok(val) = bus.read_u32(addr as u64) {
                        self.fpu_s[sd as usize] = val;
                    } else {
                        tracing::error!("Bus Read Fault (VLDR) at {:#x}", addr);
                    }
                    pc_increment = 4;
                }
                Instruction::Vstr { sd, rn, imm, add } => {
                    let base = self.read_reg(rn);
                    let addr = if add {
                        base.wrapping_add(imm as u32)
                    } else {
                        base.wrapping_sub(imm as u32)
                    };
                    let val = self.fpu_s[sd as usize];
                    if bus.write_u32(addr as u64, val).is_err() {
                        tracing::error!("Bus Write Fault (VSTR) at {:#x}", addr);
                    }
                    pc_increment = 4;
                }
                Instruction::VmulF32 { sd, sn, sm } => {
                    let a = f32::from_bits(self.fpu_s[sn as usize]);
                    let b = f32::from_bits(self.fpu_s[sm as usize]);
                    self.fpu_s[sd as usize] = (a * b).to_bits();
                    pc_increment = 4;
                }
                Instruction::VaddF32 { sd, sn, sm } => {
                    let a = f32::from_bits(self.fpu_s[sn as usize]);
                    let b = f32::from_bits(self.fpu_s[sm as usize]);
                    self.fpu_s[sd as usize] = (a + b).to_bits();
                    pc_increment = 4;
                }
                Instruction::VsubF32 { sd, sn, sm } => {
                    let a = f32::from_bits(self.fpu_s[sn as usize]);
                    let b = f32::from_bits(self.fpu_s[sm as usize]);
                    self.fpu_s[sd as usize] = (a - b).to_bits();
                    pc_increment = 4;
                }
                Instruction::VdivF32 { sd, sn, sm } => {
                    let a = f32::from_bits(self.fpu_s[sn as usize]);
                    let b = f32::from_bits(self.fpu_s[sm as usize]);
                    self.fpu_s[sd as usize] = (a / b).to_bits();
                    pc_increment = 4;
                }
                Instruction::VmovSnRt { sn, rt } => {
                    self.fpu_s[sn as usize] = self.read_reg(rt);
                    pc_increment = 4;
                }
                Instruction::VmovRtSn { rt, sn } => {
                    self.write_reg(rt, self.fpu_s[sn as usize]);
                    pc_increment = 4;
                }
                Instruction::VmovF32Reg { sd, sm } => {
                    self.fpu_s[sd as usize] = self.fpu_s[sm as usize];
                    pc_increment = 4;
                }

                Instruction::Unknown(op) => {
                    tracing::warn!("Unknown instruction at {:#x}: Opcode {:#06x}", self.pc, op);
                    crate::fidelity::record_undecoded(self.pc, op as u64, "undecoded T16");
                    pc_increment = 2; // Skip 16-bit
                }
            }
        }

        if it_block_instruction && self.it_state != 0 {
            // ITSTATEUpdate(): advance the low 5 bits, preserving only firstcond[3:1].
            // Bit 4 of the low field becomes cond[0] for the next instruction, so the full
            // 5-bit field must shift, not just the low nibble. This is what flips THEN/ELSE
            // instructions inside blocks such as `ITTEE`.
            self.it_state = (self.it_state & 0xE0) | (((self.it_state & 0x1F) << 1) & 0x1F);
            if (self.it_state & 0x0F) == 0 {
                self.it_state = 0;
            }
        }

        self.pc = self.pc.wrapping_add(pc_increment);

        // Building the register snapshot is pure waste when nothing observes it,
        // and this runs on every instruction. Gate it on having observers, the
        // same way on_step_start above is gated.
        if !_observers.is_empty() {
            let mut registers = [0u32; 17];
            for (i, reg) in registers.iter_mut().enumerate().take(16) {
                *reg = self.get_register(i as u8);
            }
            registers[16] = self.xpsr;

            crate::emit_trace_event(
                _observers,
                labwired_hw_trace::TraceEvent::InstructionRetired {
                    pc: retired_pc,
                    opcode,
                },
            );
            for obs in _observers {
                obs.on_step_end(_cycles, &registers);
            }
        }

        Ok(())
    }
}

// Thumb expand immediate - implements ARM's modified immediate constant expansion
fn thumb_expand_imm(imm12: u32) -> u32 {
    let i = (imm12 >> 11) & 1;
    let imm3 = (imm12 >> 8) & 7;
    let imm8 = imm12 & 0xFF;

    if i == 0 && (imm3 >> 2) == 0 {
        // i:imm3 is 0000, 0001, 0010, 0011.
        // Match repetition patterns:
        match imm3 {
            0 => imm8,                       // 00000000 00000000 00000000 abcdefgh
            1 => (imm8 << 16) | imm8,        // 00000000 abcdefgh 00000000 abcdefgh
            2 => (imm8 << 24) | (imm8 << 8), // abcdefgh 00000000 abcdefgh 00000000
            3 => (imm8 << 24) | (imm8 << 16) | (imm8 << 8) | imm8, // abcdefgh abcdefgh abcdefgh abcdefgh
            _ => unreachable!(),
        }
    } else {
        // Rotated immediate
        // The value to rotate is '1' concatenated with bits 6:0 of imm8.
        let val = 0x80 | (imm8 & 0x7F);
        // The rotation amount 'n' is i:imm3:imm8[7]
        let n = (i << 4) | (imm3 << 1) | (imm8 >> 7);
        val.rotate_right(n)
    }
}

fn add_with_flags(op1: u32, op2: u32) -> (u32, bool, bool) {
    let (res, overflow1) = op1.overflowing_add(op2);
    let carry = overflow1;
    let neg_op1 = (op1 as i32) < 0;
    let neg_op2 = (op2 as i32) < 0;
    let neg_res = (res as i32) < 0;
    let overflow = (neg_op1 == neg_op2) && (neg_res != neg_op1);
    (res, carry, overflow)
}

fn adc_with_flags(op1: u32, op2: u32, carry_in: u32) -> (u32, bool, bool) {
    let (res1, c1) = op1.overflowing_add(op2);
    let (res, c2) = res1.overflowing_add(carry_in);
    let carry = c1 || c2;

    // Overflow: operands have same sign AND result has different sign
    // Effectively (op1 + op2 + carry) overflowed signed range.
    // Approximate check:
    let neg_op1 = (op1 as i32) < 0;
    let neg_op2 = (op2 as i32) < 0;
    let neg_res = (res as i32) < 0;
    // Overflow if inputs same sign, output different
    // Note: Carry_in 0 or 1 usually doesn't change sign logic much, but rigorous check:
    // Sign of (op1 + op2 + carry). It's simpler to rely on basic sign logic or specific algo.
    // ARM ref: Overflow = (op1<31> == op2<31>) && (res<31> != op1<31>)
    // Wait, carry_in effectively adds small value.
    // If op1=MAX, op2=1, c=0 -> overflow pos to neg.
    // Standard V flag logic:
    let overflow = (neg_op1 == neg_op2) && (neg_res != neg_op1);
    (res, carry, overflow)
}

fn sub_with_flags(op1: u32, op2: u32) -> (u32, bool, bool) {
    let (res, borrow) = op1.overflowing_sub(op2);
    let carry = !borrow;
    let neg_op1 = (op1 as i32) < 0;
    let neg_op2 = (op2 as i32) < 0;
    let neg_res = (res as i32) < 0;
    let overflow = (neg_op1 != neg_op2) && (neg_res != neg_op1);
    (res, carry, overflow)
}

fn sbc_with_flags(op1: u32, op2: u32, carry_in: u32) -> (u32, bool, bool) {
    // SBC: op1 - op2 - NOT(carry) = op1 - op2 - (1 - carry)
    let borrow_in = 1 - carry_in;
    let (res1, b1) = op1.overflowing_sub(op2);
    let (res, b2) = res1.overflowing_sub(borrow_in);
    let borrow = b1 || b2;
    let carry = !borrow;

    let neg_op1 = (op1 as i32) < 0;
    let neg_op2 = (op2 as i32) < 0;
    let neg_res = (res as i32) < 0;
    let overflow = (neg_op1 != neg_op2) && (neg_res != neg_op1);
    (res, carry, overflow)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{DmaRequest, Machine, SimulationConfig};
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

    fn run_test_instr(cpu: &mut CortexM, bus: &mut MockBus, instr_bin: u32, is_32bit: bool) {
        let pc = cpu.pc;
        if is_32bit {
            bus.write_u16(pc as u64, (instr_bin >> 16) as u16).unwrap();
            bus.write_u16((pc + 2) as u64, (instr_bin & 0xFFFF) as u16)
                .unwrap();
        } else {
            bus.write_u16(pc as u64, instr_bin as u16).unwrap();
        }
        cpu.step_internal(bus, &[], &bus.config.clone()).unwrap();
    }

    #[test]
    fn cps_faultmask_set_clear_and_mask() {
        // CPSID f (0xB671) sets FAULTMASK; CPSIE f (0xB661) clears it. Zephyr's
        // fault handler toggles FAULTMASK; an unmodelled CPS-f decoded as Unknown
        // and the fault path ("ESF could not be retrieved") failed.
        let mut cpu = CortexM::new();
        let mut bus = MockBus::new();
        // Sequential PCs so the decode cache doesn't reuse the first opcode.
        cpu.pc = 0x1000;
        run_test_instr(&mut cpu, &mut bus, 0xB671, false); // CPSID f @0x1000
        assert!(cpu.faultmask, "CPSID f sets FAULTMASK");
        run_test_instr(&mut cpu, &mut bus, 0xB661, false); // CPSIE f @0x1002
        assert!(!cpu.faultmask, "CPSIE f clears FAULTMASK");

        // FAULTMASK masks a normal IRQ but NOT NMI (exception 2).
        cpu.pc = 0x2000;
        cpu.sp = 0x2000_0040;
        cpu.faultmask = true;
        bus.write_u16(0x2000, 0xBF00).unwrap(); // NOP
        bus.write_u32(0x40, 0x0000_5000 | 1).unwrap(); // exc 16 vector
        cpu.set_exception_pending(16);
        let cfg = bus.config.clone();
        cpu.step_internal(&mut bus, &[], &cfg).unwrap();
        assert_eq!(cpu.pc, 0x2002, "FAULTMASK must mask the IRQ; the NOP runs");

        // NMI (exception 2) still preempts under FAULTMASK.
        cpu.pc = 0x3000;
        bus.write_u32(0x08, 0x0000_6000 | 1).unwrap(); // exc 2 (NMI) vector
        cpu.set_exception_pending(2);
        cpu.step_internal(&mut bus, &[], &cfg).unwrap();
        assert_eq!(cpu.pc, 0x6000, "NMI is never masked by FAULTMASK");
    }

    #[test]
    fn basepri_masks_equal_or_lower_priority_exceptions() {
        // A non-zero BASEPRI masks any exception whose priority value is >=
        // BASEPRI. Zephyr raises BASEPRI to guard scheduler critical sections; an
        // unmodelled BASEPRI let the timer IRQ fire inside them and corrupt state.
        let mut cpu = CortexM::new();
        let mut bus = MockBus::new();
        cpu.pc = 0x1000;
        cpu.sp = 0x2000_0040;
        bus.write_u16(0x1000, 0xBF00).unwrap(); // NOP at 0x1000
                                                // Exception 16 (IRQ 0). With no NVIC wired, its priority reads 0xFF.
        let handler = 0x0000_5000u32;
        bus.write_u32(0x40, handler | 1).unwrap(); // VTOR=0 → vector[16] at 0x40
        cpu.set_exception_pending(16);

        let cfg = bus.config.clone();
        // BASEPRI=0x80 masks priority 0xFF (>= 0x80): the NOP runs, no vectoring.
        cpu.basepri = 0x80;
        cpu.step_internal(&mut bus, &[], &cfg).unwrap();
        assert_eq!(
            cpu.pc, 0x1002,
            "BASEPRI must mask exc 16; the NOP should run"
        );

        // Clearing BASEPRI lets the still-pending exception through.
        cpu.basepri = 0;
        cpu.step_internal(&mut bus, &[], &cfg).unwrap();
        assert_eq!(
            cpu.pc,
            handler & !1,
            "with BASEPRI=0 the pending exception is taken"
        );
    }

    #[test]
    fn mrs_ipsr_reads_active_exception() {
        // `mrs Rd, IPSR` (sysm = 5) must return the current exception number,
        // not 0. Zephyr's _isr_wrapper computes the IRQ line as `IPSR - 16` to
        // index the software ISR table; an IPSR of 0 made the index -16, so it
        // `blx`-ed a garbage handler and executed rodata as code. Bare-metal and
        // FreeRTOS firmware never hit this because they don't dispatch ISRs by
        // reading IPSR.
        let mut cpu = CortexM::new();
        let mut bus = MockBus::new();
        cpu.pc = 0x1000;
        cpu.active_exception = 33; // e.g. an IRQ exception (16 + IRQ 17)

        // mrs r3, IPSR = 0xF3EF 8305
        run_test_instr(&mut cpu, &mut bus, 0xF3EF8305, true);

        assert_eq!(cpu.r3, 33, "MRS IPSR must read the active exception number");
    }

    #[test]
    fn ldr_to_pc_register_offset_branches_to_target() {
        // `ldr.w pc, [r3, r0, lsl #2]` = 0xF853 0xF020 is GCC's switch
        // jump-table idiom. It must branch to the loaded value, not loaded+4:
        // the load-to-PC path was leaving pc_increment at 4, so PC landed one
        // instruction past the real target. That corrupted control flow into
        // Zephyr's onoff state machine (process_event's EVT_START dispatch),
        // tripping `__ASSERT(state == ONOFF_STATE_OFF)` and hanging boot.
        let mut cpu = CortexM::new();
        let mut bus = MockBus::new();
        cpu.pc = 0x1000;
        cpu.r3 = 0x2000; // jump-table base
        cpu.r0 = 2; // case index
                    // [0x2000 + (2 << 2)] = [0x2008] holds the (thumb) target 0x5001.
        bus.write_u32(0x2008, 0x5001).unwrap();

        run_test_instr(&mut cpu, &mut bus, 0xF853F020, true);

        assert_eq!(
            cpu.pc, 0x5000,
            "ldr.w pc,[rn,rm,lsl#n] must branch to the loaded target (thumb bit \
             cleared), not target+4"
        );
    }

    #[test]
    fn test_arm_dataproc_complex() {
        let mut cpu = CortexM::new();
        let mut bus = MockBus::new();
        cpu.pc = 0x1000;

        // Test CLZ
        cpu.r1 = 0x0000FFFF;
        // Instruction::Clz { rd: 0, rm: 1 }
        // Encoding for CLZ R0, R1 is 0xFAB1 F081
        run_test_instr(&mut cpu, &mut bus, 0xFAB1F081, true);
        assert_eq!(cpu.r0, 16);

        // Test RBIT
        cpu.r1 = 0x00000001;
        // RBIT R0, R1 is 0xFA91 F0A1
        run_test_instr(&mut cpu, &mut bus, 0xFA91F0A1, true);
        assert_eq!(cpu.r0, 0x80000000);

        // Test UDIV
        cpu.r1 = 100;
        cpu.r2 = 10;
        // UDIV R0, R1, R2 is 0xFBB1 F0F2
        run_test_instr(&mut cpu, &mut bus, 0xFBB1F0F2, true);
        assert_eq!(cpu.r0, 10);
    }

    #[test]
    fn test_svc_pends_and_takes_svcall_exception() {
        // Zephyr's fatal path, irq_offload, and userspace syscalls all execute
        // `svc`. Without taking the SVCall exception the PC sticks on the
        // instruction forever (ztest hangs). Executing SVC must pend SVCall
        // (exception 11) and the next step must vector to its handler with a
        // standard 8-word exception frame.
        let mut cpu = CortexM::new();
        let mut bus = MockBus::new();
        cpu.pc = 0x1000;
        cpu.sp = 0x2000_0040;

        // VTOR defaults to 0, so the SVCall vector (exc 11) is at 11*4 = 0x2C.
        let handler = 0x0000_5000u32;
        bus.write_u32(0x2C, handler | 1).unwrap(); // thumb bit set

        // SVC #2 at 0x1000.
        bus.write_u16(0x1000, 0xDF02).unwrap();

        let cfg = bus.config.clone();
        // 1st step executes SVC: pends SVCall, advances PC past the 16-bit instr.
        cpu.step_internal(&mut bus, &[], &cfg).unwrap();
        assert_eq!(cpu.pc, 0x1002, "SVC should advance PC past the instruction");

        // 2nd step takes the exception: vector to the handler, stack the frame.
        cpu.step_internal(&mut bus, &[], &cfg).unwrap();
        assert_eq!(cpu.pc, handler & !1, "should vector to the SVCall handler");
        assert_eq!(
            cpu.sp,
            0x2000_0040 - 32,
            "should push an 8-word exception frame"
        );
        assert_eq!(cpu.active_exception, 11, "SVCall is exception 11");
        // Stacked return address (frame + 24) is the instruction after the SVC.
        assert_eq!(bus.read_u32(cpu.sp as u64 + 24).unwrap(), 0x1002);
    }

    #[test]
    fn test_strd_predec_writeback() {
        // e96d ce04 → strd ip, lr, [sp, #-16]!  (P=1, U=0, W=1).
        // libgcc __aeabi_uldivmod prologue used by the mbedTLS bignum/RSA
        // path: it stores ip,lr below SP and updates SP. Ignoring writeback
        // left SP stale so the matching `ldr lr,[sp,#4]` read a garbage
        // return address and the RSA verify wild-jumped.
        let mut cpu = CortexM::new();
        let mut bus = MockBus::new();
        cpu.pc = 0x1000;
        cpu.sp = 0x2000_0040;
        cpu.r12 = 0xAABB_CCDD;
        cpu.lr = 0x0800_5755;

        run_test_instr(&mut cpu, &mut bus, 0xE96DCE04, true);

        // SP updated to SP-16.
        assert_eq!(cpu.sp, 0x2000_0030);
        // Doubleword stored at the new SP.
        assert_eq!(bus.read_u32(0x2000_0030).unwrap(), 0xAABB_CCDD);
        assert_eq!(bus.read_u32(0x2000_0034).unwrap(), 0x0800_5755);
    }

    #[test]
    fn test_ldrd_postindex_writeback() {
        // e8f1 2304 → ldrd r2, r3, [r1], #16  (P=0, U=1, W=1).
        // Post-indexed: load from [r1], then r1 += 16.
        let mut cpu = CortexM::new();
        let mut bus = MockBus::new();
        cpu.pc = 0x1000;
        cpu.set_register(1, 0x2000_0080);
        bus.write_u32(0x2000_0080, 0x1122_3344).unwrap();
        bus.write_u32(0x2000_0084, 0x5566_7788).unwrap();

        run_test_instr(&mut cpu, &mut bus, 0xE8F12304, true);

        assert_eq!(cpu.get_register(2), 0x1122_3344);
        assert_eq!(cpu.get_register(3), 0x5566_7788);
        // Base updated by +16 after the access.
        assert_eq!(cpu.get_register(1), 0x2000_0090);
    }

    #[test]
    fn test_ldrd_offset_no_writeback() {
        // e9d1 0702 → ldrd r0, r7, [r1, #8]  (P=1, U=1, W=0): base unchanged.
        let mut cpu = CortexM::new();
        let mut bus = MockBus::new();
        cpu.pc = 0x1000;
        cpu.set_register(1, 0x2000_0100);
        bus.write_u32(0x2000_0108, 0xDEAD_BEEF).unwrap();
        bus.write_u32(0x2000_010C, 0xFEED_FACE).unwrap();

        run_test_instr(&mut cpu, &mut bus, 0xE9D10702, true);

        assert_eq!(cpu.get_register(0), 0xDEAD_BEEF);
        assert_eq!(cpu.get_register(7), 0xFEED_FACE);
        // Offset form: base register must be unchanged.
        assert_eq!(cpu.get_register(1), 0x2000_0100);
    }

    // Helpers for flag inspection in arithmetic tests.
    const C_BIT: u32 = 1 << 29;
    const V_BIT: u32 = 1 << 28;
    const Z_BIT: u32 = 1 << 30;
    const N_BIT: u32 = 1 << 31;

    #[test]
    fn test_dataproc32_adc_carry_in() {
        let mut cpu = CortexM::new();
        let mut bus = MockBus::new();
        cpu.pc = 0x1000;

        // ADCS R3, R4, R5 with carry-in set: 1 + 1 + 1 = 3
        cpu.r4 = 1;
        cpu.r5 = 1;
        cpu.xpsr |= C_BIT; // carry-in = 1
        run_test_instr(&mut cpu, &mut bus, 0xEB540305, true);
        assert_eq!(cpu.r3, 3);
        assert_eq!(cpu.xpsr & C_BIT, 0); // no carry-out
    }

    #[test]
    fn test_dataproc32_add_sets_carry_and_overflow() {
        let mut cpu = CortexM::new();
        let mut bus = MockBus::new();
        cpu.pc = 0x1000;

        // ADDS R2, R0, R1 : 0xFFFF_FFFF + 1 = 0, carry set, zero set
        cpu.r0 = 0xFFFF_FFFF;
        cpu.r1 = 1;
        run_test_instr(&mut cpu, &mut bus, 0xEB100201, true);
        assert_eq!(cpu.r2, 0);
        assert_ne!(cpu.xpsr & C_BIT, 0);
        assert_ne!(cpu.xpsr & Z_BIT, 0);
        assert_eq!(cpu.xpsr & V_BIT, 0);

        // ADDS overflow: 0x7FFF_FFFF + 1 = 0x8000_0000, V set, N set, C clear
        cpu.pc = 0x1000;
        cpu.r0 = 0x7FFF_FFFF;
        cpu.r1 = 1;
        run_test_instr(&mut cpu, &mut bus, 0xEB100201, true);
        assert_eq!(cpu.r2, 0x8000_0000);
        assert_ne!(cpu.xpsr & V_BIT, 0);
        assert_ne!(cpu.xpsr & N_BIT, 0);
        assert_eq!(cpu.xpsr & C_BIT, 0);
    }

    #[test]
    fn test_simd_uadd8_sel_strlen_kernel() {
        // Reproduces newlib's optimised strlen inner loop, which was silently
        // NOP'd before UADD8/SEL were modelled (C strings measured as garbage).
        let mut cpu = CortexM::new();
        let mut bus = MockBus::new();
        cpu.pc = 0x1000;

        // Word bytes (LE lanes): [0x41 'A', 0x42 'B', 0x43 'C', 0x00 terminator]
        cpu.r2 = 0x0043_4241;
        cpu.r12 = 0xFFFF_FFFF; // ip = ~0
        cpu.r4 = 0;

        // UADD8 r2, r2, ip : GE[i] = (byte + 0xFF >= 0x100) = (byte != 0)
        run_test_instr(&mut cpu, &mut bus, 0xFA82_F24C, true);
        assert_eq!(cpu.get_ge(), 0b0111, "GE must flag the three nonzero bytes");

        // SEL r2, r4, ip : GE-set lanes take r4 (0x00), clear lanes take ip (0xFF).
        // Placed at the next PC (0x1004) so we don't overwrite a prefetched word.
        assert_eq!(cpu.pc, 0x1004);
        run_test_instr(&mut cpu, &mut bus, 0xFAA4_F28C, true);
        assert_eq!(
            cpu.r2, 0xFF00_0000,
            "only the null lane (byte 3) becomes 0xFF"
        );
    }

    #[test]
    fn test_simd_usub8_ssub8_ge() {
        let mut cpu = CortexM::new();
        let mut bus = MockBus::new();
        cpu.pc = 0x1000;
        // USUB8 r1, r2, r3 : GE[i] = (Rn.byte >= Rm.byte), result = wrapping diff
        cpu.r2 = 0x10_05_80_00;
        cpu.r3 = 0x08_05_7F_01;
        run_test_instr(&mut cpu, &mut bus, 0xFAC2_F143, true);
        // lanes: 0x00-0x01=0xFF(borrow,GE0), 0x80-0x7F=0x01(GE1), 0x05-0x05=0(GE1), 0x10-0x08=0x08(GE1)
        assert_eq!(cpu.r1, 0x08_00_01_FF);
        assert_eq!(cpu.get_ge(), 0b1110);
    }

    #[test]
    fn test_dataproc32_multiword_carry_chain() {
        let mut cpu = CortexM::new();
        let mut bus = MockBus::new();
        cpu.pc = 0x1000;

        // 64-bit add: (R1:R0) + (R4:R3) where low words overflow.
        // low: ADDS R2, R0, R1 -> R0=0xFFFF_FFFF + R1=1 -> 0, carry=1
        cpu.r0 = 0xFFFF_FFFF;
        cpu.r1 = 0x0000_0001;
        run_test_instr(&mut cpu, &mut bus, 0xEB100201, true); // ADDS R2,R0,R1
        assert_eq!(cpu.r2, 0);
        assert_ne!(cpu.xpsr & C_BIT, 0);

        // high: ADCS R5, R3, R4 -> 0x10 + 0x20 + carry(1) = 0x31
        cpu.r3 = 0x10;
        cpu.r4 = 0x20;
        run_test_instr(&mut cpu, &mut bus, 0xEB530504, true); // ADCS R5,R3,R4
        assert_eq!(cpu.r5, 0x31);
        // Full result: high=0x31, low=0 -> 0x0000_0031_0000_0000
    }

    #[test]
    fn test_dataproc32_sbc_borrow() {
        let mut cpu = CortexM::new();
        let mut bus = MockBus::new();
        cpu.pc = 0x1000;

        // SBCS R2, R0, R1 with carry-in clear (borrow): 5 - 3 - 1 = 1
        cpu.r0 = 5;
        cpu.r1 = 3;
        cpu.xpsr &= !C_BIT; // carry-in 0 => borrow 1
        run_test_instr(&mut cpu, &mut bus, 0xEB700201, true);
        assert_eq!(cpu.r2, 1);
        assert_ne!(cpu.xpsr & C_BIT, 0); // no final borrow -> C set

        // SBCS producing a borrow: 0 - 1 - 0 = 0xFFFF_FFFE, C clear, N set
        cpu.pc = 0x1000;
        cpu.r0 = 0;
        cpu.r1 = 1;
        cpu.xpsr |= C_BIT; // carry-in 1 => borrow 0
        run_test_instr(&mut cpu, &mut bus, 0xEB700201, true);
        assert_eq!(cpu.r2, 0xFFFF_FFFF);
        assert_eq!(cpu.xpsr & C_BIT, 0); // borrow -> C clear
        assert_ne!(cpu.xpsr & N_BIT, 0);
    }

    #[test]
    fn test_dataproc32_rsb() {
        let mut cpu = CortexM::new();
        let mut bus = MockBus::new();
        cpu.pc = 0x1000;

        // RSB R2, R0, R1 -> R2 = R1 - R0 = 30 - 10 = 20 (no flags)
        cpu.r0 = 10;
        cpu.r1 = 30;
        run_test_instr(&mut cpu, &mut bus, 0xEBC00201, true);
        assert_eq!(cpu.r2, 20);

        // RSBS R2, R0, R1 -> R2 = 0 - 5 = 0xFFFF_FFFB, flags set, C clear (borrow)
        // (pc advances naturally; the decode cache keys on pc, so we must not
        // re-use an address for a different opcode.)
        cpu.r0 = 5;
        cpu.r1 = 0;
        run_test_instr(&mut cpu, &mut bus, 0xEBD00201, true);
        assert_eq!(cpu.r2, 0xFFFF_FFFB);
        assert_ne!(cpu.xpsr & N_BIT, 0);
        assert_eq!(cpu.xpsr & C_BIT, 0);
    }

    #[test]
    fn test_dataproc32_pkh() {
        let mut cpu = CortexM::new();
        let mut bus = MockBus::new();
        cpu.pc = 0x1000;

        // PKHBT R2, R0, R1 : low half from R0, high half from R1
        cpu.r0 = 0xAAAA_BBBB;
        cpu.r1 = 0xCCCC_DDDD;
        run_test_instr(&mut cpu, &mut bus, 0xEAC00201, true);
        assert_eq!(cpu.r2, 0xCCCC_BBBB);

        // PKHTB R2, R0, R1 : high half from R0, low half from R1.
        // pc advances naturally (decode cache keys on pc, not on opcode bytes).
        cpu.r0 = 0xAAAA_BBBB;
        cpu.r1 = 0xCCCC_DDDD;
        run_test_instr(&mut cpu, &mut bus, 0xEAC00221, true);
        assert_eq!(cpu.r2, 0xAAAA_DDDD);
    }

    #[test]
    fn test_dataproc32_umaal() {
        let mut cpu = CortexM::new();
        let mut bus = MockBus::new();
        cpu.pc = 0x1000;

        // UMAAL R0, R1, R2, R3 : (R1:R0) = R2*R3 + R0 + R1
        // R2=0xFFFF_FFFF, R3=0xFFFF_FFFF -> 0xFFFF_FFFE_0000_0001
        // + R0(0x10) + R1(0x20) -> 0xFFFF_FFFE_0000_0031
        cpu.r2 = 0xFFFF_FFFF;
        cpu.r3 = 0xFFFF_FFFF;
        cpu.r0 = 0x10;
        cpu.r1 = 0x20;
        run_test_instr(&mut cpu, &mut bus, 0xFBE20163, true);
        let result = ((cpu.r1 as u64) << 32) | (cpu.r0 as u64);
        assert_eq!(result, 0xFFFF_FFFE_0000_0031);
    }

    #[test]
    fn test_arm_bitfield() {
        let mut cpu = CortexM::new();
        let mut bus = MockBus::new();
        cpu.pc = 0x2000;

        // BFI R1, R0, 4, 8
        cpu.r0 = 0x000000FF;
        cpu.r1 = 0x00000000;
        // BFI R1, R0, 4, 8 is 0xF360 110B
        run_test_instr(&mut cpu, &mut bus, 0xF360110B, true);
        assert_eq!(cpu.r1, 0x00000FF0);

        // BFC R1, 4, 4
        cpu.r1 = 0xFFFFFFFF;
        // BFC R1, 4, 4 is 0xF36F 1107
        run_test_instr(&mut cpu, &mut bus, 0xF36F1107, true);
        assert_eq!(cpu.r1, 0xFFFFFF0F);

        // UBFX R1, R0, 4, 4
        cpu.r0 = 0x000000F0;
        // UBFX R1, R0, 4, 4 is 0xF3C0 1103
        run_test_instr(&mut cpu, &mut bus, 0xF3C01103, true);
        assert_eq!(cpu.r1, 0x0000000F);
    }

    #[test]
    fn test_arm_dataproc_imm() {
        let mut cpu = CortexM::new();
        let mut bus = MockBus::new();
        cpu.pc = 0x3000;

        // ADC R0, R1, #0
        cpu.r1 = 10;
        cpu.xpsr |= 1 << 29; // Set Carry
                             // ADC.W R0, R1, #0 is 0xF141 0000
        run_test_instr(&mut cpu, &mut bus, 0xF1410000, true);
        assert_eq!(cpu.r0, 11);

        // SBC R0, R1, #0
        cpu.r1 = 10;
        cpu.xpsr &= !(1 << 29); // Clear Carry (Borrow)
                                // SBC.W R0, R1, #0 is 0xF161 0000
        run_test_instr(&mut cpu, &mut bus, 0xF1610000, true);
        assert_eq!(cpu.r0, 9);
    }

    #[test]
    fn test_arm_ldrd_strd() {
        let mut cpu = CortexM::new();
        let mut bus = MockBus::new();
        cpu.pc = 0x4000;
        cpu.r2 = 0x5000;

        // STRD R0, R1, [R2, #8]
        cpu.r0 = 0x11111111;
        cpu.r1 = 0x22222222;
        // STRD R0, R1, [R2, #8] is 0xE9C2 0102
        run_test_instr(&mut cpu, &mut bus, 0xE9C20102, true);
        assert_eq!(bus.read_u32(0x5008).unwrap(), 0x11111111);
        assert_eq!(bus.read_u32(0x500C).unwrap(), 0x22222222);

        // LDRD R3, R4, [R2, #8]
        // LDRD R3, R4, [R2, #8] is 0xE9D2 3402
        run_test_instr(&mut cpu, &mut bus, 0xE9D23402, true);
        assert_eq!(cpu.r3, 0x11111111);
        assert_eq!(cpu.r4, 0x22222222);
    }

    #[test]
    fn test_arm_ldrd_negative_offset() {
        // Regression: LDRD T1 with U=0 must subtract imm8*4 from the base.
        // `ldrd r0, r7, [r1, #-32]` = E951 0708 — mbedTLS AES round-key load.
        let mut cpu = CortexM::new();
        let mut bus = MockBus::new();
        cpu.pc = 0x4000;
        cpu.r1 = 0x5020; // base = 0x5020; addr = 0x5020 - 32 = 0x5000
        bus.write_u32(0x5000, 0xDEAD_BEEF).unwrap();
        bus.write_u32(0x5004, 0xCAFE_BABE).unwrap();
        // E951 0708: ldrd r0, r7, [r1, #-32] (U=0, imm8=8 → offset=32)
        run_test_instr(&mut cpu, &mut bus, 0xE9510708, true);
        assert_eq!(cpu.r0, 0xDEAD_BEEF);
        assert_eq!(cpu.r7, 0xCAFE_BABE);
    }

    #[test]
    fn test_ldr_t4_post_index_pc_function_return() {
        // Regression: `ldr.w pc, [sp], #4` = F85D FB04 (T4 post-index, U=1,
        // W=1) is the clang function-return idiom. Previously decoded to
        // Unknown32 and silently skipped, so the return branched nowhere and
        // execution fell through to a wrong address. Verify it loads PC from
        // [sp] and post-increments sp by 4.
        let mut cpu = CortexM::new();
        let mut bus = MockBus::new();
        cpu.pc = 0x4000;
        cpu.sp = 0x6000;
        bus.write_u32(0x6000, 0x0000_1235).unwrap(); // return addr (thumb bit set)
        run_test_instr(&mut cpu, &mut bus, 0xF85DFB04, true);
        assert_eq!(
            cpu.pc, 0x0000_1234,
            "PC must come from [sp] (thumb bit cleared)"
        );
        assert_eq!(cpu.sp, 0x6004, "post-index writeback: sp += 4");
    }

    #[test]
    fn test_ldr_t4_pre_index_writeback() {
        // `ldr.w r3, [r1, #8]!` = F851 3F08 (T4 pre-index, U=1, W=1).
        let mut cpu = CortexM::new();
        let mut bus = MockBus::new();
        cpu.pc = 0x4000;
        cpu.r1 = 0x5000;
        bus.write_u32(0x5008, 0xABCD_1234).unwrap();
        run_test_instr(&mut cpu, &mut bus, 0xF8513F08, true);
        assert_eq!(cpu.r3, 0xABCD_1234, "loaded from r1+8");
        assert_eq!(cpu.r1, 0x5008, "pre-index writeback: r1 = r1+8");
    }

    #[test]
    fn test_str_t4_pre_decrement_writeback() {
        // `str.w r2, [r1, #-4]!` = F841 2D04 (T4 pre-index, U=0, W=1).
        let mut cpu = CortexM::new();
        let mut bus = MockBus::new();
        cpu.pc = 0x4000;
        cpu.r1 = 0x5008;
        cpu.r2 = 0xDEAD_BEEF;
        run_test_instr(&mut cpu, &mut bus, 0xF8412D04, true);
        assert_eq!(bus.read_u32(0x5004).unwrap(), 0xDEAD_BEEF, "stored at r1-4");
        assert_eq!(cpu.r1, 0x5004, "pre-index writeback: r1 = r1-4");
    }

    #[test]
    fn test_thumb2_stmia_ldmdb_wide_addressing() {
        // Regression: the 0xE8xx/0xE9xx LDM/STM group was decoded as STM=>DB,
        // LDM=>IA unconditionally, so STMIA.W (the compiler's struct-copy idiom)
        // stored *below* the base instead of at it. Verify both addressing modes.
        let mut cpu = CortexM::new();
        let mut bus = MockBus::new();
        cpu.pc = 0x4000;
        cpu.r2 = 0x5000;
        cpu.r0 = 0xAAAA_AAAA;
        cpu.r1 = 0xBBBB_BBBB;

        // STMIA.W r2, {r0, r1}  (no writeback) = 0xE882 0003 — stores at the base.
        run_test_instr(&mut cpu, &mut bus, 0xE882_0003, true);
        assert_eq!(bus.read_u32(0x5000).unwrap(), 0xAAAA_AAAA);
        assert_eq!(bus.read_u32(0x5004).unwrap(), 0xBBBB_BBBB);
        assert_eq!(cpu.r2, 0x5000, "no writeback leaves Rn unchanged");

        // STMIA.W r2!, {r0, r1} (writeback) = 0xE8A2 0003 — advances Rn by 8.
        cpu.r2 = 0x6000;
        run_test_instr(&mut cpu, &mut bus, 0xE8A2_0003, true);
        assert_eq!(bus.read_u32(0x6000).unwrap(), 0xAAAA_AAAA);
        assert_eq!(cpu.r2, 0x6008, "writeback advances Rn");

        // LDMDB.W r2, {r3, r4} = 0xE912 0018 — loads from below the base.
        cpu.r2 = 0x5008;
        run_test_instr(&mut cpu, &mut bus, 0xE912_0018, true);
        assert_eq!(cpu.r3, 0xAAAA_AAAA);
        assert_eq!(cpu.r4, 0xBBBB_BBBB);
        assert_eq!(cpu.r2, 0x5008, "no writeback leaves Rn unchanged");
    }

    #[test]
    fn test_thumb2_pld_is_nop_not_pc_load() {
        // Regression: PLD/PLI/PLDW (preload memory hints) are encoded as
        // byte/halfword "loads" with Rt==15. The 0xF800 LDR/STR handler wrote the
        // loaded value into Rt=15 (PC), so `pld [r0]` — newlib's PLD-optimized
        // strlen/memchr idiom — loaded a byte from [r0] into PC and the CPU jumped
        // to a garbage (flash-alias) address, looping forever. Hints must be NOPs;
        // only a WORD load (op1&7==5) with Rt==15 is a real LDR.W PC branch.
        let mut cpu = CortexM::new();
        let mut bus = MockBus::new();
        cpu.pc = 0x4000;
        cpu.r0 = 0x5000;
        // A value that would become a bogus PC if the hint were mishandled.
        bus.write_u32(0x5000, 0x0000_0048).unwrap();

        // PLD [r0, #0] = 0xF890 0xF000 (byte-load form, Rt=15).
        run_test_instr(&mut cpu, &mut bus, 0xF890_F000, true);
        assert_eq!(
            cpu.pc, 0x4004,
            "PLD must be a NOP (PC+=4), not a load into PC"
        );

        // PLDW/halfword preload hint [r0] = 0xF8B0 0xF000 (halfword form, Rt=15).
        cpu.pc = 0x4000;
        run_test_instr(&mut cpu, &mut bus, 0xF8B0_F000, true);
        assert_eq!(cpu.pc, 0x4004, "halfword preload hint must be a NOP");

        // The byte was never consumed as a PC — confirm no spurious branch left
        // PC in the flash-alias region.
        assert!(
            cpu.pc >= 0x4000,
            "PLD/PLDW must not have branched into low memory"
        );
    }

    #[test]
    fn test_thumb2_barrier_is_nop() {
        // DMB SY = F3BF 8F5F. Executor must advance PC by 4 and not fault.
        let mut cpu = CortexM::new();
        let mut bus = MockBus::new();
        let pc_before = cpu.pc;
        run_test_instr(&mut cpu, &mut bus, 0xF3BF_8F5F, true);
        assert_eq!(cpu.pc, pc_before + 4, "DMB advances PC by 4");
    }

    #[test]
    fn test_thumb2_msr_mrs_primask() {
        // MSR PRIMASK, r0 = F380 8810 ; MRS r1, PRIMASK = F3EF 8110.
        let mut cpu = CortexM::new();
        let mut bus = MockBus::new();
        cpu.r0 = 1; // request PRIMASK = 1 (interrupts disabled)
        run_test_instr(&mut cpu, &mut bus, 0xF380_8810, true);
        assert!(cpu.primask, "MSR PRIMASK set primask from r0");

        run_test_instr(&mut cpu, &mut bus, 0xF3EF_8110, true);
        assert_eq!(cpu.r1, 1, "MRS reads primask back into r1");

        // Clearing: MSR PRIMASK, r0 with r0 = 0.
        cpu.r0 = 0;
        run_test_instr(&mut cpu, &mut bus, 0xF380_8810, true);
        assert!(!cpu.primask);
    }

    #[test]
    fn test_thumb2_wide_multiplies() {
        let mut cpu = CortexM::new();
        let mut bus = MockBus::new();

        // SMULL rd_lo=r0, rd_hi=r1, rn=r2, rm=r3.
        // Encoding: 1111 1011 1000 rn4 | rd_lo4 rd_hi4 0000 rm4
        //         = F B 8 2 | 0 1 0 3 = FB82_0103
        cpu.r2 = u32::MAX; // -1
        cpu.r3 = 2;
        run_test_instr(&mut cpu, &mut bus, 0xFB82_0103, true);
        // -1 * 2 = -2 → 64-bit 0xFFFF_FFFF_FFFF_FFFE
        assert_eq!(cpu.r0, 0xFFFF_FFFE, "SMULL low half");
        assert_eq!(cpu.r1, 0xFFFF_FFFF, "SMULL high half (sign-extended)");

        // UMULL same operands: u32::MAX * 2 = 0x1_FFFF_FFFE.
        // Encoding: 1111 1011 1010 rn4 | rd_lo4 rd_hi4 0000 rm4 = FBA2_0103
        cpu.r2 = u32::MAX;
        cpu.r3 = 2;
        run_test_instr(&mut cpu, &mut bus, 0xFBA2_0103, true);
        assert_eq!(cpu.r0, 0xFFFF_FFFE, "UMULL low half");
        assert_eq!(cpu.r1, 0x0000_0001, "UMULL high half");
    }

    #[test]
    fn test_thumb2_mla_mls() {
        let mut cpu = CortexM::new();
        let mut bus = MockBus::new();

        // MLA r0 = r3 + (r1 * r2)
        // Encoding: 1111 1011 0000 rn4 | ra4 rd4 0000 rm4
        //         = F B 0 1 | 3 0 0 2 = FB01_3002
        cpu.r1 = 2;
        cpu.r2 = 3;
        cpu.r3 = 100;
        run_test_instr(&mut cpu, &mut bus, 0xFB01_3002, true);
        assert_eq!(cpu.r0, 106, "MLA: 100 + 2*3 = 106");

        // MLS r0 = r3 - (r1 * r2) — op selector 0x1 in h2[7:4].
        // Encoding: FB01_3012
        cpu.r1 = 2;
        cpu.r2 = 3;
        cpu.r3 = 100;
        run_test_instr(&mut cpu, &mut bus, 0xFB01_3012, true);
        assert_eq!(cpu.r0, 94, "MLS: 100 - 2*3 = 94");
    }

    #[test]
    fn test_thumb2_vfp_smoke_3_14_times_2() {
        // Exact reproduction of what the NUCLEO-L476RG smoke firmware does:
        //   ldr r3, =0x4048F5C3        ; 3.14f
        //   str r3, [sp, #12]
        //   movw r3, #0x0000           ; 2.0f low half
        //   movt r3, #0x4000           ; 2.0f high half
        //   str r3, [sp, #16]
        //   vldr s15, [sp, #12]        ; load 3.14
        //   vldr s14, [sp, #16]        ; load 2.0
        //   vmul.f32 s15, s15, s14
        //   vstr s15, [sp, #20]
        //   ldr r4, [sp, #20]          ; r4 = IEEE bits of 6.28
        // Hardware-verified result: 3.14f * 2.0f = 6.28f = 0x40C8F5C3.
        let mut cpu = CortexM::new();
        let mut bus = MockBus::new();
        // Set up memory so the VLDR/VSTR have somewhere to land.
        cpu.sp = 0x2000_0100;
        bus.write_u32(0x2000_010C, 0x4048_F5C3).unwrap(); // 3.14f
        bus.write_u32(0x2000_0110, 0x4000_0000).unwrap(); // 2.0f

        // VLDR S15, [SP, #12] = EDDD 7A03
        run_test_instr(&mut cpu, &mut bus, 0xEDDD_7A03, true);
        assert_eq!(cpu.fpu_s[15], 0x4048_F5C3);

        // VLDR S14, [SP, #16] = ED9D 7A04
        run_test_instr(&mut cpu, &mut bus, 0xED9D_7A04, true);
        assert_eq!(cpu.fpu_s[14], 0x4000_0000);

        // VMUL.F32 S15, S15, S14 = EE67 7A87
        run_test_instr(&mut cpu, &mut bus, 0xEE67_7A87, true);
        assert_eq!(
            cpu.fpu_s[15], 0x40C8_F5C3,
            "3.14f * 2.0f IEEE-754 bits — must match real Cortex-M4F output"
        );

        // VSTR S15, [SP, #20] = EDCD 7A05
        run_test_instr(&mut cpu, &mut bus, 0xEDCD_7A05, true);
        assert_eq!(bus.read_u32(0x2000_0114).unwrap(), 0x40C8_F5C3);
    }

    fn vfp_arith_encoding(h1_op: u16, sd: u8, sn: u8, sm: u8, op_b: u32) -> u32 {
        // Build the 32-bit Thumb encoding for VMUL/VADD/VSUB/VDIV.F32.
        // Sd:D = (Vd<<1):D, similarly for Sn and Sm. op_b selects ADD vs
        // SUB at bit[6] of h2.
        let vd = (sd >> 1) & 0xF;
        let d = (sd & 1) as u32;
        let vn = (sn >> 1) & 0xF;
        let n = (sn & 1) as u32;
        let vm = (sm >> 1) & 0xF;
        let m = (sm & 1) as u32;
        let h1 = h1_op | ((d as u16) << 6) | (vn as u16);
        let h2 = ((vd as u16) << 12)
            | 0x0A00
            | ((n as u16) << 7)
            | ((op_b as u16) << 6)
            | ((m as u16) << 5)
            | (vm as u16);
        ((h1 as u32) << 16) | (h2 as u32)
    }

    #[test]
    fn test_thumb2_vfp_add_sub_div() {
        let mut cpu = CortexM::new();
        let mut bus = MockBus::new();
        cpu.fpu_s[0] = (6.0_f32).to_bits();
        cpu.fpu_s[1] = (2.0_f32).to_bits();

        run_test_instr(
            &mut cpu,
            &mut bus,
            vfp_arith_encoding(0xEE30, 2, 0, 1, 0),
            true,
        );
        assert_eq!(cpu.fpu_s[2], (8.0_f32).to_bits(), "VADD: 6 + 2 = 8");

        run_test_instr(
            &mut cpu,
            &mut bus,
            vfp_arith_encoding(0xEE30, 2, 0, 1, 1),
            true,
        );
        assert_eq!(cpu.fpu_s[2], (4.0_f32).to_bits(), "VSUB: 6 - 2 = 4");

        run_test_instr(
            &mut cpu,
            &mut bus,
            vfp_arith_encoding(0xEE80, 2, 0, 1, 0),
            true,
        );
        assert_eq!(cpu.fpu_s[2], (3.0_f32).to_bits(), "VDIV: 6 / 2 = 3");

        run_test_instr(
            &mut cpu,
            &mut bus,
            vfp_arith_encoding(0xEE20, 2, 0, 1, 0),
            true,
        );
        assert_eq!(cpu.fpu_s[2], (12.0_f32).to_bits(), "VMUL: 6 * 2 = 12");
    }

    #[test]
    fn test_thumb_sxth_sxtb_uxth_uxtb() {
        // Family encoding: 1011 0010 op2:2 mmm:3 ddd:3.
        //   op2 = 00 -> SXTH, 01 -> SXTB, 10 -> UXTH, 11 -> UXTB
        // Surfaced on NUCLEO-L476RG: GCC emits UXTH (0xB280) when
        // truncating a uint16_t expression to fit a u32 register;
        // sim was raising "Unknown instruction" for it.
        let mut cpu = CortexM::new();
        let mut bus = MockBus::new();

        // SXTH r0, r1 = 0xB208 (Rm=1, Rd=0).
        cpu.r1 = 0x0000_8000; // bit 15 set -> negative as i16
        run_test_instr(&mut cpu, &mut bus, 0xB208, false);
        assert_eq!(cpu.r0, 0xFFFF_8000, "SXTH sign-extends bit 15");

        // SXTB r0, r1 = 0xB248 (Rm=1, Rd=0).
        cpu.r1 = 0x0000_0080;
        run_test_instr(&mut cpu, &mut bus, 0xB248, false);
        assert_eq!(cpu.r0, 0xFFFF_FF80, "SXTB sign-extends bit 7");

        // UXTH r0, r1 = 0xB288.
        cpu.r1 = 0x1234_ABCD;
        run_test_instr(&mut cpu, &mut bus, 0xB288, false);
        assert_eq!(cpu.r0, 0x0000_ABCD, "UXTH zero-extends low 16");

        // UXTB r0, r1 = 0xB2C8.
        cpu.r1 = 0x1234_ABCD;
        run_test_instr(&mut cpu, &mut bus, 0xB2C8, false);
        assert_eq!(cpu.r0, 0x0000_00CD, "UXTB zero-extends low 8");
    }

    #[test]
    fn test_thumb2_addw_subw_plain_immediate() {
        // ADDW r3, r3, #0x789 (T4 plain immediate). Encoding F203 7389:
        //   1111 0 010 0000 0011 | 0 111 0011 10001001
        // imm12 = 0:111:10001001 = 0x789 zero-extended (NOT ThumbExpand).
        let mut cpu = CortexM::new();
        let mut bus = MockBus::new();
        cpu.r3 = 0x12344EEF;
        run_test_instr(&mut cpu, &mut bus, 0xF203_7389, true);
        assert_eq!(cpu.r3, 0x12345678, "ADDW r3, r3, #0x789");

        // SUBW r3, r3, #0x10 (T4). Encoding pattern F2A3 0310:
        //   1111 0 010 1010 0011 | 0 000 0011 00010000
        cpu.r3 = 0x100;
        run_test_instr(&mut cpu, &mut bus, 0xF2A3_0310, true);
        assert_eq!(cpu.r3, 0xF0, "SUBW r3, r3, #0x10");
    }

    #[test]
    fn test_thumb2_shift_register_lsr_lsl_asr() {
        // Regression: the Thumb-2 shift-by-register encoding (FA0x..FA7x)
        // was reading shift_type from h2[5:4] instead of h1[6:5], so
        // LSR/ASR/ROR were silently decoded as LSL. Surfaced on
        // NUCLEO-L476RG via __aeabi_u2h emit from `(v >> n) & 0xF` in a
        // stock GCC hex print loop.
        let mut cpu = CortexM::new();
        let mut bus = MockBus::new();

        // LSR.W r2, r0, r3  (FA20 F203). r0 = 0x60FC303A, r3 = 28 -> r2 = 0x6.
        cpu.r0 = 0x60FC303A;
        cpu.r3 = 28;
        run_test_instr(&mut cpu, &mut bus, 0xFA20_F203, true);
        assert_eq!(cpu.r2, 0x6, "LSR.W by 28 of 0x60FC303A");

        // LSL.W r2, r0, r3  (FA00 F203). r0 = 0x6, r3 = 28 -> r2 = 0x60000000.
        cpu.r0 = 0x6;
        cpu.r3 = 28;
        run_test_instr(&mut cpu, &mut bus, 0xFA00_F203, true);
        assert_eq!(cpu.r2, 0x6000_0000, "LSL.W by 28 of 0x6");

        // ASR.W r2, r0, r3  (FA40 F203). r0 = 0xF0000000, r3 = 4 -> r2 = 0xFF000000.
        cpu.r0 = 0xF000_0000;
        cpu.r3 = 4;
        run_test_instr(&mut cpu, &mut bus, 0xFA40_F203, true);
        assert_eq!(cpu.r2, 0xFF00_0000, "ASR.W by 4 of 0xF0000000 sign-extends");
    }

    /// SHPR3-driven priority dispatch: PendSV at lowest priority (0xFF) must
    /// not preempt an active higher-priority IRQ. This is the load-bearing
    /// behaviour for FreeRTOS — SysTick (higher prio) pends PendSV which
    /// only takes once SysTick returns. Once SysTick is no longer active,
    /// PendSV is takeable from thread mode.
    #[test]
    fn shpr3_pendsv_does_not_preempt_active_higher_priority_irq() {
        let mut cpu = CortexM::new();
        // Wire SHPR3 with PendSV (byte 2) at 0xFF, SysTick (byte 3) at 0x00.
        let shpr1 = Arc::new(AtomicU32::new(0));
        let shpr2 = Arc::new(AtomicU32::new(0));
        let shpr3 = Arc::new(AtomicU32::new(0x00FF_0000));
        cpu.set_shared_shpr(shpr1, shpr2, shpr3);

        // PendSV at 0xFF, SysTick at 0x00.
        assert_eq!(cpu.exception_priority(14), 0xFF);
        assert_eq!(cpu.exception_priority(15), 0x00);

        // PendSV pending while SysTick is active — must NOT be takeable.
        cpu.active_exception = 15;
        cpu.pending_exceptions[0] = 1u64 << 14;
        assert_eq!(cpu.highest_priority_pending(), Some(14));
        let active_prio = cpu.exception_priority(cpu.active_exception);
        let pend_prio = cpu.exception_priority(14);
        assert!(
            pend_prio >= active_prio,
            "PendSV (0xFF) must not preempt active SysTick (0x00)"
        );

        // SysTick returns; from thread mode PendSV must be takeable.
        cpu.active_exception = 0;
        let active_prio = cpu.exception_priority(0);
        assert!(
            pend_prio < active_prio,
            "PendSV at 0xFF must be takeable from thread mode (256)"
        );
    }

    /// IRQs read priorities from the shared NVIC IPR. Two pending IRQs
    /// with different priorities must dispatch by priority, not by IRQ
    /// number.
    // --- Thumb-1 register-offset load/store execution tests ---
    // Opcodes derived from ARMv7-M A6.2.4: bits[15:9] = 0101 op[2:0],
    // bits[8:6]=Rm, bits[5:3]=Rn, bits[2:0]=Rt.
    // All four tests use Rt=R0, Rn=R1 (base), Rm=R2 (offset).

    #[test]
    fn test_exec_strh_reg_offset() {
        // STRH R0, [R1, R2] — op=001 — 0101 001 010 001 000 = 0x5288
        let mut cpu = CortexM::new();
        let mut bus = MockBus::new();
        cpu.pc = 0x2000;
        // Base address in R1, zero offset in R2.
        cpu.r1 = 0x3000;
        cpu.r2 = 0x0000;
        // Value to store: 0xBEEF (bottom 16 bits).
        cpu.r0 = 0x0000BEEF;
        run_test_instr(&mut cpu, &mut bus, 0x5288, false);
        // The halfword at 0x3000 should be 0xBEEF.
        assert_eq!(bus.read_u16(0x3000).unwrap(), 0xBEEF);
    }

    #[test]
    fn test_exec_wide_register_extend() {
        // Regression: the wide (T2) register-extend instructions were not
        // decoded (fell to Unknown32 and were skipped), leaving Rd stale.
        // clang emits e.g. `uxth.w r2, ip` = FA1F F28C when extending a high
        // register, which corrupted a UDS routine-id argument (read as 0).
        // UXTH.W R2, R12  = FA1F F28C — zero-extend low 16 bits.
        {
            let mut cpu = CortexM::new();
            let mut bus = MockBus::new();
            cpu.pc = 0x2000;
            cpu.r2 = 0xDEAD_BEEF; // stale value that must be overwritten
            cpu.r12 = 0x1234_FF00;
            run_test_instr(&mut cpu, &mut bus, 0xFA1FF28C, true);
            assert_eq!(cpu.r2, 0x0000_FF00, "UXTH.W must zero-extend low 16 bits");
        }
        // UXTB.W R0, R1   = FA5F F081 — zero-extend low 8 bits.
        {
            let mut cpu = CortexM::new();
            let mut bus = MockBus::new();
            cpu.pc = 0x2000;
            cpu.r1 = 0x0000_0085;
            run_test_instr(&mut cpu, &mut bus, 0xFA5FF081, true);
            assert_eq!(cpu.r0, 0x0000_0085, "UXTB.W must zero-extend low 8 bits");
        }
        // SXTB.W R0, R1   = FA4F F081 — sign-extend low 8 bits.
        {
            let mut cpu = CortexM::new();
            let mut bus = MockBus::new();
            cpu.pc = 0x2000;
            cpu.r1 = 0x0000_0085;
            run_test_instr(&mut cpu, &mut bus, 0xFA4FF081, true);
            assert_eq!(cpu.r0, 0xFFFF_FF85, "SXTB.W must sign-extend low 8 bits");
        }
        // UXTH.W R0, R1, ROR #8 = FA1F F091 — rotate then zero-extend.
        {
            let mut cpu = CortexM::new();
            let mut bus = MockBus::new();
            cpu.pc = 0x2000;
            cpu.r1 = 0x0085_0000;
            run_test_instr(&mut cpu, &mut bus, 0xFA1FF091, true);
            // ROR #8 of 0x00850000 = 0x00008500; & 0xFFFF = 0x8500.
            assert_eq!(cpu.r0, 0x0000_8500, "UXTH.W ROR #8 must rotate then extend");
        }
        // UXTAH R0, R1, R2 = FA11 F082 — R0 = R1 + uxth(R2) (extend-and-add).
        // This is the `4 + path_len` form (uxtah r6,r3,r0) that the plain-extend
        // decode missed, leaving the result register stale.
        {
            let mut cpu = CortexM::new();
            let mut bus = MockBus::new();
            cpu.pc = 0x2000;
            cpu.r0 = 0xDEAD_BEEF; // stale value that must be overwritten
            cpu.r1 = 0x0000_0004;
            cpu.r2 = 0x1234_0002;
            run_test_instr(&mut cpu, &mut bus, 0xFA11F082, true);
            assert_eq!(cpu.r0, 0x0000_0006, "UXTAH must add Rn to the extended Rm");
        }
    }

    #[test]
    fn test_exec_wide_load_byte_halfword_extension() {
        // Regression: the wide (32-bit Thumb-2) load encodings select
        // signed vs unsigned via h1 bit 8 (0x0100), not op1 bit 3.
        // Previously LDRB.W T2 (0xF89x) and LDRH.W T2 (0xF8Bx) wrongly
        // sign-extended, corrupting any byte/halfword with the top bit set
        // (e.g. a UDS SID 0x85 read back as 0xFFFFFF85).
        // Each sub-test uses a fresh cpu/bus: instruction memory at a fixed
        // address is treated as ROM by MockBus and will not accept a rewrite,
        // so rerunning at the same pc would refetch the first instruction.
        // LDRB.W R0, [R1, #0]  = F891 0000 — must ZERO-extend.
        {
            let mut cpu = CortexM::new();
            let mut bus = MockBus::new();
            cpu.pc = 0x2000;
            cpu.r1 = 0x3000;
            bus.write_u8(0x3000, 0x85).unwrap();
            run_test_instr(&mut cpu, &mut bus, 0xF8910000, true);
            assert_eq!(cpu.r0, 0x0000_0085, "LDRB.W must zero-extend 0x85");
        }
        // LDRSB.W R0, [R1, #0] = F991 0000 — must SIGN-extend.
        {
            let mut cpu = CortexM::new();
            let mut bus = MockBus::new();
            cpu.pc = 0x2000;
            cpu.r1 = 0x3000;
            bus.write_u8(0x3000, 0x85).unwrap();
            run_test_instr(&mut cpu, &mut bus, 0xF9910000, true);
            assert_eq!(cpu.r0, 0xFFFF_FF85, "LDRSB.W must sign-extend 0x85");
        }
        // LDRH.W R0, [R1, #0]  = F8B1 0000 — must ZERO-extend.
        {
            let mut cpu = CortexM::new();
            let mut bus = MockBus::new();
            cpu.pc = 0x2000;
            cpu.r1 = 0x3000;
            bus.write_u16(0x3000, 0x8042).unwrap();
            run_test_instr(&mut cpu, &mut bus, 0xF8B10000, true);
            assert_eq!(cpu.r0, 0x0000_8042, "LDRH.W must zero-extend 0x8042");
        }
        // LDRSH.W R0, [R1, #0] = F9B1 0000 — must SIGN-extend.
        {
            let mut cpu = CortexM::new();
            let mut bus = MockBus::new();
            cpu.pc = 0x2000;
            cpu.r1 = 0x3000;
            bus.write_u16(0x3000, 0x8042).unwrap();
            run_test_instr(&mut cpu, &mut bus, 0xF9B10000, true);
            assert_eq!(cpu.r0, 0xFFFF_8042, "LDRSH.W must sign-extend 0x8042");
        }
    }

    #[test]
    fn test_exec_ldrsb_reg_offset_positive() {
        // LDRSB R0, [R1, R2] — op=011 — 0101 011 010 001 000 = 0x5688
        // Positive byte (MSB clear): no sign extension needed.
        let mut cpu = CortexM::new();
        let mut bus = MockBus::new();
        cpu.pc = 0x2000;
        cpu.r1 = 0x3000;
        cpu.r2 = 0x0004;
        bus.write_u8(0x3004, 0x7F).unwrap(); // +127
        run_test_instr(&mut cpu, &mut bus, 0x5688, false);
        assert_eq!(cpu.r0, 0x0000007F);
    }

    #[test]
    fn test_exec_ldrsb_reg_offset_negative() {
        // LDRSB R0, [R1, R2] — sign-extends a byte with MSB set.
        let mut cpu = CortexM::new();
        let mut bus = MockBus::new();
        cpu.pc = 0x2000;
        cpu.r1 = 0x3000;
        cpu.r2 = 0x0000;
        bus.write_u8(0x3000, 0xFF).unwrap(); // -1 as i8
        run_test_instr(&mut cpu, &mut bus, 0x5688, false);
        // Sign-extended to 32 bits: 0xFFFFFFFF.
        assert_eq!(cpu.r0, 0xFFFFFFFF);
    }

    #[test]
    fn test_exec_ldrh_reg_offset() {
        // LDRH R0, [R1, R2] — op=101 — 0101 101 010 001 000 = 0x5A88
        // Zero-extends the 16-bit halfword.
        let mut cpu = CortexM::new();
        let mut bus = MockBus::new();
        cpu.pc = 0x2000;
        cpu.r1 = 0x4000;
        cpu.r2 = 0x0002;
        bus.write_u16(0x4002, 0xDEAD).unwrap();
        run_test_instr(&mut cpu, &mut bus, 0x5A88, false);
        assert_eq!(cpu.r0, 0x0000DEAD);
    }

    #[test]
    fn test_exec_ldmia_base_in_list_no_writeback() {
        // LDMIA R2, {R0, R1, R2} = 0xCA07 (base R2 is IN the list → no `!` →
        // no writeback; the LOADED value wins). Regression for the KW41Z-LCD
        // "blank cow" bug: the compiler's struct-copy / stacked-arg reload
        // idiom `ldmia rN, {..., rN}` had its final register clobbered by an
        // unconditional base writeback, silently dropping a loaded argument.
        let mut cpu = CortexM::new();
        let mut bus = MockBus::new();
        cpu.pc = 0x2000;
        cpu.r2 = 0x4000;
        bus.write_u32(0x4000, 0x1111_1111).unwrap(); // -> r0
        bus.write_u32(0x4004, 0x2222_2222).unwrap(); // -> r1
        bus.write_u32(0x4008, 0x0000_0014).unwrap(); // -> r2 (the loaded value)
        run_test_instr(&mut cpu, &mut bus, 0xCA07, false);
        assert_eq!(cpu.r0, 0x1111_1111);
        assert_eq!(cpu.r1, 0x2222_2222);
        // Must be the value loaded from [base+8], NOT base+12 (0x400C).
        assert_eq!(cpu.r2, 0x0000_0014, "base-in-list LDM must not write back");
    }

    #[test]
    fn test_exec_ldmia_base_not_in_list_writes_back() {
        // LDMIA R3!, {R0, R1} = 0xCB03 (base R3 NOT in list → writeback).
        let mut cpu = CortexM::new();
        let mut bus = MockBus::new();
        cpu.pc = 0x2000;
        cpu.r3 = 0x4000;
        bus.write_u32(0x4000, 0xAAAA_AAAA).unwrap();
        bus.write_u32(0x4004, 0xBBBB_BBBB).unwrap();
        run_test_instr(&mut cpu, &mut bus, 0xCB03, false);
        assert_eq!(cpu.r0, 0xAAAA_AAAA);
        assert_eq!(cpu.r1, 0xBBBB_BBBB);
        assert_eq!(
            cpu.r3, 0x4008,
            "base-not-in-list LDM must write back base+8"
        );
    }

    #[test]
    fn test_exec_ldrsh_reg_offset_negative() {
        // LDRSH R0, [R1, R2] — op=111 — 0101 111 010 001 000 = 0x5E88
        // Sign-extends a halfword with MSB set.
        let mut cpu = CortexM::new();
        let mut bus = MockBus::new();
        cpu.pc = 0x2000;
        cpu.r1 = 0x4000;
        cpu.r2 = 0x0000;
        bus.write_u16(0x4000, 0x8001).unwrap(); // negative i16
        run_test_instr(&mut cpu, &mut bus, 0x5E88, false);
        // Sign-extended: 0xFFFF8001.
        assert_eq!(cpu.r0, 0xFFFF8001);
    }

    #[test]
    fn test_exec_rev_t1_16bit() {
        // REV T1: `rev r3, r3` = 0xBA1B — byte-reverse a 32-bit word.
        // 0x11223344 → 0x44332211.
        {
            let mut cpu = CortexM::new();
            let mut bus = MockBus::new();
            cpu.pc = 0x2000;
            cpu.r3 = 0x1122_3344;
            run_test_instr(&mut cpu, &mut bus, 0xBA1B, false);
            assert_eq!(cpu.r3, 0x4433_2211, "REV T1 must byte-swap the whole word");
        }
        // REV16 T1: `rev16 r5, r5` = 0xBA6D — swap bytes within each halfword.
        // 0x11223344 → bytes in low half swapped + bytes in high half swapped
        // = 0x22114433.
        {
            let mut cpu = CortexM::new();
            let mut bus = MockBus::new();
            cpu.pc = 0x2000;
            cpu.r5 = 0x1122_3344;
            run_test_instr(&mut cpu, &mut bus, 0xBA6D, false);
            assert_eq!(
                cpu.r5, 0x2211_4433,
                "REV16 T1 must swap bytes within each halfword"
            );
        }
        // REVSH T1: `revsh r0, r1` = 0xBAC8 — swap low two bytes, sign-extend.
        // Input r1=0x00008001: low halfword bytes swapped → 0x0180, sign-extended
        // as i16 = 0x0180 (positive, MSB not set) → 0x00000180.
        {
            let mut cpu = CortexM::new();
            let mut bus = MockBus::new();
            cpu.pc = 0x2000;
            cpu.r1 = 0x0000_8001;
            run_test_instr(&mut cpu, &mut bus, 0xBAC8, false);
            assert_eq!(cpu.r0, 0x0000_0180, "REVSH T1 positive case");
        }
        // REVSH sign case: input r1=0x00000180 — low halfword bytes swapped
        // → 0x8001, sign-extended as i16 → 0xFFFF8001.
        {
            let mut cpu = CortexM::new();
            let mut bus = MockBus::new();
            cpu.pc = 0x2000;
            cpu.r1 = 0x0000_0180;
            run_test_instr(&mut cpu, &mut bus, 0xBAC8, false);
            assert_eq!(cpu.r0, 0xFFFF_8001, "REVSH T1 sign-extend case");
        }
    }

    #[test]
    fn nvic_ipr_priority_drives_irq_dispatch_order() {
        let mut cpu = CortexM::new();
        let nvic = Arc::new(crate::peripherals::nvic::NvicState::default());
        // IRQ0 priority = 0xC0, IRQ1 priority = 0x40. IRQ1 has higher
        // priority despite being a larger IRQ number.
        nvic.ipr[0].store(0x0000_40C0, Ordering::Relaxed);
        cpu.set_shared_nvic_state(nvic);

        assert_eq!(cpu.exception_priority(16), 0xC0); // IRQ0 → exc 16
        assert_eq!(cpu.exception_priority(17), 0x40); // IRQ1 → exc 17

        cpu.pending_exceptions[0] = (1u64 << 16) | (1u64 << 17);
        assert_eq!(
            cpu.highest_priority_pending(),
            Some(17),
            "IRQ1 (prio 0x40) outranks IRQ0 (prio 0xC0)"
        );
    }

    // --- Banked MSP/PSP + CONTROL.SPSEL + EXC_RETURN (ARMv7-M) ---

    /// Pend `exc`, point its vector at `handler`, and take the exception by
    /// stepping once. Assumes priority lets it through (thread mode / IRQ).
    fn take_exception(cpu: &mut CortexM, bus: &mut MockBus, exc: u32, handler: u32) {
        bus.write_u32((exc * 4) as u64, handler).unwrap();
        cpu.set_exception_pending(exc);
        cpu.step_internal(bus, &[], &bus.config.clone()).unwrap();
    }

    #[test]
    fn msr_psp_sets_psp_without_disturbing_msp() {
        // Thread mode, MSP active. MSR PSP, r0 must bank PSP and leave the
        // live MSP stack pointer untouched. MRS r1, PSP reads it back.
        let mut cpu = CortexM::new();
        let mut bus = MockBus::new();
        cpu.sp = 0x8000; // MSP active
        cpu.r0 = 0x6000;
        // MSR PSP, r0 = F380 8809
        run_test_instr(&mut cpu, &mut bus, 0xF380_8809, true);
        assert_eq!(cpu.psp, 0x6000, "MSR PSP banks the value");
        assert_eq!(cpu.sp, 0x8000, "MSP (active sp) untouched");
        // MSP is the live bank here, so `sp` is authoritative for it.
        assert_eq!(cpu.read_msp(), 0x8000, "MSP read unchanged");

        // MRS r1, PSP = F3EF 8109
        run_test_instr(&mut cpu, &mut bus, 0xF3EF_8109, true);
        assert_eq!(cpu.r1, 0x6000, "MRS PSP reads back banked value");
    }

    #[test]
    fn control_spsel_routes_thread_stack_to_psp() {
        // Set CONTROL.SPSEL=1 in thread mode → active stack becomes PSP, and
        // a PUSH must land on PSP, not MSP.
        let mut cpu = CortexM::new();
        let mut bus = MockBus::new();
        cpu.pc = 0x1000;
        cpu.sp = 0x8000; // MSP active
        cpu.psp = 0x6000;
        cpu.r0 = 0xDEAD_BEEF;

        // MSR CONTROL, r0 with r0=2 (SPSEL=1) = F380 8814
        cpu.r0 = 0x2;
        run_test_instr(&mut cpu, &mut bus, 0xF380_8814, true);
        assert_eq!(cpu.control & 0x2, 0x2, "CONTROL.SPSEL set");
        assert_eq!(cpu.msp, 0x8000, "leaving MSP banks its value");
        assert_eq!(cpu.sp, 0x6000, "active sp switched to PSP");

        // PUSH {r0} = B401 must decrement and write PSP.
        cpu.r0 = 0xDEAD_BEEF;
        run_test_instr(&mut cpu, &mut bus, 0x0000_B401, false);
        assert_eq!(cpu.sp, 0x5FFC, "PUSH used PSP");
        assert_eq!(bus.read_u32(0x5FFC).unwrap(), 0xDEAD_BEEF);

        // MRS r2, CONTROL = F3EF 8214
        run_test_instr(&mut cpu, &mut bus, 0xF3EF_8214, true);
        assert_eq!(cpu.r2 & 0x2, 0x2, "MRS CONTROL reflects SPSEL");
    }

    #[test]
    fn exception_entry_from_thread_psp_sets_exc_return_fd() {
        // Thread/PSP fault → LR=0xFFFFFFFD, frame stacked on PSP, handler on MSP.
        let mut cpu = CortexM::new();
        let mut bus = MockBus::new();
        cpu.pc = 0x1000;
        cpu.control = 0x2; // SPSEL=1, thread/PSP
        cpu.sp = 0x6000; // PSP active
        cpu.msp = 0x8000; // banked MSP
        cpu.r0 = 0x1111_1111;

        take_exception(&mut cpu, &mut bus, 16, 0x2000);

        assert_eq!(cpu.lr, 0xFFFF_FFFD, "EXC_RETURN = Thread/PSP");
        assert_eq!(cpu.active_exception, 16, "now in handler");
        assert_eq!(cpu.sp, 0x8000, "handler runs on MSP");
        assert_eq!(cpu.psp, 0x5FE0, "frame stacked on PSP (0x6000-32)");
        assert_eq!(cpu.pc, 0x2000, "branched to handler");
        assert_eq!(
            bus.read_u32(0x5FE0).unwrap(),
            0x1111_1111,
            "r0 on PSP frame"
        );
    }

    #[test]
    fn exception_entry_from_thread_msp_sets_exc_return_f9() {
        let mut cpu = CortexM::new();
        let mut bus = MockBus::new();
        cpu.pc = 0x1000;
        cpu.control = 0x0; // SPSEL=0, thread/MSP
        cpu.sp = 0x8000; // MSP active

        take_exception(&mut cpu, &mut bus, 16, 0x2000);

        assert_eq!(cpu.lr, 0xFFFF_FFF9, "EXC_RETURN = Thread/MSP");
        assert_eq!(cpu.sp, 0x7FE0, "handler on MSP (0x8000-32)");
        assert_eq!(cpu.msp, 0x7FE0, "MSP bank updated");
    }

    #[test]
    fn nested_exception_entry_sets_exc_return_f1() {
        // Already in a handler → nested exception returns to Handler mode (F1),
        // and stacks on MSP.
        let mut cpu = CortexM::new();
        let mut bus = MockBus::new();
        cpu.pc = 0x2000;
        cpu.active_exception = 11; // SVCall in progress (prio 0)
        cpu.sp = 0x8000; // handler MSP
        cpu.psp = 0x6000; // banked thread PSP, must NOT be touched

        // HardFault (exc 3, prio -1) preempts.
        take_exception(&mut cpu, &mut bus, 3, 0x3000);

        assert_eq!(cpu.lr, 0xFFFF_FFF1, "EXC_RETURN = return to Handler");
        assert_eq!(cpu.active_exception, 3);
        assert_eq!(cpu.sp, 0x7FE0, "nested frame on MSP");
        assert_eq!(cpu.msp, 0x7FE0, "MSP bank advanced");
        assert_eq!(cpu.psp, 0x6000, "PSP bank untouched by nested entry");
    }

    #[test]
    fn exception_round_trip_from_psp_restores_state() {
        // Enter from thread/PSP, BX LR back, PSP + registers restored exactly.
        let mut cpu = CortexM::new();
        let mut bus = MockBus::new();
        cpu.pc = 0x1000;
        cpu.control = 0x2; // thread/PSP
        cpu.sp = 0x6000; // PSP active
        cpu.msp = 0x8000;
        cpu.r0 = 0xA0;
        cpu.r1 = 0xA1;
        cpu.r2 = 0xA2;
        cpu.r3 = 0xA3;
        cpu.r12 = 0xAC;
        cpu.lr = 0x1000_0001;
        cpu.xpsr = 0x0100_0000;

        // Handler at 0x2000 is a single BX LR (0x4770).
        bus.write_u16(0x2000, 0x4770).unwrap();
        take_exception(&mut cpu, &mut bus, 16, 0x2000);
        assert_eq!(cpu.active_exception, 16);
        assert_eq!(cpu.sp, 0x8000, "in handler on MSP");

        // Execute BX LR (EXC_RETURN).
        let cfg = bus.config.clone();
        cpu.step_internal(&mut bus, &[], &cfg).unwrap();

        assert_eq!(cpu.active_exception, 0, "back to thread mode");
        assert_eq!(cpu.sp, 0x6000, "PSP restored");
        assert_eq!(cpu.control & 0x2, 0x2, "still thread/PSP");
        assert_eq!(cpu.pc, 0x1000, "PC restored from frame");
        assert_eq!(cpu.r0, 0xA0);
        assert_eq!(cpu.r1, 0xA1);
        assert_eq!(cpu.r2, 0xA2);
        assert_eq!(cpu.r3, 0xA3);
        assert_eq!(cpu.r12, 0xAC);
        assert_eq!(cpu.lr, 0x1000_0001, "LR restored from frame");
    }

    #[test]
    fn exception_return_to_msp_restores_msp() {
        // EXC_RETURN 0xFFFFFFF9 returns to thread/MSP.
        let mut cpu = CortexM::new();
        let mut bus = MockBus::new();
        cpu.pc = 0x1000;
        cpu.control = 0x0; // thread/MSP
        cpu.sp = 0x8000;
        bus.write_u16(0x2000, 0x4770).unwrap(); // BX LR
        take_exception(&mut cpu, &mut bus, 16, 0x2000);
        assert_eq!(cpu.lr, 0xFFFF_FFF9);

        let cfg = bus.config.clone();
        cpu.step_internal(&mut bus, &[], &cfg).unwrap();
        assert_eq!(cpu.active_exception, 0);
        assert_eq!(cpu.sp, 0x8000, "MSP restored");
        assert_eq!(cpu.control & 0x2, 0, "SPSEL stays MSP");
    }

    #[test]
    fn wfi_decodes_and_retires_like_nop() {
        // 0xBF30 must decode to the dedicated WFI variant (not the hint-space
        // Nop) and, with no wake event pending, arm the sleep flag while still
        // advancing PC like any 16-bit hint.
        assert_eq!(decode_thumb_16(0xBF30), Instruction::Wfi);
        let mut cpu = CortexM::new();
        let mut bus = MockBus::new();
        cpu.pc = 0x1000;
        run_test_instr(&mut cpu, &mut bus, 0xBF30, false); // WFI
        assert_eq!(cpu.pc, 0x1002, "WFI advances PC like a NOP");
        assert!(
            cpu.idle_fast_forward_budget(&bus).is_some(),
            "WFI with no pending event arms idle sleep"
        );
    }

    #[test]
    fn wfi_is_nop_when_wake_already_pending() {
        // If a wake-up event is already pending when WFI retires, WFI completes
        // as a plain NOP and never arms sleep. PRIMASK is set only so the
        // pending IRQ isn't taken before the WFI retires — this isolates the
        // WFI arm's wake-pending branch.
        let mut cpu = CortexM::new();
        let mut bus = MockBus::new();
        cpu.pc = 0x1000;
        cpu.primask = true;
        cpu.set_exception_pending(16); // IRQ0 pending (priority 0xFF < 256)
        run_test_instr(&mut cpu, &mut bus, 0xBF30, false); // WFI
        assert_eq!(cpu.pc, 0x1002, "WFI retires like a NOP");
        assert!(
            cpu.idle_fast_forward_budget(&bus).is_none(),
            "an already-pending wake event means WFI does not arm sleep"
        );
    }

    #[test]
    fn wfi_wakes_and_takes_exception_when_primask_clear() {
        // WFI with nothing pending sleeps; a subsequently-pended exception is a
        // wake event (budget clears), and with PRIMASK clear the next step
        // vectors into the handler.
        let mut cpu = CortexM::new();
        let mut bus = MockBus::new();
        cpu.pc = 0x1000;
        cpu.sp = 0x8000;
        bus.write_u16(0x1000, 0xBF30).unwrap(); // WFI
        let cfg = bus.config.clone();

        cpu.step_internal(&mut bus, &[], &cfg).unwrap();
        assert_eq!(cpu.pc, 0x1002, "WFI retires like a NOP");
        assert!(
            cpu.idle_fast_forward_budget(&bus).is_some(),
            "core is sleeping"
        );

        // SysTick (exception 15) pends while the core sleeps.
        bus.write_u32(15 * 4, 0x5000 | 1).unwrap(); // VTOR=0 → vector[15]
        cpu.set_exception_pending(15);
        assert!(
            cpu.idle_fast_forward_budget(&bus).is_none(),
            "a pended exception wakes the core"
        );

        cpu.step_internal(&mut bus, &[], &cfg).unwrap();
        assert_eq!(
            cpu.active_exception, 15,
            "PRIMASK clear → the exception is taken"
        );
        assert_eq!(cpu.pc, 0x5000, "vectored into the SysTick handler");
    }

    #[test]
    fn wfi_primask_set_wakes_without_taking() {
        // The canonical `__disable_irq(); wfi();` idle pattern: PRIMASK is set,
        // so a pended exception must WAKE the core (budget clears) but must NOT
        // be taken — the core falls through to the instruction after WFI and the
        // exception stays pending.
        let mut cpu = CortexM::new();
        let mut bus = MockBus::new();
        cpu.pc = 0x1000;
        cpu.sp = 0x8000;
        cpu.primask = true; // __disable_irq()
        bus.write_u16(0x1000, 0xBF30).unwrap(); // WFI
        bus.write_u16(0x1002, 0xBF00).unwrap(); // NOP (the fall-through target)
        let cfg = bus.config.clone();

        cpu.step_internal(&mut bus, &[], &cfg).unwrap();
        assert_eq!(cpu.pc, 0x1002);
        assert!(
            cpu.idle_fast_forward_budget(&bus).is_some(),
            "core sleeps even with PRIMASK set"
        );

        bus.write_u32(15 * 4, 0x5000 | 1).unwrap();
        cpu.set_exception_pending(15);
        assert!(
            cpu.idle_fast_forward_budget(&bus).is_none(),
            "wake-on-pend fires despite PRIMASK"
        );

        cpu.step_internal(&mut bus, &[], &cfg).unwrap();
        assert_eq!(
            cpu.active_exception, 0,
            "PRIMASK blocks entry: the exception is NOT taken"
        );
        assert_eq!(cpu.pc, 0x1004, "core falls through past the WFI");
        assert!(
            cpu.pending_exceptions[0] & (1 << 15) != 0,
            "the masked exception stays pending"
        );
    }

    /// Build the minimal `WFI; b .-2` idle loop on a real `SystemBus` and run it
    /// with the legacy walk disabled so idle fast-forward is legal.
    fn wfi_idle_machine(ff: bool) -> Machine<CortexM> {
        let mut bus = SystemBus::new();
        bus.write_u16(0x0, 0xBF30).unwrap(); // WFI
        bus.write_u16(0x2, 0xE7FD).unwrap(); // B -> 0x0
        let mut cpu = CortexM::new();
        cpu.pc = 0x0;
        cpu.sp = 0x8000;
        let mut machine = Machine::new(cpu, bus);
        machine.config.idle_fast_forward_enabled = ff;
        machine.bus.legacy_walk_disabled = true;
        machine
    }

    #[test]
    fn wfi_fast_forward_is_off_by_default() {
        use crate::DebugControl;
        let mut machine = wfi_idle_machine(false);
        machine.run(Some(10)).unwrap();
        assert_eq!(machine.total_cycles, 10);
        assert_eq!(
            machine.step_profile().cpu_instructions,
            10,
            "without fast-forward every idle cycle retires an instruction"
        );
    }

    #[test]
    fn wfi_fast_forward_skips_cpu_work_when_enabled() {
        // Requires the idle fast-forward machinery (event-scheduler feature),
        // which the `--workspace` and `--features event-scheduler` CI lanes
        // build. Determinism: total_cycles is identical to the off case; only
        // the retired-instruction count drops.
        use crate::DebugControl;
        let mut off = wfi_idle_machine(false);
        off.run(Some(10)).unwrap();

        let mut on = wfi_idle_machine(true);
        on.run(Some(10)).unwrap();

        assert_eq!(
            on.total_cycles, off.total_cycles,
            "idle fast-forward must not change total_cycles"
        );
        assert_eq!(on.total_cycles, 10);
        if cfg!(feature = "event-scheduler") {
            assert!(
                on.step_profile().cpu_instructions < off.step_profile().cpu_instructions,
                "fast-forwarded cycles should not retire CPU instructions"
            );
        }
    }

    #[test]
    fn boxed_cortexm_batch_preserves_idle_fast_forward_escape() {
        // The `Box<dyn Cpu>` forwarding must still leave the batch loop at WFI so
        // the machine can fast-forward (the WASM runtime holds a boxed CPU).
        use crate::DebugControl;
        let mut bus = SystemBus::new();
        bus.write_u16(0x0, 0xBF30).unwrap(); // WFI
        bus.write_u16(0x2, 0xE7FD).unwrap(); // B -> 0x0
        let mut cpu = CortexM::new();
        cpu.pc = 0x0;
        cpu.sp = 0x8000;
        let mut machine = Machine::new(Box::new(cpu) as Box<dyn Cpu>, bus);
        machine.config.idle_fast_forward_enabled = true;
        machine.bus.legacy_walk_disabled = true;
        machine.run(Some(10)).unwrap();
        assert_eq!(machine.total_cycles, 10);
        if cfg!(feature = "event-scheduler") {
            assert!(
                machine.step_profile().cpu_instructions < 10,
                "boxed CPU path should still leave the batch loop at WFI"
            );
        }
    }
}
