// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! The generic NTC divider and diode load line reach the SAM E70 Xplained's
//! AFECs through the ATSAME70Q21B `analog_pins` map: the NTC on EXT1 ADC(+)
//! PC31 (AFE1_AD6) and the diode on EXT1 ADC(-) PA19 (AFE0_AD8), converted
//! the way Zephyr v3.7.0 `adc_sam_afec.c` does it (SWRST, MR, COCR = 512,
//! CHER + IER + CR.START, then CSELR and the CDR read). Also:
//! - the same two netlists moved to PA17 (AFE0_AD6) and PB0 (AFE0_AD10)
//!   convert in one `CR.START` on AFEC0, at 16 bits (`EMR.RES` OSR256), with
//!   `EMR.TAG` naming the last channel in `LCDR.CHNB`;
//! - `IER.EOC6` on AFEC1 pends AFEC1_IRQn (40) in the NVIC.

mod common;
use common::root;
use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::bus::SystemBus;
use labwired_core::cosim::{CosimSession, CosimSignalValue, RoutingError};
use labwired_core::Bus;
use std::path::Path;
use std::sync::atomic::Ordering;

const AFEC0: u64 = 0x4003_C000;
const AFEC1: u64 = 0x4006_4000;
const CR: u64 = 0x00;
const MR: u64 = 0x04;
const EMR: u64 = 0x08;
const CHER: u64 = 0x14;
const CHDR: u64 = 0x18;
const LCDR: u64 = 0x20;
const IER: u64 = 0x24;
const IDR: u64 = 0x28;
const ISR: u64 = 0x30;
const CSELR: u64 = 0x64;
const CDR: u64 = 0x68;
const COCR: u64 = 0x6C;
const CR_SWRST: u32 = 1 << 0;
const CR_START: u32 = 1 << 1;
/// `AFEC_MR_ONE | AFEC_MR_STARTUP_SUT96`, as adc_sam_init writes them.
const MR_ZEPHYR: u32 = (1 << 23) | (5 << 16);
const EMR_RES_OSR256: u32 = 5 << 16;
const EMR_TAG: u32 = 1 << 24;
const ISR_DRDY: u32 = 1 << 24;
const EOC_MASK: u32 = 0xFFF;
const AFEC1_IRQN: u32 = 40;

fn ntc_resistance_ohms(temperature_c: f64) -> f64 {
    10_000.0 * (3950.0 * (1.0 / (temperature_c + 273.15) - 1.0 / 298.15)).exp()
}

fn ntc_volts(temperature_c: f64) -> f64 {
    let r = ntc_resistance_ohms(temperature_c);
    3.3 * r / (10_000.0 + r)
}

fn signal_voltage(session: &CosimSession, path: &str) -> f64 {
    match session.signals().get(path) {
        Some(CosimSignalValue::F64(volts)) => *volts,
        other => panic!("expected solved voltage at {path}, got {other:?}"),
    }
}

/// Single-ended unsigned N-bit code against VREFP = 3.3 V, truncated,
/// clamped (the routed level is handed over in whole millivolts).
fn se_code(volts: f64, bits: u32) -> u32 {
    let mv = (volts * 1000.0).round() as i64;
    let full = (1i64 << bits) - 1;
    (mv * full / 3300).clamp(0, full) as u32
}

/// The SAM E70 Xplained with the generic NTC and diode netlists routed to
/// `board.analog.<pad>_volts`.
fn board(ntc_pad: &str, diode_pad: &str) -> (SystemBus, CosimSession) {
    let system_path = root("configs/systems/sam-e70-xplained.yaml");
    let mut profile =
        SystemManifest::from_file(&system_path).expect("load SAM E70 Xplained profile");
    let analog = SystemManifest::from_file(root(
        "examples/sam-e70-xplained/analog-ntc-diode-system.yaml",
    ))
    .expect("load generic Beta NTC and Shockley diode netlists");
    profile.cosim_models = analog.cosim_models;
    for model in &mut profile.cosim_models {
        let (key, pad) = match model.id.as_str() {
            "ntc_divider" => ("v_sense", ntc_pad),
            "diode_load_line" => ("v_diode", diode_pad),
            other => panic!("unexpected model {other}"),
        };
        model
            .outputs
            .insert(key.to_string(), format!("board.analog.{pad}_volts"));
    }
    let chip_path = system_path.parent().unwrap().join(&profile.chip);
    let chip = ChipDescriptor::from_file(&chip_path).expect("load ATSAME70Q21B descriptor");
    let bus = SystemBus::from_config(&chip, &profile).expect("build SAM E70 Xplained bus");
    let session = CosimSession::new(&profile.cosim_models, Path::new("."), &bus)
        .expect("build analog co-simulation")
        .expect("fixture carries both analog devices");
    assert_eq!(
        session.binding_errors(),
        &[] as &[RoutingError],
        "{ntc_pad}/{diode_pad} must resolve to AFEC inputs through analog_pins"
    );
    (bus, session)
}

/// Move the NTC and solve both netlists to the next boundary.
fn settle(session: &mut CosimSession, bus: &mut SystemBus, cycles: &mut u64, temperature_c: f64) {
    session
        .set_signal_number("ui.ntc.temperature_c", temperature_c)
        .expect("set NTC temperature");
    *cycles += session.cycles_until_boundary(*cycles);
    let (steps, errors) = session
        .advance_to(*cycles, bus)
        .expect("solve and route both netlists");
    assert_eq!(steps.len(), 2, "both netlists solve each step");
    assert!(errors.is_empty(), "route errors: {errors:?}");
}

/// adc_sam_init: SWRST, MR, and every channel's COCR at VREFP/2.
fn zephyr_init(bus: &mut SystemBus, base: u64) {
    bus.write_u32(base + CR, CR_SWRST).expect("SWRST");
    bus.write_u32(base + MR, MR_ZEPHYR).expect("MR");
    for channel in 0..12 {
        bus.write_u32(base + CSELR, channel).expect("CSELR");
        bus.write_u32(base + COCR, 512).expect("COCR");
    }
}

/// adc_sam_start_conversion + adc_sam_isr for one channel, polled.
fn zephyr_read(bus: &mut SystemBus, base: u64, channel: u32) -> u32 {
    bus.write_u32(base + CHDR, 0xFFF).expect("CHDR");
    bus.write_u32(base + IDR, 0xFFF).expect("IDR");
    bus.write_u32(base + CHER, 1 << channel).expect("CHER");
    bus.write_u32(base + CR, CR_START).expect("START");
    assert_eq!(
        bus.read_u32(base + ISR).unwrap() & EOC_MASK,
        1 << channel,
        "EOC{channel} only"
    );
    bus.write_u32(base + CSELR, channel).expect("CSELR");
    let code = bus.read_u32(base + CDR).expect("CDR");
    assert_eq!(
        bus.read_u32(base + ISR).unwrap() & EOC_MASK,
        0,
        "the CDR read clears EOC{channel}"
    );
    code
}

#[test]
fn sam_e70_xplained_ext1_pc31_and_pa19_convert_ntc_and_diode_on_afec1_and_afec0() {
    let (mut bus, mut session) = board("pc31", "pa19");
    zephyr_init(&mut bus, AFEC0);
    zephyr_init(&mut bus, AFEC1);
    let mut cycles = 0;
    let mut ntc_codes = Vec::new();
    for temperature_c in [-10.0_f64, 25.0, 80.0] {
        settle(&mut session, &mut bus, &mut cycles, temperature_c);
        let ntc_v = signal_voltage(&session, "board.analog.pc31_volts");
        assert!((ntc_v - ntc_volts(temperature_c)).abs() < 1e-9);
        let diode_v = signal_voltage(&session, "board.analog.pa19_volts");
        assert!(diode_v > 0.5 && diode_v < 0.8, "diode drop {diode_v} V");

        let ntc_code = zephyr_read(&mut bus, AFEC1, 6);
        assert_eq!(
            ntc_code,
            se_code(ntc_v, 12),
            "AFE1_AD6 at {temperature_c} °C"
        );
        assert_eq!(
            zephyr_read(&mut bus, AFEC0, 8),
            se_code(diode_v, 12),
            "AFE0_AD8"
        );
        // The same channel numbers on the other AFEC are other pads (PA17,
        // PC27), which carry nothing.
        assert_eq!(zephyr_read(&mut bus, AFEC0, 6), 0, "AFE0_AD6 / PA17");
        assert_eq!(zephyr_read(&mut bus, AFEC1, 8), 0, "AFE1_AD8 / PC27");
        ntc_codes.push(ntc_code);
    }
    assert!(
        ntc_codes[0] > ntc_codes[1] && ntc_codes[1] > ntc_codes[2],
        "the NTC codes fall as it warms: {ntc_codes:?}"
    );
}

#[test]
fn sam_e70_pa17_and_pb0_convert_in_one_afec0_start_at_16_bits_with_tag() {
    let (mut bus, mut session) = board("pa17", "pb0");
    zephyr_init(&mut bus, AFEC0);
    let mut cycles = 0;
    settle(&mut session, &mut bus, &mut cycles, 25.0);
    let ntc_v = signal_voltage(&session, "board.analog.pa17_volts");
    let diode_v = signal_voltage(&session, "board.analog.pb0_volts");

    bus.write_u32(AFEC0 + EMR, EMR_RES_OSR256 | EMR_TAG)
        .expect("EMR");
    bus.write_u32(AFEC0 + CHER, (1 << 6) | (1 << 10) | (1 << 8))
        .expect("CHER");
    bus.write_u32(AFEC0 + CR, CR_START).expect("START");
    assert_eq!(
        bus.read_u32(AFEC0 + ISR).unwrap() & (EOC_MASK | ISR_DRDY),
        (1 << 6) | (1 << 8) | (1 << 10) | ISR_DRDY
    );
    // AD10 converts last in numeric order and is tagged in CHNB.
    assert_eq!(
        bus.read_u32(AFEC0 + LCDR).unwrap(),
        se_code(diode_v, 16) | (10 << 24),
        "LCDR = AD10 diode code, CHNB = 10"
    );
    for (channel, expected) in [(6, se_code(ntc_v, 16)), (10, se_code(diode_v, 16)), (8, 0)] {
        bus.write_u32(AFEC0 + CSELR, channel).expect("CSELR");
        assert_eq!(bus.read_u32(AFEC0 + CDR).unwrap(), expected, "AD{channel}");
    }
    assert_eq!(
        bus.read_u32(AFEC0 + ISR).unwrap() & (EOC_MASK | ISR_DRDY),
        0
    );
    // PA19 (AFE0_AD8) is no longer driven; PC31 neither.
    bus.write_u32(AFEC0 + EMR, 0).expect("12-bit");
    assert_eq!(zephyr_read(&mut bus, AFEC0, 8), 0, "PA19 idle");
}

#[test]
fn sam_e70_afec1_eoc_interrupt_pends_afec1_irqn_40() {
    let (mut bus, mut session) = board("pc31", "pa19");
    zephyr_init(&mut bus, AFEC1);
    let mut cycles = 0;
    settle(&mut session, &mut bus, &mut cycles, 80.0);
    let ntc_v = signal_voltage(&session, "board.analog.pc31_volts");

    let _ = labwired_core::system::cortex_m::configure_cortex_m(&mut bus);
    let pending = |bus: &SystemBus| {
        bus.nvic
            .as_ref()
            .map(|n| n.ispr[1].load(Ordering::SeqCst) & (1 << (AFEC1_IRQN - 32)) != 0)
    };
    bus.write_u32(AFEC1 + CHER, 1 << 6).expect("CHER");
    bus.write_u32(AFEC1 + IER, 1 << 6).expect("IER.EOC6");
    bus.tick_peripherals_fully();
    assert_eq!(pending(&bus), Some(false), "no IRQ before the conversion");
    bus.write_u32(AFEC1 + CR, CR_START).expect("START");
    bus.tick_peripherals_fully();
    assert_eq!(pending(&bus), Some(true), "AFEC1_IRQn pends on EOC6");
    bus.write_u32(AFEC1 + CSELR, 6).expect("CSELR");
    assert_eq!(bus.read_u32(AFEC1 + CDR).unwrap(), se_code(ntc_v, 12));
}

/// Independent sidewall depletion charge of `CJSW·PJ` = 1 nF, `PHP` = 0.7 V,
/// `MJSW` = 0.4, valid below `FCS·PHP` (the fixture stays reverse-biased).
fn sidewall_charge(junction_v: f64) -> f64 {
    const C0_F: f64 = 1e-9;
    const PHP_V: f64 = 0.7;
    const MJSW: f64 = 0.4;
    C0_F * PHP_V / (1.0 - MJSW) * (1.0 - (1.0 - junction_v / PHP_V).powf(1.0 - MJSW))
}

/// Independent backward-Euler update of a 1 k drive into the cathode of a
/// grounded-anode diode carrying that charge, its `IS` = 1e-14 A reverse
/// current, and the solver's 1 pS GMIN.
fn expected_sidewall_voltage(previous_v: f64, drive_v: f64, step_s: f64) -> f64 {
    const DRIVE_OHMS: f64 = 1_000.0;
    const IS_A: f64 = 1e-14;
    const GMIN_S: f64 = 1e-12;
    const VT: f64 = 1.380_649e-23 * 300.0 / 1.602_176_634e-19;
    let previous_charge = sidewall_charge(-previous_v);
    let residual = |sense_v: f64| {
        let junction_v = -sense_v;
        let diode_a = IS_A * ((junction_v / VT).exp() - 1.0) + GMIN_S * junction_v;
        (drive_v - sense_v) / DRIVE_OHMS
            + diode_a
            + (sidewall_charge(junction_v) - previous_charge) / step_s
    };
    let (mut low, mut high) = (previous_v.min(drive_v), previous_v.max(drive_v));
    for _ in 0..100 {
        let mid = 0.5 * (low + high);
        if residual(mid) > 0.0 {
            low = mid;
        } else {
            high = mid;
        }
    }
    0.5 * (low + high)
}

#[test]
fn sam_e70_diode_sidewall_capacitance_reaches_afec0_alongside_ntc_and_diode() {
    const STEP_S: f64 = 1e-6;
    const SUBSTEPS: usize = 10;
    const DRIVE_V: f64 = 3.0;
    let system_path = root("configs/systems/sam-e70-xplained.yaml");
    let mut profile =
        SystemManifest::from_file(&system_path).expect("load SAM E70 Xplained profile");
    let analog = SystemManifest::from_file(root(
        "examples/sam-e70-xplained/analog-ntc-diode-system.yaml",
    ))
    .expect("load unrelated generic Beta NTC and Shockley diode netlists");
    let sidewall = SystemManifest::from_file(root(
        "examples/sam-e70-xplained/analog-diode-sidewall-charge-afec-system.yaml",
    ))
    .expect("load generic diode sidewall-charge branches");
    profile.cosim_models = analog.cosim_models;
    for model in &mut profile.cosim_models {
        let (key, pad) = match model.id.as_str() {
            "ntc_divider" => ("v_sense", "pc31"),
            "diode_load_line" => ("v_diode", "pa19"),
            other => panic!("unexpected model {other}"),
        };
        model
            .outputs
            .insert(key.to_string(), format!("board.analog.{pad}_volts"));
    }
    profile.cosim_models.extend(sidewall.cosim_models);

    let chip_path = system_path.parent().unwrap().join(&profile.chip);
    let chip = ChipDescriptor::from_file(&chip_path).expect("load ATSAME70Q21B descriptor");
    for (pad, peripheral, channel) in [
        ("PC31", "afec1", 6),
        ("PA17", "afec0", 6),
        ("PA18", "afec0", 7),
        ("PA19", "afec0", 8),
        ("PA20", "afec0", 9),
    ] {
        let route = &chip.analog_pins[pad];
        assert_eq!(route.peripheral, peripheral, "{pad} AFEC instance");
        assert_eq!(route.channel, channel, "{pad} AFEC channel");
    }
    let mut bus = SystemBus::from_config(&chip, &profile).expect("build SAM E70 Xplained bus");
    let mut session = CosimSession::new(&profile.cosim_models, Path::new("."), &bus)
        .expect("build analog co-simulation")
        .expect("fixture carries NTC, diode and three sidewall-charge branches");
    assert_eq!(
        session.binding_errors(),
        &[] as &[RoutingError],
        "PC31, PA17..PA20 must resolve to AFEC inputs through analog_pins"
    );
    zephyr_init(&mut bus, AFEC0);
    zephyr_init(&mut bus, AFEC1);

    session
        .set_signal_number("ui.diode_sidewall.drive_v", 0.0)
        .expect("start every sidewall branch discharged");
    let mut cycles = 0;
    settle_n(&mut session, &mut bus, &mut cycles, 5);
    assert!(signal_voltage(&session, "board.analog.pa17_volts").abs() < 1e-12);

    session
        .set_signal_number("ui.diode_sidewall.drive_v", DRIVE_V)
        .expect("step the drive into reverse bias");
    let mut expected_v = 0.0;
    let mut first_step_lag_v = 0.0;
    for step in 0..8 {
        settle_n(&mut session, &mut bus, &mut cycles, 5);
        for _ in 0..SUBSTEPS {
            expected_v = expected_sidewall_voltage(expected_v, DRIVE_V, STEP_S / SUBSTEPS as f64);
        }
        let sidewall_v = signal_voltage(&session, "board.analog.pa17_volts");
        let bottom_v = signal_voltage(&session, "board.analog.pa18_volts");
        let control_v = signal_voltage(&session, "board.analog.pa20_volts");
        assert!(
            (sidewall_v - expected_v).abs() < 2e-8,
            "CJSW*PJ charge at step {step}: expected {expected_v} V, got {sidewall_v} V"
        );
        assert!(
            (bottom_v - sidewall_v).abs() < 1e-9,
            "the sidewall (area=4 does not scale it) and an equal CJO junction share one law: {bottom_v} V vs {sidewall_v} V"
        );
        assert!(
            (control_v - DRIVE_V).abs() < 1e-8,
            "pj=0 stores no charge and follows the drive, got {control_v} V"
        );
        if step == 0 {
            first_step_lag_v = control_v - sidewall_v;
        }

        let ntc_v = signal_voltage(&session, "board.analog.pc31_volts");
        assert!(
            (ntc_v - ntc_volts(25.0)).abs() < 1e-9,
            "the NTC stays isolated"
        );
        let diode_v = signal_voltage(&session, "board.analog.pa19_volts");
        assert!(diode_v > 0.5 && diode_v < 0.8, "diode drop {diode_v} V");

        assert_eq!(
            zephyr_read(&mut bus, AFEC1, 6),
            se_code(ntc_v, 12),
            "AFE1_AD6 NTC"
        );
        for (channel, volts) in [(6, sidewall_v), (7, bottom_v), (8, diode_v), (9, control_v)] {
            assert_eq!(
                zephyr_read(&mut bus, AFEC0, channel),
                se_code(volts, 12),
                "AFE0_AD{channel} at step {step}"
            );
        }
        assert_eq!(zephyr_read(&mut bus, AFEC0, 10), 0, "AFE0_AD10 / PB0 idle");
    }
    assert!(
        first_step_lag_v > 0.5,
        "sidewall charge must delay the first microsecond by > 0.5 V, got {first_step_lag_v} V"
    );
    assert!(
        (expected_v - DRIVE_V).abs() < 1e-3,
        "the branch settles to the drive within 8 us, recurrence {expected_v} V"
    );
}

/// Solve every netlist to the next boundary and check that all of them ran.
fn settle_n(session: &mut CosimSession, bus: &mut SystemBus, cycles: &mut u64, netlists: usize) {
    session
        .set_signal_number("ui.ntc.temperature_c", 25.0)
        .expect("hold the NTC at 25 °C");
    *cycles += session.cycles_until_boundary(*cycles);
    let (steps, errors) = session
        .advance_to(*cycles, bus)
        .expect("solve and route every netlist");
    assert_eq!(steps.len(), netlists, "every netlist solves each step");
    assert!(errors.is_empty(), "route errors: {errors:?}");
}
