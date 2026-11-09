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
