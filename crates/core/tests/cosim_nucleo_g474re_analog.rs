// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Generic MNA devices drive the NUCLEO-G474RE model's PA0/PA1 pads, first as
//! GPIO inputs (firmware-visible threshold behaviour; the threshold ratios
//! are test assumptions), then as analog levels that resolve through the
//! chip's `analog_pins` to ADC1_IN1/IN2 and convert through ADC1's regular
//! sequence and injected group. Silicon behaviour is not claimed.

mod common;
use common::root;
use labwired_config::{ChipDescriptor, GpioInputThresholds, SystemManifest};
use labwired_core::bus::SystemBus;
use labwired_core::cosim::{CosimSession, CosimSignalValue, RoutingError};
use labwired_core::peripherals::adc::Adc;
use labwired_core::Bus;
use std::path::Path;

const RCC_AHB2ENR: u64 = 0x4002_104C;
const GPIOA_BASE: u64 = 0x4800_0000;
const GPIOA_MODER: u64 = GPIOA_BASE;
const GPIOA_IDR: u64 = GPIOA_BASE + 0x10;
const ADC1_BASE: u64 = 0x5000_0000;
const ADC_ISR: u64 = ADC1_BASE;
const ADC_CR: u64 = ADC1_BASE + 0x08;
const ADC_SQR1: u64 = ADC1_BASE + 0x30;
const ADC_DR: u64 = ADC1_BASE + 0x40;
const ADC_JSQR: u64 = ADC1_BASE + 0x4C;
const ADC_JDR1: u64 = ADC1_BASE + 0x80;
const ADC_ON: u32 = (1 << 28) | 1; // ADVREGEN | ADEN

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

fn signal_voltage(session: &CosimSession, path: &str) -> f64 {
    match session.signals().get(path) {
        Some(CosimSignalValue::F64(volts)) => *volts,
        other => panic!("expected solved voltage at {path}, got {other:?}"),
    }
}

#[test]
fn nucleo_g474re_ntc_and_diode_netlists_reach_pa0_pa1_gpio_inputs() {
    let system_path = root("configs/systems/nucleo_g474re.yaml");
    let mut profile = SystemManifest::from_file(&system_path).expect("load NUCLEO-G474RE profile");
    assert_eq!(profile.name, "nucleo_g474re");
    assert!(profile.external_devices.is_empty());
    assert!(profile
        .board_io
        .iter()
        .any(|io| io.id == "led_user" && io.peripheral == "gpioa" && io.pin == 5));
    assert!(profile
        .board_io
        .iter()
        .any(|io| io.id == "button_b1" && io.peripheral == "gpioc" && io.pin == 13));

    let analog =
        SystemManifest::from_file(root("examples/nucleo-g474re/analog-ntc-diode-system.yaml"))
            .expect("load generic NTC divider and Shockley diode netlists");
    profile.cosim_models = analog.cosim_models;

    let chip_path = system_path.parent().unwrap().join(&profile.chip);
    let mut chip = ChipDescriptor::from_file(&chip_path).expect("load STM32G474 descriptor");
    // Simulation assumptions only: no G474 threshold values are claimed here.
    chip.io_voltage_v = Some(3.3);
    chip.gpio_input_thresholds = Some(GpioInputThresholds { vil: 0.3, vih: 0.7 });

    let mut bus = SystemBus::from_config(&chip, &profile).expect("build NUCLEO-G474RE bus");
    assert_eq!(
        SystemBus::resolve_pin_idr_pub(&bus, "PA0"),
        Some((GPIOA_IDR, 0)),
        "PA0 must resolve to the modeled GPIOA IDR bit 0"
    );
    assert_eq!(
        SystemBus::resolve_pin_idr_pub(&bus, "PA1"),
        Some((GPIOA_IDR, 1)),
        "PA1 must resolve to the modeled GPIOA IDR bit 1"
    );

    bus.write_u32(RCC_AHB2ENR, 1).expect("enable GPIOA clock");
    let moder = bus.read_u32(GPIOA_MODER).expect("read GPIOA mode");
    bus.write_u32(GPIOA_MODER, moder & !0xF)
        .expect("configure PA0/PA1 as digital inputs");

    let mut session = CosimSession::new(&profile.cosim_models, Path::new("."), &bus)
        .expect("build analog co-simulation")
        .expect("fixture carries both analog models");
    assert_eq!(
        session.binding_errors(),
        &[] as &[RoutingError],
        "generic devices must resolve to separate modeled PA0/PA1 GPIO inputs"
    );

    let expected_diode_v = diode_load_line_voltage();
    let vil_v = 0.3 * 3.3;
    assert!(expected_diode_v < vil_v, "diode must fall below test VIL");
    let mut cycles = 0;
    for (temperature_c, expected_ntc_level) in [
        (-10.0_f64, true),
        (25.0, true), // hold high while the input sits inside the assumed band
        (80.0, false),
        (25.0, false), // hold low while it returns to the assumed band
        (-10.0, true),
    ] {
        session
            .set_signal_number("ui.ntc.temperature_c", temperature_c)
            .expect("set NTC temperature");
        cycles += session.cycles_until_boundary(cycles);
        let (steps, errors) = session
            .advance_to(cycles, &mut bus)
            .expect("solve and route both analog netlists");
        assert_eq!(
            steps.len(),
            2,
            "both netlists cross the co-simulation boundary"
        );
        assert!(errors.is_empty(), "unexpected route errors: {errors:?}");

        let ntc_v = signal_voltage(&session, "board.gpio_in.pa0");
        let resistance = ntc_resistance_ohms(temperature_c);
        let expected_ntc_v = 3.3 * resistance / (10_000.0 + resistance);
        assert!(
            (ntc_v - expected_ntc_v).abs() < 1e-9,
            "at {temperature_c} °C NTC divider should be {expected_ntc_v} V, got {ntc_v} V"
        );

        let diode_v = signal_voltage(&session, "board.gpio_in.pa1");
        assert!(
            (diode_v - expected_diode_v).abs() < 2e-5,
            "Shockley load line should be {expected_diode_v} V, got {diode_v} V"
        );

        let gpioa_idr = bus.read_u32(GPIOA_IDR).expect("read GPIOA input data");
        assert_eq!(
            gpioa_idr & 1 != 0,
            expected_ntc_level,
            "PA0 must follow the solved NTC voltage and retain threshold state"
        );
        assert_eq!(gpioa_idr & 2, 0, "PA1 must read low below the assumed VIL");
    }
}

fn adc_channel_count(bus: &mut SystemBus, channel: u8) -> u16 {
    let index = bus
        .find_peripheral_index_by_name("adc1")
        .expect("G474 profile declares ADC1");
    bus.peripherals[index]
        .dev
        .as_any_mut()
        .expect("ADC1 downcasts")
        .downcast_mut::<Adc>()
        .expect("ADC1 uses the shared STM32 ADC model")
        .channel_input_count(channel)
}

fn expected_adc12_count(voltage_v: f64) -> u32 {
    let millivolts = (voltage_v * 1000.0).round() as u32;
    (millivolts * 4095 / 3300).min(4095)
}

/// One software-started regular sequence (`SQR1.L` = channels - 1), read by
/// polling `EOC` and `DR`.
fn g4_adc_scan(bus: &mut SystemBus, channels: &[u8]) -> Vec<u32> {
    const EOC: u32 = 1 << 2;
    const EOS: u32 = 1 << 3;
    let sqr1 = channels
        .iter()
        .enumerate()
        .fold(channels.len() as u32 - 1, |w, (k, &ch)| {
            w | (u32::from(ch) << (6 * (k + 1)))
        });
    bus.write_u32(ADC_SQR1, sqr1).expect("SQR1");
    bus.write_u32(ADC_ISR, EOC | EOS).expect("clear EOC/EOS");
    bus.write_u32(ADC_CR, ADC_ON | (1 << 2)).expect("ADSTART");
    channels
        .iter()
        .enumerate()
        .map(|(k, _)| {
            let isr = bus.read_u32(ADC_ISR).unwrap();
            assert_ne!(isr & EOC, 0, "EOC for conversion {k}");
            assert_eq!(isr & EOS != 0, k + 1 == channels.len(), "EOS at {k}");
            bus.write_u32(ADC_ISR, EOC).expect("clear EOC");
            bus.read_u32(ADC_DR).expect("DR")
        })
        .collect()
}

/// One `JADSTART` over `channels` (JSQ1 first) in the G4 `JSQR` packing:
/// `JL[1:0]`, `JEXTSEL[6:2]`, `JEXTEN[8:7]`, `JSQ1` at bit 9 and six bits per
/// rank (RM0440, `stm32g474.svd`). Returns `JDR1..JDRn`.
fn g4_adc_injected(bus: &mut SystemBus, channels: &[u8]) -> Vec<u32> {
    const JEOC_JEOS: u32 = (1 << 5) | (1 << 6);
    let jsqr = channels
        .iter()
        .enumerate()
        .fold(channels.len() as u32 - 1, |w, (k, &ch)| {
            w | (u32::from(ch) << (9 + 6 * k))
        });
    bus.write_u32(ADC_JSQR, jsqr).expect("JSQR");
    bus.write_u32(ADC_ISR, JEOC_JEOS).expect("clear JEOC/JEOS");
    bus.write_u32(ADC_CR, ADC_ON | (1 << 3)).expect("JADSTART");
    assert_eq!(bus.read_u32(ADC_ISR).unwrap() & JEOC_JEOS, JEOC_JEOS);
    assert_eq!(
        bus.read_u32(ADC_CR).unwrap() & (1 << 3),
        0,
        "JADSTART clears"
    );
    (0..channels.len() as u64)
        .map(|k| bus.read_u32(ADC_JDR1 + 4 * k).expect("JDRx"))
        .collect()
}

#[test]
fn nucleo_g474re_ntc_and_diode_netlists_reach_pa0_pa1_adc1_end_to_end() {
    let system_path = root("configs/systems/nucleo_g474re.yaml");
    let mut profile = SystemManifest::from_file(&system_path).expect("load NUCLEO-G474RE profile");
    let analog = SystemManifest::from_file(root(
        "examples/nucleo-g474re/analog-ntc-diode-adc-system.yaml",
    ))
    .expect("load generic NTC divider and Shockley diode netlists");
    profile.cosim_models = analog.cosim_models;
    let chip = ChipDescriptor::from_file(system_path.parent().unwrap().join(&profile.chip))
        .expect("load STM32G474 descriptor");
    for (pad, channel) in [("PA0", 1), ("PA1", 2)] {
        assert_eq!(
            chip.analog_pins
                .get(pad)
                .map(|route| (route.peripheral.as_str(), route.channel)),
            Some(("adc1", channel)),
            "{pad} is ADC1_IN{channel} (ST open pin data)"
        );
    }
    let mut bus = SystemBus::from_config(&chip, &profile).expect("build NUCLEO-G474RE bus");

    bus.write_u32(RCC_AHB2ENR, 1 << 13)
        .expect("enable ADC12 clock");
    bus.write_u32(ADC_CR, 0).expect("leave ADC deep power-down");
    bus.write_u32(ADC_CR, 1 << 28)
        .expect("enable ADC voltage regulator");
    bus.write_u32(ADC_CR, ADC_ON).expect("enable ADC1");
    assert_ne!(bus.read_u32(ADC_ISR).unwrap() & 1, 0, "ADC1 reports ADRDY");

    let mut session = CosimSession::new(&profile.cosim_models, Path::new("."), &bus)
        .expect("build analog co-simulation")
        .expect("fixture carries both generic analog netlists");
    assert_eq!(
        session.binding_errors(),
        &[] as &[RoutingError],
        "generic models must resolve PA0/PA1 to ADC1 channels 1/2"
    );
    assert_eq!(adc_channel_count(&mut bus, 3), u16::MAX, "IN3 is undriven");

    let expected_diode_v = diode_load_line_voltage();
    let mut cycles = 0;
    let mut ntc_codes = Vec::new();
    for temperature_c in [-10.0_f64, 25.0, 80.0] {
        session
            .set_signal_number("ui.ntc.temperature_c", temperature_c)
            .expect("set generic NTC temperature");
        cycles += session.cycles_until_boundary(cycles);
        let (steps, errors) = session
            .advance_to(cycles, &mut bus)
            .expect("solve both netlists and route their ADC inputs");
        assert_eq!(steps.len(), 2, "both netlists cross the 1 us boundary");
        assert!(errors.is_empty(), "unexpected route errors: {errors:?}");

        let ntc_v = signal_voltage(&session, "board.analog.pa0_volts");
        let resistance = ntc_resistance_ohms(temperature_c);
        let expected_ntc_v = 3.3 * resistance / (10_000.0 + resistance);
        assert!(
            (ntc_v - expected_ntc_v).abs() < 1e-9,
            "at {temperature_c} °C NTC divider should be {expected_ntc_v} V, got {ntc_v} V"
        );
        let diode_v = signal_voltage(&session, "board.analog.pa1_volts");
        assert!(
            (diode_v - expected_diode_v).abs() < 2e-5,
            "Shockley load line should be {expected_diode_v} V, got {diode_v} V"
        );

        let ntc = expected_adc12_count(ntc_v);
        let diode = expected_adc12_count(diode_v);
        assert_eq!(adc_channel_count(&mut bus, 1), ntc as u16, "PA0 seeds IN1");
        assert_eq!(
            adc_channel_count(&mut bus, 2),
            diode as u16,
            "PA1 seeds IN2"
        );
        assert_eq!(g4_adc_scan(&mut bus, &[1]), vec![ntc], "SQ1 = IN1");
        assert_eq!(g4_adc_scan(&mut bus, &[2]), vec![diode], "SQ1 = IN2");
        assert_eq!(
            g4_adc_scan(&mut bus, &[2, 1, 2]),
            vec![diode, ntc, diode],
            "one ADSTART scans the routed sensors in SQx order"
        );
        let dr = bus.read_u32(ADC_DR).unwrap();
        assert_eq!(
            g4_adc_injected(&mut bus, &[1, 2, 1, 2]),
            vec![ntc, diode, ntc, diode],
            "one JADSTART converts JSQ1..JSQ4 into JDR1..JDR4"
        );
        assert_eq!(bus.read_u32(ADC_DR).unwrap(), dr, "injected leaves DR");
        ntc_codes.push(ntc);
    }
    assert!(
        ntc_codes[0] > ntc_codes[1] && ntc_codes[1] > ntc_codes[2],
        "the NTC code falls as temperature rises: {ntc_codes:?}"
    );
}
