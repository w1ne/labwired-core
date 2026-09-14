//! The browser engine steps a manifest's `cosim_models` the way `labwired test`
//! does.
//!
//! Firmware is the committed NUCLEO-F401RE Arduino blink image the CLI routing
//! test uses: it drives PA5 (LD2) high at cycle ~6072 and holds it for 500 ms,
//! so the RC node below charges from a pin level the firmware really produced.

use crate::{SystemManifest, WasmSimulator};
use labwired_config::ChipDescriptor;
use labwired_core::analog::AnalogChannel;
use labwired_core::console::ConsoleCapture;
use labwired_core::cosim::CosimSignalValue;
use labwired_core::system::cortex_m::configure_cortex_m;
use labwired_core::{AdvanceRequest, Cpu, Machine};
use wasm_bindgen::JsValue;

const CHIP_YAML: &str = include_str!("../../../configs/chips/stm32f401.yaml");
const FIRMWARE: &[u8] = include_bytes!("../../../tests/fixtures/stm32f401-blinky.elf");

const PLAIN_SYSTEM: &str = r#"
name: "wasm-blinky"
chip: "stm32f401"
external_devices: []
"#;

/// 10 kOhm / 100 nF low-pass on PA5, output on PA0's ADC channel: tau = 1 ms.
const RC_SYSTEM: &str = r#"
name: "wasm-blinky-rc"
chip: "stm32f401"
external_devices: []
cosim_models:
  - id: rc
    adapter: analog
    step_ns: 100000
    inputs: { gpio: board.gpio.pa5 }
    outputs: { v_out: board.analog.pa0_volts }
    config:
      netlist_text: |
        * RC low-pass driven by PA5
        Vgpio in 0 dc 0
        R1 in out 10k
        C1 out 0 100n
      probes: { v_out: "v(out)" }
      sources: { gpio: Vgpio }
"#;

fn build(system_yaml: &str) -> WasmSimulator {
    WasmSimulator::new_from_config(system_yaml, CHIP_YAML, FIRMWARE, JsValue::NULL)
        .unwrap_or_else(|_| panic!("simulator builds from:\n{system_yaml}"))
}

fn manifest(system_yaml: &str) -> SystemManifest {
    serde_yaml::from_str(system_yaml).expect("parse system manifest")
}

/// 84 MHz, so 1 ms is 84 000 cycles.
const CYCLES_PER_MS: u64 = 84_000;

fn step_until(sim: &mut WasmSimulator, total_cycles: u64, batch: u32) {
    while sim.machine.as_ref().expect("machine").total_cycles < total_cycles {
        let executed = sim
            .step_batch(batch)
            .unwrap_or_else(|_| panic!("step_batch failed"));
        assert!(executed > 0, "step_batch made no progress");
    }
}

/// The RC demo, end to end through the browser entry points: the constructor
/// builds the session, `step_batch` steps it, and `analog_trace_snapshot`
/// reads a charge curve driven by the pin the firmware set.
#[test]
fn step_batch_charges_the_rc_node_from_the_firmware_driven_pin() {
    let mut sim = build(RC_SYSTEM);
    assert!(sim.cosim.is_some(), "cosim_models must build a session");

    // A step_batch budget is spent in full across model boundaries, not cut
    // short at the first one (8400 cycles).
    let executed = sim.step_batch(50_000).unwrap_or_else(|_| panic!("step"));
    assert!(
        executed >= 50_000,
        "step_batch(50000) stopped at {executed} cycles"
    );
    step_until(&mut sim, 2 * CYCLES_PER_MS + CYCLES_PER_MS / 10, 20_000);

    let first = sim.analog_trace_batch(0);
    assert_eq!(first.channels, vec![AnalogChannel::volts("rc.v_out")]);
    assert!(
        first.samples.len() > 15,
        "expected > 15 samples over 2 ms, got {}",
        first.samples.len()
    );

    // Row 0 is the t = 0 operating point. The model sees PA5 high at the first
    // boundary whose sample says so; the interval it integrates with the
    // source high starts at the boundary before, which is the edge in model
    // time.
    let v_out: Vec<(u64, f32)> = first
        .samples
        .iter()
        .map(|s| (s.time_ns, s.values[0]))
        .collect();
    let edge = v_out
        .iter()
        .rposition(|(_, v)| *v == 0.0)
        .expect("the node starts at 0 V");
    assert!(edge + 1 < v_out.len(), "the node never started charging");
    let edge_ns = v_out[edge].0;
    for pair in v_out[edge..].windows(2) {
        assert!(
            pair[1].1 >= pair[0].1,
            "v_out fell while PA5 was high: {:?} -> {:?}",
            pair[0],
            pair[1]
        );
    }
    let tau_ns = 1_000_000;
    let (_, at_tau) = *v_out
        .iter()
        .find(|(t, _)| *t == edge_ns + tau_ns)
        .unwrap_or_else(|| panic!("no sample at edge + tau in {v_out:?}"));
    let expected = 3.3 * (1.0 - (-1.0f64).exp());
    let error = (f64::from(at_tau) - expected).abs() / expected;
    println!(
        "{} samples; edge at {edge_ns} ns; v_out(edge + tau) = {at_tau} V vs {expected:.4} V ({:.3} % off)",
        v_out.len(),
        error * 100.0
    );
    assert!(
        error < 0.03,
        "v_out at one tau is {at_tau} V, expected {expected:.4} V ({:.2} % off)",
        error * 100.0
    );

    // The cursor contract: nothing new without a step, only new rows after one.
    let again = sim.analog_trace_batch(first.next_cursor);
    assert!(again.samples.is_empty(), "no step, no new samples");
    let last_ns = v_out.last().expect("samples").0;
    let half_ms_later = sim.machine.as_ref().expect("machine").total_cycles + CYCLES_PER_MS / 2;
    step_until(&mut sim, half_ms_later, 20_000);
    let second = sim.analog_trace_batch(first.next_cursor);
    assert!(
        !second.samples.is_empty(),
        "half a millisecond more must add samples"
    );
    assert!(
        second.samples.iter().all(|s| s.time_ns > last_ns),
        "the second read repeated old samples"
    );
    assert_eq!(
        second.next_cursor,
        first.next_cursor + second.samples.len() as u64
    );
    assert_eq!(second.dropped, 0);
}

/// `mock` needs no process and no file, so it runs in the browser too: its
/// static output reaches the input register the firmware samples.
#[test]
fn a_mock_model_routes_onto_a_gpio_input() {
    const MOCK_SYSTEM: &str = r#"
name: "wasm-blinky-mock"
chip: "stm32f401"
external_devices: []
cosim_models:
  - id: contact
    adapter: mock
    step_ns: 100000
    inputs: { led: board.gpio.pa5 }
    outputs: { contact: board.gpio_in.pc13 }
    config:
      outputs: { contact: true }
"#;
    let mut sim = build(MOCK_SYSTEM);
    assert!(sim.cosim.is_some());
    step_until(&mut sim, CYCLES_PER_MS, 20_000);
    // GPIOC IDR on an F401 is 0x40020810; bit 13 is PC13.
    let idr = sim
        .read_memory(0x4002_0810, 4)
        .unwrap_or_else(|_| panic!("GPIOC IDR is mapped"));
    assert_ne!(
        u32::from_le_bytes(idr.try_into().expect("4 bytes")) & (1 << 13),
        0,
        "the mock output never reached PC13"
    );
}

fn attach_error(system_yaml: &str) -> String {
    let mut sim = build(PLAIN_SYSTEM);
    sim.attach_cosim(&manifest(system_yaml))
        .expect_err("the browser must refuse this manifest")
}

#[test]
fn external_process_is_refused_with_a_browser_error() {
    let error = attach_error(
        r#"
name: "wasm-ext"
chip: "stm32f401"
cosim_models:
  - id: plant
    adapter: external_process
    model: ./models/plant.py
    step_ns: 100000
    inputs: { gpio: board.gpio.pa5 }
    outputs: {}
"#,
    );
    assert!(
        error.contains(
            "external_process co-simulation needs a native build; use adapter: analog in the browser"
        ),
        "unexpected error: {error}"
    );
    assert!(
        error.contains("'plant'"),
        "the error must name the model: {error}"
    );
}

#[test]
fn an_analog_netlist_file_is_refused_with_a_browser_error() {
    let error = attach_error(
        r#"
name: "wasm-netlist-file"
chip: "stm32f401"
cosim_models:
  - id: rc
    adapter: analog
    step_ns: 100000
    inputs: { gpio: board.gpio.pa5 }
    outputs: { v_out: board.analog.pa0_volts }
    config:
      netlist: ./rc.cir
      probes: { v_out: "v(out)" }
      sources: { gpio: Vgpio }
"#,
    );
    assert!(
        error.contains("the browser has no filesystem; put the netlist inline as netlist_text"),
        "unexpected error: {error}"
    );
    assert!(
        error.contains("'rc'"),
        "the error must name the model: {error}"
    );
}

/// Same rule as `labwired test`: a path that does not resolve refuses to start
/// rather than running a co-simulation whose pin never reached the firmware.
#[test]
fn an_unroutable_pad_is_a_constructor_error() {
    let error = attach_error(
        r#"
name: "wasm-bad-pad"
chip: "stm32f401"
cosim_models:
  - id: probe
    adapter: mock
    step_ns: 100000
    inputs: { led: board.gpio.pz9 }
    outputs: {}
    config:
      outputs: {}
"#,
    );
    assert!(
        error.contains("pad 'pz9' does not resolve"),
        "unexpected error: {error}"
    );
}

/// An ADC route is checked against the converter when the simulator is built:
/// a channel the ADC does not have, an ADC the chip does not have, and a
/// peripheral that is not an ADC are each refused by name instead of stepping
/// a model whose volts land nowhere.
#[test]
fn an_adc_route_the_chip_cannot_take_is_a_constructor_error() {
    for (path, detail) in [
        ("adc.adc1.99_volts", "has channels 0..=18"),
        ("adc.nope.0_volts", "no peripheral named 'nope'"),
        ("adc.uart2.0_volts", "'uart2' is not an ADC"),
    ] {
        let error = attach_error(&format!(
            r#"
name: "wasm-bad-adc"
chip: "stm32f401"
cosim_models:
  - id: probe
    adapter: mock
    step_ns: 100000
    inputs: {{}}
    outputs: {{ v: {path} }}
    config:
      outputs: {{ v: 1.65 }}
"#
        ));
        assert!(
            error.contains(path) && error.contains(detail),
            "{path}: unexpected error: {error}"
        );
    }
}

/// The zero-change guarantee. With no `cosim_models:` there is no session, and
/// `step_batch` retires exactly what a bare `Machine::advance` — the path it
/// took before co-simulation existed — retires, batch for batch.
#[test]
fn a_manifest_without_cosim_models_steps_exactly_as_before() {
    const BATCH: u32 = 25_000;
    const BATCHES: usize = 40;

    let mut sim = build(PLAIN_SYSTEM);
    assert!(sim.cosim.is_none(), "no cosim_models, no session");
    assert!(!sim
        .machine
        .as_ref()
        .expect("machine")
        .analog_trace_attached());

    // The pre-cosim construction and stepping, written out against core.
    let chip: ChipDescriptor = serde_yaml::from_str(CHIP_YAML).expect("chip");
    let plain = manifest(PLAIN_SYSTEM);
    let mut bus = labwired_core::bus::SystemBus::from_config(&chip, &plain).expect("bus");
    let console = ConsoleCapture::for_manifest(&plain);
    let reference_uart = console.heard_sink();
    bus.attach_host_console(console.tapped(), reference_uart.clone())
        .expect("console");
    let (cpu, _nvic) = configure_cortex_m(&mut bus);
    let boxed: Box<dyn Cpu> = Box::new(cpu);
    let mut reference = Machine::new(boxed, bus);
    reference
        .load_firmware(&labwired_loader::load_elf_bytes(FIRMWARE).expect("elf"))
        .expect("load");

    let mut sim_uart = Vec::new();
    for batch in 0..BATCHES {
        let executed = sim.step_batch(BATCH).unwrap_or_else(|_| panic!("step"));
        let before = reference.total_cycles;
        reference
            .advance(AdvanceRequest::run(Some(u64::from(BATCH))))
            .expect("reference advance");
        assert_eq!(
            u64::from(executed),
            reference.total_cycles - before,
            "batch {batch}: cycle count diverged"
        );
        let machine = sim.machine.as_ref().expect("machine");
        assert_eq!(
            machine.total_cycles, reference.total_cycles,
            "batch {batch}"
        );
        assert_eq!(
            machine.cpu.get_pc(),
            reference.cpu.get_pc(),
            "batch {batch}: PC"
        );
        sim_uart.extend(sim.drain_uart_output());
    }
    let reference_uart = reference_uart.lock().expect("uart").clone();
    assert!(
        String::from_utf8_lossy(&sim_uart).contains("LED ON"),
        "the run should reach the sketch loop: {:?}",
        String::from_utf8_lossy(&sim_uart)
    );
    assert_eq!(sim_uart, reference_uart, "UART bytes diverged");
}

/// `step_batch` throughput with and without the RC model, same batch size.
///
/// Ignored because a number is only meaningful from a release build:
///
/// ```text
/// cargo test -p labwired-wasm --release --lib cosim_tests::step_batch_throughput \
///     -- --ignored --nocapture
/// ```
#[test]
#[ignore = "throughput measurement; run in release with --ignored --nocapture"]
fn step_batch_throughput() {
    const BATCH: u32 = 100_000;
    const TOTAL_CYCLES: u64 = 84_000_000; // one simulated second at 84 MHz
    const ROUNDS: usize = 5;

    for (label, system) in [("no cosim", PLAIN_SYSTEM), ("rc model", RC_SYSTEM)] {
        let mut rates = Vec::with_capacity(ROUNDS);
        for _ in 0..ROUNDS {
            let mut sim = build(system);
            let mut cycles = 0u64;
            let start = std::time::Instant::now();
            while cycles < TOTAL_CYCLES {
                let executed = sim.step_batch(BATCH).unwrap_or_else(|_| panic!("step"));
                assert!(executed > 0, "{label}: step_batch made no progress");
                cycles += u64::from(executed);
            }
            rates.push(cycles as f64 / start.elapsed().as_secs_f64());
        }
        rates.sort_by(f64::total_cmp);
        println!(
            "step_batch({BATCH}) {label}: median {:.2} MIPS (min {:.2}, max {:.2}) over {ROUNDS} runs",
            rates[ROUNDS / 2] / 1e6,
            rates[0] / 1e6,
            rates[ROUNDS - 1] / 1e6,
        );
    }
}

// ── Set from outside: `set_cosim_signal` ────────────────────────────────────

/// A lab whose only model input is set from outside the engine. The source
/// puts the signal's volts on a pad node the ADC of PA0 converts.
const UI_SYSTEM: &str = r#"
name: "wasm-ui-signal"
chip: "stm32f401"
external_devices: []
cosim_models:
  - id: bench
    adapter: analog
    step_ns: 100000
    inputs: { level: ui.bench.level, ctl_touch: ui.touch.pressed }
    outputs: { v_pad: board.analog.pa0_volts }
    config:
      netlist_text: |
        Vbench in 0 dc 0
        R1 in pad 1k
        Stouch pad 0 ctl_touch ron=1k roff=1e12
      probes: { v_pad: "v(pad)" }
      sources: { level: Vbench }
"#;

#[test]
fn set_cosim_signal_types_the_number_by_the_input_it_feeds() {
    let mut sim = build(UI_SYSTEM);
    let signals = |sim: &WasmSimulator| sim.cosim.as_ref().expect("session").signals().clone();
    assert_eq!(
        signals(&sim).get("ui.touch.pressed"),
        Some(&CosimSignalValue::Bool(false)),
        "a ui input exists, released, before anything is set"
    );

    sim.set_cosim_signal_number("ui.touch.pressed", 1.0)
        .expect("a model reads ui.touch.pressed");
    sim.set_cosim_signal_number("ui.bench.level", 2.5)
        .expect("a model reads ui.bench.level");
    assert_eq!(
        signals(&sim).get("ui.touch.pressed"),
        Some(&CosimSignalValue::Bool(true)),
        "1 on a switch control is a press"
    );
    assert_eq!(
        signals(&sim).get("ui.bench.level"),
        Some(&CosimSignalValue::F64(2.5)),
        "a source takes the number as volts"
    );

    // The model reads both at its next step: 2.5 V through 1 k into 1 k.
    step_until(&mut sim, CYCLES_PER_MS, 20_000);
    let v_pad = sim
        .analog_trace_batch(0)
        .samples
        .last()
        .expect("samples")
        .values[0];
    assert!((f64::from(v_pad) - 1.25).abs() < 0.01, "v_pad = {v_pad} V");
}

#[test]
fn set_cosim_signal_refuses_what_it_cannot_deliver() {
    let mut sim = build(UI_SYSTEM);
    let typo = sim
        .set_cosim_signal_number("ui.touch.presed", 1.0)
        .expect_err("no model reads the typo");
    assert!(typo.contains("no model input reads it"), "{typo}");
    let board = sim
        .set_cosim_signal_number("board.gpio.pa5", 1.0)
        .expect_err("the machine owns board paths");
    assert!(board.contains("belongs to the machine"), "{board}");
    let nan = sim
        .set_cosim_signal_number("ui.bench.level", f64::NAN)
        .expect_err("NaN is not a level");
    assert!(nan.contains("not a finite number"), "{nan}");

    let mut plain = build(PLAIN_SYSTEM);
    let none = plain
        .set_cosim_signal_number("ui.touch.pressed", 1.0)
        .expect_err("no session, nothing to set");
    assert!(none.contains("declares no cosim_models"), "{none}");
}

// ── Arduino Nano (ATmega328P) in the browser ───────────────────────────────

const AVR_CHIP_YAML: &str = include_str!("../../../configs/chips/atmega328p.yaml");
/// `Serial.println("nano-ok")` in `setup()`, then blinks PB5 (D13) every 1 ms
/// and prints a `.` per loop.
const NANO_FIRMWARE: &[u8] = include_bytes!("../../../tests/fixtures/avr/arduino-nano-blinky.elf");
/// 16 MHz.
const NANO_CYCLES_PER_MS: u64 = 16_000;

fn build_nano(system_yaml: &str) -> WasmSimulator {
    WasmSimulator::new_from_config(system_yaml, AVR_CHIP_YAML, NANO_FIRMWARE, JsValue::NULL)
        .unwrap_or_else(|_| panic!("Nano simulator builds from:\n{system_yaml}"))
}

/// The Nano's `Serial` goes through USART0 on the AVR core, not a bus UART, so
/// the Serial pane only hears it if the constructor hands the CPU the sink
/// `drain_uart_output` drains.
#[test]
fn avr_serial_output_reaches_drain_uart_output() {
    let mut sim = build_nano("name: \"wasm-nano\"\nchip: \"atmega328p\"\nexternal_devices: []\n");
    let mut heard = Vec::new();
    for _ in 0..200 {
        sim.step_batch(20_000).unwrap_or_else(|_| panic!("step"));
        heard.extend(sim.drain_uart_output());
        if String::from_utf8_lossy(&heard).contains("nano-ok") {
            break;
        }
    }
    let text = String::from_utf8_lossy(&heard);
    assert!(
        text.contains("nano-ok"),
        "Serial.println never reached the pane: {text:?}"
    );
    assert!(
        sim.drain_uart_output().len() < heard.len(),
        "drain hands each byte over once"
    );
}

/// The Nano steps through the same co-simulation advance as the ARM boards:
/// the blink level on PB5 reaches the model, and a signal set from outside
/// reaches PD2 through the ATmega's input thresholds.
#[test]
fn an_avr_lab_steps_its_models_in_the_browser() {
    const NANO_BENCH: &str = r#"
name: "wasm-nano-bench"
chip: "atmega328p"
external_devices: []
cosim_models:
  - id: bench
    adapter: analog
    step_ns: 100000
    inputs: { level: ui.bench.level, led: board.gpio.pb5 }
    outputs: { in_pd2: board.gpio_in.pd2 }
    config:
      vdd: 5.0
      netlist_text: |
        Vbench in 0 dc 0
        R1 in pad 1k
        R2 pad 0 1meg
      probes: { in_pd2: "v(pad)" }
      sources: { level: Vbench }
"#;
    let mut sim = build_nano(NANO_BENCH);
    assert!(sim.cosim.is_some(), "an AVR lab builds a session");
    // PIND as the bus-side PORTD window holds it.
    let pind = |sim: &WasmSimulator| {
        sim.read_memory(0x0001_0029, 1)
            .unwrap_or_else(|_| panic!("PORTD window is mapped"))[0]
    };

    let mut saw_led = [false, false];
    while sim.machine.as_ref().expect("machine").total_cycles < 20 * NANO_CYCLES_PER_MS {
        sim.step_batch(4_000).unwrap_or_else(|_| panic!("step"));
        if let Some(CosimSignalValue::Bool(level)) = sim
            .cosim
            .as_ref()
            .expect("session")
            .signals()
            .get("board.gpio.pb5")
        {
            saw_led[usize::from(*level)] = true;
        }
    }
    assert_eq!(saw_led, [true, true], "the model saw PB5 blink");
    assert_eq!(pind(&sim) & 0x04, 0, "PD2 reads low with the bench at 0 V");

    sim.set_cosim_signal_number("ui.bench.level", 5.0)
        .expect("a model reads ui.bench.level");
    let target = sim.machine.as_ref().expect("machine").total_cycles + NANO_CYCLES_PER_MS;
    step_until(&mut sim, target, 4_000);
    assert_eq!(pind(&sim) & 0x04, 0x04, "5 V on the pad reads high on PD2");
}

// ── Capacitive touch lab throughput ────────────────────────────────────────

/// The capacitive touch lab exactly as the playground ships it: an Arduino Nano
/// running CapacitiveSensor against the lab's own `cosim_models` block, whose
/// 1 us step puts a model boundary every 16 AVR cycles.
const TOUCH_LAB_SYSTEM: &str = include_str!("../../../examples/capacitive-touch-lab/system.yaml");
const TOUCH_LAB_FIRMWARE: &[u8] =
    include_bytes!("../../../tests/fixtures/avr/capacitive-touch-lab.elf");

/// `step_batch` throughput of the touch lab, with and without its circuit.
///
/// One simulated second per round; the pad is pressed half-way through, as in
/// the lab's `test.yaml`, so both the released and the pressed waveform are in
/// the number. Ignored because a number is only meaningful from a release
/// build:
///
/// ```text
/// cargo test -p labwired-wasm --release --lib cosim_tests::touch_lab_step_batch_throughput \
///     -- --ignored --nocapture
/// ```
#[test]
#[ignore = "throughput measurement; run in release with --ignored --nocapture"]
fn touch_lab_step_batch_throughput() {
    const BATCH: u32 = 100_000;
    const TOTAL_CYCLES: u64 = 16_000_000; // one simulated second at 16 MHz
    const ROUNDS: usize = 3;

    let mut plain: serde_yaml::Value =
        serde_yaml::from_str(TOUCH_LAB_SYSTEM).expect("parse the touch lab manifest");
    plain
        .as_mapping_mut()
        .expect("manifest is a mapping")
        .remove("cosim_models");
    let plain = serde_yaml::to_string(&plain).expect("re-serialise the manifest");

    for (label, system) in [
        ("no cosim", plain.as_str()),
        ("touch lab", TOUCH_LAB_SYSTEM),
    ] {
        let mut rates = Vec::with_capacity(ROUNDS);
        for round in 0..ROUNDS {
            if let Ok(load) = std::fs::read_to_string("/proc/loadavg") {
                println!("touch lab {label} round {round}: load {}", load.trim());
            }
            let mut sim = WasmSimulator::new_from_config(
                system,
                AVR_CHIP_YAML,
                TOUCH_LAB_FIRMWARE,
                JsValue::NULL,
            )
            .unwrap_or_else(|_| panic!("{label}: the touch lab builds"));
            let has_circuit = sim.cosim.is_some();
            let mut cycles = 0u64;
            let mut pressed = false;
            let start = std::time::Instant::now();
            while cycles < TOTAL_CYCLES {
                if has_circuit && !pressed && cycles >= TOTAL_CYCLES / 2 {
                    sim.set_cosim_signal_number("ui.touch.pressed", 1.0)
                        .expect("the circuit reads ui.touch.pressed");
                    pressed = true;
                }
                let executed = sim.step_batch(BATCH).unwrap_or_else(|_| panic!("step"));
                assert!(executed > 0, "{label}: step_batch made no progress");
                cycles += u64::from(executed);
            }
            rates.push(cycles as f64 / start.elapsed().as_secs_f64());
            let _ = sim.drain_uart_output();
        }
        rates.sort_by(f64::total_cmp);
        println!(
            "touch lab step_batch({BATCH}) {label}: median {:.2} Mcycles/s ({:.3}x real time; \
             min {:.2}, max {:.2}) over {ROUNDS} runs",
            rates[ROUNDS / 2] / 1e6,
            rates[ROUNDS / 2] / 16e6,
            rates[0] / 1e6,
            rates[ROUNDS - 1] / 1e6,
        );
    }
}
