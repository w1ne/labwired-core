// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! The Arduino-ESP32 run profile — ONE home, shared by every runner.
//!
//! Booting an Arduino-ESP32 sketch takes more than loading its ELF: a set of
//! ROM/IDF entry points have to be redirected, dual-core handshake bytes and
//! CPU-frequency globals seeded, `loopTask` repinned off APP_CPU, and a fake
//! image header planted where the bootloader would have left one.
//!
//! That setup used to live inside `labwired snapshot capture` only. The
//! consequence was not cosmetic: `labwired test` — the declarative runner that
//! owns `stimuli:`, assertions, result.json and JUnit, i.e. the one a USER
//! writes tests against — had none of it, so a classic-ESP32 Arduino firmware
//! simply did not boot there. It produced ~47 bytes of UART and stopped. The
//! runner that could inject inputs could not boot the firmware; the runner
//! that could boot it could not inject. Anyone wanting both was stuck.
//!
//! So this is deliberately in `core`, not in the CLI: `labwired test`,
//! `labwired snapshot capture` and the WASM bridge can all call the same
//! function, and a fix to the boot path cannot land for one and miss another.
//! If you are tempted to copy a piece of this into a runner, don't — add a
//! parameter here instead.

use crate::peripherals::esp_xtensa_common::rom_thunks;
use crate::{Bus, Cpu, Machine};
use std::collections::HashMap;

// ---------------------------------------------------------------------------
// The thunk debt, declared in one place so it can be counted and ratcheted.
//
// A thunk is not the same thing as skipping the bootloader. Seeding the state
// the bootloader would have left is a one-time fact about where boot ended, and
// it is verifiable — you can read the register back. A thunk redirects a call
// the firmware makes for the WHOLE RUN, so it is a standing lie with nothing to
// compare against, and it drifts silently.
//
// That is not theoretical here. `esp_timer_impl_get_counter_reg` was thunked to
// return a 32-bit count through a2, leaving a3 — the high word —
// undefined. `esp_timer_get_time` computes `(hi << 31) | (lo >> 1)`, so garbage
// landed on bit 31 of every timestamp and every sketch using the standard
// `(int32_t)(millis() - deadline) >= 0` idiom silently did nothing, forever,
// while loop() kept being called. Nothing caught it because a thunk has no
// register to be wrong about. Modelling the LACT timer and seeding
// `g_ticks_per_us` — strictly less code than the fake — fixed it.
//
// So: these lists may only ever get SHORTER. `thunk_debt_only_falls` enforces
// that. Before adding one, ask whether the thing you are faking is bootloader
// work (seed it), a value that lives in a register or eFuse (model it), or
// firmware you do not want to run (that is the debt — say why, in a comment).
// ---------------------------------------------------------------------------

/// Stubs installed unconditionally, with a hand-curated fallback address for
/// the reference firmware's fully stripped ELF.
const FIXED_STUBS: &[(&str, u32, rom_thunks::RomThunkFn)] = &[
    // NB: esp_timer_init is deliberately NOT stubbed — it is what programs
    // LACT_CONFIG (enable + divider), and TIMG0 now models LACT, so it has
    // real registers to write and the timer only advances once it has.
    (
        "spi_flash_disable_interrupts_caches_and_other_cpu",
        0x4008_17dc,
        rom_thunks::nop_return_zero,
    ),
    (
        "spi_flash_enable_interrupts_caches_and_other_cpu",
        0x4008_188c,
        rom_thunks::nop_return_zero,
    ),
    (
        "__retarget_lock_init_recursive",
        0x4008_3384,
        rom_thunks::nop_return_zero,
    ),
    (
        "__retarget_lock_close_recursive",
        0x4008_339c,
        rom_thunks::nop_return_zero,
    ),
    (
        "__retarget_lock_acquire_recursive",
        0x4008_33b0,
        rom_thunks::nop_return_zero,
    ),
    (
        "__retarget_lock_release_recursive",
        0x4008_33cc,
        rom_thunks::nop_return_zero,
    ),
    (
        "_esp_error_check_failed",
        0x4008_bbd0,
        rom_thunks::nop_return_zero,
    ),
    (
        "setCpuFrequencyMhz",
        0x400e_99dc,
        rom_thunks::nop_return_zero,
    ),
    (
        "esp_ota_get_running_partition",
        0x400e_ae18,
        rom_thunks::nop_return_fake_ptr,
    ),
    ("delay", 0x400e_5c28, rom_thunks::nop_return_zero),
];

/// Real-silicon noreturn functions — `abort_halt` prints diagnostics and halts
/// the CPU instead of returning. Without this, stubbing them as
/// `nop_return_zero` creates tight `assert → return → re-check → assert` loops
/// in xQueueGenericSend's parameter-validation path.
const ABORT_STUBS: &[&str] = &[
    "panic_abort",
    "__assert_func",
    "abort",
    "__assert",
    "__cxa_pure_virtual",
    "__cxa_throw",
];

/// ESP-IDF clock/efuse/cache/dport bring-up and the newlib/stdio surface —
/// the sim has no silicon behind these, so they return 0. Installed only when
/// the symbol is present in the ELF.
const NOP_STUBS: &[&str] = &[
    // newlib stdio init — sketch doesn't use stdio on render path
    "esp_panic_handler",
    "esp_panic_handler_reconfigure_wdts",
    // xTaskGetCurrentTaskHandle gets a proper thunk below — returning
    // 0 breaks vTaskDelete(NULL) by passing NULL into prvDeleteTLS.
    "pthread_key_create",
    "pthread_setspecific",
    "pthread_getspecific",
    "pthread_mutex_init",
    "pthread_mutex_lock",
    "pthread_mutex_unlock",
    // Dual-core sim: with cpu_secondary actually running, FreeRTOS
    // primitives can use their real implementations — stubbing them
    // would defeat the purpose. Only esp_pthread_init stays stubbed
    // (it depends on per-task TLS we don't model).
    "esp_pthread_init",
    "esp_task_wdt_reset",
    "esp_task_wdt_init",
    "esp_task_wdt_add",
    "esp_task_wdt_delete",
    "esp_clk_init",
    "esp_perip_clk_init",
    "core_intr_matrix_clear",
    "esp_efuse_check_errors",
    "esp_dport_access_stall_other_cpu_start",
    "esp_dport_access_stall_other_cpu_end",
    "esp_cpu_unstall",
    "bootloader_flash_update_id",
    "bootloader_init_mem",
    "esp_mspi_pin_init",
    "spi_flash_init_chip_state",
    // Legacy `spi_flash_*` API shim and the OS-functions swap. Still stubbed:
    // they hook the flash driver up to FreeRTOS scheduling primitives the sim
    // does not need, and they are not on the read/erase/write path.
    "spi_flash_init",
    "esp_flash_app_enable_os_functions",
    "esp_flash_app_disable_os_functions",
    //
    // ── DELETED, not moved: the esp_flash chip driver. ──────────────────────
    //
    // `esp_flash_init` / `_init_main` / `_init_default_chip` / `esp_flash_app_init`
    // and the whole `spi_flash_chip_{generic,gd}_*` + `spi_flash_hal_*` probe
    // surface used to be nop'd here — 18 symbols. That looked cheap because
    // nothing on the panel-render path calls flash. It was not cheap.
    //
    // Nopping the probes means `esp_flash_default_chip->chip_drv` is never
    // assigned, so every `esp_partition_read` / `_write` / `_erase_range` fails
    // with 0x6003 ESP_ERR_FLASH_UNSUPPORTED_CHIP. Downstream of that,
    // `nvs_flash_init()` cannot format or read NVS, so `Preferences` — WiFi
    // credentials, calibration constants, boot counters, one of the most-used
    // Arduino-ESP32 APIs — fails on the twin and works on silicon. Every
    // classic-ESP32 boot also printed an `[E]` line about it, which teaches
    // users to ignore error output right before the errors that matter.
    //
    // None of that was necessary. `peripherals::esp32::spi::Esp32Spi` already
    // models the SPI NOR command set the driver probes with — RDID (answering
    // the same Winbond W25Q32 id `seed_rom_flashchip` writes into
    // `g_rom_flashchip`), RDSR, WREN, page program, sector/block erase, read —
    // so the firmware's own driver runs against modelled registers and
    // initialises. Unstubbing them takes the profile from 71 installed thunks
    // to 53 and makes the e-reader lab boot with a clean log.
    "esp_random",
    "esp_fill_random",
    // log mutex (esp_log_impl_lock/unlock) — sim doesn't model the log
    // mutex queue, and the real impl calls xQueueGenericSend on an
    // uninitialized queue, tripping a NULL-pcHead assertion.
    // esp_ipc_init/isr_init create the IPC task per core. Its
    // semaphore-wait turns into a tight loop in the sim (xQueueSemaphoreTake
    // is stubbed to pdTRUE), starving loopTask. Stub the init so the
    // task is never created — cross-core IPC isn't used on the
    // single-CPU render path.
    "esp_ipc_init",
    "esp_ipc_isr_init",
    // The HardwareSerial / uartWrite nops are GONE. A previous attempt
    // recorded here that unstubbing them "still emitted 0 UART bytes in 8M
    // steps, so something further down swallows the write" — that run was
    // made while apb_ctrl shadowed the SYSCON model, so SYSCLK_CONF read
    // 0xFFFFFFFF, getApbFrequency() returned 78125 Hz and
    // _get_effective_baudrate divided by zero. The clock tree was broken,
    // not the write path. With the syscon window fixed (see
    // system/xtensa/esp32.rs) the real Arduino serial path runs and
    // demo-labwired-ereader.elf emits its own markers.
    "_Z14serialEventRunv",
    // FreeRTOS recursive mutexes used by newlib stdio locks — same
    // null-queue assertion problem. Stub since sim is effectively
    // single-threaded on the panel-render path. xQueueCreateMutexStatic
    // gets a separate echo_arg0 thunk below (callers assert the returned
    // handle equals the static buffer they passed in). xQueueCreateMutex is
    // NOT stubbed — the SPI bus mutex is a real FreeRTOS object (see the
    // spiStartBus note below); faking its create returned an uninitialised
    // handle that forced faking every lock op on top of it and dropped the
    // SPI payload to the panel.
    "xQueueGiveMutexRecursive",
    "xQueueTakeMutexRecursive",
];

/// Stubs that need more than return-0, installed only when the symbol is
/// present. Each one is a specific claim about what the firmware should see.
const SPECIAL_STUBS: &[(&str, rom_thunks::RomThunkFn)] = &[
    // esp_chip_info has to fill the output struct with a plausible revision so
    // the firmware's `chip_revision >= min` assert passes.
    ("esp_chip_info", rom_thunks::esp_chip_info_stub),
    // __getreent must return a non-NULL pointer to a zeroed reent struct. Real
    // silicon's per-task reent is set up by FreeRTOS task-local storage, which
    // we don't model — return a fixed pointer into DRAM (always zeroed by
    // RamPeripheral::new). ESP32-classic-specific address; an `esp32s3` profile
    // (if/when added) needs its own version pointing at S3's DRAM range.
    ("__getreent", rom_thunks::getreent_dram_fake_ptr),
    // FreeRTOS divides CPU freq by tick rate to set _xt_tick_divisor; without a
    // meaningful value the divisor is 0 and the timer ISR re-fires every CCOUNT
    // cycle, pinning CPU 0 in the tick hook. 240 matches the g_ticks_per_us
    // seed above, so the two cannot drift.
    ("esp_clk_cpu_freq", rom_thunks::esp_clk_cpu_freq_240mhz),
    // Xtensa HAL register-window-file spill. The HAL impl walks WS bits and
    // spills each live slot's a0..a3 to its stack save area — but the sim's
    // transparent shadow-spill on CALL{n} leaves WS=1 on displaced slots while
    // the AR file has the callee's data, so the HAL walk reads garbage
    // (callee's a1 is often 0 → store to 0xfffffff0 traps). This emulates the
    // spill using shadow-stack snapshots when available.
    //
    // Only the `_nw` leaf (the spill loop that would trap) is thunked; the
    // `xthal_window_spill` wrapper is a thin CALL{n}-entered PS-save shell that
    // must run natively (its real ENTRY/RETW manage the window). Thunking the
    // wrapper returns via a0 = the caller's return address, corrupting the
    // first-task dispatch.
    (
        "xthal_window_spill_nw",
        rom_thunks::xthal_window_spill_thunk,
    ),
    // Returns the caller's static buffer as the handle. Callers
    // (esp_newlib_locks_init in particular) assert the returned handle equals
    // the buffer they passed in — a nop_return_zero stub fails that check.
    (
        "xQueueCreateMutexStatic",
        rom_thunks::x_queue_create_mutex_static_echo,
    ),
    // Arduino-ESP32's main_task self-deletes after app_main returns via
    // vTaskDelete(NULL), which depends on this getter. Reads pxCurrentTCB,
    // whose address is handed to the rom_thunks side below.
    (
        "xTaskGetCurrentTaskHandle",
        rom_thunks::x_task_get_current_task_handle,
    ),
];

/// Everything a caller needs back after the profile is installed.
pub struct ArduinoEsp32Profile {
    /// ELF symbol → address, as resolved from the firmware's own symbol table.
    /// Runners use it for diagnostics and for the APP_CPU boot stack.
    pub symbols: HashMap<&'static str, u32>,
    /// Initial APP_CPU stack pointer, applied when core 1 is unhalted.
    pub appcpu_initial_sp: u32,
    /// How many flash thunks were installed (for the runner's log line).
    pub thunks_installed: usize,
    /// Dual-core handshake byte addresses (0 when the symbol is absent).
    /// The step loop re-writes these periodically while the second core is
    /// coming up, so it needs them after install, not just during it.
    pub s_resume_cores: u32,
    pub s_cpu_up: u32,
    pub s_cpu_inited: u32,
    pub s_system_inited: u32,
    pub s_other_cpu_startup_done: u32,
    /// Whether the handshake pre-seed path is active (LABWIRED_NO_DUALCORE /
    /// LABWIRED_PRESEED_HANDSHAKE). The loop only re-seeds when it is.
    pub preseed_handshake: bool,
}

/// Install the Arduino-ESP32 profile onto an already-loaded `Machine`.
///
/// Call AFTER `load_firmware` and after seeding PC — writing thunks patches
/// BREAK bytes into flash, and ELF segment loading would clobber them.
///
/// `symbol_addrs` is the firmware's own symbol table, resolved by the caller
/// (`labwired_loader::extract_arduino_esp32_thunks`). It is passed IN rather
/// than parsed here so `core` keeps no ELF-format dependency — reading object
/// files is the loader's job, and this module's job is what to do with the
/// addresses once you have them.
///
/// `entry_point` is the ELF entry, planted into the fake image header.
pub fn install_arduino_esp32_profile<C: Cpu>(
    machine: &mut Machine<C>,
    symbol_addrs: HashMap<&'static str, u32>,
    entry_point: u32,
) -> Result<ArduinoEsp32Profile, String> {
    use crate::peripherals::esp_xtensa_common::rom_thunks;
    let entry: u32 = entry_point;

    let resolve_data =
        |sym: &str, fallback: u32| -> u32 { symbol_addrs.get(sym).copied().unwrap_or(fallback) };
    // APP_CPU initial stack — read once, used on cpu1 unhalt.
    // ESP-IDF puts the boot stack at `port_IntStackTop`; if the symbol
    // is missing (stripped ELF), fall back to a safe high-DRAM addr.
    let appcpu_initial_sp: u32 = symbol_addrs
        .get("port_IntStackTop")
        .copied()
        .unwrap_or(0x3FFB_F3A0);

    // loopTask xCoreID repin: Arduino-ESP32's app_main calls
    // xTaskCreateUniversal(loopTask, ..., xCoreID=1), pinning loopTask to
    // APP_CPU. We model only PRO_CPU, so rewrite the xCoreID immediate to 0.
    // Handles both legacy and IDF-5.x app_main layouts. See
    // rom_thunks::repin_loop_task.
    if let Some(&app_main_addr) = symbol_addrs.get("app_main") {
        match rom_thunks::repin_loop_task(&mut machine.bus, app_main_addr) {
            Some((addr, shape)) => eprintln!(
                "labwired-cli snapshot: repinned loopTask xCoreID at 0x{addr:08x} (1→0, {shape}; runs on PRO_CPU)"
            ),
            // Not benign. An unrecognised layout leaves loopTask pinned to
            // APP_CPU, where the sketch deadlocks the first time it contends a
            // FreeRTOS portMUX with PRO_CPU — parking in `spinlock_acquire`
            // partway through setup(). That reads as a firmware hang, so say
            // plainly that it was us. A silently-skipped repin cost a real
            // customer rig a mid-setup() stall that looked like their bug.
            None => eprintln!(
                "labwired-cli snapshot: warn: app_main at 0x{app_main_addr:08x} matched no known \
                 xCoreID layout — loopTask stays on APP_CPU and setup() may deadlock in \
                 spinlock_acquire. This is a simulator gap, not a firmware fault."
            ),
        }
    }

    // Arduino-ESP32 bootstrap — keep in sync with
    // `wasm/src/lib.rs::install_arduino_esp32_quirks` and the e2e test.
    machine.cpu.set_sp(0x3FFE_0000);
    // Handshake-byte pre-paint: resolve s_resume_cores / s_cpu_up /
    // s_cpu_inited / s_system_inited / s_other_cpu_startup_done from the ELF
    // symbol table and write 0x01 to both bytes of each.
    // Dual-core handshake pre-seed + 10k-cycle keep-alive — now only a FALLBACK
    // for when APP_CPU is halted (`LABWIRED_NO_DUALCORE=1`). By default we run
    // the real second core, which executes the firmware's own `call_start_cpu1`
    // and sets `s_cpu_up`/etc itself — no faking. The pre-seed was a workaround
    // for the previously-halted cpu1: `call_start_cpu0` unstalls APP_CPU then
    // spin-waits on `s_cpu_up[0..1]`, so with cpu1 halted PRO_CPU would spin
    // forever. With the real second core the firmware renders byte-identical to
    // silicon (spi3=19033, ink=1429) WITHOUT the pre-seed. Enable explicitly with
    // `LABWIRED_PRESEED_HANDSHAKE=1`.
    let preseed_handshake = std::env::var("LABWIRED_NO_DUALCORE").is_ok()
        || std::env::var("LABWIRED_PRESEED_HANDSHAKE").is_ok();
    // `g_ticks_per_us_pro` / `_app` — CPU MHz, written by
    // `ets_update_cpu_frequency()`. On silicon the ROM bootloader calls that
    // before it hands control to the app image; we start at the app entry
    // (see the CHEAT(SKIP) above that seeds PC), so nothing ever writes them
    // and they stay 0.
    //
    // That is not cosmetic. `esp_clk_apb_freq()` on ESP32-classic is
    // `MIN(g_ticks_per_us_pro, 80) * MHZ`, so a zero here reports a 0 Hz APB
    // bus, and `esp_timer_impl_update_apb_freq` aborts the whole boot on
    // `apb_ticks_per_us >= 3 && "divider value too low"`. Seeding them is
    // restoring skipped boot state, not stubbing behaviour: the firmware's own
    // esp_timer code then runs and programs the real LACT divider (80 MHz APB
    // / 2 MHz = 40) into the TIMG0 registers the model implements.
    //
    // 240 matches the `esp_clk_cpu_freq` thunk below, so the two cannot drift.
    for sym in ["g_ticks_per_us_pro", "g_ticks_per_us_app"] {
        let addr = resolve_data(sym, 0);
        if addr != 0 {
            let _ = machine.bus.write_u32(addr as u64, 240);
        }
    }

    // `g_rom_flashchip` — the `esp_rom_spiflash_chip_t` the BROM fills in when
    // it attaches the SPI flash. Same class of skipped boot state as
    // `g_ticks_per_us` above: a fact about where boot ended, readable back out
    // of memory, not a redirected call.
    //
    // Leaving it zeroed is not harmless. `spi_flash_mmap` starts with
    // `if (src_addr + size > g_rom_flashchip.chip_size) return
    // ESP_ERR_INVALID_ARG;` — with `chip_size == 0` EVERY mmap fails, including
    // the one `load_partitions()` uses to read the partition table. That is the
    //
    //     E (0) partition: load_partitions returned 0x102
    //     E (0) esp_core_dump_flash: No core dump partition found!
    //     [E][esp32-hal-misc.c:264] initArduino(): Failed to initialize NVS! Error: 261
    //
    // on every classic-ESP32 boot in the browser: 0x102 is ESP_ERR_INVALID_ARG
    // and 261 is 0x105 ESP_ERR_NOT_FOUND downstream of it. Writing a table at
    // flash 0x8000 without this seed changes nothing, because the firmware
    // never gets to look at it.
    //
    // This seed used to live in `cli::commands::esp32_boot_state`, so ONLY
    // `labwired test` had it — `labwired snapshot capture` and the whole
    // browser path did not. It is now in `boot::esp_partition_table`, which both
    // call.
    let flashchip = resolve_data("g_rom_flashchip", 0);
    if flashchip != 0 {
        crate::boot::esp_partition_table::seed_rom_flashchip(&mut machine.bus, flashchip);
    }

    let s_resume_cores = resolve_data("s_resume_cores", 0);
    let s_cpu_up = resolve_data("s_cpu_up", 0);
    let s_cpu_inited = resolve_data("s_cpu_inited", 0);
    let s_system_inited = resolve_data("s_system_inited", 0);
    let s_other_cpu_startup_done = resolve_data("s_other_cpu_startup_done", 0);
    if preseed_handshake {
        if s_resume_cores != 0 {
            let _ = machine.bus.write_u8(s_resume_cores as u64, 0x01);
        }
        if s_cpu_up != 0 {
            let _ = machine.bus.write_u8(s_cpu_up as u64, 0x01);
            let _ = machine.bus.write_u8(s_cpu_up as u64 + 1, 0x01);
        }
        if s_cpu_inited != 0 {
            let _ = machine.bus.write_u8(s_cpu_inited as u64, 0x01);
            let _ = machine.bus.write_u8(s_cpu_inited as u64 + 1, 0x01);
        }
        if s_system_inited != 0 {
            let _ = machine.bus.write_u8(s_system_inited as u64, 0x01);
            let _ = machine.bus.write_u8(s_system_inited as u64 + 1, 0x01);
        }
        if s_other_cpu_startup_done != 0 {
            let _ = machine.bus.write_u8(s_other_cpu_startup_done as u64, 0x01);
        }
        // Re-assert these flags the instant PRO_CPU releases APP_CPU, so
        // newer arduino-esp32 cores (whose `start_other_core` spin-waits
        // with a tight timeout) see APP_CPU "up" without depending on the
        // coarse 10k-cycle keep-alive below. Models APP_CPU bring-up; see
        // rom_thunks::ets_set_appcpu_boot_addr.
        let mut appcpu_up_flags: Vec<u32> = Vec::new();
        for (base, two_byte) in [
            (s_cpu_up, true),
            (s_cpu_inited, true),
            (s_system_inited, true),
            (s_resume_cores, false),
            (s_other_cpu_startup_done, false),
        ] {
            if base != 0 {
                appcpu_up_flags.push(base);
                if two_byte {
                    appcpu_up_flags.push(base + 1);
                }
            }
        }
        rom_thunks::set_appcpu_up_flags(appcpu_up_flags);
    }
    // RTC XTAL-freq probe = 40 MHz.
    let _ = machine.bus.write_u32(0x3FF4_80B0, 0x0050_0050);

    // Build the thunk address list. Each entry maps a flash PC to a
    // sim-side rom_thunks function. For unstripped ELFs we use the
    // already-parsed symbol map above; the reference firmware's fully stripped ELF
    // falls back to the hand-curated address list.
    let resolve =
        |sym: &str, fallback: u32| -> u32 { symbol_addrs.get(sym).copied().unwrap_or(fallback) };
    // heap_caps_* are NO LONGER thunked. The firmware's real ESP-IDF multi_heap
    // (TLSF) allocator runs on the emulated DRAM — same as the wasm boot path and
    // the e-reader e2e (crates/core/tests/e2e_labwired_ereader.rs, which paints
    // identically with the real heap: refresh_gen=1, 1429 ink bytes). The old
    // bump-allocator thunks were debt; the "real heap walls" symptom was an
    // APP_CPU dual-core bring-up bug (fixed by the real second core), not an
    // allocator bug.
    let mut thunks: Vec<(u32, rom_thunks::RomThunkFn)> = FIXED_STUBS
        .iter()
        .map(|&(sym, fallback, f)| (resolve(sym, fallback), f))
        .collect();
    // The Arduino serial nops that used to be installed here are gone, along
    // with the empty loop that survived them. They dodged an Xtensa
    // divide-by-zero in _get_effective_baudrate whose real cause was an
    // apb_ctrl read-as-ones stub shadowing SYSCON at the same base — see
    // system/xtensa/esp32.rs. Serial now works on this path, which is what
    // makes the `<output>.uart.log` written below carry a sketch's own output.
    for sym in ABORT_STUBS {
        if let Some(&pc) = symbol_addrs.get(*sym) {
            thunks.push((pc, rom_thunks::abort_halt));
        }
    }
    // ESP-IDF clock/efuse/cache/dport bring-up — the sim has no silicon
    // behind these so we stub them to return-0. Only installed when the
    // symbol is present in the ELF (Arduino-ESP32 profile).
    for sym in NOP_STUBS {
        if let Some(&pc) = symbol_addrs.get(*sym) {
            thunks.push((pc, rom_thunks::nop_return_zero));
        }
    }
    // esp_timer_impl_get_counter_reg is NOT thunked: TIMG0 models the LACT
    // timer it reads, so the firmware's own implementation runs and returns a
    // real 64-bit count. See the debt note on the const lists above for what
    // the thunk it replaced silently broke.
    for &(sym, f) in SPECIAL_STUBS {
        if let Some(&pc) = symbol_addrs.get(sym) {
            thunks.push((pc, f));
        }
    }
    // pxCurrentTCB's address is handed to the rom_thunks side so the
    // xTaskGetCurrentTaskHandle stub above can read it.
    if let Some(&addr) = symbol_addrs.get("pxCurrentTCB") {
        rom_thunks::PX_CURRENT_TCB_ADDR.with(|s| s.set(Some(addr)));
    }
    // NO SPI-bus lock shims and NO SPI init fakes. GxEPD2_EPD::init() calls
    // SPI.begin() → the real compiled spiStartBus runs: it creates a real
    // recursive bus mutex via xQueueCreateMutex (real, backed by the real heap),
    // enables the SPI3 peripheral clock through DPORT, and configures USER/FIFO.
    // SPIClass::beginTransaction then takes that real mutex. So spi_start_bus_fake,
    // spi_class_begin_transaction, and the xQueueSemaphoreTake / xQueueGenericSend /
    // ulTaskGenericNotifyTake "force pdTRUE" lock shims are all GONE — the bus
    // mutex is a genuine FreeRTOS object and the SPI critical sections run for
    // real, so the byte stream actually reaches the panel (the fakes matched the
    // transaction count but dropped the payload → blank render). Mirrors the
    // proven e2e path in crates/core/tests/e2e_labwired_ereader.rs.
    // xQueueCreateMutexStatic is still echoed above (idle-task static mutex); the
    // SPI bus uses the dynamic xQueueCreateMutex, which is real.
    // No GxEPD2 _writeCommand / _writeData bypass. The real compiled
    // GxEPD2_EPD::_writeCommand/_writeData run: digitalWrite(DC=GPIO17) →
    // SPI.transfer(byte) → spiTransferByteNL writes the SPI3 FIFO/MOSI_DLEN/
    // CMD.USR registers, and the Esp32Spi peripheral drains the byte to the
    // panel framed by the latched DC GPIO. Verified end-to-end against the real
    // PlatformIO firmware.elf (431 real SPI3 transactions → panel refresh) by
    // tests/e2e_labwired_ereader.rs. The arduino-esp32 panel attach above sets
    // the panel's DC source to GPIO17 so the framing is real.
    // Optional debug: install vListInsert short-circuit thunk that dumps
    // list state for first 20 calls. Used to diagnose SMP race issues in
    // the FreeRTOS scheduler. Enable with `LABWIRED_DEBUG_VLIST=1`.
    if std::env::var("LABWIRED_DEBUG_VLIST").is_ok() {
        if let Some(&pc) = symbol_addrs.get("vListInsert") {
            thunks.push((pc, rom_thunks::vlist_insert_debug));
        }
    }
    // Deliberately silent: this is a library, and the count comes back in
    // `thunks_installed` so each runner phrases its own log line. Printing
    // "labwired-cli snapshot: ..." from here would have every runner claim to
    // be the snapshot command.
    for &(pc, f) in &thunks {
        if let Err(e) = machine.bus.install_flash_thunk(pc, f) {
            return Err(format!("install_flash_thunk @ {pc:#x}: {e}"));
        }
    }

    // Fake esp_image_header_t (24 bytes) at 0x3F40_0000, entry = ELF entry.
    let header: [u8; 24] = [
        0xE9,
        0x01,
        0x00,
        0x00,
        (entry & 0xFF) as u8,
        ((entry >> 8) & 0xFF) as u8,
        ((entry >> 16) & 0xFF) as u8,
        ((entry >> 24) & 0xFF) as u8,
        0xEE,
        0,
        0,
        0,
        0,
        0,
        0,
        0,
        0,
        0,
        0,
        0,
        0,
        0,
        0,
        0,
    ];
    for (i, &b) in header.iter().enumerate() {
        let _ = machine.bus.write_u8(0x3F40_0000 + i as u64, b);
    }

    // Hand the secondary's boot stack to the Machine so the shared release
    // path can give the core a stack when it unhalts it. No runner needs to
    // know about this.
    machine.secondary_boot_sp = Some(appcpu_initial_sp);

    Ok(ArduinoEsp32Profile {
        symbols: symbol_addrs,
        appcpu_initial_sp,
        thunks_installed: thunks.len(),
        s_resume_cores,
        s_cpu_up,
        s_cpu_inited,
        s_system_inited,
        s_other_cpu_startup_done,
        preseed_handshake,
    })
}

/// Every firmware symbol this profile redirects to a sim-side stub.
///
/// This is the twin's thunk debt for Arduino-ESP32, in one list. It exists so
/// the number is a fact rather than an impression — see the note on the const
/// lists above for why a thunk is worse than a boot-state seed.
pub fn declared_thunk_symbols() -> Vec<&'static str> {
    FIXED_STUBS
        .iter()
        .map(|&(sym, _, _)| sym)
        .chain(ABORT_STUBS.iter().copied())
        .chain(NOP_STUBS.iter().copied())
        .chain(SPECIAL_STUBS.iter().map(|&(sym, _)| sym))
        .collect()
}

#[cfg(test)]
mod thunk_debt {
    use super::*;
    use std::collections::BTreeSet;

    /// The number of firmware symbols the Arduino-ESP32 profile fakes.
    ///
    /// This may only ever go DOWN. If you are here because the test failed
    /// after you added a stub: the fix is almost never to raise the ceiling.
    /// Ask which of the three cases you are in —
    ///
    ///  * bootloader work (`esp_timer_init`, heap/flash bring-up): seed the
    ///    state it would have left and let the firmware run. A seed is a fact
    ///    you can read back out of a register; a thunk is not.
    ///  * a value that lives in silicon (`esp_chip_info`, `esp_clk_cpu_freq`):
    ///    model the register or eFuse it comes from.
    ///  * firmware you genuinely cannot run yet: that is real debt. Raising the
    ///    ceiling is then a deliberate act, and the comment next to the stub has
    ///    to say what is missing.
    ///   * 75 → 56 when the `esp_flash` chip driver stopped being nop'd. See the
    ///     note in `NOP_STUBS` where those 18 symbols used to be: the sim already
    ///     modelled the SPI NOR command set they probe with, so the debt was
    ///     buying nothing and costing `nvs_flash_init()`.
    const CEILING: usize = 56;

    #[test]
    fn thunk_debt_only_falls() {
        let n = declared_thunk_symbols().len();
        assert!(
            n <= CEILING,
            "arduino-esp32 thunk count rose to {n} (ceiling {CEILING}). Each thunk is a \
             standing lie the twin cannot detect — prefer a boot-state seed or a modelled \
             register. See the doc comment on CEILING before changing it."
        );
    }

    #[test]
    fn no_symbol_is_thunked_twice() {
        let all = declared_thunk_symbols();
        let unique: BTreeSet<_> = all.iter().collect();
        assert_eq!(
            all.len(),
            unique.len(),
            "a symbol appears in more than one stub list — two entries claim the same PC, so \
             which stub wins depends on install order"
        );
    }
}
