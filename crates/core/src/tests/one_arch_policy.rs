// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! ONE architecture policy — a guard, not a convention.
//!
//! Two questions decide what silicon a run models, and both had more than one
//! answer:
//!
//! 1. **"What can this chip be?"** — a match on `chip.arch`. The CLI
//!    (`system::node`) rejected `Arch::Unknown`; the browser
//!    (`crates/wasm/src/lib.rs`) ran it as a Cortex-M. Same chip, two answers,
//!    and nothing went red.
//! 2. **"What is this ELF?"** — a match on `e_machine`. `system::node`
//!    bailed on an unrecognised machine; `crates/loader` warned and returned
//!    `Arch::Unknown`, and one of the two spelled EM_XTENSA as the literal 94.
//!
//! For a product sold as an oracle, "the browser and the CLI disagree about
//! what processor your firmware is" is a correctness defect, not untidiness.
//!
//! Both questions now have exactly one implementation, in
//! [`crate::system::arch_policy`]. These tests assert the answer *and* that no
//! second copy grows back.

use crate::system::arch_policy::{elf_arch, machine_family, MachineFamily};
use labwired_config::{Arch, ChipDescriptor};
use std::path::{Path, PathBuf};

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("repo root")
}

fn read(root: &Path, rel: &str) -> String {
    std::fs::read_to_string(root.join(rel))
        .unwrap_or_else(|e| panic!("{rel} must be readable for this guard to mean anything: {e}"))
}

/// Strip `//` line comments so prose that names a symbol is not mistaken for
/// code that uses it. The same extractor idiom as `one_arduino_boot_path`.
fn code_only(src: &str) -> String {
    src.lines()
        .map(|l| match l.find("//") {
            Some(i) => &l[..i],
            None => l,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

const POLICY: &str = "crates/core/src/system/arch_policy.rs";

/// Every file that is allowed to decide an architecture is listed here, and
/// only [`POLICY`] may actually contain the decision. Adding a file to this
/// list is not how you fix a failure — routing it through `arch_policy` is.
const SCANNED: &[&str] = &[
    "crates/core/src/system/node.rs",
    "crates/loader/src/lib.rs",
    "crates/wasm/src/lib.rs",
    // Added because the compiler, not the author, found an architecture
    // literal here. Any file that names one belongs in this list.
    "crates/wasm/src/inputs.rs",
];

#[test]
fn only_one_file_maps_an_elf_machine_to_an_arch() {
    let root = repo_root();
    let mut offenders = Vec::new();
    for rel in SCANNED {
        let src = code_only(&read(&root, rel));
        // Reading `e_machine` to hand it to the policy is correct use, so the
        // marker is naming a machine *constant* — that can only be part of a
        // second table. `94` is EM_XTENSA, which is how the loader's copy was
        // written and how it escaped a search for the constant.
        let maps = src.contains("EM_ARM")
            || src.contains("EM_RISCV")
            || src.contains("EM_XTENSA")
            || src.contains("94 =>");
        if maps {
            offenders.push(*rel);
        }
    }
    assert!(
        offenders.is_empty(),
        "{} file(s) decode an ELF machine type outside {}. Two decoders drift: one of them \
         spelled EM_XTENSA as the literal 94, and they disagreed about what an unrecognised \
         machine means (bail vs Arch::Unknown). Call `arch_policy::elf_arch` instead:\n  {}",
        offenders.len(),
        POLICY,
        offenders.join("\n  ")
    );
}

#[test]
fn only_one_file_dispatches_on_a_chip_arch() {
    let root = repo_root();
    let mut offenders = Vec::new();
    for rel in SCANNED {
        let src = code_only(&read(&root, rel));
        // A dispatch is a match arm naming a config Arch variant. Storing an
        // arch (`arch: Arch::Arm` in a struct literal) is not a dispatch, so
        // the marker is the arm form `Arch::X =>` / `Arch::X |`.
        let dispatches = ["Arm", "RiscV", "Xtensa", "Unknown"].iter().any(|v| {
            src.contains(&format!("Arch::{v} =>")) || src.contains(&format!("Arch::{v} |"))
        });
        if dispatches {
            offenders.push(*rel);
        }
    }
    assert!(
        offenders.is_empty(),
        "{} file(s) decide what machine a chip becomes outside {}. That fork is how the \
         browser came to run an `arch: unknown` chip as a Cortex-M while the CLI refused it. \
         Call `arch_policy::machine_family` instead:\n  {}",
        offenders.len(),
        POLICY,
        offenders.join("\n  ")
    );
}

fn chip_with_arch(arch: Arch) -> ChipDescriptor {
    let yaml = format!(
        "name: \"guard-fixture\"\narch: \"{}\"\nregisters_count: 16\n\
         flash:\n  base: 0x8000000\n  size: \"128KB\"\nram:\n  base: 0x20000000\n  size: \"64KB\"\n\
         peripherals: []\n",
        match arch {
            Arch::Arm => "arm",
            Arch::RiscV => "riscv",
            Arch::Xtensa => "xtensa",
            Arch::Unknown => "unknown",
        }
    );
    serde_yaml::from_str(&yaml).expect("fixture chip must parse")
}

#[test]
fn an_undeclared_architecture_is_refused_not_guessed() {
    let err = machine_family(&chip_with_arch(Arch::Unknown))
        .expect_err("a chip that names no architecture must not silently become a Cortex-M");
    let msg = format!("{err:#}");
    assert!(
        msg.contains("architecture"),
        "the refusal must say what is wrong; got {msg:?}"
    );
}

#[test]
fn each_declared_architecture_reaches_its_own_family() {
    for (arch, want) in [
        (Arch::Arm, MachineFamily::CortexM),
        (Arch::RiscV, MachineFamily::RiscV),
        (Arch::Xtensa, MachineFamily::Xtensa),
        (Arch::Avr, MachineFamily::Avr),
    ] {
        let got = machine_family(&chip_with_arch(arch)).expect("declared arch must be accepted");
        assert_eq!(got, want, "chip arch {arch:?} reached the wrong family");
    }
}

#[test]
fn the_elf_machine_table_is_the_only_one() {
    assert_eq!(
        elf_arch(goblin::elf::header::EM_ARM),
        Some(crate::Arch::Arm)
    );
    assert_eq!(
        elf_arch(goblin::elf::header::EM_RISCV),
        Some(crate::Arch::RiscV)
    );
    assert_eq!(
        elf_arch(goblin::elf::header::EM_XTENSA),
        Some(crate::Arch::XtensaLx7)
    );
    // 94 is EM_XTENSA. The loader used to hard-code it; if goblin's constant
    // ever stopped being 94 the two copies would have silently disagreed.
    assert_eq!(goblin::elf::header::EM_XTENSA, 94);
    // An unrecognised machine has exactly one answer: None. Callers decide
    // whether that is fatal, but they no longer decide what it *means*.
    assert_eq!(elf_arch(0), None);
    assert_eq!(elf_arch(3), None); // EM_386 — real, but not modelled here
}

/// The repository ships exactly one chip that declares no architecture, and it
/// exists to prove this refusal. Asserting against the real file — not a
/// hand-built fixture — is what ties the guard to the thing that shipped.
#[test]
fn the_shipped_unknown_arch_fixture_is_refused() {
    let root = repo_root();
    let yaml = read(&root, "configs/chips/ci-fixture-unknown-arch.yaml");
    let chip: ChipDescriptor = serde_yaml::from_str(&yaml).expect("fixture chip must parse");
    assert_eq!(
        chip.arch,
        Arch::Unknown,
        "the fixture stopped declaring `arch: unknown`, so this test no longer proves anything"
    );
    assert!(
        machine_family(&chip).is_err(),
        "the browser used to build this chip as a Cortex-M while the CLI refused it"
    );
}

#[test]
fn the_scan_is_not_vacuous() {
    let root = repo_root();
    // The policy module must exist and carry both decisions, or every scan
    // above passes by finding nothing anywhere.
    let policy = read(&root, POLICY);
    assert!(
        policy.contains("EM_XTENSA") && policy.contains("Arch::Unknown"),
        "{POLICY} must hold both decisions; it is the only file exempt from the scans"
    );
    for rel in SCANNED {
        let src = read(&root, rel);
        assert!(
            src.len() > 1_000,
            "{rel} is {} bytes — too small to be the real file; the scan is reading the wrong path",
            src.len()
        );
    }
    // The comment stripper must actually strip, or a scan can be defeated by
    // a comment and — worse — pass while the code below it still forks.
    let stripped = code_only("let a = EM_ARM; // EM_XTENSA\n");
    assert!(
        stripped.contains("EM_ARM") && !stripped.contains("EM_XTENSA"),
        "the scan must read code and ignore comments; got {stripped:?}"
    );
}
