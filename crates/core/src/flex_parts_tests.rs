// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Flex runs generated part models (inline `parts:` packs), the catalog
//! LCD1602 and DS18B20 descriptors, and listen-only sinks (WS2812, TM1637).
//! Every test drives the runtime the way portable firmware does: port
//! writes, bus transfers and `advance_ns`, with no CPU.

use crate::flex::FlexRuntime;
use serde_json::Value;

fn state(runtime: &FlexRuntime, id: &str) -> Value {
    runtime
        .device_artifacts()
        .into_iter()
        .find(|artifact| artifact.id == id)
        .unwrap_or_else(|| panic!("no device state for '{id}'"))
        .meta
}

// ── 1. inline part packs ──────────────────────────────────────────────────

const PACKS: &str = r#"
schema_version: 1
name: generated parts
ports:
  - { name: relay_in, signal: mcu.d5, access: write, kind: bool, initial: 1 }
  - { name: coil, signal: relay.coil, access: read, kind: bool }
  - { name: ambient_c, signal: env.t, access: write, kind: number, unit: "°C", initial: 20 }
  - { name: sensor_mv, signal: sensor.out, access: read, kind: number, unit: mV }
parts:
  - schema: labwired.part/v1
    type: "gen:relay-module"
    source: generated from catalog facts
    behavior:
      primitive: gpio_device
      pins: { IN: in_pin }
      outputs: [COIL]
      pin_defaults: { COIL: false }
      rules:
        - on: { pins: [IN] }
          do: [{ pin: COIL, level: "!pin(IN)" }]
  - schema: labwired.part/v1
    type: "gen:linear-temp"
    source: generated from a datasheet transfer curve
    behavior:
      primitive: analog_source
      analog:
        formula: "400 + 19.5 * temperature"
        encode: trunc
    metadata:
      inputs:
        - { key: temperature, label: Temperature, unit: "°C", min: -40, max: 125, default: 25 }
  - schema: labwired.part/v1
    type: "gen:id-chip"
    source: generated from a register map
    behavior:
      primitive: i2c_device
      i2c:
        default_address: 0x42
        registers:
          - { name: WHO_AM_I, addr: 0x0F, width: 1, endian: be, access: r, reset: 0xA5 }
          - { name: SCRATCH, addr: 0x10, width: 1, endian: be, access: rw, reset: 0x00 }
cosim_models:
  - id: relay
    adapter: behavior
    step_ns: 1000
    inputs: { IN: mcu.d5 }
    outputs: { COIL: relay.coil }
    config: { device_type: "gen:relay-module" }
  - id: sensor
    adapter: behavior
    step_ns: 1000
    inputs: { temperature: env.t }
    outputs: { out: sensor.out }
    config: { device_type: "gen:linear-temp" }
i2c_buses:
  - name: i2c0
    frequency_hz: 100000
    devices:
      - { id: chip, type: "gen:id-chip" }
"#;

#[test]
fn inline_part_packs_run_as_models_and_bus_devices() {
    let mut runtime = FlexRuntime::from_yaml(PACKS).unwrap();
    let relay_in = runtime.port_id("relay_in").unwrap();
    let coil = runtime.port_id("coil").unwrap();
    // The generated relay module is active-low.
    runtime.write(relay_in, 0.0).unwrap();
    runtime.advance_ns(3_000).unwrap();
    assert_eq!(runtime.read(coil).unwrap(), 1.0);
    runtime.write(relay_in, 1.0).unwrap();
    runtime.advance_ns(3_000).unwrap();
    assert_eq!(runtime.read(coil).unwrap(), 0.0);

    // The generated analog curve follows its environment input.
    let ambient = runtime.port_id("ambient_c").unwrap();
    let sensor = runtime.port_id("sensor_mv").unwrap();
    runtime.write(ambient, 40.0).unwrap();
    runtime.advance_ns(3_000).unwrap();
    assert_eq!(runtime.read(sensor).unwrap(), 1180.0);

    // The generated register map answers on the bus.
    let bus = runtime.i2c_bus_id("i2c0").unwrap();
    let who = runtime.i2c_transfer(bus, 0x42, &[0x0F], 1).unwrap();
    assert!(who.ack);
    assert_eq!(who.read, [0xA5]);
    assert!(
        runtime
            .i2c_transfer(bus, 0x42, &[0x10, 0x5A], 0)
            .unwrap()
            .ack
    );
    assert_eq!(
        runtime.i2c_transfer(bus, 0x42, &[0x10], 1).unwrap().read,
        [0x5A]
    );

    let report = runtime.run_report();
    assert_eq!(
        report.part_packs,
        ["gen:relay-module", "gen:linear-temp", "gen:id-chip"]
    );
}

#[test]
fn a_pack_that_shadows_a_catalog_part_is_refused_unless_it_says_so() {
    let shadow = PACKS.replace("type: \"gen:id-chip\"", "type: bme280");
    let shadow = shadow.replace("type: \"gen:id-chip\" }", "type: bme280 }");
    let error = FlexRuntime::from_yaml(&shadow)
        .err()
        .expect("refuse shadowing");
    assert!(error.to_string().contains("shadows a built-in"), "{error}");

    let deliberate = shadow.replace(
        "    type: bme280\n",
        "    type: bme280\n    overrides: bme280\n",
    );
    let mut runtime = FlexRuntime::from_yaml(&deliberate).unwrap();
    let bus = runtime.i2c_bus_id("i2c0").unwrap();
    // The pack, not the catalog BME280 (chip id 0x60), answers.
    assert_eq!(
        runtime.i2c_transfer(bus, 0x42, &[0x0F], 1).unwrap().read,
        [0xA5]
    );
}

#[test]
fn a_pack_with_a_primitive_flex_does_not_run_is_named() {
    let uart = PACKS.replace(
        "      primitive: i2c_device\n",
        "      primitive: uart_device\n",
    );
    let error = FlexRuntime::from_yaml(&uart).err().expect("refuse");
    assert!(
        error.to_string().contains("which Flex does not run"),
        "{error}"
    );
}

// ── 2. LCD1602 (HD44780 behind a PCF8574) as catalog data ───────────────────

const LCD: &str = r#"
schema_version: 1
name: lcd
ports: []
cosim_models: []
i2c_buses:
  - name: i2c0
    frequency_hz: 100000
    devices:
      - { id: lcd, type: hd44780_pcf8574, config: { i2c_address: 0x27 } }
"#;

/// LiquidCrystal_I2C's wire traffic: each nibble is the port with E high,
/// then the port with E low; the backlight bit (P3) stays on.
struct Backpack<'a> {
    runtime: &'a mut FlexRuntime,
    bus: u32,
}

impl Backpack<'_> {
    fn port(&mut self, value: u8) {
        let transfer = self
            .runtime
            .i2c_transfer(self.bus, 0x27, &[value], 0)
            .unwrap();
        assert!(transfer.ack);
        self.runtime.advance_ns(transfer.duration_ns).unwrap();
    }
    fn nibble(&mut self, nibble: u8, rs: bool) {
        let port = (nibble << 4) | 0x08 | u8::from(rs);
        self.port(port | 0x04);
        self.port(port);
    }
    fn byte(&mut self, byte: u8, rs: bool) {
        self.nibble(byte >> 4, rs);
        self.nibble(byte & 0x0F, rs);
    }
    fn print(&mut self, text: &str) {
        for byte in text.bytes() {
            self.byte(byte, true);
        }
    }
}

#[test]
fn lcd1602_backpack_latches_nibbles_on_e_falling_and_shows_text() {
    let mut runtime = FlexRuntime::from_yaml(LCD).unwrap();
    let bus = runtime.i2c_bus_id("i2c0").unwrap();
    let mut lcd = Backpack {
        runtime: &mut runtime,
        bus,
    };
    // The 4-bit init: three single 0x3 nibbles and a 0x2.
    for nibble in [0x3, 0x3, 0x3, 0x2] {
        lcd.nibble(nibble, false);
    }
    lcd.byte(0x28, false); // 4-bit, 2 lines
    lcd.byte(0x0C, false); // display on
    lcd.byte(0x01, false); // clear
    lcd.byte(0x06, false); // entry mode
    lcd.print("Soil 41%");
    lcd.byte(0x80 | 0x40, false); // line 2
    lcd.print("Pump ON");
    // A byte that raises E only, and a backlight-only byte, move no data.
    lcd.port(0x08 | 0x04);
    lcd.port(0x00);
    lcd.port(0x08);
    let state = state(&runtime, "lcd");
    assert_eq!(state["display_on"], true);
    assert_eq!(
        state["text"],
        serde_json::json!(["Soil 41%        ", "Pump ON         "])
    );
}

// ── 3. DS18B20 on the generic one-wire primitive ────────────────────────────

const ONEWIRE: &str = r#"
schema_version: 1
name: one-wire thermometer
ports:
  - { name: dq_drive, signal: mcu.dq, access: write, kind: bool, initial: 1 }
  - { name: dq, signal: bus.dq, access: read, kind: bool }
  - { name: water_c, signal: env.water, access: write, kind: number, unit: "°C", initial: 18.5 }
cosim_models:
  - id: probe
    adapter: behavior
    step_ns: 1000
    inputs: { DQ: mcu.dq, temperature: env.water }
    outputs: { DQ: bus.dq }
    config: { device_type: ds18b20 }
"#;

/// The host side of a 1-Wire bus as portable firmware bit-bangs it.
struct OneWireHost<'a> {
    runtime: &'a mut FlexRuntime,
    drive: u32,
    line: u32,
}

impl OneWireHost<'_> {
    fn us(&mut self, us: u64) {
        self.runtime.advance_ns_in_pieces(us * 1_000).unwrap();
    }
    fn pin(&mut self, high: bool) {
        self.runtime
            .write(self.drive, f64::from(u8::from(high)))
            .unwrap();
    }
    fn sample(&mut self) -> bool {
        self.runtime.read(self.line).unwrap() != 0.0
    }
    fn reset(&mut self) -> bool {
        self.pin(false);
        self.us(480);
        self.pin(true);
        self.us(70);
        let presence = !self.sample();
        self.us(410);
        presence
    }
    fn write_bit(&mut self, bit: bool) {
        self.pin(false);
        self.us(if bit { 6 } else { 60 });
        self.pin(true);
        self.us(if bit { 64 } else { 10 });
    }
    fn read_bit(&mut self) -> bool {
        self.pin(false);
        self.us(3);
        self.pin(true);
        self.us(10);
        let bit = self.sample();
        self.us(57);
        bit
    }
    fn write(&mut self, byte: u8) {
        for bit in 0..8 {
            self.write_bit(byte >> bit & 1 == 1);
        }
    }
    fn read(&mut self) -> u8 {
        (0..8).fold(0, |byte, bit| byte | (u8::from(self.read_bit()) << bit))
    }
}

#[test]
fn ds18b20_converts_its_environment_and_reads_back_with_the_crc() {
    let mut runtime = FlexRuntime::from_yaml(ONEWIRE).unwrap();
    let drive = runtime.port_id("dq_drive").unwrap();
    let line = runtime.port_id("dq").unwrap();
    let mut host = OneWireHost {
        runtime: &mut runtime,
        drive,
        line,
    };
    assert!(host.sample(), "a released line reads high");
    assert!(host.reset(), "presence pulse");
    host.write(0x33);
    let rom: Vec<u8> = (0..8).map(|_| host.read()).collect();
    assert_eq!(rom[0], 0x28);
    assert_eq!(rom[7], labwired_config::onewire_crc8(&rom[..7]));

    assert!(host.reset());
    host.write(0xCC);
    host.write(0x44);
    // 750 ms at the power-on 12 bits; poll the conversion like DallasTemperature.
    let mut polls = 0;
    while !host.read_bit() {
        host.us(10_000);
        polls += 1;
    }
    assert!(
        (73..=76).contains(&polls),
        "conversion took {polls} polls of 10 ms"
    );
    assert!(host.reset());
    host.write(0xCC);
    host.write(0xBE);
    let scratchpad: Vec<u8> = (0..9).map(|_| host.read()).collect();
    assert_eq!(
        labwired_config::onewire_crc8(&scratchpad[..8]),
        scratchpad[8]
    );
    // 18.5 °C = 296 counts of 1/16 °C.
    assert_eq!(i16::from_le_bytes([scratchpad[0], scratchpad[1]]), 296);

    let probe = state(&runtime, "probe");
    assert_eq!(probe["registers"]["TEMP_LSB"], 296 & 0xFF);
    assert_eq!(probe["last_function"], "READ_SCRATCHPAD");
}

// ── 4 and 5. Listen-only sinks: WS2812 strip and TM1637 display ─────────────

const SINKS: &str = r#"
schema_version: 1
name: sinks
ports:
  - { name: led_data, signal: mcu.d6, access: write, kind: bool, initial: 0 }
  - { name: clk, signal: mcu.d2, access: write, kind: bool, initial: 1 }
  - { name: dio, signal: mcu.d3, access: write, kind: bool, initial: 1 }
cosim_models: []
sinks:
  - { id: strip, type: neopixel, config: { num_pixels: 2 }, pins: { DIN: mcu.d6 } }
  - { id: display, type: tm1637-7seg, pins: { CLK: mcu.d2, DIO: mcu.d3 } }
"#;

/// One WS2812 frame, MSB first, with the datasheet bit times (T0H 400 ns,
/// T1H 800 ns, 1250 ns per bit), then a 60 µs reset gap.
fn ws2812_frame(runtime: &mut FlexRuntime, port: u32, grb: &[u8]) {
    for byte in grb {
        for bit in (0..8).rev() {
            let one = byte >> bit & 1 == 1;
            runtime.write(port, 1.0).unwrap();
            runtime.advance_ns(if one { 800 } else { 400 }).unwrap();
            runtime.write(port, 0.0).unwrap();
            runtime.advance_ns(if one { 450 } else { 850 }).unwrap();
        }
    }
    runtime.advance_ns(60_000).unwrap();
}

#[test]
fn ws2812_sink_decodes_exact_edge_times_and_keeps_every_frame() {
    let mut runtime = FlexRuntime::from_yaml(SINKS).unwrap();
    let data = runtime.port_id("led_data").unwrap();
    ws2812_frame(&mut runtime, data, &[0x10, 0x20, 0x30, 0, 0, 0]);
    ws2812_frame(&mut runtime, data, &[0, 0, 0, 0xFF, 0, 0]);
    // The same frame again is not a change.
    ws2812_frame(&mut runtime, data, &[0, 0, 0, 0xFF, 0, 0]);
    ws2812_frame(&mut runtime, data, &[0, 0, 0, 0, 0, 0]);
    let strip = state(&runtime, "strip");
    let history = strip["frame_history"].as_array().unwrap();
    let frames: Vec<&Value> = history.iter().map(|frame| &frame["bytes"]).collect();
    assert_eq!(
        frames,
        [
            &serde_json::json!([0x10, 0x20, 0x30, 0, 0, 0]),
            &serde_json::json!([0, 0, 0, 0xFF, 0, 0]),
            &serde_json::json!([0, 0, 0, 0, 0, 0]),
        ]
    );
    // Each frame is stamped with the time it completed, in ns (1 GHz clock).
    let cycles: Vec<u64> = history
        .iter()
        .map(|f| f["cycle"].as_u64().unwrap())
        .collect();
    let frame_ns = 48 * 1_250 + 60_000;
    // The last bit of each frame is a 0: its falling edge is 400 ns in.
    assert_eq!(cycles[0], 47 * 1_250 + 400);
    assert_eq!(cycles[1] - cycles[0], frame_ns);
    assert_eq!(strip["frame_history_dropped"], 0);
}

/// TM1637's two-wire protocol as its Arduino library bit-bangs it.
struct Tm1637Host<'a> {
    runtime: &'a mut FlexRuntime,
    clk: u32,
    dio: u32,
}

impl Tm1637Host<'_> {
    fn set(&mut self, port: u32, high: bool) {
        self.runtime.write(port, f64::from(u8::from(high))).unwrap();
        self.runtime.advance_ns(5_000).unwrap();
    }
    fn start(&mut self) {
        self.set(self.dio, false);
    }
    fn stop(&mut self) {
        self.set(self.clk, false);
        self.set(self.dio, false);
        self.set(self.clk, true);
        self.set(self.dio, true);
    }
    fn byte(&mut self, byte: u8) {
        for bit in 0..8 {
            self.set(self.clk, false);
            self.set(self.dio, byte >> bit & 1 == 1);
            self.set(self.clk, true);
        }
        // ACK clock.
        self.set(self.clk, false);
        self.set(self.dio, true);
        self.set(self.clk, true);
        self.set(self.clk, false);
    }
}

#[test]
fn tm1637_sink_listens_to_clk_and_dio_and_publishes_its_digits() {
    let mut runtime = FlexRuntime::from_yaml(SINKS).unwrap();
    let clk = runtime.port_id("clk").unwrap();
    let dio = runtime.port_id("dio").unwrap();
    let mut host = Tm1637Host {
        runtime: &mut runtime,
        clk,
        dio,
    };
    host.start();
    host.byte(0x40);
    host.stop();
    host.start();
    host.byte(0xC0);
    for segments in [0x06, 0x5B, 0x4F, 0x66] {
        host.byte(segments);
    }
    host.stop();
    host.start();
    host.byte(0x8F);
    host.stop();
    let display = state(&runtime, "display");
    assert_eq!(display["text"], "1234", "{display}");
    assert_eq!(display["display_on"], true);
    assert_eq!(display["brightness"], 7);
}

#[test]
fn a_sink_must_bind_every_pin_of_its_part() {
    let missing = SINKS.replace(", DIO: mcu.d3 }", " }").replace(
        "  - { name: dio, signal: mcu.d3, access: write, kind: bool, initial: 1 }\n",
        "",
    );
    let error = FlexRuntime::from_yaml(&missing).err().expect("refuse");
    assert!(error.to_string().contains("pin 'DIO'"), "{error}");
}

// ── The whole device: one fixture with all of the above ─────────────────────

#[cfg(feature = "flex-script")]
mod fixture {
    use crate::flex_script::{run_script, FlexResultConfig};
    use labwired_config::FlexTestScript;

    const SYSTEM: &str = include_str!("../../wasm/tests/fixtures/flex-generated-parts/system.yaml");
    const FIRMWARE: &[u8] =
        include_bytes!("../../wasm/tests/fixtures/flex-generated-parts/firmware.wasm");

    fn run(script: &str) -> crate::flex_script::FlexTestResult {
        let script: FlexTestScript = serde_yaml::from_str(script).unwrap();
        script.validate().unwrap();
        let config = FlexResultConfig {
            script: "script.yaml".into(),
            flex: "system.yaml".into(),
            firmware: "firmware.wasm".into(),
            firmware_sha256: None,
            sequence: script.steps(),
            limits: script.limits.clone(),
        };
        run_script(&script, config, SYSTEM, FIRMWARE).result
    }

    fn assert_pass(result: &crate::flex_script::FlexTestResult) {
        let failed: Vec<_> = result
            .assertions
            .iter()
            .filter(|assertion| !assertion.passed)
            .map(|assertion| {
                format!(
                    "{:?} observed {:?} {:?}",
                    assertion.assertion, assertion.observed, assertion.observed_value
                )
            })
            .collect();
        assert_eq!(
            result.status,
            "pass",
            "{:?} {:?}; failed: {failed:#?}; devices: {}",
            result.stop_reason,
            result.message,
            serde_json::to_string(&result.devices).unwrap()
        );
    }

    #[test]
    fn cold_water_switches_the_heater_and_every_display_shows_the_reading() {
        let result = run(include_str!(
            "../../wasm/tests/fixtures/flex-generated-parts/test-cold-water.yaml"
        ));
        assert_pass(&result);
        let report = result.run_report.as_ref().unwrap();
        assert_eq!(report.part_packs, ["gen:relay-module-active-low"]);
        assert_eq!(report.sinks.len(), 2);
    }

    #[test]
    fn warm_water_leaves_the_heater_off() {
        let result = run(include_str!(
            "../../wasm/tests/fixtures/flex-generated-parts/test-warm-water.yaml"
        ));
        assert_pass(&result);
    }

    #[test]
    fn a_wrong_expected_reading_fails_rather_than_passing_vacuously() {
        let script =
            include_str!("../../wasm/tests/fixtures/flex-generated-parts/test-cold-water.yaml")
                .replace("equals: \"1850\"", "equals: \"1851\"");
        let result = run(&script);
        assert_eq!(result.status, "fail");
    }
}
