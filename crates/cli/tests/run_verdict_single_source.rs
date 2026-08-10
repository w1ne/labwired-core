// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! A run has ONE verdict, and `result.json` and the process exit code are two
//! views of it.
//!
//! `docs/simulation_protocol.md` §5 is the contract these tests are derived
//! from — not the runner's own source. It defines the exit codes a LabWired CI
//! runner may produce and what a consumer must do with each:
//!
//! | Exit | Constant            | Protocol Action                    |
//! |------|---------------------|------------------------------------|
//! | `0`  | `EXIT_PASS`         | Treat as CI Success.               |
//! | `1`  | `EXIT_ASSERT_FAIL`  | Treat as CI Failure (Logic Error). |
//! | `2`  | `EXIT_CONFIG_ERROR` | Fix configuration inputs.          |
//! | `3`  | `EXIT_RUNTIME_ERROR`| Report issue.                      |
//!
//! `result.json`'s `status` is the same judgment spelled `"pass"` / `"fail"` /
//! `"error"` (§4.1). The two therefore cannot disagree: a run whose artifact
//! says `"fail"` and whose process says "CI Success" has told two different
//! stories about one piece of silicon, and whichever the harness reads is a
//! coin toss. `docs/simulation_protocol.md` gives no licence for the exit code
//! and the status to be computed from different evidence.
//!
//! `simctl_firmware_verdict_e2e.rs` already covers the `status` side of these
//! same runs. It never inspects the process exit code, so it stayed green
//! while the two views disagreed — which is the whole reason this file exists.

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
/// ```text
///   0x0000  .word 0x20001000      ; initial SP
///   0x0004  .word 0x00000101      ; reset vector (Thumb bit set)
///   0x0100  LDR  r0, [pc, #8]     ; r0 = target   (literal @ 0x10c)
///   0x0102  LDR  r1, [pc, #12]    ; r1 = value    (literal @ 0x110)
///   0x0104  STR  r1, [r0, #0]     ; the write the whole feature rests on
///   0x0106  B    .                ; spin
///   0x010c  .word target
///   0x0110  .word value
/// ```
fn build_firmware(target: u32, value: u32) -> Vec<u8> {
    let mut image = vec![0u8; 0x114];

    image[0x00..0x04].copy_from_slice(&0x2000_1000u32.to_le_bytes()); // initial SP
    image[0x04..0x08].copy_from_slice(&0x0000_0101u32.to_le_bytes()); // reset, Thumb

    image[0x100..0x102].copy_from_slice(&0x4802u16.to_le_bytes()); // LDR r0,[pc,#8]
    image[0x102..0x104].copy_from_slice(&0x4903u16.to_le_bytes()); // LDR r1,[pc,#12]
    image[0x104..0x106].copy_from_slice(&0x6001u16.to_le_bytes()); // STR r1,[r0]
    image[0x106..0x108].copy_from_slice(&0xE7FEu16.to_le_bytes()); // B .

    image[0x10c..0x110].copy_from_slice(&target.to_le_bytes());
    image[0x110..0x114].copy_from_slice(&value.to_le_bytes());

    wrap_in_elf(&image)
}

/// Wrap a flat image in the smallest ELF32/ARM the loader accepts.
fn wrap_in_elf(image: &[u8]) -> Vec<u8> {
    const EHDR: u32 = 52;
    const PHDR: u32 = 32;
    let offset = EHDR + PHDR;

    let mut elf = Vec::new();
    elf.extend_from_slice(&[0x7f, b'E', b'L', b'F', 1, 1, 1, 0]);
    elf.extend_from_slice(&[0; 8]);
    elf.extend_from_slice(&2u16.to_le_bytes()); // e_type = ET_EXEC
    elf.extend_from_slice(&40u16.to_le_bytes()); // e_machine = EM_ARM
    elf.extend_from_slice(&1u32.to_le_bytes());
    elf.extend_from_slice(&0x101u32.to_le_bytes()); // e_entry (Thumb)
    elf.extend_from_slice(&EHDR.to_le_bytes()); // e_phoff
    elf.extend_from_slice(&0u32.to_le_bytes());
    elf.extend_from_slice(&0u32.to_le_bytes());
    elf.extend_from_slice(&(EHDR as u16).to_le_bytes());
    elf.extend_from_slice(&(PHDR as u16).to_le_bytes());
    elf.extend_from_slice(&1u16.to_le_bytes()); // e_phnum
    elf.extend_from_slice(&40u16.to_le_bytes());
    elf.extend_from_slice(&0u16.to_le_bytes());
    elf.extend_from_slice(&0u16.to_le_bytes());

    elf.extend_from_slice(&1u32.to_le_bytes()); // PT_LOAD
    elf.extend_from_slice(&offset.to_le_bytes());
    elf.extend_from_slice(&0u32.to_le_bytes());
    elf.extend_from_slice(&0u32.to_le_bytes());
    elf.extend_from_slice(&(image.len() as u32).to_le_bytes());
    elf.extend_from_slice(&(image.len() as u32).to_le_bytes());
    elf.extend_from_slice(&5u32.to_le_bytes()); // R|X
    elf.extend_from_slice(&4u32.to_le_bytes());

    elf.extend_from_slice(image);
    elf
}

const SIMCTL_BASE: u32 = 0x6000_0000;
const EXIT_OFFSET: u32 = 0x00;

/// What the protocol table permits. Exactly one exit code per status, and
/// exactly one status per exit code — this mapping IS the single-verdict claim.
///
/// `"error"` covers both `EXIT_CONFIG_ERROR` (2) and `EXIT_RUNTIME_ERROR` (3);
/// the protocol distinguishes their causes but calls both an `"error"` status,
/// so this direction of the mapping is one-to-many and checked as such.
fn assert_one_verdict(
    case: &str,
    status: &str,
    exit_code: Option<i32>,
    result: &serde_json::Value,
) {
    let code = exit_code.unwrap_or_else(|| panic!("[{case}] runner was killed by a signal"));
    let permitted: &[i32] = match status {
        "pass" => &[0],
        "fail" => &[1],
        "error" => &[2, 3],
        other => panic!("[{case}] result.json status {other:?} is not in the protocol vocabulary"),
    };
    assert!(
        permitted.contains(&code),
        "[{case}] ONE run, TWO verdicts: result.json says status={status:?} but the process \
         exited {code}. docs/simulation_protocol.md §5 permits {permitted:?} for that status. \
         A CI harness reading the exit code and a dashboard reading result.json would report \
         opposite outcomes for the same silicon.\nresult.json:\n{result:#}"
    );
}

struct RunOutput {
    result: serde_json::Value,
    exit_code: Option<i32>,
}

/// Everything one case needs on disk: firmware, a board declaring `simctl`, and
/// a script. `extra` is appended to the script verbatim (faults, verdict, ...);
/// `faults`/`verdict` are a schema 1.1 feature, so cases that use them say so.
fn stage_case(
    name: &str,
    exit_code: u32,
    assertions: &str,
    schema_version: &str,
    extra: &str,
) -> (PathBuf, PathBuf) {
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
            r#"schema_version: "{schema_version}"
inputs:
  firmware: "{}"
  system: "{}"
limits:
  max_steps: 100000
assertions:
{assertions}
{extra}"#,
            fw_path.display(),
            system_path.display()
        ),
    )
    .expect("write script");

    let output_dir = labwired_cli::test_support::unique_temp_dir(&format!("labwired-{name}"));
    let _ = std::fs::remove_dir_all(&output_dir);
    (script_path, output_dir)
}

fn run_case(script: &Path, output_dir: &Path) -> RunOutput {
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
    RunOutput {
        result: serde_json::from_str(&body).expect("result.json is valid JSON"),
        exit_code: output.status.code(),
    }
}

// ---------------------------------------------------------------------------
// The two directions the fork ran in.
// ---------------------------------------------------------------------------

/// Firmware declares failure through `simctl` and the script asserts nothing.
///
/// `simctl_firmware_verdict_e2e::a_nonzero_exit_fails_the_run_even_with_no_assertions`
/// already pins `status == "fail"` here. The exit code was computed from a
/// different chain that had no `firmware_declared_failure` term at all, so the
/// process reported CI Success for firmware that said it had failed.
#[test]
fn a_firmware_declared_failure_fails_the_process_too() {
    let (script, out) = stage_case("verdict-fw-exit", 5, "  []", "1.0", "");
    let run = run_case(&script, &out);

    assert_eq!(
        run.result["firmware_exit_code"], 5,
        "precondition: the firmware verdict must reach the result"
    );
    assert_eq!(
        run.result["status"], "fail",
        "precondition: a non-zero firmware exit fails the run"
    );
    assert_one_verdict(
        "firmware exited 5, no assertions",
        run.result["status"].as_str().unwrap(),
        run.exit_code,
        &run.result,
    );
}

/// The opposite direction: a `require_fault_fired` gate that trips.
///
/// The fault targets `uart1`, which this firmware never touches, so it can
/// never fire and the run is invalid. The exit-code chain knew that; the
/// `status` chain was computed thirty lines earlier, before `fault_gate_failed`
/// existed, so the artifact certified a pass while the process failed.
#[test]
fn a_fault_that_never_fired_fails_the_artifact_too() {
    let (script, out) = stage_case(
        "verdict-fault-gate",
        0,
        "  - firmware_exit: 0",
        "1.1",
        r#"faults:
  - id: "uart1-unclocked"
    kind: "missing_clock"
    target:
      peripheral: "uart1"
verdict:
  require_fault_fired: true
"#,
    );
    let run = run_case(&script, &out);

    assert_eq!(
        run.result["firmware_exit_code"], 0,
        "precondition: the firmware itself ran to a clean exit"
    );
    assert_one_verdict(
        "require_fault_fired gate tripped",
        run.result["status"].as_str().unwrap(),
        run.exit_code,
        &run.result,
    );
}

// ---------------------------------------------------------------------------
// ANTI-VACUITY CONTROLS. These pass on unmodified main. If a change to the
// verdict makes THESE fail, the collapse broke the ordinary paths.
// ---------------------------------------------------------------------------

#[test]
fn a_clean_run_agrees_on_success() {
    let (script, out) = stage_case("verdict-clean", 0, "  - firmware_exit: 0", "1.0", "");
    let run = run_case(&script, &out);

    assert_eq!(run.result["status"], "pass", "result: {}", run.result);
    assert_one_verdict(
        "firmware exited 0, assertion matched",
        run.result["status"].as_str().unwrap(),
        run.exit_code,
        &run.result,
    );
}

#[test]
fn a_failed_assertion_agrees_on_failure() {
    let (script, out) = stage_case("verdict-assert-fail", 7, "  - firmware_exit: 0", "1.0", "");
    let run = run_case(&script, &out);

    assert_eq!(run.result["status"], "fail", "result: {}", run.result);
    assert_one_verdict(
        "firmware exited 7, assertion demanded 0",
        run.result["status"].as_str().unwrap(),
        run.exit_code,
        &run.result,
    );
}
