// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! The generic NTC divider and diode load line reach the ZCU102's Zynq
//! UltraScale+ AMS through the `analog_pins` map: the NTC on the dedicated
//! `VP` pad (PL SYSMON channel 3, VP/VN) and the diode on `AD8P` (VAUX8,
//! channel 0x18). Both are read from the PL SYSMON status registers the way
//! the Linux `xilinx-ams` IIO driver does, which the explicit
//! `adc.ams.<ch>_volts` sweep in `cosim_onboarding_analog.rs` does not
//! cover:
//! - by pad name rather than channel number,
//! - VP/VN, which the sweep never routes,
//! - the sequencer gate: in default (safe) mode the PL SYSMON converts only
//!   its on-chip sensors, so a routed pad stays 0 until `SEQ_CH0/1` and a
//!   sequencer mode select it; a driven pad left out of `SEQ_CH1` stays 0,
//! - one-pass mode, whose latched result ignores a later input change until
//!   `CFG_REG1` is written again,
//! - bipolar transfers through `SEQ_INPUT_MDE0` (VP/VN) and `CFG_REG0.BU`.
//!
//! Register sequence (Linux v6.12 drivers/iio/adc/xilinx-ams.c):
//! `ams_init_device` polls `PS_CSTS` for `BIT(27) | BIT(16)`, checks
//! `PL_CSTS`, writes 0xFFFF to the PL `VP_VN` and puts `CFG_REG1.SEQ` in
//! default mode; `ams_enable_channel_sequence` writes `SEQ_CH0/1/2` in
//! default mode, then sets continuous mode. Offsets from embeddedsw
//! xsysmonpsu_hw.h.

mod common;
use common::root;
use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::bus::SystemBus;
use labwired_core::cosim::{CosimSession, CosimSignalValue, RoutingError};
use labwired_core::Bus;
use std::path::Path;

const AMS: u64 = 0xFFA5_0000;
const ISR_1: u64 = AMS + 0x14;
const PS_CSTS: u64 = AMS + 0x40;
const PL_CSTS: u64 = AMS + 0x44;
const PL: u64 = AMS + 0xC00;
const VP_VN: u64 = PL + 0x0C;
const VAUX0: u64 = PL + 0x40;
const VAUX8: u64 = PL + 0x60;
const CFG_REG0: u64 = PL + 0x100;
const CFG_REG1: u64 = PL + 0x104;
const SEQ_CH2: u64 = PL + 0x118;
const SEQ_CH0: u64 = PL + 0x120;
const SEQ_CH1: u64 = PL + 0x124;
const SEQ_INPUT_MDE0: u64 = PL + 0x130;

const EOC: u32 = 1 << 3;
const EOS: u32 = 1 << 4;
const SEQ_VP_VN: u32 = 1 << 11;
const CFG_REG0_BU: u32 = 1 << 10;
const SEQ_DEFAULT: u32 = 0;
const SEQ_ONEPASS: u32 = 1 << 12;
const SEQ_CONTINUOUS: u32 = 2 << 12;
const SEQ_SINGLE_CHANNEL: u32 = 3 << 12;

fn ntc_resistance_ohms(temperature_c: f64) -> f64 {
    10_000.0 * (3950.0 * (1.0 / (temperature_c + 273.15) - 1.0 / 298.15)).exp()
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

/// Unipolar: 1 LSB = 1 V / 1024, clamped at 0x3FF.
fn unipolar(volts: f64) -> u32 {
    (millivolts(volts) * 1024 / 1000).min(0x3FF)
}

/// Bipolar with VN at 0 V: positive inputs clamp at +511.
fn bipolar(volts: f64) -> u32 {
    (millivolts(volts) * 1024 / 1000).min(0x1FF)
}

/// The 10-bit code from the MSB-justified status register.
fn read_code(bus: &mut SystemBus, register: u64) -> u32 {
    let word = bus.read_u32(register).unwrap();
    assert_eq!(word & 0x3F, 0, "{register:#x}: 10 bits MSB-justified");
    word >> 6
}

/// Clear `ISR_1` and tick until `flag` latches.
fn wait(bus: &mut SystemBus, flag: u32) {
    bus.write_u32(ISR_1, flag).unwrap();
    for _ in 0..8 {
        if bus.read_u32(ISR_1).unwrap() & flag != 0 {
            return;
        }
        bus.tick_peripherals_fully();
    }
    panic!("ISR_1 {flag:#x} never latched");
}

/// `ams_enable_channel_sequence` on the PL SYSMON.
fn sequence(bus: &mut SystemBus, ch0: u32, ch1: u32, mode: u32) {
    bus.write_u32(CFG_REG1, SEQ_DEFAULT).unwrap();
    bus.write_u32(SEQ_CH0, ch0).unwrap();
    bus.write_u32(SEQ_CH1, ch1).unwrap();
    bus.write_u32(SEQ_CH2, 0).unwrap();
    bus.write_u32(CFG_REG1, mode).unwrap();
}

#[test]
fn zcu102_ntc_and_diode_netlists_convert_through_vp_and_ad8p() {
    let system_path = root("configs/systems/onboarding/zynqmp-zcu102-revB.yaml");
    let mut profile = SystemManifest::from_file(&system_path).expect("load ZCU102 profile");
    let chip_path = system_path.parent().unwrap().join(&profile.chip);
    profile.chip = chip_path.to_string_lossy().into_owned();
    let chip = ChipDescriptor::from_file(&chip_path).expect("load ZCU102 descriptor");
    let mut analog = SystemManifest::from_file(root(
        "examples/ci-fixture-unknown-arch/analog-ntc-diode-system.yaml",
    ))
    .expect("load generic NTC divider and diode netlists");
    for model in &mut analog.cosim_models {
        let (key, pad) = match model.id.as_str() {
            "ntc_divider" => ("v_sense", "vp"),
            "diode_load_line" => ("v_diode", "ad8p"),
            other => panic!("unexpected fixture model {other}"),
        };
        model
            .outputs
            .insert(key.into(), format!("board.analog.{pad}_volts"));
    }
    profile.cosim_models = analog.cosim_models;
    let mut bus = SystemBus::from_config(&chip, &profile).expect("build ZCU102 bus");
    let mut session = CosimSession::new(&profile.cosim_models, Path::new("."), &bus)
        .expect("build analog co-simulation")
        .expect("profile carries both generic devices");
    assert_eq!(
        session.binding_errors(),
        &[] as &[RoutingError],
        "VP/AD8P must resolve through analog_pins to AMS channels 3/0x18"
    );

    // ams_init_device.
    assert_eq!(
        bus.read_u32(PS_CSTS).unwrap() & 0x0801_0000,
        0x0801_0000,
        "PS SYSMON ready, as Renode's PythonPeripheral returned"
    );
    assert_ne!(bus.read_u32(PL_CSTS).unwrap(), 0, "PL SYSMON accessible");
    bus.write_u32(VP_VN, 0xFFFF).unwrap();
    bus.write_u32(CFG_REG1, SEQ_DEFAULT).unwrap();

    let mut cycles = 0;
    let mut advance = |session: &mut CosimSession, bus: &mut SystemBus, temperature_c: f64| {
        session
            .set_signal_number("ui.ntc.temperature_c", temperature_c)
            .expect("set NTC temperature");
        cycles += session.cycles_until_boundary(cycles);
        let (steps, errors) = session
            .advance_to(cycles, bus)
            .expect("solve and route both netlists");
        assert_eq!(steps.len(), 2, "both netlists solve each step");
        assert!(errors.is_empty(), "route errors: {errors:?}");
        let ntc_v = signal_voltage(session, "board.analog.vp_volts");
        let r = ntc_resistance_ohms(temperature_c);
        let expected_ntc_v = 3.3 * r / (10_000.0 + r);
        assert!(
            (ntc_v - expected_ntc_v).abs() < 1e-9,
            "at {temperature_c} °C the divider should be {expected_ntc_v} V, got {ntc_v} V"
        );
        let diode_v = signal_voltage(session, "board.analog.ad8p_volts");
        assert!(
            (0.5..1.0).contains(&diode_v),
            "a forward-biased silicon diode above +0.5 V and inside 1 V: {diode_v} V"
        );
        (ntc_v, diode_v)
    };

    // Negative control: routed and driven, but the default sequence never
    // reaches the external pads.
    advance(&mut session, &mut bus, 80.0);
    wait(&mut bus, EOS);
    assert_eq!(read_code(&mut bus, VP_VN), 0, "default mode skips VP/VN");
    assert_eq!(read_code(&mut bus, VAUX8), 0, "default mode skips VAUX8");

    let mut ntc_codes = Vec::new();
    for temperature_c in [-10.0_f64, 25.0, 80.0] {
        let (ntc_v, diode_v) = advance(&mut session, &mut bus, temperature_c);

        // Continuous sequence over VP/VN and VAUX8, both unipolar.
        sequence(&mut bus, SEQ_VP_VN, 1 << 8, SEQ_CONTINUOUS);
        wait(&mut bus, EOS);
        let ntc_code = read_code(&mut bus, VP_VN);
        assert_eq!(ntc_code, unipolar(ntc_v), "VP/VN converts the NTC");
        assert_eq!(ntc_code == 0x3FF, ntc_v >= 1.0, "VP/VN saturates above 1 V");
        assert_eq!(
            read_code(&mut bus, VAUX8),
            unipolar(diode_v),
            "VAUX8 converts the diode"
        );
        assert_eq!(read_code(&mut bus, VAUX0), 0, "unsequenced VAUX0 reads 0");
        ntc_codes.push(ntc_code);

        // VP/VN bipolar through SEQ_INPUT_MDE0 bit 11 (set in default mode,
        // as XSysMonPsu_SetSeqInputMode requires).
        bus.write_u32(CFG_REG1, SEQ_DEFAULT).unwrap();
        bus.write_u32(SEQ_INPUT_MDE0, SEQ_VP_VN).unwrap();
        bus.write_u32(CFG_REG1, SEQ_CONTINUOUS).unwrap();
        wait(&mut bus, EOS);
        let bipolar_ntc = read_code(&mut bus, VP_VN);
        assert_eq!(bipolar_ntc, bipolar(ntc_v), "bipolar VP/VN");
        assert!(bipolar_ntc <= 0x1FF, "VN at 0 V never goes negative");
        assert_eq!(
            read_code(&mut bus, VAUX8),
            unipolar(diode_v),
            "SEQ_INPUT_MDE1 clear: VAUX8 unipolar"
        );
        bus.write_u32(CFG_REG1, SEQ_DEFAULT).unwrap();
        bus.write_u32(SEQ_INPUT_MDE0, 0).unwrap();

        // Single channel on VAUX8 with CFG_REG0.BU: the diode sits above
        // +0.5 V, so it clamps at +511; EOC, no EOS from the PL SYSMON.
        bus.write_u32(CFG_REG0, CFG_REG0_BU | 0x18).unwrap();
        bus.write_u32(CFG_REG1, SEQ_SINGLE_CHANNEL).unwrap();
        wait(&mut bus, EOC);
        assert_eq!(read_code(&mut bus, VAUX8), 0x1FF, "bipolar VAUX8 clamps");
    }
    // -10 and 25 °C (2.79 V, 1.65 V) saturate the 1 V range; 80 °C resolves.
    assert_eq!(
        ntc_codes[..2],
        [0x3FF, 0x3FF],
        "dividers above 1 V saturate"
    );
    assert!(
        ntc_codes[2] < 0x3FF,
        "the hot divider resolves: {ntc_codes:?}"
    );

    // One pass latches the 80 °C reading; cooling the NTC to 25 °C changes
    // nothing until CFG_REG1 starts another pass.
    sequence(&mut bus, SEQ_VP_VN, 0, SEQ_ONEPASS);
    wait(&mut bus, EOC);
    let hot = read_code(&mut bus, VP_VN);
    assert_eq!(hot, ntc_codes[2], "one pass converts the hot divider");
    advance(&mut session, &mut bus, 25.0);
    for _ in 0..4 {
        bus.tick_peripherals_fully();
    }
    assert_eq!(read_code(&mut bus, VP_VN), hot, "one pass ran once");
    bus.write_u32(CFG_REG1, SEQ_ONEPASS).unwrap();
    bus.tick_peripherals_fully();
    assert_eq!(read_code(&mut bus, VP_VN), 0x3FF, "a new pass sees 25 °C");
}
