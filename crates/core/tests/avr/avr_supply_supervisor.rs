// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! A routed Uno rail must hold/release the CPU, and guest instructions must
//! see the reset cause in MCUSR. No synthetic UART claim stands in for power.

use std::path::Path;

use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::bus::SystemBus;
use labwired_core::cosim::CosimSession;
use labwired_core::cpu::Avr;
use labwired_core::{AdvanceRequest, Machine};

fn machine(bod: bool, models: &str) -> (Machine<Avr>, SystemManifest) {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut chip = ChipDescriptor::from_file(root.join("configs/chips/atmega328p.yaml")).unwrap();
    if bod {
        chip.supply_monitor.as_mut().unwrap().bor_level = Some("bod2v7".into());
    }
    let manifest =
        SystemManifest::from_yaml(&format!("name: avr-power\nchip: atmega328p\n{models}")).unwrap();
    let bus = SystemBus::from_config(&chip, &manifest).unwrap();
    let mut cpu = Avr::new();
    // IN r16,MCUSR; OUT MCUSR,r1(0); IN r17,MCUSR; INC r20; RJMP -2.
    cpu.load_words(0, &[0xb704, 0xbe14, 0xb714, 0x9543, 0xcffe]);
    (Machine::new(cpu, bus), manifest)
}

fn run(machine: &mut Machine<Avr>, cycles: u64) {
    machine
        .advance(AdvanceRequest::run(None).with_cycle_limit(cycles))
        .unwrap();
}

#[test]
fn voltage_thresholds_hold_the_avr_and_guest_reads_por_then_bor() {
    let (mut machine, _) = machine(true, "");
    machine.attach_supply();
    run(&mut machine, 100);
    assert!(machine.supply_status().held_in_reset);
    assert_eq!(
        machine.cpu.r[20], 0,
        "no guest instruction executes without VDD"
    );
    machine.set_supply_volts(5.0);
    run(&mut machine, 30);
    assert_eq!(machine.cpu.r[16], 1, "IN MCUSR sees PORF");
    assert_eq!(machine.cpu.r[17], 0, "OUT MCUSR,0 clears reset flags");
    assert!(machine.cpu.r[20] > 0);
    assert_eq!(machine.supply_status().power_on_resets, 1);

    machine.set_supply_volts(2.6);
    let held_counter = machine.cpu.r[20];
    run(&mut machine, 100);
    assert!(machine.supply_status().held_in_reset);
    assert_eq!(machine.cpu.r[20], held_counter);
    machine.set_supply_volts(2.70);
    run(&mut machine, 100);
    assert!(
        machine.supply_status().held_in_reset,
        "BOD hysteresis does not release below 2.725 V"
    );
    machine.set_supply_volts(2.73);
    run(&mut machine, 30);
    assert_eq!(machine.cpu.r[16], 4, "IN MCUSR sees BORF after brown-out");
    assert_eq!(machine.supply_status().brown_out_resets, 1);

    machine.set_supply_volts(0.0);
    run(&mut machine, 100);
    machine.set_supply_volts(5.0);
    run(&mut machine, 30);
    assert_eq!(
        machine.cpu.r[16], 1,
        "a complete power loss records PORF again"
    );
    assert_eq!(machine.supply_status().power_on_resets, 2);
}

#[test]
fn mcusr_reset_flags_cannot_be_set_by_software() {
    let (mut machine, _) = machine(false, "");
    // LDI r16,0xff; OUT MCUSR,r16; IN r17,MCUSR; RJMP -1.
    machine.cpu.load_words(0, &[0xef0f, 0xbf04, 0xb714, 0xcfff]);
    run(&mut machine, 20);
    assert_eq!(
        machine.cpu.r[17], 0,
        "writing ones cannot manufacture reset causes"
    );
    machine.attach_supply();
    machine.set_supply_volts(5.0);
    run(&mut machine, 20);
    assert_eq!(
        machine.cpu.r[17], 1,
        "writing one preserves a hardware-set PORF"
    );
}

#[test]
fn actual_12v_regulator_net_routes_vdd_and_releases_the_uno() {
    let (mut machine, manifest) = machine(
        false,
        r#"
cosim_models:
  - id: circuit
    adapter: analog
    step_ns: 100000
    outputs: {vdd: board.power.vdd_volts}
    config:
      netlist_text: |
        Vsup vin 0 dc 12
        Xreg vin vcc 0 NCP1117
        Cin vin 0 47u
        Cout vcc 0 47u
        .model NCP1117 LDO (VOUT=5 VDO=1.2 ILIM=1 IQ=6m ROUT=0.008375)
      probes: {vdd: "v(vcc)"}
"#,
    );
    let mut session = CosimSession::new(&manifest.cosim_models, Path::new("."), &machine.bus)
        .unwrap()
        .unwrap();
    assert!(session.binding_errors().is_empty());
    session
        .advance_budget(
            &mut machine,
            AdvanceRequest::run(None).with_cycle_limit(160_000),
        )
        .unwrap();
    let supply = machine.supply_status();
    assert!(supply.routed && !supply.held_in_reset, "{supply:?}");
    assert!((supply.vdd_volts.unwrap() - 5.0).abs() < 0.01, "{supply:?}");
    assert_eq!(supply.power_on_resets, 1);
    assert_eq!(
        machine.cpu.r[16], 1,
        "the guest sees the circuit's power-on reset"
    );
    assert!(machine.cpu.r[20] > 0);
}

#[test]
fn an_unrouted_uno_still_reports_the_assumed_rail() {
    labwired_core::fidelity::reset();
    let (mut machine, _) = machine(false, "");
    run(&mut machine, 20);
    assert!(!machine.supply_status().routed);
    let gaps = labwired_core::fidelity::report().to_gaps();
    assert!(gaps
        .iter()
        .any(|gap| gap.kind == labwired_core::fidelity::UNPOWERED_RAIL_ASSUMED));
}
