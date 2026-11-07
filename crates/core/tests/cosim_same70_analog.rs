// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Generic analog devices route onto the SAM E70 Xplained EXT1 analog pins,
//! PC31 and PA19, and are asserted through PIO PDSR. Thresholds are test
//! assumptions; the profile still has no AFEC model or silicon threshold claim.

mod common;
use common::root;
use labwired_config::{ChipDescriptor, GpioInputThresholds, SystemManifest};
use labwired_core::bus::SystemBus;
use labwired_core::cosim::{CosimSession, CosimSignalValue, RoutingError};
use labwired_core::Bus;
use std::path::Path;

const PIOA_BASE: u64 = 0x400E_0E00;
const PIOC_BASE: u64 = 0x400E_1200;
const PIO_PER: u64 = 0x0000;
const PIO_ODR: u64 = 0x0014;
const PIO_PDSR: u64 = 0x003C;
const PA19: u8 = 19;
const PC31: u8 = 31;

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
    const THERMAL_V: f64 = 1.380_649e-23 * 298.15 / 1.602_176_634e-19;
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

fn voltage(session: &CosimSession, path: &str) -> f64 {
    match session.signals().get(path) {
        Some(CosimSignalValue::F64(volts)) => *volts,
        other => panic!("expected solved voltage at {path}, got {other:?}"),
    }
}

#[test]
fn sam_e70_xplained_ntc_and_diode_netlists_reach_ext1_pio_inputs() {
    let system_path = root("configs/systems/sam-e70-xplained.yaml");
    let mut profile =
        SystemManifest::from_file(&system_path).expect("load SAM E70 Xplained profile");
    assert_eq!(profile.name, "sam-e70-xplained");
    assert!(profile.external_devices.is_empty());

    let analog = SystemManifest::from_file(root(
        "examples/sam-e70-xplained/analog-ntc-diode-system.yaml",
    ))
    .expect("load generic Beta NTC and Shockley diode netlists");
    profile.cosim_models = analog.cosim_models;

    let chip_path = system_path.parent().unwrap().join(&profile.chip);
    let mut chip = ChipDescriptor::from_file(&chip_path).expect("load ATSAME70Q21B descriptor");
    // Simulation-only digital thresholds; no SAM E70 threshold data is claimed.
    chip.io_voltage_v = Some(3.3);
    chip.gpio_input_thresholds = Some(GpioInputThresholds { vil: 0.3, vih: 0.7 });

    let mut bus = SystemBus::from_config(&chip, &profile).expect("build SAM E70 Xplained bus");
    for (base, pin) in [(PIOC_BASE, PC31), (PIOA_BASE, PA19)] {
        let mask = 1u32 << pin;
        bus.write_u32(base + PIO_PER, mask)
            .expect("assign the EXT1 pad to PIO control");
        bus.write_u32(base + PIO_ODR, mask)
            .expect("configure the EXT1 pad as a digital input");
        assert_eq!(
            bus.read_u32(base + PIO_PDSR).expect("read PIO PDSR") & mask,
            0
        );
    }

    let mut session = CosimSession::new(&profile.cosim_models, Path::new("."), &bus)
        .expect("build analog co-simulation")
        .expect("fixture carries both analog models");
    assert_eq!(
        session.binding_errors(),
        &[] as &[RoutingError],
        "the NTC and diode must resolve through distinct chip-mapped PIO pads"
    );

    let expected_diode_v = diode_load_line_voltage();
    assert!(
        expected_diode_v < 0.99,
        "diode should fall below assumed VIL"
    );
    let mut cycles = 0;
    for (temperature_c, expected_ntc_level) in [
        (-10.0_f64, true),
        (25.0, true), // retain the cold high level inside the assumed band
        (80.0, false),
        (25.0, false), // retain the hot low level inside the assumed band
        (-10.0, true),
    ] {
        session
            .set_signal_number("ui.ntc.temperature_c", temperature_c)
            .expect("set NTC temperature");
        cycles += session.cycles_until_boundary(cycles);
        let (steps, errors) = session
            .advance_to(cycles, &mut bus)
            .expect("solve and route both generic analog netlists");
        assert_eq!(steps.len(), 2, "both unrelated netlists solve");
        assert!(errors.is_empty(), "unexpected route errors: {errors:?}");

        let ntc_v = voltage(&session, "board.gpio_in.pc31");
        let resistance = ntc_resistance_ohms(temperature_c);
        let expected_ntc_v = 3.3 * resistance / (10_000.0 + resistance);
        assert!(
            (ntc_v - expected_ntc_v).abs() < 1e-9,
            "at {temperature_c} °C the NTC divider should be {expected_ntc_v} V, got {ntc_v} V"
        );
        assert_eq!(
            bus.read_u32(PIOC_BASE + PIO_PDSR).expect("read PC31 PDSR") & (1 << PC31) != 0,
            expected_ntc_level,
            "PC31 PDSR follows the solved NTC voltage and retains its Schmitt state"
        );

        let diode_v = voltage(&session, "board.gpio_in.pa19");
        assert!(
            (diode_v - expected_diode_v).abs() < 2e-5,
            "the Shockley load line should be {expected_diode_v} V, got {diode_v} V"
        );
        assert!(diode_v < 0.99, "diode node stays below assumed VIL");
        assert_eq!(
            bus.read_u32(PIOA_BASE + PIO_PDSR).expect("read PA19 PDSR") & (1 << PA19),
            0,
            "PA19 PDSR reads low from the independent diode load line"
        );
    }
}
