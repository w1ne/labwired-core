// Copyright (C) 2026 LabWired. MIT.
// Focused integration checks, avoiding a complete lib-test binary build.
use labwired_core::analog::{parse_netlist, Integration, Solver};
use labwired_core::peripherals::components::declarative_spi::GenericSpiDevice;
use labwired_core::peripherals::device::SpiDevice;
use labwired_core::sim_input::SimInput;

fn dev() -> GenericSpiDevice {
    GenericSpiDevice::from_yaml(
        labwired_config::embedded_device_yaml("bma400").expect("descriptor"),
        "CSB",
    )
    .expect("BMA400 descriptor loads")
}
fn read(d: &mut GenericSpiDevice, address: u8, len: usize) -> Vec<u8> {
    d.cs_select();
    assert_eq!(d.transfer(0x80 | address), 0x00);
    assert_eq!(d.transfer(0), 0x00, "the first reply byte is always dummy");
    let result = (0..len).map(|_| d.transfer(0)).collect();
    d.cs_release();
    result
}
fn write(d: &mut GenericSpiDevice, address: u8, value: u8) {
    d.cs_select();
    d.transfer(address);
    d.transfer(value);
    d.cs_release();
}

#[test]
fn bma400_chip_id_requires_chip_select_rising_edge_and_read_dummy() {
    let mut d = dev();
    d.cs_select();
    assert_eq!(d.transfer(0x80), 0xFF);
    assert_eq!(d.transfer(0x00), 0xFF);
    d.cs_release();
    assert_eq!(read(&mut d, 0x00, 1), [0x90]);
    assert_eq!(read(&mut d, 0x00, 2), [0x90, 0x88]);
}

#[test]
fn bma400_stimulus_x_y_z_signed_registers_and_range_setting() {
    let mut d = dev();
    d.cs_select();
    d.cs_release();
    d.set_input("accel_x", 1.0).unwrap();
    d.set_input("accel_y", -1.0).unwrap();
    d.set_input("accel_z", 0.5).unwrap();
    // 0x49 => +-4 g = 512 counts/g, little-endian signed 12-bit.
    assert_eq!(read(&mut d, 0x04, 6), [0x00, 0x02, 0x00, 0x0E, 0x00, 0x01]);
    write(&mut d, 0x1A, 0x09); // +-2 g = 1024 counts/g
    assert_eq!(read(&mut d, 0x04, 6), [0x00, 0x04, 0x00, 0x0C, 0x00, 0x02]);
}

#[test]
fn analog_supply_voltage_is_from_actual_mna_solver_not_input_echo() {
    let circuit = parse_netlist(
        "V1 source 0 DC 3.3\nR1 source supply 10\nC1 supply 0 10u\nRLOAD supply 0 100k\n.end\n",
    )
    .unwrap();
    let mut solver = Solver::new(circuit, Integration::BackwardEuler).unwrap();
    let node = solver.circuit().node("supply").unwrap();
    let index = solver.voltage_source_index("V1").unwrap();
    let before = solver.node_voltage(node);
    assert!(before > 3.29 && before < 3.3, "before: {before}");
    solver.set_voltage_source(index, 1.0);
    solver.advance(100e-6).unwrap();
    let filtered = solver.node_voltage(node);
    assert!(filtered > 1.0 && filtered < before, "filtered: {filtered}");
}
