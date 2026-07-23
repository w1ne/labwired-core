// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

use addr2line::gimli::Reader;
use anyhow::{anyhow, Context, Result};
use goblin::elf::program_header::PT_LOAD;
use goblin::elf::Elf;
use labwired_core::memory::ProgramImage;
use object::ObjectSymbol;
use std::collections::HashMap;
use std::fs;
use std::path::Path;
use std::sync::Arc;
use tracing::{debug, info, warn};

pub fn load_elf(path: &Path) -> Result<ProgramImage> {
    let buffer = fs::read(path).with_context(|| format!("Failed to read ELF file: {:?}", path))?;
    load_elf_bytes(&buffer)
}

/// Resolve a single function symbol's address from an ELF binary.
///
/// Used to auto-discover Arduino-ESP32 thunk PCs (heap_caps_init,
/// esp_timer_init, esp_ota_get_running_partition, ...) without baking
/// per-firmware constants into either the CLI snapshot-capture path
/// or the WASM `install_esp32_arduino_quirks` bootstrap. Returns `None`
/// when the ELF is stripped of symbols (in which case the caller falls
/// back to a hardcoded profile).
///
/// Skips the gimli/DWARF dance that [`SymbolProvider::new`] does — we
/// only need name→address resolution from the regular symbol table,
/// which works on `--strip-debug`-stripped binaries too.
pub fn resolve_symbol_in_elf(buffer: &[u8], name: &str) -> Option<u32> {
    use object::{Object, ObjectSymbol};
    let object = object::File::parse(buffer).ok()?;
    for sym in object.symbols() {
        if let Ok(n) = sym.name() {
            if n == name && sym.address() > 0 {
                return Some(sym.address() as u32);
            }
        }
    }
    None
}

/// Extract every Arduino-ESP32 / ESP-IDF / Arduino-core symbol the LabWired
/// sim cares about for an Arduino-ESP32 firmware. Includes:
///   * flash-thunk targets (heap_caps_*, esp_timer_init, locks, …),
///   * dual-core handshake bytes (`s_cpu_up`, `s_cpu_inited`, `s_system_inited`,
///     `s_other_cpu_startup_done`) that the single-CPU sim has to pre-write,
///   * optional bootstrap markers (`loopTask`, `app_main`).
///
/// Returns the addresses present in this firmware; the caller treats absent
/// entries as "use my hardcoded fallback" or "no patch needed." Works on
/// `--strip-debug`-stripped binaries — only requires the regular symbol
/// table, not DWARF.
pub fn extract_arduino_esp32_thunks(buffer: &[u8]) -> HashMap<&'static str, u32> {
    use object::{Object, ObjectSymbol};
    const KNOWN: &[&str] = &[
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
    let mut out = HashMap::new();
    let Ok(object) = object::File::parse(buffer) else {
        return out;
    };
    for sym in object.symbols() {
        if let Ok(name) = sym.name() {
            if sym.address() > 0 {
                for k in KNOWN {
                    if *k == name {
                        out.insert(*k, sym.address() as u32);
                    }
                }
            }
        }
    }
    out
}

pub fn load_elf_bytes(buffer: &[u8]) -> Result<ProgramImage> {
    let elf = Elf::parse(buffer).context("Failed to parse ELF binary")?;

    info!("ELF Entry Point: {:#x}", elf.entry);

    let arch = match elf.header.e_machine {
        goblin::elf::header::EM_ARM => labwired_core::Arch::Arm,
        goblin::elf::header::EM_RISCV => labwired_core::Arch::RiscV,
        94 => labwired_core::Arch::XtensaLx7, // EM_XTENSA = 94
        _ => {
            warn!("Unknown ELF machine type: {}", elf.header.e_machine);
            labwired_core::Arch::Unknown
        }
    };

    let mut program_image = ProgramImage::new(elf.entry, arch);

    for ph in elf.program_headers {
        if ph.p_type == PT_LOAD {
            // We only care about loadable segments
            let start_addr = ph.p_paddr; // Physical address (LMA) is usually what we want for flash programming
            let size = ph.p_filesz as usize;
            let offset = ph.p_offset as usize;

            if size == 0 {
                continue;
            }

            debug!(
                "Found Loadable Segment: Addr={:#x}, Size={} bytes, Offset={:#x}",
                start_addr, size, offset
            );

            if offset + size > buffer.len() {
                return Err(anyhow!("Segment out of bounds in ELF file"));
            }

            let segment_data = buffer[offset..offset + size].to_vec();
            program_image.add_segment(start_addr, segment_data);
        }
    }

    if program_image.segments.is_empty() {
        warn!("No loadable segments found in ELF file");
    }

    Ok(program_image)
}

pub struct SourceLocation {
    pub file: String,
    pub line: Option<u32>,
    pub function: Option<String>,
}

/// One row of the DWARF line-number program: the instruction address and the
/// source position it maps to. Unlike the reverse `line_map`, these are NOT
/// deduplicated — every row is retained, so the set of `is_stmt` rows is the
/// statement universe for coverage.
#[derive(Debug, Clone)]
pub struct StmtRow {
    pub addr: u64,
    pub file: String,
    pub line: u32,
    pub is_stmt: bool,
}

#[derive(Debug, Clone)]
pub enum DwarfLocation {
    Register(u16),
    Address(u64),
    FrameRelative(i64),
    Other(String),
}

#[derive(Debug, Clone)]
pub struct LocalVariable {
    pub name: String,
    pub location: DwarfLocation,
}

pub struct SymbolProvider {
    #[allow(dead_code)]
    data: Arc<Vec<u8>>,
    dwarf: addr2line::gimli::Dwarf<
        addr2line::gimli::EndianReader<addr2line::gimli::RunTimeEndian, Arc<[u8]>>,
    >,
    context: addr2line::Context<
        addr2line::gimli::EndianReader<addr2line::gimli::RunTimeEndian, Arc<[u8]>>,
    >,
    // Map of (file_name, line) -> address
    line_map: HashMap<(String, u32), u64>,
    // Full line-program rows (not deduped) — the statement universe for coverage
    stmt_rows: Vec<StmtRow>,
    // Map of symbol_name -> address
    symbol_map: HashMap<String, u64>,
    // Test-only locals: PC -> list of locals
    test_locals: HashMap<u64, Vec<LocalVariable>>,
}

impl SymbolProvider {
    pub fn new(path: &Path) -> Result<Self> {
        use gimli::Reader;
        use object::Object;
        let data = fs::read(path)
            .with_context(|| format!("Failed to read ELF for symbols: {:?}", path))?;
        let data = Arc::new(data);

        let slice: &'static [u8] = unsafe { std::mem::transmute(&data[..]) };

        let object = object::File::parse(slice).context("Failed to parse ELF for symbols")?;

        let mut line_map = std::collections::HashMap::new();
        let mut stmt_rows: Vec<StmtRow> = Vec::new();

        // Build line map using gimli for reverse lookup
        let load_section = |id: gimli::SectionId| -> std::result::Result<
            addr2line::gimli::EndianReader<gimli::RunTimeEndian, Arc<[u8]>>,
            gimli::Error,
        > {
            use object::ObjectSection;
            let data = object
                .section_by_name(id.name())
                .and_then(|s| s.uncompressed_data().ok())
                .map(|d| Arc::from(&d[..]))
                .unwrap_or_else(|| Arc::from(&[][..]));
            Ok(gimli::EndianReader::new(data, gimli::RunTimeEndian::Little))
        };

        let dwarf = gimli::Dwarf::load(&load_section).context("Failed to load DWARF")?;

        let mut iter = dwarf.units();
        while let Ok(Some(header)) = iter.next() {
            let unit = dwarf.unit(header).ok();
            if let Some(unit) = unit {
                if let Some(ref line_program) = unit.line_program {
                    let mut rows = line_program.clone().rows();
                    while let Ok(Some((_, row))) = rows.next_row() {
                        if row.end_sequence() {
                            continue;
                        }
                        let file_idx = row.file_index();
                        if let Some(file) = line_program.header().file(file_idx) {
                            let file_name = dwarf
                                .attr_string(&unit, file.path_name())
                                .ok()
                                .and_then(|s| {
                                    let s2 = s.to_string_lossy().ok()?;
                                    Some(s2.into_owned())
                                });

                            if let (Some(f), Some(line)) = (file_name, row.line()) {
                                let line_u32 = line.get() as u32;
                                // Retain every row for the statement universe...
                                stmt_rows.push(StmtRow {
                                    addr: row.address(),
                                    file: f.clone(),
                                    line: line_u32,
                                    is_stmt: row.is_stmt(),
                                });
                                // ...and the first address per file:line for reverse lookup.
                                line_map.entry((f, line_u32)).or_insert(row.address());
                            }
                        }
                    }
                }
            }
        }

        let mut symbol_map = std::collections::HashMap::new();
        for sym in object.symbols() {
            if let Ok(name) = sym.name() {
                if sym.address() > 0 {
                    symbol_map.insert(name.to_string(), sym.address());
                }
            }
        }

        let dwarf_for_context =
            gimli::Dwarf::load(&load_section).context("Failed to load DWARF for context")?;
        let context = addr2line::Context::from_dwarf(dwarf_for_context)
            .context("Failed to create context from dwarf")?;

        Ok(Self {
            data,
            dwarf,
            context,
            line_map,
            stmt_rows,
            symbol_map,
            test_locals: HashMap::new(),
        })
    }

    /// Full DWARF line-program rows (not deduplicated). The set of rows with
    /// `is_stmt` set is the statement universe: a statement is covered when an
    /// instruction at its address was executed.
    pub fn statement_rows(&self) -> &[StmtRow] {
        &self.stmt_rows
    }

    pub fn lookup(&self, addr: u64) -> Option<SourceLocation> {
        let mut frames = match self.context.find_frames(addr) {
            addr2line::LookupResult::Output(Ok(frames)) => frames,
            _ => return None,
        };

        if let Ok(Some(frame)) = frames.next() {
            let file = frame
                .location
                .as_ref()
                .and_then(|l| l.file)
                .map(|f: &str| f.to_string());
            let line = frame.location.as_ref().and_then(|l| l.line);
            let function = frame
                .function
                .as_ref()
                .and_then(|f| f.demangle().ok())
                .map(|s: std::borrow::Cow<str>| s.into_owned());

            if let Some(f) = file {
                return Some(SourceLocation {
                    file: f,
                    line,
                    function,
                });
            }
        }
        None
    }

    pub fn location_to_pc(&self, file_path: &str, line: u32) -> Option<u64> {
        self.location_to_pc_nearest(file_path, line)
            .map(|(addr, _line)| addr)
    }

    pub fn location_to_pc_nearest(&self, file_path: &str, line: u32) -> Option<(u64, u32)> {
        let requested_file = std::path::Path::new(file_path).file_name()?.to_str()?;
        let requested_norm = normalize_path_for_match(file_path);

        // Collect candidates with same basename and a path specificity score.
        let mut candidates: Vec<(u32, u64, usize)> = Vec::new();
        for ((candidate_path, candidate_line), addr) in &self.line_map {
            let Some(candidate_file) = std::path::Path::new(candidate_path)
                .file_name()
                .and_then(|n| n.to_str())
            else {
                continue;
            };
            if candidate_file != requested_file {
                continue;
            }

            let score =
                path_match_score(&requested_norm, &normalize_path_for_match(candidate_path));
            candidates.push((*candidate_line, *addr, score));
        }
        if candidates.is_empty() {
            return None;
        }

        // Prefer the most specific path match first.
        let best_score = candidates
            .iter()
            .map(|(_, _, score)| *score)
            .max()
            .unwrap_or(0);
        candidates.retain(|(_, _, score)| *score == best_score);

        // Prefer exact line, then nearest following line, then nearest previous line.
        if let Some((l, addr, _)) = candidates.iter().find(|(l, _, _)| *l == line) {
            return Some((*addr, *l));
        }

        let mut after: Vec<(u32, u64)> = candidates
            .iter()
            .filter(|(l, _, _)| *l > line)
            .map(|(l, addr, _)| (*l, *addr))
            .collect();
        after.sort_by_key(|(l, _)| *l);
        if let Some((l, addr)) = after.first() {
            return Some((*addr, *l));
        }

        let mut before: Vec<(u32, u64)> = candidates
            .iter()
            .filter(|(l, _, _)| *l < line)
            .map(|(l, addr, _)| (*l, *addr))
            .collect();
        before.sort_by_key(|(l, _)| *l);
        before.last().map(|(l, addr)| (*addr, *l))
    }

    pub fn resolve_symbol(&self, name: &str) -> Option<u64> {
        self.symbol_map.get(name).copied()
    }

    pub fn find_locals(&self, pc: u64) -> Vec<LocalVariable> {
        let mut locals = Vec::new();

        // Include test-only locals for PC 0 (default) or the specific PC
        if let Some(tl) = self.test_locals.get(&0) {
            locals.extend(tl.clone());
        }
        if pc != 0 {
            if let Some(tl) = self.test_locals.get(&pc) {
                locals.extend(tl.clone());
            }
        }

        let mut units = self.dwarf.units();

        while let Ok(Some(header)) = units.next() {
            let unit = match self.dwarf.unit(header) {
                Ok(u) => u,
                Err(_) => continue,
            };

            let mut in_subprogram = false;
            let mut subprogram_depth = 0;
            let mut entries = unit.entries();

            while let Ok(Some((depth, entry))) = entries.next_dfs() {
                if !in_subprogram {
                    if entry.tag() == addr2line::gimli::DW_TAG_subprogram {
                        let mut low_pc = None;
                        let mut high_pc = None;

                        if let Some(addr2line::gimli::AttributeValue::Addr(addr)) = entry
                            .attr_value(addr2line::gimli::DW_AT_low_pc)
                            .ok()
                            .flatten()
                        {
                            low_pc = Some(addr);
                        }

                        if let Some(attr) = entry
                            .attr_value(addr2line::gimli::DW_AT_high_pc)
                            .ok()
                            .flatten()
                        {
                            match attr {
                                addr2line::gimli::AttributeValue::Addr(addr) => {
                                    high_pc = Some(addr)
                                }
                                addr2line::gimli::AttributeValue::Udata(size) => {
                                    high_pc = low_pc.map(|l| l + size)
                                }
                                _ => {}
                            }
                        }

                        if let (Some(low), Some(high)) = (low_pc, high_pc) {
                            if pc >= low && pc < high {
                                in_subprogram = true;
                                subprogram_depth = depth;
                            }
                        }
                    }
                } else {
                    if depth <= subprogram_depth {
                        in_subprogram = false;
                        continue;
                    }

                    if entry.tag() == addr2line::gimli::DW_TAG_variable
                        || entry.tag() == addr2line::gimli::DW_TAG_formal_parameter
                    {
                        let name = entry
                            .attr_value(addr2line::gimli::DW_AT_name)
                            .ok()
                            .flatten()
                            .and_then(|attr| {
                                let s = self.dwarf.attr_string(&unit, attr).ok()?;
                                s.to_string_lossy().ok().map(|c| c.into_owned())
                            });

                        if let (Some(n), Some(addr2line::gimli::AttributeValue::Exprloc(expr))) = (
                            name,
                            entry
                                .attr_value(addr2line::gimli::DW_AT_location)
                                .ok()
                                .flatten(),
                        ) {
                            let mut ops = expr.operations(unit.encoding());
                            if let Ok(Some(op)) = ops.next() {
                                match op {
                                    addr2line::gimli::Operation::Register { register } => {
                                        locals.push(LocalVariable {
                                            name: n,
                                            location: DwarfLocation::Register(register.0),
                                        });
                                    }
                                    addr2line::gimli::Operation::FrameOffset { offset } => {
                                        locals.push(LocalVariable {
                                            name: n,
                                            location: DwarfLocation::FrameRelative(offset),
                                        });
                                    }
                                    _ => {
                                        locals.push(LocalVariable {
                                            name: n,
                                            location: DwarfLocation::Other(format!("{:?}", op)),
                                        });
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
        locals
    }

    /// Create an empty SymbolProvider for testing
    pub fn new_empty() -> Self {
        let data = Arc::new(Vec::new());

        let load_section = |_id: gimli::SectionId| -> std::result::Result<
            addr2line::gimli::EndianReader<gimli::RunTimeEndian, Arc<[u8]>>,
            gimli::Error,
        > {
            let data = Arc::from(&[][..]);
            Ok(gimli::EndianReader::new(data, gimli::RunTimeEndian::Little))
        };

        let dwarf = gimli::Dwarf::load(&load_section).unwrap();
        let dwarf_for_context = gimli::Dwarf::load(&load_section).unwrap();
        let context = addr2line::Context::from_dwarf(dwarf_for_context).unwrap();

        Self {
            data,
            dwarf,
            context,
            line_map: HashMap::new(),
            stmt_rows: Vec::new(),
            symbol_map: HashMap::new(),
            test_locals: HashMap::new(),
        }
    }

    /// Add a mock local variable for testing
    pub fn add_test_local(&mut self, name: &str, location: DwarfLocation) {
        // We use PC 0 as the default for test locals if not specified
        self.test_locals.entry(0).or_default().push(LocalVariable {
            name: name.to_string(),
            location,
        });
    }
}

fn normalize_path_for_match(path: &str) -> String {
    path.replace('\\', "/")
}

fn path_match_score(requested_norm: &str, candidate_norm: &str) -> usize {
    if requested_norm == candidate_norm {
        return 10_000;
    }

    // Absolute IDE paths commonly end with relative DWARF paths.
    if requested_norm.ends_with(candidate_norm) {
        return 1_000 + candidate_norm.len();
    }
    if candidate_norm.ends_with(requested_norm) {
        return 900 + requested_norm.len();
    }

    // Basename-only match fallback (weak, but better than no breakpoint).
    100
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_location_to_pc() {
        // This test requires the firmware to be built with debug symbols.
        // Build it with: cargo build -p firmware-ci-fixture --target thumbv7m-none-eabi
        // (see core-ci.yml "Build test firmware fixture" step).
        let elf_path =
            std::path::PathBuf::from("../../target/thumbv7m-none-eabi/debug/firmware-ci-fixture");
        if !elf_path.exists() {
            // The fast PR gate runs `cargo test --workspace --lib` WITHOUT
            // cross-building firmware, so this fixture is absent there. Skip
            // gracefully rather than fail; the post-merge full suite builds
            // firmware-ci-fixture first and exercises the real assertions.
            eprintln!(
                "skipping test_location_to_pc: fixture not built \
                 (cargo build -p firmware-ci-fixture --target thumbv7m-none-eabi)"
            );
            return;
        }

        let provider = SymbolProvider::new(&elf_path).expect("Failed to create SymbolProvider");

        // Try to resolve a location in main.rs
        // Note: Line 14 is 'fn main() -> ! {'
        let pc = provider.location_to_pc("main.rs", 26);
        assert!(pc.is_some(), "Should resolve main.rs:26 to a PC");

        let addr = pc.unwrap();
        assert!(addr > 0, "Resolved address should be valid");

        // Reverse lookup
        let loc = provider
            .lookup(addr)
            .expect("Lookup failed for resolved PC");

        // Debug info might map to main.rs or lib core/std if inlined, but line 26 is specific enough
        println!("Resolved file: {}", loc.file);
        assert!(
            loc.file.ends_with("main.rs"),
            "Resolved file '{}' does not end with 'main.rs'",
            loc.file
        );
        assert_eq!(loc.line, Some(26));
    }

    #[test]
    fn test_statement_rows_full_not_deduped() {
        let elf_path =
            std::path::PathBuf::from("../../target/thumbv7m-none-eabi/debug/firmware-ci-fixture");
        if !elf_path.exists() {
            eprintln!(
                "skipping test_statement_rows_full_not_deduped: fixture not built \
                 (cargo build -p firmware-ci-fixture --target thumbv7m-none-eabi)"
            );
            return;
        }

        let provider = SymbolProvider::new(&elf_path).expect("Failed to create SymbolProvider");
        let rows = provider.statement_rows();

        assert!(!rows.is_empty(), "expected DWARF line-program rows");
        assert!(
            rows.iter().any(|r| r.is_stmt),
            "expected at least one is_stmt row"
        );

        // The full row set must not be deduplicated the way the reverse line_map
        // is: there are more rows than distinct (file,line) keys whenever any
        // line spans multiple address ranges (loops, inlining, -O).
        let distinct_lines: std::collections::HashSet<(&str, u32)> =
            rows.iter().map(|r| (r.file.as_str(), r.line)).collect();
        assert!(
            rows.len() >= distinct_lines.len(),
            "row count must be at least the distinct-line count"
        );

        // main.rs line 26 is known-present (see test_location_to_pc).
        assert!(
            rows.iter()
                .any(|r| r.file.ends_with("main.rs") && r.line == 26),
            "expected a statement row for main.rs:26"
        );
    }

    #[test]
    fn test_location_to_pc_nearest_prefers_same_file_and_next_line() {
        let mut provider = SymbolProvider::new_empty();
        provider.line_map.insert(
            ("crates/firmware-h563-io-demo/src/main.rs".to_string(), 117),
            0x0800_00A8,
        );
        provider.line_map.insert(
            ("crates/firmware-h563-io-demo/src/main.rs".to_string(), 125),
            0x0800_00FC,
        );
        provider
            .line_map
            .insert(("main.rs".to_string(), 117), 0xDEAD_BEEF);

        // The lookup uses an absolute path that includes the workspace
        // ancestor — we only care that the suffix `crates/...` matches
        // the registered key. Use an arbitrary absolute prefix so this
        // test doesn't bake in any specific developer's home directory.
        let resolved = provider.location_to_pc_nearest(
            "/workspace/labwired/core/crates/firmware-h563-io-demo/src/main.rs",
            120,
        );
        assert_eq!(resolved, Some((0x0800_00FC, 125)));
    }
}
