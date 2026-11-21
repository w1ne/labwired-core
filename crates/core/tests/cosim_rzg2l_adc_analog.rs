// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! The generic NTC divider and diode load line reach the RZ/G2L A/D
//! converter (`renesas_adc_c` at 0x1005_9000, Linux r9a07g044.dtsi) on the
//! onboarding RZ/G2L through the descriptor's `analog_pins` map. ADC_CH0..7
//! are dedicated inputs: ADC_CHn is channel n. The NTC is on ADC_CH1 and the
//! diode on ADC_CH4, read two ways:
//! - Linux v6.12 `rzg2l_adc.c`: `ADM0 = SRESB`, `ADM0 |= PWDWNB`, `ADM1`
//!   select mode with the software trigger, `ADM2.CHSEL = BIT(ch)`,
//!   `ADINT = CSEEN | BIT(ch)`, `ADM0 |= ADCE`, then the scan-end ISR reads
//!   `ADSTS.INTST`, each flagged `ADCRn` [11:0], and writes `ADSTS` back;
//! - RZ/G FSP `r_adc_c.c`: one scan over both channels with `INTEN` on the
//!   highest, `ADCE` cleared at scan end, and the repeat mode (`ADM1.RPS`)
//!   that follows the NTC as it warms until `ADCE` is cleared.
//!
//! The reference is 1.8 V (Zephyr r9a07g044.dtsi `vref-mv = <1800>`), so the
//! -10 °C divider (2.79 V) saturates and the 25 °C one (1.65 V) does not.
//! Also: the netlists moved to ADC_CH0 and ADC_CH7 convert there while
//! ADC_CH1 and ADC_CH4 go back to 0.

mod common;
use common::root;
use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::bus::SystemBus;
use labwired_core::cosim::{CosimSession, CosimSignalValue, RoutingError};
use labwired_core::Bus;
use std::path::Path;

const ADC: u64 = 0x1005_9000;
const ADM0: u64 = ADC;
const ADM1: u64 = ADC + 0x04;
const ADM2: u64 = ADC + 0x08;
const ADM3: u64 = ADC + 0x0C;
const ADINT: u64 = ADC + 0x20;
const ADSTS: u64 = ADC + 0x24;
const ADIVC: u64 = ADC + 0x28;
const ADCR0: u64 = ADC + 0x30;
const ADCE: u32 = 1 << 0;
const ADBSY: u32 = 1 << 1;
const PWDWNB: u32 = 1 << 2;
const SRESB: u32 = 1 << 15;
const TRG: u32 = 1 << 0;
const MS: u32 = 1 << 2;
const RPS: u32 = 1 << 3;
const BS: u32 = 1 << 4;
const EGA: u32 = 0b11 << 12;
const CSEEN: u32 = 1 << 16;
const INTS: u32 = 1 << 31;
const TRGS: u32 = 1 << 31;
/// The profile's `vref_mv`.
const VREF_MV: u32 = 1800;

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

/// 12-bit, (2^12 - 1) / VREF, truncated, clamped.
fn code(volts: f64) -> u32 {
    (millivolts(volts) * 4095 / VREF_MV).min(4095)
}

/// The onboarding RZ/G2L with the generic NTC and diode netlists routed to
/// the two pads.
fn board(ntc_pad: &str, diode_pad: &str) -> (SystemBus, CosimSession) {
    let system_path = root("configs/systems/onboarding/renesas_rz_g2l.yaml");
    let mut profile = SystemManifest::from_file(&system_path).expect("load RZ/G2L");
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
    let chip = ChipDescriptor::from_file(&chip_path).expect("load RZ/G2L descriptor");
    let bus = SystemBus::from_config(&chip, &profile).expect("build RZ/G2L bus");
    let session = CosimSession::new(&profile.cosim_models, Path::new("."), &bus)
        .expect("build analog co-simulation")
        .expect("fixture carries both analog devices");
    assert_eq!(
        session.binding_errors(),
        &[] as &[RoutingError],
        "{ntc_pad}/{diode_pad} must resolve to ADC_CHn through analog_pins"
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

fn adc_irq(bus: &SystemBus) -> bool {
    let adc = bus
        .peripherals
        .iter()
        .find(|p| p.name == "adc")
        .expect("adc on the bus");
    adc.dev
        .irq_line_level()
        .expect("renesas_adc_c reports its request")
}

fn adcr(bus: &SystemBus, channel: u32) -> u32 {
    bus.read_u32(ADCR0 + 4 * u64::from(channel)).unwrap() & 0xFFF
}

/// `rzg2l_adc_hw_init` + the runtime-PM resume: software reset released,
/// ADIVC divide-by-4, ADM3 = ADCMP 0xE | ADSMP 0x578, then power on.
fn linux_init(bus: &mut SystemBus) {
    let adm0 = bus.read_u32(ADM0).unwrap();
    bus.write_u32(ADM0, adm0 | SRESB).expect("SRESB");
    assert_ne!(bus.read_u32(ADM0).unwrap() & SRESB, 0, "reset released");
    bus.write_u32(ADIVC, 4).expect("ADIVC");
    bus.write_u32(ADM3, (0x0E << 16) | 0x578).expect("ADM3");
    let adm0 = bus.read_u32(ADM0).unwrap();
    bus.write_u32(ADM0, adm0 | PWDWNB).expect("PWDWNB");
}

/// `rzg2l_adc_conversion` for one channel and its scan-end ISR.
fn linux_read(bus: &mut SystemBus, channel: u32) -> u32 {
    assert_eq!(bus.read_u32(ADM0).unwrap() & ADBSY, 0, "not busy");
    let adm1 = bus.read_u32(ADM1).unwrap();
    bus.write_u32(ADM1, (adm1 & !(EGA | BS | TRG)) | MS)
        .expect("ADM1");
    let adm2 = bus.read_u32(ADM2).unwrap();
    bus.write_u32(ADM2, (adm2 & !0xFF) | (1 << channel))
        .expect("CHSEL");
    let adint = bus.read_u32(ADINT).unwrap();
    bus.write_u32(ADINT, (adint & !(INTS | 0xFF)) | CSEEN | (1 << channel))
        .expect("ADINT");
    let adm0 = bus.read_u32(ADM0).unwrap();
    bus.write_u32(ADM0, adm0 | ADCE).expect("ADCE");
    assert!(adc_irq(bus), "ADC_CH{channel}: scan end requests SPI 347");
    let sts = bus.read_u32(ADSTS).unwrap();
    assert_eq!(sts & 0xFF, 1 << channel, "INTST flags ADC_CH{channel}");
    let value = adcr(bus, channel);
    bus.write_u32(ADSTS, sts).expect("clear ADSTS");
    assert!(!adc_irq(bus), "writing ADSTS back drops the request");
    value
}

/// FSP `r_adc_c_open_sub` + `r_adc_c_scan_cfg`: software trigger, select
/// mode, `extra` (RPS for the repeat mode), `INTEN` on the highest channel.
fn fsp_open(bus: &mut SystemBus, scan_mask: u32, extra: u32) {
    bus.write_u32(ADM0, SRESB).expect("SRESB");
    bus.write_u32(ADSTS, TRGS | 0xFF).expect("clear ADSTS");
    bus.write_u32(ADM1, MS | extra).expect("ADM1");
    bus.write_u32(ADM3, (0x0E << 16) | 0x578).expect("ADM3");
    bus.write_u32(ADIVC, 4).expect("ADIVC");
    bus.write_u32(ADM0, SRESB | PWDWNB).expect("PWDWNB");
    bus.write_u32(ADM2, scan_mask).expect("ADM2");
    let highest = 31 - scan_mask.leading_zeros();
    bus.write_u32(ADINT, 1 << highest).expect("ADINT");
}

#[test]
fn rzg2l_adc_ch1_and_adc_ch4_convert_ntc_and_diode() {
    let (mut bus, mut session) = board("ADC_CH1", "ADC_CH4");
    assert_eq!(bus.read_u32(ADM0).unwrap(), 0, "ADM0 reset");
    linux_init(&mut bus);
    let mut cycles = 0;
    let mut ntc_codes = Vec::new();
    for temperature_c in [25.0_f64, 50.0, 80.0] {
        settle(&mut session, &mut bus, &mut cycles, temperature_c);
        let ntc_v = signal_voltage(&session, "board.analog.ADC_CH1_volts");
        assert!((ntc_v - ntc_volts(temperature_c)).abs() < 1e-9);
        let diode_v = signal_voltage(&session, "board.analog.ADC_CH4_volts");
        assert!(diode_v > 0.5 && diode_v < 0.8, "diode drop {diode_v} V");

        let ntc_code = linux_read(&mut bus, 1);
        assert_eq!(ntc_code, code(ntc_v), "ADC_CH1 at {temperature_c} °C");
        assert!(ntc_code < 4095, "below full scale at {temperature_c} °C");
        assert_eq!(linux_read(&mut bus, 4), code(diode_v), "ADC_CH4");
        for idle in [0, 5, 7] {
            assert_eq!(linux_read(&mut bus, idle), 0, "ADC_CH{idle} is not driven");
        }
        ntc_codes.push(ntc_code);
    }
    assert!(
        ntc_codes[0] > ntc_codes[1] && ntc_codes[1] > ntc_codes[2],
        "the NTC codes fall as it warms: {ntc_codes:?}"
    );
    // The IIO scale is vref / 4095: code * 1800 / 4095 mV.
    let mv = ntc_codes[0] * VREF_MV / 4095;
    assert!(
        mv.abs_diff(millivolts(ntc_volts(25.0))) <= 1,
        "code * VREF / 4095 gives {mv} mV"
    );
    // At -10 °C the divider (about 2.8 V) is above the 1.8 V reference.
    settle(&mut session, &mut bus, &mut cycles, -10.0);
    assert_eq!(linux_read(&mut bus, 1), 4095, "over-range clamps");
}

#[test]
fn rzg2l_fsp_single_and_repeat_scans_over_both_channels() {
    let (mut bus, mut session) = board("ADC_CH1", "ADC_CH4");
    let mut cycles = 0;
    settle(&mut session, &mut bus, &mut cycles, 25.0);
    let diode = code(signal_voltage(&session, "board.analog.ADC_CH4_volts"));
    let ntc_25 = code(signal_voltage(&session, "board.analog.ADC_CH1_volts"));

    // Single scan: R_ADC_C_ScanStart needs ADCE = 0, the scan clears it.
    fsp_open(&mut bus, (1 << 1) | (1 << 4), 0);
    assert_eq!(bus.read_u32(ADM0).unwrap() & ADCE, 0);
    bus.write_u32(ADM0, SRESB | PWDWNB | ADCE)
        .expect("ScanStart");
    assert_eq!(
        bus.read_u32(ADM0).unwrap() & (ADCE | ADBSY),
        0,
        "scan ended"
    );
    assert!(adc_irq(&bus), "INTEN on channel 4, the highest");
    assert_eq!(bus.read_u32(ADSTS).unwrap() & 0xFF, 0b1_0010);
    assert_eq!((adcr(&bus, 1), adcr(&bus, 4)), (ntc_25, diode));
    bus.write_u32(ADSTS, 0xFF)
        .expect("scan-end ISR clears INTST");
    assert!(!adc_irq(&bus));

    // Repeat scan: ADCE stays 1 and every tick converts again.
    fsp_open(&mut bus, (1 << 1) | (1 << 4), RPS);
    bus.write_u32(ADM0, SRESB | PWDWNB | ADCE)
        .expect("ScanStart");
    assert_ne!(bus.read_u32(ADM0).unwrap() & ADCE, 0, "repeat keeps ADCE");
    settle(&mut session, &mut bus, &mut cycles, 80.0);
    let ntc_80 = code(signal_voltage(&session, "board.analog.ADC_CH1_volts"));
    assert!(ntc_80 < ntc_25);
    bus.tick_peripherals();
    assert_eq!((adcr(&bus, 1), adcr(&bus, 4)), (ntc_80, diode));
    // R_ADC_C_ScanStop: ADCE = 0, wait for it; the results freeze.
    bus.write_u32(ADM0, SRESB | PWDWNB).expect("ScanStop");
    assert_eq!(bus.read_u32(ADM0).unwrap() & ADCE, 0);
    settle(&mut session, &mut bus, &mut cycles, 25.0);
    bus.tick_peripherals();
    assert_eq!(adcr(&bus, 1), ntc_80, "stopped scans do not update");
}

#[test]
fn rzg2l_adc_ch0_and_adc_ch7_convert_after_the_netlists_move() {
    let (mut bus, mut session) = board("ADC_CH0", "ADC_CH7");
    linux_init(&mut bus);
    let mut cycles = 0;
    settle(&mut session, &mut bus, &mut cycles, 80.0);
    let ntc_v = signal_voltage(&session, "board.analog.ADC_CH0_volts");
    let diode_v = signal_voltage(&session, "board.analog.ADC_CH7_volts");
    assert_eq!(linux_read(&mut bus, 0), code(ntc_v), "ADC_CH0");
    assert_eq!(linux_read(&mut bus, 7), code(diode_v), "ADC_CH7");
    assert_eq!(linux_read(&mut bus, 1), 0, "ADC_CH1 is not driven");
    assert_eq!(linux_read(&mut bus, 4), 0, "ADC_CH4 is not driven");
}
