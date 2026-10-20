// Regression gate for `impl Cpu for Box<dyn Cpu>`.
//
// The browser runtime holds `Machine<Box<dyn Cpu>>`. Every method that has a
// default on the trait must be forwarded explicitly, or the boxed machine
// silently keeps the default while the concrete CPU uses its override. That
// exact shape has already bitten this tree twice: `as_any_mut` disabled every
// downcast (including the browser JIT dispatcher), and `invalidate_code_caches`
// left stale decoded instructions after a debugger write.
//
// This test uses a spy core whose overrides are observable through the boxed
// trait object. It is deliberately narrow: it pins the hooks that are both
// easy to forget and observable from the browser, not every method.

use std::sync::Arc;

use labwired_core::bus::SystemBus;
use labwired_core::snapshot;
use labwired_core::{
    Bus, Cpu, SecondaryExecutionState, SimResult, SimulationConfig, SimulationObserver,
};

#[derive(Debug, Default)]
struct SpyCpu {
    invalidated: usize,
    exit: Option<u32>,
    vtor: Option<u32>,
}

impl SpyCpu {
    fn new() -> Self {
        Self {
            exit: Some(42),
            ..Self::default()
        }
    }
}

impl Cpu for SpyCpu {
    fn reset(&mut self, _bus: &mut dyn Bus) -> SimResult<()> {
        Ok(())
    }

    fn step(
        &mut self,
        _bus: &mut dyn Bus,
        _observers: &[Arc<dyn SimulationObserver>],
        _config: &SimulationConfig,
    ) -> SimResult<()> {
        Ok(())
    }

    fn set_pc(&mut self, _val: u32) {}
    fn get_pc(&self) -> u32 {
        0
    }
    fn set_sp(&mut self, _val: u32) {}
    fn set_exception_pending(&mut self, _exception_num: u32) {}
    fn get_register(&self, _id: u8) -> u32 {
        0
    }
    fn set_register(&mut self, _id: u8, _val: u32) {}
    fn get_register_names(&self) -> Vec<String> {
        vec!["pc".to_string()]
    }
    fn index_of_register(&self, _name: &str) -> Option<u8> {
        None
    }
    fn snapshot(&self) -> snapshot::CpuSnapshot {
        snapshot::CpuSnapshot::Avr(snapshot::AvrCpuSnapshot {
            registers: Vec::new(),
            pc: 0,
            sp: 0,
            sreg: 0,
        })
    }
    fn apply_snapshot(&mut self, _snapshot: &snapshot::CpuSnapshot) {}

    // The hooks this gate exists for.
    fn as_any_mut(&mut self) -> Option<&mut dyn std::any::Any> {
        Some(self)
    }
    fn invalidate_code_caches(&mut self) {
        self.invalidated += 1;
    }
    fn take_firmware_exit(&mut self) -> Option<u32> {
        self.exit.take()
    }
    fn set_vector_table_base(&mut self, base: u32) -> bool {
        self.vtor = Some(base);
        true
    }
    fn secondary_execution_state(&self) -> SecondaryExecutionState {
        SecondaryExecutionState::ParkedIdle
    }
    fn supports_semihosting(&self) -> bool {
        true
    }
    fn idle_fast_forward_budget(&self, _bus: &dyn Bus) -> Option<u64> {
        Some(1234)
    }
}

#[test]
fn boxed_dyn_cpu_forwards_browser_only_hooks() {
    let mut cpu: Box<dyn Cpu> = Box::new(SpyCpu::new());

    assert_eq!(cpu.take_firmware_exit(), Some(42));
    assert!(cpu.set_vector_table_base(0x2000_0000));
    assert_eq!(
        cpu.secondary_execution_state(),
        SecondaryExecutionState::ParkedIdle
    );
    assert!(cpu.supports_semihosting());
    assert_eq!(cpu.idle_fast_forward_budget(&SystemBus::new()), Some(1234));

    cpu.invalidate_code_caches();

    let spy = cpu
        .as_any_mut()
        .expect("Box<dyn Cpu> must forward as_any_mut")
        .downcast_mut::<SpyCpu>()
        .expect("the forwarded pointer must reach the concrete core");
    assert_eq!(spy.invalidated, 1);
    assert_eq!(spy.vtor, Some(0x2000_0000));
    assert_eq!(spy.exit, None, "take_firmware_exit must be destructive");
}
