// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! The generic NTC divider and diode load line reach the MIMXRT595-EVK's
//! LPADC0 through the `analog_pins` map and convert the way Zephyr v4.0.0
//! `adc_mcux_lpadc.c` drives it: `LPADC_Init`, one command per channel
//! chained by `CMDH.NEXT`, trigger 0 by software, the FIFO0 watermark
//! interrupt, and `RESFIFO0` reads. Both netlists run from 1.8 V, the
//! EVK's VDDA/VREFH (the Zephyr EVK overlay sets `zephyr,vref-mv = 1800`).
//! Also:
//! - the Zephyr overlay's differential channel (`MCUX_LPADC_CH0B` minus
//!   `MCUX_LPADC_CH0A`, P0_6 - P0_5) converts the signed difference of the
//!   two netlists, decoded the way the Zephyr ISR does it;
//! - the secure alias `0x5013_A000` reaches the same block, and the FIFO0
//!   watermark pends `ADC0_IRQn` (22) in the NVIC;
//! - a chain that loops back on itself keeps following the NTC with no
//!   further trigger.

mod common;
use common::root;
use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::bus::SystemBus;
use labwired_core::cosim::{CosimSession, CosimSignalValue, RoutingError};
use labwired_core::Bus;
use std::path::Path;
use std::sync::atomic::Ordering;

const ADC0: u64 = 0x4013_A000;
const ADC0_SECURE: u64 = 0x5013_A000;
const ADC0_IRQN: u32 = 22;
const CTRL: u64 = 0x10;
const STAT: u64 = 0x14;
const IE: u64 = 0x18;
const CFG: u64 = 0x20;
const SWTRIG: u64 = 0x34;
const TCTRL0: u64 = 0xA0;
const FCTRL0: u64 = 0xE0;
const CMD1: u64 = 0x100;
const RESFIFO0: u64 = 0x300;

const CTRL_ADCEN: u32 = 1 << 0;
const CTRL_RST: u32 = 1 << 1;
const CTRL_DOZEN: u32 = 1 << 2;
const CTRL_RSTFIFO0: u32 = 1 << 8;
const CTRL_RSTFIFO1: u32 = 1 << 9;
const STAT_RDY0: u32 = 1 << 0;
const STAT_ADC_ACTIVE: u32 = 1 << 11;
const IE_FWMIE0: u32 = 1 << 0;
const CFG_PWREN: u32 = 1 << 28;
const CMDL_ABSEL: u32 = 1 << 5;
const CMDL_DIFF: u32 = 1 << 6;
const CMDL_CSCALE: u32 = 1 << 13;
const CMDH_NEXT_SHIFT: u32 = 24;
const TCTRL_TCMD_SHIFT: u32 = 24;
const RESFIFO_VALID: u32 = 1 << 31;
const RESFIFO_CMDSRC_SHIFT: u32 = 24;

const SUPPLY_MV: i64 = 1800;

fn ntc_resistance_ohms(temperature_c: f64) -> f64 {
    10_000.0 * (3950.0 * (1.0 / (temperature_c + 273.15) - 1.0 / 298.15)).exp()
}

fn ntc_volts(temperature_c: f64) -> f64 {
    let r = ntc_resistance_ohms(temperature_c);
    1.8 * r / (10_000.0 + r)
}

fn signal_voltage(session: &CosimSession, path: &str) -> f64 {
    match session.signals().get(path) {
        Some(CosimSignalValue::F64(volts)) => *volts,
        other => panic!("expected solved voltage at {path}, got {other:?}"),
    }
}

fn millivolts(volts: f64) -> i64 {
    (volts * 1000.0).round() as i64
}

/// 12-bit single-ended code against VREFH 1.8 V, as it sits in `D[14:3]`.
fn se_d(volts: f64) -> u32 {
    (((millivolts(volts) << 12) / SUPPLY_MV).clamp(0, 4095) as u32) << 3
}

/// The Zephyr ISR's decode of a 13-bit differential `D`.
fn zephyr_diff(d: u32) -> i64 {
    let mut result = i64::from((d >> 3) & 0xFFF);
    if d & 0x8000 != 0 {
        result -= 0x1000;
    }
    result
}

/// The MIMXRT595-EVK with the generic NTC and diode netlists, moved to the
/// EVK's 1.8 V analog rail, routed to `board.analog.<pad>_volts`.
fn board(ntc_pad: &str, diode_pad: &str) -> (SystemBus, CosimSession) {
    let system_path = root("configs/systems/mimxrt595-evk.yaml");
    let mut profile = SystemManifest::from_file(&system_path).expect("load MIMXRT595-EVK profile");
    let analog =
        SystemManifest::from_file(root("examples/mimxrt595-evk/analog-ntc-diode-system.yaml"))
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
        let netlist = model.config["netlist_text"]
            .as_str()
            .expect("netlist_text")
            .replace("dc 3.3", "dc 1.8");
        assert!(
            netlist.contains("dc 1.8"),
            "the fixture runs from a DC rail"
        );
        model.config.insert("netlist_text".into(), netlist.into());
        model.config.insert("vdd".into(), 1.8.into());
    }
    let chip_path = system_path.parent().unwrap().join(&profile.chip);
    let chip = ChipDescriptor::from_file(&chip_path).expect("load MIMXRT595S descriptor");
    let bus = SystemBus::from_config(&chip, &profile).expect("build MIMXRT595-EVK bus");
    let session = CosimSession::new(&profile.cosim_models, Path::new("."), &bus)
        .expect("build analog co-simulation")
        .expect("fixture carries both analog devices");
    assert_eq!(
        session.binding_errors(),
        &[] as &[RoutingError],
        "{ntc_pad}/{diode_pad} must resolve to LPADC0 inputs through analog_pins"
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

/// fsl_lpadc LPADC_Init with Zephyr's config (`enableAnalogPreliminary`,
/// FWMARK 0), then `LPADC_EnableInterrupts(FIFO0 watermark)`.
fn zephyr_init(bus: &mut SystemBus, base: u64) {
    bus.write_u32(base + CTRL, CTRL_RST).expect("CTRL.RST");
    bus.write_u32(base + CTRL, 0).expect("release RST");
    bus.write_u32(base + CTRL, CTRL_RSTFIFO0).expect("RSTFIFO0");
    bus.write_u32(base + CTRL, CTRL_RSTFIFO1).expect("RSTFIFO1");
    bus.write_u32(base + CTRL, CTRL_DOZEN).expect("disable");
    bus.write_u32(base + CFG, CFG_PWREN | (0x80 << 16))
        .expect("CFG");
    bus.write_u32(base + FCTRL0, 0).expect("FCTRL0");
    bus.write_u32(base + FCTRL0 + 4, 0).expect("FCTRL1");
    bus.write_u32(base + CTRL, CTRL_DOZEN | CTRL_ADCEN)
        .expect("ADCEN");
    bus.write_u32(base + IE, IE_FWMIE0).expect("IE.FWMIE0");
}

fn command(bus: &mut SystemBus, base: u64, id: u64, cmdl: u32, next: u32) {
    let at = base + CMD1 + (id - 1) * 8;
    bus.write_u32(at, cmdl).expect("CMDL");
    bus.write_u32(at + 4, next << CMDH_NEXT_SHIFT)
        .expect("CMDH");
}

/// One valid `RESFIFO0` entry: (command, D).
fn pop(bus: &mut SystemBus, base: u64) -> (u32, u32) {
    let entry = bus.read_u32(base + RESFIFO0).expect("RESFIFO0");
    assert_ne!(entry & RESFIFO_VALID, 0, "a result is waiting");
    ((entry >> RESFIFO_CMDSRC_SHIFT) & 0xF, entry & 0xFFFF)
}

#[test]
fn mimxrt595_evk_lpadc0_converts_the_ntc_and_diode_chain_on_p0_5_and_p0_19() {
    // NTC on J30-1 (P0_5 = CH0A), diode on J30-3 (P0_19 = CH2A).
    let (mut bus, mut session) = board("p0_5", "p0_19");
    zephyr_init(&mut bus, ADC0);
    // Zephyr channels 0 and 1 -> commands 1 and 2, CMD1.NEXT = 2;
    // trigger 0 -> CMD1.
    command(&mut bus, ADC0, 1, CMDL_CSCALE, 2);
    command(&mut bus, ADC0, 2, CMDL_CSCALE | 2, 0);
    bus.write_u32(ADC0 + TCTRL0, 1 << TCTRL_TCMD_SHIFT)
        .expect("TCTRL0");

    let mut cycles = 0;
    let mut ntc_codes = Vec::new();
    for temperature_c in [-10.0_f64, 25.0, 80.0] {
        settle(&mut session, &mut bus, &mut cycles, temperature_c);
        let ntc_v = signal_voltage(&session, "board.analog.p0_5_volts");
        assert!((ntc_v - ntc_volts(temperature_c)).abs() < 1e-9);
        let diode_v = signal_voltage(&session, "board.analog.p0_19_volts");
        assert!(diode_v > 0.45 && diode_v < 0.75, "diode drop {diode_v} V");

        assert_eq!(bus.read_u32(ADC0 + STAT).unwrap() & STAT_RDY0, 0);
        bus.write_u32(ADC0 + SWTRIG, 1).expect("SWTRIG.SWT0");
        assert_ne!(bus.read_u32(ADC0 + STAT).unwrap() & STAT_RDY0, 0);
        assert_eq!(bus.read_u32(ADC0 + FCTRL0).unwrap() & 0x1F, 2);

        let (cmd, ntc_d) = pop(&mut bus, ADC0);
        assert_eq!(cmd, 1);
        assert_eq!(ntc_d, se_d(ntc_v), "CH0A at {temperature_c} °C");
        let (cmd, diode_d) = pop(&mut bus, ADC0);
        assert_eq!(cmd, 2, "CMD1.NEXT chains CMD2");
        assert_eq!(diode_d, se_d(diode_v), "CH2A at {temperature_c} °C");
        assert_eq!(
            bus.read_u32(ADC0 + RESFIFO0).unwrap() & RESFIFO_VALID,
            0,
            "the chain ends at CMD2"
        );
        ntc_codes.push(ntc_d >> 3);
    }
    assert!(
        ntc_codes[0] > ntc_codes[1] && ntc_codes[1] > ntc_codes[2],
        "the NTC codes fall as it warms: {ntc_codes:?}"
    );
    // At 25 °C the divider sits at half the rail: code 2048 of 4096.
    assert!((ntc_codes[1] as i64 - 2048).abs() <= 2, "{ntc_codes:?}");

    // CH1A (P0_12) carries nothing.
    command(&mut bus, ADC0, 3, CMDL_CSCALE | 1, 0);
    bus.write_u32(ADC0 + TCTRL0 + 4, 3 << TCTRL_TCMD_SHIFT)
        .expect("TCTRL1");
    bus.write_u32(ADC0 + SWTRIG, 1 << 1).expect("SWT1");
    assert_eq!(pop(&mut bus, ADC0), (3, 0));
}

#[test]
fn mimxrt595_evk_zephyr_ch0b_minus_ch0a_differential_follows_both_netlists() {
    // NTC on P0_5 (CH0A), diode on P0_6 (CH0B): the overlay's channel 0.
    let (mut bus, mut session) = board("p0_5", "p0_6");
    zephyr_init(&mut bus, ADC0);
    // kLPADC_SampleChannelDiffBothSideBA: DIFF | ABSEL, CH0B - CH0A.
    command(&mut bus, ADC0, 1, CMDL_CSCALE | CMDL_DIFF | CMDL_ABSEL, 0);
    // Single-ended B side of the same channel.
    command(&mut bus, ADC0, 2, CMDL_CSCALE | CMDL_ABSEL, 0);
    bus.write_u32(ADC0 + TCTRL0, 1 << TCTRL_TCMD_SHIFT)
        .expect("TCTRL0");
    bus.write_u32(ADC0 + TCTRL0 + 4, 2 << TCTRL_TCMD_SHIFT)
        .expect("TCTRL1");

    let mut cycles = 0;
    let mut signs = Vec::new();
    for temperature_c in [-10.0_f64, 25.0, 80.0] {
        settle(&mut session, &mut bus, &mut cycles, temperature_c);
        let ntc = millivolts(signal_voltage(&session, "board.analog.p0_5_volts"));
        let diode = millivolts(signal_voltage(&session, "board.analog.p0_6_volts"));
        bus.write_u32(ADC0 + SWTRIG, 0b11).expect("SWT0 | SWT1");
        let (_, d) = pop(&mut bus, ADC0);
        let expected = ((diode - ntc) << 12) / SUPPLY_MV;
        assert_eq!(
            zephyr_diff(d),
            expected,
            "CH0B - CH0A at {temperature_c} °C ({diode} - {ntc} mV)"
        );
        let (_, b) = pop(&mut bus, ADC0);
        assert_eq!(b, se_d(diode as f64 / 1000.0), "CH0B single-ended");
        signs.push(expected.signum());
    }
    // The NTC divider (1.52 V, 0.90 V, 0.20 V) drops below the diode
    // (~0.6 V) between 25 and 80 °C, so the difference changes sign.
    assert_eq!(signs, [-1, -1, 1]);
}

#[test]
fn mimxrt595_evk_secure_alias_irq_22_and_a_free_running_chain() {
    let (mut bus, mut session) = board("p0_5", "p0_19");
    // Zephyr runs secure: the same block at 0x5013_A000.
    zephyr_init(&mut bus, ADC0_SECURE);
    assert_eq!(bus.read_u32(ADC0 + IE).unwrap(), IE_FWMIE0, "same block");
    let mut cycles = 0;
    settle(&mut session, &mut bus, &mut cycles, -10.0);

    let _ = labwired_core::system::cortex_m::configure_cortex_m(&mut bus);
    let pending = |bus: &SystemBus| {
        bus.nvic
            .as_ref()
            .map(|n| n.ispr[0].load(Ordering::SeqCst) & (1 << ADC0_IRQN) != 0)
    };
    // CMD1 -> CMD1: a free-running conversion of CH0A.
    command(&mut bus, ADC0_SECURE, 1, CMDL_CSCALE, 1);
    bus.write_u32(ADC0_SECURE + TCTRL0, 1 << TCTRL_TCMD_SHIFT)
        .expect("TCTRL0");
    bus.tick_peripherals_fully();
    assert_eq!(pending(&bus), Some(false), "no IRQ before the trigger");
    bus.write_u32(ADC0_SECURE + SWTRIG, 1).expect("SWT0");
    bus.tick_peripherals_fully();
    assert_eq!(pending(&bus), Some(true), "ADC0_IRQn pends on FIFO0 RDY");
    assert_ne!(
        bus.read_u32(ADC0_SECURE + STAT).unwrap() & STAT_ADC_ACTIVE,
        0
    );

    for temperature_c in [25.0_f64, 80.0, -10.0] {
        settle(&mut session, &mut bus, &mut cycles, temperature_c);
        let ntc_v = signal_voltage(&session, "board.analog.p0_5_volts");
        bus.write_u32(ADC0 + CTRL, CTRL_DOZEN | CTRL_ADCEN | CTRL_RSTFIFO0)
            .expect("drop the older results");
        bus.tick_peripherals_fully();
        let (cmd, d) = pop(&mut bus, ADC0_SECURE);
        assert_eq!(cmd, 1);
        assert_eq!(d, se_d(ntc_v), "free-running CH0A at {temperature_c} °C");
    }
    // Clearing ADCEN stops it: no new entry after the FIFO is emptied.
    bus.write_u32(ADC0 + CTRL, CTRL_DOZEN | CTRL_RSTFIFO0)
        .expect("ADCEN = 0");
    bus.tick_peripherals_fully();
    assert_eq!(bus.read_u32(ADC0 + RESFIFO0).unwrap(), 0);
    assert_eq!(bus.read_u32(ADC0 + STAT).unwrap() & STAT_ADC_ACTIVE, 0);
}
