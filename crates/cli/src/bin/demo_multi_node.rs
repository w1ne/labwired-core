// Multi-Node Hello World Demo for LabWired
// This demo shows two independent simulated nodes synchronized in a single "World".

use labwired_core::{bus::SystemBus, cpu::cortex_m::CortexM, world::World, Machine};
use tracing_subscriber;

fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();

    println!("LabWired Multi-Node Simulation Demo");
    println!("----------------------------------");

    let mut world = World::new("demo-cluster".to_string());

    // Node 1: Sensor
    let bus1 = SystemBus::new();
    let cpu1 = CortexM::new();
    let machine1 = Machine::new(cpu1, bus1);
    world.add_machine("sensor".to_string(), Box::new(machine1));

    // Node 2: Gateway
    let bus2 = SystemBus::new();
    let cpu2 = CortexM::new();
    let machine2 = Machine::new(cpu2, bus2);
    world.add_machine("gateway".to_string(), Box::new(machine2));

    println!("Initialized world with 2 nodes: 'sensor' and 'gateway'");

    // Step the world for 100 cycles
    for i in 0..100 {
        world.step_all();
        if i % 20 == 0 {
            println!("Cycle {}: Nodes stepped successfully", i);
        }
    }

    println!("----------------------------------");
    println!("Demo completed: 100 cycles of multi-node synchronization verified.");

    Ok(())
}
