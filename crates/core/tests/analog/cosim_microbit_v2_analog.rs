// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Generic MNA devices route through the BBC micro:bit v2 (nRF52833) profile
//! into SAADC AIN0/AIN1. The same two decks already proven on the
//! NUCLEO-F103RB (`cosim_nucleo_f103rb_analog`) and NUCLEO-F767ZI
//! (`cosim_nucleo_f767zi_analog`) run here unchanged; only the chip
//! descriptor differs. That makes this the second, unrelated ADC peripheral
//! family on the same routing path: Nordic SAADC, 12-bit conversion against
//! an internal 600 mV reference at the reset gain of 1/6, EasyDMA result
//! buffer, and `P0.xx` pad labels instead of `PAxx`.
//!
//! Neither netlist names the board, the SAADC, or a channel: a Beta-parameter
//! NTC divider and a Shockley diode load line publish `board.analog.<pad>_volts`,
//! and the nRF52833 descriptor's `analog_pins:` map selects AIN0 and AIN1.

use crate::common;
use common::root;
use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::bus::SystemBus;
use labwired_core::cosim::{CosimSession, CosimSignalValue, RoutingError};
use labwired_core::cpu::CortexM;
use labwired_core::system::cortex_m::configure_cortex_m;
use labwired_core::{Bus, DebugControl, Machine};
use std::path::Path;

const SAADC_BASE: u64 = 0x4000_7000;
/// PS v1.7 §6.21.7: TASKS_START, TASKS_SAMPLE.
const SAADC_TASKS_START: u64 = 0x000;
const SAADC_TASKS_SAMPLE: u64 = 0x004;
/// PS v1.7 §6.21.7: EVENTS_DONE.
const SAADC_EVENTS_DONE: u64 = 0x108;
const SAADC_ENABLE: u64 = 0x500;
/// CH[n] block at 0x510 + 0x10*n; PSELP is the first word.
const SAADC_CH0_PSELP: u64 = 0x510;
const SAADC_CH1_PSELP: u64 = 0x520;
/// PS v1.7 §6.21.7: RESOLUTION, RESULT.PTR/MAXCNT/AMOUNT.
const SAADC_RESOLUTION: u64 = 0x5f0;
const SAADC_RESULT_PTR: u64 = 0x62c;
const SAADC_RESULT_MAXCNT: u64 = 0x630;
const SAADC_RESULT_AMOUNT: u64 = 0x634;

/// PSELP selectors: 1..=8 select AIN0..7.
const PSELP_AIN0: u32 = 1;
const PSELP_AIN1: u32 = 2;
const PSELP_AIN2: u32 = 3;
/// RESOLUTION code 2 is 12 bit (PS v1.7 §6.21.7).
const RESOLUTION_12BIT: u32 = 2;

/// EasyDMA destination in nRF52833 RAM.
const RESULT_BUFFER: u64 = 0x2000_1000;
/// Parked guest: `b .` at a real flash word, so cycles still advance.
const SPIN_PC: u64 = 0x0000_0040;

/// Beta-parameter NTC resistance, the model card in the fixture.
fn ntc_resistance_ohms(temperature_c: f64) -> f64 {
    10_000.0 * (3950.0 * (1.0 / (temperature_c + 273.15) - 1.0 / 298.15)).exp()
}

/// Independent Shockley load line, including the diode's series resistance.
fn diode_load_line_voltage() -> f64 {
    const SUPPLY_V: f64 = 3.3;
    const LIMIT_OHMS: f64 = 1_000.0;
    const DIODE_IS_A: f64 = 2.52e-9;
    const DIODE_N: f64 = 1.752;
    const DIODE_RS_OHMS: f64 = 0.568;
    const THERMAL_V: f64 = 1.380_649e-23 * 300.0 / 1.602_176_634e-19;
    let current_at_junction_voltage =
        |junction_v: f64| DIODE_IS_A * ((junction_v / (DIODE_N * THERMAL_V)).exp() - 1.0);
    let residual = |junction_v: f64| {
        let current = current_at_junction_voltage(junction_v);
        (SUPPLY_V - junction_v - current * DIODE_RS_OHMS) / LIMIT_OHMS - current
    };
    let (mut low, mut high) = (0.0, 1.2);
    for _ in 0..100 {
        let mid = (low + high) / 2.0;
        if residual(mid) > 0.0 {
            low = mid;
        } else {
            high = mid;
        }
    }
    let junction_v = (low + high) / 2.0;
    junction_v + current_at_junction_voltage(junction_v) * DIODE_RS_OHMS
}

fn signal_voltage(session: &CosimSession, path: &str) -> f64 {
    match session.signals().get(path) {
        Some(CosimSignalValue::F64(volts)) => *volts,
        other => panic!("expected solved voltage at {path}, got {other:?}"),
    }
}

/// The code the SAADC model must produce for a routed pad voltage.
///
/// The router converts volts to `u16` millivolts by rounding, and the SAADC
/// computes `(VP - VN) * gain / reference * 2^bits` with integer arithmetic.
/// At the reset CONFIG values (gain 1/6, internal 600 mV reference,
/// single-ended) and 12-bit resolution that is `mv * 4096 / 3600`, truncated.
fn expected_saadc_code(voltage_v: f64) -> u16 {
    let millivolts = (voltage_v * 1000.0).round() as i64;
    ((millivolts * 4096) / (6 * 600)).clamp(0, 4095) as u16
}

/// Wrap the board bus in the production `Machine`, parked on a branch-to-self
/// guest, so a conversion completes through the same boundary firmware uses.
fn board_machine(mut bus: SystemBus) -> Machine<CortexM> {
    bus.write_u16(SPIN_PC, 0xe7fe).expect("write b ."); // b .
    let (mut cpu, _) = configure_cortex_m(&mut bus);
    cpu.pc = SPIN_PC as u32;
    cpu.sp = 0x2002_0000;
    let mut machine = Machine::new(cpu, bus);
    machine.config.peripheral_tick_interval = 1;
    machine.bus.config.peripheral_tick_interval = 1;
    machine
}

/// Run one SAADC EasyDMA scan of two configured channel slots, the register
/// sequence a driver performs: ENABLE, RESOLUTION, PSELP, RESULT.PTR/MAXCNT,
/// START, SAMPLE.
///
/// `pselp` is the pair of physical inputs each converter slot samples.
fn saadc_scan(machine: &mut Machine<CortexM>, pselp: [u32; 2]) -> [u16; 2] {
    let bus = &mut machine.bus;
    bus.write_u32(SAADC_BASE + SAADC_ENABLE, 1)
        .expect("enable SAADC");
    bus.write_u32(SAADC_BASE + SAADC_RESOLUTION, RESOLUTION_12BIT)
        .expect("select 12-bit resolution");
    bus.write_u32(SAADC_BASE + SAADC_CH0_PSELP, pselp[0])
        .expect("select CH0 positive input");
    bus.write_u32(SAADC_BASE + SAADC_CH1_PSELP, pselp[1])
        .expect("select CH1 positive input");
    bus.write_u32(SAADC_BASE + SAADC_RESULT_PTR, RESULT_BUFFER as u32)
        .expect("point EasyDMA at the result buffer");
    bus.write_u32(SAADC_BASE + SAADC_RESULT_MAXCNT, 2)
        .expect("sized for two samples");
    bus.write_u32(SAADC_BASE + SAADC_EVENTS_DONE, 0)
        .expect("clear DONE");
    bus.write_u32(SAADC_BASE + SAADC_TASKS_START, 1)
        .expect("start acquisition");
    bus.write_u32(SAADC_BASE + SAADC_TASKS_SAMPLE, 1)
        .expect("trigger one scan");

    for _ in 0..64 {
        machine.run(Some(1)).expect("advance the guest one cycle");
        if machine
            .bus
            .read_u32(SAADC_BASE + SAADC_RESULT_AMOUNT)
            .expect("read RESULT.AMOUNT")
            == 2
        {
            let bus = &mut machine.bus;
            assert_ne!(
                bus.read_u32(SAADC_BASE + SAADC_EVENTS_DONE)
                    .expect("read EVENTS_DONE"),
                0,
                "a nonempty scan latches DONE"
            );
            return [
                bus.read_u16(RESULT_BUFFER).expect("read sample 0"),
                bus.read_u16(RESULT_BUFFER + 2).expect("read sample 1"),
            ];
        }
    }
    panic!("SAADC scan did not fill the two-sample buffer");
}

#[test]
fn microbit_v2_ntc_and_diode_netlists_reach_saadc_ain0_ain1_end_to_end() {
    let system_path = root("configs/systems/microbit-v2.yaml");
    let mut profile = SystemManifest::from_file(&system_path).expect("load micro:bit v2 profile");
    assert_eq!(profile.name, "microbit-v2");

    let analog =
        SystemManifest::from_file(root("examples/microbit-v2/analog-ntc-diode-system.yaml"))
            .expect("load generic NTC divider and diode netlists");
    profile.cosim_models = analog.cosim_models;
    let chip_path = system_path.parent().unwrap().join(&profile.chip);
    let chip = ChipDescriptor::from_file(&chip_path).expect("load nRF52833 descriptor");
    assert_eq!(chip.analog_pins["P0.02"].peripheral, "saadc");
    assert_eq!(chip.analog_pins["P0.02"].channel, 0);
    assert_eq!(chip.analog_pins["P0.03"].peripheral, "saadc");
    assert_eq!(chip.analog_pins["P0.03"].channel, 1);

    let bus = SystemBus::from_config(&chip, &profile).expect("build micro:bit v2 bus");
    let mut machine = board_machine(bus);
    let mut session = CosimSession::new(&profile.cosim_models, Path::new("."), &machine.bus)
        .expect("build analog co-simulation")
        .expect("fixture carries both analog devices");
    assert_eq!(
        session.binding_errors(),
        &[] as &[RoutingError],
        "the generic netlists must resolve through P0.02/P0.03 to SAADC AIN0/AIN1"
    );

    let expected_diode_v = diode_load_line_voltage();
    assert!(
        (0.3..0.9).contains(&expected_diode_v),
        "the load line sits inside the reset 3600 mV full scale, got {expected_diode_v} V"
    );
    let mut cycles = 0;
    let mut samples = Vec::new();
    for temperature_c in [-20.0_f64, 25.0, 85.0] {
        session
            .set_signal_number("ui.ntc.temperature_c", temperature_c)
            .expect("set generic NTC temperature");
        cycles += session.cycles_until_boundary(cycles);
        let (steps, errors) = session
            .advance_to(cycles, &mut machine.bus)
            .expect("solve and route both analog netlists");
        assert_eq!(steps.len(), 2, "both netlists cross the 1 us boundary");
        assert!(errors.is_empty(), "unexpected route errors: {errors:?}");

        let ntc_v = signal_voltage(&session, "board.analog.p0.02_volts");
        let expected_ntc_v = 3.3 * 10_000.0 / (ntc_resistance_ohms(temperature_c) + 10_000.0);
        assert!(
            (ntc_v - expected_ntc_v).abs() < 1e-9,
            "at {temperature_c} C the Beta divider should be {expected_ntc_v} V, got {ntc_v} V"
        );
        let diode_v = signal_voltage(&session, "board.analog.p0.03_volts");
        assert!(
            (diode_v - expected_diode_v).abs() < 2e-5,
            "the Shockley diode load line should be {expected_diode_v} V, got {diode_v} V"
        );

        let codes = saadc_scan(&mut machine, [PSELP_AIN0, PSELP_AIN1]);
        let expected_codes = [expected_saadc_code(ntc_v), expected_saadc_code(diode_v)];
        assert_eq!(
            codes, expected_codes,
            "firmware-visible SAADC EasyDMA buffer must report the solved NTC and diode voltages"
        );
        samples.push((temperature_c, ntc_v, codes[0]));
    }

    assert!(
        samples[2].1 > samples[1].1 && samples[1].1 > samples[0].1,
        "the divider voltage rises with NTC temperature: {samples:?}"
    );
    assert!(
        samples[2].2 > samples[1].2 && samples[1].2 > samples[0].2,
        "SAADC codes track the solved temperature-dependent voltage: {samples:?}"
    );
    assert_eq!(
        samples[1].2, 1877,
        "the 25 C NTC divider is half of the 3600 mV full scale at gain 1/6"
    );

    // AIN2 is in the descriptor's pad map but no netlist drives it: the scan
    // must read the model's explicit ground, not the neighbour's result.
    assert_eq!(
        saadc_scan(&mut machine, [PSELP_AIN2, PSELP_AIN2]),
        [0, 0],
        "an unrouted analog pad stays at 0 mV instead of taking a routed input"
    );
}
