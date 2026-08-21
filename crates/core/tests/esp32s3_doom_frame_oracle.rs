#![cfg(feature = "event-scheduler")]
// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! Frame-1 pixel oracle for the ESP32-S3 Doom lab.
//!
//! # What this gate is for
//!
//! Real Doom (doomgeneric + the shareware DOOM1.WAD) runs on a physical
//! ESP32-S3-N16R8, in this native simulator, and in the browser. The firmware
//! itself measures every frame — it computes a 32-bit FNV-1a over the
//! `DOOMGENERIC_RESX * DOOMGENERIC_RESY` ARGB8888 screen buffer and prints it
//! at frame 1 and every 60th frame thereafter (`main/doom_display.c`,
//! `doom_display_frame`). On all three of hardware, native and browser that
//! frame-1 hash is [`ORACLE_FRAME1_FNV1A32`] = `0xec236c72`.
//!
//! That number is the regression check. It exists so the engine can be made
//! FASTER without anyone having to take "the pixels are still fine" on trust:
//! a speed change that alters a single rendered pixel moves this hash, and this
//! test goes red.
//!
//! # Why the step/cycle counts here are REPORTED, not asserted
//!
//! This is the point of the gate and the one thing that must not be got wrong.
//! An optimisation is *supposed* to change how many host instructions, batches
//! and simulated cycles it takes to reach frame 1. If those numbers were golden
//! the gate would fail on every legitimate speed-up and would be turned off
//! within a week. So the only golden values here are the ones that describe
//! **pixels and inputs**:
//!
//!   * the firmware-computed frame-1 FNV-1a (the oracle),
//!   * the flash image the ROM loads (digest-pinned),
//!   * the boot ROM images (digest-pinned).
//!
//! Everything about *how long it took* is printed as an observation with only a
//! loose upper bound, so a runaway sim still terminates.
//!
//! # Why ROM boot, not fast boot
//!
//! `esp32s3_oled_profile.rs` — the harness this file follows — uses
//! `boot::esp32s3::fast_boot`, which loads the ELF's segments straight into the
//! cache windows and jumps to the entry point. That is a legitimate but
//! DIFFERENT benchmark boundary and it cannot be substituted here:
//!
//!   * The oracle `0xec236c72` was established on silicon and reproduced in the
//!     simulator through `labwired run --rom-boot`. Presenting a fast-boot
//!     result as ROM-boot fidelity is exactly the substitution the OLED profile
//!     doc comment warns against.
//!   * The Doom lab is a 16 MiB N16R8 image whose WAD lives in a data partition
//!     past 4 MiB. Under ROM boot the real mask ROM + 2nd-stage bootloader
//!     program the flash MMU, and the firmware then reads the WAD through the
//!     SPI-flash controller (`components/doomgeneric/w_file_flash.c`). Fast boot
//!     skips the bootloader's MMU programming entirely and populates each cache
//!     window from ELF segments only — there is no partition table and no WAD.
//!     Frame 1 could not be rendered at all.
//!   * ROM boot is also what the BROWSER does, so the boot path matches the
//!     other two places the oracle holds. `new_from_config_xtensa_esp32s3_flash`
//!     in `crates/wasm/src/lib.rs` sets `real_reset_boot: true` and injects
//!     `rom_images` + `flash_image` as bytes — the construction below is the
//!     same one, not merely an equivalent one.
//!
//! ROM boot costs a lot of simulated instructions before `app_main` runs (frame
//! 1 lands at step 73,781,248), which is part of why this test is `#[ignore]`d —
//! see the attribute on the test for the measured cost and for where it still
//! has to be wired.
//!
//! # Why no debug ELF is required
//!
//! Under `--rom-boot` the flash image IS the program: `commands/run.rs` reads
//! the ELF and then does `let _ = &elf_bytes;` — it contributes symbols for
//! diagnostics and nothing to execution. Requiring a multi-MB ELF that lives in
//! a scratch directory would make this gate rot the first time that directory
//! is cleaned. The flash image, which is a committed playground asset, is the
//! only firmware input, and it is digest-pinned below.

use labwired_config::{Arch, ChipDescriptor, SystemManifest};
use labwired_core::boot::esp32s3_rom::{provision_rom_images, RomImages};
use labwired_core::bus::SystemBus;
use labwired_core::cpu::xtensa_lx7::XtensaLx7;
use labwired_core::system::xtensa::{
    attach_esp32_external_devices, configure_xtensa_esp32s3, Esp32s3BootMode, Esp32s3Opts,
};
use labwired_core::{Cpu, Machine};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Instant;

/// The oracle. Frame-1 FNV-1a-32 of the ARGB8888 Doom screen buffer, identical
/// on ESP32-S3-N16R8 silicon, in this native simulator, and in the browser.
const ORACLE_FRAME1_FNV1A32: u32 = 0xec23_6c72;

/// FNV-1a-64 of the curated Doom flash image. Pinned so that swapping the
/// asset fails HERE, with a clear message, rather than silently moving the
/// oracle and looking like an engine regression.
const GOLDEN_FLASH_FNV1A64: u64 = 0xeb9f_1b30_4ac0_8435;
const GOLDEN_FLASH_LEN: usize = 8_455_860;

/// FNV-1a-64 of the ESP32-S3 boot ROM images this gate was baselined against.
/// `provision_rom_images()` prefers an installed toolchain's ROM ELF over the
/// vendored blob, so two machines can silently boot DIFFERENT mask ROMs. The
/// program the ROM loads is part of the pixel pipeline; pin it.
const GOLDEN_ROM_IROM_FNV1A64: u64 = 0xb848_45dd_0ee7_03ab;
const GOLDEN_ROM_DROM_FNV1A64: u64 = 0xdc81_1ae1_a604_0f0e;

/// The firmware's own frame-1 console line, e.g.
/// `I (12345) doom_video: frame 1  0.030 fps  hash 0xec236c72  ink 12.3%`.
/// `doom_display.c` formats `frame %PRIu32 "  "`, so the two trailing spaces
/// make this match frame 1 and never frame 10/100/1000.
const FRAME1_MARKER: &str = "doom_video: frame 1  ";

/// Upper bound on primary scheduling quanta. NOT a golden: it exists only so a
/// wedged run terminates with a readable message instead of hanging a lane.
/// Frame 1 was measured at 73,781,248 steps, so this leaves ~13x headroom.
/// Raise it if a legitimate engine change genuinely needs more — never lower it
/// to "tighten" the gate, that would turn a slow engine into a pixel failure.
const MAX_STEPS: u64 = 1_000_000_000;

/// The core repository root (`crates/core/../..`). Chip descriptor and system
/// manifest both live inside the core checkout, so this resolves in a
/// standalone core clone and inside the monorepo alike. Deliberately NOT
/// `../../target` or any build-output path.
fn core_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("resolve core repository root")
}

fn fnv1a_64(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |hash, byte| {
        (hash ^ u64::from(*byte)).wrapping_mul(0x0000_0100_0000_01b3)
    })
}

/// Resolve the curated Doom flash image.
///
/// `LABWIRED_ESP32S3_DOOM_FLASH` wins; otherwise the monorepo layout (core is a
/// submodule at `<mono>/core`, the asset is a committed playground file). If
/// neither resolves this FAILS — it never falls back to a placeholder and never
/// returns early, because a skipped fidelity gate reads as a green one.
fn doom_flash_image() -> (PathBuf, Vec<u8>) {
    const ENV: &str = "LABWIRED_ESP32S3_DOOM_FLASH";
    const DEFAULT_RELATIVE: &str =
        "../packages/playground/public/wasm/demo-esp32s3-doom-lab-flash.bin";
    let path = std::env::var_os(ENV)
        .map(PathBuf::from)
        .unwrap_or_else(|| core_root().join(DEFAULT_RELATIVE));
    assert!(
        path.is_file(),
        "the ESP32-S3 Doom frame oracle requires the curated Doom flash image at {}. \
         Set {ENV} to it. Refusing to substitute another image or to skip: this test \
         asserts on pixels, and a skipped pixel gate reads as a passing one.",
        path.display()
    );
    let bytes = std::fs::read(&path).expect("read curated ESP32-S3 Doom flash image");
    assert_eq!(
        bytes.len(),
        GOLDEN_FLASH_LEN,
        "Doom flash image at {} is {} bytes, expected {GOLDEN_FLASH_LEN}. The oracle \
         0x{ORACLE_FRAME1_FNV1A32:08x} describes ONE image; a different one is a new \
         baseline, not a regression.",
        path.display(),
        bytes.len(),
    );
    assert_eq!(
        fnv1a_64(&bytes),
        GOLDEN_FLASH_FNV1A64,
        "Doom flash image at {} has digest 0x{:016x}, expected 0x{GOLDEN_FLASH_FNV1A64:016x}. \
         The firmware changed; re-baseline the oracle deliberately rather than reading this \
         as an engine regression.",
        path.display(),
        fnv1a_64(&bytes),
    );
    (path, bytes)
}

/// Resolve and digest-pin the ESP32-S3 boot ROM.
fn pinned_rom_images() -> RomImages {
    let images = provision_rom_images().expect(
        "the ESP32-S3 Doom frame oracle needs the real ESP32-S3 boot ROM. \
         labwired-core vendors it (crates/core/roms/esp32s3/), so this only fails if \
         LABWIRED_ESP32S3_FASTBOOT is set — which would silently turn a ROM-boot \
         fidelity gate into a fast-boot one.",
    );
    let irom = fnv1a_64(&images.irom);
    let drom = fnv1a_64(&images.drom);
    eprintln!("S3_DOOM_ORACLE rom_irom_fnv1a64=0x{irom:016x} rom_drom_fnv1a64=0x{drom:016x}");
    assert_eq!(
        irom, GOLDEN_ROM_IROM_FNV1A64,
        "boot ROM IROM digest 0x{irom:016x} != golden 0x{GOLDEN_ROM_IROM_FNV1A64:016x}. \
         provision_rom_images() prefers an installed toolchain's ROM ELF over the vendored \
         blob, so this machine is booting a DIFFERENT mask ROM than the oracle was taken on. \
         Unset LABWIRED_ESP32S3_ROM_ELF / LABWIRED_ESP32S3_ROM / _DROM, or re-baseline."
    );
    assert_eq!(
        drom, GOLDEN_ROM_DROM_FNV1A64,
        "boot ROM DROM digest 0x{drom:016x} != golden 0x{GOLDEN_ROM_DROM_FNV1A64:016x}; \
         see the IROM message."
    );
    images
}

/// Build the faithful ROM-boot ESP32-S3 Doom machine.
///
/// This mirrors the `args.rom_boot` arm of `labwired run` (`crates/cli/src/
/// commands/run.rs`) and the ELF-less production twin `run_s3_rom_boot_no_elf`
/// (`crates/cli/src/commands/test.rs`): `configure_xtensa_esp32s3` with
/// `real_reset_boot: true` and the chip descriptor's flash size, the manifest's
/// external devices through the generic factory, faithful windowed registers on
/// BOTH cores, and a real APP_CPU halted at the ROM reset vector.
fn build_doom_machine(
    chip: &ChipDescriptor,
    manifest: &SystemManifest,
    flash: Vec<u8>,
    rom_images: RomImages,
) -> (Machine<XtensaLx7>, Arc<Mutex<Vec<u8>>>) {
    assert_eq!(chip.name, "esp32s3");
    assert_eq!(chip.arch, Arch::Xtensa);
    let tft = manifest
        .external_devices
        .iter()
        .find(|device| device.id == "tft")
        .expect("Doom manifest must declare external device 'tft'");
    assert_eq!(tft.r#type, "ili9341-16bit");

    let flash_size = u32::try_from(chip.flash.size).expect("S3 flash size fits u32");
    assert!(
        flash_size as usize >= GOLDEN_FLASH_LEN,
        "chip descriptor models {flash_size} B of flash but the Doom image is \
         {GOLDEN_FLASH_LEN} B. A short backing truncates the WAD partition to 0xFF \
         with no error, which looks like a corrupt asset rather than a small model."
    );

    let mut bus = SystemBus::new();
    let opts = Esp32s3Opts {
        real_reset_boot: true,
        flash_size,
        // Passed in rather than read from LABWIRED_ESP32S3_FLASH: a
        // process-global env var is shared by every test in this binary and by
        // anything else in the process, so injecting the bytes is what makes
        // this gate's firmware input unambiguous.
        flash_image: Some(flash),
        rom_images: Some(rom_images),
        ..Esp32s3Opts::default()
    };
    let wiring = configure_xtensa_esp32s3(&mut bus, &opts);
    assert_eq!(
        wiring.boot_mode,
        Esp32s3BootMode::Faithful,
        "ROM boot needs the real ESP32-S3 boot ROM; the harness ROM would make this \
         a fast-boot measurement wearing a ROM-boot label"
    );
    attach_esp32_external_devices(&mut bus, manifest)
        .expect("attach the Doom lab's ILI9341 parallel panel from the manifest");
    bus.refresh_peripheral_index();
    assert_eq!(
        bus.ili9341_parallel.len(),
        1,
        "the manifest's 16-bit parallel ILI9341 must be on the bus; without it the \
         GPIO->panel pixel path is not exercised at all"
    );

    let mut cpu = wiring.cpu;
    // rom-boot runs the real ROM + firmware, which install the OF/UF window
    // vectors and build a real stack save chain, so use the per-access
    // overflow / RETW underflow path rather than the sim shadow stack.
    cpu.faithful_windows = true;

    // ONE buffer for BOTH consoles. The mask ROM and the 2nd-stage bootloader
    // talk on UART0; the ESP-IDF app talks on whichever console its sdkconfig
    // selected. Tapping only one of the two can capture the boot banner and
    // nothing the firmware ever prints, which reads as "Doom never ran".
    let serial = Arc::new(Mutex::new(Vec::new()));
    bus.attach_uart_tx_sink(serial.clone(), false);

    // The S3 is a DUAL-core chip and an SMP ESP-IDF build's `start_other_core`
    // spins waiting for core 1 before it ever reaches app_main, so a
    // single-core machine stalls in the mask ROM and prints only the banner.
    // This particular Doom image never releases core 1 (the run loop's
    // `appcpu_released_at_step` line does not appear — it is a unicore build),
    // but the APP_CPU is attached anyway so the machine shape matches
    // `labwired run --rom-boot` and the wasm constructor rather than being a
    // reduced one that happens to work for this firmware.
    let mut app_cpu = XtensaLx7::new_app_cpu();
    app_cpu.faithful_windows = true;
    let mut machine = Machine::new(cpu, bus).with_secondary_cpu(app_cpu);
    assert!(
        machine.bus.attach_usb_serial_jtag_sink(serial.clone()),
        "the S3 machine must expose a USB-Serial-JTAG console to tap"
    );
    (machine, serial)
}

/// Parse `hash 0x........` out of the firmware's own frame-1 line.
fn parse_frame1_hash(line: &str) -> u32 {
    let rest = line
        .split_once("hash 0x")
        .unwrap_or_else(|| panic!("frame-1 line has no `hash 0x` field: {line:?}"))
        .1;
    let hex: String = rest.chars().take_while(|c| c.is_ascii_hexdigit()).collect();
    u32::from_str_radix(&hex, 16)
        .unwrap_or_else(|e| panic!("frame-1 hash {hex:?} is not hex ({e}): {line:?}"))
}

/// Boot the ESP32-S3 Doom lab through the real mask ROM and assert that the
/// firmware's own frame-1 pixel hash is still the hardware oracle.
///
/// `#[ignore]`d for two reasons, both measured rather than assumed:
///
///   1. Reaching frame 1 takes 73,781,248 interpreted Xtensa steps — the real
///      mask ROM, the 2nd-stage bootloader, ESP-IDF bring-up, the WAD load and
///      a full title-screen render. That is 25.8 s of wall clock in `--release`
///      on a heavily loaded machine (load avg ~59), and minutes in `debug`.
///   2. Its firmware input is an 8.5 MB flash image that lives in the
///      MONOREPO's `packages/playground/public/wasm/`, not in this repo, so a
///      core-only checkout cannot supply it without
///      `LABWIRED_ESP32S3_DOOM_FLASH`.
///
/// **Nothing runs this automatically.** Until it is wired into a lane it is a
/// gate nobody pulls. It belongs as an `-- --ignored` step in
/// `.github/workflows/core-nightly.yml` (next to the "WiFi thunk bring-up
/// harness" step, which exists for exactly this reason), with
/// `LABWIRED_ESP32S3_DOOM_FLASH` pointed at a checked-out copy of the image.
/// See also the `NIGHTLY_ONLY` entry for this file in
/// `crates/core/src/tests/scheduler_lane_coverage.rs`.
#[test]
#[ignore = "ROM-boots the S3 Doom lab to frame 1 (73.8M steps, ~26s release) and needs the monorepo's Doom flash image; run with --release --ignored"]
fn esp32s3_doom_frame1_matches_hardware_oracle() {
    let root = core_root();
    let chip_yaml = root.join("configs/chips/esp32s3.yaml");
    let system_yaml = root.join("examples/esp32s3-i80-doom/system.yaml");
    assert!(
        chip_yaml.is_file(),
        "missing chip descriptor {}",
        chip_yaml.display()
    );
    assert!(
        system_yaml.is_file(),
        "missing Doom system manifest {}",
        system_yaml.display()
    );
    let chip = ChipDescriptor::from_file(&chip_yaml).expect("parse S3 chip descriptor");
    let manifest = SystemManifest::from_file(&system_yaml).expect("parse Doom system manifest");

    let (flash_path, flash) = doom_flash_image();
    eprintln!(
        "S3_DOOM_ORACLE flash={} bytes={} fnv1a64=0x{:016x}",
        flash_path.display(),
        flash.len(),
        fnv1a_64(&flash),
    );
    let rom_images = pinned_rom_images();
    let (mut machine, serial) = build_doom_machine(&chip, &manifest, flash, rom_images);

    // Advance exactly the way `labwired run` does — `Machine::step()`, i.e.
    // `advance(AdvanceRequest::single())`: one primary quantum, then the
    // secondary, then one peripheral boundary. `AdvanceRequest::run()` is a
    // different path (BatchPolicy::Auto + idle fast-forward) and a batched
    // advance can hide a faulting core 1, so the gate does not use it.
    let started = Instant::now();
    let mut steps: u64 = 0;
    let mut scanned: usize = 0;
    let mut appcpu_released = false;
    let frame1_line = loop {
        assert!(
            steps < MAX_STEPS,
            "the Doom lab did not print its frame-1 line within {MAX_STEPS} steps \
             ({} simulated cycles). Console so far ({} bytes):\n{}",
            machine.total_cycles,
            serial.lock().unwrap().len(),
            String::from_utf8_lossy(&serial.lock().unwrap()),
        );

        // Release the APP_CPU on the real hardware edge: the PRO_CPU clearing
        // CORE_1_RESETING, surfaced by SYSTEM_CORE_1_CONTROL. Same edge the
        // CLI's rom-boot loop acts on; no firmware-symbol hooks.
        if !appcpu_released
            && labwired_core::peripherals::esp_xtensa_common::rom_thunks::APPCPU_RESET_RELEASED
                .with(|slot| slot.take())
        {
            appcpu_released = true;
            if let Some(core1) = machine.cpu_secondary.as_mut() {
                core1.halted = false;
            }
            eprintln!("S3_DOOM_ORACLE appcpu_released_at_step={steps}");
        }

        machine.step().unwrap_or_else(|e| {
            panic!(
                "simulator error at step {steps} (pc=0x{:08x}): {e}",
                machine.cpu.get_pc()
            )
        });
        steps += 1;

        // Scan the console in slices from a cursor so a multi-billion-step run
        // does not re-read the whole transcript every instruction. The frame-1
        // stats line carries the hash AFTER the marker, so wait for the line to
        // be terminated before reading it — stopping on the marker alone can
        // catch a half-emitted line and parse a truncated hash.
        if steps % 4096 != 0 {
            continue;
        }
        let console = serial.lock().unwrap();
        if console.len() <= scanned {
            continue;
        }
        let text = String::from_utf8_lossy(&console);
        if let Some(at) = text[scanned..].find(FRAME1_MARKER) {
            let line_start = scanned + at;
            if let Some(end) = text[line_start..].find('\n') {
                break text[line_start..line_start + end].trim_end().to_string();
            }
            // Marker present but the line is still being emitted: keep the
            // cursor before it so the next pass re-finds it.
            continue;
        }
        // Keep the tail so a marker split across two scans is still found.
        scanned = console.len().saturating_sub(FRAME1_MARKER.len());
    };
    let elapsed = started.elapsed();

    let observed = parse_frame1_hash(&frame1_line);

    // Observations, NOT goldens. An optimisation is meant to change these.
    let panel = &machine.bus.ili9341_parallel[0];
    let panel_fb = panel.oriented_framebuffer();
    let panel_ink = panel_fb.iter().filter(|&&b| b != 0).count();
    eprintln!(
        "S3_DOOM_ORACLE frame1_line={frame1_line:?}\n\
         S3_DOOM_ORACLE frame1_fnv1a32=0x{observed:08x} oracle=0x{ORACLE_FRAME1_FNV1A32:08x} \
         steps={steps} total_cycles={} wall_ms={} panel_display_on={} panel_fb_bytes={} \
         panel_ink_bytes={panel_ink} panel_fnv1a64=0x{:016x} console_bytes={}",
        machine.total_cycles,
        elapsed.as_millis(),
        panel.display_on(),
        panel_fb.len(),
        fnv1a_64(&panel_fb),
        serial.lock().unwrap().len(),
    );

    // THE GATE.
    assert_eq!(
        observed, ORACLE_FRAME1_FNV1A32,
        "ESP32-S3 Doom frame-1 pixels CHANGED.\n\
         observed 0x{observed:08x}, hardware oracle 0x{ORACLE_FRAME1_FNV1A32:08x}\n\
         firmware line: {frame1_line}\n\
         This hash is computed by the firmware itself over its own ARGB screen buffer and \
         is identical on ESP32-S3-N16R8 silicon, in this simulator and in the browser. A \
         change here means the simulated machine now renders different pixels than the \
         hardware does. It is NOT allowed to be updated to make a speed change pass."
    );

    // The oracle above is computed by the FIRMWARE over its own ARGB buffer, so
    // it says nothing about whether those pixels ever reached the panel. The
    // LCD_CAM -> GDMA -> i80 -> ILI9341 path is a second pixel pipeline that a
    // speed change can break with the oracle still green. Assert its LIVENESS
    // (speed-independent) and report its digest rather than pinning it: how much
    // of an async DMA push has landed at the instant the firmware logs its line
    // legitimately moves when the engine's timing moves.
    assert!(
        panel.display_on(),
        "the ILI9341 never left sleep/display-off: the firmware's frame-1 hash is right \
         but nothing was ever shown. Panel digest 0x{:016x}.",
        fnv1a_64(&panel_fb),
    );
    assert!(
        panel_ink > panel_fb.len() / 10,
        "the ILI9341 framebuffer holds only {panel_ink}/{} non-zero bytes at frame 1. \
         The firmware rendered the right pixels (0x{observed:08x}) but the \
         LCD_CAM -> GDMA -> i80 -> panel path did not deliver them.",
        panel_fb.len(),
    );
}
