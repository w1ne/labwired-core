// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! The generic NTC divider and diode load line reach the Allwinner A20
//! low-resolution ADC (`sunxi_lradc` at 0x01C2_2800, Linux sun7i-a20.dtsi
//! `lradc@1c22800`) on the onboarding A20 through the descriptor's
//! `analog_pins` map. LRADC0/LRADC1 are dedicated pads: LRADCn is channel n.
//! The NTC is on LRADC0 and the diode on LRADC1, read two ways:
//! - as data: the NetBSD `sunxi_lradc.c` open with `CHAN_SELECT` 2 (both
//!   channels), then per read clear `INTS`, wait for `CHANn_DATA`, take
//!   `DATAn & 0x3f`, write `INTS` back;
//! - as a key: the Linux `sun4i-lradc-keys.c` open (`CHAN_SELECT` 0,
//!   `INTC = KEYUP | KEYDOWN`) and its ISR, which reads `DATA0 & 0x3f` on
//!   `KEYDOWN` and takes `voltage = val * vref / 63`. The NTC divider
//!   crosses level B (0x3C) as it warms, so it acts as a threshold switch.
//!
//! VREF is 2/3 of the 3.0 V `vref-supply` (Linux `lradc_variant_a10`), 2 V,
//! so the -10 °C divider (2.79 V) saturates at 0x3F and the 25 °C one
//! (1.65 V) does not. Also: the netlists swapped to LRADC1/LRADC0 convert
//! there.

mod common;
use common::root;
use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::bus::SystemBus;
use labwired_core::cosim::{CosimSession, CosimSignalValue, RoutingError};
use labwired_core::Bus;
use std::path::Path;

const LRADC: u64 = 0x01C2_2800;
const CTRL: u64 = LRADC;
const INTC: u64 = LRADC + 0x04;
const INTS: u64 = LRADC + 0x08;
const DATA0: u64 = LRADC + 0x0C;
const ENABLE: u32 = 1;
const HOLD_EN: u32 = 1 << 6;
const DATA0_IRQ: u32 = 1 << 0;
const KEYDOWN0: u32 = 1 << 1;
const KEYUP0: u32 = 1 << 4;
const KEY1_EVENTS: u32 = 0x1E << 8;
/// 3000 mV `vref-supply` * 2 / 3.
const VREF_MV: u32 = 2000;

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

fn millivolts(volts: f64) -> u32 {
    (volts * 1000.0).round() as u32
}

/// 6-bit, 63 / VREF, truncated, clamped.
fn code(volts: f64) -> u32 {
    (millivolts(volts) * 63 / VREF_MV).min(63)
}

/// The onboarding A20 with the generic NTC and diode netlists routed to the
/// two pads.
fn board(ntc_pad: &str, diode_pad: &str) -> (SystemBus, CosimSession) {
    let system_path = root("configs/systems/onboarding/a20.yaml");
    let mut profile = SystemManifest::from_file(&system_path).expect("load A20");
    let analog = SystemManifest::from_file(root(
        "examples/ci-fixture-unknown-arch/analog-ntc-diode-system.yaml",
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
    profile.chip = chip_path.to_string_lossy().into_owned();
    let chip = ChipDescriptor::from_file(&chip_path).expect("load A20 descriptor");
    let bus = SystemBus::from_config(&chip, &profile).expect("build A20 bus");
    let session = CosimSession::new(&profile.cosim_models, Path::new("."), &bus)
        .expect("build analog co-simulation")
        .expect("fixture carries both analog devices");
    assert_eq!(
        session.binding_errors(),
        &[] as &[RoutingError],
        "{ntc_pad}/{diode_pad} must resolve to LRADCn through analog_pins"
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

fn lradc_irq(bus: &SystemBus) -> bool {
    let lradc = bus
        .peripherals
        .iter()
        .find(|p| p.name == "lradc")
        .expect("lradc on the bus");
    lradc
        .dev
        .irq_line_level()
        .expect("sunxi_lradc reports its request")
}

/// `FIRST_CONVERT_DLY(2) | LEVELA_B_CNT(1) | HOLD_EN | SAMPLE_RATE(0) |
/// ENABLE` with `chan_select` in [23:22].
fn open(bus: &mut SystemBus, chan_select: u32, intc: u32) {
    bus.write_u32(
        CTRL,
        (2 << 24) | (chan_select << 22) | (1 << 8) | HOLD_EN | ENABLE,
    )
    .expect("CTRL");
    bus.write_u32(INTC, intc).expect("INTC");
}

/// One data read on `channel`: clear `INTS`, wait a sample, `DATAn & 0x3f`.
fn read_data(bus: &mut SystemBus, channel: u32) -> u32 {
    bus.write_u32(INTS, !0).expect("clear INTS");
    bus.tick_peripherals();
    let ints = bus.read_u32(INTS).unwrap();
    assert_ne!(ints & (DATA0_IRQ << (8 * channel)), 0, "CHAN{channel}_DATA");
    let value = bus.read_u32(DATA0 + 4 * u64::from(channel)).unwrap() & 0x3F;
    bus.write_u32(INTS, ints).expect("write INTS back");
    value
}

/// Linux `sun4i_lradc_irq`: `(INTS, Some(mV) on KEYDOWN)`, then clear.
fn linux_isr(bus: &mut SystemBus) -> (u32, Option<u32>) {
    let ints = bus.read_u32(INTS).unwrap();
    let voltage =
        (ints & KEYDOWN0 != 0).then(|| (bus.read_u32(DATA0).unwrap() & 0x3F) * VREF_MV / 63);
    bus.write_u32(INTS, ints).expect("write INTS back");
    (ints, voltage)
}

#[test]
fn a20_lradc0_and_lradc1_convert_ntc_and_diode() {
    let (mut bus, mut session) = board("LRADC0", "LRADC1");
    assert_eq!(bus.read_u32(CTRL).unwrap(), 0, "CTRL reset");
    bus.tick_peripherals();
    assert_eq!(bus.read_u32(INTS).unwrap(), 0, "no sample while disabled");
    open(&mut bus, 2, DATA0_IRQ | (DATA0_IRQ << 8));
    let mut cycles = 0;
    let mut ntc_codes = Vec::new();
    for temperature_c in [25.0_f64, 50.0, 80.0] {
        settle(&mut session, &mut bus, &mut cycles, temperature_c);
        let ntc_v = signal_voltage(&session, "board.analog.LRADC0_volts");
        assert!((ntc_v - ntc_volts(temperature_c)).abs() < 1e-9);
        let diode_v = signal_voltage(&session, "board.analog.LRADC1_volts");
        assert!(diode_v > 0.5 && diode_v < 0.8, "diode drop {diode_v} V");

        let ntc_code = read_data(&mut bus, 0);
        assert_eq!(ntc_code, code(ntc_v), "LRADC0 at {temperature_c} °C");
        assert!(ntc_code < 63, "below full scale at {temperature_c} °C");
        assert_eq!(read_data(&mut bus, 1), code(diode_v), "LRADC1");
        ntc_codes.push(ntc_code);
    }
    assert!(
        ntc_codes[0] > ntc_codes[1] && ntc_codes[1] > ntc_codes[2],
        "the NTC codes fall as it warms: {ntc_codes:?}"
    );
    // The driver scale is vref / 63: within one 31.7 mV step of the divider.
    let mv = ntc_codes[0] * VREF_MV / 63;
    let true_mv = millivolts(ntc_volts(25.0));
    assert!(mv <= true_mv && true_mv - mv < VREF_MV / 63 + 1, "{mv} mV");
    // At -10 °C the divider (about 2.8 V) is above the 2 V full scale.
    settle(&mut session, &mut bus, &mut cycles, -10.0);
    assert_eq!(read_data(&mut bus, 0), 63, "over-range clamps");
}

#[test]
fn a20_linux_lradc_keys_isr_sees_the_ntc_cross_level_b() {
    let (mut bus, mut session) = board("LRADC0", "LRADC1");
    let mut cycles = 0;
    // Cold: 2.79 V saturates at 0x3F, above level B 0x3C. No key.
    settle(&mut session, &mut bus, &mut cycles, -10.0);
    open(&mut bus, 0, KEYUP0 | KEYDOWN0);
    bus.tick_peripherals();
    assert!(!lradc_irq(&bus), "idle: no key event");
    assert_eq!(
        bus.read_u32(INTS).unwrap() & KEY1_EVENTS,
        0,
        "LRADC1 not selected"
    );
    bus.write_u32(INTS, !0).expect("clear data events");

    // Warm: 1.65 V is code 51, below level B. One KEYDOWN on SPI 31.
    settle(&mut session, &mut bus, &mut cycles, 25.0);
    bus.tick_peripherals();
    assert!(lradc_irq(&bus), "KEYDOWN raises the request");
    let (ints, voltage) = linux_isr(&mut bus);
    assert_eq!(ints & (KEYDOWN0 | KEYUP0), KEYDOWN0);
    let ntc_mv = millivolts(signal_voltage(&session, "board.analog.LRADC0_volts"));
    let key_mv = voltage.expect("the ISR reads DATA0 on KEYDOWN");
    assert_eq!(key_mv, code(ntc_volts(25.0)) * VREF_MV / 63);
    assert!(
        ntc_mv - key_mv < VREF_MV / 63 + 1,
        "{key_mv} mV vs {ntc_mv} mV"
    );
    assert!(!lradc_irq(&bus), "the ISR's write-back drops the request");

    // Hotter still: the key stays down, no new event.
    settle(&mut session, &mut bus, &mut cycles, 80.0);
    bus.tick_peripherals();
    assert!(!lradc_irq(&bus), "a held key reports one KEYDOWN");

    // Cold again: back above level B. One KEYUP.
    settle(&mut session, &mut bus, &mut cycles, -10.0);
    bus.tick_peripherals();
    assert!(lradc_irq(&bus), "KEYUP raises the request");
    let (ints, voltage) = linux_isr(&mut bus);
    assert_eq!(ints & (KEYDOWN0 | KEYUP0), KEYUP0);
    assert_eq!(voltage, None);

    // sun4i_lradc_close: ENABLE = 0, INTC = 0; no more samples.
    bus.write_u32(CTRL, (2 << 24) | (1 << 8) | HOLD_EN | (2 << 2))
        .expect("close CTRL");
    bus.write_u32(INTC, 0).expect("close INTC");
    settle(&mut session, &mut bus, &mut cycles, 25.0);
    bus.tick_peripherals();
    assert_eq!(bus.read_u32(INTS).unwrap(), 0, "closed: no events");
    assert_eq!(
        bus.read_u32(DATA0).unwrap(),
        63,
        "DATA0 holds the last sample"
    );
}

#[test]
fn a20_lradc1_and_lradc0_convert_after_the_netlists_swap() {
    let (mut bus, mut session) = board("LRADC1", "LRADC0");
    open(&mut bus, 2, 0);
    let mut cycles = 0;
    settle(&mut session, &mut bus, &mut cycles, 80.0);
    let ntc_v = signal_voltage(&session, "board.analog.LRADC1_volts");
    let diode_v = signal_voltage(&session, "board.analog.LRADC0_volts");
    assert_eq!(read_data(&mut bus, 1), code(ntc_v), "LRADC1");
    assert_eq!(read_data(&mut bus, 0), code(diode_v), "LRADC0");
    assert_ne!(code(ntc_v), code(diode_v), "the swap is observable");
}
