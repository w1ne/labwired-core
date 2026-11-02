// LabWired - Firmware Simulation Platform
// SPDX-License-Identifier: MIT
//! Optional privileged Cortex-A7. Physical RAM remains owned by the Bus;
//! translation, banked registers and exception entry remain native.
use crate::snapshot::{CortexACpuSnapshot, CpuSnapshot};
use crate::{Bus, Cpu, SimResult, SimulationConfig, SimulationError, SimulationObserver};
pub use labwired_arm_system::ArmModel;
use labwired_arm_system::{ArmCpu, PhysicalBus};
use std::cell::RefCell;
use std::sync::Arc;

fn native_error(message: String) -> SimulationError {
    SimulationError::Other(format!("Cortex-A7 system: {message}"))
}
struct State {
    cpu: ArmCpu,
    registers: [u32; 17],
    pending_error: Option<SimulationError>,
}
/// Owns a single isolated native engine. Infallible Cpu register methods keep
/// their first failure for the next fallible execution instead of discarding it.
pub struct CortexA7System {
    state: RefCell<State>,
    waiting: bool,
    halted: bool,
    cp15_written: bool,
}
impl CortexA7System {
    pub fn new() -> SimResult<Self> {
        Self::with_model(ArmModel::CortexA7)
    }
    /// The same privileged ARMv7 engine built as another pinned tlib model.
    /// `CortexR5F` is PMSAv7 (16-region MPU, no MMU) with VFPv3-D16, as on the
    /// Zynq UltraScale+ RPU; the bus-owned memory and IRQ/FIQ contract is
    /// unchanged.
    pub fn with_model(model: ArmModel) -> SimResult<Self> {
        let mut cpu = ArmCpu::with_model(model).map_err(native_error)?;
        let mut registers = [0; 17];
        for (n, v) in registers.iter_mut().enumerate() {
            *v = cpu.register(n as u8).map_err(native_error)?;
        }
        Ok(Self {
            state: RefCell::new(State {
                cpu,
                registers,
                pending_error: None,
            }),
            waiting: false,
            halted: false,
            cp15_written: false,
        })
    }
    /// Physical executable regions, never virtual aliases. The bus provides
    /// authoritative bytes when the native MMU resolves a fetch.
    pub fn executable_range(&mut self, start: u64, length: u64) -> SimResult<()> {
        self.state
            .get_mut()
            .cpu
            .executable_range(start, length)
            .map_err(native_error)
    }
    /// Checked reset-state Linux entry: ARM/SVC, IRQ+FIQ masked, MMU/cache
    /// remain off from a freshly created native engine. Failures are returned
    /// immediately so callers can keep RAM unchanged.
    pub fn initialize_linux(&mut self, entry: u32, dtb: u32) -> SimResult<()> {
        // Reset privileged state synchronously, even when the caller has used
        // the CPU before. Linux requires MMU/cache off at this entry point.
        self.state.get_mut().cpu.reset().map_err(native_error)?;
        self.waiting = false;
        self.halted = false;
        self.cp15_written = false;
        let s = self.state.get_mut();
        s.pending_error = None;
        for (id, value) in [(16, 0xd3), (0, 0), (1, u32::MAX), (2, dtb), (15, entry)] {
            s.cpu.set_register(id, value).map_err(native_error)?;
            s.registers[id as usize] = s.cpu.register(id).map_err(native_error)?;
            if s.registers[id as usize] != value {
                return Err(native_error(
                    "Linux entry register readback mismatch".into(),
                ));
            }
        }
        Ok(())
    }
    fn defer(&mut self, error: SimulationError) {
        let s = self.state.get_mut();
        if s.pending_error.is_none() {
            s.pending_error = Some(error);
        }
    }
    fn write_register(&mut self, id: u8, value: u32) {
        let s = self.state.get_mut();
        match s.cpu.set_register(id, value) {
            Ok(()) => {
                if let Some(v) = s.registers.get_mut(id as usize) {
                    *v = value;
                }
            }
            Err(error) => {
                if s.pending_error.is_none() {
                    s.pending_error = Some(native_error(error));
                }
            }
        }
    }
}
struct Physical<'a> {
    bus: &'a mut dyn Bus,
    identity: u64,
    error: Option<SimulationError>,
    cp15_written: bool,
}
impl Physical<'_> {
    fn result<T>(&mut self, result: SimResult<T>) -> Result<T, String> {
        result.map_err(|e| {
            let message = e.to_string();
            if self.error.is_none() {
                self.error = Some(e);
            }
            message
        })
    }
}
impl PhysicalBus for Physical<'_> {
    fn code_identity(&self) -> u64 {
        self.identity
    }
    fn read(&mut self, a: u64, w: u32, state: u64) -> Result<u64, String> {
        // This pinned backend supplies transaction state zero. Bus has no
        // privilege/security metadata interface; refuse a future nonzero state.
        if state != 0 {
            return self.result(Err(SimulationError::NotImplemented(format!(
                "ARM transaction state {state:#x}"
            ))));
        }
        let result = match w {
            1 => self.bus.read_u8(a).map(u64::from),
            2 => self.bus.read_u16(a).map(u64::from),
            4 => self.bus.read_u32(a).map(u64::from),
            8 => self.bus.read_u64(a),
            _ => Err(SimulationError::NotImplemented(format!(
                "ARM physical read width {w}"
            ))),
        };
        self.result(result)
    }
    fn write(&mut self, a: u64, w: u32, v: u64, state: u64) -> Result<(), String> {
        // This pinned backend supplies transaction state zero. Bus has no
        // privilege/security metadata interface; refuse a future nonzero state.
        if state != 0 {
            return self.result(Err(SimulationError::NotImplemented(format!(
                "ARM transaction state {state:#x}"
            ))));
        }
        let result = match w {
            1 => self.bus.write_u8(a, v as u8),
            2 => self.bus.write_u16(a, v as u16),
            4 => self.bus.write_u32(a, v as u32),
            8 => self.bus.write_u64(a, v),
            _ => Err(SimulationError::NotImplemented(format!(
                "ARM physical write width {w}"
            ))),
        };
        self.result(result)
    }
    fn cp15_read(&mut self, k: u32, w: bool) -> Result<u64, String> {
        let result = self.bus.arm_cp15_read(k, w);
        self.result(result)
    }
    fn cp15_write(&mut self, k: u32, w: bool, v: u64) -> Result<(), String> {
        self.cp15_written = true;
        let result = self.bus.arm_cp15_write(k, w, v);
        self.result(result)
    }
}
impl Cpu for CortexA7System {
    fn reset(&mut self, _bus: &mut dyn Bus) -> SimResult<()> {
        let s = self.state.get_mut();
        s.cpu.reset().map_err(native_error)?;
        for n in 0..17 {
            s.registers[n] = s.cpu.register(n as u8).map_err(native_error)?;
        }
        s.pending_error = None;
        self.waiting = false;
        self.halted = false;
        self.cp15_written = false;
        Ok(())
    }
    fn step(
        &mut self,
        bus: &mut dyn Bus,
        observers: &[Arc<dyn SimulationObserver>],
        config: &SimulationConfig,
    ) -> SimResult<()> {
        self.step_retired(bus, observers, config).map(|_| ())
    }
    fn step_retired(
        &mut self,
        bus: &mut dyn Bus,
        observers: &[Arc<dyn SimulationObserver>],
        _config: &SimulationConfig,
    ) -> SimResult<u32> {
        let s = self.state.get_mut();
        if let Some(error) = s.pending_error.take() {
            return Err(error);
        }
        if self.halted {
            return Err(SimulationError::Halt);
        }
        let identity = bus.arm_code_identity().ok_or_else(|| {
            SimulationError::NotImplemented(
                "native ARM requires a stable allocation-unique bus code identity".into(),
            )
        })?;
        let (irq, fiq) = bus.arm_irq_fiq_levels().unwrap_or((false, false));
        s.cpu.interrupt_levels(irq, fiq).map_err(native_error)?;
        self.cp15_written = false;
        let mut physical = Physical {
            bus,
            identity,
            error: None,
            cp15_written: false,
        };
        // Native IRQ entry can make progress without retiring an instruction.
        // Resolve only a bounded number of such transitions; parked WFI returns
        // immediately and never fabricates an instruction or observer event.
        for _ in 0..8 {
            let result = if observers.is_empty() {
                s.cpu.execute(&mut physical, 1).map(|e| (e, None))
            } else {
                s.cpu.execute_traced(&mut physical)
            };
            if let Some(error) = physical.error.take() {
                return Err(error);
            }
            let (execution, trace) = result.map_err(native_error)?;
            self.waiting = execution.waiting;
            self.cp15_written |= physical.cp15_written;
            if execution.retired == 0 {
                if execution.waiting {
                    return Ok(0);
                }
                continue;
            }
            if execution.retired != 1 {
                return Err(SimulationError::Other(
                    "native single step retired more than one instruction".into(),
                ));
            }
            if !observers.is_empty() {
                let trace = trace.ok_or_else(|| {
                    SimulationError::Other(
                        "native retired instruction has no trace encoding".into(),
                    )
                })?;
                for n in 0..17 {
                    s.registers[n] = s.cpu.register(n as u8).map_err(native_error)?;
                }
                // Delivery follows execution: a pre-execution notification would
                // report phantom retirement for IRQ transitions or native faults.
                for o in observers {
                    o.on_step_start(trace.pc, trace.opcode);
                }
                crate::emit_trace_event(
                    observers,
                    labwired_hw_trace::TraceEvent::InstructionRetired {
                        pc: trace.pc,
                        opcode: trace.opcode,
                    },
                );
                let mut registers = s.registers.to_vec();
                registers.extend([s.registers[13], s.registers[15]]);
                for o in observers {
                    o.on_step_end(1, &registers);
                }
            }
            return Ok(1);
        }
        Err(SimulationError::Other(
            "native ARM exceeded eight zero-retirement exception transitions".into(),
        ))
    }
    fn step_batch(
        &mut self,
        bus: &mut dyn Bus,
        observers: &[Arc<dyn SimulationObserver>],
        config: &SimulationConfig,
        max_count: u32,
    ) -> SimResult<u32> {
        let start = bus.current_cycle();
        let tap = bus.logic_tap().filter(|t| t.push_armed());
        let stamp = tap.as_ref().map(|t| t.clock());
        let mut retired = 0;
        for _ in 0..max_count {
            bus.publish_cycle(start + u64::from(retired));
            if let (Some(t), Some(stamp)) = (&tap, stamp) {
                t.set_clock(stamp + u64::from(retired) + 1);
            }
            let n = self.step_retired(bus, observers, config)?;
            if n == 0 {
                if let (Some(t), Some(stamp)) = (&tap, stamp) {
                    t.set_clock(stamp + u64::from(retired));
                }
                break;
            }
            retired += n;
            bus.publish_cycle(start + u64::from(retired));
            let new_interrupt = bus
                .arm_irq_fiq_levels()
                .is_some_and(|(irq, fiq)| irq || fiq);
            if self.waiting
                || self.cp15_written
                || new_interrupt
                || tap.as_ref().is_some_and(|t| t.batch_stop_hit())
            {
                break;
            }
        }
        Ok(retired)
    }
    fn set_pc(&mut self, v: u32) {
        self.write_register(15, v);
    }
    fn get_pc(&self) -> u32 {
        self.get_register(15)
    }
    fn set_sp(&mut self, v: u32) {
        self.write_register(13, v);
    }
    fn set_exception_pending(&mut self, n: u32) {
        self.defer(SimulationError::NotImplemented(format!(
            "NVIC exception {n} cannot be delivered to Cortex-A7; use ARM IRQ/FIQ bus levels"
        )));
    }
    fn get_register(&self, id: u8) -> u32 {
        let mut s = self.state.borrow_mut();
        match s.cpu.register(id) {
            Ok(v) => {
                if let Some(slot) = s.registers.get_mut(id as usize) {
                    *slot = v;
                }
                v
            }
            Err(e) => {
                if s.pending_error.is_none() {
                    s.pending_error = Some(native_error(e));
                }
                s.registers.get(id as usize).copied().unwrap_or(0)
            }
        }
    }
    fn set_register(&mut self, id: u8, v: u32) {
        self.write_register(id, v);
    }
    fn invalidate_code_caches(&mut self) {
        if let Err(e) = self.state.get_mut().cpu.invalidate_code() {
            self.defer(native_error(e));
        }
    }
    fn snapshot(&self) -> CpuSnapshot {
        CpuSnapshot::CortexA(CortexACpuSnapshot {
            registers: (0..16).map(|n| self.get_register(n)).collect(),
            pc: self.get_pc(),
            cpsr: self.get_register(16),
        })
    }
    fn apply_snapshot(&mut self, _: &CpuSnapshot) {
        self.defer(SimulationError::NotImplemented(
            "Cortex-A7 snapshot is diagnostic; hidden/banked CP15 state cannot be restored".into(),
        ));
    }
    fn try_apply_snapshot(&mut self, _: &CpuSnapshot) -> SimResult<()> {
        Err(SimulationError::NotImplemented(
            "Cortex-A7 snapshot is diagnostic; hidden/banked CP15 state cannot be restored".into(),
        ))
    }
    fn uses_elf_entry_point(&self) -> bool {
        true
    }
    fn get_register_names(&self) -> Vec<String> {
        (0..13)
            .map(|n| format!("r{n}"))
            .chain(["sp", "lr", "pc", "cpsr"].into_iter().map(String::from))
            .collect()
    }
    fn index_of_register(&self, name: &str) -> Option<u8> {
        self.get_register_names()
            .iter()
            .position(|n| n.eq_ignore_ascii_case(name))
            .map(|n| n as u8)
    }
    fn is_parked_idle(&self) -> bool {
        self.waiting
    }
    fn idle_fast_forward_budget(&self, bus: &dyn Bus) -> Option<u64> {
        if !self.waiting
            || bus
                .arm_irq_fiq_levels()
                .is_some_and(|(irq, fiq)| irq || fiq)
        {
            return None;
        }
        bus.next_cpu_interrupt_deadline_cycle()?
            .checked_sub(bus.current_cycle())
            .filter(|n| *n > 0)
    }
    fn halt(&mut self) {
        self.halted = true;
    }
    fn unhalt(&mut self) {
        self.halted = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn nonzero_transaction_state_is_refused_before_read_or_write() {
        let mut bus = crate::bus::SystemBus::new();
        let a = bus.ram.base_addr;
        let mut physical = Physical {
            bus: &mut bus,
            identity: 1,
            error: None,
            cp15_written: false,
        };
        assert!(physical.read(a, 4, 1).is_err());
        assert!(matches!(
            physical.error.take(),
            Some(SimulationError::NotImplemented(_))
        ));
        assert!(physical.write(a, 4, 42, 2).is_err());
        assert!(matches!(
            physical.error.take(),
            Some(SimulationError::NotImplemented(_))
        ));
        assert_eq!(bus.read_u32(a).unwrap(), 0);
    }
}
