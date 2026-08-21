// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! WasmSimulator firmware-quirk installers + runtime-snapshot save/restore.
//! Split out of lib.rs.

use crate::*;
use wasm_bindgen::prelude::*;

#[wasm_bindgen]
impl WasmSimulator {
    /// Arduino-ESP32 boot bootstrap (symbol-table autodiscovery).
    ///
    /// Mirrors the CLI's `arduino-esp32` snapshot-capture profile —
    /// resolves Arduino-ESP32 thunk PCs from the ELF symbol table instead
    /// of hand-curated hardcoded addresses. Works for any GxEPD2-class
    /// sketch (labwired-ereader, future user sketches) without needing
    /// to know its binary layout in advance.
    ///
    /// Caller must pass the same ELF bytes that were loaded via
    /// `load_firmware`. The thunks are installed as flash patches over
    /// the resolved PCs; calling this without the matching ELF is a no-op
    /// (symbols don't resolve → no thunks installed).
    ///
    /// Attaches no peripheral of its own: the panel (model, CS, DC) comes
    /// from the board manifest via `attach_esp32_external_devices` at system
    /// load — see the body below. This method used to hardcode a panel here;
    /// that behaviour is gone, and the manifest is the single source of truth.
    ///
    /// For the record, because the deleted comment had it backwards:
    /// `GxEPD2_290_C90c` is an **SSD1680** controller (0x12 SWRESET, 0x11 data
    /// entry, 0x24/0x26 RAM, 0x22+0x20 update), not UC8151D. UC8151D
    /// (`0x00 PSR` / `0x04 PON` / `0x10 DTM1` / `0x12 DRF` / `0x13 DTM2`) is
    /// what `GxEPD2_290_Z13c` emits. `peripherals::kit::registry::TYPE_ALIASES`
    /// owns that mapping.
    #[wasm_bindgen]
    pub fn install_arduino_esp32_quirks(&mut self, elf_bytes: &[u8]) -> Result<(), JsValue> {
        use labwired_core::peripherals::esp_xtensa_common::rom_thunks;
        // Re-install can happen after a soft re-run without a full construct;
        // always start from a clean session-global slate.
        rom_thunks::reset_esp32_session_state();
        let machine = self
            .machine
            .as_mut()
            .ok_or_else(|| JsValue::from_str("no machine"))?;

        // NO hardcoded peripheral here. The panel (and any other external device)
        // is attached from the board manifest by attach_esp32_external_devices
        // during system load — the single source of truth for peripheral wiring,
        // model, CS and DC pins. This method only installs the firmware-boot
        // thunks + CPU seed below.

        // Seed SP — call_start_cpu0 expects BROM to have placed SP near
        // top of DRAM. We skip BROM.
        machine.cpu.set_sp(0x3FFE_0000);
        // RTC_APB_FREQ_REG (0x3FF4_80B0) now comes pre-seeded with the 40 MHz
        // encoding (0x0050_0050) from the RtcCntl peripheral — no quirk
        // write needed.

        let symbol_addrs = labwired_loader::extract_arduino_esp32_thunks(elf_bytes);

        // Real dual-core when a secondary APP_CPU is attached (the default for
        // the Xtensa path). In that mode the firmware drives the SMP rendezvous
        // itself on a genuine second core, so we must NOT forge the handshake
        // flags or repin loopTask — doing so would race the real core. The
        // single-core fallback (no secondary CPU) keeps the old stub behaviour.
        let dual_core = machine.cpu_secondary.is_some();

        // Dual-core handshake bytes — resolved per firmware. Recorded for
        // the keep-alive in step_with_esp32_aids so the firmware's `.bss`
        // zero-init (which runs after this install but before the spin-wait
        // check in call_start_cpu0) can't wipe them.
        let mut handshake_bytes: Vec<u32> = Vec::new();
        if !dual_core {
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
            if let Some(&addr) = symbol_addrs.get("s_other_cpu_startup_done") {
                let _ = machine.bus.write_u8(addr as u64, 0x01);
                handshake_bytes.push(addr);
            }
        }
        // Re-assert these flags the instant PRO_CPU releases APP_CPU, so
        // newer arduino-esp32 cores whose `start_other_core` spin-waits
        // with a tight cycle-count timeout see APP_CPU "up" without
        // depending on the coarse 10k-cycle keep-alive in
        // step_with_esp32_aids. Models APP_CPU bring-up; see
        // labwired_core rom_thunks::ets_set_appcpu_boot_addr. Empty list in
        // real dual-core mode (the live APP_CPU marks the flags itself).
        labwired_core::peripherals::esp_xtensa_common::rom_thunks::set_appcpu_up_flags(
            handshake_bytes.clone(),
        );

        // loopTask xCoreID patch — repin loopTask from APP_CPU to PRO_CPU.
        // ONLY for the single-core fallback; with a real APP_CPU, loopTask
        // genuinely runs on core 1 (CONFIG_ARDUINO_RUNNING_CORE=1) and must
        // NOT be repinned. Handles both legacy and IDF-5.x app_main layouts.
        if !dual_core {
            if let Some(&app_main_addr) = symbol_addrs.get("app_main") {
                let _ = rom_thunks::repin_loop_task(&mut machine.bus, app_main_addr);
            }
        }

        // `g_ticks_per_us_pro` / `_app` — CPU MHz, normally written by
        // `ets_update_cpu_frequency()` in the ROM bootloader, which this path
        // skips. `esp_clk_apb_freq()` on ESP32-classic is
        // `MIN(g_ticks_per_us_pro, 80) * MHZ`, so leaving them zero reports a
        // 0 Hz APB bus and `esp_timer_impl_update_apb_freq` aborts boot.
        //
        // Without this the browser had the SAME defect the CLI had until
        // core#742: millis() came back with bit 31 set, so every
        // `(int32_t)(millis() - deadline) >= 0` in a sketch compared negative
        // and loop() ran forever doing NOTHING. A bay-occupancy lab showed a
        // blank panel and no serial for as long as you cared to wait.
        for sym in ["g_ticks_per_us_pro", "g_ticks_per_us_app"] {
            if let Some(&addr) = symbol_addrs.get(sym) {
                let _ = machine.bus.write_u32(addr as u64, 240);
            }
        }

        // pxCurrentTCB pointer seed for xTaskGetCurrentTaskHandle thunk.
        if let Some(&addr) = symbol_addrs.get("pxCurrentTCB") {
            rom_thunks::PX_CURRENT_TCB_ADDR.with(|s| s.set(Some(addr)));
        }

        // Build the thunk list — by-symbol lookups, skip when symbol
        // missing (sketch doesn't import that function).
        let mut thunks: Vec<(u32, rom_thunks::RomThunkFn)> = Vec::new();
        let push_named = |list: &mut Vec<(u32, rom_thunks::RomThunkFn)>,
                          sym: &str,
                          f: rom_thunks::RomThunkFn| {
            if let Some(&pc) = symbol_addrs.get(sym) {
                list.push((pc, f));
            }
        };

        // heap_caps_* are NO LONGER thunked. The firmware's real ESP-IDF
        // multi_heap (TLSF) allocator runs on the emulated DRAM. The old
        // bump-allocator thunks were debt; the "real heap walls" symptom
        // (heap_caps_malloc handing out a rodata pointer → vector_desc.next =
        // "lock" → APP_CPU fault in esp_intr_alloc) was actually an APP_CPU
        // dual-core bring-up bug, not an allocator bug — it disappeared once
        // the real second core landed (see new_from_config_xtensa_esp32 +
        // crates/core/tests/e2e_labwired_ereader.rs, which paints identically
        // with the real heap: refresh_gen=1, 1429 ink bytes).

        // No-op stubs for ESP-IDF / Arduino-ESP32 init paths we don't model.
        // NB: esp_timer_init is deliberately absent — it is what programs
        // LACT_CONFIG (enable + divider), and TIMG0 models LACT.
        for sym in &[
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
            "__assert_func",
            "__assert",
            "abort",
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
            // NB: `esp_startup_start_app` is intentionally NOT stubbed —
            // its real impl calls `vTaskStartScheduler()` which never
            // returns. Stubbing makes `start_cpu0` fall into the `j .`
            // safety-loop at its tail and the FreeRTOS scheduler never
            // takes over (loopTask / setup() never run). Required for the
            // labwired-ereader Arduino sketch to actually paint.
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
            "spi_flash_init_chip_state",
            "esp_log_timestamp",
            "esp_log_early_timestamp",
            "esp_log_writev",
            "esp_random",
            "esp_fill_random",
            // The Arduino serial nops that used to sit here are GONE, and so
            // is the fidelity debt they represented. They existed to dodge an
            // Xtensa divide-by-zero in _get_effective_baudrate, whose real
            // cause was an apb_ctrl read-as-ones stub shadowing the SYSCON
            // model at the same base: SYSCLK_CONF read 0xFFFFFFFF, PRE_DIV_CNT
            // came out 1023, and getApbFrequency() reported 78 kHz. Fixed in
            // system/xtensa/esp32.rs by mapping apb_ctrl to its tail; the real
            // HardwareSerial path now runs against the real UART model and
            // demo-labwired-ereader.elf emits its own markers while still
            // painting. See crates/core/tests/esp32_syscon_overlap.rs.
            "_Z14serialEventRunv",
            // vListInsert is NOT nop'd: with the fake FreeRTOS create functions
            // gone, real xQueueCreateMutex / scheduler code runs and depends on
            // genuine list insertion. A nop'd vListInsert leaves those lists
            // uninitialised — it was part of the same fake bundle as the create
            // fakes (see e2e_labwired_ereader.rs) and must go with them.
        ] {
            push_named(&mut thunks, sym, rom_thunks::nop_return_zero);
        }

        // Functions that need real returns / args.
        push_named(
            &mut thunks,
            "esp_ota_get_running_partition",
            rom_thunks::nop_return_fake_ptr,
        );
        // FreeRTOS queue/mutex/event-group create are NO LONGER faked. The
        // firmware's own FreeRTOS runs on the emulated registers + real heap, so
        // xQueueCreateMutex / xQueueGenericCreate / xSemaphoreCreate* /
        // xEventGroupCreate return genuine, fully-initialised handles. The old
        // fake-handle creates were pure debt: an opaque non-NULL pointer left the
        // queue's list structures uninitialised, which forced faking every op
        // built on them (xQueueSemaphoreTake/Send "always succeed") and dropped
        // the SPI payload → blank render. Removing all of it lets the real SPI
        // bus mutex and critical sections run, matching the proven e2e path in
        // crates/core/tests/e2e_labwired_ereader.rs. (xQueueCreateMutexStatic
        // keeps its echo thunk below — callers assert the returned handle equals
        // the static buffer they passed in.)
        // Stub spi_flash_init_lock — the real impl creates a mutex via
        // xSemaphoreCreateMutex and asserts non-NULL; we don't need real
        // flash-op locking in the single-task sim.
        for sym in &[
            "spi_flash_init_lock",
            "spi_flash_op_lock",
            "spi_flash_op_unlock",
        ] {
            push_named(&mut thunks, sym, rom_thunks::nop_return_zero);
        }
        push_named(&mut thunks, "esp_chip_info", rom_thunks::esp_chip_info_stub);
        push_named(
            &mut thunks,
            "__getreent",
            rom_thunks::getreent_dram_fake_ptr,
        );
        // esp_timer_impl_get_counter_reg is NOT thunked: TIMG0 models the LACT
        // timer it reads. The thunk it replaces returned 32 bits through a2 and
        // left a3 — the HIGH word — undefined, and `esp_timer_get_time` computes
        // `(hi << 31) | (lo >> 1)`, putting garbage on bit 31 of every
        // microsecond timestamp. See core#742.
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
        // NO SPI-bus lock shims and NO SPI init fakes. GxEPD2_EPD::init() calls
        // SPI.begin() → the real compiled spiStartBus runs: it creates a real
        // recursive bus mutex via xQueueCreateMutex (real, backed by the real
        // heap), enables the SPI3 clock through DPORT, and configures USER/FIFO.
        // SPIClass::beginTransaction then takes that real mutex. So
        // spi_start_bus_fake, spi_class_begin_transaction, and the
        // xQueueSemaphoreTake / xQueueGenericSend / ulTaskGenericNotifyTake
        // "force pdTRUE" lock shims are all GONE — the bus mutex is a genuine
        // FreeRTOS object and the SPI critical sections run for real, so the byte
        // stream actually reaches the panel (the fakes matched the transaction
        // count but dropped the payload → blank render). Mirrors the proven e2e
        // path in crates/core/tests/e2e_labwired_ereader.rs.

        // No GxEPD2 cmd/data bypass. The real compiled _writeCommand/_writeData
        // run: digitalWrite(DC=GPIO17) → SPI.transfer → spiTransferByteNL writes
        // the SPI3 FIFO/MOSI_DLEN/CMD.USR registers, and Esp32Spi drains the byte
        // to the panel framed by the latched DC GPIO. Bytes reach the panel
        // through real register machinery (verified against the real firmware.elf
        // in tests/e2e_labwired_ereader.rs: 431 SPI3 transactions → refresh).

        // xthal_window_spill_nw — semantic spill via shadow stack. Only the
        // `_nw` leaf is thunked; the `xthal_window_spill` wrapper is a thin
        // CALL{n}-entered PS-save shell that must run its real
        // `entry / call0 _nw / retw` natively — thunking it returns via a0
        // (the caller's return addr, since the wrapper's clobbered ENTRY
        // never set up a0), faulting in the first-task dispatch.
        push_named(
            &mut thunks,
            "xthal_window_spill_nw",
            rom_thunks::xthal_window_spill_thunk,
        );

        // Real-silicon noreturn — abort_halt prints diagnostics and
        // halts the CPU rather than returning, to avoid tight
        // assert→return loops.
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

        for (pc, f) in thunks {
            machine
                .bus
                .install_flash_thunk(pc, f)
                .map_err(|e| JsValue::from_str(&format!("install thunk @{pc:#x}: {e}")))?;
        }

        // Only arm the single-core IPI bridge / handshake keep-alive when there
        // is no real APP_CPU. With real dual-core, step_with_esp32_aids delegates
        // straight to Machine::step (which delivers the DPORT IPI), so the bridge
        // would be dead weight.
        if !dual_core {
            self.esp32_ipi = Some(Esp32IpiBridge {
                handshake_bytes,
                ..Esp32IpiBridge::default()
            });
        }
        Ok(())
    }

    /// Apply a binary `MachineRuntimeSnapshot` (LWRS-framed bincode blob,
    /// produced by `labwired-cli snapshot capture` or `Machine::take_runtime_snapshot`)
    /// to the currently-loaded machine. Bypasses the cold boot — the firmware
    /// resumes mid-flight from the captured CPU + peripheral state.
    ///
    /// Must be called after firmware has been loaded onto the same system
    /// manifest (peripheral names + CPU arch must match the snapshot). On
    /// mismatch the call returns an error and the machine state is left
    /// partially overwritten — callers should treat that as a hard reset.
    #[wasm_bindgen]
    pub fn apply_runtime_snapshot(&mut self, bytes: &[u8]) -> Result<(), JsValue> {
        let machine = self
            .machine
            .as_mut()
            .ok_or_else(|| JsValue::from_str("no machine"))?;
        let snap = labwired_core::runtime_snapshot::MachineRuntimeSnapshot::from_bytes(bytes)
            .map_err(|e| JsValue::from_str(&format!("snapshot decode: {e}")))?;
        machine
            .apply_runtime_snapshot(&snap)
            .map_err(|e| JsValue::from_str(&format!("snapshot apply: {e}")))?;
        Ok(())
    }

    /// Capture the current machine state as a binary `MachineRuntimeSnapshot`
    /// (LWRS-framed bincode blob). Mirror of `apply_runtime_snapshot` —
    /// returned bytes can be fed back to `apply_runtime_snapshot` on a fresh
    /// `WasmSimulator` with the same firmware + bus topology.
    #[wasm_bindgen]
    pub fn take_runtime_snapshot(&self) -> Result<Vec<u8>, JsValue> {
        let machine = self
            .machine
            .as_ref()
            .ok_or_else(|| JsValue::from_str("no machine"))?;
        // A CPU without a runtime_snapshot impl answers `None`. This used to
        // reach `unimplemented!()`, and a Rust panic in wasm is an
        // `unreachable` TRAP: no destructors run, so wasm-bindgen's borrow
        // guard leaks and the simulator is borrowed forever. Every later
        // call — `step_batch` above all — then dies with "recursive use of
        // an object", which is what froze every Cortex-M lab a few seconds
        // in. An Err returns normally and leaves the machine healthy.
        machine
            .take_runtime_snapshot()
            .map(|snap| snap.to_bytes())
            .ok_or_else(|| JsValue::from_str("runtime snapshot is not supported for this CPU"))
    }
}
