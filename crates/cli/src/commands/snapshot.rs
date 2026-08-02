// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! `labwired snapshot` subcommands: capture and inspect machine snapshots.

use crate::*;

pub(crate) fn run_snapshot(args: SnapshotArgs) -> ExitCode {
    match args.command {
        SnapshotCommands::Capture(a) => run_snapshot_capture(a),
    }
}

/// Drive a firmware mid-flight in a headless sim and write a runtime
/// snapshot blob. The playground reads the same blob to skip cold boot.
///
/// The `arduino-esp32` profile mirrors what
/// `WasmSimulator::install_arduino_esp32_quirks` plus `step_with_esp32_aids`
/// do on the web side — same configure_xtensa_esp32 bus, same handshake,
/// same thunk setup, same IPI bridge cadence — so the captured state will
/// resume bit-identically inside the browser. Thunk PCs are resolved from the
/// ELF symbol table (no hand-curated per-firmware address list).
pub(crate) fn run_snapshot_capture(args: SnapshotCaptureArgs) -> ExitCode {
    use labwired_core::bus::SystemBus;
    use labwired_core::peripherals::components::{Ssd1680Tricolor290, Uc8151dTricolor290};
    use labwired_core::peripherals::esp32::spi::Esp32Spi;
    use labwired_core::peripherals::esp_xtensa_common::rom_thunks;
    use labwired_core::system::xtensa::configure_xtensa_esp32;
    use labwired_core::{Machine, SimulationError};
    use labwired_loader::{extract_arduino_esp32_thunks, load_elf_bytes};

    if args.profile != "arduino-esp32" {
        eprintln!(
            "error: unknown profile '{p}' — supported: 'arduino-esp32' (any Arduino-ESP32 ELF with symbols intact, auto-discovers thunk PCs)",
            p = args.profile
        );
        return ExitCode::from(EXIT_CONFIG_ERROR);
    }

    let elf_bytes = match std::fs::read(&args.firmware) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("error: cannot read firmware ELF {:?}: {e}", args.firmware);
            return ExitCode::from(EXIT_CONFIG_ERROR);
        }
    };

    // Bus + CPU — same configure_xtensa_esp32 that the WASM uses.
    let mut bus = SystemBus::new();
    let cpu = configure_xtensa_esp32(&mut bus);

    // Peripherals come from the board manifest, never hardcoded here. The
    // generic attach_esp32_external_devices factory wires every declared
    // external device (panel, etc.) onto its bus with the right model, CS and
    // DC pins. --system points at the board manifest (e.g. the ereader's
    // board.yaml declaring the SSD1680 e-paper on spi3, CS=GPIO5, DC=GPIO17).
    if let Some(sys_path) = &args.system {
        match labwired_config::SystemManifest::from_file(sys_path) {
            Ok(manifest) => {
                if let Err(e) = labwired_core::system::xtensa::attach_esp32_external_devices(
                    &mut bus, &manifest,
                ) {
                    eprintln!("error: attaching external devices from {sys_path:?}: {e}");
                    return ExitCode::from(EXIT_CONFIG_ERROR);
                }
            }
            Err(e) => {
                eprintln!("error: cannot load system manifest {sys_path:?}: {e}");
                return ExitCode::from(EXIT_CONFIG_ERROR);
            }
        }
    } else {
        eprintln!(
            "warning: no --system manifest; no external peripherals attached \
             (firmware that drives a panel will not render)"
        );
    }
    // Enable wire-byte capture on spi3 for snapshot diagnostics (a capture
    // concern, not a device wiring concern).
    if let Some(spi3_idx) = bus.find_peripheral_index_by_name("spi3") {
        if let Some(any) = bus.peripherals[spi3_idx].dev.as_any_mut() {
            if let Some(spi3) = any.downcast_mut::<Esp32Spi>() {
                spi3.enable_byte_capture(65536);
            }
        }
    }
    bus.refresh_peripheral_index();

    // Drain UART TX so a sketch's own Serial output is recoverable. Without a
    // sink nothing reads the peripheral and this path could only ever report a
    // panel paint — a sketch that proves itself by printing had no evidence at
    // all. Written beside the snapshot as `<output>.uart.log`.
    let uart_tx = std::sync::Arc::new(std::sync::Mutex::new(Vec::<u8>::new()));
    bus.attach_uart_tx_sink(uart_tx.clone(), false);

    let boxed: Box<dyn Cpu> = Box::new(cpu);
    let mut machine = Machine::new(boxed, bus);
    // Arduino-ESP32 sketches reach `xTaskCreatePinnedToCore(..., 1)`
    // for `loopTask` and others — without an APP_CPU to schedule onto,
    // FreeRTOS spins in `vListInsert` forever. Attach a secondary CPU
    // (PRID=0xABAB, halted at construction, released by
    // `ets_set_appcpu_boot_addr` during PRO_CPU boot).
    let cpu1 = labwired_core::cpu::xtensa_lx7::XtensaLx7::new_app_cpu();
    machine.cpu_secondary = Some(Box::new(cpu1));

    // Load firmware FIRST — load_firmware writes ELF segments into bus
    // memory, so any bytes we write before this risk being clobbered.
    // The handshake/header writes and `install_flash_thunk` (which patches
    // BREAK bytes into flash) must happen AFTER the ELF is in place.
    let program_image = match load_elf_bytes(&elf_bytes) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("error: load_elf_bytes: {e}");
            return ExitCode::from(EXIT_CONFIG_ERROR);
        }
    };
    if let Err(e) = machine.load_firmware(&program_image) {
        eprintln!("error: load_firmware: {e}");
        return ExitCode::from(EXIT_RUNTIME_ERROR);
    }
    // XtensaLx7::reset() leaves PC at the 0x40000400 BROM reset vector.
    // Skip BROM and jump straight to the ELF's app entry — same as WASM.
    // CHEAT(SKIP): bypasses the boot ROM and hand-seeds PC (SP seeded below).
    // See FIDELITY.md §C.
    machine.cpu.set_pc(program_image.entry_point as u32);

    // Resolve every Arduino-ESP32 symbol we know how to patch / thunk.
    // Empty for the reference firmware (stripped) — those fall back to hardcoded PCs.
    let symbol_addrs = extract_arduino_esp32_thunks(&elf_bytes);
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
    let mut thunks: Vec<(u32, rom_thunks::RomThunkFn)> = vec![
        (
            resolve("esp_timer_init", 0x4012_9034),
            rom_thunks::nop_return_zero,
        ),
        (
            resolve(
                "spi_flash_disable_interrupts_caches_and_other_cpu",
                0x4008_17dc,
            ),
            rom_thunks::nop_return_zero,
        ),
        (
            resolve(
                "spi_flash_enable_interrupts_caches_and_other_cpu",
                0x4008_188c,
            ),
            rom_thunks::nop_return_zero,
        ),
        (
            resolve("__retarget_lock_init_recursive", 0x4008_3384),
            rom_thunks::nop_return_zero,
        ),
        (
            resolve("__retarget_lock_close_recursive", 0x4008_339c),
            rom_thunks::nop_return_zero,
        ),
        (
            resolve("__retarget_lock_acquire_recursive", 0x4008_33b0),
            rom_thunks::nop_return_zero,
        ),
        (
            resolve("__retarget_lock_release_recursive", 0x4008_33cc),
            rom_thunks::nop_return_zero,
        ),
        (
            resolve("_esp_error_check_failed", 0x4008_bbd0),
            rom_thunks::nop_return_zero,
        ),
        (
            resolve("setCpuFrequencyMhz", 0x400e_99dc),
            rom_thunks::nop_return_zero,
        ),
        (
            resolve("esp_ota_get_running_partition", 0x400e_ae18),
            rom_thunks::nop_return_fake_ptr,
        ),
        (resolve("delay", 0x400e_5c28), rom_thunks::nop_return_zero),
    ];
    // The Arduino serial nops that used to be installed here are gone, along
    // with the empty loop that survived them. They dodged an Xtensa
    // divide-by-zero in _get_effective_baudrate whose real cause was an
    // apb_ctrl read-as-ones stub shadowing SYSCON at the same base — see
    // system/xtensa/esp32.rs. Serial now works on this path, which is what
    // makes the `<output>.uart.log` written below carry a sketch's own output.
    // Real-silicon noreturn functions — abort_halt prints diagnostics and
    // halts the CPU instead of returning. Without this, stubbing them as
    // nop_return_zero creates tight `assert → return → re-check → assert`
    // loops in xQueueGenericSend's parameter-validation path.
    for sym in &[
        "panic_abort",
        "__assert_func",
        "abort",
        "__assert",
        "__cxa_pure_virtual",
        "__cxa_throw",
    ] {
        if let Some(&pc) = symbol_addrs.get(*sym) {
            thunks.push((pc, rom_thunks::abort_halt));
        }
    }
    // ESP-IDF clock/efuse/cache/dport bring-up — the sim has no silicon
    // behind these so we stub them to return-0. Only installed when the
    // symbol is present in the ELF (Arduino-ESP32 profile).
    for sym in &[
        // newlib stdio init — sketch doesn't use stdio on render path
        "__sinit",
        "__sfp",
        "__sfp_lock_acquire",
        "__sfp_lock_release",
        "__sflags",
        "__swsetup_r",
        "__srefill_r",
        "__sread",
        "__swrite",
        "__seek",
        "__sclose",
        "esp_reent_init",
        "_fflush_r",
        "_fclose_r",
        "_fwrite_r",
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
        "esp_log_timestamp",
        // SPI-flash HAL — see loader::extract_arduino_esp32_thunks for why.
        "spi_flash_hal_configure_host_io_mode",
        "spi_flash_chip_generic_config_host_io_mode",
        "spi_flash_chip_generic_get_io_mode",
        "spi_flash_chip_generic_set_io_mode",
        "spi_flash_chip_generic_probe",
        "spi_flash_chip_generic_detect_size",
        "spi_flash_chip_generic_read",
        "spi_flash_chip_generic_yield",
        "spi_flash_chip_gd_probe",
        "spi_flash_chip_gd_detect_size",
        "spi_flash_chip_gd_get_io_mode",
        "spi_flash_chip_gd_set_io_mode",
        "spi_flash_init",
        "spi_flash_hal_init",
        "spi_flash_hal_supports_direct_write",
        "spi_flash_hal_supports_direct_read",
        "esp_flash_app_enable_os_functions",
        "esp_flash_app_disable_os_functions",
        "esp_flash_app_init",
        "esp_flash_init_main",
        "esp_flash_init_default_chip",
        "esp_flash_init",
        "esp_random",
        "esp_fill_random",
        "esp_log_early_timestamp",
        "esp_log_writev",
        "esp_log_write",
        "esp_log_buffer_hex_internal",
        "esp_log_buffer_char_internal",
        "esp_log_buffer_hexdump_internal",
        // log mutex (esp_log_impl_lock/unlock) — sim doesn't model the log
        // mutex queue, and the real impl calls xQueueGenericSend on an
        // uninitialized queue, tripping a NULL-pcHead assertion.
        "esp_log_impl_lock",
        "esp_log_impl_lock_timeout",
        "esp_log_impl_unlock",
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
        "__sfvwrite_r",
        "__swsetup_r",
        "__sflush_r",
        "_printf_r",
        "_fprintf_r",
        "_vfprintf_r",
        "_vprintf_r",
        "printf",
        "fprintf",
        "vfprintf",
        "vprintf",
        "puts",
        "fputs",
        "fputc",
        "putchar",
        "_puts_r",
        "_fputs_r",
        "_putchar_r",
        "_write_r",
        "write",
    ] {
        if let Some(&pc) = symbol_addrs.get(*sym) {
            thunks.push((pc, rom_thunks::nop_return_zero));
        }
    }
    // esp_chip_info needs more than nop — has to fill the output struct
    // with a plausible revision so the firmware's chip_revision >= min
    // assert passes.
    if let Some(&pc) = symbol_addrs.get("esp_chip_info") {
        thunks.push((pc, rom_thunks::esp_chip_info_stub));
    }
    // __getreent must return a non-NULL pointer to a zeroed reent struct.
    // Real silicon's per-task reent is set up by FreeRTOS task local
    // storage which we don't model — return a fixed pointer into DRAM
    // (always zeroed by RamPeripheral::new). ESP32-classic-specific
    // address; an `esp32s3` profile (if/when added) would need its own
    // version of this thunk pointing at S3's DRAM range.
    if let Some(&pc) = symbol_addrs.get("__getreent") {
        thunks.push((pc, rom_thunks::getreent_dram_fake_ptr));
    }
    // esp_timer_impl_get_counter_reg must return a monotonically increasing
    // value, otherwise polling-loop callers (esp-idf flash HAL, FreeRTOS
    // timeout helpers) spin forever.
    if let Some(&pc) = symbol_addrs.get("esp_timer_impl_get_counter_reg") {
        thunks.push((pc, rom_thunks::monotonic_counter_32));
    }
    // esp_clk_cpu_freq() — FreeRTOS divides CPU freq by tick rate to set
    // _xt_tick_divisor; without a meaningful value, divisor is 0 and the
    // timer ISR re-fires every CCOUNT cycle, pinning CPU 0 in the tick hook.
    if let Some(&pc) = symbol_addrs.get("esp_clk_cpu_freq") {
        thunks.push((pc, rom_thunks::esp_clk_cpu_freq_240mhz));
    }
    // Xtensa HAL register-window-file spill. The HAL impl walks WS bits
    // and spills each live slot's a0..a3 to its stack save area — but
    // our sim's transparent shadow-spill on CALL{n} leaves WS=1 on
    // displaced slots while the AR file has the callee's data, so the
    // HAL walk reads garbage (callee's a1 is often 0 → store to
    // 0xfffffff0 traps). The custom thunk emulates the spill using
    // shadow-stack snapshots when available.
    //
    // Only the `_nw` leaf (the spill loop that would trap) is thunked;
    // the `xthal_window_spill` wrapper is a thin CALL{n}-entered
    // PS-save shell that must run natively (its real ENTRY/RETW manage
    // the window). Thunking the wrapper returns via a0 = the caller's
    // return address, corrupting the first-task dispatch.
    if let Some(&pc) = symbol_addrs.get("xthal_window_spill_nw") {
        thunks.push((pc, rom_thunks::xthal_window_spill_thunk));
    }
    // xQueueCreateMutexStatic returns the caller's static buffer as the
    // handle. Callers (esp_newlib_locks_init in particular) assert that the
    // returned handle equals the buffer they passed in — a nop_return_zero
    // stub fails that check.
    if let Some(&pc) = symbol_addrs.get("xQueueCreateMutexStatic") {
        thunks.push((pc, rom_thunks::x_queue_create_mutex_static_echo));
    }
    // pxCurrentTCB symbol → feed into the rom_thunks side so the
    // xTaskGetCurrentTaskHandle thunk can read it. Arduino-ESP32's
    // main_task self-deletes after app_main returns via vTaskDelete(NULL),
    // which depends on this getter.
    if let Some(&addr) = symbol_addrs.get("pxCurrentTCB") {
        rom_thunks::PX_CURRENT_TCB_ADDR.with(|s| s.set(Some(addr)));
    }
    if let Some(&pc) = symbol_addrs.get("xTaskGetCurrentTaskHandle") {
        thunks.push((pc, rom_thunks::x_task_get_current_task_handle));
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
    eprintln!(
        "labwired-cli snapshot: installing {} thunks ({} resolved from ELF symbols)",
        thunks.len(),
        symbol_addrs.len(),
    );
    for &(pc, f) in &thunks {
        if let Err(e) = machine.bus.install_flash_thunk(pc, f) {
            eprintln!("error: install_flash_thunk @ {:#x}: {e}", pc);
            return ExitCode::from(EXIT_RUNTIME_ERROR);
        }
    }

    // Fake esp_image_header_t (24 bytes) at 0x3F40_0000, entry = ELF entry.
    let entry: u32 = program_image.entry_point as u32;
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

    // IPI bridge state — DPORT FROM_CPU intmatrix mapping observed each
    // cycle, raised on the CPU as an internal interrupt edge.
    let mut from_cpu_bit0: Option<u8> = None;
    let mut from_cpu_bit1: Option<u8> = None;
    let mut last_from_cpu0_val: u32 = 0;
    let mut last_from_cpu1_val: u32 = 0;

    eprintln!(
        "labwired-cli snapshot: stepping firmware to cycle {}",
        args.steps
    );
    // Instruction trace. Off unless asked for: attaching an observer forces
    // the interpreter path (a compiled block cannot emit per-step events), so
    // an always-on trace would silently tax every capture.
    let ring = args
        .trace_out
        .as_ref()
        .map(|_| std::sync::Arc::new(labwired_core::trace::RetiredRing::new(args.trace_last)));
    let observers: Vec<std::sync::Arc<dyn labwired_core::SimulationObserver>> = match &ring {
        Some(r) => vec![r.clone()],
        None => Vec::new(),
    };
    if let Some(path) = &args.trace_out {
        eprintln!(
            "labwired-cli snapshot: tracing the last {} retired instructions to {}",
            args.trace_last,
            path.display()
        );
    }
    // Written on EVERY exit path, faults included — a trace that only survives
    // a clean run is useless for the case it exists to explain.
    let dump_trace = |ring: &Option<std::sync::Arc<labwired_core::trace::RetiredRing>>| {
        let (Some(ring), Some(path)) = (ring, args.trace_out.as_ref()) else {
            return;
        };
        let entries = ring.entries();
        let total = ring.total_retired();
        let payload = serde_json::json!({
            "total_retired": total,
            "kept": entries.len(),
            "dropped": total.saturating_sub(entries.len() as u64),
            "instructions": entries,
        });
        match std::fs::File::create(path) {
            Ok(f) => {
                if let Err(e) = serde_json::to_writer_pretty(f, &payload) {
                    eprintln!("error: failed to write {}: {e}", path.display());
                } else {
                    eprintln!(
                        "labwired-cli snapshot: wrote {} of {} retired instructions to {}",
                        entries.len(),
                        total,
                        path.display()
                    );
                }
            }
            Err(e) => eprintln!("error: failed to create {}: {e}", path.display()),
        }
    };

    let config = labwired_core::SimulationConfig::default();
    let mut i: u64 = 0;
    let progress = args.progress_every;
    while i < args.steps {
        if let Ok(v) = machine.bus.read_u32(0x3FF0_0164) {
            let bit = (v & 0x1F) as u8;
            if v != 0 && bit < 32 {
                from_cpu_bit0 = Some(bit);
            }
        }
        if let Ok(v) = machine.bus.read_u32(0x3FF0_0168) {
            let bit = (v & 0x1F) as u8;
            if v != 0 && bit < 32 {
                from_cpu_bit1 = Some(bit);
            }
        }
        if let Ok(v0) = machine.bus.read_u32(0x3FF0_00DC) {
            if v0 != 0 && v0 != last_from_cpu0_val {
                if let Some(bit) = from_cpu_bit0 {
                    machine.cpu.raise_interrupt_bits(1u32 << bit);
                }
                let _ = machine.bus.write_u32(0x3FF0_00DC, 0);
            }
            last_from_cpu0_val = 0;
        }
        if let Ok(v1) = machine.bus.read_u32(0x3FF0_00E0) {
            if v1 != 0 && v1 != last_from_cpu1_val {
                if let Some(bit) = from_cpu_bit1 {
                    machine.cpu.raise_interrupt_bits(1u32 << bit);
                }
                let _ = machine.bus.write_u32(0x3FF0_00E0, 0);
            }
            last_from_cpu1_val = 0;
        }
        // Re-stamp the dual-core handshake bytes every 10k cycles so
        // start_other_core / do_other_cpu_settings keep seeing them as
        // "up." Write to each resolved symbol's [0]+[1] slots.
        if preseed_handshake && i.is_multiple_of(10_000) {
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
        }
        match machine.cpu.step(&mut machine.bus, &observers, &config) {
            Ok(()) => {}
            Err(SimulationError::BreakpointHit(_)) => {}
            Err(e) => {
                eprintln!(
                    "error: sim step at cycle {i} pc=0x{:08x}: {e}",
                    machine.cpu.get_pc()
                );
                dump_trace(&ring);
                return ExitCode::from(EXIT_RUNTIME_ERROR);
            }
        }
        // Dual-core: snapshot capture bypasses Machine::step, so the
        // appcpu-release + cpu1.step loop has to live here too. Plain
        // round-robin per-instruction interleaving — matches the chip's
        // true parallelism within the granularity of one instruction.
        // S32C1I is atomic within step() so spinlocks work correctly.
        if let Some(cpu1) = machine.cpu_secondary.as_mut() {
            if let Some(boot_addr) =
                labwired_core::peripherals::esp_xtensa_common::rom_thunks::APPCPU_BOOT_ADDR
                    .with(|s| s.take())
            {
                cpu1.set_pc(boot_addr);
                cpu1.set_sp(appcpu_initial_sp);
                // Run APP_CPU for real by default: it executes the firmware's
                // own `call_start_cpu1`, sets s_cpu_up/etc itself, and runs the
                // FreeRTOS SMP scheduler on core 1 — so we DON'T pre-seed the
                // handshake flags (that was a workaround for keeping cpu1
                // halted). Proven byte-identical to silicon with the real second
                // core (spi3=19033, ink=1429). LABWIRED_NO_DUALCORE=1 halts it
                // (falls back to the pre-seed path) for debugging SMP races.
                if std::env::var("LABWIRED_NO_DUALCORE").is_err() {
                    cpu1.unhalt();
                }
            }
            match cpu1.step(&mut machine.bus, &observers, &config) {
                Ok(()) => {}
                Err(SimulationError::BreakpointHit(_)) => {}
                Err(e) => {
                    eprintln!(
                        "error: sim step cpu1 at cycle {i} pc=0x{:08x}: {e}",
                        cpu1.get_pc()
                    );
                    dump_trace(&ring);
                    return ExitCode::from(EXIT_RUNTIME_ERROR);
                }
            }
        }
        machine.bus.tick_peripherals_with_costs();
        i += 1;
        if progress > 0 && i.is_multiple_of(progress) {
            let cpu1_state = match machine.cpu_secondary.as_ref() {
                Some(cpu1) => format!("  cpu1=0x{:08x}", cpu1.get_pc()),
                None => String::new(),
            };
            eprintln!(
                "  step {i:>10}  pc=0x{:08x}{cpu1_state}",
                machine.cpu.get_pc()
            );
            // Optional DC7 debug: dump vListInsert state on spin. Set
            // LABWIRED_DEBUG_LIST=1 to enable. Shows cpu intlevel,
            // xTaskQueueMutex state, pxList walk, and newItem state.
            if std::env::var("LABWIRED_DEBUG_LIST").is_ok() {
                eprintln!(
                    "    cpu0 intlevel={} a0=0x{:08x} a1=0x{:08x}",
                    machine.cpu.intlevel(),
                    machine.cpu.get_register(0),
                    machine.cpu.get_register(1)
                );
                let mux_owner = machine.bus.read_u32(0x3ffbf3b8).unwrap_or(0xDEAD);
                let mux_count = machine.bus.read_u32(0x3ffbf3bc).unwrap_or(0xDEAD);
                eprintln!("    xTaskQueueMutex.owner=0x{mux_owner:08x} .count={mux_count}");
                if let Some(cpu1) = machine.cpu_secondary.as_ref() {
                    eprintln!(
                        "    cpu1 intlevel={} a0=0x{:08x} a1=0x{:08x}",
                        cpu1.intlevel(),
                        cpu1.get_register(0),
                        cpu1.get_register(1)
                    );
                }
                let px_list = machine.cpu.get_register(2);
                let r = |off: u32| {
                    machine
                        .bus
                        .read_u32((px_list + off) as u64)
                        .unwrap_or(0xDEAD)
                };
                eprintln!(
                    "    cpu0 pxList=0x{px_list:08x} num={} idx=0x{:08x} end.val=0x{:08x} end.next=0x{:08x} end.prev=0x{:08x}",
                    r(0), r(4), r(8), r(12), r(16)
                );
                if let Some(cpu1) = machine.cpu_secondary.as_ref() {
                    let px_list1 = cpu1.get_register(2);
                    let r1 = |off: u32| {
                        machine
                            .bus
                            .read_u32((px_list1 + off) as u64)
                            .unwrap_or(0xDEAD)
                    };
                    eprintln!(
                        "    cpu1 pxList=0x{px_list1:08x} num={} idx=0x{:08x} end.val=0x{:08x} end.next=0x{:08x} end.prev=0x{:08x}",
                        r1(0), r1(4), r1(8), r1(12), r1(16)
                    );
                }
                let mut iter = r(12);
                let end_addr = px_list + 8;
                for hop in 0..6 {
                    if iter == end_addr {
                        eprintln!("      [hop {hop}] -> xListEnd (terminator)");
                        break;
                    }
                    let item_next = machine.bus.read_u32((iter + 4) as u64).unwrap_or(0xDEAD);
                    let item_val = machine.bus.read_u32(iter as u64).unwrap_or(0xDEAD);
                    eprintln!("      [hop {hop}] item=0x{iter:08x} val=0x{item_val:08x} next=0x{item_next:08x}");
                    iter = item_next;
                }
                let new_item = machine.cpu.get_register(3);
                let ri = |off: u32| {
                    machine
                        .bus
                        .read_u32((new_item + off) as u64)
                        .unwrap_or(0xDEAD)
                };
                eprintln!(
                    "    cpu0 newItem=0x{new_item:08x} item.val=0x{:08x} item.next=0x{:08x} item.prev=0x{:08x} item.owner=0x{:08x}",
                    ri(0), ri(4), ri(8), ri(12)
                );
            }
        }
    }

    // Sanity-check the captured state — we expect the panel to have been
    // driven through at least one refresh cycle by the time the snapshot
    // lands. Print this so the operator can tell "yes, this snapshot is
    // post-paint" without re-running the playground.
    if let Some(idx) = machine.bus.find_peripheral_index_by_name("spi3") {
        if let Some(any) = machine.bus.peripherals[idx].dev.as_any() {
            if let Some(spi3) = any.downcast_ref::<Esp32Spi>() {
                // Diagnostic: dump the full captured wire stream when asked, so
                // we can inspect the 0x24/0x26 RAM-write payloads end-to-end.
                if let Ok(path) = std::env::var("LABWIRED_DUMP_SPI") {
                    let _ = std::fs::write(&path, spi3.captured_bytes());
                    eprintln!(
                        "labwired-cli snapshot: dumped {} captured spi3 bytes to {path}",
                        spi3.captured_bytes().len()
                    );
                }
                eprintln!(
                    "labwired-cli snapshot: spi3 transactions={}",
                    spi3.transactions(),
                );
                let cap = spi3.captured_bytes();
                if !cap.is_empty() {
                    let head_n = cap.len().min(120);
                    let head_hex: Vec<String> =
                        cap[..head_n].iter().map(|b| format!("{b:02x}")).collect();
                    eprintln!(
                        "labwired-cli snapshot: first {head_n} spi3 bytes: {}",
                        head_hex.join(" ")
                    );
                    if cap.len() > 240 {
                        let tail = &cap[cap.len() - 120..];
                        let tail_hex: Vec<String> =
                            tail.iter().map(|b| format!("{b:02x}")).collect();
                        eprintln!(
                            "labwired-cli snapshot: last 120 spi3 bytes: {}",
                            tail_hex.join(" ")
                        );
                    }
                }
                for attached in &spi3.attached_devices {
                    if let Some(panel_any) = attached.as_any() {
                        if let Some(panel) = panel_any.downcast_ref::<Ssd1680Tricolor290>() {
                            let bp = panel.black_plane();
                            let non_ff = bp.iter().filter(|&&b| b != 0xFF).count();
                            eprintln!(
                                "labwired-cli snapshot: panel (ssd1680) state — refresh_generation={}, power_on={}, black-plane non-FF bytes={}/{}",
                                panel.refresh_generation(),
                                panel.power_on(),
                                non_ff,
                                bp.len(),
                            );
                        } else if let Some(panel) = panel_any
                            .downcast_ref::<labwired_core::peripherals::components::Ili9341>(
                        ) {
                            // An RGB565 TFT has no e-paper "refresh" — the
                            // frame memory IS the screen — so the evidence is
                            // DISPON plus how much of the framebuffer the
                            // firmware actually wrote. Without this line the
                            // panel produced no run evidence at all: a `display`
                            // oracle clause could not resolve, and a lab could
                            // only assert that `tft.begin()` returned, which is
                            // a host-side value the driver tracks itself and
                            // would read the same with no panel on the bus.
                            //
                            // `refresh_generation` is reported as 1 once the
                            // display is on and pixels exist, so one oracle
                            // shape covers both panel families.
                            let fb = panel.framebuffer();
                            let painted = fb.iter().filter(|&&b| b != 0x00).count();
                            let generation = u32::from(panel.display_on() && painted > 0);
                            let (w, h) = panel.dimensions();
                            // The most common non-black pixel, so the line says
                            // WHAT was drawn and not merely that something was.
                            // "10176 bytes changed" cannot be checked against a
                            // photo of the real panel; "top colour 0x07E0"
                            // (RGB565 green) can.
                            let mut counts: std::collections::HashMap<u16, usize> =
                                std::collections::HashMap::new();
                            for px in fb.chunks_exact(2) {
                                let v = u16::from_be_bytes([px[0], px[1]]);
                                if v != 0 {
                                    *counts.entry(v).or_default() += 1;
                                }
                            }
                            let top = counts
                                .iter()
                                .max_by_key(|&(_, n)| *n)
                                .map(|(v, n)| format!("0x{v:04X} x{n}"))
                                .unwrap_or_else(|| "none".to_string());
                            eprintln!(
                                "labwired-cli snapshot: panel (ili9341) state — refresh_generation={}, display_on={}, painted bytes={}/{}, {}x{}, top colour {}",
                                generation,
                                panel.display_on(),
                                painted,
                                fb.len(),
                                w,
                                h,
                                top,
                            );
                        } else if let Some(panel) = panel_any.downcast_ref::<Uc8151dTricolor290>() {
                            let bp = panel.black_plane();
                            let non_ff = bp.iter().filter(|&&b| b != 0xFF).count();
                            let rp = panel.red_plane();
                            let non_ff_red = rp.iter().filter(|&&b| b != 0xFF).count();
                            eprintln!(
                                "labwired-cli snapshot: panel (uc8151d) state — refresh_generation={}, power_on={}, black-plane non-FF bytes={}/{}, red-plane non-FF bytes={}/{}",
                                panel.refresh_generation(),
                                panel.power_on(),
                                non_ff,
                                bp.len(),
                                non_ff_red,
                                rp.len(),
                            );
                            // Render the panel as a PPM next to the
                            // snapshot output so an operator can visually
                            // confirm "yes, this looks like the real-HW
                            // panel image" before shipping the snapshot.
                            let (w, h) = panel.dimensions();
                            let stride = w / 8;
                            let mut ppm = format!("P6\n{w} {h}\n255\n").into_bytes();
                            for y in 0..h {
                                for x in 0..w {
                                    let idx = y * stride + x / 8;
                                    let bit = 7 - (x % 8);
                                    let black_bit = (bp[idx] >> bit) & 1;
                                    let red_bit = (rp[idx] >> bit) & 1;
                                    let (r, g, b) = if red_bit == 0 {
                                        (220u8, 30u8, 40u8)
                                    } else if black_bit == 0 {
                                        (0u8, 0u8, 0u8)
                                    } else {
                                        (245u8, 245u8, 240u8)
                                    };
                                    ppm.extend_from_slice(&[r, g, b]);
                                }
                            }
                            let ppm_path = args.output.with_extension("ppm");
                            if std::fs::write(&ppm_path, &ppm).is_ok() {
                                eprintln!(
                                    "labwired-cli snapshot: panel PPM written to {}",
                                    ppm_path.display()
                                );
                            }
                        }
                    }
                }
            }
        }
    }

    dump_trace(&ring);

    let snap = machine.take_runtime_snapshot();
    let bytes = snap.to_bytes();

    if let Some(parent) = args.output.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Err(e) = std::fs::write(&args.output, &bytes) {
        eprintln!("error: write {:?}: {e}", args.output);
        return ExitCode::from(EXIT_RUNTIME_ERROR);
    }

    // Persist whatever the sketch printed, and report the byte count so a caller
    // parsing stderr knows a log exists without stat-ing for it.
    {
        let bytes = uart_tx.lock().map(|b| b.clone()).unwrap_or_default();
        let uart_path = args.output.with_extension("uart.log");
        match std::fs::write(&uart_path, &bytes) {
            Ok(()) => eprintln!(
                "labwired-cli snapshot: uart {} bytes -> {:?}",
                bytes.len(),
                uart_path
            ),
            Err(e) => eprintln!("labwired-cli snapshot: warn: write {uart_path:?}: {e}"),
        }
    }

    // The universal final-state inspect block — the SAME `machine.inspect()`
    // payload the `test` path puts in result.json, written beside the snapshot
    // as `<output>.inspect.json`.
    //
    // Without this the profile path's only evidence was an e-paper refresh
    // scraped out of the stderr text above, so a classic-ESP32 sketch that
    // reported through GPIO — or through any peripheral that is not a panel on
    // spi3 — produced a clean run with nothing to verify against, and an oracle
    // asserting pin state could never pass however correct the firmware was.
    // Two run paths, two different answers to "what did the hardware do";
    // this makes it one.
    {
        let inspect_block = machine.inspect(
            None,
            &labwired_core::inspect::InspectOpts {
                include_bytes: false,
                peripheral: None,
            },
        );
        let inspect_path = args.output.with_extension("inspect.json");
        match serde_json::to_vec(&inspect_block) {
            Ok(json) => match std::fs::write(&inspect_path, &json) {
                Ok(()) => eprintln!(
                    "labwired-cli snapshot: inspect {} peripheral(s) -> {:?}",
                    inspect_block.peripherals.len(),
                    inspect_path
                ),
                Err(e) => eprintln!("labwired-cli snapshot: warn: write {inspect_path:?}: {e}"),
            },
            Err(e) => eprintln!("labwired-cli snapshot: warn: encode inspect block: {e}"),
        }
    }

    eprintln!(
        "labwired-cli snapshot: wrote {} bytes to {:?} (pc=0x{:08x} after {} cycles)",
        bytes.len(),
        args.output,
        machine.cpu.get_pc(),
        args.steps,
    );
    // Phase 3.2 JIT pilot (issue #124): report block hit count if the
    // build was compiled with `--features jit-core`. Without the feature
    // the trait default returns 0 and this line is harmless.
    let jit_hits = machine.cpu.jit_hit_count();
    if jit_hits > 0 {
        eprintln!("labwired-cli snapshot: jit block hits: {jit_hits}");
    }
    ExitCode::from(EXIT_PASS)
}
