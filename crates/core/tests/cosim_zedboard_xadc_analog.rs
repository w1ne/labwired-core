// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! The generic NTC divider and diode load line reach the ZedBoard's Zynq-7000
//! XADC through the `analog_pins` map: the NTC on the dedicated `VP_0` pad
//! (DRP channel 3, VP/VN) and the diode on `AD8P` (VAUX8, DRP 0x18). Both are
//! read through the PS-XADC interface the way the Xilinx `xadcps` driver
//! does, which the explicit `adc.xadc.<ch>_volts` sweep in
//! `cosim_onboarding_analog.rs` does not cover:
//! - by pad name rather than channel number,
//! - VP/VN, which the sweep never routes,
//! - unipolar and bipolar transfers: single-channel mode with `CFR0.BU`
//!   (the NTC), and continuous-sequence mode with `SEQ05` bit 8 (the diode),
//!   both against +-0.5 V in two's complement.
//!
//! Register sequence (embeddedsw xadcps.c): `XAdcPs_CfgInitialize` sets
//! `CFG.ENABLE` and the FIFO thresholds, then clears `MCTL`;
//! `XAdcPs_WriteInternalReg` writes a DRP command and pops one `RDFIFO`
//! word; `XAdcPs_ReadInternalReg` writes a read, pops the dummy, writes the
//! dummy back as a no-op, and pops the result.

mod common;
use common::root;
use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::bus::SystemBus;
use labwired_core::cosim::{CosimSession, CosimSignalValue, RoutingError};
use labwired_core::Bus;
use std::path::Path;

const XADCIF: u64 = 0xF800_7100;
const CFG: u64 = XADCIF;
const CMDFIFO: u64 = XADCIF + 0x10;
const RDFIFO: u64 = XADCIF + 0x14;
const MCTL: u64 = XADCIF + 0x18;
const CFG_ENABLE_FIFOTH: u32 = (1 << 31) | (0xF << 20) | (0xF << 16);

const CMD_READ: u32 = 1 << 26;
const CMD_WRITE: u32 = 2 << 26;

const VPVN: u32 = 0x03;
const VAUX8: u32 = 0x18;
const VAUX0: u32 = 0x10;
const CFR0: u32 = 0x40;
const CFR1: u32 = 0x41;
const SEQ05: u32 = 0x4D;
const CFR0_BU: u16 = 1 << 10;
const SEQ_CONTINUOUS: u16 = 2 << 12;
const SEQ_SINGLE_CHANNEL: u16 = 3 << 12;

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

/// Unipolar: 1 LSB = 1 V / 4096, clamped at 0xFFF.
fn unipolar(volts: f64) -> u32 {
    (millivolts(volts) * 4096 / 1000).min(0xFFF)
}

/// Bipolar with VN at 0 V: positive inputs clamp at +2047.
fn bipolar(volts: f64) -> u32 {
    (millivolts(volts) * 4096 / 1000).min(0x7FF)
}

fn write_internal(bus: &mut SystemBus, address: u32, data: u16) {
    bus.write_u32(CMDFIFO, CMD_WRITE | (address << 16) | u32::from(data))
        .unwrap();
    bus.read_u32(RDFIFO).unwrap();
}

/// The 12-bit code from the MSB-justified status register.
fn read_code(bus: &mut SystemBus, channel: u32) -> u32 {
    bus.write_u32(CMDFIFO, CMD_READ | (channel << 16)).unwrap();
    let dummy = bus.read_u32(RDFIFO).unwrap();
    bus.write_u32(CMDFIFO, dummy).unwrap();
    let word = bus.read_u32(RDFIFO).unwrap();
    assert_eq!(word & 0xF, 0, "DRP 0x{channel:x}: 12 bits MSB-justified");
    word >> 4
}

#[test]
fn zedboard_ntc_and_diode_netlists_convert_through_vp_0_and_ad8p() {
    let system_path = root("configs/systems/onboarding/zedboard.yaml");
    let mut profile = SystemManifest::from_file(&system_path).expect("load ZedBoard profile");
    let chip_path = system_path.parent().unwrap().join(&profile.chip);
    profile.chip = chip_path.to_string_lossy().into_owned();
    let chip = ChipDescriptor::from_file(&chip_path).expect("load ZedBoard descriptor");
    let mut analog = SystemManifest::from_file(root(
        "examples/ci-fixture-unknown-arch/analog-ntc-diode-system.yaml",
    ))
    .expect("load generic NTC divider and diode netlists");
    for model in &mut analog.cosim_models {
        let (key, pad) = match model.id.as_str() {
            "ntc_divider" => ("v_sense", "vp_0"),
            "diode_load_line" => ("v_diode", "ad8p"),
            other => panic!("unexpected fixture model {other}"),
        };
        model
            .outputs
            .insert(key.into(), format!("board.analog.{pad}_volts"));
    }
    profile.cosim_models = analog.cosim_models;
    let mut bus = SystemBus::from_config(&chip, &profile).expect("build ZedBoard bus");
    let mut session = CosimSession::new(&profile.cosim_models, Path::new("."), &bus)
        .expect("build analog co-simulation")
        .expect("profile carries both generic devices");
    assert_eq!(
        session.binding_errors(),
        &[] as &[RoutingError],
        "VP_0/AD8P must resolve through analog_pins to XADC channels 3/0x18"
    );

    // XAdcPs_CfgInitialize.
    let cfg = bus.read_u32(CFG).unwrap();
    bus.write_u32(CFG, cfg | CFG_ENABLE_FIFOTH).unwrap();
    bus.write_u32(MCTL, 0).unwrap();

    let mut cycles = 0;
    let mut ntc_codes = Vec::new();
    for temperature_c in [-10.0_f64, 25.0, 80.0] {
        session
            .set_signal_number("ui.ntc.temperature_c", temperature_c)
            .expect("set NTC temperature");
        cycles += session.cycles_until_boundary(cycles);
        let (steps, errors) = session
            .advance_to(cycles, &mut bus)
            .expect("solve and route both netlists");
        assert_eq!(steps.len(), 2, "both netlists solve each step");
        assert!(errors.is_empty(), "route errors: {errors:?}");

        let ntc_v = signal_voltage(&session, "board.analog.vp_0_volts");
        let r = ntc_resistance_ohms(temperature_c);
        let expected_ntc_v = 3.3 * r / (10_000.0 + r);
        assert!(
            (ntc_v - expected_ntc_v).abs() < 1e-9,
            "at {temperature_c} °C the divider should be {expected_ntc_v} V, got {ntc_v} V"
        );
        let diode_v = signal_voltage(&session, "board.analog.ad8p_volts");
        assert!(
            (0.5..1.0).contains(&diode_v),
            "a forward-biased silicon diode above +0.5 V and inside 1 V: {diode_v} V"
        );

        // Default safe mode: everything unipolar.
        write_internal(&mut bus, CFR1, 0);
        let ntc_code = read_code(&mut bus, VPVN);
        assert_eq!(ntc_code, unipolar(ntc_v), "VP/VN converts the NTC");
        assert_eq!(ntc_code == 0xFFF, ntc_v >= 1.0, "VP/VN saturates above 1 V");
        assert_eq!(
            read_code(&mut bus, VAUX8),
            unipolar(diode_v),
            "VAUX8 converts the diode"
        );
        assert_eq!(read_code(&mut bus, VAUX0), 0, "unrouted VAUX0 reads 0 V");
        ntc_codes.push(ntc_code);

        // Single channel on VP/VN with CFR0.BU: +-0.5 V two's complement.
        write_internal(&mut bus, CFR1, SEQ_SINGLE_CHANNEL);
        write_internal(&mut bus, CFR0, CFR0_BU | VPVN as u16);
        let bipolar_ntc = read_code(&mut bus, VPVN);
        assert_eq!(bipolar_ntc, bipolar(ntc_v), "bipolar VP/VN");
        assert!(bipolar_ntc <= 0x7FF, "VN at 0 V never goes negative");
        assert_eq!(
            read_code(&mut bus, VAUX8),
            unipolar(diode_v),
            "BU applies to the selected channel only"
        );

        // Continuous sequence with VAUX8 bipolar: the diode sits above
        // +0.5 V, so it clamps at +2047.
        write_internal(&mut bus, CFR1, SEQ_CONTINUOUS);
        write_internal(&mut bus, SEQ05, 1 << 8);
        assert_eq!(read_code(&mut bus, VAUX8), 0x7FF, "bipolar VAUX8 clamps");
        assert_eq!(
            read_code(&mut bus, VPVN),
            unipolar(ntc_v),
            "SEQ04 clear: VP/VN unipolar"
        );
        write_internal(&mut bus, SEQ05, 0);
    }
    // -10 and 25 °C (2.79 V, 1.65 V) saturate the 1 V range; 80 °C resolves.
    assert_eq!(
        ntc_codes[..2],
        [0xFFF, 0xFFF],
        "dividers above 1 V saturate"
    );
    assert!(
        ntc_codes[2] < 0xFFF,
        "the hot divider resolves: {ntc_codes:?}"
    );
}
