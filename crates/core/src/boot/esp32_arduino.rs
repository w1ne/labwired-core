// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! THE classic-ESP32 Arduino-ELF boot path.
//!
//! Why this module exists
//! ======================
//! Every other chip family that boots here has one: `boot::esp32c3_rom` owns the
//! C3 ROM path, `boot::esp32s3_rom` owns the S3's. Classic ESP32 had none. Its
//! recipe — bus, dual-core CPU pair, external devices from the manifest, stack
//! seeding, symbol-driven thunk install, APP_CPU flag policy — lived as
//! hand-copied sequences in whichever caller needed it: the wasm playground's
//! `install_arduino_esp32_quirks`, and each end-to-end test that boots an
//! Arduino ELF.
//!
//! That is the shape every defect fixed on this chip has had. A copy that misses
//! a step does not fail loudly; it boots a machine that is subtly wrong, and the
//! symptom surfaces somewhere else entirely — a blank panel, an empty UART sink,
//! a `loop()` that never runs. The stack seeds below are the sharpest example:
//! they are two magic DRAM addresses that must not collide with `.bss` or with
//! each other, and nothing about a wrong value announces itself.
//!
//! So: one home. Callers describe WHAT they are booting (an image, its symbols,
//! a manifest) and this module owns HOW. Adding a step here reaches every caller
//! at once, which is the only property that makes the step reliable.
//!
//! Inputs are passed in
//! ====================
//! `symbol_addrs` and `image` are passed IN, exactly as
//! `install_arduino_esp32_profile` already requires, so a caller that already
//! parsed the ELF does not parse it twice. The symbol set itself is
//! [`ARDUINO_ESP32_SYMBOLS`] and [`arduino_esp32_symbols`] resolves it; the
//! loader's `extract_arduino_esp32_thunks` is that same function, which is
//! what lets a world node (built in core, which cannot depend on the loader)
//! boot an Arduino sketch exactly as the CLI and the debugger do.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use crate::bus::SystemBus;
use crate::cpu::XtensaLx7;
use crate::memory::ProgramImage;
use crate::system::xtensa::{
    attach_esp32_external_devices, configure_xtensa_esp32, install_arduino_esp32_profile,
    ArduinoEsp32Profile,
};
use crate::{Cpu, Machine};

/// PRO_CPU initial stack pointer.
///
/// Real silicon's BROM seeds SP near the top of DRAM before jumping to
/// `call_start_cpu0`. The sim skips BROM, so it must be seeded here.
pub const PRO_CPU_INITIAL_SP: u32 = 0x3FFE_0000;

/// APP_CPU initial stack pointer.
///
/// A SEPARATE DRAM region: above `.bss` (which ends around 0x3FFC_5CE8 on the
/// firmwares we ship) and below PRO_CPU's stack. The ROM sets this before
/// releasing APP_CPU to `call_start_cpu1`, whose first instruction is
/// `entry a1,32` — so an unseeded or overlapping value corrupts the first frame
/// rather than faulting somewhere you would think to look.
pub const APP_CPU_INITIAL_SP: u32 = 0x3FFD_8000;

/// How to bring up the machine. Defaults match the shipped browser path.
#[derive(Debug, Clone)]
pub struct ArduinoElfBootOpts {
    /// Attach a real second LX6 as APP_CPU (PRID 0xABAB → `xPortGetCoreID()==1`,
    /// halted until PRO_CPU releases it via `ets_set_appcpu_boot_addr`).
    ///
    /// arduino-esp32 pins `loopTask` to `CONFIG_ARDUINO_RUNNING_CORE=1`. With a
    /// real APP_CPU that is modelled and the firmware drives the whole rendezvous
    /// itself. With a single core there is nobody to mark the startup flags, so
    /// `appcpu_up_flag_addrs` has to forge them instead — see that field.
    pub dual_core: bool,
    /// Addresses to force-mark as "APP_CPU is up" for SINGLE-CORE frontends.
    ///
    /// Meaningless (and passed empty) when `dual_core` is set: the second CPU
    /// marks them for real. Forging them in the dual-core case is not merely
    /// redundant, it papers over a genuine bring-up failure.
    pub appcpu_up_flag_addrs: Vec<u32>,
    pub pro_cpu_sp: u32,
    pub app_cpu_sp: u32,
}

impl Default for ArduinoElfBootOpts {
    fn default() -> Self {
        Self {
            dual_core: true,
            appcpu_up_flag_addrs: Vec::new(),
            pro_cpu_sp: PRO_CPU_INITIAL_SP,
            app_cpu_sp: APP_CPU_INITIAL_SP,
        }
    }
}

/// Every Arduino-ESP32 / ESP-IDF / Arduino-core symbol the simulator resolves
/// from an Arduino-ESP32 firmware: flash-thunk targets, the dual-core
/// handshake bytes, the CPU-frequency globals the ROM bootloader would have
/// written, and optional markers (`loopTask`, `app_main`).
///
/// Lives here rather than in `labwired-loader` so that every caller — the
/// CLI, the debugger, and a world node built by `system::node` (core cannot
/// depend on the loader) — resolves the same set.
pub const ARDUINO_ESP32_SYMBOLS: &[&str] = &[
    // ── SMP / APP_CPU bring-up (rom-boot dual-core). ────────────────────
    "call_start_cpu1",
    "esp_cpu_unstall",
    // ── Flash thunks — heap caps suite (bump allocator). ────────────────
    "heap_caps_init",
    "heap_caps_malloc",
    "heap_caps_calloc",
    "heap_caps_free",
    "heap_caps_realloc",
    // ── Flash thunks — no-op stubs. ─────────────────────────────────────
    "esp_timer_init",
    "spi_flash_disable_interrupts_caches_and_other_cpu",
    "spi_flash_enable_interrupts_caches_and_other_cpu",
    "__retarget_lock_init_recursive",
    "__retarget_lock_close_recursive",
    "__retarget_lock_acquire_recursive",
    "__retarget_lock_release_recursive",
    // Newlib-stdio-driven mutex API. Real silicon backs these via
    // FreeRTOS recursive mutexes whose handles live in (uninitialised
    // in our sim) static memory; calling the real impl asserts on
    // pcHead != NULL. Stub to no-op since the sim is effectively
    // single-threaded on the render path.
    "xQueueGiveMutexRecursive",
    "xQueueTakeMutexRecursive",
    "xQueueCreateMutex",
    "xQueueCreateMutexStatic",
    "xQueueGenericCreate",
    "xEventGroupCreate",
    "spi_flash_init_lock",
    "spi_flash_op_lock",
    "spi_flash_op_unlock",
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
    "esp_startup_start_app",
    "esp_partition_main_flash_region_safe",
    "spi_flash_init",
    "spi_flash_init_chip_state",
    // xQueueCreateMutex returns NULL (stubbed), so SPIClass and friends
    // end up storing NULL as their internal mutex. We force these to
    // return pdTRUE so the call path proceeds; real silicon would only
    // reach this on a held mutex anyway in the single-task render flow.
    "xQueueSemaphoreTake",
    // Arduino-ESP32's loopTask wraps `ulTaskGenericNotifyTake(pdTRUE,
    // portMAX_DELAY)` around setup()/loop() to coordinate with the
    // wdt-feed timer. In single-task sim there's nobody to notify it,
    // so the take blocks forever. Stub to return non-zero so
    // setup()/loop() actually run.
    "ulTaskGenericNotifyTake",
    // SPIClass::endTransaction calls xQueueGenericSend (the give side
    // of xSemaphoreGive) on the same NULL mutex. Force success too.
    "xQueueGenericSend",
    // esp_ipc_init creates the per-core IPC task which spin-blocks on
    // an empty semaphore. With our take-returns-pdTRUE stub above,
    // that "block" becomes a tight loop — never yielding, never
    // letting loopTask run. Stub esp_ipc_init out and skip the IPC
    // task altogether; cross-core IPC isn't needed on the
    // single-CPU render path.
    "esp_ipc_init",
    "esp_ipc_isr_init",
    // HardwareSerial-only stubs — leave Print/Stream alone so virtual
    // dispatch through Print::print → Adafruit_GFX::write → drawPixel
    // (the display.print path) keeps working. The original spin was
    // in HardwareSerial::write's buffer-available wait, not in Print.
    "_ZN14HardwareSerial5writeEh",
    "_ZN14HardwareSerial5writeEPKhj",
    "_ZN14HardwareSerial9availableEv",
    "_ZN14HardwareSerial5flushEv",
    "_ZN14HardwareSerial9readBytesEPcj",
    "_ZN14HardwareSerial9readBytesEPhj",
    // HardwareSerial::begin(unsigned long, unsigned int, signed char,
    // signed char, bool, unsigned long, unsigned char) — Arduino-ESP32's
    // serial init walks through `_get_effective_baudrate`, which divides
    // by `getApbFrequency()`. Our sim doesn't drive that register, so
    // the division is by zero. Skip the whole begin() rather than emulate
    // the baud calculation; we don't model UART output anyway. The
    // demangled placeholder above (`HardwareSerial::begin(...)`) never
    // matched object's mangled symbol name; the mangled form here does.
    "_ZN14HardwareSerial5beginEmjaabmh",
    "_get_effective_baudrate",
    "uartAvailable",
    "uartAvailableForWrite",
    "uartWrite",
    "uartWriteBuf",
    "_Z14serialEventRunv",
    // SPI bus init — real impl needs DPORT clock-enable we don't
    // model, so it returns NULL → SPIClass._spi = NULL → all
    // downstream spiTransferByte calls bail without touching the SPI
    // peripheral. Custom thunk returns a fake spi_t with dev = SPI3.
    "spiStartBus",
    // The Arduino SPI global. We resolve it for diagnostics; lazy
    // init happens via the SPIClass::beginTransaction thunk.
    "SPI",
    "_ZN8SPIClass16beginTransactionE11SPISettings",
    // GxEPD2_EPD::_writeCommand / _writeData — intercepted at the
    // top of the Arduino driver so DC=cmd vs DC=data routing is
    // explicit (the real silicon uses a sideband GPIO pin we don't
    // observe in the SPI peripheral model). The thunks write the
    // byte straight into the attached UC8151D panel's
    // `command_byte` / `data_byte` API, bypassing the
    // Arduino-ESP32 SPI library (whose `_spi` struct fields aren't
    // fully populated without a real `SPI.begin()` call) and the
    // Esp32Spi FIFO routing. Same byte stream the real panel
    // receives, byte-for-byte.
    "_ZN10GxEPD2_EPD13_writeCommandEh",
    "_ZN10GxEPD2_EPD10_writeDataEh",
    "_esp_error_check_failed",
    "setCpuFrequencyMhz",
    "esp_ota_get_running_partition", // fake non-null ptr
    // NB: HardwareSerial::begin lives above as its mangled symbol
    // (_ZN14HardwareSerial5beginEmjaabmh) since object/goblin return
    // mangled names from the symbol table.
    "delay",
    // ── Dual-core handshake bytes (in .bss). ────────────────────────────
    // `call_start_cpu0` busy-waits until various handshake bytes
    // become non-zero. Single-CPU sim has to pre-write 0x01 to each
    // of these. Resolving the .bss symbol gives the per-firmware
    // base address.
    "s_resume_cores",
    "s_cpu_up",
    "s_cpu_inited",
    "s_system_inited",
    "s_other_cpu_startup_done",
    // ── CPU-frequency globals the ROM bootloader normally sets. ──────────
    // `ets_update_cpu_frequency()` writes both; the ROM calls it before
    // handing off to the app image, and we start at the app entry, so
    // nothing writes them and they stay 0. `esp_clk_apb_freq()` is
    // `MIN(g_ticks_per_us_pro, 80) * MHZ` on ESP32-classic, so zero here
    // reports a 0 Hz APB bus and `esp_timer_impl_update_apb_freq` aborts
    // boot on `apb_ticks_per_us >= 3`. The cli seeds these — see
    // snapshot.rs. Note these resolve as ABSOLUTE (nm type `A`) symbols,
    // not .bss, because the ROM linker script fixes their addresses.
    "g_ticks_per_us_pro",
    "g_ticks_per_us_app",
    // ── ROM flash-chip descriptor (`esp_rom_spiflash_chip_t`). ───────────
    // The BROM fills this in when it attaches the SPI flash. We start at
    // the app entry, so it stays zeroed — and `spi_flash_mmap` rejects
    // every request with `src_addr + size > g_rom_flashchip.chip_size`,
    // i.e. ESP_ERR_INVALID_ARG (0x102). That is what
    // `load_partitions returned 0x102` on every classic-ESP32 boot was:
    // not a bad partition table, a flash chip the firmware thinks is
    // 0 bytes long. Seeded in `install_arduino_esp32_profile`.
    "g_rom_flashchip",
    // ── Optional markers. ────────────────────────────────────────────────
    "app_main",
    "loopTask",
    // ── Panic / abort / assert path — stubbed to no-op so the firmware
    //    doesn't double-fault when an init-time assertion (esp_reent_init,
    //    multi_heap, etc.) fires and the assert handler itself re-enters
    //    stdio which re-asserts. Real silicon has the panic vector wired
    //    to a reboot; our sim has no reboot, so without a stub we loop
    //    forever between __assert_func and __sfp / __getreent / __utoa.
    "panic_abort",
    "__assert_func",
    "abort",
    "__assert",
    "__cxa_pure_virtual",
    "__cxa_throw",
    // ── newlib stdio init — looping forever in __sfp / __swsetup_r /
    //    __srefill_r / __sinit because esp_reent_init didn't construct
    //    a valid reent struct (it would on real silicon via FreeRTOS
    //    task-local storage we don't model). The sketch doesn't use
    //    stdio on the panel-render path, so stubbing the lot is fine.
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
    // ── more FreeRTOS / panic / pthread bring-up the sim can't model.
    //    All stubbed to no-op or fake-ptr; consumers that don't actually
    //    use the returned data (which is most setup() / loop() code on
    //    the sketch's render path) get to keep running.
    "__getreent",        // returns a DRAM pointer (zeroed reent struct)
    "esp_panic_handler", // we don't want to enter the panic path at all
    "esp_panic_handler_reconfigure_wdts",
    "xTaskGetCurrentTaskHandle",
    "pthread_key_create",
    "pthread_setspecific",
    "pthread_getspecific",
    "pthread_mutex_init",
    "pthread_mutex_lock",
    "pthread_mutex_unlock",
    // ── FreeRTOS port-layer critical sections.  Single-task sim has no
    //    concurrent access to guard, no other core to interrupt, no
    //    other task to preempt — return success immediately.  Real
    //    silicon's RSIL+spinlock spin forever if the lock owner is
    //    a CPU we don't model.
    // Dual-core sim: real FreeRTOS primitives are used now that
    // cpu_secondary runs. Only esp_pthread_init stays stubbed —
    // per-task pthread TLS isn't modeled.
    "esp_pthread_init",
    // ── Watchdog refresh — sketches loop fast in sim so the WDT-feed
    //    matters less, but stub it to avoid any extra cycles burned.
    "esp_task_wdt_reset",
    "esp_task_wdt_init",
    "esp_task_wdt_add",
    "esp_task_wdt_delete",
    // ── ESP-IDF clock/efuse/cache init — sim has no real silicon
    //    behind these, stubbing them out lets call_start_cpu0 fall
    //    through to esp_startup_start_app.
    "esp_clk_init",
    "esp_perip_clk_init",
    "esp_clk_cpu_freq",
    "core_intr_matrix_clear",
    "esp_efuse_check_errors",
    "esp_dport_access_stall_other_cpu_start",
    "esp_dport_access_stall_other_cpu_end",
    "esp_cpu_unstall",
    "bootloader_flash_update_id",
    "bootloader_init_mem",
    "esp_mspi_pin_init",
    "spi_flash_init_chip_state",
    "esp_chip_info",
    "esp_log_timestamp",
    // SPI-flash HAL — host io-mode config polls a flash-controller status
    // bit the sim does not model. No-op out so spi_flash_init completes.
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
    // Time sources — must return monotonically increasing values, so
    // resolved here and routed to a dedicated thunk in the cli (not
    // the nop_return_zero list).
    "esp_timer_impl_get_counter_reg",
    // APP_CPU initial stack — call_start_cpu1 starts with `entry a1, 32`
    // assuming a valid stack. ESP-IDF puts the boot stack at
    // `port_IntStackTop`. The cli reads this symbol and seeds a1
    // before unhalting cpu_secondary.
    "port_IntStackTop",
    // Xtensa HAL register-window-file spill. Called explicitly by
    // setjmp / exception unwinding / GxEPD2 internals. The "_nw"
    // variant uses a non-standard CALL0 ABI (a0 = return address) and
    // walks the AR file storing each slot's a0..a3 to *(slot.sp - 16)
    // ... -4. If any live slot has sp = 0 (e.g. a freshly-pushed
    // shadow frame the firmware hasn't yet primed), the store
    // dereferences 0 - 16 = 0xfffffff0 and traps. The sim already does
    // shadow-spill on CALL{n}, so the explicit HAL spill is redundant
    // for the panel-render path — stub to a return-zero no-op.
    "xthal_window_spill_nw",
    "xthal_window_spill",
    "vListInsert",
    // app_main — start of patching window for the loopTask xCoreID
    // arg-clobber. We scan ~64 bytes forward looking for the
    // movi.n + s32i.n a14, a1, 0 pattern.
    "app_main",
    // RNG — esp_random does an APB-clock-divisor computation that
    // div0s in the sim. We don't need real entropy; nop_return_zero
    // is fine (callers use it for jitter, never as a primary key).
    "esp_random",
    "esp_fill_random",
    // Newlib stdio output — sketches don't depend on serial output
    // for correctness; stubbing avoids div0s deep in fvwrite.c when
    // the underlying FILE* refers to our zeroed fake reent struct.
    "esp_log_early_timestamp",
    "esp_log_writev",
    "esp_log_impl_lock",
    "esp_log_impl_lock_timeout",
    "esp_log_impl_unlock",
    // Backs `xTaskGetCurrentTaskHandle()` — per-core array of TCB
    // pointers. Address is firmware-dependent; resolving the symbol
    // lets the thunk return a real handle so `vTaskDelete(NULL)`
    // (used by Arduino-ESP32's main_task self-delete) doesn't pass
    // NULL into prvDeleteTLS. ESP-IDF exports the dual-core array as
    // `pxCurrentTCBs` (with trailing s); keep both names.
    "pxCurrentTCB",
    "pxCurrentTCBs",
    "xTaskGetCurrentTaskHandle",
    "esp_log_write",
    "esp_log_buffer_hex_internal",
    "esp_log_buffer_char_internal",
    "esp_log_buffer_hexdump_internal",
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
];

/// Resolve [`ARDUINO_ESP32_SYMBOLS`] from an ELF's regular symbol table.
///
/// Returns only the symbols present in this firmware (address > 0); callers
/// treat an absent entry as "use my fallback" or "no patch needed". Works on
/// `--strip-debug` binaries: only `.symtab` is read, never DWARF. A buffer
/// that is not an ELF yields an empty map.
pub fn arduino_esp32_symbols(elf_bytes: &[u8]) -> HashMap<&'static str, u32> {
    let mut out = HashMap::new();
    let Ok(elf) = goblin::elf::Elf::parse(elf_bytes) else {
        return out;
    };
    for sym in elf.syms.iter() {
        if sym.st_value == 0 {
            continue;
        }
        let Some(name) = elf.strtab.get_at(sym.st_name) else {
            continue;
        };
        if let Some(known) = ARDUINO_ESP32_SYMBOLS.iter().find(|k| **k == name) {
            out.insert(*known, sym.st_value as u32);
        }
    }
    out
}

/// A booted classic-ESP32 Arduino machine, plus the handles callers always want.
pub struct ArduinoElfMachine {
    pub machine: Machine<XtensaLx7>,
    /// Everything the firmware wrote to UART TX.
    ///
    /// Attached AFTER `configure_xtensa_esp32`, never before: the sink walks the
    /// peripherals already on the bus, so attaching it to an empty bus captures
    /// nothing — and an empty sink then reads as "the firmware never printed",
    /// which is a different and much more expensive conclusion.
    pub uart_sink: Arc<Mutex<Vec<u8>>>,
    pub profile: ArduinoEsp32Profile,
}

/// Build and boot a classic-ESP32 machine running an Arduino-ESP32 ELF.
///
/// Ordering here is load-bearing and is the reason this is one function rather
/// than a documented sequence:
///  1. bus + PRO_CPU, then the UART sink (peripherals must exist first);
///  2. external devices from the manifest, then `refresh_peripheral_index`;
///  3. `load_firmware` and seed PC — ELF segment loading clobbers patched bytes,
///     so it must precede the thunk install;
///  4. seed both stacks;
///  5. install the profile LAST, because it patches BREAK bytes into flash.
pub fn build_arduino_elf_machine(
    image: &ProgramImage,
    symbol_addrs: HashMap<&'static str, u32>,
    manifest: &labwired_config::SystemManifest,
    opts: &ArduinoElfBootOpts,
) -> Result<ArduinoElfMachine, String> {
    let mut bus = SystemBus::new();
    let cpu = configure_xtensa_esp32(&mut bus);

    let uart_sink = Arc::new(Mutex::new(Vec::new()));
    bus.attach_uart_tx_sink(uart_sink.clone(), false);

    attach_esp32_external_devices(&mut bus, manifest)
        .map_err(|e| format!("attach external devices from manifest: {e}"))?;
    bus.refresh_peripheral_index();

    let mut machine = Machine::new(cpu, bus);
    if opts.dual_core {
        machine = machine.with_secondary_cpu(XtensaLx7::new_app_cpu());
    }

    machine
        .load_firmware(image)
        .map_err(|e| format!("load firmware: {e}"))?;
    machine.cpu.set_pc(image.entry_point as u32);

    machine.cpu.set_sp(opts.pro_cpu_sp);
    if let Some(cpu1) = machine.cpu_secondary.as_mut() {
        cpu1.set_sp(opts.app_cpu_sp);
    }

    crate::peripherals::esp_xtensa_common::rom_thunks::set_appcpu_up_flags(
        opts.appcpu_up_flag_addrs.clone(),
    );

    let profile =
        install_arduino_esp32_profile(&mut machine, symbol_addrs, image.entry_point as u32)?;

    Ok(ArduinoElfMachine {
        machine,
        uart_sink,
        profile,
    })
}
