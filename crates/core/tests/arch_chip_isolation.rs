// Architecture guard: a chip's peripheral models must be SELF-CONTAINED.
//
// Each `crates/core/src/peripherals/<chip>/` directory models one SoC family's
// silicon. A chip's peripheral must NOT reach into another chip's peripheral
// module (e.g. classic ESP32 using `esp32s3::gpio`) — that's a cross-chip
// fidelity leak: you'd be running another chip's register behavior. Generic,
// cross-family infrastructure (a ROM-thunk bank, a peripheral-RAM stub) is
// shared deliberately and tracked in ALLOWED below until it's relocated to a
// chip-neutral module.
//
// This test scans the source and fails if any chip module references another
// chip module's path, outside comments and outside the explicit allow-list.

use std::fs;
use std::path::Path;

/// Chip families whose peripheral dirs must stay mutually isolated.
const CHIPS: &[&str] = &["esp32", "esp32s3", "esp32c3", "nrf52"];

/// Known, deliberate cross-chip references (generic infra / test helpers),
/// each as `(file_suffix, referenced_path_fragment, why)`. Shrink this list as
/// the shared infrastructure is relocated to a chip-neutral module.
const ALLOWED: &[(&str, &str, &str)] = &[
    // S3's ROM-thunk-bank unit tests reuse the classic SPI register-bit consts
    // (REG_USER / USER_USR_MOSI_BIT). Test-only; harmless. Relocate the consts
    // to a shared module to drop this.
    (
        "esp_xtensa_common/rom_thunks.rs",
        "peripherals::esp32::spi::",
        "test-only: shared SPI register-bit constants",
    ),
    // ⚠️ NOT a test helper and NOT generic infra — a PRODUCTION peripheral model
    // shared across two CPU architectures. `esp32s3::wifi_mac` is a type alias
    // of `Esp32c3WifiMac` (#1019), so an edit to `esp32c3/wifi_mac.rs` ALSO
    // changes the ESP32-S3: one model, one register map, two chips.
    //
    // The "same WDEV IP" claim is NOT established in this repo. Every offset in
    // that model — MAC-ready `0xD14`, RX ring `0x88`, event `0xC3C`/`0xC40`,
    // PLCP0 `0xD08` — was reverse-engineered on live *C3* silicon (see
    // docs/esp32c3_radio_reverse_engineering.md, scripts/hw-oracle/wifi-re,
    // captures/esp32c3/wifi-radio-20260611T214340Z). There is no S3 radio
    // capture, and the vendored S3 SVD (tests/fixtures/svd/esp32s3.svd) carries
    // no Wi-Fi MAC peripheral at all. The one S3-corroborated claim is the
    // interrupt source: INTERRUPT_CORE0 `PRO_MAC_INTR_MAP` @ 0x000 ⇒ source 0.
    // validation/manifest.yaml already tracks the owed live capture of
    // 0x60033D14. Precedent for caution: esp32s3/interrupt_core0.yaml carried
    // the C3's map until 2026-08-20 and was wrong by 8/20/40 bytes.
    //
    // Drop this entry when the S3 gets its own model. That needs the virtual-
    // WiFi medium relocated to a chip-neutral module first: the MAC struct
    // holds an `esp32c3::virtual_wifi::VirtualWifiBus` by value, so a
    // self-contained S3 model is not reachable by copying wifi_mac.rs alone.
    (
        "esp32s3/wifi_mac.rs",
        "peripherals::esp32c3::wifi_mac::",
        "UNVERIFIED on S3 silicon: the S3 MAC is an alias of the C3 model (#1019)",
    ),
];

/// Strip a `//`-comment tail (best-effort; ignores `//` inside string literals,
/// which is acceptable for an import/path guard since paths don't embed `//`).
fn strip_comment(line: &str) -> &str {
    match line.find("//") {
        Some(i) => &line[..i],
        None => line,
    }
}

fn is_allowed(file_rel: &str, line: &str) -> bool {
    ALLOWED
        .iter()
        .any(|(suffix, frag, _)| file_rel.ends_with(suffix) && line.contains(frag))
}

fn scan_dir(dir: &Path, chip: &str, others: &[&str], violations: &mut Vec<String>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            scan_dir(&path, chip, others, violations);
            continue;
        }
        if path.extension().and_then(|e| e.to_str()) != Some("rs") {
            continue;
        }
        let Ok(src) = fs::read_to_string(&path) else {
            continue;
        };
        let file_rel = path
            .strip_prefix(env!("CARGO_MANIFEST_DIR"))
            .unwrap_or(&path)
            .to_string_lossy()
            .replace('\\', "/");
        for (lineno, raw) in src.lines().enumerate() {
            let code = strip_comment(raw);
            for other in others {
                let needle = format!("peripherals::{other}::");
                if code.contains(&needle) && !is_allowed(&file_rel, code) {
                    violations.push(format!(
                        "{file_rel}:{}: chip `{chip}` references `{other}`: {}",
                        lineno + 1,
                        raw.trim()
                    ));
                }
            }
        }
    }
}

#[test]
fn chip_peripheral_modules_are_self_contained() {
    let base = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/peripherals");
    let mut violations = Vec::new();
    for &chip in CHIPS {
        let dir = base.join(chip);
        if !dir.exists() {
            continue;
        }
        let others: Vec<&str> = CHIPS.iter().copied().filter(|&c| c != chip).collect();
        scan_dir(&dir, chip, &others, &mut violations);
    }
    assert!(
        violations.is_empty(),
        "cross-chip peripheral references found (a chip must not use another chip's \
         peripheral model; add to ALLOWED only for genuine shared infra):\n{}",
        violations.join("\n")
    );
}
