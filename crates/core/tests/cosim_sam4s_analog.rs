// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Generic analog devices reach SAM4S Xplained EXT1 analog inputs PA17/PA18,
//! asserted twice: through PIO PDSR (thresholds are test assumptions, not
//! silicon data) and through the `analog_pins` pad map into ADC AD0/AD1,
//! converted by the firmware register sequence (CHER, CR.START, CDRx/LCDR).

mod common;
use common::root;
use labwired_config::{ChipDescriptor, GpioInputThresholds, SystemManifest};
use labwired_core::bus::SystemBus;
use labwired_core::cosim::{CosimSession, CosimSignalValue, RoutingError};
use labwired_core::Bus;
use std::path::Path;

const PIOA_BASE: u64 = 0x400E_0E00;
const PIO_PER: u64 = 0x0000;
const PIO_ODR: u64 = 0x0014;
const PIO_PDSR: u64 = 0x003C;
const PA17: u8 = 17;
const PA18: u8 = 18;

const ADC_BASE: u64 = 0x4003_8000;
const ADC_CR: u64 = ADC_BASE;
const ADC_MR: u64 = ADC_BASE + 0x04;
const ADC_SEQR1: u64 = ADC_BASE + 0x08;
const ADC_CHER: u64 = ADC_BASE + 0x10;
const ADC_CHDR: u64 = ADC_BASE + 0x14;
const ADC_LCDR: u64 = ADC_BASE + 0x20;
const ADC_IER: u64 = ADC_BASE + 0x24;
const ADC_ISR: u64 = ADC_BASE + 0x30;
const ADC_OVER: u64 = ADC_BASE + 0x3C;
const ADC_EMR: u64 = ADC_BASE + 0x40;
const ADC_CDR0: u64 = ADC_BASE + 0x50;
const CR_START: u32 = 1 << 1;
const MR_LOWRES: u32 = 1 << 4;
const MR_USEQ: u32 = 1 << 31;
const EMR_TAG: u32 = 1 << 24;
const ISR_DRDY: u32 = 1 << 24;
const ISR_GOVRE: u32 = 1 << 25;

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
fn sam4s_xplained_ntc_and_diode_netlists_reach_ext1_pio_inputs() {
    let system_path = root("configs/systems/sam4s-xplained.yaml");
    let mut profile = SystemManifest::from_file(&system_path).expect("load SAM4S Xplained profile");
    assert_eq!(profile.name, "sam4s-xplained");
    assert!(profile.external_devices.is_empty());

    let analog =
        SystemManifest::from_file(root("examples/sam4s-xplained/analog-ntc-diode-system.yaml"))
            .expect("load generic Beta NTC and Shockley diode netlists");
    profile.cosim_models = analog.cosim_models;

    let chip_path = system_path.parent().unwrap().join(&profile.chip);
    let mut chip = ChipDescriptor::from_file(&chip_path).expect("load ATSAM4S16C descriptor");
    // Simulation-only digital thresholds; no SAM4S silicon data is claimed.
    chip.io_voltage_v = Some(3.3);
    chip.gpio_input_thresholds = Some(GpioInputThresholds { vil: 0.3, vih: 0.7 });

    let mut bus = SystemBus::from_config(&chip, &profile).expect("build SAM4S Xplained bus");
    for pin in [PA17, PA18] {
        let mask = 1u32 << pin;
        bus.write_u32(PIOA_BASE + PIO_PER, mask)
            .expect("assign EXT1 analog pad to PIO control");
        bus.write_u32(PIOA_BASE + PIO_ODR, mask)
            .expect("configure EXT1 analog pad as a digital input");
        assert_eq!(
            bus.read_u32(PIOA_BASE + PIO_PDSR).expect("read PIO PDSR") & mask,
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

        let ntc_v = voltage(&session, "board.gpio_in.pa17");
        let resistance = ntc_resistance_ohms(temperature_c);
        let expected_ntc_v = 3.3 * resistance / (10_000.0 + resistance);
        assert!(
            (ntc_v - expected_ntc_v).abs() < 1e-9,
            "at {temperature_c} °C the NTC divider should be {expected_ntc_v} V, got {ntc_v} V"
        );
        assert_eq!(
            bus.read_u32(PIOA_BASE + PIO_PDSR).expect("read PA17 PDSR") & (1 << PA17) != 0,
            expected_ntc_level,
            "PA17 PDSR follows the solved NTC voltage and retains its Schmitt state"
        );

        let diode_v = voltage(&session, "board.gpio_in.pa18");
        assert!(
            (diode_v - expected_diode_v).abs() < 2e-5,
            "the Shockley load line should be {expected_diode_v} V, got {diode_v} V"
        );
        assert!(diode_v < 0.99, "diode node stays below assumed VIL");
        assert_eq!(
            bus.read_u32(PIOA_BASE + PIO_PDSR).expect("read PA18 PDSR") & (1 << PA18),
            0,
            "PA18 PDSR reads low from the independent diode load line"
        );
    }
}

/// `mv * full_scale / 3300`, truncated: ADVREF is 3.3 V on this board.
fn code(volts: f64, full_scale: u32) -> u32 {
    let mv = (volts * 1000.0).round() as u32;
    (mv * full_scale / 3300).min(full_scale)
}

#[test]
fn sam4s_xplained_ntc_and_diode_netlists_convert_through_pa17_pa18_adc() {
    let system_path = root("configs/systems/sam4s-xplained.yaml");
    let mut profile = SystemManifest::from_file(&system_path).expect("load SAM4S Xplained profile");
    let analog = SystemManifest::from_file(root(
        "examples/sam4s-xplained/analog-ntc-diode-adc-system.yaml",
    ))
    .expect("load generic NTC and diode netlists routed to analog pads");
    profile.cosim_models = analog.cosim_models;
    let chip_path = system_path.parent().unwrap().join(&profile.chip);
    let chip = ChipDescriptor::from_file(&chip_path).expect("load ATSAM4S16C descriptor");
    let mut bus = SystemBus::from_config(&chip, &profile).expect("build SAM4S Xplained bus");
    let mut session = CosimSession::new(&profile.cosim_models, Path::new("."), &bus)
        .expect("build analog co-simulation")
        .expect("fixture carries both analog models");
    assert_eq!(
        session.binding_errors(),
        &[] as &[RoutingError],
        "PA17/PA18 must resolve through analog_pins to ADC AD0/AD1"
    );

    let expected_diode_v = diode_load_line_voltage();
    let mut cycles = 0;
    let mut ntc_codes = Vec::new();
    for temperature_c in [-10.0_f64, 25.0, 80.0] {
        session
            .set_signal_number("ui.ntc.temperature_c", temperature_c)
            .expect("set NTC temperature");
        cycles += session.cycles_until_boundary(cycles);
        let (steps, errors) = session
            .advance_to(cycles, &mut bus)
            .expect("solve and route both generic analog netlists");
        assert_eq!(steps.len(), 2, "both unrelated netlists solve");
        assert!(errors.is_empty(), "unexpected route errors: {errors:?}");

        let ntc_v = voltage(&session, "board.analog.pa17_volts");
        let resistance = ntc_resistance_ohms(temperature_c);
        let expected_ntc_v = 3.3 * resistance / (10_000.0 + resistance);
        assert!(
            (ntc_v - expected_ntc_v).abs() < 1e-9,
            "at {temperature_c} °C the NTC divider should be {expected_ntc_v} V, got {ntc_v} V"
        );
        let diode_v = voltage(&session, "board.analog.pa18_volts");
        assert!(
            (diode_v - expected_diode_v).abs() < 2e-5,
            "the Shockley load line should be {expected_diode_v} V, got {diode_v} V"
        );

        // 1) Ascending scan of AD0, AD1 and the unrouted AD2 on one START,
        //    12-bit, with EMR.TAG naming the last channel in LCDR.
        bus.write_u32(ADC_MR, 0).unwrap();
        bus.write_u32(ADC_EMR, EMR_TAG).unwrap();
        bus.write_u32(ADC_CHER, 0b111).unwrap();
        bus.write_u32(ADC_CR, CR_START).unwrap();
        assert_eq!(
            bus.read_u32(ADC_ISR).unwrap() & (0b111 | ISR_DRDY),
            0b111 | ISR_DRDY,
            "START raises EOC0..2 and DRDY"
        );
        assert_eq!(bus.read_u32(ADC_LCDR).unwrap(), 2 << 12, "AD2 is undriven");
        let ntc_code = bus.read_u32(ADC_CDR0).unwrap();
        assert_eq!(ntc_code, code(ntc_v, 4095), "AD0 converts the NTC divider");
        assert_eq!(
            bus.read_u32(ADC_CDR0 + 4).unwrap(),
            code(diode_v, 4095),
            "AD1 converts the diode load line"
        );
        assert_eq!(bus.read_u32(ADC_CDR0 + 8).unwrap(), 0);
        assert_eq!(
            bus.read_u32(ADC_ISR).unwrap() & (0b111 | ISR_DRDY),
            0,
            "CDR and LCDR reads clear EOC and DRDY"
        );
        bus.write_u32(ADC_CHDR, 0b111).unwrap();
        ntc_codes.push(ntc_code);

        // 2) User sequence AD1 then AD0 at LOWRES 10-bit: LCDR ends on AD0.
        bus.write_u32(ADC_SEQR1, 1).unwrap(); // USCH1 = AD1, USCH2 = AD0
        bus.write_u32(ADC_MR, MR_USEQ | MR_LOWRES).unwrap();
        bus.write_u32(ADC_CHER, 0b11).unwrap();
        bus.write_u32(ADC_IER, ISR_DRDY).unwrap();
        bus.write_u32(ADC_CR, CR_START).unwrap();
        assert_eq!(
            bus.read_u32(ADC_LCDR).unwrap(),
            code(ntc_v, 1023),
            "the user sequence converts AD0 last at 10 bits"
        );
        assert_eq!(bus.read_u32(ADC_CDR0 + 4).unwrap(), code(diode_v, 1023));
        // AD0's CDR is unread: a second START overruns it and DRDY.
        bus.write_u32(ADC_CR, CR_START).unwrap();
        assert_eq!(bus.read_u32(ADC_OVER).unwrap(), 1, "OVRE0 only");
        assert_ne!(bus.read_u32(ADC_ISR).unwrap() & ISR_GOVRE, 0);
        bus.read_u32(ADC_CDR0).unwrap();
        bus.read_u32(ADC_CDR0 + 4).unwrap();
        bus.read_u32(ADC_LCDR).unwrap();
        bus.write_u32(ADC_CHDR, 0b11).unwrap();
        bus.write_u32(ADC_BASE + 0x28, ISR_DRDY).unwrap();
    }
    assert!(
        ntc_codes[0] > ntc_codes[1] && ntc_codes[1] > ntc_codes[2],
        "AD0 codes fall as the low-side NTC heats: {ntc_codes:?}"
    );
}
