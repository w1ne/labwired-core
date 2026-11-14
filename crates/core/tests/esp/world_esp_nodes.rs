// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! ESP32-S3, classic ESP32 and ESP32-C3 bare-ELF chips as nodes of a
//! multi-chip world, each wired to an STM32G0B1 by a `gpio_net`.
//!
//! The STM32 runs the unchanged `examples/gpio-net-two-boards` firmware: it
//! counts every edge on its `irq` pad (PB0, EXTI line 0, both edges) in an
//! interrupt handler and keeps the tallies at SRAM 0x2000_0100
//! (`[irq rising, irq falling, ...]`). Each ESP node drives that wire from its
//! own firmware, so a count on the STM32 is a cross-chip fact: the ESP node
//! booted, ran its firmware to the pin, and the edge crossed the net.
//!
//! Before these nodes were built like the single-chip engine builds them, none
//! of this ran: an S3 node was constructed as a classic ESP32 (its chip YAML
//! spells the core only in `arch:`), a C3 bare ELF got no mask ROM, and a
//! classic-ESP32 Arduino sketch had no way to take its fast boot.

use labwired_config::EnvironmentManifest;
use labwired_core::world::World;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

fn example() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples/gpio-net-two-boards")
}

/// A two-node world: the STM32 edge counter and one ESP node whose `pin`
/// drives the `irq` net.
fn env_yaml(esp_system: &str, esp_firmware: &str, pin: u8, profile: Option<&str>) -> String {
    let profile = profile
        .map(|p| format!("    profile: {p}\n"))
        .unwrap_or_default();
    format!(
        "schema_version: \"1.0\"\n\
         name: esp-node\n\
         nodes:\n\
         \x20 - id: stm\n\
         \x20   system: \"../stm32g0b1re/system.yaml\"\n\
         \x20   firmware: \"firmware/stm.elf\"\n\
         \x20 - id: esp\n\
         \x20   system: \"{esp_system}\"\n\
         \x20   firmware: \"{esp_firmware}\"\n\
         {profile}\
         interconnects:\n\
         \x20 - type: gpio_net\n\
         \x20   nodes: [esp, stm]\n\
         \x20   config:\n\
         \x20     name: irq\n\
         \x20     pull: down\n\
         \x20     members:\n\
         \x20       - {{ node: esp, peripheral: gpio, pin: {pin} }}\n\
         \x20       - {{ node: stm, peripheral: gpiob, pin: 0 }}\n"
    )
}

struct Run {
    world: World,
    esp_uart: Arc<Mutex<Vec<u8>>>,
}

fn build(yaml: &str) -> anyhow::Result<Run> {
    let manifest: EnvironmentManifest = serde_yaml::from_str(yaml)?;
    let mut world = World::from_manifest(manifest, &example())?;
    let esp_uart = Arc::new(Mutex::new(Vec::new()));
    world
        .machines
        .get_mut("esp")
        .unwrap()
        .attach_uart_tx_sink(esp_uart.clone(), false)?;
    Ok(Run { world, esp_uart })
}

impl Run {
    fn console(&self) -> String {
        String::from_utf8_lossy(&self.esp_uart.lock().unwrap()).into_owned()
    }

    /// STM32 `[irq rising, irq falling]`.
    fn stm_irq_counts(&self) -> [u32; 2] {
        let b = self.world.machines["stm"]
            .read_memory(0x2000_0100, 8)
            .unwrap();
        [
            u32::from_le_bytes([b[0], b[1], b[2], b[3]]),
            u32::from_le_bytes([b[4], b[5], b[6], b[7]]),
        ]
    }

    fn irq_net_edges(&self) -> u64 {
        self.world
            .gpio_net_reports()
            .iter()
            .find(|r| r.name == "irq")
            .expect("irq net")
            .edges
    }

    /// Step the world until `done` holds or `max_ms` of simulated time pass.
    fn run_until(&mut self, max_ms: u64, done: impl Fn(&Run) -> bool) {
        let end = max_ms * 1_000_000_000;
        while self.world.round_now_ps().unwrap() < end {
            for (id, r) in self.world.step_rounds(50) {
                r.unwrap_or_else(|e| panic!("node {id}: {e:?}\nconsole: {}", self.console()));
            }
            if done(self) {
                return;
            }
        }
    }
}

/// ESP32-S3 bare ELF (the TIER1 self-test, fast boot): its GPIO check drives
/// GPIO4 high then low once, and it prints every check on its console.
#[test]
fn an_esp32s3_elf_node_pulses_the_stm32_and_reports_its_self_test() {
    let mut run = build(&env_yaml(
        "../../configs/systems/esp32s3.yaml",
        "../../tests/fixtures/tier1/esp32s3.elf",
        4,
        None,
    ))
    .expect("world");
    run.run_until(500, |r| r.console().contains("TIER1 done"));
    let console = run.console();
    for class in ["clock", "gpio", "timer", "irq"] {
        assert!(
            console.contains(&format!("TIER1 {class} PASS")),
            "{class}: {console}"
        );
    }
    assert!(console.contains("TIER1 done"), "{console}");
    // One pulse, both edges counted by the STM32's EXTI interrupt.
    assert_eq!(run.stm_irq_counts(), [1, 1], "console: {console}");
    assert_eq!(run.irq_net_edges(), 2);
}

/// ESP32-S3 merged flash image (the hosted Arduino build of
/// `tests/fixtures/source-debug/main.ino`) booted through the genuine mask
/// ROM. Its image header declares an 8 MB part: a node built on the old fixed
/// 4 MiB backing failed `esp_flash`'s size probe before `app_main`; the
/// backing is now the chip descriptor's 16 MiB, as on the single-chip engine.
#[test]
fn an_esp32s3_flash_image_node_boots_through_the_rom_and_pulses_the_stm32() {
    let mut run = build(&env_yaml(
        "../../configs/systems/esp32s3.yaml",
        "../../tests/fixtures/source-debug/esp32s3-arduino-flash.bin",
        2,
        None,
    ))
    .expect("world");
    // The first loop() iteration: one pulse on GPIO2 and one printed number.
    // (Each later one is a `delay(100)` away — 100 ms of a 240 MHz core.)
    run.run_until(3_000, |r| {
        r.stm_irq_counts()[1] >= 1 && numbers(&r.console()) >= 1
    });
    let console = run.console();
    assert!(console.contains("ESP-ROM:esp32s3"), "{console}");
    assert!(!console.contains("Detected size"), "{console}");
    let [rise, fall] = run.stm_irq_counts();
    assert!(
        rise >= 1 && fall >= 1,
        "STM32 {rise}/{fall}; console: {console}"
    );
    assert!(numbers(&console) >= 1, "loop() output: {console}");
}

/// Lines of `loop()` output: the sketch prints one number per iteration.
fn numbers(console: &str) -> usize {
    console
        .lines()
        .filter(|l| l.trim().parse::<u32>().is_ok())
        .count()
}

/// Classic ESP32 running a stock Arduino-ESP32 sketch
/// (`tests/fixtures/source-debug/main.ino`: `loop()` pulses GPIO2 and prints a
/// number) on the node profile `arduino-esp32` — the CLI's
/// `profile: arduino-esp32` fast boot.
#[test]
fn a_classic_esp32_arduino_node_runs_loop_against_the_stm32() {
    let mut run = build(&env_yaml(
        "../../configs/systems/esp32-wroom-32.yaml",
        "../../tests/fixtures/source-debug/esp32-arduino.elf",
        2,
        Some("arduino-esp32"),
    ))
    .expect("world");
    run.run_until(3_000, |r| {
        r.stm_irq_counts()[1] >= 3 && numbers(&r.console()) >= 3
    });
    let console = run.console();
    let [rise, fall] = run.stm_irq_counts();
    assert!(
        rise >= 3 && fall >= 3,
        "STM32 counted {rise} rising / {fall} falling; console: {console}"
    );
    // loop() prints `bump(ticks)`: ticks goes 0 -> 1 -> 4 -> 13, so the
    // sketch's first three lines are 1, 3 and 9.
    let printed: Vec<&str> = console
        .lines()
        .map(str::trim)
        .filter(|l| l.parse::<u32>().is_ok())
        .take(3)
        .collect();
    assert_eq!(printed, ["1", "3", "9"], "console: {console:?}");
}

/// ESP32-C3 esp-hal ELF (`tests/fixtures/world-esp/esp32c3-esp-hal-pulses.md`):
/// esp-hal's clock bring-up calls into the mask ROM and esp-println prints
/// through USB-Serial-JTAG, so this boots only with the ROM and the console
/// model the single-chip engine installs.
#[test]
fn an_esp32c3_esp_hal_elf_node_pulses_the_stm32() {
    let mut run = build(&env_yaml(
        "../../configs/systems/esp32c3-devkit.yaml",
        "../../tests/fixtures/world-esp/esp32c3-esp-hal-pulses.elf",
        4,
        None,
    ))
    .expect("world");
    run.run_until(200, |r| r.console().contains("C3 PULSES DONE"));
    let console = run.console();
    assert!(console.contains("C3 ESP-HAL BOOT"), "{console}");
    assert!(console.contains("C3 pulse 10"), "{console}");
    assert!(console.contains("C3 PULSES DONE"), "{console}");
    assert_eq!(run.stm_irq_counts(), [10, 10], "console: {console}");
    assert_eq!(run.irq_net_edges(), 20);
}

/// The profile names one chip's boot path; anywhere else it is refused.
#[test]
fn the_arduino_profile_is_refused_off_a_classic_esp32() {
    let err = match build(&env_yaml(
        "../../configs/systems/esp32c3-devkit.yaml",
        "../../tests/fixtures/world-esp/esp32c3-esp-hal-pulses.elf",
        4,
        Some("arduino-esp32"),
    )) {
        Ok(_) => panic!("a C3 node on the arduino-esp32 profile must not build"),
        Err(e) => format!("{e:#}"),
    };
    assert!(err.contains("not a classic ESP32"), "{err}");

    let yaml = env_yaml(
        "../../configs/systems/esp32-wroom-32.yaml",
        "../../tests/fixtures/source-debug/esp32-arduino.elf",
        2,
        Some("arduino-esp33"),
    );
    let err = match build(&yaml) {
        Ok(_) => panic!("an unknown profile must not build"),
        Err(e) => format!("{e:#}"),
    };
    assert!(err.contains("arduino-esp33"), "{err}");
}

/// The profile's thunks keep per-thread state, so two such nodes in one world
/// are refused instead of sharing it.
#[test]
fn two_arduino_profile_nodes_are_refused() {
    let yaml = "schema_version: \"1.0\"\n\
                name: two-arduino\n\
                nodes:\n\
                \x20 - { id: a, system: \"../../configs/systems/esp32-wroom-32.yaml\", firmware: \"../../tests/fixtures/source-debug/esp32-arduino.elf\", profile: arduino-esp32 }\n\
                \x20 - { id: b, system: \"../../configs/systems/esp32-wroom-32.yaml\", firmware: \"../../tests/fixtures/source-debug/esp32-arduino.elf\", profile: arduino-esp32 }\n";
    let manifest: EnvironmentManifest = serde_yaml::from_str(yaml).unwrap();
    let err = match World::from_manifest(manifest, &example()) {
        Ok(_) => panic!("two arduino-esp32 nodes must not build"),
        Err(e) => format!("{e:#}"),
    };
    assert!(err.contains("one such node"), "{err}");
}
