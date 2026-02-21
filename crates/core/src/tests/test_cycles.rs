use crate::{Bus, Cpu, DebugControl, Machine, SimResult, SimulationObserver, StopReason};
use std::sync::Arc;

#[derive(Default)]
struct MockCpu {
    pc: u32,
}

impl Cpu for MockCpu {
    fn reset(&mut self, _bus: &mut dyn Bus) -> SimResult<()> {
        Ok(())
    }
    fn step(
        &mut self,
        _bus: &mut dyn Bus,
        _observers: &[Arc<dyn SimulationObserver>],
        _config: &crate::SimulationConfig,
    ) -> SimResult<()> {
        self.pc += 2;
        Ok(())
    }
    fn set_pc(&mut self, val: u32) {
        self.pc = val;
    }
    fn get_pc(&self) -> u32 {
        self.pc
    }
    fn set_sp(&mut self, _val: u32) {}
    fn set_exception_pending(&mut self, _exception_num: u32) {}
    fn get_register(&self, _id: u8) -> u32 {
        0
    }
    fn set_register(&mut self, _id: u8, _val: u32) {}
    fn snapshot(&self) -> crate::snapshot::CpuSnapshot {
        crate::snapshot::CpuSnapshot::Arm(crate::snapshot::ArmCpuSnapshot {
            registers: vec![0; 16],
            xpsr: 0,
            primask: false,
            pending_exceptions: 0,
            vtor: 0,
        })
    } // Dummy
    fn apply_snapshot(&mut self, _snapshot: &crate::snapshot::CpuSnapshot) {}
    fn get_register_names(&self) -> Vec<String> {
        vec![]
    }
}

#[test]
fn test_machine_run_cycles() {
    let cpu = MockCpu::default();
    let bus = crate::bus::SystemBus::new();
    let mut machine = Machine::new(cpu, bus);

    assert_eq!(machine.total_cycles, 0);

    // Run 100 steps
    let reason = machine.run(Some(100)).unwrap();
    assert_eq!(reason, StopReason::MaxStepsReached);
    assert_eq!(machine.total_cycles, 100);

    // Run 50 steps
    let reason = machine.run(Some(50)).unwrap();
    assert_eq!(reason, StopReason::MaxStepsReached);
    assert_eq!(machine.total_cycles, 150);
}
