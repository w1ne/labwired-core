// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Process-derived MOSFET threshold through the RA ADC12, on two boards.
//!
//! `examples/ek-ra6m5/analog-mosfet-nsub-adc-system.yaml` holds three matched
//! level-1 NMOS stages whose thresholds come from `NSUB` (and `NSS`/`TPG`)
//! rather than `VTO`. This test routes them to ADC1 channels 3/4/5 beside the
//! generic NTC and Shockley diode on channels 0/2, sweeps the gate, and checks
//! each drain against an independent Shichman–Hodges load line that uses the
//! threshold ngspice reports for the same cards. A firmware-visible group-A
//! scan then converts all five channels. This is model coverage on already
//! covered profiles, not silicon validation.

mod common;
use common::root;
use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::bus::SystemBus;
use labwired_core::cosim::{CosimSession, CosimSignalValue, RoutingError};
use labwired_core::system::cortex_m::configure_cortex_m;
use labwired_core::{AdvanceRequest, Bus, Cpu, Machine};
use std::path::Path;

/// `VTO` ngspice-45 `showmod` reports for `NMOS_NSUB` and `NMOS_NSS`; `NMOS_REF`
/// has no `NSUB`, so its `VTO` stays at the 0 V default.
const VTO_NSUB: f64 = 0.068_466_2;
const VTO_NSS: f64 = 1.090_76;
const VTO_REF: f64 = 0.0;
const BETA: f64 = 50e-6 * 10e-6 / 2e-6;
const SUPPLY_V: f64 = 3.3;
const LOAD_OHMS: f64 = 10_000.0;

/// Drain voltage of a level-1 NMOS (LAMBDA=0, source and bulk grounded)
/// behind a resistor to the supply, by bisection on the load line.
fn common_source_drain(gate_v: f64, vto: f64) -> f64 {
    let overdrive = gate_v - vto;
    let channel = |vds: f64| {
        if overdrive <= 0.0 {
            0.0
        } else if vds < overdrive {
            BETA * (overdrive - 0.5 * vds) * vds
        } else {
            0.5 * BETA * overdrive * overdrive
        }
    };
    let (mut low, mut high) = (0.0, SUPPLY_V);
    for _ in 0..100 {
        let mid = 0.5 * (low + high);
        if (SUPPLY_V - mid) / LOAD_OHMS > channel(mid) {
            low = mid;
        } else {
            high = mid;
        }
    }
    0.5 * (low + high)
}

fn signal(session: &CosimSession, path: &str) -> f64 {
    match session.signals().get(path) {
        Some(CosimSignalValue::F64(volts)) => *volts,
        other => panic!("{path} did not reach the RA ADC1: {other:?}"),
    }
}

fn assert_ra_adc12_mosfet_substrate_doping(
    system_manifest: &str,
    expected_name: &str,
    adc1: u64,
    idle: u32,
    stack_pointer: u32,
) {
    let adansa0 = adc1 + 0x04;
    let addr = |channel: u64| adc1 + 0x20 + 2 * channel;
    const ADST: u16 = 1 << 15;

    let profile_path = root(system_manifest);
    let mut profile = SystemManifest::from_file(&profile_path).expect("load board profile");
    assert_eq!(profile.name, expected_name);
    let mut models = SystemManifest::from_file(root("examples/ek-ra6m5/analog-ntc-system.yaml"))
        .expect("load generic NTC and diode netlists")
        .cosim_models;
    models.extend(
        SystemManifest::from_file(root("examples/ek-ra6m5/analog-mosfet-nsub-adc-system.yaml"))
            .expect("load process-derived NMOS netlist")
            .cosim_models,
    );
    profile.cosim_models = models;

    let chip_path = profile_path.parent().unwrap().join(&profile.chip);
    let chip = ChipDescriptor::from_file(&chip_path).expect("load board chip descriptor");
    let mut bus = SystemBus::from_config(&chip, &profile).expect("build board bus");
    let (cpu, _) = configure_cortex_m(&mut bus);
    let mut machine = Machine::new(cpu, bus);
    machine
        .bus
        .write_u16(u64::from(idle), 0xE7FE)
        .expect("write a branch-to-self instruction for lockstep time");
    machine.cpu.set_pc(idle);
    machine.cpu.set_sp(stack_pointer);

    let mut session = CosimSession::new(&profile.cosim_models, Path::new("."), &machine.bus)
        .expect("build session")
        .expect("models were declared, so there is a session");
    assert_eq!(
        session.binding_errors(),
        &[] as &[RoutingError],
        "every route must resolve on {expected_name}"
    );
    session
        .set_signal_number("ui.ntc.temperature_c", 25.0)
        .expect("the NTC source reads the temperature input");

    let mut previous: Option<[f64; 3]> = None;
    for gate_v in [0.5_f64, 1.5, 2.5] {
        session
            .set_signal_number("ui.mosfet_nsub.gate_v", gate_v)
            .expect("the gate source reads the drive input");
        let mut reached_boundary = false;
        for _ in 0..256 {
            let advance = session
                .advance(&mut machine, AdvanceRequest::single())
                .expect("advance the machine and analog models in lockstep");
            assert!(
                advance.new_routing_errors.is_empty(),
                "unexpected analog route errors: {:?}",
                advance.new_routing_errors
            );
            if !advance.routed.is_empty() {
                reached_boundary = true;
                break;
            }
        }
        assert!(
            reached_boundary,
            "the analog models did not reach a boundary"
        );

        let drains = [
            signal(&session, "adc.adc1.3_volts"),
            signal(&session, "adc.adc1.4_volts"),
            signal(&session, "adc.adc1.5_volts"),
        ];
        for ((label, vto), volts) in [("nsub", VTO_NSUB), ("nss", VTO_NSS), ("ref", VTO_REF)]
            .into_iter()
            .zip(drains)
        {
            let expected = common_source_drain(gate_v, vto);
            assert!(
                (volts - expected).abs() < 1e-3,
                "{expected_name} {label} at Vg={gate_v} V: load line gives {expected} V, \
                 solved {volts} V"
            );
        }
        // The derived thresholds sit above the 0 V an engine ignoring NSUB
        // would use, so both process stages conduct less than the reference.
        assert!(
            drains[1] > drains[0] && drains[0] > drains[2],
            "{expected_name} at Vg={gate_v} V: drains must order nss > nsub > ref: {drains:?}"
        );
        if let Some(previous) = previous {
            for (now, before) in drains.iter().zip(previous) {
                assert!(
                    *now <= before,
                    "{expected_name}: a higher gate must not raise a drain: {drains:?} vs {previous:?}"
                );
            }
        }
        previous = Some(drains);

        let ntc_v = signal(&session, "adc.adc1.0_volts");
        assert!(
            (ntc_v - 1.65).abs() < 1e-9,
            "the NTC divider is half supply at 25 °C"
        );
        let diode_v = signal(&session, "adc.adc1.2_volts");

        machine
            .bus
            .write_u16(adansa0, 0b11_1101)
            .expect("select RA ADC1 channels 0, 2, 3, 4 and 5");
        machine
            .bus
            .write_u16(adc1, ADST)
            .expect("start a firmware-visible ADC1 group-A scan");
        let mut conversion_complete = false;
        for _ in 0..4096 {
            if machine.bus.read_u16(adc1).expect("read ADC1 ADCSR") & ADST == 0 {
                conversion_complete = true;
                break;
            }
            let advance = session
                .advance(&mut machine, AdvanceRequest::single())
                .expect("advance the RA ADC12 conversion");
            assert!(
                advance.new_routing_errors.is_empty(),
                "unexpected analog route errors: {:?}",
                advance.new_routing_errors
            );
        }
        assert!(conversion_complete, "RA ADC1 single scan did not complete");

        let count = |volts: f64| {
            let millivolts = (volts * 1000.0 + 0.5).floor() as u32;
            (millivolts * 4095 / 3300) as u16
        };
        for (channel, volts) in [
            (0, ntc_v),
            (2, diode_v),
            (3, drains[0]),
            (4, drains[1]),
            (5, drains[2]),
        ] {
            assert_eq!(
                machine
                    .bus
                    .read_u16(addr(channel))
                    .expect("read RA ADC1 ADDRn"),
                count(volts),
                "{expected_name} ADC1 channel {channel} at Vg={gate_v} V must convert {volts} V"
            );
        }
        assert_eq!(
            machine.bus.read_u16(addr(1)).expect("read RA ADC1 ADDR1"),
            0,
            "the unselected channel 1 must not be converted"
        );
        assert!(
            count(drains[0]) > count(drains[2]),
            "{expected_name} at Vg={gate_v} V: the NSUB threshold must be visible in counts"
        );
    }
}

#[test]
fn ek_ra6m5_mosfet_substrate_doping_reaches_adc1_alongside_ntc_and_diode() {
    assert_ra_adc12_mosfet_substrate_doping(
        "configs/systems/ek-ra6m5.yaml",
        "ek-ra6m5",
        0x4017_0200,
        0x2000_2000,
        0x2000_7F00,
    );
}

#[test]
fn ek_ra8m1_mosfet_substrate_doping_reaches_adc1_alongside_ntc_and_diode() {
    assert_ra_adc12_mosfet_substrate_doping(
        "configs/systems/ek-ra8m1.yaml",
        "ek-ra8m1",
        0x4033_2200,
        0x2200_2000,
        0x2200_7F00,
    );
}
