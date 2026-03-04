// LabWired - Multi-Node IoT Integration Test
// Demonstrates: Controller (CAN) -> Actuator (CAN + Wireless) -> Monitor (Wireless)

use labwired_core::bus::SystemBus;
use labwired_core::cpu::cortex_m::CortexM;
use labwired_core::network::{CanBus, WirelessBus};
use labwired_core::peripherals::can::CanController;
use labwired_core::peripherals::radio::RadioController;
use labwired_core::world::World;
use labwired_core::{Machine, SimResult};

#[test]
fn test_real_world_iot_chain() {
    let mut world = World::new("iot-system".to_string());

    // 1. Setup Interconnects
    let mut can_bus = CanBus::new();
    let (tx_can_ctrl, rx_can_ctrl) = can_bus.attach();
    let (tx_can_act, rx_can_act) = can_bus.attach();
    world.add_interconnect(Box::new(can_bus));

    let mut wireless_bus = WirelessBus::new();
    let (tx_radio_act, rx_radio_act) = wireless_bus.attach();
    let (tx_radio_mon, rx_radio_mon) = wireless_bus.attach();
    world.add_interconnect(Box::new(wireless_bus));

    // 2. Setup Machines (Nodes)
    let can_base = 0x4000_A000;
    let radio_base = 0x4000_B000;

    // Node 1: Controller
    let mut bus1 = SystemBus::new();
    bus1.peripherals.push(labwired_core::bus::PeripheralEntry {
        name: "can_ctrl".to_string(),
        base: can_base,
        size: 0x100,
        irq: None,
        dev: Box::new(CanController::new(tx_can_ctrl, rx_can_ctrl)),
        ticks_remaining: 0,
    });
    world.add_machine(
        "controller".to_string(),
        Box::new(Machine::new(CortexM::new(), bus1)),
    );

    // Node 2: Actuator (Bridge)
    let mut bus2 = SystemBus::new();
    bus2.peripherals.push(labwired_core::bus::PeripheralEntry {
        name: "can_act".to_string(),
        base: can_base,
        size: 0x100,
        irq: None,
        dev: Box::new(CanController::new(tx_can_act, rx_can_act)),
        ticks_remaining: 0,
    });
    bus2.peripherals.push(labwired_core::bus::PeripheralEntry {
        name: "radio_act".to_string(),
        base: radio_base,
        size: 0x100,
        irq: None,
        dev: Box::new(RadioController::new(tx_radio_act, rx_radio_act)),
        ticks_remaining: 0,
    });
    world.add_machine(
        "actuator".to_string(),
        Box::new(Machine::new(CortexM::new(), bus2)),
    );

    // Node 3: Monitor
    let mut bus3 = SystemBus::new();
    bus3.peripherals.push(labwired_core::bus::PeripheralEntry {
        name: "radio_mon".to_string(),
        base: radio_base,
        size: 0x100,
        irq: None,
        dev: Box::new(RadioController::new(tx_radio_mon, rx_radio_mon)),
        ticks_remaining: 0,
    });
    world.add_machine(
        "monitor".to_string(),
        Box::new(Machine::new(CortexM::new(), bus3)),
    );

    // 3. Scenario Configuration

    // Set both radios to Channel 15
    world
        .machines
        .get_mut("actuator")
        .unwrap()
        .write_u8(radio_base + 0, 15)
        .unwrap();
    world
        .machines
        .get_mut("monitor")
        .unwrap()
        .write_u8(radio_base + 0, 15)
        .unwrap();

    // 4. Step 1: Controller sends CAN Command (ID=0x55, Data=0x01)
    let controller = world.machines.get_mut("controller").unwrap();
    controller.write_u8(can_base + 0, 0x55).unwrap(); // TX ID
    controller.write_u8(can_base + 4, 0x01).unwrap(); // TX Data
    controller.write_u8(can_base + 8, 1).unwrap(); // Trigger TX

    // Step the world a few times to ensure propagation
    for _ in 0..5 {
        world.step_all();
    }

    // 5. Step 2: Actuator processes CAN and triggers Wireless
    let actuator = world.machines.get_mut("actuator").unwrap();

    // Check if CAN msg arrived
    let can_status = actuator.read_u8(can_base + 0x08).unwrap();
    assert_eq!(can_status, 1, "Actuator should have received CAN msg");

    let can_id = actuator.read_u8(can_base + 0x0C).unwrap();
    assert_eq!(can_id, 0x55);

    // Actuator bridges command to Wireless (Trigger Radio TX)
    actuator.write_u8(radio_base + 0x08, 1).unwrap();

    // Step world a few more times for wireless propagation
    for _ in 0..5 {
        world.step_all();
    }

    // 6. Step 3: Monitor receives Wireless Status
    let monitor = world.machines.get_mut("monitor").unwrap();

    // Check Wireless status
    let radio_status = monitor.read_u8(radio_base + 0x0C).unwrap();
    assert_eq!(
        radio_status, 1,
        "Monitor should have received Wireless packet"
    );

    let radio_ch = monitor.read_u8(radio_base + 0x10).unwrap();
    assert_eq!(radio_ch, 15);
}
