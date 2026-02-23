// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

use crate::{Bus, Cpu, SimResult, SimulationConfig, SimulationError, SimulationObserver};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

/// Types of faults that can be injected into the simulation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Fault {
    /// Skip the next instruction by advancing PC.
    InstructionSkip,
    /// Flip a bit in a specific register.
    RegisterBitFlip { register_id: u8, bit: u8 },
    /// Flip a bit at a specific memory address.
    MemoryBitFlip { address: u64, bit: u8 },
}

/// The Shadow Engine provides lockstep execution of two CPU instances
/// to detect and analyze the effects of fault injection.
pub struct ShadowEngine {
    pub golden: Box<dyn Cpu>,
    pub shadow: Box<dyn Cpu>,
}

impl ShadowEngine {
    pub fn new(golden: Box<dyn Cpu>, shadow: Box<dyn Cpu>) -> Self {
        Self { golden, shadow }
    }

    /// Execute one step in lockstep across both golden and shadow cores.
    pub fn step(
        &mut self,
        bus: &mut dyn Bus,
        observers: &[Arc<dyn SimulationObserver>],
        config: &SimulationConfig,
    ) -> SimResult<()> {
        // 1. Step golden core
        self.golden.step(bus, observers, config)?;

        // 2. Step shadow core
        self.shadow.step(bus, observers, config)?;

        // 3. Compare state (lockstep parity check)
        self.check_parity()
    }

    /// Check parity between golden and shadow core registers.
    pub fn check_parity(&self) -> SimResult<()> {
        Self::check_parity_between(self.golden.as_ref(), self.shadow.as_ref())
    }

    pub fn check_parity_between(golden: &dyn Cpu, shadow: &dyn Cpu) -> SimResult<()> {
        let golden_snap = golden.snapshot();
        let shadow_snap = shadow.snapshot();

        // Note: We need a generic way to compare CpuSnapshots.
        // For now, we perform a deep comparison via serialization or direct field check.
        // In a high-performance implementation, this would be a raw byte comparison.

        match (golden_snap, shadow_snap) {
            (crate::snapshot::CpuSnapshot::Arm(g), crate::snapshot::CpuSnapshot::Arm(s)) => {
                if g.registers != s.registers || g.xpsr != s.xpsr {
                    return Err(SimulationError::Other(format!(
                        "Lockstep mismatch detected (Arm)! Golden PC: 0x{:08X}, Shadow PC: 0x{:08X}",
                        g.registers[15], s.registers[15]
                    )));
                }
            }
            (crate::snapshot::CpuSnapshot::RiscV(g), crate::snapshot::CpuSnapshot::RiscV(s)) => {
                if g.registers != s.registers || g.pc != s.pc {
                    return Err(SimulationError::Other(format!(
                        "Lockstep mismatch detected (Risc-V)! Golden PC: 0x{:08X}, Shadow PC: 0x{:08X}",
                        g.pc, s.pc
                    )));
                }
            }
            _ => {
                return Err(SimulationError::Other(
                    "Incompatible CPU architectures in lockstep".to_string(),
                ))
            }
        }

        Ok(())
    }

    /// Inject a fault into the shadow core.
    pub fn inject_fault(&mut self, bus: &mut dyn Bus, fault: Fault) -> SimResult<()> {
        match fault {
            Fault::InstructionSkip => {
                let pc = self.shadow.get_pc();
                // Simple heuristic for Thumb-2/RV32: advance by 2 or 4?
                // For now, we assume 2 for Thumb, but this should be instruction-aware.
                self.shadow.set_pc(pc + 2);
            }
            Fault::RegisterBitFlip { register_id, bit } => {
                let val = self.shadow.get_register(register_id);
                self.shadow.set_register(register_id, val ^ (1 << bit));
            }
            Fault::MemoryBitFlip { address, bit } => {
                let val = bus.read_u8(address)?;
                bus.write_u8(address, val ^ (1 << bit))?;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bus::SystemBus;
    use crate::cpu::CortexM;
    use crate::system::cortex_m::configure_cortex_m;

    fn setup_shadow_engine() -> (ShadowEngine, SystemBus) {
        let mut bus_g = SystemBus::new();
        let (cpu_g, _) = configure_cortex_m(&mut bus_g);

        let mut bus_s = SystemBus::new();
        let (cpu_s, _) = configure_cortex_m(&mut bus_s);

        (ShadowEngine::new(Box::new(cpu_g), Box::new(cpu_s)), bus_g)
    }

    #[test]
    fn test_lockstep_parity() {
        let (mut engine, mut bus) = setup_shadow_engine();
        let observers = vec![];
        let config = SimulationConfig::default();

        // Initially in sync
        assert!(engine.check_parity().is_ok());

        // Step once (empty/NOP)
        engine.step(&mut bus, &observers, &config).unwrap();
        assert!(engine.check_parity().is_ok());
    }

    #[test]
    fn test_lockstep_mismatch() {
        let (mut engine, _) = setup_shadow_engine();

        // Induce mismatch manually
        engine.shadow.set_register(0, 0x1234);

        let result = engine.check_parity();
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("Lockstep mismatch detected"));
    }

    #[test]
    fn test_fault_injection_register() {
        let (mut engine, _) = setup_shadow_engine();

        engine.golden.set_register(0, 0x0);
        engine.shadow.set_register(0, 0x0);

        engine
            .inject_fault(
                &mut SystemBus::new(),
                Fault::RegisterBitFlip {
                    register_id: 0,
                    bit: 0,
                },
            )
            .unwrap();

        assert_eq!(engine.shadow.get_register(0), 1);
        assert_eq!(engine.golden.get_register(0), 0);
        assert!(engine.check_parity().is_err());
    }

    #[test]
    fn test_multicore_lockstep() {
        let mut bus = SystemBus::new();
        let (cpu0, _) = configure_cortex_m(&mut bus);
        let (cpu1, _) = configure_cortex_m(&mut bus);

        let mut mc = crate::multi_core::MultiCoreMachine::new(bus);
        mc.add_core(Box::new(cpu0));
        mc.add_core(Box::new(cpu1));
        mc.lockstep = true;

        // Sync
        let results = mc.step_all();
        assert!(results.iter().all(|r| r.is_ok()));

        // Mismatch
        mc.cores[1].set_register(0, 0xBAD);
        let results = mc.step_all();
        assert!(results.iter().any(|r| r.is_err()));
    }
}
