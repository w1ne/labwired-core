// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! The generic NTC divider and diode load line reach the STM32W108's ADC
//! (`stm32w108_adc`) through the onboarding descriptor's `analog_pins` map
//! (DocID16252 Rev 16 Table 37: ADC0..5 = PB5, PB6, PB7, PC1, PA4, PA5): the
//! NTC on PB7 (ADC2) and the diode on PC1 (ADC3), read the way the ST HAL
//! (Contiki `cpu/stm32w108/hal/micro/cortexm3/adc.c`) does:
//! - `halInternalInitAdc`: `ADC_CFG = 0`, reset OFFSET/GAIN, `ADC_DMACFG =
//!   ADC_DMARST`, `ADC_DMABEG = &adcData`, `ADC_DMASIZE = 1`, `AUTOWRAP |
//!   LOAD`, `INT_ADCCFG = INT_ADCULDFULL`, `INT_ADCFLAG = 0xFFFF`;
//! - a conversion: `ADC_CFG = SMP | MUXP | MUXN | ADC_1MHZCLK | ADC_ENABLE`,
//!   `INT_ADCFLAG = 0xFFFF`, the ISR fires on `INT_ADCULDFULL` (DMABF),
//!   takes `adcData`, `ADC_CFG = 0`;
//! - `halAdcCalibrate` + `halConvertValueToVolts`: GND and VREG/2 (0.9 V)
//!   against VREF/2 in the low range, VREF and VREF/2 against VREF/2 in the
//!   high range (input buffers, gain 0.25), then the HAL's own fixed-point
//!   formulas give volts in 100 µV units.
//!
//! `INT_ADCFLAG` (0x4000A810) and `INT_ADCCFG` (0x4000A850) are reached at
//! their real addresses, through the descriptor's mirrors.

mod common;
use common::root;
use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::bus::SystemBus;
use labwired_core::cosim::{CosimSession, CosimSignalValue, RoutingError};
use labwired_core::Bus;
use std::path::Path;

const ADC: u64 = 0x4000_D000;
const ADC_CFG: u64 = ADC + 0x04;
const ADC_OFFSET: u64 = ADC + 0x08;
const ADC_GAIN: u64 = ADC + 0x0C;
const ADC_DMACFG: u64 = ADC + 0x10;
const ADC_DMASTAT: u64 = ADC + 0x14;
const ADC_DMABEG: u64 = ADC + 0x18;
const ADC_DMASIZE: u64 = ADC + 0x1C;
const INT_ADCFLAG: u64 = 0x4000_A810;
const INT_ADCCFG: u64 = 0x4000_A850;

const ADC_DMARST: u32 = 1 << 4;
const ADC_DMAAUTOWRAP: u32 = 1 << 1;
const ADC_DMALOAD: u32 = 1 << 0;
const INT_ADCULDFULL: u32 = 1 << 2;
const ADC_HVSELP: u32 = 1 << 12;
const ADC_HVSELN: u32 = 1 << 11;
/// `halInternalInitAdc`: `adcStaticConfig = ADC_1MHZCLK | ADC_ENABLE`.
const STATIC_CONFIG: u32 = (1 << 2) | 1;
/// `ADC_CONVERSION_TIME_US_4096`: SMP 7, 12 significant bits.
const RATE_4096: u32 = 7 << 13;

const MUX_GND: u32 = 0x8;
const MUX_VREF2: u32 = 0x9;
const MUX_VREF: u32 = 0xA;
const MUX_VREG2: u32 = 0xB;

/// `static uint16_t adcData`, the one-sample DMA buffer.
const ADC_DATA_RAM: u64 = 0x2000_0400;

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

/// The onboarding STM32W108 with the generic NTC and diode netlists routed to
/// the two pads.
fn board(ntc_pad: &str, diode_pad: &str) -> (SystemBus, CosimSession) {
    let system_path = root("configs/systems/onboarding/stm32w108.yaml");
    let mut profile = SystemManifest::from_file(&system_path).expect("load STM32W108");
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
    let chip = ChipDescriptor::from_file(&chip_path).expect("load STM32W108 descriptor");
    let bus = SystemBus::from_config(&chip, &profile).expect("build STM32W108 bus");
    let session = CosimSession::new(&profile.cosim_models, Path::new("."), &bus)
        .expect("build analog co-simulation")
        .expect("fixture carries both analog devices");
    assert_eq!(
        session.binding_errors(),
        &[] as &[RoutingError],
        "{ntc_pad}/{diode_pad} must resolve to ADC pads through analog_pins"
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
        .expect("stm32w108_adc reports its request")
}

/// `halInternalInitAdc` without `stCalibrateVref`.
fn hal_init(bus: &mut SystemBus) {
    bus.write_u32(ADC_CFG, 0).expect("ADC off");
    bus.write_u32(ADC_OFFSET, 0).expect("ADC_OFFSET_RESET");
    bus.write_u32(ADC_GAIN, 0x8000).expect("ADC_GAIN_RESET");
    bus.write_u32(ADC_DMACFG, ADC_DMARST).expect("ADC_DMARST");
    bus.write_u32(ADC_DMABEG, ADC_DATA_RAM as u32)
        .expect("ADC_DMABEG = &adcData");
    bus.write_u32(ADC_DMASIZE, 1).expect("ADC_DMASIZE = 1");
    bus.write_u32(ADC_DMACFG, ADC_DMAAUTOWRAP | ADC_DMALOAD)
        .expect("AUTOWRAP | LOAD");
    bus.write_u32(INT_ADCCFG, INT_ADCULDFULL)
        .expect("INT_ADCCFG = INT_ADCULDFULL");
    bus.write_u32(INT_ADCFLAG, 0xFFFF)
        .expect("clear INT_ADCFLAG");
    assert_eq!(bus.read_u32(ADC_DMASTAT).unwrap(), 1, "ADC_DMAACT");
}

/// `startNextConversion` + `halAdcIsr` + `halReadAdcBlocking` for one
/// `ADC_SOURCE_<p>_<n>` at `ADC_CONVERSION_TIME_US_4096`.
fn hal_read(bus: &mut SystemBus, muxp: u32, muxn: u32, range_high: bool) -> u16 {
    let range = if range_high {
        ADC_HVSELP | ADC_HVSELN
    } else {
        0
    };
    bus.write_u32(
        ADC_CFG,
        RATE_4096 | (muxp << 7) | (muxn << 3) | range | STATIC_CONFIG,
    )
    .expect("ADC_CFG = adcConfig[id]");
    bus.write_u32(INT_ADCFLAG, 0xFFFF)
        .expect("INT_ADCFLAG = 0xFFFF");
    let mut ticks = 0;
    while !adc_irq(bus) {
        bus.tick_peripherals();
        ticks += 1;
        assert!(ticks < 16, "INT_ADCULDFULL for MUXP {muxp} MUXN {muxn}");
    }
    assert_ne!(bus.read_u32(INT_ADCFLAG).unwrap() & INT_ADCULDFULL, 0);
    let value = bus.read_u16(ADC_DATA_RAM).unwrap();
    bus.write_u32(ADC_CFG, 0)
        .expect("no request left: ADC_CFG = 0");
    bus.write_u32(INT_ADCFLAG, 0xFFFF)
        .expect("INT_ADCFLAG = 0xFFFF");
    assert!(!adc_irq(bus), "clearing INT_ADCFLAG drops the request");
    value
}

/// `halAdcCalibrate` + `halConvertValueToVolts`, low range: volts in
/// 100 µV units from the ratio to VREG/2 (0.9 V).
fn hal_volts_low(bus: &mut SystemBus, muxp: u32) -> i32 {
    let nvss = hal_read(bus, MUX_GND, MUX_VREF2, false) as i16 as i32;
    let nvdd = hal_read(bus, MUX_VREG2, MUX_VREF2, false) as i16 as i32 - nvss;
    // The HAL takes `uint16_t value`, so an input below VREF/2 would wrap
    // there; the datasheet defines the result as two's complement (Table 41
    // notes), so it is read as int16_t here.
    let value = hal_read(bus, muxp, MUX_VREF2, false) as i16 as i32;
    let nvalue = value - nvss;
    let n = ((nvalue << 16) + nvdd / 2) / nvdd;
    (((n as i64 * 9000) >> 16) as i32).min(12000)
}

/// The HAL's high-range branch (`ENABLE_ADC_EXTENDED_RANGE_BROKEN`):
/// calibrated against VREF and VREF/2 through the input buffers.
fn hal_volts_high(bus: &mut SystemBus, muxp: u32) -> i32 {
    let nvref = hal_read(bus, MUX_VREF, MUX_VREF2, true) as i16 as i32;
    let nvref2 = hal_read(bus, MUX_VREF2, MUX_VREF2, true) as i16 as i32;
    let value = hal_read(bus, muxp, MUX_VREF2, true) as i16 as i32;
    let n = ((value + nvref - 2 * nvref2) << 16) / (2 * (nvref - nvref2));
    (((n as i64 * 12000) >> 16) as i32).min(21000)
}

fn tenths_of_mv(volts: f64) -> i32 {
    (volts * 10_000.0).round() as i32
}

#[test]
fn stm32w108_pb7_and_pc1_convert_ntc_and_diode_on_adc2_and_adc3() {
    let (mut bus, mut session) = board("PB7", "PC1");
    assert_eq!(bus.read_u32(ADC_CFG).unwrap(), 0x1800, "ADC_CR reset");
    hal_init(&mut bus);
    let mut cycles = 0;
    let mut ntc_mv = Vec::new();
    for temperature_c in [-10.0_f64, 25.0, 80.0] {
        settle(&mut session, &mut bus, &mut cycles, temperature_c);
        let ntc_v = signal_voltage(&session, "board.analog.PB7_volts");
        assert!((ntc_v - ntc_volts(temperature_c)).abs() < 1e-9);
        let diode_v = signal_voltage(&session, "board.analog.PC1_volts");
        assert!(diode_v > 0.5 && diode_v < 0.8, "diode drop {diode_v} V");

        // The 3.3 V divider needs the input buffers (0 to VDD_PADS); the
        // 12-bit LSB through the 0.25 gain is about 2.3 mV.
        // `halConvertValueToVolts` limits the high range to 21000 (2.1 V),
        // so the -10 °C divider (2.82 V) reads as 2.1 V.
        let ntc = hal_volts_high(&mut bus, 2);
        assert!(
            ntc.abs_diff(tenths_of_mv(ntc_v).min(21000)) <= 30,
            "ADC2 at {temperature_c} °C: HAL gives {ntc} x 100 µV for {ntc_v} V"
        );
        // The diode stays in the 1.2 V low range, LSB about 0.6 mV.
        let diode = hal_volts_low(&mut bus, 3);
        assert!(
            diode.abs_diff(tenths_of_mv(diode_v)) <= 12,
            "ADC3: HAL gives {diode} x 100 µV for {diode_v} V"
        );
        // Undriven pads read as GND: the HAL scales them to 0 V.
        for idle in [0, 1, 4, 5] {
            assert!(
                hal_volts_low(&mut bus, idle).abs() <= 1,
                "ADC{idle} is not driven"
            );
        }
        ntc_mv.push(ntc);
    }
    assert!(
        ntc_mv[0] > ntc_mv[1] && ntc_mv[1] > ntc_mv[2],
        "the NTC volts fall as it warms: {ntc_mv:?}"
    );
    // At 80 °C (0.37 V) the divider is inside the low range as well, and
    // the finer LSB agrees with the buffered reading.
    let low = hal_volts_low(&mut bus, 2);
    assert!(low.abs_diff(tenths_of_mv(ntc_volts(80.0))) <= 12);
    assert!(low.abs_diff(ntc_mv[2]) <= 30);
}

#[test]
fn stm32w108_raw_codes_follow_table_41_and_the_low_range_saturates() {
    let (mut bus, mut session) = board("PB7", "PC1");
    let mut cycles = 0;
    settle(&mut session, &mut bus, &mut cycles, 25.0);
    hal_init(&mut bus);
    // DocID16252 Table 41: GND ≈ 0xE000, VREF ≈ 0x2000 against VREF/2;
    // VREF through the P buffer alone ≈ 0xF000.
    assert_eq!(hal_read(&mut bus, MUX_GND, MUX_VREF2, false), 0xE000);
    assert_eq!(hal_read(&mut bus, MUX_VREF, MUX_VREF2, false), 0x2000);
    bus.write_u32(
        ADC_CFG,
        RATE_4096 | (MUX_VREF << 7) | (MUX_VREF2 << 3) | ADC_HVSELP | STATIC_CONFIG,
    )
    .unwrap();
    bus.write_u32(INT_ADCFLAG, 0xFFFF).unwrap();
    bus.tick_peripherals();
    assert_eq!(bus.read_u16(ADC_DATA_RAM).unwrap(), 0xF000);
    bus.write_u32(ADC_CFG, 0).unwrap();
    // The diode against VREF/2: 16384 * (V - 0.6) / 1.2, 12 bits.
    let diode_v = signal_voltage(&session, "board.analog.PC1_volts");
    let mv = (diode_v * 1000.0).round() as i64;
    let raw = ((32768 * (mv - 600)).div_euclid(1200)) & !0xF;
    assert_eq!(
        hal_read(&mut bus, 3, MUX_VREF2, false) as i16 as i64,
        raw >> 1,
        "ADC3 raw code for {diode_v} V"
    );
    // The 1.65 V divider (25 °C) against GND exceeds VREF and saturates.
    assert_eq!(hal_read(&mut bus, 2, MUX_GND, false), 0x3FF8);
}

#[test]
fn stm32w108_pa4_and_pa5_convert_after_the_netlists_move() {
    let (mut bus, mut session) = board("PA4", "PA5");
    let mut cycles = 0;
    settle(&mut session, &mut bus, &mut cycles, 80.0);
    hal_init(&mut bus);
    let ntc_v = signal_voltage(&session, "board.analog.PA4_volts");
    let diode_v = signal_voltage(&session, "board.analog.PA5_volts");
    assert!(
        hal_volts_low(&mut bus, 4).abs_diff(tenths_of_mv(ntc_v)) <= 12,
        "ADC4"
    );
    assert!(
        hal_volts_low(&mut bus, 5).abs_diff(tenths_of_mv(diode_v)) <= 12,
        "ADC5"
    );
    assert!(hal_volts_low(&mut bus, 2).abs() <= 1, "PB7 is not driven");
    assert!(hal_volts_low(&mut bus, 3).abs() <= 1, "PC1 is not driven");
}
