// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! The generic NTC divider and diode load line reach the KV260's PL SYSMON
//! through the Zynq UltraScale+ AMS (`zynqmp_ams` at 0xFFA5_0000, embeddedsw
//! sysmonpsu `XSYSMONPSU_BASEADDR`).
//!
//! The board input is the VAUX11 pair on Pmod J2: VAUXP11 = C11
//! (`IO_L9P_AD11P_45`, `pmod[3]`), VAUXN11 = B10 (`IO_L9N_AD11N_45`,
//! `pmod[4]`), from Xilinx/Kria-PYNQ @8b975f43
//! `kv260/base/vivado/constraints/base.xdc`. It is the only complete
//! auxiliary pair on that connector, so the NTC and the diode each have a
//! system file and take the same channel in turn (16 + 11 = 27).
//!
//! The XCK26 RPU descriptor stays declared-only (CPU and memory). The example
//! systems add the AMS with a system-level `peripherals:` entry and route by
//! channel (`adc.ams.27_volts`). The register sequences are the Linux v6.12
//! `xilinx-ams.c` continuous sequence and the embeddedsw single-channel mode.
//!
//! Assumed, not modelled: a PL design that connects SYSMONE4 VAUXP/VAUXN[11]
//! to the two pads, and VAUXN11 at ground. The native RPU machine does not
//! instantiate the AMS.

mod common;
use common::root;
use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::bus::SystemBus;
use labwired_core::cosim::{CosimSession, CosimSignalValue, RoutingError};
use labwired_core::Bus;
use std::path::Path;

const AMS: u64 = 0xFFA5_0000;
const ISR_1: u64 = AMS + 0x14;
const IER_1: u64 = AMS + 0x24;
const IDR_1: u64 = AMS + 0x2C;
const PS_CSTS: u64 = AMS + 0x40;
const PL_CSTS: u64 = AMS + 0x44;
const EOC: u32 = 1 << 3;
const EOS: u32 = 1 << 4;
/// `XSYSMONPSU_PS_SYSMON_READY`.
const PS_SYSMON_READY: u32 = 0x0801_0000;

// PL SYSMON DRP addresses (xsysmonpsu_hw.h offsets / 4).
const CFG_REG0: u64 = 0x40;
const CFG_REG1: u64 = 0x41;
const SEQ_CH0: u64 = 0x48;
const SEQ_CH1: u64 = 0x49;
const SEQ_INPUT_MDE1: u64 = 0x4D;
const SEQ_DEFAULT: u32 = 0;
const SEQ_CONTINUOUS: u32 = 2 << 12;
const SEQ_SINGLE_CHANNEL: u32 = 3 << 12;

/// `XSM_CH_AUX_MIN` + 11.
const VAUX11: u64 = 16 + 11;
const ROUTE: &str = "adc.ams.27_volts";
const SUPPLY_V: f64 = 3.3;
const TOP_OHMS: f64 = 150_000.0;

/// A PL SYSMON register by DRP address (`XPL_BA_OFFSET` 0xC00).
fn pl(drp: u64) -> u64 {
    AMS + 0xC00 + drp * 4
}

fn ntc_resistance_ohms(temperature_c: f64) -> f64 {
    10_000.0 * (3950.0 * (1.0 / (temperature_c + 273.15) - 1.0 / 298.15)).exp()
}

fn ntc_volts(temperature_c: f64, top_ohms: f64) -> f64 {
    let r = ntc_resistance_ohms(temperature_c);
    SUPPLY_V * r / (top_ohms + r)
}

fn diode_load_line_voltage() -> f64 {
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

fn signal_voltage(session: &CosimSession) -> f64 {
    match session.signals().get(ROUTE) {
        Some(CosimSignalValue::F64(volts)) => *volts,
        other => panic!("expected solved voltage at {ROUTE}, got {other:?}"),
    }
}

/// 10 bits against 1 V, truncated, clamped (`XSysMonPsu_VpVnRawToVoltage`:
/// raw * 1 V / 65536, "the 10 MSB bits of the 16 data read").
fn code(volts: f64) -> u32 {
    let millivolts = (volts * 1000.0).round() as u32;
    (millivolts * 1024 / 1000).min(1023)
}

fn system(file: &str) -> SystemManifest {
    SystemManifest::from_file(root(&format!("examples/kv260-r5/{file}")))
        .unwrap_or_else(|e| panic!("load {file}: {e}"))
}

fn chip() -> ChipDescriptor {
    let chip = ChipDescriptor::from_file(root("configs/chips/xck26-rpu.yaml"))
        .expect("load XCK26 RPU descriptor");
    assert!(
        chip.peripherals.is_empty(),
        "the descriptor stays declared-only; the system adds the AMS"
    );
    chip
}

fn board(system: &SystemManifest) -> (SystemBus, CosimSession) {
    let bus = SystemBus::from_config(&chip(), system).expect("build KV260 RPU SystemBus");
    let session = CosimSession::new(&system.cosim_models, Path::new("."), &bus)
        .expect("build analog co-simulation")
        .expect("the system carries an analog device");
    assert_eq!(
        session.binding_errors(),
        &[] as &[RoutingError],
        "the netlist must bind to PL SYSMON VAUX11"
    );
    (bus, session)
}

/// Solve the netlist to the next boundary and route it.
fn settle(session: &mut CosimSession, bus: &mut SystemBus, cycles: &mut u64) {
    *cycles += session.cycles_until_boundary(*cycles);
    let (steps, errors) = session
        .advance_to(*cycles, bus)
        .expect("solve and route the netlist");
    assert_eq!(steps.len(), 1, "the netlist solves each step");
    assert!(errors.is_empty(), "route errors: {errors:?}");
}

fn ams_irq(bus: &SystemBus) -> bool {
    let ams = bus
        .peripherals
        .iter()
        .find(|p| p.name == "ams")
        .expect("ams on the bus");
    ams.dev
        .irq_line_level()
        .expect("zynqmp_ams reports its request")
}

/// `ams_init_device`: the PS SYSMON reports ready and the PL SYSMON is
/// accessible; mask and clear every event.
fn driver_init(bus: &mut SystemBus) {
    assert_eq!(
        bus.read_u32(PS_CSTS).unwrap() & PS_SYSMON_READY,
        PS_SYSMON_READY
    );
    assert_eq!(
        bus.read_u32(PL_CSTS).unwrap() & 1,
        1,
        "PL SYSMON accessible"
    );
    bus.write_u32(IDR_1, 0xE000_001F).expect("IDR_1");
    bus.write_u32(ISR_1, 0xE000_001F).expect("ISR_1");
}

/// `ams_enable_channel_sequence` with only VAUX11 in the scan: default mode,
/// the channel masks, then continuous mode.
fn driver_sequence_vaux11(bus: &mut SystemBus) {
    bus.write_u32(pl(CFG_REG1), SEQ_DEFAULT).expect("CFG_REG1");
    bus.write_u32(pl(SEQ_CH0), 0).expect("SEQ_CH0");
    bus.write_u32(pl(SEQ_CH1), 1 << 11).expect("SEQ_CH1");
    bus.write_u32(pl(SEQ_INPUT_MDE1), 0).expect("unipolar");
    bus.write_u32(pl(CFG_REG1), SEQ_CONTINUOUS)
        .expect("CFG_REG1");
}

/// Clear `flag`, wait for the next one, and read the 10-bit VAUX11 result.
fn read_vaux11(bus: &mut SystemBus, flag: u32) -> u32 {
    bus.write_u32(ISR_1, flag).expect("clear ISR_1");
    assert_eq!(bus.read_u32(ISR_1).unwrap() & flag, 0);
    bus.tick_peripherals();
    assert_eq!(bus.read_u32(ISR_1).unwrap() & flag, flag, "event latches");
    let word = bus.read_u32(pl(VAUX11)).unwrap();
    assert_eq!(word & !0xFFC0, 0, "10 bits, MSB-justified in 16");
    word >> 6
}

#[test]
fn kv260_pmod_vaux11_converts_the_ntc_divider() {
    let ntc = system("analog-vaux11-ntc-system.yaml");
    let (mut bus, mut session) = board(&ntc);
    driver_init(&mut bus);

    let mut cycles = 0;
    session
        .set_signal_number("ui.ntc.temperature_c", 25.0)
        .expect("set NTC temperature");
    settle(&mut session, &mut bus, &mut cycles);
    bus.tick_peripherals();
    assert_eq!(
        bus.read_u32(pl(VAUX11)).unwrap(),
        0,
        "the default sequence converts only the on-chip sensors"
    );

    driver_sequence_vaux11(&mut bus);
    assert!(!ams_irq(&bus), "every event is masked after init");
    let mut codes = Vec::new();
    for temperature_c in [-10.0_f64, 25.0, 50.0, 80.0] {
        session
            .set_signal_number("ui.ntc.temperature_c", temperature_c)
            .expect("set NTC temperature");
        settle(&mut session, &mut bus, &mut cycles);
        let volts = signal_voltage(&session);
        let expected = ntc_volts(temperature_c, TOP_OHMS);
        assert!(
            (volts - expected).abs() < 1e-9,
            "at {temperature_c} °C the divider should be {expected} V, got {volts} V"
        );
        let raw = read_vaux11(&mut bus, EOS);
        assert_eq!(raw, code(volts), "VAUX11 at {temperature_c} °C");
        assert!(raw > 0 && raw < 1023, "inside the 1 V range: {raw}");
        codes.push(raw);
    }
    assert!(
        codes.windows(2).all(|pair| pair[0] > pair[1]),
        "the NTC codes fall as it warms: {codes:?}"
    );
    // 3.3 V * 10 k / 160 k = 206.25 mV at 25 °C: 206 * 1024 / 1000.
    assert_eq!(codes[1], 210);
    // The driver scale is 1 V / 1024: within one 0.98 mV step of the node.
    let reported_uv = codes[0] * 1_000_000 / 1024;
    let true_uv = (ntc_volts(-10.0, TOP_OHMS) * 1e6).round() as u32;
    assert!(
        reported_uv <= true_uv + 500 && true_uv <= reported_uv + 1500,
        "{reported_uv} uV vs {true_uv} uV"
    );

    // `ams_unmask`: with EOS enabled the latched event raises the request.
    bus.write_u32(IER_1, EOS).expect("IER_1");
    assert!(ams_irq(&bus), "EOS is latched and now enabled");
    bus.write_u32(IDR_1, EOS).expect("IDR_1");
    assert!(!ams_irq(&bus));
}

#[test]
fn kv260_pmod_vaux11_converts_the_diode_in_single_channel_mode() {
    let diode = system("analog-vaux11-diode-system.yaml");
    let (mut bus, mut session) = board(&diode);
    driver_init(&mut bus);
    let mut cycles = 0;
    settle(&mut session, &mut bus, &mut cycles);
    let volts = signal_voltage(&session);
    let expected = diode_load_line_voltage();
    assert!(
        (volts - expected).abs() < 2e-5,
        "the Shockley load line should be {expected} V, got {volts} V"
    );
    assert!(volts > 0.5 && volts < 0.8, "a forward drop: {volts} V");

    // XSysMonPsu_SetSingleChParams: MUX_CH = VAUX11, unipolar, then mode 3.
    bus.write_u32(pl(CFG_REG1), SEQ_DEFAULT).expect("CFG_REG1");
    bus.write_u32(pl(CFG_REG0), VAUX11 as u32)
        .expect("CFG_REG0");
    bus.write_u32(pl(CFG_REG1), SEQ_SINGLE_CHANNEL)
        .expect("CFG_REG1");
    let raw = read_vaux11(&mut bus, EOC);
    assert_eq!(raw, code(volts));

    // The NTC system on the same pair gives a different code at 25 °C, so
    // the result follows the wired device, not the channel.
    let ntc = system("analog-vaux11-ntc-system.yaml");
    let (mut ntc_bus, mut ntc_session) = board(&ntc);
    let mut ntc_cycles = 0;
    settle(&mut ntc_session, &mut ntc_bus, &mut ntc_cycles);
    ntc_bus
        .write_u32(pl(CFG_REG0), VAUX11 as u32)
        .expect("CFG_REG0");
    ntc_bus
        .write_u32(pl(CFG_REG1), SEQ_SINGLE_CHANNEL)
        .expect("CFG_REG1");
    let ntc_raw = read_vaux11(&mut ntc_bus, EOC);
    assert_eq!(ntc_raw, code(signal_voltage(&ntc_session)));
    assert_ne!(ntc_raw, raw);
}

/// The input range is 1 V on a 3.3 V bank. The 10 k pull-up that suits a
/// 3.3 V converter drives the pair past full scale, and the code clamps.
#[test]
fn kv260_pmod_vaux11_clamps_an_unscaled_divider() {
    let mut ntc = system("analog-vaux11-ntc-system.yaml");
    let netlist = ntc.cosim_models[0]
        .config
        .get_mut("netlist_text")
        .expect("inline netlist");
    let text = netlist.as_str().expect("netlist text");
    assert!(text.contains("Rtop vcc sense 150k"));
    *netlist = text
        .replace("Rtop vcc sense 150k", "Rtop vcc sense 10k")
        .into();

    let (mut bus, mut session) = board(&ntc);
    driver_init(&mut bus);
    driver_sequence_vaux11(&mut bus);
    let mut cycles = 0;
    session
        .set_signal_number("ui.ntc.temperature_c", 25.0)
        .expect("set NTC temperature");
    settle(&mut session, &mut bus, &mut cycles);
    let volts = signal_voltage(&session);
    assert!((volts - ntc_volts(25.0, 10_000.0)).abs() < 1e-9);
    assert!((volts - 1.65).abs() < 1e-9, "half the 3.3 V rail");
    assert_eq!(read_vaux11(&mut bus, EOS), 0x3FF, "past 1 V full scale");
}

#[test]
fn declared_only_kv260_profile_refuses_the_adc_route() {
    let system_path = root("configs/systems/kv260-r5.yaml");
    let mut profile = SystemManifest::from_file(&system_path).expect("load KV260 R5 profile");
    assert_eq!(profile.name, "kv260-r5");
    assert!(profile.peripherals.is_empty());
    profile.cosim_models = system("analog-vaux11-ntc-system.yaml").cosim_models;
    let bus = SystemBus::from_config(&chip(), &profile).expect("build KV260 RPU SystemBus");
    let session = CosimSession::new(&profile.cosim_models, Path::new("."), &bus)
        .expect("build analog co-simulation")
        .expect("the system carries an analog device");
    assert_eq!(
        session.binding_errors().len(),
        1,
        "without the AMS the route is an error, not a guess: {:?}",
        session.binding_errors()
    );
}

/// The PL SYSMON takes external inputs on VP/VN and VAUX0..15 only; an
/// on-chip sensor channel is not a route.
#[test]
fn kv260_ams_refuses_a_route_to_an_on_chip_sensor_channel() {
    let mut ntc = system("analog-vaux11-ntc-system.yaml");
    // Channel 0 is the on-chip temperature sensor (XSM_CH_TEMP).
    ntc.cosim_models[0]
        .outputs
        .insert("v_sense".to_string(), "adc.ams.0_volts".to_string());
    let mut bus = SystemBus::from_config(&chip(), &ntc).expect("build KV260 RPU SystemBus");
    let mut session = CosimSession::new(&ntc.cosim_models, Path::new("."), &bus)
        .expect("build analog co-simulation")
        .expect("the system carries an analog device");
    assert!(
        session.binding_errors().is_empty(),
        "the block has 32 channels"
    );
    let cycles = session.cycles_until_boundary(0);
    let (_, errors) = session
        .advance_to(cycles, &mut bus)
        .expect("solve the netlist");
    assert_eq!(
        errors,
        vec![RoutingError::AdcUnavailable {
            path: "adc.ams.0_volts".to_string(),
            channel: 0
        }],
        "channel 0 must not accept a pad voltage"
    );
}
