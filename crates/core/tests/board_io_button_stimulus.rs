// A button on the canvas must be drivable from a stimulus track.
//
// Buttons are the one input family the canvas emits WITHOUT an
// `external_devices` entry — a passive contact needs no device block, so it is
// declared only as a `board_io` binding. Until that binding materialised as a
// bus-resident device, a button was inert in a headless run: it drove no pin,
// exposed no channel, and every stimulus naming it was rejected as an unknown
// channel. That made "press the button and prove the LED toggles" — the most
// common beginner circuit there is — impossible to prove on the twin.
//
// This test pins the whole path end to end on a real chip: discovery
// (`list_inputs`), dispatch (`set_input` / `set_input_on`), and the pin level
// the firmware actually samples through the GPIO input register.

use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::bus::SystemBus;
use labwired_core::Bus;
use std::path::PathBuf;

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf()
}

/// An STM32F103 with one active-low button on PC13 — the exact shape the canvas
/// compiler emits for a push button wired pin→PC13, other terminal→GND.
fn f103_with_button(active_high: bool) -> SystemBus {
    let chip_path = workspace_root().join("configs/chips/stm32f103.yaml");
    let chip = ChipDescriptor::from_file(&chip_path).expect("load stm32f103 chip");
    let manifest_yaml = format!(
        r#"
name: "f103-button"
chip: "{}"
external_devices: []
board_io:
  - id: "btn_pc13"
    kind: "button"
    peripheral: "gpioc"
    pin: 13
    signal: "input"
    active_high: {active_high}
"#,
        chip_path.display()
    );
    let manifest: SystemManifest = serde_yaml::from_str(&manifest_yaml).expect("parse manifest");
    SystemBus::from_config(&chip, &manifest).expect("build bus")
}

/// The level the firmware would read on PC13, sampled the same way it does —
/// through the GPIO input register.
fn pc13_high(bus: &mut SystemBus) -> bool {
    let (addr, bit) = SystemBus::resolve_pin_idr_pub(bus, "PC13").expect("PC13 resolves to an IDR");
    bus.read_u32(addr).expect("read IDR") >> bit & 1 != 0
}

/// One peripheral tick — the pass that services bus-resident devices.
fn settle(bus: &mut SystemBus) {
    bus.tick_peripherals_fully();
}

#[test]
fn a_board_io_button_is_discoverable_as_a_drivable_channel() {
    let mut bus = f103_with_button(false);
    let inputs = bus.list_inputs();
    let found: Vec<_> = inputs
        .iter()
        .filter(|(owner, ch)| owner == "btn_pc13" && ch.key == "pressed")
        .collect();
    assert_eq!(
        found.len(),
        1,
        "the button must appear exactly once in list_inputs; got {inputs:?}"
    );
}

#[test]
fn an_active_low_button_holds_its_pin_high_until_pressed() {
    let mut bus = f103_with_button(false);
    settle(&mut bus);
    assert!(
        pc13_high(&mut bus),
        "a released pull-up button must drive its pin HIGH — the IDR resets to 0, \
         so an unattached button leaves a phantom press the firmware never sees released"
    );

    bus.set_input(Some("btn_pc13"), "pressed", 1.0)
        .expect("press the button");
    settle(&mut bus);
    assert!(!pc13_high(&mut bus), "pressing must pull PC13 LOW");

    bus.set_input(Some("btn_pc13"), "pressed", 0.0)
        .expect("release the button");
    settle(&mut bus);
    assert!(pc13_high(&mut bus), "releasing must return PC13 to HIGH");
}

#[test]
fn an_active_high_button_is_the_mirror_image() {
    let mut bus = f103_with_button(true);
    settle(&mut bus);
    assert!(!pc13_high(&mut bus), "released pull-down button reads LOW");

    bus.set_input(Some("btn_pc13"), "pressed", 1.0)
        .expect("press");
    settle(&mut bus);
    assert!(pc13_high(&mut bus), "pressing must pull PC13 HIGH");
}

#[test]
fn the_channel_resolves_without_naming_the_component_when_it_is_unique() {
    // A stimulus track that says `pressed` with no `component` must work when
    // there is only one button — the agent should not be forced to disambiguate
    // something that is unambiguous.
    let mut bus = f103_with_button(false);
    bus.set_input(None, "pressed", 1.0)
        .expect("bare channel resolves to the only button");
    settle(&mut bus);
    assert!(!pc13_high(&mut bus));
}

#[test]
fn driving_an_unknown_button_is_a_typed_error_not_a_silent_no_op() {
    use labwired_core::sim_input::SimInputError;
    let mut bus = f103_with_button(false);
    let err = bus
        .set_input(Some("btn_pa0"), "pressed", 1.0)
        .expect_err("no such button");
    assert!(
        matches!(err, SimInputError::NoDevice(_)),
        "expected NoDevice, got {err:?}"
    );
}

#[test]
fn an_out_of_range_press_is_rejected_and_leaves_the_pin_alone() {
    let mut bus = f103_with_button(false);
    settle(&mut bus);
    assert!(bus.set_input(Some("btn_pc13"), "pressed", 5.0).is_err());
    settle(&mut bus);
    assert!(
        pc13_high(&mut bus),
        "a rejected stimulus must not move the pin"
    );
}

#[test]
fn a_board_io_output_binding_is_not_a_stimulus_target() {
    // `kind: button, signal: output` is not a contact the firmware samples —
    // attaching one would invent a drive point that does not exist.
    let chip_path = workspace_root().join("configs/chips/stm32f103.yaml");
    let chip = ChipDescriptor::from_file(&chip_path).expect("load chip");
    let manifest_yaml = format!(
        r#"
name: "f103-led"
chip: "{}"
external_devices: []
board_io:
  - id: "led_pa5"
    kind: "led"
    peripheral: "gpioa"
    pin: 5
    signal: "output"
    active_high: true
"#,
        chip_path.display()
    );
    let manifest: SystemManifest = serde_yaml::from_str(&manifest_yaml).expect("parse manifest");
    let mut bus = SystemBus::from_config(&chip, &manifest).expect("build bus");
    assert!(
        bus.list_inputs()
            .iter()
            .all(|(owner, _)| owner != "led_pa5"),
        "an LED output must not become a drivable input"
    );
}

/// The ESP32 family models GPIO as ONE matrix peripheral rather than per-port
/// register blocks, so a button there cannot be anchored to a port's input
/// register. It is anchored to the peripheral instead and driven through the
/// same `set_gpio_input` seam — this test is the guard that the two chip shapes
/// stay reachable from one attach path.
#[test]
fn a_button_is_drivable_on_a_matrix_gpio_chip_too() {
    let chip_path = workspace_root().join("configs/chips/esp32c3.yaml");
    let chip = ChipDescriptor::from_file(&chip_path).expect("load esp32c3 chip");
    let manifest_yaml = format!(
        r#"
name: "c3-button"
chip: "{}"
external_devices: []
board_io:
  - id: "btn_gpio2"
    kind: "button"
    peripheral: "gpio"
    pin: 2
    signal: "input"
    active_high: false
"#,
        chip_path.display()
    );
    let manifest: SystemManifest = serde_yaml::from_str(&manifest_yaml).expect("parse manifest");
    let mut bus = SystemBus::from_config(&chip, &manifest).expect("build bus");

    assert!(
        bus.list_inputs()
            .iter()
            .any(|(owner, ch)| owner == "btn_gpio2" && ch.key == "pressed"),
        "the button must be discoverable on a matrix-GPIO chip"
    );

    let gpio = bus
        .find_peripheral_index_by_name("gpio")
        .expect("c3 registers a gpio peripheral");
    let level = |bus: &mut SystemBus| bus.peripherals[gpio].dev.read_gpio_input(2);

    settle(&mut bus);
    assert_eq!(level(&mut bus), Some(true), "released pull-up reads HIGH");

    bus.set_input(Some("btn_gpio2"), "pressed", 1.0)
        .expect("press");
    settle(&mut bus);
    assert_eq!(level(&mut bus), Some(false), "pressing pulls GPIO2 LOW");
}

// ---------------------------------------------------------------------------
// End-to-end: real firmware, real board, the printed byte moves.
//
// Everything above samples the pin directly. This one closes the loop on the
// artifact that actually matters — a compiled binary reading IDR and printing
// what it found — because a pin level nothing observes proves nothing.
// ---------------------------------------------------------------------------

/// Run `nucleo-l476rg-demo.elf` on the real NUCLEO-L476RG manifest, optionally
/// pressing B1 before the first instruction, and return what it printed.
///
/// Mirrors `firmware_survival::run_cortex_m_firmware`; kept separate because
/// that harness has no seam for pre-run stimulus, and widening it for one case
/// would put a stimulus hook on 50 cases that must never have one.
fn run_l476_demo(press_b1: bool) -> String {
    use labwired_core::system::cortex_m::configure_cortex_m;
    use labwired_core::Machine;
    use std::sync::{Arc, Mutex};

    let root = workspace_root();
    let chip = ChipDescriptor::from_file(root.join("configs/chips/stm32l476.yaml"))
        .expect("load stm32l476 chip");
    let sys_path = root.join("configs/systems/nucleo-l476rg.yaml");
    let mut manifest = SystemManifest::from_file(&sys_path).expect("load nucleo-l476rg system");
    manifest.chip = sys_path
        .parent()
        .unwrap()
        .join(&manifest.chip)
        .to_str()
        .unwrap()
        .to_string();

    let mut bus = SystemBus::from_config(&chip, &manifest).expect("build bus");
    let uart = Arc::new(Mutex::new(Vec::new()));
    bus.attach_uart_tx_sink(uart.clone(), false);

    if press_b1 {
        bus.set_input(Some("button_b1"), "pressed", 1.0)
            .expect("B1 is drivable from the board_io binding alone");
    }

    let (cpu, _nvic) = configure_cortex_m(&mut bus);
    let mut machine = Machine::new(cpu, bus);
    let image = labwired_loader::load_elf(&root.join("tests/fixtures/nucleo-l476rg-demo.elf"))
        .expect("load demo elf");
    machine.load_firmware(&image).expect("load firmware");
    for _ in 0..600_000 {
        machine.step().expect("demo firmware runs clean");
    }

    let bytes = uart.lock().unwrap().clone();
    String::from_utf8_lossy(&bytes).into_owned()
}

/// A released B1 must read HIGH, exactly as real silicon does through the R34
/// pull-up to VDD.
///
/// The demo prints `BTN=` from `(GPIOC_IDR >> 13) & 1 == 0` — active-low, so
/// `BTN=1` means *pressed*. Before the board_io button was a real device, PC13
/// floated at the reset-value 0 and the firmware printed `BTN=1` on an
/// untouched board: the simulator reported a phantom press. `firmware-l476-demo`
/// documented that as a known gap in its own source. It is closed here, so this
/// asserts the *silicon* answer, not the old simulated one.
#[test]
fn the_l476_demo_reads_its_released_user_button_as_silicon_does() {
    let out = run_l476_demo(false);
    assert!(
        out.contains("BTN=0\r\n"),
        "a released active-low B1 must read HIGH (BTN=0); got:\n{out}"
    );
}

/// ...and pressing it flips the byte the firmware prints.
///
/// This is the whole point of B5: a stimulus track naming the button on the
/// canvas changes what compiled firmware observes. Same binary, same board, one
/// `set_input` — the only difference is the contact.
#[test]
fn pressing_b1_flips_the_byte_the_l476_demo_prints() {
    let released = run_l476_demo(false);
    let pressed = run_l476_demo(true);
    assert!(
        released.contains("BTN=0\r\n"),
        "released must print BTN=0; got:\n{released}"
    );
    assert!(
        pressed.contains("BTN=1\r\n"),
        "pressing B1 must make the firmware print BTN=1; got:\n{pressed}"
    );
}

// ---------------------------------------------------------------------------
// Named channels: the same contact, the right word.
//
// A PIR, IR-obstacle, hall or vibration sensor is electrically a push button —
// one pin asserting a digital level — so they share the Button model. Their
// catalog entries name the channel (`motion`, `obstacle`, `field`,
// `vibration`, `touch`), and the compiler stamps that name onto the board_io
// binding. Without the stamp the catalog advertised `obstacle` while the engine
// only answered to `pressed`, so a stimulus the TS layer accepted was rejected
// by the engine — a vocabulary split across two layers.
// ---------------------------------------------------------------------------

/// An F103 with one active-high digital sensor on PC13 answering to `channel`.
fn f103_with_named_contact(channel: &str) -> SystemBus {
    let chip_path = workspace_root().join("configs/chips/stm32f103.yaml");
    let chip = ChipDescriptor::from_file(&chip_path).expect("load stm32f103 chip");
    let manifest_yaml = format!(
        r#"
name: "f103-sensor"
chip: "{}"
external_devices: []
board_io:
  - id: "sensor1"
    kind: "button"
    peripheral: "gpioc"
    pin: 13
    signal: "input"
    active_high: true
    channel: "{channel}"
"#,
        chip_path.display()
    );
    let manifest: SystemManifest = serde_yaml::from_str(&manifest_yaml).expect("parse manifest");
    SystemBus::from_config(&chip, &manifest).expect("build bus")
}

#[test]
fn a_named_contact_channel_is_discoverable_and_drivable() {
    for channel in ["motion", "obstacle", "field", "vibration", "touch"] {
        let mut bus = f103_with_named_contact(channel);
        assert!(
            bus.list_inputs()
                .iter()
                .any(|(owner, ch)| owner == "sensor1" && ch.key == channel),
            "{channel} must be discoverable under its own name"
        );
        // Active-high: idle reads LOW, asserting drives the pin HIGH.
        assert!(!pc13_high(&mut bus), "{channel}: idle reads LOW");
        bus.set_input(Some("sensor1"), channel, 1.0)
            .unwrap_or_else(|e| panic!("{channel} must be drivable: {e:?}"));
        assert!(
            pc13_high(&mut bus),
            "{channel}: asserting drives the pin HIGH"
        );
        bus.set_input(Some("sensor1"), channel, 0.0).expect("clear");
        assert!(
            !pc13_high(&mut bus),
            "{channel}: clearing returns the pin LOW"
        );
    }
}

/// A binding with no `channel` — every plain push button — still says `pressed`.
#[test]
fn an_unnamed_contact_still_answers_to_pressed() {
    let mut bus = f103_with_button(false);
    assert!(bus.list_inputs().iter().any(|(_, ch)| ch.key == "pressed"));
}

/// An unknown channel name must NOT mint a channel nothing can drive: it falls
/// back to `pressed` rather than being advertised and then failing on set.
#[test]
fn an_unknown_channel_name_falls_back_rather_than_advertising_a_dead_one() {
    let mut bus = f103_with_named_contact("teleportation");
    let keys: Vec<_> = bus.list_inputs().into_iter().map(|(_, c)| c.key).collect();
    assert_eq!(keys, vec!["pressed"], "unknown name falls back to pressed");
    bus.set_input(Some("sensor1"), "pressed", 1.0)
        .expect("the fallback channel really works");
}
