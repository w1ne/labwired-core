// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! IBT-2 control polarity, cross-port enables and the existing physical DC
//! motor plant must agree. A real analogWrite fixture also exercises Timer0
//! PWM edges through the AVR port mirror into the driver observer and plant.

use std::path::PathBuf;

use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::bus::SystemBus;
use labwired_core::cpu::Avr;
use labwired_core::peripherals::components::h_bridge_motor::HBridgeMotor;
use labwired_core::system::cortex_m::configure_cortex_m;
use labwired_core::{Bus, DebugControl, Machine};

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn bus(enables: bool) -> SystemBus {
    let chip = ChipDescriptor::from_file(root().join("configs/chips/atmega328p.yaml")).unwrap();
    let bridge_enable = if enables {
        "      l_en_pin: PD7\n      r_en_pin: PB0\n"
    } else {
        "      l_en_pin: 5V\n      r_en_pin: 5V\n"
    };
    let motor_enable = if enables {
        "      enable_pin: PD7\n      enable2_pin: PB0\n"
    } else {
        ""
    };
    let manifest = SystemManifest::from_yaml(&format!(
        r#"
name: desk-bridge
chip: atmega328p
external_devices:
  - id: bridge
    type: bts7960
    connection: gpio
    config:
      lpwm_pin: PD6
      rpwm_pin: PD5
{bridge_enable}  - id: motor
    type: dc-motor
    connection: gpio
    config:
      in1_pin: PD5
      in2_pin: PD6
      pwm_pin: 5V
      both_inputs_low: brake
{motor_enable}      simulation_clock_hz: 16000000
      resistance_ohm: 6.0
      inductance_h: 0.002
      torque_constant_nm_per_a: 0.08
      back_emf_constant_v_per_rad_s: 0.08
      rotor_inertia_kg_m2: 0.00004
      viscous_friction_nm_per_rad_s: 0.00001
      supply_voltage_v: 12.0
      load_torque_nm: 0.0
      encoder_cpr: 1024
"#
    ))
    .unwrap();
    SystemBus::from_config(&chip, &manifest).unwrap()
}

fn effort(bus: &SystemBus) -> f32 {
    bus.observed_of::<HBridgeMotor>().next().unwrap().effort()
}

fn run_motor(bus: &mut SystemBus, start: u64, cycles: u64) {
    for cycle in (start..start + cycles).step_by(1000) {
        bus.set_current_cycle(cycle);
        bus.tick_peripherals_fully();
    }
}

#[test]
fn shared_gpio_changes_both_bts_enable_terminals() {
    let bridge = HBridgeMotor::new_ibt2("bridge", 1, 2, Some(3), Some(3));
    bridge.on_gpio_edge(3, true, 0); // L_EN and R_EN are wired together.
    bridge.on_gpio_edge(2, true, 1); // RPWM drives M+.
    assert_eq!(bridge.effort(), 1.0);
    bridge.on_gpio_edge(3, false, 2);
    assert_eq!(bridge.effort(), 0.0);
    bridge.on_gpio_edge(2, false, 3);
    bridge.on_gpio_edge(1, true, 4); // LPWM drives M-.
    bridge.on_gpio_edge(3, true, 5);
    assert_eq!(bridge.effort(), -1.0);

    // Equal PWM terminals cannot produce a differential motor voltage,
    // including when they are physically tied to the same output pad.
    let tied_pwm = HBridgeMotor::new_ibt2("tied-pwm", 1, 1, Some(3), Some(3));
    tied_pwm.on_gpio_edge(3, true, 0);
    tied_pwm.on_gpio_edge(1, true, 1);
    assert_eq!(tied_pwm.effort(), 0.0);
}

#[test]
fn bts7960_on_uno_observes_rpwm_positive_and_both_enables_across_ports() {
    let mut bus = bus(true);
    bus.write_u8(0x1002a, 0xe0).unwrap(); // DDRD: D5/D6/D7 outputs
    bus.write_u8(0x10024, 0x01).unwrap(); // DDRB: D8 output
    assert_eq!(effort(&bus), 0.0);
    bus.write_u8(0x1002b, 0xa0).unwrap(); // RPWM and L_EN
    assert_eq!(effort(&bus), 0.0, "R_EN low must inhibit the driver");
    bus.write_u8(0x10025, 1).unwrap(); // R_EN
    assert_eq!(effort(&bus), 1.0, "RPWM drives M+ positive");
    run_motor(&mut bus, 0, 800_000);
    let forward = bus.motor_snapshots()[0].clone();
    assert_eq!(forward.control_state, "forward");
    assert!(forward.speed_rpm > 10.0, "{forward:?}");
    bus.write_u8(0x1002b, 0xc0).unwrap(); // LPWM and L_EN
    assert_eq!(effort(&bus), -1.0);
    run_motor(&mut bus, 800_000, 3_200_000);
    let reverse = bus.motor_snapshots()[0].clone();
    assert_eq!(reverse.control_state, "reverse");
    assert!(reverse.speed_rpm < -10.0, "{reverse:?}");
    // R_EN alone cuts the real plant as well as the lightweight observer.
    bus.write_u8(0x10025, 0).unwrap();
    run_motor(&mut bus, 4_000_000, 32_000);
    assert_eq!(effort(&bus), 0.0);
    assert_eq!(bus.motor_snapshots()[0].control_state, "coast");
    // The winding inductance retains a decaying current after the bridge
    // opens; it is not physically zero at the disable edge.
    assert!(bus.motor_snapshots()[0].current_a.unwrap().abs() < 0.001);
    bus.write_u8(0x10025, 1).unwrap();
    bus.write_u8(0x1002b, 0x40).unwrap(); // L_EN alone low
    run_motor(&mut bus, 4_032_000, 32_000);
    assert_eq!(effort(&bus), 0.0);
    assert_eq!(bus.motor_snapshots()[0].control_state, "coast");
}

#[test]
fn arduino_timer0_pwm_reaches_the_bts_observer_and_physical_motor() {
    let image =
        labwired_loader::load_elf(&root().join("tests/fixtures/avr/arduino-uno-pwm.elf")).unwrap();
    let mut cpu = Avr::new();
    cpu.load_program_image(&image);
    let mut machine = Machine::new(cpu, bus(false));
    let mut drive = false;
    let mut stopped = false;
    while machine.cpu.cycles < 1_600_000 {
        machine.step().unwrap();
        let e = effort(&machine.bus);
        drive |= e > 0.0;
        stopped |= drive && e == 0.0;
    }
    assert!(
        drive && stopped,
        "the observer must see both phases of analogWrite(5,128)"
    );
    let motor = &machine.bus.motor_snapshots()[0];
    assert!(
        motor.speed_rpm > 10.0,
        "PWM must turn the attached plant: {motor:?}"
    );
    assert!(
        motor.speed_rpm_min >= 0.0,
        "RPWM cannot reverse the motor: {motor:?}"
    );
}

#[test]
fn stm32_af_pwm_and_disabled_pads_override_stale_gpio_latches() {
    let chip: ChipDescriptor = serde_yaml::from_str(
        r#"
name: stm32f1-dual-pwm
arch: arm
core: cortex-m3
flash: {base: 0x08000000, size: "64KB"}
ram: {base: 0x20000000, size: "20KB"}
peripherals:
  - id: gpioa
    type: gpio
    base_address: 0x40010800
    size: "1KB"
    config: {profile: stm32f1}
  - id: tim2
    type: timer
    base_address: 0x40000000
    size: "1KB"
"#,
    )
    .unwrap();
    let manifest = SystemManifest::from_yaml(
        r#"
name: af-bridge
chip: unused
external_devices:
  - id: bridge
    type: bts7960
    connection: gpio
    config: {rpwm_pin: PA0, lpwm_pin: PA1, r_en_pin: PA2, l_en_pin: PA3}
  - id: motor
    type: dc-motor
    connection: gpio
    config:
      in1_pin: PA0
      in2_pin: PA1
      pwm_pin: 3V3
      enable_pin: PA2
      enable2_pin: PA3
      both_inputs_low: brake
      simulation_clock_hz: 16000000
      resistance_ohm: 6.0
      inductance_h: 0.002
      torque_constant_nm_per_a: 0.08
      back_emf_constant_v_per_rad_s: 0.08
      rotor_inertia_kg_m2: 0.00004
      viscous_friction_nm_per_rad_s: 0.00001
      supply_voltage_v: 12.0
      load_torque_nm: 0.0
      encoder_cpr: 1024
"#,
    )
    .unwrap();
    let mut bus = SystemBus::from_config(&chip, &manifest).unwrap();
    // A branch-to-self guest advances the production Machine boundary. With
    // event-scheduler enabled, a bare bus tick does not drain the timer's
    // scheduled output-wave events and would leave its AF pads frozen.
    let spin_pc = 0x0800_0040;
    bus.write_u16(spin_pc, 0xe7fe).unwrap(); // b .
    let (mut cpu, _) = configure_cortex_m(&mut bus);
    cpu.pc = spin_pc as u32;
    cpu.sp = 0x2000_5000;
    let mut machine = Machine::new(cpu, bus);
    machine.config.peripheral_tick_interval = 1;
    machine.bus.config.peripheral_tick_interval = 1;
    // PA0/PA1: AF push-pull; PA2/PA3: GPIO push-pull. ODR deliberately
    // retains RPWM high, so a latch read would falsely drive continuously.
    machine.bus.write_u32(0x4001_0800, 0x4444_33bb).unwrap();
    machine.bus.write_u32(0x4001_080c, 0b1101).unwrap();
    machine.bus.write_u32(0x4000_0028, 0).unwrap(); // PSC
    machine.bus.write_u32(0x4000_002c, 99).unwrap(); // ARR
    machine.bus.write_u32(0x4000_0034, 25).unwrap(); // RPWM CCR1: 25%
    machine.bus.write_u32(0x4000_0038, 0).unwrap(); // LPWM CCR2: 0%
    machine.bus.write_u32(0x4000_0018, 0x6060).unwrap(); // PWM1 on CH1/CH2
    machine.bus.write_u32(0x4000_0020, 0x11).unwrap(); // CC1E + CC2E
    machine.bus.write_u32(0x4000_0000, 1).unwrap(); // CEN

    let (mut saw_drive, mut saw_brake, mut saw_high, mut saw_low) = (false, false, false, false);
    // No GPIO stores occur here: only the timer advances its AF waveform.
    for _ in 0..4000 {
        machine.run(Some(1)).unwrap();
        let bus = &machine.bus;
        let state = &bus.motor_snapshots()[0].control_state;
        saw_drive |= state == "forward";
        saw_brake |= state == "brake";
        saw_high |= effort(bus) == 1.0;
        saw_low |= effort(bus) == 0.0;
    }
    assert!(saw_drive && saw_brake, "the plant must see both PWM phases");
    assert!(
        saw_high && saw_low,
        "the observer must see autonomous AF edges"
    );
    assert!(machine.bus.motor_snapshots()[0].speed_rpm > 0.0);

    machine.bus.write_u32(0x4000_0034, 0).unwrap();
    machine.bus.write_u32(0x4000_0038, 75).unwrap(); // reverse on the other PWM input
    let mut saw_reverse = false;
    for _ in 0..4000 {
        machine.run(Some(1)).unwrap();
        let bus = &machine.bus;
        saw_reverse |= bus.motor_snapshots()[0].control_state == "reverse" && effort(bus) == -1.0;
    }
    assert!(saw_reverse, "LPWM must reach both the observer and plant");

    machine.bus.write_u32(0x4000_0020, 0).unwrap(); // disable both channel outputs
    machine.bus.write_u32(0x4000_0000, 0).unwrap();
    machine.run(Some(1)).unwrap();
    assert_eq!(machine.bus.read_u32(0x4001_080c).unwrap(), 0b1101);
    assert_eq!(
        effort(&machine.bus),
        0.0,
        "a released AF pad cannot use stale ODR"
    );
    assert_eq!(machine.bus.motor_snapshots()[0].control_state, "brake");

    // A high enable latch also stops driving once its pin becomes an input.
    machine.bus.write_u32(0x4001_0800, 0x4444_34bb).unwrap();
    machine.run(Some(1)).unwrap();
    assert_eq!(machine.bus.motor_snapshots()[0].control_state, "coast");
}
