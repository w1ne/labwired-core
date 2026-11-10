// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Ordered UART checks must observe later bytes. Mixed memory checks must also
//! observe state changes after the UART has stopped growing. These cases run
//! real Thumb instructions through the CLI, rather than testing the cache's
//! classification table. The complete thermal scenarios cover the long-run
//! performance regression caused by evaluating ordered checks every step.

use serde_json::Value;
use std::path::PathBuf;
use std::process::Command;

/// Emit A, execute eight NOPs, emit B, execute eight more NOPs, write 42 to
/// RAM, then spin. Distinct writes exercise invalidation after an initially
/// false UART verdict; the later RAM write has no accompanying UART byte.
fn firmware() -> Vec<u8> {
    let mut image = vec![0u8; 0x13c];
    image[..4].copy_from_slice(&0x2000_1000u32.to_le_bytes());
    image[4..8].copy_from_slice(&0x101u32.to_le_bytes());
    let code: [u16; 25] = [
        0x480c, // LDR r0, [pc, #48] -> UART DR literal at 0x134
        0x2141, // MOVS r1, #'A'
        0x7001, // STRB r1, [r0]
        0xbf00, 0xbf00, 0xbf00, 0xbf00, 0xbf00, 0xbf00, 0xbf00, 0xbf00,
        0x2142, // MOVS r1, #'B'
        0x7001, // STRB r1, [r0]
        0xbf00, 0xbf00, 0xbf00, 0xbf00, 0xbf00, 0xbf00, 0xbf00, 0xbf00,
        0x4803, // LDR r0, [pc, #12] -> RAM literal at 0x138
        0x212a, // MOVS r1, #42
        0x6001, // STR r1, [r0]
        0xe7fe, // B .
    ];
    for (index, op) in code.iter().enumerate() {
        image[0x100 + index * 2..0x102 + index * 2].copy_from_slice(&op.to_le_bytes());
    }
    image[0x134..0x138].copy_from_slice(&0x4000_c004u32.to_le_bytes());
    image[0x138..0x13c].copy_from_slice(&0x2000_0000u32.to_le_bytes());

    // One executable PT_LOAD segment; no cross compiler or checked-in binary
    // is needed to keep this small instruction sequence reproducible.
    let mut elf = Vec::new();
    elf.extend_from_slice(&[0x7f, b'E', b'L', b'F', 1, 1, 1, 0]);
    elf.extend_from_slice(&[0; 8]);
    elf.extend_from_slice(&2u16.to_le_bytes()); // ET_EXEC
    elf.extend_from_slice(&40u16.to_le_bytes()); // EM_ARM
    elf.extend_from_slice(&1u32.to_le_bytes());
    elf.extend_from_slice(&0x101u32.to_le_bytes());
    elf.extend_from_slice(&52u32.to_le_bytes()); // program-header offset
    elf.extend_from_slice(&0u32.to_le_bytes()); // section-header offset
    elf.extend_from_slice(&0u32.to_le_bytes()); // flags
    for value in [52u16, 32, 1, 40, 0, 0] {
        elf.extend_from_slice(&value.to_le_bytes());
    }
    for value in [1u32, 84, 0, 0, image.len() as u32, image.len() as u32, 5, 4] {
        elf.extend_from_slice(&value.to_le_bytes());
    }
    elf.extend_from_slice(&image);
    elf
}

fn run(name: &str, assertions: &str) -> (i32, Value) {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let dir = labwired_cli::test_support::unique_temp_dir(name);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("fixture.elf"), firmware()).unwrap();
    let system = root
        .join("configs/systems/ci-fixture-uart1.yaml")
        .canonicalize()
        .unwrap();
    std::fs::write(
        dir.join("test.yaml"),
        format!(
            "schema_version: \"1.0\"\ninputs:\n  firmware: ./fixture.elf\n  system: {}\n\
             limits:\n  max_steps: 1000\n  stop_when_assertions_pass: true\n  \
             stop_when_assertions_pass_settle_steps: 0\nassertions:\n{assertions}\n",
            serde_json::to_string(&system.to_string_lossy()).unwrap(),
        ),
    )
    .unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_labwired"))
        .arg("test")
        .arg("--script")
        .arg(dir.join("test.yaml"))
        .arg("--output-dir")
        .arg(&dir)
        .arg("--no-uart-stdout")
        .output()
        .unwrap();
    let result: Value = serde_json::from_slice(
        &std::fs::read(dir.join("result.json")).unwrap_or_else(|error| {
            panic!(
                "missing result: {error}; stderr={}",
                String::from_utf8_lossy(&output.stderr)
            )
        }),
    )
    .unwrap();
    assert_eq!(std::fs::read_to_string(dir.join("uart.log")).unwrap(), "AB");
    std::fs::remove_dir_all(dir).unwrap();
    (output.status.code().unwrap(), result)
}

#[test]
fn new_uart_bytes_complete_an_initially_false_ordered_check() {
    let (exit, result) = run("ordered-new-data", "  - uart_ordered: [A, B]");
    assert_eq!(exit, 0);
    assert_eq!(result["status"], "pass");
    assert_eq!(result["stop_reason"], "assertions_passed");
    assert!(result["steps_executed"].as_u64().unwrap() < 1000);
}

fn assert_rejected(name: &str, tokens: &str) {
    let (exit, result) = run(name, &format!("  - uart_ordered: {tokens}"));
    assert_eq!(exit, 1);
    assert_eq!(result["status"], "fail");
    assert_eq!(result["stop_reason"], "max_steps");
    assert_eq!(result["assertions"][0]["passed"], false);
}

#[test]
fn wrong_uart_token_cannot_pass() {
    assert_rejected("ordered-wrong", "[A, X]");
}

#[test]
fn missing_uart_suffix_cannot_pass() {
    assert_rejected("ordered-missing", "[A, B, C]");
}

#[test]
fn reversed_uart_tokens_cannot_pass() {
    assert_rejected("ordered-reversed", "[B, A]");
}

#[test]
fn mixed_memory_assertion_observes_change_after_last_uart_byte() {
    let (_, uart_only) = run("ordered-uart-baseline", "  - uart_ordered: [A, B]");
    let (exit, mixed) = run(
        "ordered-and-memory",
        "  - uart_ordered: [A, B]\n  - memory_value: {address: 536870912, expected_value: 42}",
    );
    assert_eq!(exit, 0);
    assert_eq!(mixed["status"], "pass");
    assert_eq!(mixed["stop_reason"], "assertions_passed");
    assert_eq!(mixed["assertions"][1]["passed"], true);
    assert!(
        mixed["steps_executed"].as_u64().unwrap() > uart_only["steps_executed"].as_u64().unwrap(),
        "RAM changes only after the UART sequence has completed",
    );
}
