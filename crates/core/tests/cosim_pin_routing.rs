// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Co-simulation board routing, on a REAL chip.
//!
//! `crates/core/src/cosim/routing.rs` unit-tests the grammar and the unit
//! conversions in isolation. What this file covers is the part those cannot:
//! that a manifest path resolves against an actual `SystemBus` built from a
//! shipped chip descriptor, and that a routed value lands where the FIRMWARE
//! would look for it — the GPIO input register it samples, the ADC channel it
//! converts — rather than merely in the signal store.
//!
//! Both directions are asserted through the same seams the firmware uses:
//! `read_gpio_output` for an output pad, the GPIO input register for a driven
//! pin, and the ADC's own injected-channel readback for an analog level.

mod common;
use common::root;
use labwired_config::{ChipDescriptor, CosimAdapter, CosimModelConfig, SystemManifest};
use labwired_core::bus::SystemBus;
use labwired_core::cosim::{CosimSession, CosimSignalValue, RoutingError};
use labwired_core::Bus;
use std::collections::HashMap;
use std::path::Path;

/// A bare bus for a shipped chip descriptor, with no external devices, so
/// nothing but the co-simulation touches the ADC.
fn chip_bus(chip: &str) -> SystemBus {
    let chip_path = root(&format!("configs/chips/{chip}.yaml"));
    let descriptor = ChipDescriptor::from_file(&chip_path).expect("load chip descriptor");
    let manifest_yaml = format!(
        "name: \"cosim-routing\"\nchip: \"{}\"\nexternal_devices: []\n",
        chip_path.display()
    );
    let manifest: SystemManifest = serde_yaml::from_str(&manifest_yaml).expect("parse manifest");
    SystemBus::from_config(&descriptor, &manifest).expect("build bus")
}

/// A bare NUCLEO-F401RE: the chip the `cosim-spice-rc` example targets.
fn f401_bus() -> SystemBus {
    chip_bus("stm32f401")
}

/// A `mock` model: static outputs from `config.outputs`, routed through the
/// manifest's `outputs:` map like any other adapter's.
fn mock_model(
    step_ns: u64,
    inputs: &[(&str, &str)],
    outputs: &[(&str, &str)],
    static_outputs: &[(&str, serde_yaml::Value)],
) -> CosimModelConfig {
    let mut config = HashMap::new();
    let mapping: serde_yaml::Mapping = static_outputs
        .iter()
        .map(|(k, v)| (serde_yaml::Value::String((*k).to_string()), v.clone()))
        .collect();
    config.insert("outputs".to_string(), serde_yaml::Value::Mapping(mapping));
    CosimModelConfig {
        id: "routing_probe".to_string(),
        adapter: CosimAdapter::Mock,
        model: None,
        step_ns,
        inputs: inputs
            .iter()
            .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
            .collect(),
        outputs: outputs
            .iter()
            .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
            .collect(),
        config,
    }
}

fn build_session(bus: &SystemBus, models: &[CosimModelConfig]) -> CosimSession {
    let session = CosimSession::new(models, Path::new("."), bus)
        .expect("build session")
        .expect("models were declared, so there is a session");
    assert_eq!(
        session.binding_errors(),
        &[] as &[RoutingError],
        "every path in this fixture must resolve on an F401"
    );
    session
}

/// The injected 12-bit count the next conversion on `channel` would return.
/// `0xFFFF` means nothing has been injected.
fn adc_channel_count(bus: &mut SystemBus, channel: u8) -> u16 {
    let idx = bus
        .find_peripheral_index_by_name("adc1")
        .expect("F401 declares adc1");
    bus.peripherals[idx]
        .dev
        .as_any_mut()
        .expect("adc1 downcasts")
        .downcast_mut::<labwired_core::peripherals::adc::Adc>()
        .expect("adc1 is an Adc")
        .channel_input_count(channel)
}

/// Drive a GPIO output latch the way firmware does — an MMIO store to ODR.
fn write_odr_bit(bus: &mut SystemBus, pad: &str, level: bool) {
    let (addr, bit) = SystemBus::resolve_pin_odr_pub(bus, pad).expect("pad resolves to an ODR");
    let current = bus.read_u32(addr).expect("read ODR");
    let next = if level {
        current | (1 << bit)
    } else {
        current & !(1 << bit)
    };
    bus.write_u32(addr, next).expect("write ODR");
}

/// Set one pad's two-bit MODER field the way firmware does: read-modify-write
/// of the port's mode register. 00 input, 01 output, 10 alternate function,
/// 11 analog (RM0368 §8.4.1).
fn write_moder(bus: &mut SystemBus, port: &str, pin: u8, mode: u32) {
    let idx = bus
        .find_peripheral_index_by_name(port)
        .expect("F401 declares the port");
    let moder = bus.peripherals[idx].base;
    let current = bus.read_u32(moder).expect("read MODER");
    let shift = u32::from(pin) * 2;
    let next = (current & !(0b11 << shift)) | (mode << shift);
    bus.write_u32(moder, next).expect("write MODER");
}

fn read_idr_bit(bus: &mut SystemBus, pad: &str) -> bool {
    let (addr, bit) = SystemBus::resolve_pin_idr_pub(bus, pad).expect("pad resolves to an IDR");
    bus.read_u32(addr).expect("read IDR") >> bit & 1 != 0
}

/// 84 MHz core, 100 us model period: the boundary is 8400 cycles, and both
/// edges of that are asserted — a machine at 0 may run the whole period, and a
/// machine already at the boundary must still be allowed to make progress
/// rather than be handed a zero budget and hang.
#[test]
fn the_cycle_budget_lands_the_machine_on_the_model_boundary() {
    let bus = f401_bus();
    assert_eq!(bus.cpu_hz, 84_000_000, "F401 descriptor declares 84 MHz");
    let session = build_session(&bus, &[mock_model(100_000, &[], &[], &[])]);

    assert_eq!(session.cpu_hz(), 84_000_000);
    assert_eq!(session.step_ns(), 100_000);
    assert_eq!(session.cycles_until_boundary(0), 8_400);
    assert_eq!(session.cycles_until_boundary(8_000), 400);
    assert_eq!(session.cycles_until_boundary(8_400), 1);
    assert_eq!(session.cycles_until_boundary(100_000), 1);
}

/// The finest declared period wins: stepping at anything coarser would let the
/// machine run past the faster model's boundary before that model saw the pin
/// levels that produced it.
#[test]
fn the_lockstep_granularity_is_the_finest_model_period() {
    let bus = f401_bus();
    let mut fast = mock_model(10_000, &[], &[], &[]);
    fast.id = "fast".to_string();
    let mut slow = mock_model(1_000_000, &[], &[], &[]);
    slow.id = "slow".to_string();
    let session = build_session(&bus, &[slow, fast]);
    assert_eq!(session.step_ns(), 10_000);
    assert_eq!(session.model_count(), 2);
}

/// Machine → model. The level is read through `read_gpio_output`, the same
/// accessor a `board_io` LED reads, so a pin the firmware drives and a pin a
/// model samples can never disagree.
#[test]
fn a_firmware_driven_pad_reaches_the_signal_store() {
    let mut bus = f401_bus();
    let mut session = build_session(
        &bus,
        &[mock_model(
            100_000,
            &[("gpio", "board.gpio.pa5")],
            &[],
            &[("unused", serde_yaml::Value::Bool(false))],
        )],
    );

    write_odr_bit(&mut bus, "PA5", true);
    session
        .advance_to(8_400, &mut bus)
        .expect("step at the boundary");
    assert_eq!(
        session.signals().get("board.gpio.pa5"),
        Some(&CosimSignalValue::Bool(true)),
        "PA5 driven high must reach the model as true"
    );

    write_odr_bit(&mut bus, "PA5", false);
    session
        .advance_to(16_800, &mut bus)
        .expect("step at the next boundary");
    assert_eq!(
        session.signals().get("board.gpio.pa5"),
        Some(&CosimSignalValue::Bool(false)),
        "PA5 driven low must reach the model as false"
    );
}

/// Machine → model, direction. `board.gpio_output.<pad>` reads MODER, so a pad
/// the firmware made an output reads true and an input, analog or
/// alternate-function pad reads false. The output latch is left HIGH on both
/// pads throughout: direction must come from the mode register, never from the
/// level `board.gpio.<pad>` reads.
#[test]
fn the_direction_path_reads_the_mode_register() {
    let mut bus = f401_bus();
    let mut session = build_session(
        &bus,
        &[mock_model(
            100_000,
            &[
                ("pa5_out", "board.gpio_output.pa5"),
                ("pa0_out", "board.gpio_output.pa0"),
            ],
            &[],
            &[("unused", serde_yaml::Value::Bool(false))],
        )],
    );
    write_odr_bit(&mut bus, "PA5", true);
    write_odr_bit(&mut bus, "PA0", true);

    let direction = |session: &CosimSession, path: &str| session.signals().get(path).cloned();

    write_moder(&mut bus, "gpioa", 5, 0b01);
    write_moder(&mut bus, "gpioa", 0, 0b00);
    session.advance_to(8_400, &mut bus).expect("step");
    assert_eq!(
        direction(&session, "board.gpio_output.pa5"),
        Some(CosimSignalValue::Bool(true)),
        "MODER 01 makes PA5 an output"
    );
    assert_eq!(
        direction(&session, "board.gpio_output.pa0"),
        Some(CosimSignalValue::Bool(false)),
        "MODER 00 leaves PA0 an input, whatever its output latch holds"
    );

    write_moder(&mut bus, "gpioa", 5, 0b10);
    write_moder(&mut bus, "gpioa", 0, 0b11);
    session.advance_to(16_800, &mut bus).expect("step");
    assert_eq!(
        direction(&session, "board.gpio_output.pa5"),
        Some(CosimSignalValue::Bool(false)),
        "an alternate-function pad is not driven by the latch board.gpio reads"
    );
    assert_eq!(
        direction(&session, "board.gpio_output.pa0"),
        Some(CosimSignalValue::Bool(false)),
        "an analog pad is not an output"
    );

    write_moder(&mut bus, "gpioa", 0, 0b01);
    session.advance_to(25_200, &mut bus).expect("step");
    assert_eq!(
        direction(&session, "board.gpio_output.pa0"),
        Some(CosimSignalValue::Bool(true)),
        "reconfiguring PA0 as an output is seen at the next boundary"
    );
}

/// An in-core analog model: `netlist_text`, `probes` and `sources` as the
/// manifest spells them.
fn analog_model(
    step_ns: u64,
    inputs: &[(&str, &str)],
    outputs: &[(&str, &str)],
    netlist: &str,
    probes: &[(&str, &str)],
    sources: &[(&str, &str)],
    vdd: f64,
) -> CosimModelConfig {
    let mapping = |pairs: &[(&str, &str)]| {
        serde_yaml::Value::Mapping(
            pairs
                .iter()
                .map(|(k, v)| {
                    (
                        serde_yaml::Value::String((*k).to_string()),
                        serde_yaml::Value::String((*v).to_string()),
                    )
                })
                .collect(),
        )
    };
    let config = HashMap::from([
        (
            "netlist_text".to_string(),
            serde_yaml::Value::String(netlist.to_string()),
        ),
        ("probes".to_string(), mapping(probes)),
        ("sources".to_string(), mapping(sources)),
        ("vdd".to_string(), serde_yaml::Value::from(vdd)),
    ]);
    CosimModelConfig {
        id: "circuit".to_string(),
        adapter: CosimAdapter::Analog,
        model: None,
        step_ns,
        inputs: inputs
            .iter()
            .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
            .collect(),
        outputs: outputs
            .iter()
            .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
            .collect(),
        config,
    }
}

/// Machine → model, direction, on the ATmega328P. `board.gpio_output.<pad>` is
/// the DDRx bit and `board.gpio.<pad>` the PORTx latch, both read from the
/// port window the AVR core mirrors its I/O registers to. PORTD is the port the
/// capacitive-touch lab wires (send D4 = PD4, receive D2 = PD2).
#[test]
fn the_direction_path_reads_ddr_on_the_atmega328p() {
    let mut bus = chip_bus("atmega328p");
    let mut session = build_session(
        &bus,
        &[mock_model(
            1_000,
            &[
                ("ctl_pd4", "board.gpio_output.pd4"),
                ("ctl_pd2", "board.gpio_output.pd2"),
                ("drv_pd4", "board.gpio.pd4"),
            ],
            &[],
            &[("unused", serde_yaml::Value::Bool(false))],
        )],
    );
    let portd = bus
        .find_peripheral_index_by_name("portd")
        .expect("atmega328p maps a portd window");
    let base = bus.peripherals[portd].base;
    let (ddrd, portd_latch) = (base + 1, base + 2);
    // 16 MHz: 1 us is 16 cycles.
    let direction = |session: &CosimSession, path: &str| session.signals().get(path).cloned();

    // Both pads start as inputs; PD4's pull-up latch is set to prove PORT is
    // not mistaken for direction.
    bus.write_u8(portd_latch, 1 << 4).unwrap();
    session.advance_to(16, &mut bus).expect("step");
    assert_eq!(
        direction(&session, "board.gpio_output.pd4"),
        Some(CosimSignalValue::Bool(false)),
        "DDRD bit 4 clear: PD4 is an input, pull-up or not"
    );
    assert_eq!(
        direction(&session, "board.gpio_output.pd2"),
        Some(CosimSignalValue::Bool(false))
    );

    bus.write_u8(ddrd, 1 << 4).unwrap();
    session.advance_to(32, &mut bus).expect("step");
    assert_eq!(
        direction(&session, "board.gpio_output.pd4"),
        Some(CosimSignalValue::Bool(true)),
        "DDRD bit 4 set: PD4 drives"
    );
    assert_eq!(
        direction(&session, "board.gpio.pd4"),
        Some(CosimSignalValue::Bool(true)),
        "and it drives its PORTD latch, high"
    );
    assert_eq!(
        direction(&session, "board.gpio_output.pd2"),
        Some(CosimSignalValue::Bool(false)),
        "PD2 is still an input"
    );

    bus.write_u8(portd_latch, 0).unwrap();
    bus.write_u8(ddrd, (1 << 4) | (1 << 2)).unwrap();
    session.advance_to(48, &mut bus).expect("step");
    assert_eq!(
        direction(&session, "board.gpio_output.pd2"),
        Some(CosimSignalValue::Bool(true)),
        "CapacitiveSensor discharges the pad by making PD2 an output"
    );
    assert_eq!(
        direction(&session, "board.gpio.pd4"),
        Some(CosimSignalValue::Bool(false))
    );
}

/// A pad on a port the chip does not have does not resolve, and an AVR port is
/// eight bits wide.
#[test]
fn an_atmega_pad_outside_its_ports_is_refused() {
    let bus = chip_bus("atmega328p");
    for (path, pad) in [
        ("board.gpio_output.pd8", "pd8"),
        ("board.gpio_output.pa0", "pa0"),
    ] {
        let models = [mock_model(
            1_000,
            &[("drive", path)],
            &[],
            &[("unused", serde_yaml::Value::Bool(false))],
        )];
        let session = CosimSession::new(&models, Path::new("."), &bus)
            .expect("building the session is not itself an error")
            .expect("models were declared");
        assert_eq!(
            session.binding_errors(),
            &[RoutingError::UnknownPad {
                path: path.to_string(),
                pad: pad.to_string(),
            }]
        );
    }
}

/// Model → machine, volts. A voltage routed to `board.gpio_in.<pad>` is a
/// Schmitt input with the chip's datasheet thresholds: on the F401 at 3.3 V
/// that is VIL 0.99 V and VIH 2.31 V. The ramp is set from outside with
/// `set_signal_number`, through a voltage source, so the level the model
/// outputs is exactly the one asked for.
#[test]
fn volts_on_a_pin_path_read_through_the_datasheet_thresholds() {
    let mut bus = f401_bus();
    let mut session = build_session(
        &bus,
        &[analog_model(
            100_000,
            &[("level", "ui.bench.volts")],
            &[("v_pin", "board.gpio_in.pc13")],
            "Vbench in 0 dc 0\nR1 in pin 1\nR2 pin 0 1e12\n",
            &[("v_pin", "v(pin)")],
            &[("level", "Vbench")],
            3.3,
        )],
    );

    let mut cycles = 0;
    let mut level_at = |session: &mut CosimSession, bus: &mut SystemBus, volts: f64| {
        session
            .set_signal_number("ui.bench.volts", volts)
            .expect("the model reads ui.bench.volts");
        cycles += 8_400;
        let (_, errors) = session.advance_to(cycles, bus).expect("step");
        assert!(errors.is_empty(), "{errors:?}");
        read_idr_bit(bus, "PC13")
    };

    assert!(!level_at(&mut session, &mut bus, 0.0));
    assert!(
        !level_at(&mut session, &mut bus, 2.0),
        "2.0 V: rising, below VIH"
    );
    assert!(
        level_at(&mut session, &mut bus, 2.4),
        "2.4 V: at or above VIH"
    );
    assert!(
        level_at(&mut session, &mut bus, 1.2),
        "1.2 V: falling, above VIL"
    );
    assert!(
        !level_at(&mut session, &mut bus, 0.9),
        "0.9 V: at or below VIL"
    );
    assert!(
        !level_at(&mut session, &mut bus, 2.2),
        "2.2 V: rising again, in the band"
    );
}

/// `ui.` inputs start at 0 / false before anyone sets them, typed by what the
/// model does with them: a switch control is a level, a source is a number.
#[test]
fn ui_inputs_exist_from_the_start_at_zero() {
    let bus = f401_bus();
    let session = build_session(
        &bus,
        &[analog_model(
            100_000,
            &[
                ("ctl_touch", "ui.touch.pressed"),
                ("level", "ui.bench.volts"),
            ],
            &[],
            "Vbench in 0 dc 1\nR1 in pad 1k\nStouch pad 0 ctl_touch ron=1k roff=1e12\n",
            &[("v_pad", "v(pad)")],
            &[("level", "Vbench")],
            3.3,
        )],
    );
    assert_eq!(
        session.signals().get("ui.touch.pressed"),
        Some(&CosimSignalValue::Bool(false))
    );
    assert_eq!(
        session.signals().get("ui.bench.volts"),
        Some(&CosimSignalValue::F64(0.0))
    );
}

/// `set_signal` stores the value for the models to read at their next step; a
/// number meant as a press closes a switch whatever the circuit's supply.
#[test]
fn set_signal_reaches_the_model_at_its_next_step() {
    let mut bus = chip_bus("atmega328p");
    // 5 V through 10 k onto a pad node; a press switches 1 k from the node to
    // ground. The node voltage is routed to a plain store path to read it back.
    let mut session = build_session(
        &bus,
        &[analog_model(
            1_000,
            &[("ctl_touch", "ui.touch.pressed")],
            &[("v_pad", "bench.v_pad")],
            "V1 in 0 dc 5\nR1 in pad 10k\nStouch pad 0 ctl_touch ron=1k roff=1e12\n",
            &[("v_pad", "v(pad)")],
            &[],
            5.0,
        )],
    );
    let v_pad = |session: &CosimSession| match session.signals().get("bench.v_pad") {
        Some(CosimSignalValue::F64(volts)) => *volts,
        other => panic!("no pad voltage in the store: {other:?}"),
    };

    // 16 MHz: 1 us (one model step) is 16 cycles.
    session.advance_to(16, &mut bus).expect("step");
    let released = v_pad(&session);
    assert!(
        released > 4.9,
        "released: the pad sits at 5 V ({released} V)"
    );

    session
        .set_signal_number("ui.touch.pressed", 1.0)
        .expect("a model reads ui.touch.pressed");
    assert_eq!(
        session.signals().get("ui.touch.pressed"),
        Some(&CosimSignalValue::Bool(true)),
        "1 is a press on a switch control, not 1 V against a 2.5 V threshold"
    );
    assert!(v_pad(&session) > 4.9, "nothing moves until the model steps");
    session.advance_to(32, &mut bus).expect("step");
    let pressed = v_pad(&session);
    assert!(
        (pressed - 5.0 / 11.0).abs() < 0.01,
        "pressed: 1 k to ground divides 10 k from 5 V ({pressed} V)"
    );

    session
        .set_signal("ui.touch.pressed", CosimSignalValue::Bool(false))
        .expect("set false");
    session.advance_to(48, &mut bus).expect("step");
    assert!(v_pad(&session) > 4.9, "released again");
}

#[test]
fn set_signal_refuses_a_path_no_model_reads() {
    let bus = f401_bus();
    let mut session = build_session(
        &bus,
        &[mock_model(
            100_000,
            &[("touch", "ui.touch.pressed"), ("led", "board.gpio.pa5")],
            &[],
            &[("unused", serde_yaml::Value::Bool(false))],
        )],
    );
    assert_eq!(
        session.set_signal("ui.touch.presed", CosimSignalValue::Bool(true)),
        Err(RoutingError::UnknownSignal {
            path: "ui.touch.presed".to_string()
        })
    );
    assert!(
        !session.signals().contains_key("ui.touch.presed"),
        "a refused signal is not stored"
    );
    assert_eq!(
        session.set_signal_number("board.gpio.pa5", 1.0),
        Err(RoutingError::NotSettable {
            path: "board.gpio.pa5".to_string()
        }),
        "the machine owns board paths"
    );
    assert!(matches!(
        session.set_signal_number("ui.touch.pressed", f64::NAN),
        Err(RoutingError::NotFinite { .. })
    ));
    session
        .set_signal("ui.touch.pressed", CosimSignalValue::F64(1.0))
        .expect("a path a model reads is settable");
    assert_eq!(
        session.signals().get("ui.touch.pressed"),
        Some(&CosimSignalValue::F64(1.0)),
        "a mock cannot say what it expects, so the number is kept as given"
    );
}

/// A pad `board.gpio.<pad>` cannot address cannot be asked its direction
/// either. The RP2040's GPIOs live on the `sio` block, which no board path
/// resolves, so the session refuses the route instead of reading false.
#[test]
fn a_direction_pad_that_does_not_resolve_is_refused_at_bind_time() {
    let bus = chip_bus("rp2040");
    let models = [mock_model(
        100_000,
        &[("drive", "board.gpio_output.gpio5")],
        &[],
        &[("unused", serde_yaml::Value::Bool(false))],
    )];
    let session = CosimSession::new(&models, Path::new("."), &bus)
        .expect("building the session is not itself an error")
        .expect("models were declared");
    assert_eq!(
        session.binding_errors(),
        &[RoutingError::UnknownPad {
            path: "board.gpio_output.gpio5".to_string(),
            pad: "gpio5".to_string(),
        }]
    );
}

/// Model → machine, analog. 1.65 V is half of the 3.3 V reference, which the
/// ADC model converts to 2047 counts at 12 bits — the same count
/// `examples/ntc-thermistor-lab` asserts for its divider midpoint, because the
/// routing hands the ADC millivolts and lets it own the conversion instead of
/// doing the arithmetic a second time with a second rounding rule.
#[test]
fn a_model_voltage_lands_on_the_adc_channel_of_its_pad() {
    let mut bus = f401_bus();
    assert_eq!(
        adc_channel_count(&mut bus, 0),
        0xFFFF,
        "nothing is injected before the first co-sim step"
    );

    let mut session = build_session(
        &bus,
        &[mock_model(
            100_000,
            &[],
            &[("v_out", "board.analog.pa0_volts")],
            &[("v_out", serde_yaml::Value::from(1.65))],
        )],
    );
    let (routed, errors) = session
        .advance_to(8_400, &mut bus)
        .expect("step at the boundary");
    assert!(errors.is_empty(), "unexpected routing errors: {errors:?}");
    assert_eq!(routed.len(), 1, "one model, one boundary crossed");
    assert_eq!(adc_channel_count(&mut bus, 0), 2047);
}

/// The pad form resolves through the chip descriptor's `analog_pins:`, not a
/// built-in table: on an F401 PC5 is ADC1_IN15, so a model voltage routed to
/// `board.analog.pc5_volts` must land on channel 15 and leave channel 0 alone.
#[test]
fn the_pad_form_resolves_through_the_descriptor() {
    let mut bus = f401_bus();
    let mut session = build_session(
        &bus,
        &[mock_model(
            100_000,
            &[],
            &[("v_out", "board.analog.pc5_volts")],
            &[("v_out", serde_yaml::Value::from(1.65))],
        )],
    );
    session.advance_to(8_400, &mut bus).expect("step");
    assert_eq!(adc_channel_count(&mut bus, 15), 2047);
    assert_eq!(adc_channel_count(&mut bus, 0), 0xFFFF);
}

/// On an L476 PA0 is ADC1_IN5, not IN0, and its descriptor records no
/// `analog_pins:`. The route must be refused at bind time. The old built-in
/// F1/F4 table resolved it to channel 0, so firmware converting IN5 read a
/// different value with nothing reporting the mismatch.
#[test]
fn a_pad_the_descriptor_does_not_name_is_refused() {
    let bus = chip_bus("stm32l476");
    let models = [mock_model(
        100_000,
        &[],
        &[("v_out", "board.analog.pa0_volts")],
        &[("v_out", serde_yaml::Value::from(1.65))],
    )];
    let session = CosimSession::new(&models, Path::new("."), &bus)
        .expect("building the session is not itself an error")
        .expect("models were declared");
    assert_eq!(
        session.binding_errors(),
        &[RoutingError::NoAdcChannel {
            path: "board.analog.pa0_volts".to_string(),
            pad: "pa0".to_string(),
        }]
    );
    assert!(session.binding_errors()[0]
        .to_string()
        .contains("adc.<peripheral>.<channel>_volts"));
}

/// The explicit form is the way through on such a chip: it names the ADC and
/// input, so it needs no descriptor data.
#[test]
fn the_explicit_form_works_where_the_descriptor_is_silent() {
    let mut bus = chip_bus("stm32l476");
    let session = CosimSession::new(
        &[mock_model(
            100_000,
            &[],
            &[("v_out", "adc.adc1.5_volts")],
            &[("v_out", serde_yaml::Value::from(1.65))],
        )],
        Path::new("."),
        &bus,
    )
    .expect("build")
    .expect("models were declared");
    assert!(
        session.binding_errors().is_empty(),
        "{:?}",
        session.binding_errors()
    );
    let mut session = session;
    let boundary = session.cycles_until_boundary(0);
    let (_, errors) = session.advance_to(boundary, &mut bus).expect("step");
    assert!(errors.is_empty(), "{errors:?}");
    assert_eq!(adc_channel_count(&mut bus, 5), 2047);
}

/// The chip-neutral form addresses a controller and channel directly, for
/// parts whose pad → channel map LabWired does not model. It must land on the
/// same channel the pad form does.
#[test]
fn the_explicit_adc_channel_form_routes_to_the_same_place() {
    let mut bus = f401_bus();
    let mut session = build_session(
        &bus,
        &[mock_model(
            100_000,
            &[],
            &[("v_out", "adc.adc1.0_volts")],
            &[("v_out", serde_yaml::Value::from(1.65))],
        )],
    );
    session.advance_to(8_400, &mut bus).expect("step");
    assert_eq!(adc_channel_count(&mut bus, 0), 2047);
}

/// Model → machine, digital. The level goes through `set_gpio_input`, the seam
/// a `board_io` button and a sensor status line already use, so the firmware
/// samples it from the input register exactly as it would a real pin.
#[test]
fn a_model_output_drives_a_gpio_input_pin() {
    let mut bus = f401_bus();
    assert!(!read_idr_bit(&mut bus, "PC13"), "PC13 starts low");

    let mut session = build_session(
        &bus,
        &[mock_model(
            100_000,
            &[],
            &[("pressed", "board.gpio_in.pc13")],
            &[("pressed", serde_yaml::Value::Bool(true))],
        )],
    );
    let (_, errors) = session.advance_to(8_400, &mut bus).expect("step");
    assert!(errors.is_empty(), "unexpected routing errors: {errors:?}");
    assert!(
        read_idr_bit(&mut bus, "PC13"),
        "the model's true must be readable on PC13's input register"
    );
}

/// No boundary reached, no model stepped, nothing written. A co-simulation
/// must not run ahead of simulated time just because the loop called it.
#[test]
fn nothing_is_routed_before_the_first_boundary() {
    let mut bus = f401_bus();
    let mut session = build_session(
        &bus,
        &[mock_model(
            100_000,
            &[],
            &[("v_out", "board.analog.pa0_volts")],
            &[("v_out", serde_yaml::Value::from(1.65))],
        )],
    );
    let (routed, errors) = session
        .advance_to(8_399, &mut bus)
        .expect("step below the boundary");
    assert!(routed.is_empty(), "8399 cycles is 99.99 us, not 100 us");
    assert!(errors.is_empty());
    assert_eq!(adc_channel_count(&mut bus, 0), 0xFFFF);
}

/// A pad that does not exist on this chip is caught when the session is built,
/// not silently read as low for the whole run.
#[test]
fn an_unroutable_pad_is_reported_at_bind_time() {
    let bus = f401_bus();
    // The F401 descriptor declares gpioa/gpiob/gpioc only.
    let models = [mock_model(
        100_000,
        &[("gpio", "board.gpio.pz9")],
        &[],
        &[("unused", serde_yaml::Value::Bool(false))],
    )];
    let session = CosimSession::new(&models, Path::new("."), &bus)
        .expect("building the session is not itself an error")
        .expect("models were declared");
    assert_eq!(
        session.binding_errors(),
        &[RoutingError::UnknownPad {
            path: "board.gpio.pz9".to_string(),
            pad: "pz9".to_string(),
        }]
    );
}

// ── ADC routes are checked against the converter that will take them ───────

/// The binding errors for one model output routed to `path`.
fn output_binding_errors(bus: &SystemBus, path: &str) -> Vec<RoutingError> {
    let models = [mock_model(
        100_000,
        &[],
        &[("v_out", path)],
        &[("v_out", serde_yaml::Value::from(1.65))],
    )];
    CosimSession::new(&models, Path::new("."), bus)
        .expect("building the session is not itself an error")
        .expect("models were declared")
        .binding_errors()
        .to_vec()
}

fn assert_mentions(error: &RoutingError, needles: &[&str]) {
    let message = error.to_string();
    for needle in needles {
        assert!(
            message.contains(needle),
            "`{message}` does not mention `{needle}`"
        );
    }
}

/// An F4 ADC1 has regular channels 0..=18. A route to channel 99 used to bind,
/// step, and write nothing — the ADC model drops a channel it does not have —
/// so the firmware converted an untouched input while the run reported no
/// error. It is refused when the session is built, with the valid range.
#[test]
fn an_adc_channel_the_converter_does_not_have_is_refused_at_bind_time() {
    let errors = output_binding_errors(&f401_bus(), "adc.adc1.99_volts");
    assert_eq!(
        errors,
        vec![RoutingError::NoSuchAdcChannel {
            path: "adc.adc1.99_volts".to_string(),
            peripheral: "adc1".to_string(),
            channel: 99,
            channels: 19,
        }]
    );
    assert_mentions(&errors[0], &["adc.adc1.99_volts", "'adc1'", "0..=18", "99"]);
}

#[test]
fn an_adc_the_bus_does_not_have_is_refused_at_bind_time() {
    let errors = output_binding_errors(&f401_bus(), "adc.nope.0_volts");
    assert_eq!(
        errors,
        vec![RoutingError::UnknownPeripheral {
            path: "adc.nope.0_volts".to_string(),
            peripheral: "nope".to_string(),
        }]
    );
    assert_mentions(&errors[0], &["adc.nope.0_volts", "'nope'"]);
}

/// USART2 exists on this bus; it is just not a converter.
#[test]
fn a_peripheral_that_is_not_an_adc_is_refused_at_bind_time() {
    let errors = output_binding_errors(&chip_bus("stm32f401cdu6"), "adc.usart2.0_volts");
    assert_eq!(
        errors,
        vec![RoutingError::NotAnAdc {
            path: "adc.usart2.0_volts".to_string(),
            peripheral: "usart2".to_string(),
        }]
    );
    assert_mentions(
        &errors[0],
        &["adc.usart2.0_volts", "'usart2'", "not an ADC"],
    );
}

/// The pad form goes through the same check: whatever `analog_pins:` says, the
/// named ADC must have that channel. A descriptor typo is a bind error naming
/// the pad's path, not a silent write to nowhere.
#[test]
fn a_descriptor_pad_on_a_channel_the_adc_does_not_have_is_refused() {
    let chip_path = root("configs/chips/stm32f401.yaml");
    let mut descriptor = ChipDescriptor::from_file(&chip_path).expect("load chip descriptor");
    descriptor.analog_pins.insert(
        "PA0".to_string(),
        labwired_config::AdcPinFn {
            peripheral: "adc1".to_string(),
            channel: 40,
        },
    );
    let manifest: SystemManifest = serde_yaml::from_str(&format!(
        "name: \"cosim-routing\"\nchip: \"{}\"\nexternal_devices: []\n",
        chip_path.display()
    ))
    .expect("parse manifest");
    let bus = SystemBus::from_config(&descriptor, &manifest).expect("build bus");

    let errors = output_binding_errors(&bus, "board.analog.pa0_volts");
    assert_eq!(
        errors,
        vec![RoutingError::NoSuchAdcChannel {
            path: "board.analog.pa0_volts".to_string(),
            peripheral: "adc1".to_string(),
            channel: 40,
            channels: 19,
        }]
    );
}

/// The top of the range is a real channel (IN18, VBAT on an F4): it binds, and
/// the routed volts land on it.
#[test]
fn the_last_channel_the_adc_has_is_routable() {
    let mut bus = f401_bus();
    let mut session = build_session(
        &bus,
        &[mock_model(
            100_000,
            &[],
            &[("v_out", "adc.adc1.18_volts")],
            &[("v_out", serde_yaml::Value::from(1.65))],
        )],
    );
    let (_, errors) = session.advance_to(8_400, &mut bus).expect("step");
    assert!(errors.is_empty(), "{errors:?}");
    assert_eq!(adc_channel_count(&mut bus, 18), 2047);
}

/// Every in-tree descriptor's `analog_pins:` names an ADC that exists on its
/// bus and a channel that ADC has, so the new check refuses no shipped pad.
#[test]
fn every_descriptor_analog_pin_names_a_channel_its_adc_has() {
    let mut checked = 0;
    let mut chips: Vec<_> = std::fs::read_dir(root("configs/chips"))
        .expect("read configs/chips")
        .map(|entry| entry.expect("dir entry").path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "yaml"))
        .filter(|path| {
            std::fs::read_to_string(path)
                .map(|text| text.contains("\nanalog_pins:"))
                .unwrap_or(false)
        })
        .collect();
    chips.sort();
    for chip_path in chips {
        let chip = chip_path
            .file_stem()
            .and_then(|stem| stem.to_str())
            .expect("chip file name")
            .to_string();
        let bus = chip_bus(&chip);
        let descriptor = ChipDescriptor::from_file(&chip_path).expect("load chip descriptor");
        assert!(!descriptor.analog_pins.is_empty(), "{chip}");
        let outputs: Vec<(String, String)> = descriptor
            .analog_pins
            .keys()
            .map(|pad| {
                (
                    format!("v_{}", pad.to_ascii_lowercase()),
                    format!("board.analog.{}_volts", pad.to_ascii_lowercase()),
                )
            })
            .collect();
        let routes: Vec<(&str, &str)> = outputs
            .iter()
            .map(|(name, path)| (name.as_str(), path.as_str()))
            .collect();
        let values: Vec<(&str, serde_yaml::Value)> = outputs
            .iter()
            .map(|(name, _)| (name.as_str(), serde_yaml::Value::from(1.0)))
            .collect();
        let session = CosimSession::new(
            &[mock_model(100_000, &[], &routes, &values)],
            Path::new("."),
            &bus,
        )
        .expect("build")
        .expect("models were declared");
        assert_eq!(
            session.binding_errors(),
            &[] as &[RoutingError],
            "{chip}: a shipped analog pad does not bind"
        );
        checked += descriptor.analog_pins.len();
    }
    assert!(
        checked > 50,
        "only {checked} pads checked; the scan found too little"
    );
}

/// The channel count a model reports is only a guard if its top channel really
/// takes an input. For each model with an injected-count readback, the last
/// reported channel must hold the level driven onto it. (RP2040 and EFR32
/// report the bound of their own input table, `INPUTS` and `channel_for`.)
#[test]
fn every_drivable_adc_takes_an_input_on_its_last_channel() {
    use labwired_core::peripherals::adc::{Adc, AdcRegisterLayout};
    use labwired_core::peripherals::esp32::sar_adc::Esp32SarAdc;
    use labwired_core::peripherals::esp32c3::apb_saradc::Esp32c3ApbSarAdc;
    use labwired_core::peripherals::esp32s3::sens::Esp32s3Sens;
    use labwired_core::Peripheral;

    for (layout, channels) in [
        (AdcRegisterLayout::Stm32F1, 19),
        (AdcRegisterLayout::Stm32L4, 19),
        (AdcRegisterLayout::Stm32H5, 19),
        (AdcRegisterLayout::Stm32H7, 20),
    ] {
        let mut adc = Adc::new_with_layout(layout);
        assert_eq!(adc.adc_channel_count(), Some(channels), "{layout:?}");
        adc.set_channel_input(channels - 1, 1650);
        assert_eq!(adc.channel_input_count(channels - 1), 2047, "{layout:?}");
    }

    let mut sens = Esp32s3Sens::new();
    let last = sens.adc_channel_count().expect("S3 SENS is an ADC") - 1;
    sens.set_channel_input(last, 1650);
    assert_eq!(sens.channel_input_count(last), 2047);

    let mut c3 = Esp32c3ApbSarAdc::default();
    let last = c3.adc_channel_count().expect("C3 APB_SARADC is an ADC") - 1;
    c3.set_channel_input(last, 1650);
    assert_eq!(c3.channel_input_count(last), 2047);

    let mut classic = Esp32SarAdc::new();
    let last = classic.adc_channel_count().expect("ESP32 SENS is an ADC") - 1;
    classic.set_channel_input(last, 1650);
    assert_eq!(classic.channel_input_count(last), 2047);
}
