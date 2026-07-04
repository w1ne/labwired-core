// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT
//
// Faithfulness gate for the app-entry snapshot / resume fast-path.
//
// The premise: faithful ESP32-C3 rom-boot (mask ROM -> 2nd-stage bootloader
// -> SHA-verify -> app) costs ~150M steps before the application runs a single
// instruction. We want to pay that ONCE: cold-boot faithfully, snapshot the
// live machine the instant control reaches the app, and let later runs resume
// from that snapshot. This test proves the snapshot is a genuine cache of the
// REAL boot (not a hand-modeled handoff): the serial the app emits after a
// resume is byte-identical to continuing the cold boot from the same instant,
// and the resume reaches the app in dramatically fewer steps.
//
// `#[ignore]` because the cold boot alone is ~150M steps. Run with:
//   cargo test -p labwired-core --release --test e2e_esp32c3_snapshot_resume \
//       -- --ignored --nocapture

use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::boot::esp32c3_rom::{build_rom_boot_machine, inject_rom_regions, RomBootOpts};
use labwired_core::boot::esp32s3_rom::RomImages;
use labwired_core::bus::SystemBus;
use labwired_core::cpu::RiscV;
use labwired_core::runtime_snapshot::MachineRuntimeSnapshot;
use labwired_core::{Cpu, Machine};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// First PC in the XIP app window signals control has reached the application
/// (excludes ROM / DRAM / bootloader IRAM). This is the exact fallback the CLI
/// `--capture-app-entry` uses when no `call_start_cpu0` / `app_main` symbol is
/// available (here we only have the merged flash image, no ELF symbol table).
const APP_WINDOW: std::ops::Range<u32> = 0x4200_0000..0x4400_0000;

/// Build a fresh, faithful C3 rom-boot machine wired to `sink` for serial.
/// Identical assembly to the CLI `--rom-boot` path and the wasm rom-boot path
/// (the shared `build_rom_boot_machine` core), so its boot is the real thing.
fn build_c3(flash: Vec<u8>, sink: Arc<Mutex<Vec<u8>>>) -> Machine<RiscV> {
    let chip_yaml = root().join("../../configs/chips/esp32c3.yaml");
    let sys_yaml = root().join("../../configs/systems/esp32c3-devkit.yaml");
    let chip = ChipDescriptor::from_file(&chip_yaml).expect("load esp32c3 chip");
    let manifest = SystemManifest::from_file(&sys_yaml).expect("load esp32c3-devkit system");

    let mut bus = SystemBus::from_config(&chip, &manifest).expect("from_config");

    let irom = std::fs::read(root().join("roms/esp32c3/esp32c3_rom.bin")).expect("read C3 IROM");
    let drom = std::fs::read(root().join("roms/esp32c3/esp32c3_drom.bin")).expect("read C3 DROM");
    assert!(
        inject_rom_regions(&mut bus, &RomImages { irom, drom }),
        "chip YAML must declare an IROM region at 0x40000000 for the boot ROM"
    );

    // Single console sink (UART0), matching the native CLI `--rom-boot` path.
    // Do NOT also pass usb_serial_sink: the BROM/IDF mirror the console to both
    // UART0 and USB-Serial-JTAG, and routing both into one buffer doubles every
    // byte and interleaves nondeterministically — which would mask the exact
    // byte-equality we are here to prove.
    bus.attach_uart_tx_sink(sink, false);

    build_rom_boot_machine(
        bus,
        flash,
        RomBootOpts {
            efuse_mac: None,
            usb_serial_sink: None,
        },
        |c| c,
    )
}

fn flash_image() -> Vec<u8> {
    std::fs::read(root().join("../wasm/tests/fixtures/esp32c3-hello-world-flash.bin"))
        .expect("read C3 hello_world flash image")
}

/// Cold-boot faithfully, snapshot at app-entry, then prove a resume from that
/// snapshot re-emits the app serial byte-for-byte and skips the ~150M-step boot.
#[test]
#[ignore = "boots the real C3 mask ROM (~150M steps); run with --release --ignored"]
fn resume_is_byte_equivalent_to_cold_boot_and_far_cheaper() {
    const MAX_BOOT_STEPS: u64 = 250_000_000;
    // Post-capture window: enough app instructions to emit the banner.
    const POST_STEPS: u64 = 5_000_000;

    // ── 1. Cold boot faithfully, capture at app-entry ────────────────────
    let cold_sink = Arc::new(Mutex::new(Vec::new()));
    let mut cold = build_c3(flash_image(), cold_sink.clone());

    let mut steps_to_app: Option<u64> = None;
    let mut snapshot: Option<MachineRuntimeSnapshot> = None;
    let mut serial_len_at_capture = 0usize;
    for step in 0..MAX_BOOT_STEPS {
        if APP_WINDOW.contains(&cold.cpu.get_pc()) {
            // Snapshot the LIVE machine mid-flight — a real boot state.
            let mut snap = cold.take_runtime_snapshot();
            // Self-key it exactly as `--capture-app-entry` does.
            let fw_sha = sha256(&flash_image());
            snap.set_self_key("esp32c3", fw_sha);
            serial_len_at_capture = cold_sink.lock().unwrap().len();
            steps_to_app = Some(step);
            snapshot = Some(snap);
            break;
        }
        cold.step().expect("cold step");
    }
    let steps_to_app = steps_to_app.expect("cold boot never reached the app window");
    let snapshot = snapshot.unwrap();
    eprintln!("cold boot reached app-entry after {steps_to_app} steps");
    assert!(
        steps_to_app > 500_000,
        "faithful cold boot (mask ROM -> bootloader -> app) should cost hundreds of \
         thousands of steps; got {steps_to_app}"
    );

    // Continue the SAME cold machine for POST_STEPS so we have a ground-truth
    // app-phase serial to compare the resume against.
    for _ in 0..POST_STEPS {
        cold.step().expect("cold app step");
    }
    let cold_tail: Vec<u8> = cold_sink.lock().unwrap()[serial_len_at_capture..].to_vec();
    let cold_tail_str = String::from_utf8_lossy(&cold_tail).into_owned();
    eprintln!(
        "cold app-phase serial ({} bytes):\n{cold_tail_str}",
        cold_tail.len()
    );
    assert!(
        cold_tail_str.contains("Hello world!"),
        "expected the app banner in the post-capture cold serial; got:\n{cold_tail_str}"
    );

    // ── 2. Round-trip the snapshot through bytes + self-key validation ────
    let blob = snapshot.to_bytes();
    eprintln!("resume snapshot blob size: {} bytes", blob.len());
    let decoded = MachineRuntimeSnapshot::from_bytes(&blob).expect("decode .lwrs");
    decoded
        .validate_self_key("esp32c3", &sha256(&flash_image()))
        .expect("self-key must validate against the same firmware");

    // ── 3. Resume: fresh machine, same firmware, apply snapshot, run ──────
    let resume_sink = Arc::new(Mutex::new(Vec::new()));
    let mut resumed = build_c3(flash_image(), resume_sink.clone());
    resumed
        .apply_runtime_snapshot(&decoded)
        .expect("apply resume snapshot");

    // Resume starts AT app-entry — zero mask-ROM replay. It should reach the
    // banner within the same small POST_STEPS window that cost ~150M cold.
    let mut resume_steps_to_banner: Option<u64> = None;
    for step in 0..POST_STEPS {
        resumed.step().expect("resume step");
        if resume_steps_to_banner.is_none()
            && String::from_utf8_lossy(&resume_sink.lock().unwrap()).contains("Hello world!")
        {
            resume_steps_to_banner = Some(step);
        }
    }
    let resume_tail: Vec<u8> = resume_sink.lock().unwrap().clone();
    let resume_steps = resume_steps_to_banner.expect("resume never printed the banner");
    eprintln!("resume reached the app banner after {resume_steps} steps (vs {steps_to_app} cold)");

    // ── 4. Faithfulness: the APPLICATION output is byte-identical ────────
    // Compare from the IDF "Calling app_main()" handoff onward — the actual
    // application-phase serial. This is byte-for-byte identical between a
    // resume and a continued cold boot: same app code, same restored SRAM/IRAM,
    // same peripheral + timer state, right down to the heap-size line. (The
    // IDF pre-app_main startup log carries absolute esp_timer timestamp
    // annotations like "cpu_start (5149)" that encode wall-time-base estimates
    // — not deterministic application output — so the app-phase slice is the
    // faithful comparison point.)
    const MARKER: &[u8] = b"Calling app_main()";
    let cold_app = &cold_tail[find_subslice(&cold_tail, MARKER)
        .unwrap_or_else(|| panic!("cold serial missing app_main marker"))..];
    let resume_app = &resume_tail[find_subslice(&resume_tail, MARKER)
        .unwrap_or_else(|| panic!("resume serial missing app_main marker"))..];
    if cold_app != resume_app {
        let first_diff = cold_app
            .iter()
            .zip(resume_app.iter())
            .position(|(a, b)| a != b)
            .unwrap_or(cold_app.len().min(resume_app.len()));
        let lo = first_diff.saturating_sub(40);
        eprintln!("FIRST APP DIFF at offset {first_diff}");
        eprintln!(
            "cold  : {:?}",
            String::from_utf8_lossy(&cold_app[lo..(first_diff + 80).min(cold_app.len())])
        );
        eprintln!(
            "resume: {:?}",
            String::from_utf8_lossy(&resume_app[lo..(first_diff + 80).min(resume_app.len())])
        );
    }
    assert_eq!(
        resume_app, cold_app,
        "resume application-phase serial must be byte-identical to continuing the cold boot"
    );
    // The application banner + deterministic payload must be present (proves we
    // actually compared real app output, not two empty slices).
    let app_str = String::from_utf8_lossy(cold_app);
    assert!(
        app_str.contains("Hello world!") && app_str.contains("Minimum free heap size:"),
        "app-phase serial should contain the hello_world banner + heap line; got:\n{app_str}"
    );
    // ── Step-count reduction: resume skips the entire cold boot ──────────
    // Resume starts AT app-entry, so the ~steps_to_app the cold path burned in
    // the mask ROM + 2nd-stage bootloader BEFORE the app ran is skipped
    // entirely: resume reaches the banner in fewer steps than the cold boot
    // spent just getting to app-entry.
    assert!(
        resume_steps < steps_to_app,
        "resume ({resume_steps} steps to banner) must be cheaper than the cold boot's \
         {steps_to_app} steps just to reach app-entry"
    );
}

/// Byte-offset of the first occurrence of `needle` in `hay`, if present.
fn find_subslice(hay: &[u8], needle: &[u8]) -> Option<usize> {
    hay.windows(needle.len()).position(|w| w == needle)
}

fn sha256(bytes: &[u8]) -> [u8; 32] {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    h.update(bytes);
    let mut out = [0u8; 32];
    out.copy_from_slice(&h.finalize());
    out
}
