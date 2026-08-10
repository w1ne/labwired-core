// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! The `simctl` verdict, end to end through the real `labwired` binary.
//!
//! Everything else about this feature is tested one layer down: the device in
//! `peripherals::simctl`, the drain in `Machine::advance`, the verdict mapping
//! in `simctl_verdict_tests`. All of those would stay green if the runner threw
//! the verdict away — which is exactly what it did before this change, because
//! `Machine::step` discards the `AdvanceReport`.
//!
//! So these tests spawn the actual CLI, with real ARM firmware that writes to
//! the device, and read `result.json`. Nothing is mocked and no internal API is
//! reached for: this is the path a user's `labwired test` takes.

use std::path::{Path, PathBuf};
use std::process::Command;

/// Repository root, from this crate's manifest directory.
fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("canonicalize workspace root")
}

fn temp_dir() -> PathBuf {
    let dir = std::env::temp_dir().join("labwired-tests");
    std::fs::create_dir_all(&dir).expect("create temp dir");
    dir
}

/// Assemble a minimal Cortex-M ELF whose only job is to store `value` to
/// `target`, then spin.
///
/// Built here rather than checked in: a binary blob in the repo could not be
/// reviewed, and the layout below IS the documentation.
///
/// ```text
///   0x0000  .word 0x20001000      ; initial SP
///   0x0004  .word 0x00000101      ; reset vector (Thumb bit set)
///   0x0100  LDR  r0, [pc, #8]     ; r0 = target   (literal @ 0x10c)
///   0x0102  LDR  r1, [pc, #12]    ; r1 = value    (literal @ 0x110)
///   0x0104  STR  r1, [r0, #0]     ; the write the whole feature rests on
///   0x0106  B    .                ; spin — reaching here means simctl did nothing
///   0x010c  .word target
///   0x0110  .word value
/// ```
fn build_firmware(target: u32, value: u32) -> Vec<u8> {
    let mut image = vec![0u8; 0x114];

    // Vector table.
    image[0x00..0x04].copy_from_slice(&0x2000_1000u32.to_le_bytes()); // initial SP
    image[0x04..0x08].copy_from_slice(&0x0000_0101u32.to_le_bytes()); // reset, Thumb

    // Code at 0x100. PC-relative literal loads read from (pc+4) & !3.
    image[0x100..0x102].copy_from_slice(&0x4802u16.to_le_bytes()); // LDR r0,[pc,#8]
    image[0x102..0x104].copy_from_slice(&0x4903u16.to_le_bytes()); // LDR r1,[pc,#12]
    image[0x104..0x106].copy_from_slice(&0x6001u16.to_le_bytes()); // STR r1,[r0]
    image[0x106..0x108].copy_from_slice(&0xE7FEu16.to_le_bytes()); // B .

    // Literal pool.
    image[0x10c..0x110].copy_from_slice(&target.to_le_bytes());
    image[0x110..0x114].copy_from_slice(&value.to_le_bytes());

    wrap_in_elf(&image)
}

/// Wrap a flat image in the smallest ELF32/ARM the loader accepts: one PT_LOAD
/// segment at address 0, which is where the fixture chip's flash lives.
fn wrap_in_elf(image: &[u8]) -> Vec<u8> {
    const EHDR: u32 = 52;
    const PHDR: u32 = 32;
    let offset = EHDR + PHDR;

    let mut elf = Vec::new();
    elf.extend_from_slice(&[0x7f, b'E', b'L', b'F', 1, 1, 1, 0]); // magic, 32-bit LE
    elf.extend_from_slice(&[0; 8]); // padding
    elf.extend_from_slice(&2u16.to_le_bytes()); // e_type = ET_EXEC
    elf.extend_from_slice(&40u16.to_le_bytes()); // e_machine = EM_ARM
    elf.extend_from_slice(&1u32.to_le_bytes()); // e_version
    elf.extend_from_slice(&0x101u32.to_le_bytes()); // e_entry (Thumb)
    elf.extend_from_slice(&EHDR.to_le_bytes()); // e_phoff
    elf.extend_from_slice(&0u32.to_le_bytes()); // e_shoff
    elf.extend_from_slice(&0u32.to_le_bytes()); // e_flags
    elf.extend_from_slice(&(EHDR as u16).to_le_bytes()); // e_ehsize
    elf.extend_from_slice(&(PHDR as u16).to_le_bytes()); // e_phentsize
    elf.extend_from_slice(&1u16.to_le_bytes()); // e_phnum
    elf.extend_from_slice(&40u16.to_le_bytes()); // e_shentsize
    elf.extend_from_slice(&0u16.to_le_bytes()); // e_shnum
    elf.extend_from_slice(&0u16.to_le_bytes()); // e_shstrndx

    elf.extend_from_slice(&1u32.to_le_bytes()); // p_type = PT_LOAD
    elf.extend_from_slice(&offset.to_le_bytes()); // p_offset
    elf.extend_from_slice(&0u32.to_le_bytes()); // p_vaddr
    elf.extend_from_slice(&0u32.to_le_bytes()); // p_paddr — the loader uses LMA
    elf.extend_from_slice(&(image.len() as u32).to_le_bytes()); // p_filesz
    elf.extend_from_slice(&(image.len() as u32).to_le_bytes()); // p_memsz
    elf.extend_from_slice(&5u32.to_le_bytes()); // p_flags = R|X
    elf.extend_from_slice(&4u32.to_le_bytes()); // p_align

    elf.extend_from_slice(image);
    elf
}

const SIMCTL_BASE: u32 = 0x6000_0000;
const EXIT_OFFSET: u32 = 0x00;

/// Everything one case needs on disk: firmware, a board declaring `simctl`, and
/// a script. Returns the script path and the output directory.
fn stage_case(name: &str, exit_code: u32, assertions: &str) -> (PathBuf, PathBuf) {
    let unique = labwired_cli::test_support::unique_name(name);
    let dir = temp_dir();

    let fw_path = dir.join(format!("{unique}.elf"));
    std::fs::write(
        &fw_path,
        build_firmware(SIMCTL_BASE + EXIT_OFFSET, exit_code),
    )
    .expect("write firmware");

    let chip = workspace_root().join("configs/chips/ci-fixture-cortex-m3-uart1.yaml");
    let system_path = dir.join(format!("{unique}-system.yaml"));
    std::fs::write(
        &system_path,
        format!(
            r#"name: "{unique}-system"
chip: "{}"
peripherals:
  - id: "simctl"
    type: "simctl"
    base_address: 0x60000000
    size: "32"
"#,
            chip.display()
        ),
    )
    .expect("write system manifest");

    let script_path = dir.join(format!("{unique}.yaml"));
    std::fs::write(
        &script_path,
        format!(
            r#"schema_version: "1.0"
inputs:
  firmware: "{}"
  system: "{}"
limits:
  max_steps: 100000
assertions:
{assertions}
"#,
            fw_path.display(),
            system_path.display()
        ),
    )
    .expect("write script");

    let output_dir = labwired_cli::test_support::unique_temp_dir(&format!("labwired-{name}"));
    let _ = std::fs::remove_dir_all(&output_dir);
    (script_path, output_dir)
}

fn run_case(script: &Path, output_dir: &Path) -> serde_json::Value {
    let output = Command::new(env!("CARGO_BIN_EXE_labwired"))
        .args([
            "test",
            "--script",
            script.to_str().unwrap(),
            "--no-uart-stdout",
            "--output-dir",
            output_dir.to_str().unwrap(),
        ])
        .output()
        .expect("run the labwired binary");

    let result_path = output_dir.join("result.json");
    let body = std::fs::read_to_string(&result_path).unwrap_or_else(|e| {
        panic!(
            "no result.json at {}: {e}\nstdout:\n{}\nstderr:\n{}",
            result_path.display(),
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )
    });
    serde_json::from_str(&body).expect("result.json is valid JSON")
}

#[test]
fn firmware_that_exits_zero_passes_and_reports_its_code() {
    let (script, out) = stage_case("simctl-pass", 0, "  - firmware_exit: 0");
    let result = run_case(&script, &out);

    assert_eq!(
        result["stop_reason"], "firmware_exit",
        "the run must end because the FIRMWARE said so, not on a limit; got {}",
        result["stop_reason"]
    );
    assert_eq!(
        result["firmware_exit_code"], 0,
        "the exit code must reach the run result"
    );
    assert_eq!(result["status"], "pass", "result: {result}");
}

#[test]
fn the_assertion_fails_when_the_code_is_not_the_one_asserted() {
    // Firmware exits 7; the script demands 0.
    let (script, out) = stage_case("simctl-wrong-code", 7, "  - firmware_exit: 0");
    let result = run_case(&script, &out);

    assert_eq!(result["firmware_exit_code"], 7);
    assert_eq!(
        result["status"], "fail",
        "asserting exit 0 against a run that exited 7 must fail; result: {result}"
    );
}

#[test]
fn a_nonzero_exit_fails_the_run_even_with_no_assertions() {
    // The silent-pass trap: with an empty assertion list the runner has nothing
    // to judge, so firmware that declared failure would otherwise report pass.
    let (script, out) = stage_case("simctl-bare-fail", 5, "  []");
    let result = run_case(&script, &out);

    assert_eq!(result["firmware_exit_code"], 5);
    assert_eq!(
        result["status"], "fail",
        "firmware that exits non-zero must fail the run on its own; result: {result}"
    );
}

#[test]
fn a_run_with_no_simctl_reports_no_exit_code() {
    // ANTI-VACUITY CONTROL. Same firmware, board WITHOUT the device: the store
    // lands in unmapped space, nothing ends the run, and the field must be
    // absent rather than defaulting to something a harness could misread.
    let unique = labwired_cli::test_support::unique_name("simctl-absent");
    let dir = temp_dir();

    let fw_path = dir.join(format!("{unique}.elf"));
    std::fs::write(&fw_path, build_firmware(SIMCTL_BASE + EXIT_OFFSET, 0)).unwrap();

    let system_path = workspace_root().join("configs/systems/ci-fixture-uart1.yaml");
    let script_path = dir.join(format!("{unique}.yaml"));
    std::fs::write(
        &script_path,
        format!(
            r#"schema_version: "1.0"
inputs:
  firmware: "{}"
  system: "{}"
limits:
  max_steps: 5000
assertions: []
"#,
            fw_path.display(),
            system_path.display()
        ),
    )
    .unwrap();

    let output_dir = labwired_cli::test_support::unique_temp_dir("labwired-simctl-absent");
    let _ = std::fs::remove_dir_all(&output_dir);
    let result = run_case(&script_path, &output_dir);

    assert_ne!(
        result["stop_reason"], "firmware_exit",
        "with no simctl on the board nothing may report a firmware verdict"
    );
    assert!(
        result.get("firmware_exit_code").is_none(),
        "firmware_exit_code must be absent, not null or zero; result: {result}"
    );
}
