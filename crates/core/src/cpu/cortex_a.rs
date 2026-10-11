// LabWired - Firmware Simulation Platform
// SPDX-License-Identifier: MIT

//! Optional native Cortex-A7, using Unicorn's embedded QEMU-derived CPU.
//! See docs/qemu-arm-core.md. This is a CPU integration, not RV1103/Linux support.

use crate::snapshot::{CortexACpuSnapshot, CpuSnapshot};
use crate::{
    Bus, Cpu, FetchPermission, SimResult, SimulationConfig, SimulationError, SimulationObserver,
};
use std::collections::BTreeSet;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::Arc;
use unicorn_engine::unicorn_const::{
    uc_error, Arch, ArmCpuModel, HookType, MemType, Mode, Prot, RegisterARMCP,
};
use unicorn_engine::{RegisterARM, UcHookId, Unicorn};

const PAGE: u64 = 4096;
const THUMB: u32 = 1 << 5;
const REGS: [RegisterARM; 17] = [
    RegisterARM::R0,
    RegisterARM::R1,
    RegisterARM::R2,
    RegisterARM::R3,
    RegisterARM::R4,
    RegisterARM::R5,
    RegisterARM::R6,
    RegisterARM::R7,
    RegisterARM::R8,
    RegisterARM::R9,
    RegisterARM::R10,
    RegisterARM::R11,
    RegisterARM::R12,
    RegisterARM::SP,
    RegisterARM::LR,
    RegisterARM::PC,
    RegisterARM::CPSR,
];

fn native_error(error: uc_error) -> SimulationError {
    SimulationError::Other(format!("embedded Cortex-A7: {error:?}"))
}

/// Engine ownership never leaves this type; no Unicorn clones or persistent
/// callbacks are exposed. LabWired's bus remains authoritative.
pub struct CortexA7 {
    engine: Option<Unicorn<'static, ()>>,
    pages: BTreeSet<u64>,
    refused: Option<String>,
    halted: bool,
    unavailable_registers: [u32; 17],
}

// SAFETY: Unicorn supports independent engines on different threads. The
// private Rc is never cloned outside a synchronous step; temporary wrappers
// and hooks are removed before that step returns. Cpu requires &mut self to
// run, so moving an idle owner cannot race an execution. This type is not Sync.
unsafe impl Send for CortexA7 {}

impl CortexA7 {
    pub fn new() -> SimResult<Self> {
        let mut engine = Unicorn::new(Arch::ARM, Mode::ARM).map_err(native_error)?;
        engine
            .ctl_set_cpu_model(ArmCpuModel::CORTEX_A7 as i32)
            .map_err(native_error)?;
        engine
            .reg_write(RegisterARM::CPSR, 0xd3)
            .map_err(native_error)?;
        Ok(Self {
            engine: Some(engine),
            pages: BTreeSet::new(),
            refused: None,
            halted: false,
            unavailable_registers: [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0xd3],
        })
    }

    fn engine(&self) -> &Unicorn<'static, ()> {
        self.engine
            .as_ref()
            .expect("native engine unavailable after callback cleanup failure")
    }

    fn write_reg(&mut self, id: u8, value: u32) {
        if let Some(reg) = REGS.get(id as usize) {
            if let Some(engine) = &mut self.engine {
                if let Err(error) = engine.reg_write(*reg, u64::from(value)) {
                    self.refused = Some(format!("register write failed: {error:?}"));
                }
            }
        }
    }

    fn check_execution_mode(&self) -> SimResult<()> {
        // Unicorn treats a whole Thumb IT block as one counted instruction.
        // Accepting restored ITSTATE would bypass our per-instruction bus
        // fetch/permission check even if the IT opcode itself was rejected.
        if self.get_register(16) & ((3 << 25) | (0x3f << 10)) != 0 {
            return Err(SimulationError::NotImplemented(
                "Cortex-A7 Thumb IT-state single stepping".into(),
            ));
        }
        let mut sctlr = RegisterARMCP::new().cp(15).crn(1);
        self.engine()
            .reg_read_arm_coproc(&mut sctlr)
            .map_err(native_error)?;
        if sctlr.val & 1 != 0 {
            return Err(SimulationError::NotImplemented(
                "Cortex-A7 MMU bus translation (Linux execution requires this)".into(),
            ));
        }
        if self.get_register(16) & (1 << 9) != 0 {
            return Err(SimulationError::NotImplemented(
                "Cortex-A7 big-endian execution".into(),
            ));
        }
        Ok(())
    }
}

struct StepBus<'a> {
    bus: &'a mut dyn Bus,
    pages: &'a mut BTreeSet<u64>,
    error: Option<SimulationError>,
}

/// The foreign handle is borrowed only for one step. Cleanup runs on every
/// return path and unwind. If removal fails, close the owning engine BEFORE
/// the callback storage/bus borrow expires; no dangling hook can run later.
struct StepEngine<'a> {
    uc: Unicorn<'a, StepBus<'a>>,
    hooks: Vec<UcHookId>,
    owner: &'a mut Option<Unicorn<'static, ()>>,
    unavailable_registers: &'a mut [u32; 17],
}

impl Drop for StepEngine<'_> {
    fn drop(&mut self) {
        for hook in self.hooks.drain(..) {
            if self.uc.remove_hook(hook).is_err() {
                // The CLI still asks for diagnostics after an execution error.
                // Preserve visible state before retiring the unsafe engine.
                for (i, reg) in REGS.iter().enumerate() {
                    if let Ok(value) = self.uc.reg_read(*reg) {
                        self.unavailable_registers[i] = value as u32;
                    }
                }
                drop(self.owner.take());
                break;
            }
        }
    }
}

fn ensure_pages(uc: &mut Unicorn<'_, StepBus<'_>>, address: u64, size: usize) -> SimResult<()> {
    let end = address
        .checked_add(size as u64)
        .filter(|end| *end <= 1u64 << 32)
        .ok_or(SimulationError::MemoryViolation(address))?;
    if size == 0 {
        return Err(SimulationError::MemoryViolation(address));
    }
    let first = address & !(PAGE - 1);
    let last = (end - 1) & !(PAGE - 1);
    for page in (first..=last).step_by(PAGE as usize) {
        if !uc.get_data().pages.contains(&page) {
            uc.mem_map(page, PAGE, Prot::ALL).map_err(native_error)?;
            uc.get_data_mut().pages.insert(page);
        }
    }
    Ok(())
}

fn data_access(
    uc: &mut Unicorn<'_, StepBus<'_>>,
    kind: MemType,
    address: u64,
    size: usize,
    value: i64,
) -> SimResult<()> {
    if !matches!(size, 1 | 2 | 4) {
        return Err(SimulationError::NotImplemented(format!(
            "Cortex-A7 {size}-byte bus access"
        )));
    }
    match kind {
        MemType::READ => {
            let value = match size {
                1 => u32::from(uc.get_data().bus.read_u8(address)?),
                2 => u32::from(uc.get_data().bus.read_u16(address)?),
                _ => uc.get_data().bus.read_u32(address)?,
            };
            ensure_pages(uc, address, size)?;
            uc.mem_write(address, &value.to_le_bytes()[..size])
                .map_err(native_error)?;
        }
        MemType::WRITE => {
            match size {
                1 => uc.get_data_mut().bus.write_u8(address, value as u8)?,
                2 => uc.get_data_mut().bus.write_u16(address, value as u16)?,
                _ => uc.get_data_mut().bus.write_u32(address, value as u32)?,
            }
            ensure_pages(uc, address, size)?;
        }
        _ => return Err(SimulationError::MemoryViolation(address)),
    }
    Ok(())
}

fn stop_with_error(uc: &mut Unicorn<'_, StepBus<'_>>, error: SimulationError) {
    if uc.get_data().error.is_none() {
        uc.get_data_mut().error = Some(error);
    }
    if let Err(stop_error) = uc.emu_stop() {
        uc.get_data_mut().error = Some(native_error(stop_error));
    }
}

impl Cpu for CortexA7 {
    fn reset(&mut self, _bus: &mut dyn Bus) -> SimResult<()> {
        *self = Self::new()?;
        Ok(())
    }
    fn uses_elf_entry_point(&self) -> bool {
        true
    }

    fn step(
        &mut self,
        bus: &mut dyn Bus,
        observers: &[Arc<dyn SimulationObserver>],
        _config: &SimulationConfig,
    ) -> SimResult<()> {
        if self.halted {
            return Err(SimulationError::Halt);
        }
        if let Some(reason) = &self.refused {
            return Err(SimulationError::NotImplemented(reason.clone()));
        }
        if self.engine.is_none() {
            return Err(SimulationError::Other(
                "Cortex-A7 callback cleanup failed; reset required".into(),
            ));
        }
        self.check_execution_mode()?;
        let pc = self.get_pc();
        if bus.check_fetch_permission(u64::from(pc)) != FetchPermission::Allowed {
            return Err(SimulationError::MemoryViolation(u64::from(pc)));
        }
        let thumb = self.get_register(16) & THUMB != 0;
        let low = bus.read_u16(u64::from(pc))?;
        let size = if thumb && !matches!(low >> 11, 0b11101..=0b11111) {
            2
        } else {
            4
        };
        if bus.check_fetch_permission(u64::from(pc) + size as u64 - 1) != FetchPermission::Allowed {
            return Err(SimulationError::MemoryViolation(
                u64::from(pc) + size as u64 - 1,
            ));
        }
        let mut bytes = [0u8; 4];
        bytes[..2].copy_from_slice(&low.to_le_bytes());
        if size == 4 {
            bytes[2..].copy_from_slice(&bus.read_u16(u64::from(pc) + 2)?.to_le_bytes());
        }
        let opcode = u32::from_le_bytes(bytes);
        if thumb && low & 0xff00 == 0xbf00 && low & 0xf != 0 {
            return Err(SimulationError::NotImplemented(
                "Cortex-A7 Thumb IT-block single stepping".into(),
            ));
        }
        if (thumb && matches!(low, 0xbf20 | 0xbf30))
            || (!thumb && matches!(opcode & 0x0fffffff, 0x0320f002 | 0x0320f003))
        {
            return Err(SimulationError::NotImplemented(
                "Cortex-A7 WFE/WFI event scheduling".into(),
            ));
        }
        for observer in observers {
            observer.on_step_start(pc, opcode);
        }

        {
            let handle = self.engine().get_handle();
            // SAFETY: handle is owned by self.engine and remains alive for
            // this entire scope. The wrapper is non-owning. No callback or
            // native invocation escapes StepEngine's cleanup guard.
            let uc = unsafe {
                Unicorn::from_handle_with_data(
                    handle,
                    StepBus {
                        bus,
                        pages: &mut self.pages,
                        error: None,
                    },
                )
            }
            .map_err(native_error)?;
            let mut scoped = StepEngine {
                uc,
                hooks: Vec::new(),
                owner: &mut self.engine,
                unavailable_registers: &mut self.unavailable_registers,
            };
            ensure_pages(&mut scoped.uc, u64::from(pc), size)?;
            scoped
                .uc
                .mem_write(u64::from(pc), &bytes[..size])
                .map_err(native_error)?;
            scoped
                .uc
                .ctl_remove_cache(u64::from(pc), u64::from(pc) + size as u64)
                .map_err(native_error)?;
            let memory_hook = scoped
                .uc
                .add_mem_hook(
                    HookType::MEM_READ
                        | HookType::MEM_WRITE
                        | HookType::MEM_READ_UNMAPPED
                        | HookType::MEM_WRITE_UNMAPPED
                        | HookType::MEM_FETCH_UNMAPPED,
                    1,
                    0,
                    |uc, kind, address, size, value| {
                        if uc.get_data().error.is_some() {
                            return false;
                        }
                        // Never unwind through Unicorn's C callback boundary.
                        let result = catch_unwind(AssertUnwindSafe(|| {
                            match kind {
                                // Reads fault before the READ hook. Writes
                                // use a cached mapping lookup taken before
                                // WRITE: even if WRITE mapped it, the fault
                                // hook must acknowledge the refreshed map.
                                MemType::READ_UNMAPPED | MemType::WRITE_UNMAPPED => {
                                    ensure_pages(uc, address, size)
                                }
                                // TCG may prefetch past the instruction. Only
                                // the single bus-validated instruction can
                                // execute (count=1); the next step validates
                                // its own PC/bytes before any native execution.
                                MemType::FETCH_UNMAPPED => ensure_pages(uc, address, size),
                                _ => data_access(uc, kind, address, size, value),
                            }
                        }));
                        match result {
                            Ok(Ok(())) => true,
                            Ok(Err(error)) => {
                                stop_with_error(uc, error);
                                false
                            }
                            Err(_) => {
                                stop_with_error(
                                    uc,
                                    SimulationError::Other(
                                        "Cortex-A7 bus callback panicked".into(),
                                    ),
                                );
                                false
                            }
                        }
                    },
                )
                .map_err(native_error)?;
            scoped.hooks.push(memory_hook);
            let exception_hook = scoped
                .uc
                .add_intr_hook(|uc, cause| {
                    stop_with_error(
                        uc,
                        SimulationError::NotImplemented(format!(
                            "Cortex-A7 architectural exception {cause}"
                        )),
                    );
                })
                .map_err(native_error)?;
            scoped.hooks.push(exception_hook);
            let result = scoped
                .uc
                .emu_start(u64::from(pc) | u64::from(thumb), 1u64 << 32, 0, 1);
            if let Some(error) = scoped.uc.get_data_mut().error.take() {
                return Err(error);
            }
            result.map_err(native_error)?;
        }
        if self.engine.is_none() {
            return Err(SimulationError::Other(
                "Cortex-A7 callback cleanup failed; reset required".into(),
            ));
        }
        self.check_execution_mode()?;
        crate::emit_trace_event(
            observers,
            labwired_hw_trace::TraceEvent::InstructionRetired { pc, opcode },
        );
        if !observers.is_empty() {
            let mut registers: Vec<u32> = (0..17).map(|i| self.get_register(i)).collect();
            registers.extend([self.get_register(13), self.get_pc()]);
            for observer in observers {
                observer.on_step_end(1, &registers);
            }
        }
        Ok(())
    }

    fn set_pc(&mut self, value: u32) {
        let cpsr = (self.get_register(16) & !THUMB) | if value & 1 != 0 { THUMB } else { 0 };
        self.write_reg(16, cpsr);
        // Unicorn's PC write uses bit 0 to select the execution state.
        self.write_reg(15, value);
    }
    fn get_pc(&self) -> u32 {
        self.get_register(15)
    }
    fn set_sp(&mut self, value: u32) {
        self.write_reg(13, value);
    }
    fn set_exception_pending(&mut self, exception: u32) {
        self.refused = Some(format!(
            "Cortex-A7 IRQ delivery ({exception}); GIC/exception entry not connected"
        ));
    }
    fn get_register(&self, id: u8) -> u32 {
        REGS.get(id as usize)
            .map(|reg| {
                self.engine
                    .as_ref()
                    .and_then(|engine| engine.reg_read(*reg).ok())
                    .map(|value| value as u32)
                    .unwrap_or(self.unavailable_registers[id as usize])
            })
            .unwrap_or(0)
    }
    fn set_register(&mut self, id: u8, value: u32) {
        if id == 15 {
            self.set_pc(value);
        } else {
            self.write_reg(id, value);
        }
    }
    fn snapshot(&self) -> CpuSnapshot {
        CpuSnapshot::CortexA(CortexACpuSnapshot {
            registers: (0..16).map(|i| self.get_register(i)).collect(),
            pc: self.get_pc(),
            cpsr: self.get_register(16),
        })
    }
    fn apply_snapshot(&mut self, _snapshot: &CpuSnapshot) {
        self.refused =
            Some("Cortex-A7 full snapshot restore (banked registers and CP15 state)".into());
    }
    fn try_apply_snapshot(&mut self, _snapshot: &CpuSnapshot) -> SimResult<()> {
        Err(SimulationError::NotImplemented(
            "Cortex-A7 full snapshot restore (banked registers and CP15 state)".into(),
        ))
    }
    fn get_register_names(&self) -> Vec<String> {
        (0..13)
            .map(|i| format!("r{i}"))
            .chain(["sp", "lr", "pc", "cpsr"].map(str::to_owned))
            .collect()
    }
    fn index_of_register(&self, name: &str) -> Option<u8> {
        self.get_register_names()
            .iter()
            .position(|candidate| candidate.eq_ignore_ascii_case(name))
            .map(|i| i as u8)
    }
    fn halt(&mut self) {
        self.halted = true;
    }
    fn unhalt(&mut self) {
        self.halted = false;
    }
}
