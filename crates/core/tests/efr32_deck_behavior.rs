// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! The BRD2709A agent deck, RUNNING — the chip's L2 evidence.
//!
//! `efr32mg26`'s conformance row already carries a silicon reset oracle and
//! matches 219/219 registers. That is an L1 claim: the register FILE agrees
//! with the die. It says nothing about whether a driver written against those
//! registers makes a panel light up, so the chip's `behavior_gate` was `None`.
//!
//! This closes it, and does so with the deck rather than a blinky, because the
//! deck is the thing that can fail interestingly: a display on USART0 in SPI
//! mode, a microphone on USART2 in I2S mode, an IADC conversion and five GPIO
//! contacts, all at once.
//!
//! ⚠️ IN-PROCESS ON PURPOSE. `examples/brd2709a/deck-smoke.yaml` asserts the
//! same things through the CLI, but it runs in the coverage-matrix workflow,
//! which is not a required PR check. A `behavior_gate` that only runs
//! elsewhere is not holding anything, so the evidence lives here too.

use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::bus::SystemBus;
use labwired_core::cpu::cortex_m::CortexM;
use labwired_core::inspect::InspectOpts;
use labwired_core::system::cortex_m::configure_cortex_m;
use labwired_core::Machine;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

fn root(rel: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(rel)
}

/// The deck ELF this gate runs.
///
/// ⚠️ A COMMITTED FIXTURE FIRST, A CROSS-BUILD SECOND. This originally always
/// shelled out to `cargo build --target thumbv7m-none-eabi`, which passes on a
/// developer machine and FAILS on a PR runner with no cross toolchain — so the
/// chip's `behavior_gate` was red on every PR while being green locally. The
/// tree's answer for that is `scripts/ci/workspace-test-shards.json`'s
/// `cross_build_excluded` list, but moving this there would send it to the
/// nightly lane only, and a behaviour gate that never runs on a PR is the very
/// thing writing this test in process was meant to avoid.
///
/// So: build from source when the target is installed (a developer, and
/// core-full, which installs the ARM targets — that lane is what catches a
/// STALE fixture), and fall back to the committed ELF otherwise, so every PR
/// shard still executes the assertions. This is not a graceful skip: the same
/// firmware runs and the same things are asserted either way.
///
/// Regenerate with:
///   cargo build --release --target thumbv7m-none-eabi -p firmware-mg26-deck
///   cp target/thumbv7m-none-eabi/release/firmware-mg26-deck \
///      crates/core/tests/fixtures/mg26-deck.elf
fn deck_elf() -> PathBuf {
    let fixture = root("crates/core/tests/fixtures/mg26-deck.elf");
    let built = root("target/thumbv7m-none-eabi/release/firmware-mg26-deck");

    let has_target = std::process::Command::new("rustc")
        .args(["--print", "target-list"])
        .output()
        .ok()
        .map(|o| String::from_utf8_lossy(&o.stdout).contains("thumbv7m-none-eabi"))
        .unwrap_or(false)
        && std::process::Command::new(env!("CARGO"))
            .args([
                "build",
                "-p",
                "firmware-mg26-deck",
                "--target",
                "thumbv7m-none-eabi",
                "--release",
            ])
            // Clear coverage instrumentation flags so the no_std cross-build
            // does not fail with E0463 under llvm-cov.
            .env_remove("CARGO_ENCODED_RUSTFLAGS")
            .env_remove("RUSTFLAGS")
            .current_dir(root(""))
            .status()
            .map(|s| s.success())
            .unwrap_or(false);

    if has_target && built.exists() {
        return built;
    }
    assert!(
        fixture.exists(),
        "no cross toolchain AND no committed fixture at {fixture:?} — this gate \
         would assert nothing"
    );
    fixture
}

struct Run {
    console: String,
    machine: Machine<CortexM>,
}

fn run_deck(elf: &Path) -> Run {
    let sys_path = root("examples/brd2709a/agent-deck-system.yaml");
    let manifest = SystemManifest::from_file(&sys_path).expect("load the deck manifest");
    let chip_path = sys_path.parent().unwrap().join(&manifest.chip);
    let chip = ChipDescriptor::from_file(&chip_path).expect("load efr32mg26");

    let mut bus = SystemBus::from_config(&chip, &manifest).expect("build the deck bus");
    let uart = Arc::new(Mutex::new(Vec::new()));
    bus.attach_uart_tx_sink(uart.clone(), false);

    let (cpu, _nvic) = configure_cortex_m(&mut bus);
    let mut machine = Machine::new(cpu, bus);
    let image = labwired_loader::load_elf(elf).expect("parse the deck ELF");
    machine
        .load_firmware(&image)
        .expect("load the deck firmware");

    // Filling 54,400 pixels one SPI byte at a time is most of the budget. Stop
    // as soon as the firmware says it is finished rather than always paying
    // the ceiling.
    const CEILING: u64 = 40_000_000;
    let mut steps = 0u64;
    while steps < CEILING {
        machine.step().expect("the deck firmware runs clean");
        steps += 1;
        if steps.is_multiple_of(100_000) {
            let seen = uart.lock().unwrap().clone();
            if String::from_utf8_lossy(&seen).contains("MG26-DECK DONE") {
                break;
            }
        }
    }

    let console = String::from_utf8_lossy(&uart.lock().unwrap().clone()).into_owned();
    Run { console, machine }
}

#[test]
fn the_deck_firmware_drives_every_part() {
    let run = run_deck(&deck_elf());
    let out = &run.console;

    // The run reached the end. Without this every assertion below could pass
    // on a truncated console that simply never got to the failing part.
    assert!(
        out.contains("MG26-DECK DONE"),
        "the deck firmware never finished, so nothing below is conclusive:\n{out}"
    );

    // ── Panel ────────────────────────────────────────────────────────────
    assert!(out.contains("tft: slpout colmod dispon"), "console:\n{out}");
    assert!(out.contains("tft: filled 54400 px"), "console:\n{out}");

    // ── Microphone ───────────────────────────────────────────────────────
    // ⚠️ THE HALVES SEPARATELY. L/R is tied low, so the INMP441 drives the
    // LEFT half of each stereo frame and tristates the right. This gate first
    // asserted "4 of 8 slots driven" — a TOTAL, which is equally true of a mic
    // on the right channel, and retyping the manifest to `right` passed it.
    // Counting the halves apart is what makes the L/R strap observable.
    assert!(
        out.contains("mic: left=4 right=0"),
        "a left-channel mic drives the even slots and tristates the odd ones; console:\n{out}"
    );

    // ── Fader ────────────────────────────────────────────────────────────
    // ⚠️ THE VALUE, NOT THE LABEL. The pot boots centred: 1650 mV of a 3300 mV
    // reference is 2048 on a 12-bit conversion. Asserting only "code=" passes
    // on a zero, and zero is exactly what a fader on the wrong IADC channel
    // prints — this deck shipped that way for one run.
    assert!(
        out.contains("fader: code=2048"),
        "a centred fader reads 2048; console:\n{out}"
    );

    // ── Contacts ─────────────────────────────────────────────────────────
    // Every contact idles released. The pushbutton module DRIVES its SIG line
    // and so idles LOW; the rest close to ground through a pull-up and idle
    // HIGH. Reading them uniformly would mean the polarity was never modelled.
    assert!(
        out.contains("in: CLK=1 DT=1 SW=1 BTN=0 TOGGLE=1"),
        "released contacts must read at their own idle levels; console:\n{out}"
    );

    // ── And the glass itself, not the firmware's opinion of it ───────────
    //
    // ⚠️ THROUGH `display_artifact`, NOT A DOWNCAST. This first reached for
    // `St7789` by concrete type, which the downcast ratchet correctly refused:
    // that is debt, and it is also the WEAKER check. `display_artifact` is the
    // same door the CLI's `display_region` assertion and the browser both use,
    // so asserting here proves the evidence a user would actually see — a panel
    // that painted but whose artifact never reached that door would satisfy a
    // downcast and still show a person nothing.
    let art = run
        .machine
        .bus
        .display_artifact(
            "tft",
            &InspectOpts {
                include_bytes: true,
                peripheral: None,
            },
        )
        .expect("the panel must publish a display artifact under its own id");

    assert_eq!(
        art.meta.get("lit").and_then(|v| v.as_bool()),
        Some(true),
        "the panel must be AWAKE and DISPON, not merely painted: a firmware \
         that fills frame memory but skips SLPOUT drives a dark panel"
    );
    assert_eq!(
        (
            art.meta.get("w").and_then(|v| v.as_u64()),
            art.meta.get("h").and_then(|v| v.as_u64()),
        ),
        (Some(170), Some(320)),
        "the artifact must be cropped to this module's glass, not the 240x320 \
         frame memory the ST7789V datasheet describes; meta was {:?}",
        art.meta
    );

    // Ink is a non-black pixel, counted off the artifact's own bytes rather
    // than trusting the firmware's "filled 54400 px" line: that line proves the
    // loop ran, not that anything reached the panel.
    let bytes = art.bytes.as_ref().expect("include_bytes was requested");
    let inked = bytes
        .chunks_exact(2)
        .filter(|px| px[0] != 0 || px[1] != 0)
        .count();
    assert_eq!(
        inked,
        170 * 320,
        "every pixel of the 170x320 glass must carry ink; got {inked}"
    );

    // And the whole glass is ONE colour, the RGB565 blue the firmware wrote.
    // Ink alone would pass on a panel painted with garbage; this says the
    // bytes that arrived are the bytes that were sent.
    assert_eq!(
        (
            art.meta.get("top_colour").and_then(|v| v.as_str()),
            art.meta.get("top_colour_pixels").and_then(|v| v.as_u64()),
        ),
        (Some("0x001F"), Some(170 * 320)),
        "the glass must be uniformly the 0x001F the firmware wrote; meta was {:?}",
        art.meta
    );
}
