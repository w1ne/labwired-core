#![cfg(feature = "event-scheduler")]
// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! Pixel oracles for the ESP32-S3 Doom lab: frame 1, and the dual-core steady
//! state at frame 180.
//!
//! # What these gates are for
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
//! # Why frame 1 alone is not enough
//!
//! Frame 1 is a STATIC title screen, reached while core 1 has spent most of the
//! run parked in `waiti`. Measured here (the `S3_DOOM_ORACLE core1_…` lines):
//! the APP_CPU leaves reset at step 8,898,394 and is idle-parked from roughly
//! step 16M to 52M, so frame 1 says very little about the dual-core rendering
//! pipeline the surrounding performance work is actually changing.
//!
//! It is blind in a second way too. The firmware prints the SAME hash
//! `0xec236c72` at frames 1, 60 and 120 — Doom is still on its title page. The
//! first logged frame whose pixels have moved is frame 180
//! ([`ORACLE_STEADY_FNV1A32`] = `0xb05338b8`), by which point the attract-mode
//! demo is playing and both cores are executing.
//! `esp32s3_doom_steady_state_matches_recorded_oracle` runs that far and
//! asserts there too.
//!
//! # Why the step/cycle counts here are REPORTED, not asserted
//!
//! This is the point of the gate and the one thing that must not be got wrong.
//! An optimisation is *supposed* to change how many host instructions, batches
//! and simulated cycles it takes to reach a frame. If those numbers were golden
//! the gate would fail on every legitimate speed-up and would be turned off
//! within a week. So the only golden values here are the ones that describe
//! **pixels and inputs**:
//!
//!   * the firmware-computed frame-1 FNV-1a (the hardware oracle),
//!   * the firmware-computed frame-180 FNV-1a (the recorded steady-state
//!     oracle — see the caveat on [`ORACLE_STEADY_FNV1A32`]),
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
//! 1 lands at step 73,781,248), which is part of why these tests are
//! `#[ignore]`d — see the attributes for the measured cost and for where they
//! still have to be wired.
//!
//! # Why no debug ELF is required
//!
//! Under `--rom-boot` the flash image IS the program: `commands/run.rs` reads
//! the ELF and then does `let _ = &elf_bytes;` — it contributes symbols for
//! diagnostics and nothing to execution. Requiring a multi-MB ELF that lives in
//! a scratch directory would make this gate rot the first time that directory
//! is cleaned. The flash image, which is a committed playground asset, is the
//! only firmware input, and it is digest-pinned below.
//!
//! The price of being ELF-less is that core-1 activity is reported as PC
//! buckets rather than function names: `cmap_to_fb` / `doom_display_frame`
//! cannot be named from inside this test. What IS asserted is symbol-free and
//! harder to fake than a name lookup — that the APP_CPU left reset on the real
//! hardware edge, and that it is out of `waiti` for most of the steady-state
//! window rather than idling through it.

use labwired_config::{Arch, ChipDescriptor, SystemManifest};
use labwired_core::boot::esp32s3_rom::{provision_rom_images, RomImages};
use labwired_core::bus::SystemBus;
use labwired_core::cpu::xtensa_lx7::XtensaLx7;
use labwired_core::system::xtensa::{
    attach_esp32_external_devices, configure_xtensa_esp32s3, Esp32s3BootMode, Esp32s3Opts,
};
use labwired_core::{AdvanceRequest, Cpu, Machine};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Instant;

/// The oracle. Frame-1 FNV-1a-32 of the ARGB8888 Doom screen buffer, identical
/// on ESP32-S3-N16R8 silicon, in this native simulator, and in the browser.
const ORACLE_FRAME1_FNV1A32: u32 = 0xec23_6c72;

/// The dual-core steady-state oracle: the firmware's own FNV-1a-32 at
/// [`STEADY_FRAME`], RECORDED in this simulator. Nobody has read a frame-180
/// hash off a board, so this one does not carry the frame-1 oracle's
/// three-way (silicon / native / browser) provenance.
///
/// It is a weaker provenance and a STRICTER gate, and both facts matter:
///
///   * Frames 1, 60 and 120 all hash to `0xec236c72` — Doom is on its static
///     title page. Frame 180 is the first logged frame whose pixels have moved,
///     so it is the first one that cannot be satisfied by a stale frame-1
///     buffer.
///   * Which demo frame Doom is showing at its own frame 180 depends on how
///     many game tics of SIMULATED time have elapsed. So unlike the frame-1
///     oracle, this hash is sensitive to changes in simulated-cycle accounting
///     (idle fast-forward, peripheral cost models, batch boundaries) and not
///     only to changes in rendered pixels. A speed change that leaves pixels
///     alone but moves simulated time WILL move this hash.
///
/// If this moves while frame 1 stays green, that is the signal to look at what
/// the change did to simulated timing, and then to re-record deliberately —
/// with the frame-1 oracle as the proof that pixels themselves are unharmed.
const ORACLE_STEADY_FNV1A32: u32 = 0xb053_38b8;

/// The frame the steady-state gate stops on. The firmware logs every 60th
/// frame.
///
/// # Why 420 and not 180
///
/// This was 180, recorded before `12fe74e76` (#1026) fixed the S3 core clock.
/// `Esp32s3Opts::cpu_clock_hz` had defaulted to 80 MHz while every chip
/// descriptor declared 240 MHz, and SYSTIMER divides the CPU cycle stream by
/// `cpu_clock_hz / 16 MHz` — so the divider was 5 where the part needs 15 and
/// the S3's simulated wall clock ran exactly 3x too fast.
///
/// Doom's attract demo advances on GAME time, so a firmware that now measures
/// time correctly renders more frames before reaching the same demo tic. The
/// pixel value below did not change: `0xb053_38b8` was the hash at frame 180
/// on the broken clock and is the hash at frame 420 on the correct one
/// (420/180 = 2.33). Only the frame INDEX moved. Measured on the corrected
/// clock, at 1,803,251,712 steps:
///
/// | frame | hash | note |
/// |---|---|---|
/// | 300 | `0xec236c72` | still the static title page |
/// | 360 | `0x97960091` | first logged frame off the title page |
/// | 420 | `0xb05338b8` | this gate — same pixels as old frame 180 |
///
/// That the constant survived a 3x clock correction unchanged is the evidence
/// it describes rendered PIXELS and not an accident of pacing.
const STEADY_FRAME: u32 = 420;

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

/// Upper bound on primary scheduling quanta for the frame-1 gate. NOT a golden:
/// it exists only so a wedged run terminates with a readable message instead of
/// hanging a lane. Frame 1 was measured at 73,781,248 steps, so this leaves
/// ~13x headroom. Raise it if a legitimate engine change genuinely needs more —
/// never lower it to "tighten" the gate, that would turn a slow engine into a
/// pixel failure.
const MAX_STEPS: u64 = 1_000_000_000;

/// The same loose bound for the steady-state gate. Frame 180 was measured at
/// 627,408,896 steps, which leaves ~4.8x headroom.
const MAX_STEPS_STEADY: u64 = 3_000_000_000;

/// How often the console is re-read and core 1 is sampled. A multi-billion-step
/// run must not re-read the whole transcript on every instruction.
const SAMPLE_EVERY: u64 = 4096;

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

/// The firmware's own periodic stats line for `frame n`, e.g.
/// `I (12345) doom_video: frame 1  0.030 fps  hash 0xec236c72  ink 12.3%`.
/// `doom_display.c` formats `frame %PRIu32 "  "`, so the two trailing spaces
/// make `frame 1` match frame 1 and never frame 10/100/1000, and keep this
/// clear of the `frame 300:` detail block the firmware also prints.
fn frame_marker(frame: u32) -> String {
    format!("doom_video: frame {frame}  ")
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
        bus.observed_of::<labwired_core::peripherals::components::ili9341_parallel::Ili9341Parallel>().count(),
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

    // The S3 is a DUAL-core chip and this Doom image is a DUAL-core ESP-IDF
    // build: the bootloader prints `cpu_start: Multicore app`, the PRO_CPU
    // clears SYSTEM_CORE_1_CONTROL_0.RESETING at step 8,898,394, and core 1
    // then boots the real ROM and goes on to execute application text.
    //
    // Nothing in THIS file releases it, and nothing here may: the release is
    // the firmware's own store, turned into an unhalt by `Machine::advance`
    // (`crates/core/src/machine/advance.rs`, the `APPCPU_RESET_RELEASED` block
    // at the top of the advance loop) for every frontend at once. A
    // frontend-side `.take()` of that flag — which both this file and
    // `crates/cli/src/commands/run.rs` used to carry — can never win the race:
    // `Machine::step()` is `advance(AdvanceRequest::single())`, and advance
    // re-checks the flag at the top of its second iteration, before it returns
    // on the fuel limit. So the engine owns the bring-up; this gate only
    // observes it, in `DoomRun::core1_release_step`, which is asserted rather
    // than merely printed.
    let mut app_cpu = XtensaLx7::new_app_cpu();
    app_cpu.faithful_windows = true;
    let mut machine = Machine::new(cpu, bus).with_secondary_cpu(app_cpu);
    assert!(
        machine.bus.attach_usb_serial_jtag_sink(serial.clone()),
        "the S3 machine must expose a USB-Serial-JTAG console to tap"
    );
    (machine, serial)
}

/// Parse `hash 0x........` out of one of the firmware's frame stats lines.
fn parse_frame_hash(line: &str) -> u32 {
    let rest = line
        .split_once("hash 0x")
        .unwrap_or_else(|| panic!("frame line has no `hash 0x` field: {line:?}"))
        .1;
    let hex: String = rest.chars().take_while(|c| c.is_ascii_hexdigit()).collect();
    u32::from_str_radix(&hex, 16)
        .unwrap_or_else(|e| panic!("frame hash {hex:?} is not hex ({e}): {line:?}"))
}

/// A booted Doom machine plus the console cursor and the dual-core
/// observations, so one run can be driven to several frames in turn.
struct DoomRun {
    machine: Machine<XtensaLx7>,
    serial: Arc<Mutex<Vec<u8>>>,
    scanned: usize,
    steps: u64,
    /// The step at which `Machine::advance` unhalted the APP_CPU, i.e. the
    /// first step after the PRO_CPU cleared `SYSTEM_CORE_1_CONTROL_0.RESETING`.
    core1_release_step: Option<u64>,
    core1_samples: u64,
    core1_running_samples: u64,
    core1_pc_buckets: BTreeMap<u32, u64>,
}

impl DoomRun {
    fn new(chip: &ChipDescriptor, manifest: &SystemManifest) -> Self {
        let (flash_path, flash) = doom_flash_image();
        eprintln!(
            "S3_DOOM_ORACLE flash={} bytes={} fnv1a64=0x{:016x}",
            flash_path.display(),
            flash.len(),
            fnv1a_64(&flash),
        );
        let rom_images = pinned_rom_images();
        let (machine, serial) = build_doom_machine(chip, manifest, flash, rom_images);
        Self {
            machine,
            serial,
            scanned: 0,
            steps: 0,
            core1_release_step: None,
            core1_samples: 0,
            core1_running_samples: 0,
            core1_pc_buckets: BTreeMap::new(),
        }
    }

    /// Advance until the firmware prints the stats line for `frame`, and return
    /// that line. Panics with the console transcript if `max_steps` is reached.
    ///
    /// Advances exactly the way `labwired run` does — `Machine::step()`, i.e.
    /// `advance(AdvanceRequest::single())`: one primary quantum, then the
    /// secondary, then one peripheral boundary. `AdvanceRequest::run()` is a
    /// different path (BatchPolicy::Auto + idle fast-forward) and a batched
    /// advance can hide a faulting core 1, so the gate does not use it.
    fn run_to_frame(&mut self, frame: u32, max_steps: u64) -> String {
        let marker = frame_marker(frame);
        loop {
            assert!(
                self.steps < max_steps,
                "the Doom lab did not print its frame-{frame} line within {max_steps} steps \
                 ({} simulated cycles). Console so far ({} bytes):\n{}",
                self.machine.total_cycles,
                self.serial.lock().unwrap().len(),
                String::from_utf8_lossy(&self.serial.lock().unwrap()),
            );

            self.machine
                .advance(AdvanceRequest::single())
                .unwrap_or_else(|e| {
                    panic!(
                        "simulator error at step {} (pc=0x{:08x}): {e}",
                        self.steps,
                        self.machine.cpu.get_pc()
                    )
                });
            self.steps += 1;

            // Observe — never drive — the APP_CPU's release from reset.
            if self.core1_release_step.is_none()
                && self
                    .machine
                    .cpu_secondary
                    .as_ref()
                    .is_some_and(|core1| !core1.halted)
            {
                self.core1_release_step = Some(self.steps);
                eprintln!("S3_DOOM_ORACLE core1_released_at_step={}", self.steps);
            }

            if !self.steps.is_multiple_of(SAMPLE_EVERY) {
                continue;
            }

            if let Some(core1) = self.machine.cpu_secondary.as_ref() {
                self.core1_samples += 1;
                if !core1.halted && !core1.is_parked_idle() {
                    self.core1_running_samples += 1;
                    *self
                        .core1_pc_buckets
                        .entry(core1.get_pc() & !0xfff)
                        .or_default() += 1;
                }
            }

            // The frame stats line carries the hash AFTER the marker, so wait
            // for the line to be terminated before reading it — stopping on the
            // marker alone can catch a half-emitted line and parse a truncated
            // hash. Scan forward from a cursor over whole lines only, so a
            // multi-billion-step run never re-reads the whole transcript and a
            // marker split across two scans is still found.
            let console = self.serial.lock().unwrap();
            if console.len() <= self.scanned {
                continue;
            }
            let text = String::from_utf8_lossy(&console[self.scanned..]);
            let mut consumed = 0usize;
            let mut hit: Option<String> = None;
            for line in text.split_inclusive('\n') {
                if !line.ends_with('\n') {
                    break;
                }
                consumed += line.len();
                if hit.is_none() && line.contains(marker.as_str()) {
                    hit = Some(line.trim_end().to_string());
                }
            }
            self.scanned += consumed;
            drop(console);
            if let Some(line) = hit {
                return line;
            }
        }
    }

    /// Fraction of sampled steps on which core 1 was out of reset and NOT
    /// parked in `waiti` — i.e. actually retiring instructions.
    fn core1_running_fraction(&self) -> f64 {
        if self.core1_samples == 0 {
            return 0.0;
        }
        self.core1_running_samples as f64 / self.core1_samples as f64
    }

    /// The busiest 4 KiB code buckets core 1 was seen in, most-sampled first.
    /// Reported, never asserted: this file has no ELF and therefore no symbols,
    /// so these are the raw evidence of where core 1 spent the window.
    fn core1_hot_buckets(&self, top: usize) -> Vec<(u32, u64)> {
        let mut buckets: Vec<(u32, u64)> = self
            .core1_pc_buckets
            .iter()
            .map(|(pc, count)| (*pc, *count))
            .collect();
        buckets.sort_by_key(|bucket| std::cmp::Reverse(bucket.1));
        buckets.truncate(top);
        buckets
    }

    /// Assert the ILI9341 actually received pixels, and report its digest.
    ///
    /// The firmware's oracle is computed over its OWN ARGB buffer, so it says
    /// nothing about whether those pixels ever reached the panel. The
    /// LCD_CAM -> GDMA -> i80 -> ILI9341 path is a second pixel pipeline that a
    /// speed change can break with the oracle still green. Assert its LIVENESS
    /// (speed-independent) and report its digest rather than pinning it: how
    /// much of an async DMA push has landed at the instant the firmware logs
    /// its line legitimately moves when the engine's timing moves.
    fn assert_panel_live(&self, observed: u32) -> (usize, usize, u64) {
        let panel = self.machine
            .bus
            .observed_of::<labwired_core::peripherals::components::ili9341_parallel::Ili9341Parallel>()
            .next()
            .expect("parallel panel attached");
        let panel_fb = panel.oriented_framebuffer();
        let panel_ink = panel_fb.iter().filter(|&&byte| byte != 0).count();
        let digest = fnv1a_64(&panel_fb);
        assert!(
            panel.display_on(),
            "the ILI9341 never left sleep/display-off: the firmware's frame hash is right \
             but nothing was ever shown. Panel digest 0x{digest:016x}.",
        );
        assert!(
            panel_ink > panel_fb.len() / 10,
            "the ILI9341 framebuffer holds only {panel_ink}/{} non-zero bytes. \
             The firmware rendered the right pixels (0x{observed:08x}) but the \
             LCD_CAM -> GDMA -> i80 -> panel path did not deliver them.",
            panel_fb.len(),
        );
        (panel_fb.len(), panel_ink, digest)
    }
}

fn doom_inputs() -> (ChipDescriptor, SystemManifest) {
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
    (
        ChipDescriptor::from_file(&chip_yaml).expect("parse S3 chip descriptor"),
        SystemManifest::from_file(&system_yaml).expect("parse Doom system manifest"),
    )
}

/// Boot the ESP32-S3 Doom lab through the real mask ROM and assert that the
/// firmware's own frame-1 pixel hash is still the hardware oracle.
///
/// `#[ignore]`d for two reasons, both measured rather than assumed:
///
///   1. Reaching frame 1 takes 73,781,248 interpreted Xtensa steps — the real
///      mask ROM, the 2nd-stage bootloader, ESP-IDF bring-up, the WAD load and
///      a full title-screen render. That is 18 s of wall clock in `--release`
///      on an M-series Mac, and minutes in `debug`.
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
#[ignore = "ROM-boots the S3 Doom lab to frame 1 (160M steps, ~38 s release) and needs the monorepo's Doom flash image; run with --release --ignored"]
fn esp32s3_doom_frame1_matches_hardware_oracle() {
    let (chip, manifest) = doom_inputs();
    let mut run = DoomRun::new(&chip, &manifest);

    let started = Instant::now();
    let frame1_line = run.run_to_frame(1, MAX_STEPS);
    let elapsed = started.elapsed();
    let observed = parse_frame_hash(&frame1_line);

    // Observations, NOT goldens. An optimisation is meant to change these.
    let (panel_bytes, panel_ink, panel_digest) = run.assert_panel_live(observed);
    eprintln!(
        "S3_DOOM_ORACLE frame1_line={frame1_line:?}\n\
         S3_DOOM_ORACLE frame1_fnv1a32=0x{observed:08x} oracle=0x{ORACLE_FRAME1_FNV1A32:08x} \
         steps={} total_cycles={} wall_ms={} panel_fb_bytes={panel_bytes} \
         panel_ink_bytes={panel_ink} panel_fnv1a64=0x{panel_digest:016x} console_bytes={} \
         core1_release_step={:?} core1_running_fraction={:.3}",
        run.steps,
        run.machine.total_cycles,
        elapsed.as_millis(),
        run.serial.lock().unwrap().len(),
        run.core1_release_step,
        run.core1_running_fraction(),
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

    // The APP_CPU must have left reset on its own. Nothing in this file
    // releases it: the firmware writes SYSTEM_CORE_1_CONTROL_0, the
    // core1_control peripheral raises APPCPU_RESET_RELEASED on the RESETING
    // 1->0 edge, and `Machine::advance` unhalts the secondary. If that chain
    // breaks, a dual-core ESP-IDF image runs half a machine and this gate has
    // to say so rather than quietly measuring a unicore run.
    assert!(
        run.core1_release_step.is_some(),
        "the APP_CPU never left reset before frame 1. The Doom image is a DUAL-core \
         ESP-IDF build (`cpu_start: Multicore app`); core 1 was measured leaving reset at \
         step 8,898,394. A frame-1 hash produced without core 1 is not the machine the \
         oracle was taken on."
    );
}

/// Run the same machine on into the DUAL-CORE steady state and assert the
/// firmware's pixel hash there too.
///
/// This is the gate the frame-1 one cannot be: by [`STEADY_FRAME`] the attract
/// demo is playing, the screen contents have moved off the title page, and core
/// 1 is executing application text rather than sitting in `waiti`. It
/// deliberately re-asserts frame 1 on the way through, so a failure that is
/// really a frame-1 regression is reported as one.
///
/// `#[ignore]`d for the same two reasons as the frame-1 gate, with a much
/// bigger first one: frame 180 is 627,408,896 steps, measured at 128 s of wall
/// clock in `--release` on an M-series Mac — 8.5x the frame-1 gate. That is a
/// nightly cost, not a per-PR one.
#[test]
#[ignore = "ROM-boots the S3 Doom lab into dual-core steady state (frame 420, 1.80B steps, ~495 s release) and needs the monorepo's Doom flash image; run with --release --ignored"]
fn esp32s3_doom_steady_state_matches_recorded_oracle() {
    let (chip, manifest) = doom_inputs();
    let mut run = DoomRun::new(&chip, &manifest);

    let started = Instant::now();
    let frame1_line = run.run_to_frame(1, MAX_STEPS);
    let frame1_hash = parse_frame_hash(&frame1_line);
    let frame1_steps = run.steps;
    assert_eq!(
        frame1_hash, ORACLE_FRAME1_FNV1A32,
        "frame-1 pixels changed; fix that before reading the steady-state result. \
         observed 0x{frame1_hash:08x}, hardware oracle 0x{ORACLE_FRAME1_FNV1A32:08x}\n\
         firmware line: {frame1_line}"
    );
    // Restart the core-1 activity census at frame 1 so the reported fraction
    // describes the STEADY state and is not diluted by the long boot that
    // precedes it, during which core 1 is legitimately in reset or parked.
    run.core1_samples = 0;
    run.core1_running_samples = 0;
    run.core1_pc_buckets.clear();

    let steady_line = run.run_to_frame(STEADY_FRAME, MAX_STEPS_STEADY);
    let elapsed = started.elapsed();
    let observed = parse_frame_hash(&steady_line);

    let (panel_bytes, panel_ink, panel_digest) = run.assert_panel_live(observed);
    let hot: Vec<String> = run
        .core1_hot_buckets(6)
        .into_iter()
        .map(|(pc, count)| format!("0x{pc:08x}:{count}"))
        .collect();
    let hot = hot.join(",");
    eprintln!(
        "S3_DOOM_ORACLE steady_line={steady_line:?}\n\
         S3_DOOM_ORACLE steady_frame={STEADY_FRAME} steady_fnv1a32=0x{observed:08x} \
         oracle=0x{ORACLE_STEADY_FNV1A32:08x} steps={} frame1_steps={frame1_steps} \
         total_cycles={} wall_ms={} panel_fb_bytes={panel_bytes} panel_ink_bytes={panel_ink} \
         panel_fnv1a64=0x{panel_digest:016x} console_bytes={} core1_release_step={:?} \
         core1_running_fraction={:.3} core1_hot_pc_4k={hot}",
        run.steps,
        run.machine.total_cycles,
        elapsed.as_millis(),
        run.serial.lock().unwrap().len(),
        run.core1_release_step,
        run.core1_running_fraction(),
    );

    // THE GATE.
    assert_eq!(
        observed, ORACLE_STEADY_FNV1A32,
        "ESP32-S3 Doom frame-{STEADY_FRAME} pixels CHANGED.\n\
         observed 0x{observed:08x}, recorded oracle 0x{ORACLE_STEADY_FNV1A32:08x}\n\
         firmware line: {steady_line}\n\
         frame 1 was still 0x{frame1_hash:08x} (correct), so the rendered pixels themselves \
         are intact and this is the DUAL-CORE steady state moving. Two things can do that: \
         a real rendering difference on core 1, or a change in SIMULATED timing that puts \
         Doom's attract demo on a different game tic at its own frame {STEADY_FRAME}. \
         Work out which before re-recording."
    );

    // Core 1 must have been doing real work through the window this gate
    // covers, otherwise the hash above came from the same single core that
    // produced frame 1 and this test adds nothing over the cheaper one.
    assert!(
        run.core1_release_step.is_some(),
        "the APP_CPU never left reset; see the frame-1 gate's message."
    );
    let running = run.core1_running_fraction();
    assert!(
        running > 0.25,
        "core 1 was out of `waiti` on only {:.1}% of samples between frame 1 and frame \
         {STEADY_FRAME} ({}/{} samples). This gate exists to cover the DUAL-core steady \
         state; if the APP_CPU is parked for nearly all of it, the steady-state hash is \
         just a longer single-core run, and the real regression is that core 1 stopped \
         doing rendering work. Hot core-1 PC buckets (4 KiB): {hot}",
        running * 100.0,
        run.core1_running_samples,
        run.core1_samples,
    );
}
