//! The BLE Pong flash fixture must stay bound to the sketch it was built from.
//!
//! `world_esp32c3_ble_pong` runs a COMMITTED flash image against two ESP32-C3
//! instances and calls it proof that "a real user's real sketch works". That
//! claim is only true while the image is actually the user's sketch, and there
//! is no way to check that at runtime: CI has no network, and the `.bin` does
//! not carry its own provenance. So the pairing is recorded here by hash.
//!
//! WHY THIS FILE EXISTS. It did not, and the gate it guards went vacuous. The
//! image was frozen at the labwired-core #828 build while the owner's published
//! project moved on repeatedly in a single day. `world_esp32c3_ble_pong` kept
//! passing the whole time, because it was exercising firmware that no longer
//! existed anywhere but in `tests/fixtures/`. Its docstring promised coverage
//! it did not have, which is worse than no test: it bought confidence that the
//! published lab worked while a panel was dead in the browser.
//!
//! Refreshing the fixture is what surfaced the bug. Against the then-current
//! sketch the world test failed immediately — the guest never received a world
//! frame, because `PUBLISH_MS` was 700 ms and the host's only advertisement in
//! that window was the guest-shaped one from `setup()`. Same engine, old
//! firmware green, new firmware red. A gate that tracks the real artifact finds
//! things; a gate pinned to a convenient old one cannot.
//!
//! DELIBERATELY NOT `cfg`-GATED. The test it protects is release-only for cost
//! (two mask-ROM boots, ~180M cycles) and therefore runs on push-to-main, not on
//! PRs. Hashing two files costs milliseconds, so this one runs everywhere —
//! `pr-gate` names it explicitly and `core-integrity`'s `cargo test -p
//! labwired-core` picks it up — and a PR that edits the sketch is told before
//! the merge, not after.
//!
//! SCOPE. Two hashes cannot prove the `.bin` is the compiler's output for the
//! `.ino`; nothing short of compiling can. What they do prove is that the pair
//! is the pair somebody last recorded, so neither file can move alone and
//! silently. The only way to land a NEW pair is to actually rebuild the image
//! (recipe in the failure message).

use std::path::PathBuf;

fn fixtures() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hasher
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// The owner's published sketch, verbatim: api.labwired.com project
/// `c477f82961e86f601e7b908ae7e12311`, field `source_code` (identical to
/// `firmware[0].source_code` — the lab's premise is ONE image on both chips).
const SKETCH: &str = "esp32c3-ble-pong.ino";
const SKETCH_SHA256: &str = "3b4d793e9b129784e94002a04dd00dc1ba0a5191a8700e413a47d2301d69245f";

/// The merged 4 MB flash the hosted PlatformIO toolchain builds from `SKETCH`.
const FLASH: &str = "esp32c3-ble-pong-flash.bin";
const FLASH_SHA256: &str = "1789ebcb62fcd6d72dd4eb88718a8efd2c6da74ff5df0793509e3892653f66e0";

/// How to mint a new pair. Printed on failure, because a hash mismatch with no
/// recipe just gets the constant hand-edited to whatever the file now hashes
/// to — which re-opens exactly the hole this file closes.
const REBUILD: &str = "\
To change the sketch, rebuild the image and update BOTH constants together:
  1. labwired_compile, board `esp32-c3-supermini`, language `arduino`,
     lib_deps `adafruit/Adafruit SSD1306` + `adafruit/Adafruit GFX Library`.
  2. Concatenate the returned flash_image_refs at their offsets
     (0, 0x8000, 0xe000, 0x10000), padding the gaps with 0xFF.
  3. Write it to tests/fixtures/esp32c3-ble-pong-flash.bin and put the new
     sha256 in FLASH_SHA256 (and the sketch's in SKETCH_SHA256).
  4. Re-run: cargo test --release -p labwired-core --features event-scheduler \
--test world_esp32c3_ble_pong
NEVER update one constant alone. A sketch with no matching image means
world_esp32c3_ble_pong is testing firmware nobody runs.";

#[test]
fn ble_pong_sketch_matches_its_recorded_hash() {
    let path = fixtures().join(SKETCH);
    let bytes = std::fs::read(&path).expect("read committed BLE Pong sketch");
    let got = sha256_hex(&bytes);
    assert_eq!(
        got, SKETCH_SHA256,
        "\n{SKETCH} changed but SKETCH_SHA256 did not.\n  expected {SKETCH_SHA256}\n  actual   {got}\n{REBUILD}\n"
    );
}

#[test]
fn ble_pong_flash_image_matches_its_recorded_hash() {
    let path = fixtures().join(FLASH);
    let bytes = std::fs::read(&path).expect("read committed BLE Pong flash image");
    let got = sha256_hex(&bytes);
    assert_eq!(
        got, FLASH_SHA256,
        "\n{FLASH} changed but FLASH_SHA256 did not.\n  expected {FLASH_SHA256}\n  actual   {got}\n{REBUILD}\n"
    );
}

/// The image has to be a plausible ESP32-C3 flash before the world test spends
/// two mask-ROM boots discovering otherwise: `0xE9` image magic for the
/// bootloader at 0 and the application at 0x10000. A truncated or
/// wrong-offset concatenation is the likeliest way to get the rebuild wrong.
#[test]
fn ble_pong_flash_image_has_bootloader_and_app_at_the_expected_offsets() {
    let bytes = std::fs::read(fixtures().join(FLASH)).expect("read committed BLE Pong flash image");
    assert!(
        bytes.len() > 0x10000,
        "{FLASH} is {} bytes — too short to hold an app at 0x10000.\n{REBUILD}",
        bytes.len()
    );
    assert_eq!(
        bytes[0], 0xE9,
        "no ESP image magic at offset 0 — the bootloader segment is missing.\n{REBUILD}"
    );
    assert_eq!(
        bytes[0x10000], 0xE9,
        "no ESP image magic at offset 0x10000 — the app segment is missing or \
         misplaced.\n{REBUILD}"
    );
}
