// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! The capacitive-touch measurement, end to end on an ATmega328P.
//!
//! `CapacitiveSensor` raises its send pin and counts loop iterations until the
//! receive pin reads high, so the whole demo rests on one chain: the firmware
//! drives D4 (PD4) through a DDRD/PORTD write, the circuit sees a driver switch
//! close, a megohm charges the pad capacitance, and the pad voltage reaches D2
//! (PD2) as a digital level through the ATmega's input thresholds, at the right
//! simulated time.
//!
//! The circuit is the `cosim_models` block the playground's canvas compiler
//! emits for the touch lab, pasted verbatim: core must accept it as written.
//!
//! No committed AVR firmware toggles PD4 and reads PD2, so the firmware here is
//! a few hand-assembled instructions that do exactly that, run by the real AVR
//! core on a bus built from the shipped `atmega328p` descriptor.
//!
//! PD2 must first read high after t = R·C·ln(1 / (1 − 0.6)), VIH being 0.6·VCC
//! on this part: 1 MΩ × 20 pF gives 18.3 µs released, and with the finger's
//! 100 pF switched in through 1 kΩ, 1 MΩ × 120 pF gives 110 µs.

mod common;
use common::root;
use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::bus::SystemBus;
use labwired_core::cosim::routing::{cycles_to_ns, ns_to_cycles};
use labwired_core::cosim::{
    build_cosim_adapter_with_base, validate_analog_models, CosimInputKind, CosimRunner,
    CosimRunnerModel, CosimSession, CosimSignalValue, RoutingError,
};
use labwired_core::cpu::avr::Avr;
use labwired_core::{AdvanceRequest, Cpu, Machine};
use std::collections::BTreeMap;
use std::path::Path;

/// The canvas compiler's output for the touch lab, exactly as emitted.
const CANVAS_TOUCH_MODELS: &str = r#"
cosim_models:
  - id: "circuit"
    adapter: "analog"
    step_ns: 1000
    inputs:
      ctl_touch: "ui.touch.pressed"
      drv_pd2: "board.gpio.pd2"
      ctl_pd2: "board.gpio_output.pd2"
      drv_pd4: "board.gpio.pd4"
      ctl_pd4: "board.gpio_output.pd4"
    outputs:
      in_pd2: "board.gpio_in.pd2"
      in_pd4: "board.gpio_in.pd4"
    config:
      vdd: 5.0
      netlist_text: |
        RR1 v_net_mcu_d4 v_net_mcu_d2 1meg
        Ctouch_pad v_net_mcu_d2 0 20p
        Ctouch_finger f_touch 0 100p
        Stouch v_net_mcu_d2 f_touch ctl_touch ron=1k roff=1e12
        Vdrv_pd2 d_pd2 0 dc 0
        Sdrv_pd2 d_pd2 v_net_mcu_d2 ctl_pd2 ron=25 roff=1e12
        Vdrv_pd4 d_pd4 0 dc 0
        Sdrv_pd4 d_pd4 v_net_mcu_d4 ctl_pd4 ron=25 roff=1e12
      probes:
        in_pd2: "v(v_net_mcu_d2)"
        in_pd4: "v(v_net_mcu_d4)"
        v_net_mcu_d2: "v(v_net_mcu_d2)"
        v_net_mcu_d4: "v(v_net_mcu_d4)"
      sources:
        drv_pd2: "Vdrv_pd2"
        drv_pd4: "Vdrv_pd4"
"#;

/// ```text
/// 0x0000  SBI  DDRD,4     ; pinMode(4, OUTPUT)
/// 0x0002  SBI  PORTD,4    ; digitalWrite(4, HIGH) — the send edge
/// 0x0004  SBIS PIND,2     ; skip the jump once PD2 reads high
/// 0x0006  RJMP 0x0004     ; keep polling
/// 0x0008  IN   R16,PIND   ; what the firmware reads once it left the poll
/// 0x000A  RJMP 0x000A     ; park
/// ```
const PROGRAM: [u16; 6] = [0x9A54, 0x9A5C, 0x9B4A, 0xCFFE, 0xB109, 0xCFFF];
const POLL_PCS: [u32; 2] = [0x0004, 0x0006];
const PARKED_PC: u32 = 0x000A;

const R_OHMS: f64 = 1.0e6;
const RON_DRIVER_OHMS: f64 = 25.0;
const C_PAD_FARADS: f64 = 20.0e-12;
const C_FINGER_FARADS: f64 = 100.0e-12;
const VIH_RATIO: f64 = 0.6;

/// A minimal ATmega328P system carrying the canvas block.
fn touch_manifest() -> SystemManifest {
    let chip_path = root("configs/chips/atmega328p.yaml");
    let manifest: SystemManifest = serde_yaml::from_str(&format!(
        "name: \"touch-lab\"\nchip: \"{}\"\nexternal_devices: []\n{CANVAS_TOUCH_MODELS}",
        chip_path.display()
    ))
    .expect("the canvas block parses as a system manifest");
    assert_eq!(manifest.cosim_models.len(), 1);
    manifest
}

fn atmega_machine(manifest: &SystemManifest, program: &[u16]) -> Machine<Avr> {
    let descriptor = ChipDescriptor::from_file(root("configs/chips/atmega328p.yaml"))
        .expect("load chip descriptor");
    let bus = SystemBus::from_config(&descriptor, manifest).expect("build bus");
    let mut cpu = Avr::new();
    cpu.load_words(0, program);
    Machine::new(cpu, bus)
}

fn touch_session(manifest: &SystemManifest, machine: &Machine<Avr>) -> CosimSession {
    let session = CosimSession::new(&manifest.cosim_models, Path::new("."), &machine.bus)
        .expect("build session")
        .expect("a model was declared");
    assert_eq!(
        session.binding_errors(),
        &[] as &[RoutingError],
        "every path of the canvas block must resolve on the atmega328p"
    );
    session
}

/// The `portd` window's view of one PD pad, as co-simulation reads it:
/// (pad level, output latch, is an output).
fn portd_pad(machine: &Machine<Avr>, pad: u8) -> (Option<bool>, Option<bool>, Option<bool>) {
    let index = machine
        .bus
        .find_peripheral_index_by_name("portd")
        .expect("portd window");
    let dev = &machine.bus.peripherals[index].dev;
    (
        dev.read_gpio_input(pad),
        dev.read_gpio_output(pad),
        dev.read_gpio_is_output(pad),
    )
}

struct Crossing {
    /// Machine cycle at which the `SBI PORTD,4` that raises PD4 completed.
    send_edge: u64,
    /// Machine cycle at which the routed pad voltage first read high on PD2.
    pin_high: u64,
    /// Machine cycle of the advance in which the firmware left its poll.
    parked: u64,
    /// `IN R16, PIND` right after leaving the poll.
    pind: u8,
}

/// Run [`PROGRAM`] until the firmware has seen PD2 high.
fn run_until_pd2_reads_high(machine: &mut Machine<Avr>, session: &mut CosimSession) -> Crossing {
    // The two SBIs, one step each, so the edge is known to the cycle: the
    // machine clock charges the core's datasheet cycles, not one per
    // instruction. Both land well before the first 1 us boundary.
    machine.step().expect("SBI DDRD,4");
    machine.step().expect("SBI PORTD,4");
    let send_edge = machine.total_cycles;
    assert!(
        send_edge < ns_to_cycles(session.step_ns(), session.cpu_hz()),
        "the send edge must precede the first model boundary"
    );
    let mut pin_high = None;
    let mut saw_poll = false;
    while machine.total_cycles < 64_000 {
        let advance = session
            .advance(machine, AdvanceRequest::run(None))
            .expect("advance");
        assert!(
            advance.new_routing_errors.is_empty(),
            "{:?}",
            advance.new_routing_errors
        );
        let pc = machine.cpu.get_pc();
        saw_poll |= POLL_PCS.contains(&pc);
        if pin_high.is_none() && portd_pad(machine, 2).0 == Some(true) {
            pin_high = Some(machine.total_cycles);
        }
        if pin_high.is_none() {
            assert!(
                POLL_PCS.contains(&pc) || pc < POLL_PCS[0],
                "the firmware left the poll before PD2 rose (pc {pc:#06x})"
            );
        }
        if pc == PARKED_PC {
            assert!(saw_poll, "the firmware never polled PD2");
            return Crossing {
                send_edge,
                pin_high: pin_high.expect("parked, so PD2 read high"),
                parked: machine.total_cycles,
                pind: machine.cpu.r[16],
            };
        }
        assert!(
            pc < PARKED_PC,
            "the firmware ran off its program: pc {pc:#06x}"
        );
    }
    panic!("PD2 never read high in 4 ms");
}

fn assert_crossing_time(
    label: &str,
    crossing: &Crossing,
    session: &CosimSession,
    capacitance: f64,
) {
    let cpu_hz = session.cpu_hz();
    let delay_s = (cycles_to_ns(crossing.pin_high, cpu_hz)
        - cycles_to_ns(crossing.send_edge, cpu_hz)) as f64
        * 1e-9;
    let expected_s = (R_OHMS + RON_DRIVER_OHMS) * capacitance * (1.0 / (1.0 - VIH_RATIO)).ln();
    let error = (delay_s - expected_s).abs() / expected_s;
    println!(
        "{label}: PD2 high {:.3} us after the send edge (cycle {}), firmware out of its poll by \
         cycle {}; R*C*ln(1/(1-0.6)) = {:.3} us ({:.2} % off)",
        delay_s * 1e6,
        crossing.pin_high,
        crossing.parked,
        expected_s * 1e6,
        error * 100.0
    );
    assert!(
        error < 0.05,
        "{label}: PD2 read high {:.3} us after the send edge, expected {:.3} us within 5 %",
        delay_s * 1e6,
        expected_s * 1e6
    );
    // The level lands in PIND at a boundary; the firmware must be out of its
    // poll by the end of the next one.
    let boundary = ns_to_cycles(session.step_ns(), cpu_hz);
    assert!(
        crossing.parked > crossing.pin_high && crossing.parked <= crossing.pin_high + boundary,
        "{label}: the level reached PIND at cycle {} but the firmware only left its poll by \
         cycle {}",
        crossing.pin_high,
        crossing.parked
    );
}

/// The block validates and builds as written: the switch controls are logic
/// levels, the drivers are voltage sources, and the two probes no output routes
/// are traced under the model id rather than refused.
#[test]
fn the_canvas_touch_block_builds_as_written() {
    let manifest = touch_manifest();
    assert_eq!(manifest.validate_cosim_models(), Vec::<String>::new());
    assert_eq!(
        validate_analog_models(&manifest.cosim_models, Path::new(".")),
        Vec::<String>::new()
    );

    let config = manifest.cosim_models[0].clone();
    let adapter = build_cosim_adapter_with_base(&config, Path::new(".")).expect("adapter builds");
    for (input, kind) in [
        ("ctl_touch", CosimInputKind::Bool),
        ("ctl_pd2", CosimInputKind::Bool),
        ("ctl_pd4", CosimInputKind::Bool),
        ("drv_pd2", CosimInputKind::Number),
        ("drv_pd4", CosimInputKind::Number),
    ] {
        assert_eq!(adapter.input_kind(input), Some(kind), "{input}");
    }
    let runner = CosimRunner::new(vec![CosimRunnerModel::new(config, adapter)]);
    assert_eq!(
        runner
            .analog_channels()
            .iter()
            .map(|c| c.name.as_str())
            .collect::<Vec<_>>(),
        vec![
            "circuit.in_pd2",
            "circuit.in_pd4",
            "circuit.v_net_mcu_d2",
            "circuit.v_net_mcu_d4"
        ]
    );
}

/// Bool inputs drive the netlist the way the block means them: `ctl_pd4` true
/// closes `Sdrv_pd4` and `drv_pd4` true puts vdd on `Vdrv_pd4`, so the send
/// node sits at 5 V. The unrouted probes land in the trace ring every step and
/// never in the signal store.
#[test]
fn bool_inputs_close_switches_and_put_vdd_on_sources() {
    let manifest = touch_manifest();
    let mut runner = CosimRunner::from_configs(&manifest.cosim_models).expect("runner");
    let mut signals = BTreeMap::from([
        ("board.gpio.pd4".to_string(), CosimSignalValue::Bool(true)),
        (
            "board.gpio_output.pd4".to_string(),
            CosimSignalValue::Bool(true),
        ),
        (
            "board.gpio_output.pd2".to_string(),
            CosimSignalValue::Bool(false),
        ),
        (
            "ui.touch.pressed".to_string(),
            CosimSignalValue::Bool(false),
        ),
    ]);
    let routed = runner
        .step_until_with_signals(3_000, &mut signals)
        .expect("three 1 us steps");
    assert_eq!(routed.len(), 3);
    let in_pd4 = match signals.get("board.gpio_in.pd4") {
        Some(CosimSignalValue::F64(volts)) => *volts,
        other => panic!("in_pd4 was not routed: {other:?}"),
    };
    assert!((in_pd4 - 5.0).abs() < 1e-3, "send node at vdd: {in_pd4} V");
    assert!(
        !signals.keys().any(|path| path.contains("v_net_mcu")),
        "unrouted probes stay out of the store: {signals:?}"
    );

    let batch = runner.analog_trace_snapshot(0);
    let d4 = batch
        .channels
        .iter()
        .position(|c| c.name == "circuit.v_net_mcu_d4")
        .expect("unrouted probe channel");
    let newest = batch.samples.last().expect("samples");
    assert_eq!(newest.time_ns, 3_000);
    assert!(
        (f64::from(newest.values[d4]) - 5.0).abs() < 1e-3,
        "the unrouted probe is traced: {} V",
        newest.values[d4]
    );
}

#[test]
fn a_released_pad_reads_high_after_18_3_us() {
    let manifest = touch_manifest();
    let mut machine = atmega_machine(&manifest, &PROGRAM);
    let mut session = touch_session(&manifest, &machine);
    assert_eq!(
        session.signals().get("ui.touch.pressed"),
        Some(&CosimSignalValue::Bool(false)),
        "the finger starts released"
    );
    let crossing = run_until_pd2_reads_high(&mut machine, &mut session);
    assert_crossing_time("released", &crossing, &session, C_PAD_FARADS);

    // `in_pd4` writes the input latch of PD4, which the firmware drives. That
    // is harmless: the pad keeps its direction and latch, and PIND reads the
    // latch for an output bit.
    assert_eq!(
        portd_pad(&machine, 4),
        (Some(true), Some(true), Some(true)),
        "PD4: driven high, latch high, still an output"
    );
    assert_eq!(
        crossing.pind & 0x14,
        0x14,
        "PIND: PD2 high, PD4 its own latch"
    );
}

#[test]
fn a_pressed_pad_reads_high_after_110_us() {
    let manifest = touch_manifest();
    let mut machine = atmega_machine(&manifest, &PROGRAM);
    let mut session = touch_session(&manifest, &machine);
    session
        .set_signal_number("ui.touch.pressed", 1.0)
        .expect("the circuit reads ui.touch.pressed");
    let crossing = run_until_pd2_reads_high(&mut machine, &mut session);
    assert_crossing_time(
        "pressed",
        &crossing,
        &session,
        C_PAD_FARADS + C_FINGER_FARADS,
    );
}

/// An external level in PD4's input latch does not override the firmware's
/// driver: PIND reads the latch for an output bit, and the latch and direction
/// the circuit reads stay the firmware's.
#[test]
fn an_input_level_on_a_driven_pad_does_not_override_the_driver() {
    let manifest = touch_manifest();
    // SBI DDRD,4 (PD4 an output, latch low); IN R16,PIND; park.
    let mut machine = atmega_machine(&manifest, &[0x9A54, 0xB109, 0xCFFF]);
    let mut session = touch_session(&manifest, &machine);
    // Hold PD4's input latch high, as a stale `in_pd4` would.
    let portd = machine
        .bus
        .find_peripheral_index_by_name("portd")
        .expect("portd window");
    assert!(machine.bus.peripherals[portd].dev.set_gpio_input(4, true));
    while machine.total_cycles < 160 {
        session
            .advance(&mut machine, AdvanceRequest::run(None))
            .expect("advance");
    }
    assert_eq!(
        machine.cpu.r[16] & 0x10,
        0,
        "PIND bit 4 reads the low latch"
    );
    assert_eq!(
        portd_pad(&machine, 4),
        (Some(false), Some(false), Some(true)),
        "PD4 is an output driving low"
    );
}

/// With the send pin never made an output, the driver switch stays open and
/// the pad holds its charge: the direction path, not the output latch, decides
/// whether the firmware drives the net.
#[test]
fn a_send_pin_that_is_an_input_does_not_charge_the_pad() {
    let manifest = touch_manifest();
    // SBI PORTD,4 only: the latch is high, but DDRD never makes PD4 an output.
    let mut machine = atmega_machine(&manifest, &[0x9A5C, 0xCFFF]);
    let mut session = touch_session(&manifest, &machine);
    // 200 us is ten released time constants: a closed driver would have charged
    // the pad to 5 V and PD2 long since high.
    while machine.total_cycles < 3_200 {
        session
            .advance(&mut machine, AdvanceRequest::run(None))
            .expect("advance");
    }
    assert_eq!(
        portd_pad(&machine, 2).0,
        Some(false),
        "PD2 rose with PD4 configured as an input"
    );
}
