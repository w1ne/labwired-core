// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT
//
// End-to-end smoke test for the `labwired-ereader` Arduino-ESP32 sketch.
//
// Goal: load the ereader's stock ELF (built with PlatformIO) into our
// ESP32-classic sim, mirror the wasm playground's
// `install_arduino_esp32_quirks` install path **minimally** by resolving
// every thunk address from the ELF's symbol table (so the test isn't
// pinned to one firmware build), and step long enough to either see the
// UC8151D panel get a `refresh()` or stall.
//
// This is the native-Rust counterpart to the wasm playground path —
// same panel attach, same SP seed, same handshake bytes, same ROM
// thunks, same step budget. The cross-core FROM_CPU yield IPI is
// modeled in the core (DPORT interrupt matrix), not bridged here. If
// this test paints, the firmware paints in the playground too.
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

use labwired_core::bus::SystemBus;
use labwired_core::cpu::xtensa_lx7::XtensaLx7;
use labwired_core::peripherals::components::Ssd1680Tricolor290;
use labwired_core::peripherals::esp32::spi::Esp32Spi;
use labwired_core::peripherals::esp32s3::rom_thunks;
use labwired_core::system::xtensa::configure_xtensa_esp32;
use labwired_core::{Cpu, Machine};
use std::path::PathBuf;

const DEFAULT_ELF: &str = "/tmp/labwired-ereader/build/labwired-ereader.ino.elf";

#[test]
#[ignore = "loads the 12MB labwired-ereader Arduino-ESP32 ELF and runs up to 200M cycles. \
            Run with: cargo test -p labwired-core --test e2e_labwired_ereader -- --ignored --nocapture"]
fn labwired_ereader_runs_to_panel_paint() {
    let elf_path = std::env::var("LABWIRED_EREADER_ELF")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from(DEFAULT_ELF));
    if !elf_path.exists() {
        eprintln!(
            "[skip] labwired-ereader ELF not found at {elf_path:?}; \
             build labwired-ereader and/or set LABWIRED_EREADER_ELF to enable"
        );
        return;
    }

    let elf_bytes = std::fs::read(&elf_path).expect("read ELF");
    let image = labwired_loader::load_elf(&elf_path).expect("parse ELF");

    // ── 1. Bring up an ESP32-classic and attach the panel from the board
    //       manifest via the generic attach_esp32_external_devices factory —
    //       the SAME path cli/wasm use. No peripheral is hardcoded here. The
    //       GxEPD2_290_C90c panel is an SSD1680 controller (see the GxEPD2 driver
    //       header); the factory maps the gxepd2_290_c90c alias to the SSD1680
    //       model, wires CS=GPIO5 and latches DC=GPIO17 (the GPIO GxEPD2 toggles
    //       via digitalWrite before each SPI.transfer — real wire framing).
    let mut bus = SystemBus::new();
    let cpu = configure_xtensa_esp32(&mut bus);

    let manifest: labwired_config::SystemManifest = serde_yaml::from_str(
        r#"
name: esp32-epaper-ereader
chip: esp32
external_devices:
  - id: epd
    type: gxepd2_290_c90c
    connection: spi3
    config:
      cs_pin: GPIO5
      dc_pin: GPIO17
"#,
    )
    .expect("parse inline ereader board manifest");
    labwired_core::system::xtensa::attach_esp32_external_devices(&mut bus, &manifest)
        .expect("attach e-paper panel from manifest");
    bus.refresh_peripheral_index();

    // Real dual-core: attach a second LX6 as APP_CPU (PRID 0xABAB →
    // xPortGetCoreID()==1, starts halted until PRO_CPU releases it via
    // ets_set_appcpu_boot_addr). Step 1 of the dual-core bring-up.
    let mut machine = Machine::new(cpu, bus).with_secondary_cpu(XtensaLx7::new_app_cpu());
    machine.load_firmware(&image).expect("load firmware");
    machine.cpu.set_pc(image.entry_point as u32);

    // ── 2. SP seed — real silicon's BROM places SP near the top of
    //       DRAM before jumping to call_start_cpu0; we skip BROM in the
    //       sim so seed it ourselves. Same for APP_CPU: the ROM sets its
    //       SP before releasing it to call_start_cpu1 (whose first insn is
    //       `entry a1,32`), so seed the secondary's SP in a separate DRAM
    //       region (above .bss @0x3ffc5ce8, below PRO_CPU's stack).
    machine.cpu.set_sp(0x3FFE_0000);
    if let Some(cpu1) = machine.cpu_secondary.as_mut() {
        cpu1.set_sp(0x3FFD_8000);
    }

    // ── 3. Symbol-driven thunk install. Resolves addresses from the
    //       ereader ELF and installs only the thunks for symbols
    //       actually present — silently skips missing ones. Identical
    //       in spirit to the wasm playground's install_arduino_esp32_quirks.
    let symbol_addrs = labwired_loader::extract_arduino_esp32_thunks(&elf_bytes);
    eprintln!(
        "[ereader-sim] resolved {} Arduino-ESP32 thunk symbols from ELF",
        symbol_addrs.len()
    );

    // No dual-core startup-handshake forges. With APP_CPU running for real,
    // the firmware drives the whole rendezvous itself: PRO_CPU releases
    // APP_CPU (ets_set_appcpu_boot_addr), APP_CPU runs call_start_cpu1 and
    // marks s_cpu_up[1]/s_cpu_inited[1]/s_system_inited[1], PRO_CPU sets
    // s_resume_cores, and APP_CPU's IDLE idle-hook sets s_other_cpu_startup_done
    // — all with no help from the harness. (Verified: forging these vs not
    // makes no difference to the paint; both ELFs reach refresh.) The
    // cross-core yield IPI that quiesces APP_CPU to IDLE is delivered by the
    // core's DPORT (Dport::cross_core_pending → bus.pending_cpu_irqs(core_id)),
    // not bridged here.
    //
    // set_appcpu_up_flags stays available for SINGLE-CORE frontends (wasm/cli)
    // where no APP_CPU exists to mark the flags; this dual-core test passes an
    // empty list so the ets_set_appcpu_boot_addr re-assert is a no-op.
    rom_thunks::set_appcpu_up_flags(Vec::new());

    // loopTask now runs on the REAL APP_CPU (core 1) — no repin. arduino-esp32
    // pins loopTask to CONFIG_ARDUINO_RUNNING_CORE=1, which is genuinely
    // modeled now. (Step 5 of dual-core bring-up: repin_loop_task deleted.)

    // pxCurrentTCB pointer seed for xTaskGetCurrentTaskHandle thunk.
    if let Some(&addr) = symbol_addrs.get("pxCurrentTCB") {
        rom_thunks::PX_CURRENT_TCB_ADDR.with(|s| s.set(Some(addr)));
        eprintln!("[ereader-sim] pxCurrentTCB @0x{addr:08x}");
    }

    // Build the thunk list — by-symbol lookups; missing symbols are
    // silently skipped (the sketch doesn't pull in that path).
    let mut thunks: Vec<(u32, rom_thunks::RomThunkFn)> = Vec::new();
    let push_named =
        |list: &mut Vec<(u32, rom_thunks::RomThunkFn)>, sym: &str, f: rom_thunks::RomThunkFn| {
            if let Some(&pc) = symbol_addrs.get(sym) {
                list.push((pc, f));
            }
        };

    // Heap: the sim-side bump allocator (default). It's debt — the real
    // ESP-IDF heap_caps should run on emulated DRAM. LABWIRED_REAL_HEAP=1
    // un-thunks it, but that currently walls.
    //
    // Diagnosed root cause (2026-06-04, via the WiFi fixture — same wall):
    // the real heap_caps_init mis-registers a heap region, so heap_caps_malloc
    // hands out pointers into non-heap memory. A vector_desc node ends up
    // pointing at memory whose first word is the bytes "lock" (0x6b636f6c),
    // and APP_CPU's esp_intr_alloc_intrstatus_bind faults dereferencing it
    // (node->next = "lock") while PRO_CPU spins in main_task waiting for
    // s_other_cpu_startup_done.
    //
    // Eliminated hypotheses (don't re-tread):
    //   * seeded-stack collision (#173) — moving SP to top-of-DRAM gives the
    //     IDENTICAL crash, and "lock" is a fixed string, not stack noise;
    //   * unbacked DRAM — 0x3FFA_0000..0x4000_0000 is fully RamPeripheral-backed;
    //   * broken atomic CAS — S32C1I is correct and the sim interleaves cores
    //     at instruction granularity, so spinlocks serialize;
    //   * unshared memory — both cores step against the same SystemBus.
    // Remaining suspect: the real heap_caps_init mis-registers a heap region
    // (wrong soc_memory_regions read via flash XIP, or a subtle instruction
    // mis-emulation in the TLSF/multi_heap registration path), so malloc
    // returns an out-of-region pointer. Next step: trace heap_caps_malloc
    // returns / multi_heap_register args under LABWIRED_REAL_HEAP=1. Real fix
    // lands there; then delete this bump allocator.
    if std::env::var("LABWIRED_REAL_HEAP").is_err() {
        push_named(
            &mut thunks,
            "heap_caps_init",
            rom_thunks::esp_idf_heap_caps_init,
        );
        push_named(
            &mut thunks,
            "heap_caps_malloc",
            rom_thunks::esp_idf_heap_caps_malloc,
        );
        push_named(
            &mut thunks,
            "heap_caps_calloc",
            rom_thunks::esp_idf_heap_caps_calloc,
        );
        push_named(
            &mut thunks,
            "heap_caps_free",
            rom_thunks::esp_idf_heap_caps_free,
        );
        push_named(
            &mut thunks,
            "heap_caps_realloc",
            rom_thunks::esp_idf_heap_caps_realloc,
        );
    }

    // No-op stubs for ESP-IDF / Arduino-ESP32 init paths we don't model.
    for sym in &[
        "esp_timer_init",
        "spi_flash_disable_interrupts_caches_and_other_cpu",
        "spi_flash_enable_interrupts_caches_and_other_cpu",
        "__retarget_lock_init_recursive",
        "__retarget_lock_close_recursive",
        "__retarget_lock_acquire_recursive",
        "__retarget_lock_release_recursive",
        "_esp_error_check_failed",
        "setCpuFrequencyMhz",
        "delay",
        "xQueueGiveMutexRecursive",
        "xQueueTakeMutexRecursive",
        "esp_ipc_init",
        "esp_ipc_isr_init",
        "esp_log_impl_lock",
        "esp_log_impl_lock_timeout",
        "esp_log_impl_unlock",
        "esp_panic_handler",
        "esp_panic_handler_reconfigure_wdts",
        "pthread_key_create",
        "pthread_setspecific",
        "pthread_getspecific",
        "pthread_mutex_init",
        "pthread_mutex_lock",
        "pthread_mutex_unlock",
        "_lock_acquire",
        "_lock_acquire_recursive",
        "_lock_release",
        "_lock_release_recursive",
        "_lock_init",
        "_lock_init_recursive",
        "_lock_close",
        "_lock_close_recursive",
        "_lock_try_acquire",
        "_lock_try_acquire_recursive",
        "esp_pthread_init",
        "esp_task_wdt_reset",
        "esp_task_wdt_init",
        "esp_task_wdt_add",
        "esp_task_wdt_delete",
        "esp_clk_init",
        "esp_perip_clk_init",
        "core_intr_matrix_clear",
        "esp_flash_init",
        "esp_flash_init_default_chip",
        "esp_flash_init_main",
        "esp_flash_app_init",
        "esp_flash_app_enable_os_functions",
        "esp_flash_app_disable_protect",
        "esp_flash_app_disable_os_functions",
        "esp_flash_read_chip_id",
        "esp_flash_chip_driver_initialized",
        "do_core_init",
        "do_secondary_init",
        // NOTE: `esp_startup_start_app` is INTENTIONALLY NOT STUBBED.
        // The real impl calls `vTaskStartScheduler()` which never returns
        // — control goes off to the first task. Stubbing it makes start_cpu0
        // fall into the `j .` safety-net loop at the bottom of start_cpu0.
        "esp_partition_main_flash_region_safe",
        "spi_flash_init",
        "spi_flash_init_chip_state",
        "esp_efuse_check_errors",
        "esp_dport_access_stall_other_cpu_start",
        "esp_dport_access_stall_other_cpu_end",
        "esp_cpu_unstall",
        "bootloader_flash_update_id",
        "bootloader_init_mem",
        "esp_mspi_pin_init",
        "esp_log_timestamp",
        "esp_log_early_timestamp",
        "esp_log_writev",
        "esp_random",
        "esp_fill_random",
        // HardwareSerial::begin chain hits `_get_effective_baudrate` →
        // `quou a10, a8, a10` with divisor=0 because getApbFrequency()
        // returns 0 in the sim → divide-by-zero exception. Stub the
        // whole begin so the UART init never runs (the sim has no UART
        // model the firmware can observe anyway).
        "_ZN14HardwareSerial5beginEmjaabmh",
        // Belt-and-braces: stub the leaf too, in case anything outside
        // begin reaches it.
        "_get_effective_baudrate",
        "_ZN14HardwareSerial5writeEh",
        "_ZN14HardwareSerial5writeEPKhj",
        "_ZN14HardwareSerial9availableEv",
        "_ZN14HardwareSerial5flushEv",
        "_ZN14HardwareSerial9readBytesEPcj",
        "_ZN14HardwareSerial9readBytesEPhj",
        "uartAvailable",
        "uartAvailableForWrite",
        "uartWrite",
        "uartWriteBuf",
        "_Z14serialEventRunv",
    ] {
        push_named(&mut thunks, sym, rom_thunks::nop_return_zero);
    }

    // Real FreeRTOS: queue/mutex/event-group create + vListInsert are NOT
    // thunked — the firmware's own FreeRTOS runs on the emulated registers +
    // heap. (The old fakes — nop'd vListInsert + fake-handle creates + always-
    // succeed ops — were pure debt: faking the create functions left their
    // list structures uninitialised, which forced faking everything built on
    // them. Removing all of it still paints refresh_gen=2.)

    // SPI-flash lock stubs (real impl asserts on uninitialised mutex).
    for sym in &[
        "spi_flash_init_lock",
        "spi_flash_op_lock",
        "spi_flash_op_unlock",
    ] {
        push_named(&mut thunks, sym, rom_thunks::nop_return_zero);
    }

    // esp_ota_get_running_partition → fake non-NULL ptr so assertions pass.
    push_named(
        &mut thunks,
        "esp_ota_get_running_partition",
        rom_thunks::nop_return_fake_ptr,
    );

    // Custom-return thunks.
    push_named(&mut thunks, "esp_chip_info", rom_thunks::esp_chip_info_stub);
    push_named(
        &mut thunks,
        "__getreent",
        rom_thunks::getreent_dram_fake_ptr,
    );
    push_named(
        &mut thunks,
        "esp_timer_impl_get_counter_reg",
        rom_thunks::monotonic_counter_32,
    );
    push_named(
        &mut thunks,
        "esp_clk_cpu_freq",
        rom_thunks::esp_clk_cpu_freq_240mhz,
    );
    push_named(
        &mut thunks,
        "xQueueCreateMutexStatic",
        rom_thunks::x_queue_create_mutex_static_echo,
    );
    push_named(
        &mut thunks,
        "xTaskGetCurrentTaskHandle",
        rom_thunks::x_task_get_current_task_handle,
    );
    // NO SPI init shims. GxEPD2_EPD::init() calls SPI.begin() → the real
    // compiled spiStartBus runs: it creates a real recursive bus mutex via
    // xQueueCreateMutex (real, IRAM-resident, backed by the real heap),
    // enables the SPI3 peripheral clock through DPORT, sets USER.USR_MOSI/
    // USR_MISO, and zeroes the FIFO. SPIClass::beginTransaction then takes that
    // real mutex. So spi_start_bus_fake, spi_class_begin_transaction, and the
    // xQueueSemaphoreTake/Send "force pdTRUE" lock shims are all GONE — the bus
    // mutex is a genuine FreeRTOS object and the SPI critical sections run for
    // real. xQueueCreateMutexStatic is still echoed (idle-task static mutex);
    // the SPI bus uses the dynamic xQueueCreateMutex, which is real.

    // NO gxepd cmd/data bypass. GxEPD2_EPD::_writeCommand / _writeData run for
    // real: digitalWrite(DC) → SPI.transfer(byte) → spiTransferByteNL writes the
    // SPI3 FIFO/MOSI_DLEN/CMD.USR registers, and our Esp32Spi peripheral drains
    // the byte to the panel framed by the latched DC GPIO. Bytes reach the panel
    // through real register machinery, not a Rust-side panel injection.

    // xthal_window_spill_nw — semantic spill via shadow stack. Only the
    // `_nw` leaf (the actual spill loop that would trap on the displaced
    // frames) is thunked; the `xthal_window_spill` wrapper is a thin
    // PS-save/restore shell that is CALL{n}-entered and must run its real
    // `entry / call0 _nw / retw` natively — thunking it returns via a0,
    // which is the *caller's* return address (the wrapper's ENTRY, which
    // would set up a0, is clobbered by the thunk's BREAK), corrupting the
    // return and faulting in xPortStartScheduler's first-task dispatch.
    push_named(
        &mut thunks,
        "xthal_window_spill_nw",
        rom_thunks::xthal_window_spill_thunk,
    );

    // Real-silicon noreturn — halt the CPU rather than letting assert →
    // return turn into a tight loop.
    for sym in &[
        "panic_abort",
        "__assert_func",
        "abort",
        "__assert",
        "__cxa_pure_virtual",
        "__cxa_throw",
    ] {
        push_named(&mut thunks, sym, rom_thunks::abort_halt);
    }

    let installed = thunks.len();
    for (pc, f) in thunks {
        machine
            .bus
            .install_flash_thunk(pc, f)
            .unwrap_or_else(|e| panic!("install thunk @{pc:#x}: {e}"));
    }
    eprintln!("[ereader-sim] installed {installed} flash thunks");

    // ── 4. Step loop. Mirrors step_with_esp32_aids: handshake keep-alive
    //       every 10k cycles. The cross-core FROM_CPU yield IPI that quiesces
    //       APP_CPU to IDLE is now modeled inside the core (DPORT
    //       `cross_core_pending` → per-core `bus.pending_cpu_irqs`), so this
    //       harness no longer bridges it — `machine.step()` delivers it.
    const MAX_STEPS: u64 = 200_000_000;
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
                    if p.refresh_generation() >= 1 {
                        break;
                    }
                }
            }
        }
    }

    // ── 5. Report.
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

    // ── 6. Verdict. Painting = at least one refresh AND a non-blank framebuffer.
    // A refresh with an all-white black plane is a false positive (the DC line
    // was mis-latched and the 0x24 RAM stream was dropped); the real firmware
    // renders text, so the black plane must carry ink.
    let black_ink = panel.black_plane().iter().filter(|&&b| b != 0xFF).count();
    eprintln!("[ereader-sim] black-plane ink bytes: {black_ink}");
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
}
