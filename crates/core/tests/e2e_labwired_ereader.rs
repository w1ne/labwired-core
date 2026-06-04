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
// thunks, same IPI bridge, same step budget. If this test paints,
// the firmware paints in the playground too.
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
use labwired_core::peripherals::components::Uc8151dTricolor290;
use labwired_core::peripherals::esp32::spi::Esp32Spi;
use labwired_core::peripherals::esp32s3::rom_thunks;
use labwired_core::system::xtensa::configure_xtensa_esp32;
use labwired_core::{Bus, Cpu, Machine};
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

    // ── 1. Bring up an ESP32-classic and attach the UC8151D tri-color
    //       panel to spi3 (same as install_arduino_esp32_quirks). The
    //       default configure step doesn't attach a panel.
    let mut bus = SystemBus::new();
    let cpu = configure_xtensa_esp32(&mut bus);

    let spi3_idx = bus
        .find_peripheral_index_by_name("spi3")
        .expect("spi3 registered by configure_xtensa_esp32");
    {
        let any = bus.peripherals[spi3_idx].dev.as_any_mut().unwrap();
        let spi3 = any.downcast_mut::<Esp32Spi>().unwrap();
        spi3.attach(Box::new(Uc8151dTricolor290::new("GPIO5")));
    }
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

    // Boot rendezvous aid. The REAL APP_CPU runs call_start_cpu1 and its
    // scheduler/loopTask for real. We still pre-assert the *early*
    // startup-handshake flags (s_resume_cores / s_cpu_up / s_cpu_inited /
    // s_system_inited) below — these mark BROM/early-boot milestones the sim
    // skips. The FINAL barrier flag, s_other_cpu_startup_done, is NOT forged:
    // APP_CPU's scheduler quiesces to IDLE and its idle hook sets it for real
    // (see below). This is a boot aid, NOT the loopTask fake (gone — loopTask
    // genuinely runs on core 1).
    let mut handshake_bytes: Vec<u32> = Vec::new();
    for sym in &[
        "s_resume_cores",
        "s_cpu_up",
        "s_cpu_inited",
        "s_system_inited",
    ] {
        if let Some(&addr) = symbol_addrs.get(*sym) {
            let _ = machine.bus.write_u8(addr as u64, 0x01);
            let _ = machine.bus.write_u8(addr as u64 + 1, 0x01);
            handshake_bytes.push(addr);
            handshake_bytes.push(addr + 1);
        }
    }
    // s_other_cpu_startup_done is NO LONGER forged. APP_CPU's FreeRTOS
    // scheduler now genuinely quiesces to its IDLE task, whose idle hook
    // (other_cpu_startup_idle_hook_cb @ 0x400f8c08) writes this flag for
    // real — letting PRO_CPU's startup barrier (app_startup.c spin on
    // s_other_cpu_startup_done) clear. This works because the FROM_CPU_INTR1
    // crosscore yield is now delivered to APP_CPU via the correct APP-side
    // interrupt-matrix MAP register (see the IPI bridge below).
    // Re-assert the flags at the un-stall cycle (post-.bss) so .bss zero-init
    // can't wipe them — see rom_thunks::ets_set_appcpu_boot_addr.
    rom_thunks::set_appcpu_up_flags(handshake_bytes.clone());

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

    // Heap caps suite (bump allocator).
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
        "vListInsert",
    ] {
        push_named(&mut thunks, sym, rom_thunks::nop_return_zero);
    }

    // Non-NULL fake handle returns for FreeRTOS object creation.
    for sym in &[
        "xQueueCreateMutex",
        "xQueueCreateMutexStatic",
        "xQueueGenericCreate",
        "xSemaphoreCreateMutex",
        "xSemaphoreCreateBinary",
        "xSemaphoreCreateCounting",
        "xQueueCreateCountingSemaphore",
        "xEventGroupCreate",
    ] {
        push_named(&mut thunks, sym, rom_thunks::nop_return_fake_ptr);
    }

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
    push_named(
        &mut thunks,
        "xQueueSemaphoreTake",
        rom_thunks::return_pd_true,
    );
    push_named(&mut thunks, "xQueueGenericSend", rom_thunks::return_pd_true);
    push_named(
        &mut thunks,
        "ulTaskGenericNotifyTake",
        rom_thunks::return_pd_true,
    );
    push_named(&mut thunks, "spiStartBus", rom_thunks::spi_start_bus_fake);
    push_named(
        &mut thunks,
        "_ZN8SPIClass16beginTransactionE11SPISettings",
        rom_thunks::spi_class_begin_transaction,
    );

    // GxEPD2 cmd/data → straight into the attached UC8151D panel.
    push_named(
        &mut thunks,
        "_ZN10GxEPD2_EPD13_writeCommandEh",
        rom_thunks::gxepd_write_command,
    );
    push_named(
        &mut thunks,
        "_ZN10GxEPD2_EPD10_writeDataEh",
        rom_thunks::gxepd_write_data,
    );

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
    //       every 10k cycles, plus the cross-core IPI bridge sampling
    //       DPORT FROM_CPU_n mapping + trigger registers and synthesising
    //       the matching CPU interrupt edge.
    const MAX_STEPS: u64 = 200_000_000;
    const SAMPLE_EVERY: u64 = 1_000_000;
    let mut step_count = 0u64;
    let mut last_pc = machine.cpu.get_pc();
    let mut same_pc_streak = 0u64;
    let mut samples: Vec<(u64, u32)> = Vec::new();
    let mut last_distinct: std::collections::VecDeque<u32> =
        std::collections::VecDeque::with_capacity(64);

    // IPI bridge state.
    let mut from_cpu_bit0: Option<u8> = None;
    let mut from_cpu_bit1: Option<u8> = None;

    let mut step_err: Option<String> = None;
    let mut stalled = false;

    for _ in 0..MAX_STEPS {
        step_count += 1;

        // IPI bridge. DPORT_PRO_CPU_INTR_FROM_CPU_n_MAP_REG @ 0x3FF0_0164/_0168
        // captures the bit assignment, and writes to CPU_INTR_FROM_CPU_n
        // @ 0x3FF0_00DC/_00E0 trigger that bit on PRO_CPU.
        if let Ok(v) = machine.bus.read_u32(0x3FF0_0164) {
            let bit = (v & 0x1F) as u8;
            if v != 0 && bit < 32 {
                from_cpu_bit0 = Some(bit);
            }
        }
        // FROM_CPU_INTR1 (ESP32 source 25) is the crosscore IPI targeting
        // APP_CPU. Its source→CPU-interrupt routing lives in the APP-side
        // interrupt matrix MAP register: DPORT_APP_MAC_INTR_MAP_REG
        // (0x3FF0_0208) + 25*4 = 0x3FF0_026C. The firmware programs it from
        // APP_CPU's esp_crosscore_int_init→esp_intr_alloc→intr_matrix_set
        // (our esp_rom_route_intr_matrix thunk). Reading the *PRO*-side
        // binding (the old 0x3FF0_0168) always returns 0, so the yield IPI
        // was never delivered and APP_CPU's Tmr Svc task spun on
        // xQueueReceive→block→re-wake instead of quiescing to IDLE.
        if let Ok(v) = machine.bus.read_u32(0x3FF0_026C) {
            let bit = (v & 0x1F) as u8;
            if v != 0 && bit < 32 {
                from_cpu_bit1 = Some(bit);
            }
        }
        if let Ok(v0) = machine.bus.read_u32(0x3FF0_00DC) {
            if v0 != 0 {
                if let Some(bit) = from_cpu_bit0 {
                    machine.cpu.raise_interrupt_bits(1u32 << bit);
                }
                let _ = machine.bus.write_u32(0x3FF0_00DC, 0);
            }
        }
        if let Ok(v1) = machine.bus.read_u32(0x3FF0_00E0) {
            if v1 != 0 {
                // FROM_CPU_1 is the crosscore IRQ targeting APP_CPU (core 1):
                // raise it on the SECONDARY, not PRO. This is what lets
                // APP_CPU's self-yield (esp_crosscore_int_send_yield(1)) switch
                // it to IDLE so the startup idle hook can run.
                if let Some(bit) = from_cpu_bit1 {
                    if let Some(c1) = machine.cpu_secondary.as_mut() {
                        c1.raise_interrupt_bits(1u32 << bit);
                    }
                }
                let _ = machine.bus.write_u32(0x3FF0_00E0, 0);
            }
        }

        if let Err(e) = machine.step() {
            step_err = Some(format!("{e}"));
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
                                .and_then(|a| a.downcast_ref::<Uc8151dTricolor290>())
                        })
                    })
                {
                    if p.refresh_generation() >= 2 {
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
                .and_then(|a| a.downcast_ref::<Uc8151dTricolor290>())
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

    // ── 6. Verdict. Painting = at least one refresh().
    assert!(
        refresh_gen >= 1,
        "labwired-ereader did not reach a panel refresh in {step_count} cycles \
         (final PC=0x{final_pc:08x}, refresh_gen={refresh_gen}, stalled={stalled})"
    );
}
