// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! RP2040 SPI: a byte goes into the shift engine and the same byte comes back.
//!
//! Why this file exists
//! ====================
//! `validation/bus_proof_matrix.json` recorded `rp2040/spi` as `shallow`, and
//! the reason was not that the evidence was weak — it was that the evidence ran
//! nowhere. `examples/tier1-fixture/rp2040/src/main.rs::check_spi` already does
//! the real thing: put the PL022 in internal loopback (`CR1.LBM`), write `0xA5`
//! to `SSPDR`, wait for `SR.RNE`, and require the byte read back to be `0xA5`.
//! Its ELF is committed and sha256-pinned (`tests/fixtures/tier1/MANIFEST.json`).
//!
//! But the only things that read its verdict were `tier1_matrix` (which asserts
//! the row has the right NUMBER of cells, not that `spi` passed) and
//! `tier1_matrix_ratchet` (`#[cfg_attr(debug_assertions, ignore)]`, invoked only
//! by `core-full` — nightly / `workflow_dispatch` / `[full-ci]`). An RP2040 SPI
//! regression could not fail a PR.
//!
//! This test closes that: it lives under `crates/core/src/tests/`, so
//! `cargo test --lib` picks it up and it runs in `pr-gate` on EVERY pull
//! request. It needs no toolchain, no `--release`, and no env var.
//!
//! Not vacuous, by construction
//! ============================
//! Three ways this could have been a green gate over nothing, all closed:
//!
//! 1. A missing fixture would make it silently pass. It does not: the ELF is
//!    committed, and its absence is an `assert!`, not a `return`.
//! 2. `TIER1 spi PASS` could be printed by a fixture whose `check_spi` had been
//!    weakened to a register poke. So the test also reads the fixture's SOURCE
//!    and requires the `0xA5` comparison to still be in it. Weakening the check
//!    breaks this test even if the stale ELF keeps printing PASS.
//! 3. `contains("TIER1 spi PASS")` would also be satisfied by an empty-ish
//!    transcript in a future where the marker changed. So the test additionally
//!    requires `TIER1 spi FAIL` to be absent AND the transcript to be non-empty,
//!    and prints the captured bytes so a lane log can tell "ran and saw the
//!    transcript" from "was skipped".

use crate::bus::SystemBus;
use crate::Machine;
use labwired_config::{ChipDescriptor, SystemManifest};
use std::path::Path;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

/// Minimal PT_LOAD walk. Deliberately does NOT use `labwired-loader`: that
/// crate depends on `labwired-core`, so calling it from inside core's own lib
/// tests pulls a second copy of `ProgramImage` into the graph and the types no
/// longer unify. `tests::nrf52` bypasses it the same way, for the same reason.
fn load_elf_image(path: &Path) -> crate::memory::ProgramImage {
    let bytes = std::fs::read(path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
    let elf = goblin::elf::Elf::parse(&bytes).expect("parse ELF");
    let mut image = crate::memory::ProgramImage::new(elf.entry, crate::Arch::Arm);
    for ph in &elf.program_headers {
        if ph.p_type != goblin::elf::program_header::PT_LOAD || ph.p_filesz == 0 {
            continue;
        }
        let off = ph.p_offset as usize;
        let size = ph.p_filesz as usize;
        image.add_segment(ph.p_paddr, bytes[off..off + size].to_vec());
    }
    image
}

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|p| p.parent())
        .expect("workspace root")
        .to_path_buf()
}

/// Budget. `check_spi` is the FOURTH check the fixture runs (clock, timer,
/// gpio, then spi), so the verdict lands early; the loop below stops the moment
/// the line arrives, so this is a ceiling, not a cost.
const MAX_STEPS: u64 = 4_000_000;

#[test]
fn rp2040_spi_loopback_carries_a_real_byte_under_firmware() {
    let root = repo_root();

    // ── the claim's other half: the fixture must still MAKE the comparison ──
    let fixture_src = root.join("examples/tier1-fixture/rp2040/src/main.rs");
    let src = std::fs::read_to_string(&fixture_src)
        .unwrap_or_else(|e| panic!("read {}: {e}", fixture_src.display()));
    let spi_body = src
        .split_once("fn check_spi()")
        .map(|(_, rest)| rest.split_once("\n}").map(|(b, _)| b).unwrap_or(rest))
        .expect("examples/tier1-fixture/rp2040/src/main.rs has no check_spi");
    for needed in ["CR1_LBM", "0xA5", "SR_RNE", "spi-data"] {
        assert!(
            spi_body.contains(needed),
            "rp2040 tier-1 check_spi no longer contains `{needed}`. This test asserts that \
             `TIER1 spi PASS` means a byte round-tripped through the PL022 shift engine; if \
             check_spi has been reduced to a register poke, that claim is no longer true. \
             Fix check_spi, or downgrade rp2040/spi in validation/bus_proof_matrix.json."
        );
    }

    // ── run the committed firmware ──────────────────────────────────────────
    // The tier-1 rp2040 target is a fast-boot ELF-entry target (see
    // `TIER1_TARGETS` in crates/cli/src/tier1.rs), so the mask ROM at address 0
    // must not shadow flash — same opt-out as `tests::rp2040::rp2040_bus()`.
    std::env::set_var("LABWIRED_RP2040_BOOTROM", "");

    let elf = root.join("tests/fixtures/tier1/rp2040.elf");
    assert!(
        elf.exists(),
        "committed fixture missing: {}. It is a plain binary in git (sha256 in \
         tests/fixtures/tier1/MANIFEST.json), not LFS — a checkout without it is broken, \
         and this test refuses to pass over the gap.",
        elf.display()
    );

    let system_path = root.join("configs/systems/rp2040-pico.yaml");
    let mut manifest = SystemManifest::from_file(&system_path)
        .unwrap_or_else(|e| panic!("load {}: {e}", system_path.display()));
    let chip_path = system_path.parent().unwrap().join(&manifest.chip);
    let chip = ChipDescriptor::from_file(&chip_path)
        .unwrap_or_else(|e| panic!("load {}: {e}", chip_path.display()));
    manifest.chip = chip_path.to_str().unwrap().to_string();

    let mut bus = SystemBus::from_config(&chip, &manifest).expect("build rp2040 bus");
    let sink = Arc::new(Mutex::new(Vec::new()));
    bus.attach_uart_tx_sink(sink.clone(), false);

    let (cpu, _nvic) = crate::system::cortex_m::configure_cortex_m(&mut bus);
    let mut machine = Machine::new(cpu, bus);
    let image = load_elf_image(&elf);
    machine.load_firmware(&image).expect("load firmware");

    let mut steps = 0u64;
    while steps < MAX_STEPS {
        if machine.step().is_err() {
            break;
        }
        steps += 1;
        // Poll cheaply; the verdict line is short and lands early.
        if steps % 20_000 == 0 {
            let seen = String::from_utf8_lossy(&sink.lock().unwrap()).into_owned();
            if seen.contains("TIER1 spi PASS") || seen.contains("TIER1 spi FAIL") {
                break;
            }
        }
    }

    let uart = String::from_utf8_lossy(&sink.lock().unwrap()).into_owned();
    eprintln!(
        "rp2040 tier-1 fixture: steps={steps} uart_bytes={} transcript:\n{}",
        uart.len(),
        uart.trim()
    );

    assert!(
        !uart.is_empty(),
        "no UART bytes at all after {steps} steps — the fixture did not run, so a `PASS` \
         check below would have been vacuous"
    );
    assert!(
        !uart.contains("TIER1 spi FAIL"),
        "rp2040 SPI loopback FAILED in firmware. `spi-no-rx` = the byte never reached the RX \
         FIFO; `spi-data` = it arrived corrupted. Transcript:\n{uart}"
    );
    assert!(
        uart.contains("TIER1 spi PASS"),
        "the rp2040 tier-1 fixture never reported an SPI verdict within {MAX_STEPS} steps. \
         Transcript:\n{uart}"
    );
}
