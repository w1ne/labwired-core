// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! DRIFT GUARD: `configs/chips/stm32u545.yaml` is a self-contained derivative
//! of `stm32u575.yaml` (built-in chips cannot `include:`). Same IP family, so
//! every peripheral present in both must be identical, and the only differences
//! are the explicit allow-list below, each justified by an ST source.
//!
//! A fix made to the U575 descriptor that is not mirrored into the U545 fails
//! here with a message naming the peripheral and field. To accept a real new
//! divergence, add it to the allow-list WITH its source; otherwise mirror the
//! change.

use labwired_config::ChipDescriptor;
use serde_json::Value;
use std::path::PathBuf;

/// Peripherals the STM32U545 does not have: absent from ST's STM32U545 SVD and
/// stm32u545xx.h (the U535/U545 die has no USART2, GPIOF or GPIOI).
const REMOVED_ON_U545: &[&str] = &["usart2", "gpiof", "gpioi"];

/// (peripheral id, field path) pairs that legitimately differ.
const ALLOWED_PERIPHERAL_DIFFS: &[(&str, &str)] = &[
    // DBGMCU IDCODE reset 0x10026455 in the U545 SVD (U575: 0x30016482).
    ("dbgmcu", "config.idcode"),
    // Factory flash-size word follows each part's mapped flash: 512 vs 2048 KiB.
    ("sysmem", "config.flash_size_kb"),
];

/// Top-level descriptor fields that legitimately differ.
const ALLOWED_TOP_LEVEL_DIFFS: &[&str] = &[
    "name",
    // 512 KiB flash (ST open pin data STM32U545RETxQ.xml Flash=512).
    "flash.size",
    // SRAM1+SRAM2 = 256 KiB contiguous (stm32u545xx.h SRAM1_SIZE+SRAM2_SIZE);
    // no SRAM3.
    "ram.size",
];

/// U575ZITxQ is LQFP144; U545RETxQ is LQFP64 (ST STM32_open_pin_data at
/// 7d1f1514ed5583ec5007ad91236b4e1d377295b1). Package pin maps flatten to a
/// field per pad, so accept the sourced delta by prefix instead of maintaining
/// 111 individual leaf paths.
const ALLOWED_TOP_LEVEL_DIFF_PREFIXES: &[&str] = &["pins."];

fn load(name: &str) -> Value {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../configs/chips")
        .join(format!("{name}.yaml"));
    let chip = ChipDescriptor::from_file(&path).unwrap_or_else(|e| panic!("{name}: {e:#}"));
    serde_json::to_value(chip).unwrap()
}

/// Flatten a JSON value into `path -> leaf` pairs (arrays/objects recurse).
fn flatten(prefix: &str, v: &Value, out: &mut Vec<(String, Value)>) {
    match v {
        Value::Object(m) => {
            for (k, x) in m {
                let p = if prefix.is_empty() {
                    k.clone()
                } else {
                    format!("{prefix}.{k}")
                };
                flatten(&p, x, out);
            }
        }
        _ => out.push((prefix.to_string(), v.clone())),
    }
}

fn diff(a: &Value, b: &Value) -> Vec<String> {
    let (mut fa, mut fb) = (Vec::new(), Vec::new());
    flatten("", a, &mut fa);
    flatten("", b, &mut fb);
    let mut keys: Vec<&String> = fa.iter().chain(fb.iter()).map(|(k, _)| k).collect();
    keys.sort();
    keys.dedup();
    let get =
        |f: &Vec<(String, Value)>, k: &str| f.iter().find(|(p, _)| p == k).map(|(_, v)| v.clone());
    keys.into_iter()
        .filter(|k| get(&fa, k) != get(&fb, k))
        .cloned()
        .collect()
}

fn by_id(chip: &Value) -> std::collections::BTreeMap<String, Value> {
    chip["peripherals"]
        .as_array()
        .expect("peripherals array")
        .iter()
        .map(|p| (p["id"].as_str().unwrap().to_string(), p.clone()))
        .collect()
}

#[test]
fn u545_peripherals_match_u575_except_allow_list() {
    let (u575, u545) = (load("stm32u575"), load("stm32u545"));
    let (p575, p545) = (by_id(&u575), by_id(&u545));
    let mut problems = Vec::new();

    for id in p545.keys() {
        if !p575.contains_key(id) {
            problems.push(format!(
                "peripheral `{id}` exists in stm32u545.yaml but not stm32u575.yaml"
            ));
        }
    }
    for (id, base) in &p575 {
        match p545.get(id) {
            None if REMOVED_ON_U545.contains(&id.as_str()) => {}
            None => problems.push(format!(
                "peripheral `{id}` is in stm32u575.yaml but missing from stm32u545.yaml \
                 (mirror it, or add it to REMOVED_ON_U545 with an ST source)"
            )),
            Some(derived) => {
                for field in diff(base, derived) {
                    if !ALLOWED_PERIPHERAL_DIFFS.contains(&(id.as_str(), field.as_str())) {
                        problems.push(format!(
                            "peripheral `{id}` field `{field}` differs between stm32u575.yaml \
                             and stm32u545.yaml (mirror the U575 change into the U545)"
                        ));
                    }
                }
            }
        }
    }
    for (id, field) in ALLOWED_PERIPHERAL_DIFFS {
        let (a, b) = (&p575[*id], &p545[*id]);
        assert!(
            diff(a, b).iter().any(|f| f == field),
            "stale allow-list entry: `{id}` `{field}` no longer differs"
        );
    }
    for id in REMOVED_ON_U545 {
        assert!(p575.contains_key(*id), "stale REMOVED_ON_U545 entry `{id}`");
        assert!(!p545.contains_key(*id), "`{id}` must stay removed on U545");
    }
    assert!(
        problems.is_empty(),
        "U545/U575 drift:\n  {}",
        problems.join("\n  ")
    );
}

#[test]
fn u545_top_level_matches_u575_except_allow_list() {
    let (mut u575, mut u545) = (load("stm32u575"), load("stm32u545"));
    for v in [&mut u575, &mut u545] {
        v.as_object_mut().unwrap().remove("peripherals");
    }
    let differences = diff(&u575, &u545);
    for prefix in ALLOWED_TOP_LEVEL_DIFF_PREFIXES {
        assert!(
            differences.iter().any(|field| field.starts_with(prefix)),
            "stale allow-list prefix: `{prefix}` no longer differs"
        );
    }
    let bad: Vec<String> = differences
        .into_iter()
        .filter(|f| {
            !ALLOWED_TOP_LEVEL_DIFFS.contains(&f.as_str())
                && !ALLOWED_TOP_LEVEL_DIFF_PREFIXES
                    .iter()
                    .any(|prefix| f.starts_with(prefix))
        })
        .map(|f| format!("top-level field `{f}` differs between stm32u575.yaml and stm32u545.yaml"))
        .collect();
    assert!(bad.is_empty(), "U545/U575 drift:\n  {}", bad.join("\n  "));
}

#[test]
fn u545_merged_memory_map_is_the_u545_one() {
    let chip = load("stm32u545");
    assert_eq!(chip["flash"]["base"], 0x0800_0000u64);
    assert_eq!(chip["flash"]["size"], 512u64 * 1024);
    assert_eq!(chip["ram"]["base"], 0x2000_0000u64);
    assert_eq!(chip["ram"]["size"], 256u64 * 1024);
    let regions = chip["memory_regions"].as_array().unwrap();
    assert_eq!(regions.len(), 2);
    assert_eq!(regions[0]["name"], "sram4");
    assert_eq!(regions[0]["base"], 0x2800_0000u64);
    assert_eq!(regions[0]["size"], 16u64 * 1024);
    assert_eq!(regions[1]["name"], "bkpsram");
    assert_eq!(regions[1]["base"], 0x4003_6400u64);
    assert_eq!(regions[1]["size"], 2u64 * 1024);
    let ids: Vec<String> = by_id(&chip).into_keys().collect();
    for gone in REMOVED_ON_U545 {
        assert!(
            !ids.contains(&gone.to_string()),
            "{gone} must not exist on U545"
        );
    }
    for need in [
        "usart1", "usart3", "uart4", "lpuart1", "gpioa", "gpioc", "gpiog", "gpioh", "flash",
        "spi1", "i2c1",
    ] {
        assert!(ids.contains(&need.to_string()), "{need} missing");
    }
}
