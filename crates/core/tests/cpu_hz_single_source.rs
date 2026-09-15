// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! The simulated CPU clock has ONE home: `ChipDescriptor::cpu_hz`, overridable
//! per board by `SystemManifest::cpu_hz`.
//!
//! It used to have four. The declarative-device attach arms defaulted to a flat
//! `80_000_000`, the WS2812 kit to `160_000_000`, and the TypeScript emitter
//! chose between those same two numbers by comparing the board name against the
//! string `"esp32c3"`. Ten system manifests declared a `cpu_hz:` that serde
//! silently dropped, because `SystemManifest` had no such field; `arduino-nano`
//! declared its 16 MHz under a `clock:` key that did not exist either.
//!
//! Nothing failed when those disagreed. A DHT22 converts its datasheet
//! microseconds to simulated cycles with this number, so a device told 80 MHz
//! on a 16 MHz part stretches every bit cell by five and the firmware decodes
//! noise — a wrong answer from an oracle, which is the one failure mode this
//! simulator cannot have.
//!
//! These tests are the guard. They are written against the corpus and the
//! public attach path, not against the code that resolves the clock, so they
//! fail if a chip stops declaring one, if a manifest key goes back to being
//! ignored, or if the resolution order is reordered.

use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::bus::SystemBus;
use labwired_core::peripherals::components::dht22::Dht22;
use std::path::PathBuf;

fn configs_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../configs")
}

fn yaml_files(sub: &str) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = std::fs::read_dir(configs_dir().join(sub))
        .unwrap_or_else(|e| panic!("read configs/{sub}: {e}"))
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "yaml"))
        .collect();
    out.sort();
    assert!(!out.is_empty(), "configs/{sub} holds no YAML");
    out
}

/// Every chip in the corpus states the clock it runs at.
///
/// The field defaults to `0` so an out-of-tree descriptor written before it
/// existed still loads; in-tree, `0` means someone added a chip and left the
/// engine guessing, which is what this catches.
#[test]
fn every_chip_descriptor_declares_a_cpu_hz() {
    let mut undeclared = Vec::new();
    for path in yaml_files("chips") {
        let chip = ChipDescriptor::from_file(&path)
            .unwrap_or_else(|e| panic!("parse {}: {e}", path.display()));
        if chip.cpu_hz == 0 {
            undeclared.push(path.file_name().unwrap().to_string_lossy().into_owned());
        }
    }
    assert!(
        undeclared.is_empty(),
        "these chip descriptors declare no `cpu_hz:` — add one (the clock the \
         simulator runs that part at, in Hz) so self-timed devices on the board \
         are not handed a guess: {undeclared:?}"
    );
}

/// No system manifest carries a top-level key the engine throws away.
///
/// `arduino-nano.yaml` spelled its clock `clock: {cpu_hz: …}`. Serde ignored the
/// whole block and nothing reported it, so the board looked configured and was
/// not. `deny_unknown_fields` would catch this at load time, but it would also
/// reject manifests the hosted runner receives from older clients, so the guard
/// lives here instead — over the corpus we own.
#[test]
fn system_manifests_declare_no_key_the_engine_ignores() {
    // Every field `SystemManifest` models. Add a field there → add it here.
    const MODELLED: &[&str] = &[
        "schema_version",
        "name",
        "chip",
        "cpu_hz",
        "memory_overrides",
        "external_devices",
        "parts",
        "cosim_models",
        "motor_models",
        "board_io",
        "debug_uart",
        "wifi_ap",
        "peripherals",
        "walk_deleted",
    ];

    let mut stray: Vec<String> = Vec::new();
    for path in yaml_files("systems") {
        let text = std::fs::read_to_string(&path).expect("read manifest");
        let doc: serde_yaml::Value =
            serde_yaml::from_str(&text).unwrap_or_else(|e| panic!("parse {}: {e}", path.display()));
        let Some(map) = doc.as_mapping() else {
            continue;
        };
        for key in map.keys().filter_map(|k| k.as_str()) {
            if !MODELLED.contains(&key) {
                stray.push(format!(
                    "{}: {key}",
                    path.file_name().unwrap().to_string_lossy()
                ));
            }
        }
    }
    assert!(
        stray.is_empty(),
        "these manifests declare top-level keys `SystemManifest` does not model, \
         so the engine reads none of them: {stray:?}"
    );
}

/// A data pin each chip family actually has, for the DHT22 to hang off.
fn data_pin(chip_yaml: &str) -> &'static str {
    match chip_yaml {
        "esp32c3.yaml" => "GPIO8",
        _ => "PA8",
    }
}

/// Build a bus for `chip_yaml` with one DHT22 and return the clock it got.
/// `manifest_clock` and `device_clock` are the two overrides under test.
fn dht22_clock(chip_yaml: &str, manifest_clock: Option<u64>, device_clock: Option<u64>) -> u64 {
    let chip = ChipDescriptor::from_file(configs_dir().join("chips").join(chip_yaml))
        .expect("read chip descriptor");
    let pin = data_pin(chip_yaml);
    let manifest_line = manifest_clock
        .map(|hz| format!("cpu_hz: {hz}\n"))
        .unwrap_or_default();
    let device_line = device_clock
        .map(|hz| format!("      cpu_hz: {hz}\n"))
        .unwrap_or_default();
    let manifest: SystemManifest = serde_yaml::from_str(&format!(
        r#"
name: "clock-resolution"
chip: "../chips/{chip_yaml}"
{manifest_line}external_devices:
  - id: "sensor"
    type: "dht22"
    connection: "gpio"
    config:
      data_pin: "{pin}"
{device_line}board_io: []
"#
    ))
    .expect("parse manifest");

    let bus = SystemBus::from_config(&chip, &manifest).expect("build bus");
    let sensors: Vec<&Dht22> = bus.gpio_devices_of::<Dht22>().collect();
    assert_eq!(sensors.len(), 1, "exactly one DHT22 attached");
    sensors[0].cpu_hz
}

/// With nothing else declared, a self-timed device runs off the chip's clock.
///
/// The STM32F103 is 72 MHz. Before the descriptor carried it, this device was
/// handed the attach arm's flat 80 MHz — an 11% timing error on every bit cell,
/// on a device whose entire job is to time bit cells.
#[test]
fn chip_clock_reaches_a_self_timed_device() {
    assert_eq!(dht22_clock("stm32f103.yaml", None, None), 72_000_000);
    assert_eq!(dht22_clock("esp32c3.yaml", None, None), 160_000_000);
}

/// The clock a board declares is the clock, whatever the part's rating says.
///
/// Three chips whose real clocks are nowhere near the 80 MHz every board used
/// to be handed: the ATmega328P at 16 MHz (five times slower), the RP2040 at
/// 125 MHz, and the STM32H735 at 550 MHz (nearly seven times faster). The
/// assertion is at the descriptor rather than through an attached device
/// because the AVR has no GPIO output register the one-wire primitive can bind
/// to, so a DHT22 cannot be placed on it in the engine at all today.
#[test]
fn chip_descriptors_carry_the_real_clock_not_the_old_flat_default() {
    let clock = |f: &str| {
        ChipDescriptor::from_file(configs_dir().join("chips").join(f))
            .expect("read chip descriptor")
            .cpu_hz
    };
    assert_eq!(clock("atmega328p.yaml"), 16_000_000);
    assert_eq!(clock("stm32h735.yaml"), 550_000_000);
    assert_eq!(clock("rp2040.yaml"), 125_000_000);
}

/// A board that runs the part slower than its rating says so, and is believed.
///
/// This is the NUCLEO-L476RG case: an 80 MHz part whose firmware never leaves
/// the 4 MHz MSI reset clock. The board has declared that for a long time; until
/// `SystemManifest::cpu_hz` existed, serde dropped the line.
#[test]
fn manifest_clock_overrides_the_chip() {
    assert_eq!(
        dht22_clock("stm32l476.yaml", None, None),
        80_000_000,
        "chip default is the rated clock"
    );
    assert_eq!(
        dht22_clock("stm32l476.yaml", Some(4_000_000), None),
        4_000_000,
        "the board's declared clock wins over the chip's rating"
    );
}

/// The clocks the shipped manifests declare are the clocks they now get.
///
/// Not a synthetic manifest: these are the corpus files, read from disk,
/// through the same loader the CLI and the browser use. The corpus writes
/// hertz with underscores (`160_000_000`), which YAML hands to serde as a
/// *string* — so this is also the proof that the lax parse is what makes those
/// lines mean anything.
#[test]
fn shipped_manifests_parse_the_clock_they_declare() {
    let manifest = |f: &str| {
        SystemManifest::from_file(configs_dir().join("systems").join(f))
            .unwrap_or_else(|e| panic!("load {f}: {e}"))
    };
    assert_eq!(
        manifest("nucleo-l476rg.yaml").cpu_hz,
        Some(4_000_000),
        "the MSI reset clock this board documents in a comment is now read"
    );
    assert_eq!(
        manifest("esp32c3-devkit.yaml").cpu_hz,
        Some(160_000_000),
        "underscored hertz survive the parse"
    );
    assert_eq!(
        manifest("arduino-nano.yaml").cpu_hz,
        Some(16_000_000),
        "was nested under a `clock:` key nothing modelled"
    );
}

/// A `config.cpu_hz` on the placed device still beats everything.
///
/// This is what the diagram emitter writes, so every lab compiled today takes
/// this path and behaves exactly as it did before the chip carried a clock.
#[test]
fn explicit_device_clock_beats_board_and_chip() {
    assert_eq!(
        dht22_clock("stm32l476.yaml", Some(4_000_000), Some(8_000_000)),
        8_000_000
    );
}

/// The engine's own ESP32 clock constants and the chip descriptors must agree.
///
/// These constants are what the UART divisor, the SPI bit time and the systimer
/// are computed from. If a descriptor said one thing and the peripheral another,
/// the board's serial output and its DHT22 would disagree about how long a
/// microsecond is — and the descriptor's number would be the plausible lie.
#[test]
fn esp_chip_descriptors_match_the_engine_constants() {
    let clock = |f: &str| {
        ChipDescriptor::from_file(configs_dir().join("chips").join(f))
            .expect("read chip descriptor")
            .cpu_hz
    };
    assert_eq!(
        clock("esp32c3.yaml"),
        labwired_core::peripherals::esp32c3::uart::CPU_CLOCK_HZ,
        "esp32c3.yaml vs peripherals/esp32c3/uart.rs"
    );
    assert_eq!(
        clock("esp32c3.yaml"),
        labwired_core::peripherals::esp32c3::rtc_timer::CPU_HZ,
        "esp32c3.yaml vs peripherals/esp32c3/rtc_timer.rs"
    );
    assert_eq!(
        clock("nrf54l15.yaml"),
        u64::from(labwired_core::peripherals::nrf54l::grtc::CPU_HZ_DEFAULT),
        "nrf54l15.yaml vs peripherals/nrf54l/grtc.rs"
    );
    // The S3's core clock does not live in a peripheral: it is carried by
    // `Esp32s3Opts`, which every S3 construction site spreads from
    // `..Esp32s3Opts::default()`. That default is therefore the S3's engine
    // constant, and it is the number the SYSTIMER divides down — so it is
    // pinned here exactly like the C3's.
    assert_eq!(
        clock("esp32s3.yaml"),
        u64::from(labwired_core::system::xtensa::Esp32s3Opts::default().cpu_clock_hz),
        "esp32s3.yaml vs Esp32s3Opts::default() in system/xtensa/esp32s3.rs"
    );
    assert_eq!(
        clock("esp32s3.yaml"),
        u64::from(labwired_core::system::xtensa::ESP32S3_CPU_CLOCK_HZ),
        "esp32s3.yaml vs system/xtensa/esp32s3.rs ESP32S3_CPU_CLOCK_HZ"
    );
    // A board that declares its own clock must reach the opts, not just the
    // descriptor: `for_chip` is the only path that carries it.
    assert_eq!(
        clock("esp32s3-zero.yaml"),
        u64::from(
            labwired_core::system::xtensa::Esp32s3Opts::for_chip(
                &ChipDescriptor::from_file(configs_dir().join("chips").join("esp32s3-zero.yaml"))
                    .expect("read chip descriptor")
            )
            .cpu_clock_hz
        ),
        "esp32s3-zero.yaml vs Esp32s3Opts::for_chip"
    );
}

/// The S3's SYSTIMER must keep simulated time at the rate the part runs at.
///
/// This is the user-visible half of the pin above: SYSTIMER divides the CPU
/// cycle stream by `cpu_clock_hz / 16 MHz`, and `sim_time_us` — the counter
/// esp-idf's `esp_timer` and esp-hal's `Delay` read — is that tick count over
/// 16. With the opts stuck at 80 MHz while the part (and TIMG0, and every
/// board YAML) runs at 240, the divider was 5 instead of 15 and firmware saw
/// every microsecond elapse three times too fast: a one-second delay returned
/// after a third of a second of modelled time.
///
/// Asserted through the production wiring — `configure_xtensa_esp32s3` →
/// peripheral config → factory → model — so a field that is set but never
/// threaded still fails.
#[test]
fn esp32s3_systimer_keeps_time_at_the_declared_core_clock() {
    use labwired_core::system::xtensa::{configure_xtensa_esp32s3, Esp32s3Opts};

    let chip = ChipDescriptor::from_file(configs_dir().join("chips").join("esp32s3.yaml"))
        .expect("read chip descriptor");

    let mut bus = SystemBus::new();
    let _wiring = configure_xtensa_esp32s3(&mut bus, &Esp32s3Opts::for_chip(&chip));

    // Exactly one second of CPU cycles at the clock the chip declares.
    bus.set_current_cycle(chip.cpu_hz);

    let systimer = bus
        .peripherals
        .iter()
        .find(|p| p.name == "systimer")
        .expect("configure_xtensa_esp32s3 registers a systimer");

    assert_eq!(
        systimer.dev.sim_time_us(),
        Some(1_000_000),
        "one second of CPU cycles at the declared {} Hz must read as 1 s of \
         simulated time; a wrong divider makes esp_timer_get_time lie by that ratio",
        chip.cpu_hz
    );
}
