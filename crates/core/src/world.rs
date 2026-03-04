// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

use crate::network::Interconnect;
use crate::{Cpu, Machine, SimResult};
use std::collections::HashMap;

/// The orchestrator for a multi-node simulation environment.
///
/// A `World` manages multiple independent `Machine` instances, each with its
/// own address space and clock context, and handles their synchronization.
pub struct World {
    pub name: String,
    pub machines: HashMap<String, Box<dyn MachineTrait>>,
    pub interconnects: Vec<Box<dyn Interconnect>>,
}

/// Type-erased trait for machines to allow heterogeneous machines in the world.
pub trait MachineTrait: Send {
    fn name(&self) -> &str;
    fn step(&mut self) -> SimResult<()>;
    fn reset(&mut self) -> SimResult<()>;
    fn total_cycles(&self) -> u64;
}

impl<C: Cpu + 'static> MachineTrait for Machine<C> {
    fn name(&self) -> &str {
        // We might need to add a name field to Machine or handle mapping in World
        "unnamed"
    }

    fn step(&mut self) -> SimResult<()> {
        self.step()
    }

    fn reset(&mut self) -> SimResult<()> {
        self.reset()
    }

    fn total_cycles(&self) -> u64 {
        self.total_cycles
    }
}

impl World {
    pub fn new(name: String) -> Self {
        Self {
            name,
            machines: HashMap::new(),
            interconnects: Vec::new(),
        }
    }

    pub fn add_machine(&mut self, id: String, machine: Box<dyn MachineTrait>) {
        self.machines.insert(id, machine);
    }

    pub fn add_interconnect(&mut self, interconnect: Box<dyn Interconnect>) {
        self.interconnects.push(interconnect);
    }

    /// Step all machines in the world.
    ///
    /// This is the simplest synchronization strategy: step every machine once.
    /// Future improvements will include Global Virtual Time (GVT) and
    /// Chandy-Lamport for distributed snapshots.
    pub fn step_all(&mut self) -> HashMap<String, SimResult<()>> {
        let mut results = HashMap::new();
        for (id, machine) in &mut self.machines {
            results.insert(id.clone(), machine.step());
        }
        for interconnect in &mut self.interconnects {
            if let Err(e) = interconnect.tick() {
                eprintln!("Interconnect error: {:?}", e);
            }
        }
        results
    }

    pub fn reset_all(&mut self) -> HashMap<String, SimResult<()>> {
        let mut results = HashMap::new();
        for (id, machine) in &mut self.machines {
            results.insert(id.clone(), machine.reset());
        }
        results
    }

    /// Load a simulation environment from a manifest.
    pub fn from_manifest(
        _manifest: labwired_config::EnvironmentManifest,
        _root_dir: &std::path::Path,
    ) -> anyhow::Result<Self> {
        // Implementation will involve:
        // 1. Parsing SystemManifest for each node
        // 2. Initializing Machines with correct CPUs
        // 3. Loading ELF binaries
        // 4. Setting up interconnects
        anyhow::bail!("Loading from manifest not yet implemented")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bus::SystemBus;
    use crate::cpu::cortex_m::CortexM;

    #[test]
    fn test_multi_node_basic_sync() {
        let mut world = World::new("test-world".to_string());

        let bus1 = SystemBus::new();
        let cpu1 = CortexM::new();
        let machine1 = Machine::new(cpu1, bus1);

        let bus2 = SystemBus::new();
        let cpu2 = CortexM::new();
        let machine2 = Machine::new(cpu2, bus2);

        world.add_machine("node1".to_string(), Box::new(machine1));
        world.add_machine("node2".to_string(), Box::new(machine2));

        // Step the world
        let results = world.step_all();
        assert_eq!(results.len(), 2);
        assert!(results.get("node1").unwrap().is_ok());
        assert!(results.get("node2").unwrap().is_ok());

        assert_eq!(world.machines.get("node1").unwrap().total_cycles(), 1);
        assert_eq!(world.machines.get("node2").unwrap().total_cycles(), 1);
    }

    use crate::network::CanBus;
    use crate::peripherals::can::CanController;
    use crate::Peripheral;

    #[test]
    fn test_can_bus_transmission() {
        let mut world = World::new("test-can".to_string());

        let mut can_bus = CanBus::new();
        let (tx1, rx1) = can_bus.attach();
        let (tx2, rx2) = can_bus.attach();

        world.add_interconnect(Box::new(can_bus));

        let mut can1 = CanController::new(tx1, rx1);
        let mut can2 = CanController::new(tx2, rx2);

        can1.write(0x00, 0xAA).unwrap();
        can1.write(0x04, 0x12).unwrap();
        can1.write(0x05, 0x34).unwrap();
        can1.write(0x08, 0x01).unwrap();

        let _ = world.step_all();

        let _ = can2.tick();

        let status = can2.read(0x08).unwrap();
        assert_eq!(status, 1, "RX pending should be 1");

        let rx_id = can2.read(0x0C).unwrap();
        assert_eq!(rx_id, 0xAA);

        let rx_data_0 = can2.read(0x10).unwrap();
        let rx_data_1 = can2.read(0x11).unwrap();
        assert_eq!(rx_data_0, 0x12);
        assert_eq!(rx_data_1, 0x34);
    }
}
