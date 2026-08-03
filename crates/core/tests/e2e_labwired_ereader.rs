// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT
//
// End-to-end smoke test for the `labwired-ereader` Arduino-ESP32 sketch.
//
// Goal: load the ereader's stock ELF (built with PlatformIO) into our
// ESP32-classic sim and step long enough to either see the SSD1680 panel get a
// `refresh()` or stall.
//
// Bring-up is NOT hand-rolled here. It used to be — ~250 lines that rebuilt the
// bus, the CPU pair, the stack seeds and the whole thunk list by hand, in
// parallel with the same sequence in the wasm playground and in
// `install_arduino_esp32_profile`. A copy that misses a step does not fail
// loudly; it boots a machine that is subtly wrong and the symptom surfaces
// somewhere else entirely. So the boot goes through
// `boot::esp32_arduino::build_arduino_elf_machine`, the one home for this
// chip's Arduino-ELF path, and this file is left saying only what is specific
// to the e-reader: which board, which panel pins, and what counts as painted.
//
// The cross-core FROM_CPU yield IPI is modeled in the core (DPORT interrupt
// matrix), not bridged here. If this test paints, the firmware paints in the
// playground too.
//
// Heavy and slow (~200M cycles in the worst case), so `#[ignore]`d by
// default. Run with:
//
//     cargo test -p labwired-core --test e2e_labwired_ereader \
//         -- --ignored --nocapture
//
// Skips quietly with `[skip]` when the ELF isn't present — the test only
// fires when a recently-built ereader image is at
// `/tmp/labwired-ereader/build/labwired-ereader.ino.elf`, or wherever
// `LABWIRED_EREADER_ELF` points.

use labwired_core::boot::esp32_arduino::{build_arduino_elf_machine, ArduinoElfBootOpts};
use labwired_core::peripherals::components::Ssd1680Tricolor290;
use labwired_core::peripherals::esp32::spi::Esp32Spi;
use labwired_core::Cpu;
use std::path::PathBuf;

const DEFAULT_ELF: &str = "/tmp/labwired-ereader/build/labwired-ereader.ino.elf";

// NOT `#[ignore]`d. It used to be, on the grounds of "up to 200M cycles" — but
// it reaches an inked refresh in ~13.6M and finishes in seconds. Being both
// `#[ignore]`d AND self-skipping meant it never ran anywhere, in CI or locally,
// and a completely blank flagship e-reader demo shipped behind a green board.
#[test]
fn labwired_ereader_runs_to_panel_paint() {
    let elf_path = std::env::var("LABWIRED_EREADER_ELF")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from(DEFAULT_ELF));
    if !elf_path.exists() {
        // A guard that skips itself is not a guard. CI sets
        // LABWIRED_REQUIRE_EREADER_ELF=1 (alongside LABWIRED_EREADER_ELF
        // pointing at the shipped demo binary) so a missing artifact is a
        // failure there, while a local checkout without one still skips.
        if std::env::var("LABWIRED_REQUIRE_EREADER_ELF").as_deref() == Ok("1") {
            panic!(
                "labwired-ereader ELF not found at {elf_path:?} but \
                 LABWIRED_REQUIRE_EREADER_ELF=1 — the e-reader lab is unguarded. \
                 Point LABWIRED_EREADER_ELF at the shipped \
                 demo-labwired-ereader.elf."
            );
        }
        eprintln!(
            "[skip] labwired-ereader ELF not found at {elf_path:?}; \
             build labwired-ereader and/or set LABWIRED_EREADER_ELF to enable"
        );
        return;
    }

    let elf_bytes = std::fs::read(&elf_path).expect("read ELF");
    let image = labwired_loader::load_elf(&elf_path).expect("parse ELF");

    // ── 1. The board. The panel is attached from this manifest by the generic
    //       attach_esp32_external_devices factory (inside the builder below) —
    //       the SAME path cli/wasm use. No peripheral is hardcoded here. The
    //       GxEPD2_290_C90c panel is an SSD1680 controller (see the GxEPD2 driver
    //       header); the factory maps the gxepd2_290_c90c alias to the SSD1680
    //       model, wires CS=GPIO5 and latches DC=GPIO17 (the GPIO GxEPD2 toggles
    //       via digitalWrite before each SPI.transfer — real wire framing).
    //
    //       The manifest below MUST stay ssd1680_tricolor_290. C90c emits
    //       SSD1680 opcodes (0x12 SWRESET, 0x11 data-entry, 0x24/0x26 RAM,
    //       0x22+0x20 update). A uc8151d_tricolor_290 panel decodes those as
    //       PWR/LUT/DRF, never drives BUSY low, and the firmware hangs in
    //       _waitWhileBusy: refresh_gen=0, zero ink bytes, blank panel.
    //
    //       The manifest stays INLINE rather than reading
    //       `configs/systems/esp32-wroom-epaper.yaml`: it carries
    //       `busy_pin: GPIO4`, which that file does not, and the BUSY wire is
    //       what stops GxEPD2's `_waitWhileBusy` from blocking to its
    //       multi-second timeout.
    let manifest: labwired_config::SystemManifest = serde_yaml::from_str(
        r#"
name: esp32-epaper-ereader
chip: esp32
external_devices:
  - id: epd
    type: ssd1680_tricolor_290
    connection: spi3
    config:
      cs_pin: GPIO5
      dc_pin: GPIO17
      busy_pin: GPIO4
"#,
    )
    .expect("parse inline ereader board manifest");

    // ── 2. Boot it. `build_arduino_elf_machine` owns the whole sequence: bus,
    //       dual-core CPU pair, UART sink attached AFTER the peripherals exist,
    //       external devices from the manifest, both stack seeds, and the
    //       symbol-driven thunk install via `install_arduino_esp32_profile`.
    //
    //       What used to be here instead was ~250 lines re-deriving all of it,
    //       including a second copy of the thunk list. That copy had drifted:
    //       it still thunked `esp_timer_impl_get_counter_reg` to a 32-bit fake,
    //       the exact stub the profile deleted because it left the high word
    //       undefined and silently broke every `millis()` deadline. A test
    //       running a boot path nobody ships is not a test of the thing we ship.
    let symbol_addrs = labwired_loader::extract_arduino_esp32_thunks(&elf_bytes);
    eprintln!(
        "[ereader-sim] resolved {} Arduino-ESP32 thunk symbols from ELF",
        symbol_addrs.len()
    );
    let mut booted = build_arduino_elf_machine(
        &image,
        symbol_addrs,
        &manifest,
        // Real dual-core: a second LX6 as APP_CPU (PRID 0xABAB →
        // xPortGetCoreID()==1, halted until PRO_CPU releases it via
        // ets_set_appcpu_boot_addr). arduino-esp32 pins loopTask to
        // CONFIG_ARDUINO_RUNNING_CORE=1, and with a real second core the
        // firmware drives the whole SMP rendezvous itself — no forged
        // handshake bytes, hence the empty `appcpu_up_flag_addrs`.
        &ArduinoElfBootOpts::default(),
    )
    .expect("build classic-ESP32 Arduino machine");
    eprintln!(
        "[ereader-sim] installed {} flash thunks",
        booted.profile.thunks_installed
    );

    // Capture the raw MOSI stream. "9 SPI transactions then nothing" says the
    // firmware stopped, but not WHICH byte it stopped after — and the GxEPD2
    // init sequence is identifiable byte-for-byte (0x12, 0x01 27 01 00, ...).
    // Reached through the booted machine's bus: the panel is attached by the
    // builder, so there is no pre-boot bus to configure.
    if let Some(idx) = booted.machine.bus.find_peripheral_index_by_name("spi3") {
        if let Some(spi) = booted.machine.bus.peripherals[idx]
            .dev
            .as_any_mut()
            .and_then(|a| a.downcast_mut::<Esp32Spi>())
        {
            spi.enable_byte_capture(4096);
        }
    }

    let uart_sink = booted.uart_sink.clone();
    let machine = &mut booted.machine;

    // ── 3. Step loop. Single-stepping on purpose — `machine.run()` would be
    //       faster but this loop is also the PC trace: the same-PC streak and
    //       the 64-deep distinct-PC ring below are what turn "it stalled" into
    //       "it stalled HERE". The cross-core FROM_CPU yield IPI that quiesces
    //       APP_CPU to IDLE is modeled inside the core (DPORT
    //       `cross_core_pending` → per-core `bus.pending_cpu_irqs`), so this
    //       harness does not bridge it — `machine.step()` delivers it.
    // Overridable so a diagnostic run can stop early: the interesting failure
    // (firmware stops mid-_InitDisplay) happens within the first few million
    // cycles, and paying 200M for it makes every iteration a 6-minute round trip.
    #[allow(non_snake_case)]
    let MAX_STEPS: u64 = std::env::var("LABWIRED_EREADER_MAX_CYCLES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(200_000_000);
    const SAMPLE_EVERY: u64 = 1_000_000;
    let mut step_count = 0u64;
    let mut last_pc = machine.cpu.get_pc();
    let mut same_pc_streak = 0u64;
    let mut samples: Vec<(u64, u32)> = Vec::new();
    let mut last_distinct: std::collections::VecDeque<u32> =
        std::collections::VecDeque::with_capacity(64);

    let mut step_err: Option<String> = None;
    let mut stalled = false;

    for _ in 0..MAX_STEPS {
        step_count += 1;

        if let Err(e) = machine.step() {
            let c1 = machine
                .cpu_secondary
                .as_ref()
                .map(|c| c.get_pc())
                .unwrap_or(0);
            step_err = Some(format!(
                "{e} (core0 pc=0x{:08x} core1 pc=0x{c1:08x})",
                machine.cpu.get_pc()
            ));
            break;
        }
        let pc = machine.cpu.get_pc();
        if pc == last_pc {
            same_pc_streak += 1;
            // 1M same-PC streak = definitely stalled (spin-wait that
            // we're not feeding correctly, or HALT loop).
            if same_pc_streak > 1_000_000 {
                stalled = true;
                break;
            }
        } else {
            same_pc_streak = 0;
            last_pc = pc;
            last_distinct.push_back(pc);
            if last_distinct.len() > 64 {
                last_distinct.pop_front();
            }
        }
        if step_count.is_multiple_of(SAMPLE_EVERY) {
            samples.push((step_count, pc));
        }
        // BUSY (GPIO4) is a sideband GPIO that neither panel model drives, so
        // it floats at the busy-active level and GxEPD2's _waitWhileBusy blocks
        // until its multi-second timeout — far beyond any sane cycle budget.
        // Hold it at the idle level (LOW; _busy_level is HIGH on SSD1680) so the
        // driver proceeds. The panel refreshes instantly in the model, so "never
        // busy" is the faithful reading of it.
        // Early-exit once the panel has painted — keeps dual-core iteration
        // fast (paint lands well before the 200M budget).
        if step_count.is_multiple_of(200_000) {
            if let Some(idx) = machine.bus.find_peripheral_index_by_name("spi3") {
                if let Some(p) = machine.bus.peripherals[idx]
                    .dev
                    .as_any()
                    .and_then(|a| a.downcast_ref::<Esp32Spi>())
                    .and_then(|spi| {
                        spi.attached_devices.iter().find_map(|d| {
                            d.as_any()
                                .and_then(|a| a.downcast_ref::<Ssd1680Tricolor290>())
                        })
                    })
                {
                    // The FIRST refresh is GxEPD2's clearScreen(0xFF) — an
                    // all-white plane. Exiting on it stops before drawPage()
                    // ever renders, so the ink assertion below could never pass.
                    // Wait for a refresh that actually carries ink.
                    if p.refresh_generation() >= 1 && p.black_plane().iter().any(|&b| b != 0xFF) {
                        break;
                    }
                }
            }
        }
    }

    // ── 4. Report.
    let final_pc = machine.cpu.get_pc();

    // Pull the panel back out and read its state.
    let spi3_idx = machine.bus.find_peripheral_index_by_name("spi3").unwrap();
    let any = machine.bus.peripherals[spi3_idx].dev.as_any().unwrap();
    let spi = any.downcast_ref::<Esp32Spi>().unwrap();
    let panel = spi
        .attached_devices
        .iter()
        .find_map(|d| {
            d.as_any()
                .and_then(|a| a.downcast_ref::<Ssd1680Tricolor290>())
        })
        .expect("panel attached");
    let refresh_gen = panel.refresh_generation();
    let power_on = panel.power_on();
    let txns = spi.transactions();

    eprintln!("[ereader-sim] ── final state ─────────────────────────────────");
    eprintln!("[ereader-sim] cycles executed:    {step_count}");
    eprintln!("[ereader-sim] final PC:           0x{final_pc:08x}");
    eprintln!("[ereader-sim] same-PC streak:     {same_pc_streak}");
    eprintln!("[ereader-sim] panel refresh_gen:  {refresh_gen}");
    eprintln!("[ereader-sim] panel power_on:     {power_on}");
    eprintln!("[ereader-sim] SPI3 transactions:  {txns}");
    if let Some(e) = &step_err {
        eprintln!("[ereader-sim] cpu step error:    {e}");
    }
    if stalled {
        eprintln!(
            "[ereader-sim] STALLED at PC=0x{final_pc:08x} (same PC for {same_pc_streak} cycles)"
        );
        eprintln!("[ereader-sim] last 64 distinct PCs (oldest → newest):");
        for p in last_distinct.iter() {
            eprintln!("    0x{p:08x}");
        }
    }
    eprintln!("[ereader-sim] last 10 PC samples:");
    for &(s, p) in samples.iter().rev().take(10) {
        eprintln!("    step {s:>10}: pc=0x{p:08x}");
    }

    // ── 5. Verdict. Painting = at least one refresh AND a non-blank framebuffer.
    // A refresh with an all-white black plane is a false positive (the DC line
    // was mis-latched and the 0x24 RAM stream was dropped); the real firmware
    // renders text, so the black plane must carry ink.
    let black_ink = panel.black_plane().iter().filter(|&&b| b != 0xFF).count();
    eprintln!("[ereader-sim] black-plane ink bytes: {black_ink}");

    let wire: Vec<u8> = spi.captured_bytes().to_vec();
    eprintln!("[ereader-sim] MOSI bytes captured: {}", wire.len());
    eprintln!(
        "[ereader-sim] wire: {}",
        wire.iter()
            .map(|b| format!("{b:02x}"))
            .collect::<Vec<_>>()
            .join(" ")
    );

    let uart_text = String::from_utf8_lossy(&uart_sink.lock().unwrap().clone()).to_string();
    eprintln!("[ereader-sim] ── firmware serial ─────────────────────────────");
    if uart_text.trim().is_empty() {
        eprintln!("[ereader-sim] (UART sink empty)");
    } else {
        for line in uart_text.lines() {
            eprintln!("[ereader-sim] | {line}");
        }
    }
    // Serial is now REAL on this path: the Arduino HardwareSerial nops are gone
    // and the sketch's own markers come back through the modelled UART. Assert
    // it, because "no UART output" was mistaken for firmware behaviour for a
    // year while the actual causes were a shadowed SYSCON model (SYSCLK_CONF
    // read 0xFFFFFFFF → getApbFrequency() → divide-by-zero) and, in this test,
    // a sink attached before the peripherals existed.
    assert!(
        uart_text.contains("[reader] setup() entered"),
        "expected the sketch's own Serial marker in the UART sink, got {} byte(s): {uart_text:?}",
        uart_text.len(),
    );

    assert!(
        refresh_gen >= 1,
        "labwired-ereader did not reach a panel refresh in {step_count} cycles \
         (final PC=0x{final_pc:08x}, refresh_gen={refresh_gen}, stalled={stalled})"
    );
    assert!(
        black_ink > 0,
        "labwired-ereader refreshed but rendered a BLANK black plane \
         ({black_ink} non-0xFF bytes) — the 0x24 framebuffer stream was dropped \
         (DC mis-latched?). The real firmware draws text, so this must be > 0."
    );

    // ── 6. The partition table has to be READABLE, not just present.
    //
    // These strings are not ours. They are ESP-IDF's and arduino-esp32's own
    // error text (`esp_partition.c`, `esp_core_dump_flash.c`,
    // `esp32-hal-misc.c:264`), which is what makes them worth asserting on: the
    // firmware decides whether to print them, from state we seeded, and nothing
    // in this repo can make the check pass by agreeing with itself.
    //
    // All three fired on every classic-ESP32 boot — visible live in the browser
    // on app.labwired.com — for two independent reasons:
    //
    //   * flash 0x8000 was erased, so there was no partition table at all
    //     (`boot::esp_partition_table` now supplies one);
    //   * `g_rom_flashchip.chip_size` was 0, so `spi_flash_mmap` rejected the
    //     read of that table with ESP_ERR_INVALID_ARG regardless
    //     (`install_arduino_esp32_profile` now seeds the descriptor).
    //
    // Either one alone leaves the twin lying about silicon, so both are asserted
    // here rather than trusted.
    assert!(
        !uart_text.contains("load_partitions returned"),
        "ESP-IDF could not read the partition table at flash 0x8000. 0x102 is \
         ESP_ERR_INVALID_ARG out of spi_flash_mmap — usually g_rom_flashchip \
         left unseeded (chip_size 0). Serial:\n{uart_text}"
    );
    assert!(
        !uart_text.contains("No core dump partition found"),
        "esp_core_dump_flash found no coredump partition — the partition table at \
         flash 0x8000 is missing or failed its ROM-MD5 verification. Serial:\n{uart_text}"
    );
    // And `nvs_flash_init()` must SUCCEED, not merely find its partition.
    // `initArduino()` prints this line for any non-zero return, so the absence
    // of it is the firmware's own verdict that NVS came up. Two error codes have
    // been seen here and each one was a different missing piece:
    //
    //   261    = 0x105 ESP_ERR_NOT_FOUND          — no NVS partition (table)
    //   24579  = 0x6003 ESP_ERR_FLASH_UNSUPPORTED_CHIP
    //                                             — no flash chip driver, which
    //                                               is what nopping the
    //                                               `spi_flash_chip_*` probes
    //                                               did (see NOP_STUBS)
    //
    // With the table, the `g_rom_flashchip` seed and the un-nopped driver, the
    // line is gone entirely and a `Preferences`-using sketch has real NVS
    // underneath it.
    assert!(
        !uart_text.contains("Failed to initialize NVS"),
        "initArduino() could not bring up NVS. 261 (0x105) means no NVS partition — \
         check seed_esp32_flash_image writes boot::esp_partition_table. 24579 (0x6003) \
         means no esp_flash chip driver — check the spi_flash_chip_* probes are not \
         being nop'd by the profile. Serial:\n{uart_text}"
    );
}
