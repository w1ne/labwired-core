// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! The S32K388 EVB's potentiometer pads PTA11 (ADC_POT0) and PTA17
//! (ADC_POT1) carry the generic NTC divider and diode load line into the
//! SAR ADCs: PTA11 is `ADC1_S10` and PTA17 is `ADC2_S19` (RTD 7.0.1
//! Port_Cfg.h of an S32K388 project), resolved by pad name through the
//! descriptor's `analog_pins`. The firmware side is the RTD `Adc_Sar_Ip`
//! flow: power-down/power-up on `MSR.ADCSTATUS`, `SetResolution`,
//! `DoCalibration`, a one-shot normal chain on `NCMR1`, `ISR.ECH`, `CDR`
//! with `VALID`, and the `Adc_Sar_GetMaskedResult` unpacking.

mod common;
use common::root;
use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::bus::SystemBus;
use labwired_core::cosim::{CosimSession, CosimSignalValue, RoutingError};
use labwired_core::Bus;
use std::path::Path;
use std::sync::atomic::Ordering;

const ADC1: u64 = 0x400A_4000;
const ADC2: u64 = 0x400A_8000;
const MCR: u64 = 0x00;
const MSR: u64 = 0x04;
const ISR: u64 = 0x10;
const CEOCFR1: u64 = 0x18;
const IMR: u64 = 0x20;
const NCMR1: u64 = 0xA8;
const CDR0: u64 = 0x100;
const CALBISTREG: u64 = 0x3A0;
const PWDN: u32 = 1 << 0;
const NSTART: u32 = 1 << 24;
const MODE_SCAN: u32 = 1 << 29;
const WLSIDE: u32 = 1 << 30;
const OWREN: u32 = 1 << 31;
const ECH: u32 = 1 << 0;
const EOC: u32 = 1 << 1;
const VALID: u32 = 1 << 19;
const OVERW: u32 = 1 << 18;
const TEST_EN: u32 = 1 << 0;
const TEST_FAIL: u32 = 1 << 3;
const C_T_BUSY: u32 = 1 << 15;
/// `CALBISTREG.RESN`: 0 = 14, 1 = 12, 2 = 10, 3 = 8 bits.
const fn resn(bits: u32) -> u32 {
    (match bits {
        14 => 0,
        12 => 1,
        10 => 2,
        _ => 3,
    }) << 29
}
/// Standard-input channel ids (`ADCx_Sn` = 32 + n - 8).
const ADC1_S10: u8 = 34;
const ADC2_S19: u8 = 43;
/// `ADC1_S13` (PTA16), a real ADC1 pad left unwired by the pot fixture.
const ADC1_S13: u8 = 37;
/// `ADC1_S11` (PTB15), `ADC1_S12` (PTB16) and `ADC2_S12` (PTA21).
const ADC1_S11: u8 = 35;
const ADC1_S12: u8 = 36;
const ADC2_S12: u8 = 36;
/// `ADC1_IRQn` (S32K388_COMMON.h).
const ADC1_IRQ: usize = 181;

fn ntc_volts(temperature_c: f64) -> f64 {
    let r = 10_000.0 * (3950.0 * (1.0 / (temperature_c + 273.15) - 1.0 / 298.15)).exp();
    3.3 * r / (10_000.0 + r)
}

fn signal_voltage(session: &CosimSession, path: &str) -> f64 {
    match session.signals().get(path) {
        Some(CosimSignalValue::F64(volts)) => *volts,
        other => panic!("expected solved voltage at {path}, got {other:?}"),
    }
}

/// N-bit code against VREFH 3.3 V, truncated, clamped.
fn code(volts: f64, bits: u32) -> u32 {
    let mv = (volts * 1000.0).round() as i64;
    ((mv << bits) / 3300).clamp(0, (1 << bits) - 1) as u32
}

/// The S32K388 EVB with the generic netlists re-pointed from the SIUL2
/// GPIO inputs to the ADC pads of the same two pins.
fn board() -> (SystemBus, CosimSession) {
    let system_path = root("configs/systems/s32k388-evb.yaml");
    let mut profile = SystemManifest::from_file(&system_path).expect("load S32K388 EVB profile");
    let analog =
        SystemManifest::from_file(root("examples/s32k388-evb/analog-ntc-diode-system.yaml"))
            .expect("load generic Beta NTC and Shockley diode netlists");
    profile.cosim_models = analog.cosim_models;
    for model in &mut profile.cosim_models {
        let (key, pad) = match model.id.as_str() {
            "ntc_divider" => ("v_sense", "pta11"),
            "diode_load_line" => ("v_diode", "pta17"),
            other => panic!("unexpected model {other}"),
        };
        model
            .outputs
            .insert(key.to_string(), format!("board.analog.{pad}_volts"));
    }
    let chip_path = system_path.parent().unwrap().join(&profile.chip);
    let chip = ChipDescriptor::from_file(&chip_path).expect("load S32K388 descriptor");
    let bus = SystemBus::from_config(&chip, &profile).expect("build S32K388 EVB bus");
    let session = CosimSession::new(&profile.cosim_models, Path::new("."), &bus)
        .expect("build analog co-simulation")
        .expect("fixture carries both analog devices");
    assert_eq!(
        session.binding_errors(),
        &[] as &[RoutingError],
        "PTA11/PTA17 must resolve to ADC1/ADC2 inputs through analog_pins"
    );
    (bus, session)
}

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

/// `Adc_Sar_Ip_Init` power cycle, `SetResolution`, `DoCalibration`.
fn init(bus: &mut SystemBus, adc: u64, bits: u32) {
    bus.write_u32(adc + MCR, PWDN).expect("Powerdown");
    assert_eq!(
        bus.read_u32(adc + MSR).unwrap() & 7,
        1,
        "ADCSTATUS power-down"
    );
    bus.write_u32(adc + MCR, 0).expect("Powerup");
    assert_eq!(bus.read_u32(adc + MSR).unwrap() & 7, 0, "ADCSTATUS idle");
    bus.write_u32(adc + CALBISTREG, resn(bits) | TEST_FAIL)
        .expect("RESN, clear TEST_FAIL");
    bus.write_u32(adc + CALBISTREG, resn(bits) | TEST_EN)
        .expect("TEST_EN");
    assert_eq!(
        bus.read_u32(adc + CALBISTREG).unwrap() & (C_T_BUSY | TEST_FAIL),
        0,
        "calibration done and passed"
    );
}

/// One-shot normal chain on a standard input; returns the raw `CDR`.
fn convert(bus: &mut SystemBus, adc: u64, channel: u8, mcr: u32) -> u32 {
    let bit = 1u32 << (channel - 32);
    bus.write_u32(adc + NCMR1, bit).expect("NCMR1");
    bus.write_u32(adc + MCR, mcr | NSTART).expect("NSTART");
    assert_ne!(bus.read_u32(adc + ISR).unwrap() & ECH, 0, "ISR.ECH");
    assert_ne!(bus.read_u32(adc + CEOCFR1).unwrap() & bit, 0, "CEOCFR1");
    let cdr = bus
        .read_u32(adc + CDR0 + 4 * u64::from(channel))
        .expect("CDR");
    assert_ne!(cdr & VALID, 0, "CDR[{channel}] VALID");
    bus.write_u32(adc + CEOCFR1, bit).expect("CEOCFR1 w1c");
    bus.write_u32(adc + ISR, ECH | EOC).expect("ISR w1c");
    cdr
}

/// `Adc_Sar_GetMaskedResult`, right-aligned.
fn unpack(cdr: u32, bits: u32) -> u32 {
    (cdr & 0x7FFF & (0xFFFF << (15 - bits))) >> (15 - bits)
}

#[test]
fn s32k388_evb_pot_pads_convert_ntc_on_adc1_and_diode_on_adc2() {
    let (mut bus, mut session) = board();
    init(&mut bus, ADC1, 12);
    init(&mut bus, ADC2, 12);
    let mut cycles = 0;
    let mut ntc_codes = Vec::new();
    for temperature_c in [-10.0_f64, 25.0, 80.0] {
        settle(&mut session, &mut bus, &mut cycles, temperature_c);
        let ntc_v = signal_voltage(&session, "board.analog.pta11_volts");
        assert!((ntc_v - ntc_volts(temperature_c)).abs() < 1e-9);
        let diode_v = signal_voltage(&session, "board.analog.pta17_volts");
        assert!(diode_v > 0.5 && diode_v < 0.8, "diode drop {diode_v} V");

        let ntc = unpack(convert(&mut bus, ADC1, ADC1_S10, 0), 12);
        assert_eq!(ntc, code(ntc_v, 12), "ADC1_S10 at {temperature_c} °C");
        let diode = unpack(convert(&mut bus, ADC2, ADC2_S19, 0), 12);
        assert_eq!(diode, code(diode_v, 12), "ADC2_S19");
        // S10 on ADC2 and S19 on ADC1 are other pads: nothing drives them.
        assert_eq!(unpack(convert(&mut bus, ADC2, ADC1_S10, 0), 12), 0);
        assert_eq!(unpack(convert(&mut bus, ADC1, ADC2_S19, 0), 12), 0);
        assert_eq!(unpack(convert(&mut bus, ADC1, ADC1_S13, 0), 12), 0, "PTA16");
        ntc_codes.push(ntc);
    }
    assert!(
        ntc_codes[0] > ntc_codes[1] && ntc_codes[1] > ntc_codes[2],
        "the NTC codes fall as it warms: {ntc_codes:?}"
    );
}

#[test]
fn s32k388_evb_resolution_alignment_and_powered_down_start() {
    let (mut bus, mut session) = board();
    let mut cycles = 0;
    settle(&mut session, &mut bus, &mut cycles, 25.0);
    let ntc_v = signal_voltage(&session, "board.analog.pta11_volts");

    // At reset the ADC is powered down: NSTART converts nothing.
    bus.write_u32(ADC1 + NCMR1, 1 << (ADC1_S10 - 32))
        .expect("NCMR1");
    bus.write_u32(ADC1 + MCR, PWDN | NSTART)
        .expect("NSTART while powered down");
    assert_eq!(bus.read_u32(ADC1 + ISR).unwrap(), 0, "no chain");
    assert_eq!(
        bus.read_u32(ADC1 + CDR0 + 4 * u64::from(ADC1_S10)).unwrap() & VALID,
        0
    );

    for bits in [14, 12, 10, 8] {
        init(&mut bus, ADC1, bits);
        let right = convert(&mut bus, ADC1, ADC1_S10, 0);
        assert_eq!(unpack(right, bits), code(ntc_v, bits), "{bits}-bit right");
        let left = convert(&mut bus, ADC1, ADC1_S10, WLSIDE) & 0xFFFF;
        assert_eq!(left, code(ntc_v, bits) << (16 - bits), "{bits}-bit left");
    }
}

#[test]
fn s32k388_evb_scan_mode_follows_the_ntc_and_pends_adc1_irq() {
    let (mut bus, mut session) = board();
    init(&mut bus, ADC1, 12);
    let _ = labwired_core::system::cortex_m::configure_cortex_m(&mut bus);
    let pending = |bus: &SystemBus| {
        bus.nvic
            .as_ref()
            .map(|n| n.ispr[ADC1_IRQ / 32].load(Ordering::SeqCst) & (1 << (ADC1_IRQ % 32)) != 0)
    };
    assert_eq!(pending(&bus), Some(false), "no IRQ before a conversion");

    // Scan mode: one NSTART, then a new chain per tick.
    bus.write_u32(ADC1 + NCMR1, 1 << (ADC1_S10 - 32))
        .expect("NCMR1");
    bus.write_u32(ADC1 + IMR, ECH).expect("IMR.MSKECH");
    bus.write_u32(ADC1 + MCR, MODE_SCAN | NSTART)
        .expect("scan start");
    assert_ne!(bus.read_u32(ADC1 + MSR).unwrap() & NSTART, 0, "MSR.NSTART");
    let cdr_at = ADC1 + CDR0 + 4 * u64::from(ADC1_S10);
    // Without OWREN the first, unread result (taken before the netlists
    // were solved) survives the next scan, which only flags OVERW.
    let mut cycles = 0;
    settle(&mut session, &mut bus, &mut cycles, -10.0);
    bus.tick_peripherals_fully();
    let stale = bus.read_u32(cdr_at).unwrap();
    assert_ne!(stale & OVERW, 0, "OVERW");
    assert_eq!(unpack(stale, 12), 0, "OWREN = 0 keeps the unread result");
    // With OWREN each scan replaces the data.
    bus.write_u32(ADC1 + MCR, MODE_SCAN | OWREN | NSTART)
        .expect("scan with OWREN");
    for temperature_c in [-10.0_f64, 25.0, 80.0, 25.0] {
        settle(&mut session, &mut bus, &mut cycles, temperature_c);
        let ntc_v = signal_voltage(&session, "board.analog.pta11_volts");
        bus.tick_peripherals_fully();
        let cdr = bus.read_u32(cdr_at).unwrap();
        assert_ne!(cdr & VALID, 0, "fresh scan result");
        assert_eq!(
            unpack(cdr, 12),
            code(ntc_v, 12),
            "scan at {temperature_c} °C"
        );
    }
    assert_eq!(pending(&bus), Some(true), "ADC1_IRQn pends on ISR.ECH");
    bus.write_u32(ADC1 + MCR, MODE_SCAN | OWREN)
        .expect("stop scan");
    assert_eq!(bus.read_u32(ADC1 + MSR).unwrap() & NSTART, 0);
}

/// BJT quasi-saturation (`RCO`/`VO`/`GAMMA`) on the SAR ADCs. Four switch
/// stages share one base drive and differ only in the collector model; the
/// expected collector voltages are ngspice-42 `.op` values (`.temp 26.85`,
/// `tnom=26.85`). An engine that ignored `RCO` would put the PTA16 stage on
/// the PTB15 values, and one that read it as a plain resistor on the PTB16
/// values.
#[test]
fn s32k388_evb_bjt_quasi_saturation_reaches_adc1_and_adc2_beside_ntc_and_diode() {
    let system_path = root("configs/systems/s32k388-evb.yaml");
    let mut profile = SystemManifest::from_file(&system_path).expect("load S32K388 EVB profile");
    let analog = SystemManifest::from_file(root(
        "examples/s32k388-evb/analog-bjt-quasi-saturation-adc.yaml",
    ))
    .expect("load NTC, diode and BJT switch netlists");
    profile.cosim_models = analog.cosim_models;
    let chip_path = system_path.parent().unwrap().join(&profile.chip);
    let chip = ChipDescriptor::from_file(&chip_path).expect("load S32K388 descriptor");
    for (pad, peripheral, channel) in [
        ("PTA16", "adc1", ADC1_S13),
        ("PTB15", "adc1", ADC1_S11),
        ("PTB16", "adc1", ADC1_S12),
        ("PTA21", "adc2", ADC2_S12),
    ] {
        assert_eq!(chip.analog_pins[pad].peripheral, peripheral, "{pad}");
        assert_eq!(chip.analog_pins[pad].channel, channel, "{pad}");
    }
    let mut bus = SystemBus::from_config(&chip, &profile).expect("build S32K388 EVB bus");
    let mut session = CosimSession::new(&profile.cosim_models, Path::new("."), &bus)
        .expect("build analog co-simulation")
        .expect("fixture carries six analog devices");
    assert_eq!(
        session.binding_errors(),
        &[] as &[RoutingError],
        "every pad must resolve to an ADC1/ADC2 input through analog_pins"
    );
    init(&mut bus, ADC1, 12);
    init(&mut bus, ADC2, 12);
    session
        .set_signal_number("ui.ntc.temperature_c", 25.0)
        .expect("set NTC temperature");

    // (base drive, quasi-saturation, RC only, RC = 52 Ω, PNP mirror).
    let ngspice = [
        (
            0.0,
            3.299_999_999_338_5,
            3.299_999_999_338_5,
            3.299_999_999_338_5,
            3.314_999_985_751_9e-10,
        ),
        (
            0.8,
            2.412_357_176_719_6,
            2.412_357_173_140_8,
            2.412_357_176_666_8,
            8.876_428_431_343_4e-1,
        ),
        (
            1.2,
            9.196_663_842_814_4e-1,
            1.844_354_238_128_9e-1,
            1.193_335_448_565_9,
            2.380_333_617_669_5,
        ),
        (
            2.0,
            5.164_523_078_025_2e-1,
            1.391_453_167_792_8e-1,
            1.171_130_339_596_0,
            2.783_547_693_160_3,
        ),
        (
            3.3,
            2.235_675_251_767_3e-1,
            1.199_240_335_400_7e-1,
            1.159_979_452_818_3,
            3.076_432_475_002_1,
        ),
    ];
    let mut cycles = 0;
    let mut quasi_codes = Vec::new();
    for (drive, quasi_ref, ohmic_ref, resistive_ref, pnp_ref) in ngspice {
        session
            .set_signal_number("ui.bjt_switch.base_v", drive)
            .expect("set the common base drive");
        cycles += session.cycles_until_boundary(cycles);
        let (steps, errors) = session
            .advance_to(cycles, &mut bus)
            .expect("solve and route all six netlists");
        assert_eq!(steps.len(), 6, "all six netlists solve each step");
        assert!(errors.is_empty(), "route errors: {errors:?}");

        let ntc_v = signal_voltage(&session, "board.analog.pta11_volts");
        assert!((ntc_v - ntc_volts(25.0)).abs() < 1e-9, "NTC stays isolated");
        let diode_v = signal_voltage(&session, "board.analog.pta17_volts");
        assert!(diode_v > 0.5 && diode_v < 0.8, "diode drop {diode_v} V");
        assert_eq!(
            unpack(convert(&mut bus, ADC1, ADC1_S10, 0), 12),
            code(ntc_v, 12)
        );
        assert_eq!(
            unpack(convert(&mut bus, ADC2, ADC2_S19, 0), 12),
            code(diode_v, 12)
        );

        let mut codes = [0_u32; 4];
        for (slot, (pad, adc, channel, reference)) in [
            ("pta16", ADC1, ADC1_S13, quasi_ref),
            ("ptb15", ADC1, ADC1_S11, ohmic_ref),
            ("ptb16", ADC1, ADC1_S12, resistive_ref),
            ("pta21", ADC2, ADC2_S12, pnp_ref),
        ]
        .into_iter()
        .enumerate()
        {
            let volts = signal_voltage(&session, &format!("board.analog.{pad}_volts"));
            assert!(
                (volts - reference).abs() < 2e-5,
                "{pad} at {drive} V drive: ngspice-42 {reference} V, got {volts} V"
            );
            codes[slot] = unpack(convert(&mut bus, adc, channel, 0), 12);
            assert_eq!(codes[slot], code(volts, 12), "{pad} at {drive} V drive");
            assert!(
                codes[slot].abs_diff(code(reference, 12)) <= 1,
                "{pad} at {drive} V drive: code {} against ngspice-42 {reference} V",
                codes[slot]
            );
        }
        let [quasi, ohmic, resistive, pnp] = codes;
        // The PNP stage is the mirror image about the 3.3 V rail.
        assert!(
            (quasi + pnp).abs_diff(code(3.3, 12)) <= 2,
            "PNP mirror at {drive} V drive: {quasi} + {pnp}"
        );
        if drive >= 1.2 {
            // The epitaxial region sits between a hard-saturated collector
            // and an unmodulated 52 Ω one, at least 100 codes from each.
            assert!(
                quasi > ohmic + 100 && quasi + 100 < resistive,
                "{drive} V drive: RC only {ohmic}, quasi-saturation {quasi}, 52 Ω {resistive}"
            );
        }
        quasi_codes.push(quasi);
    }
    // More base drive injects more carriers into the epitaxial region and
    // pulls the collector down; the 52 Ω stage moves by under 50 codes over
    // the last three drives.
    assert!(
        quasi_codes.windows(2).all(|pair| pair[0] > pair[1]),
        "the collector code falls with base drive: {quasi_codes:?}"
    );
    assert!(
        quasi_codes[2] - quasi_codes[4] > 800,
        "conductivity modulation spans the saturated drives: {quasi_codes:?}"
    );
}
