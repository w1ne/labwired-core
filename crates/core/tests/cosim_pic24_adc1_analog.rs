// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Generic analog netlists reach the dsPIC33F ADC1 through the generic ADC
//! routes, and real XC16 firmware converts them.
//!
//! ADC1 is CPU-internal on the PIC24 core, so the routes land on the chip's
//! `pic24_adc_pads` window (`board.analog.<pad>_volts` through the descriptor's
//! `analog_pins`, or `adc.adc1.<n>_volts`), and the core pulls the levels when
//! they change. The firmware is the XC16 M3 program of each example: it
//! samples AN4 (RB2) once with the 10-bit converter and stores ADC1BUF0.
//!
//! Expected codes follow the ideal transfer function `Vin · 1024 / VREF`,
//! truncated (DS70183D §16.12); the route carries whole millivolts.

use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::bus::SystemBus;
use labwired_core::cosim::{CosimSession, CosimSignalValue, RoutingError};
use labwired_core::cpu::Pic24;
use labwired_core::peripherals::components::i2c_factory::build_external_i2c_device;
use labwired_core::peripherals::spi_loopback::SpiLoopback;
use labwired_core::system::node::parse_elf_image;
use labwired_core::{Cpu, SimulationConfig};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

const MARKER: u16 = 0x0E00;
const ADC_RESULT: u16 = 0x0E02;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// The board bus as the PIC24 session builder makes it, with `analog` as the
/// manifest's co-simulation models.
fn board_bus(board: &str, analog: &str) -> (SystemBus, SystemManifest) {
    let root = repo_root();
    let system_path = root.join(format!("configs/systems/{board}-demo.yaml"));
    let mut profile = SystemManifest::from_file(&system_path).expect("load the dsPIC profile");
    profile.cosim_models = serde_yaml::from_str(analog).expect("parse the analog models");
    let chip_path = system_path.parent().unwrap().join(&profile.chip);
    let chip = ChipDescriptor::from_file(&chip_path).expect("load the chip descriptor");
    let bus = SystemBus::from_config(&chip, &profile).expect("build the dsPIC bus");
    (bus, profile)
}

/// Runs the example's XC16 M3 firmware on `bus` and returns ADC1BUF0.
fn convert_an4_with_m3_firmware(board: &str, port_masks: [u16; 3], bus: &mut SystemBus) -> u16 {
    let root = repo_root();
    let example = root.join(format!("examples/{board}"));
    let system = SystemManifest::from_file(example.join("system-m3.yaml"))
        .expect("parse the M3 example system manifest");
    let elf = std::fs::read(example.join(format!("firmware/{board}-xc16-m3-adc-spi-i2c.elf")))
        .expect("read the XC16 M3 ELF");
    let image = parse_elf_image(&elf).expect("parse ELF");

    let mut cpu = Pic24::new();
    cpu.set_port_masks(port_masks);
    // As the session builder does for a chip that declares `pic24_adc_pads`.
    cpu.set_adc_pad_window(true);
    cpu.load_program_image(&image);
    // The M3 program also talks to a TMP102 and an SPI loopback before it
    // writes its marker; attach them as the session builder does.
    for device in &system.external_devices {
        match device.connection.as_str() {
            "i2c1" => cpu.attach_i2c_device(
                build_external_i2c_device(&device.r#type, &device.id, &device.config)
                    .expect("build the manifest's I2C device"),
            ),
            "spi1" => cpu.attach_spi_device(Box::new(SpiLoopback::new(""))),
            _ => {}
        }
    }
    let sink: Arc<Mutex<Vec<u8>>> = Arc::new(Mutex::new(Vec::new()));
    cpu.set_serial_sink(sink.clone());
    cpu.reset(bus).expect("reset");
    cpu.set_pc(0);
    // No harness code: only the routed pad level may reach the converter.
    for _ in 0..200_000 {
        cpu.step(bus, &[], &SimulationConfig::default())
            .expect("M3 firmware instruction must decode and execute");
        if cpu.data_read(MARKER).unwrap() == 0xBEEF {
            assert_eq!(sink.lock().unwrap().as_slice(), b"M3 OK\n");
            return cpu.data_read(ADC_RESULT).unwrap();
        }
    }
    panic!("the M3 firmware never reached its completion marker");
}

fn signal_voltage(session: &CosimSession, path: &str) -> f64 {
    match session.signals().get(path) {
        Some(CosimSignalValue::F64(volts)) => *volts,
        other => panic!("expected solved voltage at {path}, got {other:?}"),
    }
}

/// The 10-bit code for `volts` against `vref_v`, through the route's mV.
fn ideal_code(volts: f64, vref_v: f64) -> u16 {
    let mv = (volts * 1000.0).round() as u32;
    let vref_mv = (vref_v * 1000.0).round() as u32;
    (mv * 1024 / vref_mv).min(1023) as u16
}

fn ntc_resistance_ohms(temperature_c: f64) -> f64 {
    10_000.0 * (3950.0 * (1.0 / (temperature_c + 273.15) - 1.0 / 298.15)).exp()
}

fn diode_load_line_voltage(supply_v: f64) -> f64 {
    const LIMIT_OHMS: f64 = 1_000.0;
    const DIODE_IS_A: f64 = 2.52e-9;
    const DIODE_N: f64 = 1.752;
    const DIODE_RS_OHMS: f64 = 0.568;
    const THERMAL_V: f64 = 1.380_649e-23 * 298.15 / 1.602_176_634e-19;
    let current = |junction_v: f64| DIODE_IS_A * ((junction_v / (DIODE_N * THERMAL_V)).exp() - 1.0);
    let residual = |junction_v: f64| {
        let i = current(junction_v);
        (supply_v - junction_v - i * DIODE_RS_OHMS) / LIMIT_OHMS - i
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
    junction_v + current(junction_v) * DIODE_RS_OHMS
}

const NTC_ON_RB2: &str = r#"
- id: "ntc_divider"
  adapter: "analog"
  step_ns: 1000
  inputs:
    temperature: "ui.ntc.temperature_c"
  outputs:
    v_sense: "board.analog.RB2_volts"
  config:
    netlist_text: |
      Vcc vcc 0 dc 3.3
      Rpull vcc sense 10k
      Rntc sense 0 10k beta=3950 tnom=25 temp=25
    probes:
      v_sense: "v(sense)"
    sources:
      temperature: "Rntc"
    vdd: 3.3
"#;

#[test]
fn dspic33fj32mc202_xc16_adc1_converts_a_routed_ntc_divider() {
    let (mut bus, profile) = board_bus("dspic33fj32mc202", NTC_ON_RB2);
    let mut session = CosimSession::new(&profile.cosim_models, Path::new("."), &bus)
        .expect("build analog co-simulation")
        .expect("fixture carries an analog model");
    assert_eq!(
        session.binding_errors(),
        &[] as &[RoutingError],
        "RB2 is AN4 in the 202 descriptor's analog_pins"
    );

    let mut cycles = 0;
    let mut codes = Vec::new();
    for temperature_c in [-10.0_f64, 25.0, 80.0] {
        session
            .set_signal_number("ui.ntc.temperature_c", temperature_c)
            .unwrap();
        cycles += session.cycles_until_boundary(cycles);
        let (_, errors) = session
            .advance_to(cycles, &mut bus)
            .expect("solve and route");
        assert!(errors.is_empty(), "unexpected route errors: {errors:?}");

        let v = signal_voltage(&session, "board.analog.RB2_volts");
        let r = ntc_resistance_ohms(temperature_c);
        assert!(
            (v - 3.3 * r / (10_000.0 + r)).abs() < 1e-9,
            "NTC divider at {temperature_c} °C"
        );

        let code =
            convert_an4_with_m3_firmware("dspic33fj32mc202", [0x001F, 0xFFFF, 0x0000], &mut bus);
        assert_eq!(
            code,
            ideal_code(v, 3.3),
            "ADC1BUF0 at {temperature_c} °C ({v} V on RB2/AN4)"
        );
        codes.push(code);
    }
    // -10 °C = 2.816 V, 25 °C = 1.650 V, 80 °C = 0.372 V: the codes must track.
    assert_eq!(codes, [873, 512, 115]);
}

/// A second, unrelated device on the 44-pin part, through the explicit
/// `adc.adc1.<n>_volts` form, with the converter reference routed from the
/// circuit too (`board.power.vdda_volts`): a Shockley diode load line from
/// 3.0 V converts against a 3.0 V AVDD.
#[test]
fn dspic33fj32mc204_xc16_adc1_converts_a_diode_against_a_routed_reference() {
    const ANALOG: &str = r#"
- id: "diode_load_line"
  adapter: "analog"
  step_ns: 1000
  outputs:
    v_diode: "adc.adc1.4_volts"
    v_ref: "board.power.vdda_volts"
  config:
    netlist_text: |
      Vcc supply 0 dc 3.0
      Rlim supply sense 1k
      D1 sense 0 DREF TEMP=25
      .model DREF D(IS=2.52n N=1.752 RS=0.568 TNOM=25 EG=1.11 XTI=3)
    probes:
      v_diode: "v(sense)"
      v_ref: "v(supply)"
    vdd: 3.0
"#;
    let (mut bus, profile) = board_bus("dspic33fj32mc204", ANALOG);
    let mut session = CosimSession::new(&profile.cosim_models, Path::new("."), &bus)
        .expect("build analog co-simulation")
        .expect("fixture carries an analog model");
    assert_eq!(session.binding_errors(), &[] as &[RoutingError]);
    let cycles = session.cycles_until_boundary(0);
    let (_, errors) = session
        .advance_to(cycles, &mut bus)
        .expect("solve and route");
    assert!(errors.is_empty(), "unexpected route errors: {errors:?}");

    let v = signal_voltage(&session, "adc.adc1.4_volts");
    let expected_v = diode_load_line_voltage(3.0);
    assert!(
        (v - expected_v).abs() < 2e-5,
        "diode load line {expected_v} V, got {v} V"
    );
    let code = convert_an4_with_m3_firmware("dspic33fj32mc204", [0x079F, 0xFFFF, 0x03FF], &mut bus);
    assert_eq!(code, ideal_code(v, 3.0), "ADC1BUF0 for {v} V against 3.0 V");
    assert_ne!(
        code,
        ideal_code(v, 3.3),
        "the routed reference must change the code"
    );
}

/// Routes the package does not bond out are refused when the session is
/// built: the 28-pin 202 has AN0-AN5 only (no PORTC, so no RC0/AN6).
#[test]
fn dspic33fj32mc202_refuses_adc_routes_outside_its_package() {
    const ANALOG: &str = r#"
- id: "divider"
  adapter: "analog"
  step_ns: 1000
  outputs:
    by_pad: "board.analog.RC0_volts"
    by_channel: "adc.adc1.6_volts"
  config:
    netlist_text: |
      V1 a 0 dc 1.0
      R1 a 0 1k
    probes:
      by_pad: "v(a)"
      by_channel: "v(a)"
    vdd: 3.3
"#;
    let (bus, profile) = board_bus("dspic33fj32mc202", ANALOG);
    let session = CosimSession::new(&profile.cosim_models, Path::new("."), &bus)
        .expect("build analog co-simulation")
        .expect("fixture carries an analog model");
    let errors = session.binding_errors();
    assert!(
        errors
            .iter()
            .any(|e| matches!(e, RoutingError::NoAdcChannel { pad, .. } if pad == "RC0")),
        "RC0 is not an analog pin of the 202: {errors:?}"
    );
    assert!(
        errors.iter().any(|e| matches!(
            e,
            RoutingError::NoSuchAdcChannel {
                channel: 6,
                channels: 6,
                ..
            }
        )),
        "AN6 is outside the 202's AN0-AN5: {errors:?}"
    );
}
