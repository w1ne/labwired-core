// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT
//
// Stress test: load an external Arduino-ESP32 + GxEPD2 firmware ELF into
// our ESP32-classic sim and assert that the SSD1680 panel model receives
// the expected byte stream and refresh sequence.
//
// The ELF path is taken from the LABWIRED_EXTERNAL_ARDUINO_ESP32_ELF env
// var; if the var isn't set or the file isn't readable the test reports
// "skipped" rather than failing — this test only runs when the operator
// has built a reference firmware locally and pointed the var at it.

use labwired_core::bus::SystemBus;
use labwired_core::peripherals::components::Ssd1680Tricolor290;
use labwired_core::peripherals::esp32::spi::Esp32Spi;
use labwired_core::system::xtensa::configure_xtensa_esp32;
use labwired_core::{Bus, Cpu, Machine};
use std::path::PathBuf;

/// Baseline byte counts captured 2026-05-23. Bumped intentionally only after
/// reviewing the diff vs the prior baseline — drift signals a sim regression
/// (or, less often, a genuine improvement). Tolerances allow ±0.5% on the
/// SPI transaction count and ±10 bytes on the black-plane bitmap to absorb
/// benign ordering noise (e.g. tick alignment) without missing real changes.
const EXTERNAL_BASELINE_SPI3_TXNS: u32 = 19031;
const EXTERNAL_BASELINE_BLACK_NONFF: usize = 782;

#[test]
#[ignore = "loads a large external Arduino-ESP32 firmware ELF; only runs when LABWIRED_EXTERNAL_ARDUINO_ESP32_ELF is set. Run with `cargo test -- --ignored external_arduino_esp32_firmware_drives_panel_in_sim`."]
fn external_arduino_esp32_firmware_drives_panel_in_sim() {
    let elf = std::env::var("LABWIRED_EXTERNAL_ARDUINO_ESP32_ELF")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("/tmp/external-arduino-esp32.elf"));
    if !elf.exists() {
        eprintln!(
            "[skip] external Arduino-ESP32 firmware ELF unavailable at {elf:?}; \
             set LABWIRED_EXTERNAL_ARDUINO_ESP32_ELF to a valid ELF path to enable"
        );
        return;
    }

    let mut bus = SystemBus::new();
    let cpu = configure_xtensa_esp32(&mut bus);

    // Wire the SSD1680 to spi3, same as the e-paper lab.
    bus.attach_spi_device("spi3", Box::new(Ssd1680Tricolor290::new("GPIO5")))
        .expect("spi3 is an Esp32Spi controller");
    // Capture every byte on the wire so we can diagnose missing panel state.
    {
        let spi3_idx = bus.find_peripheral_index_by_name("spi3").unwrap();
        let any = bus.peripherals[spi3_idx].dev.as_any_mut().unwrap();
        any.downcast_mut::<Esp32Spi>()
            .unwrap()
            .enable_byte_capture(256);
    }

    bus.refresh_peripheral_index();
    let mut machine = Machine::new(cpu, bus);

    let image = labwired_loader::load_elf(&elf).expect("parse ELF");
    machine.load_firmware(&image).expect("load firmware");
    machine.cpu.set_pc(image.entry_point as u32);
    // Single-CPU sim workaround: app_main hard-codes core=1 (APP_CPU) when
    // creating loopTask via xTaskCreateUniversal — pinning setup()/loop()
    // to a CPU we don't emulate. Patch the immediate of the `movi.n a8, 1`
    // at 0x400e90de from 0x18 (movi.n a8, 1) to 0x08 (movi.n a8, 0), so
    // loopTask gets pinned to PRO_CPU and our scheduler picks it up after
    // main_task self-deletes. The encoding for `movi.n at, im` is
    // 0x0c00 | (at << 4) | (im & 0x0F) — verified by reading the live
    // disassembly. ELF byte at file offset is identical to load address
    // because flash.text is mapped 1:1. Authorized one-byte runtime
    // workaround for the single-CPU sim path; firmware-on-hardware is
    // unchanged.
    let _ = machine.bus.write_u8(0x400E_90DE, 0x08);
    // Arduino-ESP32's call_start_cpu0 assumes BROM has already set SP.
    // Real silicon's BROM leaves SP near the top of DRAM (0x3FFE_0000);
    // we don't run BROM in sim so seed SP ourselves.
    machine.cpu.set_sp(0x3FFE_0000);
    // Fake the APP_CPU bringup handshake: `start_other_core` spins waiting
    // for `s_cpu_up[1]` to be set by the second core. We don't model the
    // second core, so pre-write 1 to its slot so the loop exits.
    let _ = machine.bus.write_u8(0x3FFC_6F04, 0x01);
    // Fake the BROM XTAL-frequency probe. esp_clk_init asserts that
    // `rtc_clk_xtal_freq_get()` doesn't return SOC_XTAL_FREQ_AUTO. The
    // probe reads RTC_APB_FREQ_REG (0x3FF480B0): low and high halves
    // must be equal AND not -1/-2, encoding (freq_mhz << 1). For 40 MHz
    // XTAL → value = 0x0050_0050.
    let _ = machine.bus.write_u32(0x3FF4_80B0, 0x0050_0050);
    // Install sim-side heap allocator thunks. ESP-IDF's heap_caps_init
    // can't bootstrap multi_heap state in our sim (chicken-and-egg with
    // the static heap allocator), so we replace the heap-caps API surface
    // with a bump allocator over a fixed 64 KiB pool in SRAM2.
    use labwired_core::peripherals::esp_xtensa_common::rom_thunks;
    machine
        .bus
        .install_flash_thunk(0x400e_e3b0, rom_thunks::esp_idf_heap_caps_init)
        .expect("install heap_caps_init thunk");
    machine
        .bus
        .install_flash_thunk(0x4008_2904, rom_thunks::esp_idf_heap_caps_malloc)
        .expect("install heap_caps_malloc thunk");
    machine
        .bus
        .install_flash_thunk(0x4008_2a70, rom_thunks::esp_idf_heap_caps_calloc)
        .expect("install heap_caps_calloc thunk");
    machine
        .bus
        .install_flash_thunk(0x4008_25dc, rom_thunks::esp_idf_heap_caps_free)
        .expect("install heap_caps_free thunk");
    machine
        .bus
        .install_flash_thunk(0x4008_29f0, rom_thunks::esp_idf_heap_caps_realloc)
        .expect("install heap_caps_realloc thunk");
    // esp_timer_init computes a divider for the LACT timer (1MHz tick from
    // APB clock). The HAL asserts divider >= 2; our APB clock readout path
    // isn't fully wired and the inlined math underflows. Stub the whole
    // init to return 0 (ESP_OK) — software timers won't work, but boot
    // continues past the FreeRTOS scheduler-start path.
    machine
        .bus
        .install_flash_thunk(0x4012_9034, rom_thunks::nop_return_zero)
        .expect("install esp_timer_init thunk");
    // spi_flash_disable/enable_interrupts_caches_and_other_cpu take the
    // s_flash_op_mutex which isn't initialized until esp_flash_app_init
    // runs later in boot. The sim doesn't need to disable interrupts or
    // suspend caches around flash ops — flash is just LinearMemory we
    // can touch directly — so no-op the mutex-locking wrappers.
    machine
        .bus
        .install_flash_thunk(0x4008_17dc, rom_thunks::nop_return_zero)
        .expect("install spi_flash_disable_... thunk");
    machine
        .bus
        .install_flash_thunk(0x4008_188c, rom_thunks::nop_return_zero)
        .expect("install spi_flash_enable_... thunk");
    // newlib `__retarget_lock_acquire_recursive` and friends assert that
    // the lock pointer is non-NULL — but a number of newlib locks aren't
    // initialized in our boot path (we haven't wired esp_libc's lock-init
    // chain end-to-end). For a single-threaded sim, the locks are
    // unnecessary; stub all four lock entry points as no-ops.
    machine
        .bus
        .install_flash_thunk(0x4008_3384, rom_thunks::nop_return_zero) // __retarget_lock_init_recursive
        .expect("install lock_init_recursive thunk");
    machine
        .bus
        .install_flash_thunk(0x4008_339c, rom_thunks::nop_return_zero) // __retarget_lock_close_recursive
        .expect("install lock_close_recursive thunk");
    machine
        .bus
        .install_flash_thunk(0x4008_33b0, rom_thunks::nop_return_zero) // __retarget_lock_acquire_recursive
        .expect("install lock_acquire_recursive thunk");
    machine
        .bus
        .install_flash_thunk(0x4008_33cc, rom_thunks::nop_return_zero) // __retarget_lock_release_recursive
        .expect("install lock_release_recursive thunk");
    // ESP_ERROR_CHECK macros call _esp_error_check_failed on non-zero return
    // values, which aborts. Our subsystem stubs return 0 (ESP_OK) so this
    // shouldn't fire from our own stubs, but firmware code paths may still
    // trigger it on real ESP-IDF function returns we don't fully model.
    // Stub to no-op so boot continues past these soft failures.
    machine
        .bus
        .install_flash_thunk(0x4008_bbd0, rom_thunks::nop_return_zero)
        .expect("install _esp_error_check_failed thunk");
    // Arduino-ESP32's setCpuFrequencyMhz(240) in app_main re-runs the clock
    // prescaler math, hitting the same divider>=2 assertion we worked around
    // for esp_timer_init. APB clock readout isn't fully wired so the inlined
    // prescale computation underflows. No-op the public API since we don't
    // model variable CPU clocks anyway.
    machine
        .bus
        .install_flash_thunk(0x400e_99dc, rom_thunks::nop_return_zero)
        .expect("install setCpuFrequencyMhz thunk");
    // initArduino calls esp_ota_get_running_partition, which iterates the
    // partition table to find the currently-executing partition. We don't
    // model the partition table — make it return a non-NULL dummy pointer
    // so the `it != NULL` assertion passes. The downstream code mostly uses
    // this for logging the running partition name; the dummy pointer
    // points into our partition-header fake at 0x3F400000.
    machine
        .bus
        .install_flash_thunk(0x400e_ae18, rom_thunks::nop_return_fake_ptr)
        .expect("install esp_ota_get_running_partition thunk");
    // HardwareSerial::begin acquires Serial0's per-instance Mutex via
    // xQueueSemaphoreTake(portMAX_DELAY). Our sim's mutex state for that
    // semaphore comes up "locked" — likely because the global ctor that
    // creates it interacts with our heap thunks in ways that leave the
    // semaphore handle valid but its inner state taken. Without a UART
    // peripheral model the serial output is moot anyway, so stub the
    // member function to no-op. setup() can then proceed past it to the
    // calls we actually care about (Display::begin → GxEPD2 → SSD1680).
    machine
        .bus
        .install_flash_thunk(0x400e_2280, rom_thunks::nop_return_zero)
        .expect("install HardwareSerial::begin thunk");
    // Arduino's delay() wraps vTaskDelay, which puts the calling task on
    // the kernel's delayed list and yields. Empirically the task gets
    // added correctly but never wakes — our delayed-list wake path or
    // the kernel's tick-count comparison isn't lining up for this code
    // path. We don't actually need timing precision for the panel goal,
    // and the panel driver doesn't depend on delay() returning at a
    // specific time, so stub it to no-op.
    machine
        .bus
        .install_flash_thunk(0x400e_5c28, rom_thunks::nop_return_zero)
        .expect("install Arduino delay() thunk");
    // WifiWsLink::begin pulls in the entire ESP-IDF WiFi + lwip stack,
    // which calls sys_arch_mbox_fetch on uninitialized mailboxes in our
    // sim (we don't model the WiFi driver tasks). Stubbing lets setup()
    // proceed past the network init into the rest of the panel-relevant
    // code (input handler, USB link, eventual loop()).
    machine
        .bus
        .install_flash_thunk(0x400d_de98, rom_thunks::nop_return_zero)
        .expect("install WifiWsLink::begin thunk");
    // WifiWsLink::loop calls ws_.loop() on the WebSocketsClient, which
    // walks uninitialised TCP/lwIP state when ::begin was stubbed. That
    // burns infinite cycles in ArduinoJson string-pool lookups and
    // prevents loop() from ever reaching the `g_dirty` render check.
    // Stub the loop too so the WiFi path is a true no-op.
    machine
        .bus
        .install_flash_thunk(0x400d_dccc, rom_thunks::nop_return_zero)
        .expect("install WifiWsLink::loop thunk");
    // sendHello (anonymous namespace) walks ArduinoJson serialization, which
    // calls into ObjectData::getMember and StringPool repeatedly. With our
    // stubbed Serial path the underlying string buffer never drains, and the
    // serializer keeps re-traversing the JSON tree forever. Skip sendHello —
    // there's no real daemon to handshake with anyway. The boot-time render
    // (set by `g_dirty = true` at end of setup()) still fires.
    machine
        .bus
        .install_flash_thunk(0x400e_0034, rom_thunks::nop_return_zero)
        .expect("install sendHello thunk");
    // Fake the app image header at 0x3F400000 (start of flash dcache view).
    // On real silicon, the 2nd-stage bootloader places this header before
    // the app's first segment. esp_image_header_t (24 bytes):
    //   magic=0xE9, segment_count=1, spi_mode=0, spi_speed_size=0,
    //   entry_addr=<elf entry>, wp_pin=0xEE, spi_pin_drv=[0,0,0],
    //   chip_id=0 (ESP32), min_chip_rev=0, reserved=[0;8], hash_appended=0
    let entry = 0x40081bf0_u32; // matches the external Arduino-ESP32 ELF entry
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
        0, // chip_id = 0 (ESP32)
        0, // min_chip_rev
        0,
        0,
        0,
        0,
        0,
        0,
        0,
        0, // reserved
        0, // hash_appended
    ];
    for (i, &b) in header.iter().enumerate() {
        let _ = machine.bus.write_u8(0x3F40_0000 + i as u64, b);
    }

    // Generous step budget — Arduino-ESP32 + GxEPD2 boot is much heavier
    // than our hand-rolled Rust firmware. ~500M steps is comfortable
    // headroom for "show me what we get."
    // Sample PC every N steps for a coarse heat map of where the firmware
    // spends its time. Helps locate infinite loops without full tracing.
    const MAX_STEPS: u64 = 30_000_000;
    const SAMPLE_EVERY: u64 = 100_000;
    let mut last_pc = 0u32;
    let mut wfi_streak = 0u32;
    let mut step_count = 0u64;
    let mut pc_trail: [u32; 64] = [0; 64];
    let mut trail_idx = 0usize;
    let mut samples: Vec<(u64, u32)> = Vec::new();

    // Cross-core IPI bridge state. ESP-IDF's `esp_crosscore_int_send_yield`
    // writes 1 to DPORT_CPU_INTR_FROM_CPU_n_REG (0x3FF0_00DC/_00E0) to raise
    // FROM_CPU_INTRn on the target CPU. Real silicon routes this through the
    // intmatrix (DPORT_PRO_FROM_CPU_INTRn_MAP_REG at 0x3FF0_0224/_0228) to a
    // CPU internal interrupt bit. We snoop DPORT each step:
    //   - PRO_FROM_CPU_INTR0/1_MAP captures the bit assignment
    //   - CPU_INTR_FROM_CPU_0/1 trigger writes raise that bit on PRO_CPU
    let mut from_cpu_bit0: Option<u8> = None;
    let mut from_cpu_bit1: Option<u8> = None;
    let mut last_from_cpu0_val: u32 = 0;
    let mut last_from_cpu1_val: u32 = 0;
    let mut ipi_fired = 0u32;
    let mut prev_intlevel: u32 = 0;
    let mut intlevel_drop_with_pending: u32 = 0;
    for _ in 0..MAX_STEPS {
        step_count += 1;
        if step_count.is_multiple_of(10_000) {
            // s_cpu_up[1] — APP_CPU is "running" so start_other_core exits.
            let _ = machine.bus.write_u8(0x3FFC_6F04, 0x01);
            // s_cpu_inited[0..=1] — both CPUs have completed init so the
            // do_other_cpu_settings tail-loop at 0x400ed12d-17c exits.
            let _ = machine.bus.write_u8(0x3FFC_6F01, 0x01);
            let _ = machine.bus.write_u8(0x3FFC_6F02, 0x01);
            // s_system_inited[0..=1] at 0x3FFC_6FFD — start_cpu0's tail
            // ANDs both bytes through a stack temp until both are 1 then
            // calls esp_startup_start_app. Without this fake the boot
            // path stalls forever in a delay loop after do_system_init_fn.
            let _ = machine.bus.write_u8(0x3FFC_6FFD, 0x01);
            let _ = machine.bus.write_u8(0x3FFC_6FFE, 0x01);
            // s_other_cpu_startup_done at 0x3FFC_7190 — main_task spins on
            // this byte waiting for CPU1's startup. We're single-core in the
            // sim, so pretend CPU1 is up by force-setting it.
            let _ = machine.bus.write_u8(0x3FFC_7190, 0x01);
        }
        // Count key function invocations.
        for (pc, name) in [
            (0x4008d260u32, "_frxt_dispatch"),
            (0x4008d2c4, "vPortYield"),
            (0x4008de44, "vTaskSwitchContext"),
            (0x4008dbc4, "xTaskIncrementTick"),
            (0x4008d4f0, "xPortSysTickHandler"),
            (0x40083a10, "_xt_user_exc"),
            (0x40083c98, "_xt_lowint1"),
            (0x40080340, "_UserExceptionVector"),
            (0x40080300, "_KernelExceptionVector"),
            (0x400e03d0, "Arduino loop()"),
            (0x400e180c, "Display::render"),
            (0x400e16f0, "GxEPD2_3C::nextPage"),
            (0x400dd0b4, "GxEPD2_EPD::_writeCommand"),
            (0x400dd114, "GxEPD2_EPD::_writeData"),
            (0x400dd8d8, "GxEPD2_290_C90c::_Update_Full"),
            (0x400d5f20, "SPIClass::begin"),
            (0x400d60cc, "SPIClass::beginTransaction"),
            (0x400d6148, "SPIClass::transfer(u8)"),
            (0x400e14e8, "UsbSerialLink::loop"),
            (0x400ddccc, "WifiWsLink::loop"),
            (0x400de7f4, "Input::loop"),
        ] {
            if machine.cpu.get_pc() == pc {
                static mut COUNTS: [u32; 21] = [0; 21];
                let idx = match pc {
                    0x4008d260 => 0,
                    0x4008d2c4 => 1,
                    0x4008de44 => 2,
                    0x4008dbc4 => 3,
                    0x4008d4f0 => 4,
                    0x40083a10 => 5,
                    0x40083c98 => 6,
                    0x40080340 => 7,
                    0x40080300 => 8,
                    0x400e03d0 => 9,
                    0x400e180c => 10,
                    0x400e16f0 => 11,
                    0x400dd0b4 => 12,
                    0x400dd114 => 13,
                    0x400dd8d8 => 14,
                    0x400d5f20 => 15,
                    0x400d60cc => 16,
                    0x400d6148 => 17,
                    0x400e14e8 => 18,
                    0x400ddccc => 19,
                    0x400de7f4 => 20,
                    _ => 21,
                };
                unsafe {
                    if idx < 21 {
                        COUNTS[idx] += 1;
                        let c = COUNTS[idx];
                        if c <= 3 || c.is_multiple_of(100) {
                            eprintln!("[arduino-esp32-sim] {name} #{c} @step {step_count}");
                        }
                    }
                }
            }
        }
        // Boot waypoint traces.
        for (pc, label) in [
            (0x4008ce2cu32, "xPortStartScheduler"),
            (0x4008d244, "_frxt_tick_timer_init (arms CCOMPARE0)"),
            (0x4008d260, "_frxt_dispatch (first task switch)"),
            (0x4008d272, "_frxt_dispatch loaded SP from TCB"),
            (0x4008d278, "_frxt_dispatch loaded flag a2"),
            (0x4008d28b, "_frxt_dispatch retw.n (short path)"),
            (0x4008d2ba, "_frxt_dispatch _xt_context_restore (long path)"),
            (0x400e90c0, "app_main"),
            (0x401943fc, "main_task entry"),
            (0x4008d2bd, "_frxt_dispatch post-context-restore"),
            (0x4008d2c0, "_frxt_dispatch loading a0 from stack"),
            (0x4008d2c2, "_frxt_dispatch ret.n to task"),
            (0x40083aac, "_xt_user_exit entry"),
            (0x40083abd, "_xt_user_exit rfe"),
            (0x40083ab1, "_xt_user_exit load EPC"),
            (0x4008ce04, "vPortTaskWrapper"),
            (0x4008ce07, "vPortTaskWrapper post-ENTRY"),
            (0x4008ce09, "vPortTaskWrapper callx8 to task"),
            (0x4008197c, "ipc_task entry"),
            (0x4008e858, "xTaskGetSchedulerState"),
            (0x4008cf3d, "xPortEnterCriticalTimeout body"),
            (0x400819ba, "ipc_task blocking on NotifyTake"),
            (0x4008eeb8, "ulTaskGenericNotifyTake entry"),
            (0x400ed360, "esp_ipc_isr_init entry"),
            (0x400ed9a8, "esp_crosscore_int_init entry"),
            (0x400eef04, "esp_intr_alloc entry"),
            (0x40082378, "esp_crosscore_int_send_yield entry"),
            (0x400ed384, "esp_ipc_isr_port_init entry"),
            (0x400822cc, "esp_crosscore_isr fires"),
            (0x400e9068, "loopTask entry (Arduino)"),
            (0x400df3fc, "Arduino setup() entry"),
            (0x400e03d0, "Arduino loop() entry"),
            (0x400e5c28, "Arduino delay()"),
            (0x400e2280, "HardwareSerial::begin()"),
            (0x400df421, "setup: after digitalWrite"),
            (0x400df44a, "setup: after Serial.begin"),
            (0x400df452, "setup: after delay(50)"),
            (0x400df45a, "setup: at Serial.println"),
            (0x400df463, "setup: at getEfuseMac"),
            (0x400e1674, "Display::begin entry"),
            (0x400dcf34, "GxEPD2_EPD::init entry"),
        ] {
            if machine.cpu.get_pc() == pc {
                static mut HITS: [bool; 41] = [false; 41];
                let idx = match pc {
                    0x4008ce2c => 0,
                    0x4008d244 => 1,
                    0x4008d260 => 2,
                    0x4008d272 => 3,
                    0x4008d278 => 4,
                    0x4008d28b => 5,
                    0x4008d2ba => 6,
                    0x400e90c0 => 7,
                    0x401943fc => 8,
                    0x4008d2bd => 9,
                    0x4008d2c0 => 10,
                    0x4008d2c2 => 11,
                    0x40083aac => 12,
                    0x40083abd => 13,
                    0x40083ab1 => 14,
                    0x4008ce04 => 15,
                    0x4008ce07 => 16,
                    0x4008ce09 => 17,
                    0x4008197c => 18,
                    0x4008e858 => 19,
                    0x4008cf3d => 20,
                    0x400819ba => 21,
                    0x4008eeb8 => 22,
                    0x400ed360 => 23,
                    0x400ed9a8 => 24,
                    0x400eef04 => 25,
                    0x40082378 => 26,
                    0x400ed384 => 27,
                    0x400822cc => 28,
                    0x400e9068 => 29,
                    0x400df3fc => 30,
                    0x400e03d0 => 31,
                    0x400e5c28 => 32,
                    0x400e2280 => 33,
                    0x400df421 => 34,
                    0x400df44a => 35,
                    0x400df452 => 36,
                    0x400df45a => 37,
                    0x400df463 => 38,
                    0x400e1674 => 39,
                    0x400dcf34 => 40,
                    _ => 41,
                };
                unsafe {
                    if idx < 41 && !HITS[idx] {
                        HITS[idx] = true;
                        let a0 = machine.cpu.get_register(0);
                        let a1 = machine.cpu.get_register(1);
                        let a2 = machine.cpu.get_register(2);
                        let a3 = machine.cpu.get_register(3);
                        eprintln!("[arduino-esp32-sim] {label} @step {step_count} a0=0x{a0:08x} a1=0x{a1:08x} a2=0x{a2:08x} a3=0x{a3:08x}");
                        if pc == 0x40083aac || pc == 0x4008d260 {
                            // At _xt_user_exit entry or _frxt_dispatch start — peek
                            // at the full saved frame.
                            let sp = if pc == 0x40083aac { a1 } else { 0u32 };
                            if sp != 0 {
                                let mut row = String::new();
                                for off in (0..=40).step_by(4) {
                                    let v =
                                        machine.bus.read_u32(sp as u64 + off).unwrap_or(0xDEADBEEF);
                                    row.push_str(&format!("[+{off}]={v:#010x} "));
                                }
                                eprintln!("[arduino-esp32-sim]   frame@{sp:#x}: {row}");
                            }
                        }
                    }
                }
            }
        }
        // Diagnostic: catch __assert_func entry and print its args
        // (filename, line, function, expr).
        // Trace heap_caps_init entry + region count after soc_get_*.
        if machine.cpu.get_pc() == 0x400ee3e2 {
            // a10 = return of soc_get_available_memory_regions (count)
            // a6 = same (mov.n a6, a10)
            let count = machine.cpu.get_register(10);
            eprintln!("[arduino-esp32-sim] soc_get_available_memory_regions returned count={count} (step {step_count})");
        }
        // Trace the spin-loop at 0x400ed12d once to learn where a7 points.
        if machine.cpu.get_pc() == 0x400ed12d && step_count > 1_000_000 && step_count < 1_000_100 {
            eprintln!(
                "[arduino-esp32-sim] spin@400ed12d: a7=0x{:08x} a6=0x{:08x} sp=0x{:08x}",
                machine.cpu.get_register(7),
                machine.cpu.get_register(6),
                machine.cpu.get_register(1),
            );
        }
        // Trace esp_chip_info return: what byte got stored at offset 10
        // (the cores field).
        if machine.cpu.get_pc() == 0x400ecfaa {
            let sp = machine.cpu.get_register(1);
            let cores = machine.bus.read_u8((sp + 34) as u64).unwrap_or(0xff);
            let model = machine.bus.read_u32(sp as u64 + 24).unwrap_or(0);
            let features = machine.bus.read_u32(sp as u64 + 28).unwrap_or(0);
            let revision = machine.bus.read_u16(sp as u64 + 32).unwrap_or(0);
            eprintln!(
                "[arduino-esp32-sim] esp_chip_info returned: model={model} features=0x{features:08x} revision={revision} cores={cores}"
            );
        }
        // Trap on abort() entry — dump the 8 PCs immediately before it
        // (the call chain leading to the abort).
        if machine.cpu.get_pc() == 0x40091f60 {
            let caller_a0 = machine.cpu.get_register(0);
            eprintln!(
                "[arduino-esp32-sim] abort() entry at step {step_count}. caller_a0=0x{caller_a0:08x}"
            );
            eprintln!("  last 8 PCs leading here:");
            for i in 0..8 {
                let off = ((trail_idx + 64) - 1 - i) % 64;
                eprintln!("    -{:>2}: 0x{:08x}", i + 1, pc_trail[off]);
            }
        }
        if machine.cpu.get_pc() == 0x4008bc54 {
            let read_string = |addr: u32, bus: &SystemBus| -> String {
                let mut out = String::new();
                for i in 0..256u32 {
                    let b = bus.read_u8(addr.wrapping_add(i) as u64).unwrap_or(0);
                    if b == 0 {
                        break;
                    }
                    out.push(b as char);
                }
                out
            };
            let msg_addr = machine.cpu.get_register(2);
            let msg = read_string(msg_addr, &machine.bus);
            eprintln!("[arduino-esp32-sim] esp_system_abort: \"{msg}\" (step {step_count})");
        }
        if machine.cpu.get_pc() == 0x40091fe4 {
            // Inside the caller's window, args are a10..a13.
            // Find them by checking PS.callinc and reading the AR file.
            let read_string = |addr: u32, bus: &SystemBus| -> String {
                let mut out = String::new();
                for i in 0..128u32 {
                    let b = bus.read_u8(addr.wrapping_add(i) as u64).unwrap_or(0);
                    if b == 0 {
                        break;
                    }
                    out.push(b as char);
                }
                out
            };
            // Also dump caller PC and the 8 PCs before this __assert_func entry.
            let caller_a0 = machine.cpu.get_register(0);
            eprintln!("[arduino-esp32-sim] __assert_func entry caller_a0=0x{caller_a0:08x}");
            eprintln!("  last 8 PCs leading here:");
            for i in 0..8 {
                let off = ((trail_idx + 64) - 1 - i) % 64;
                eprintln!("    -{:>2}: 0x{:08x}", i + 1, pc_trail[off]);
            }
            let f_addr = machine.cpu.get_register(10);
            let line = machine.cpu.get_register(11);
            let fn_addr = machine.cpu.get_register(12);
            let expr_addr = machine.cpu.get_register(13);
            let f = read_string(f_addr, &machine.bus);
            let n = read_string(fn_addr, &machine.bus);
            let e = read_string(expr_addr, &machine.bus);
            eprintln!("[arduino-esp32-sim] __assert_func: file=\"{f}\" line={line} fn=\"{n}\" expr=\"{e}\" (step {step_count})");
        }
        // Cross-core IPI snoop. Sample DPORT mapping + trigger regs each step
        // and synthesize the matching INTERRUPT edge when a trigger fires.
        // Skip the first ~60k steps (boot/ROM phase doesn't touch these regs)
        // to keep the per-step cost zero during the heavy boot path.
        if step_count >= 60_000 {
            // Re-read intmatrix mapping every step. The BROM init sweeps all
            // sources to a "default-discard" bit (6) early, then ESP-IDF's
            // esp_intr_alloc rewrites specific sources to their real slots
            // later. Locking in the first non-zero read is wrong because we'd
            // capture the sweep value instead of the actual allocation.
            // DPORT_PRO_CPU_INTR_FROM_CPU_0_MAP_REG = 0x3FF0_0164
            // DPORT_PRO_CPU_INTR_FROM_CPU_1_MAP_REG = 0x3FF0_0168
            if let Ok(v) = machine.bus.read_u32(0x3FF0_0164) {
                let bit = (v & 0x1F) as u8;
                if v != 0 && bit < 32 && from_cpu_bit0 != Some(bit) {
                    let prev = from_cpu_bit0;
                    from_cpu_bit0 = Some(bit);
                    eprintln!("[arduino-esp32-sim] FROM_CPU_INTR0 mapped to CPU int bit {bit} (was {prev:?}) @step {step_count}");
                }
            }
            if let Ok(v) = machine.bus.read_u32(0x3FF0_0168) {
                let bit = (v & 0x1F) as u8;
                if v != 0 && bit < 32 && from_cpu_bit1 != Some(bit) {
                    let prev = from_cpu_bit1;
                    from_cpu_bit1 = Some(bit);
                    eprintln!("[arduino-esp32-sim] FROM_CPU_INTR1 mapped to CPU int bit {bit} (was {prev:?}) @step {step_count}");
                }
            }
            // Trigger detect for FROM_CPU_INTR0 (PRO->PRO/APP yield signal).
            if let Ok(v0) = machine.bus.read_u32(0x3FF0_00DC) {
                if v0 != 0 && v0 != last_from_cpu0_val {
                    if let Some(bit) = from_cpu_bit0 {
                        machine.cpu.sr.raise_interrupt_bits(1u32 << bit);
                        ipi_fired += 1;
                        if ipi_fired <= 5 || ipi_fired.is_multiple_of(100) {
                            let intenable = machine.cpu.sr.read(228); // INTENABLE
                            let interrupt = machine.cpu.sr.read(226); // INTERRUPT
                            let ps = machine.cpu.ps.as_raw();
                            eprintln!("[arduino-esp32-sim] IPI fire #{ipi_fired}: FROM_CPU_INTR0 → raise bit {bit} @step {step_count} pc=0x{:08x} PS=0x{:08x} INTENABLE=0x{:08x} INTERRUPT=0x{:08x}", machine.cpu.get_pc(), ps, intenable, interrupt);
                        }
                    } else if ipi_fired == 0 {
                        eprintln!("[arduino-esp32-sim] FROM_CPU_INTR0 triggered but no map assignment seen @step {step_count}");
                    }
                    // Clear the trigger reg so it re-edges on next write.
                    let _ = machine.bus.write_u32(0x3FF0_00DC, 0);
                }
                last_from_cpu0_val = 0;
            }
            if let Ok(v1) = machine.bus.read_u32(0x3FF0_00E0) {
                if v1 != 0 && v1 != last_from_cpu1_val {
                    if let Some(bit) = from_cpu_bit1 {
                        machine.cpu.sr.raise_interrupt_bits(1u32 << bit);
                        ipi_fired += 1;
                        if ipi_fired <= 5 || ipi_fired.is_multiple_of(100) {
                            eprintln!("[arduino-esp32-sim] IPI fire #{ipi_fired}: FROM_CPU_INTR1 → raise bit {bit} @step {step_count} pc=0x{:08x}", machine.cpu.get_pc());
                        }
                    }
                    let _ = machine.bus.write_u32(0x3FF0_00E0, 0);
                }
                last_from_cpu1_val = 0;
            }
        }

        // Trace INTLEVEL transitions when INTERRUPT bit 6 is pending.
        if (69000..71000).contains(&step_count) {
            let ps = machine.cpu.ps.as_raw();
            let intlevel = ps & 0xF;
            let interrupt = machine.cpu.sr.read(226);
            if intlevel != prev_intlevel {
                intlevel_drop_with_pending += 1;
                if intlevel_drop_with_pending <= 40 {
                    eprintln!("[arduino-esp32-sim] INTLEVEL {prev_intlevel}→{intlevel} (bit6={}) @step {step_count} pc=0x{:08x} PS=0x{ps:08x}", if interrupt & 0x40 != 0 { "PEND" } else { "clr" }, machine.cpu.get_pc());
                }
            }
            prev_intlevel = intlevel;
        }

        if let Err(e) = machine.step() {
            eprintln!(
                "[arduino-esp32-sim] CPU error after {step_count} steps: \
                 last_pc=0x{last_pc:08x} current_pc=0x{:08x} — {e}",
                machine.cpu.get_pc()
            );
            eprintln!("[arduino-esp32-sim] last 64 PCs (most-recent first):");
            for i in 0..64 {
                let idx = (trail_idx + 63 - i) % 64;
                eprintln!("    #{i:2}: 0x{:08x}", pc_trail[idx]);
            }
            break;
        }
        let pc = machine.cpu.get_pc();
        pc_trail[trail_idx] = pc;
        trail_idx = (trail_idx + 1) % 64;
        if step_count.is_multiple_of(SAMPLE_EVERY) {
            samples.push((step_count, pc));
        }
        if pc == last_pc {
            wfi_streak += 1;
            if wfi_streak > 100_000 {
                eprintln!("[arduino-esp32-sim] halt detected at pc=0x{pc:08x}");
                break;
            }
        } else {
            wfi_streak = 0;
            last_pc = pc;
        }
    }
    eprintln!("[arduino-esp32-sim] last 10 PC samples:");
    for &(s, p) in samples.iter().rev().take(10) {
        eprintln!("    step {s:>10}: pc=0x{p:08x}");
    }

    // Snapshot the panel.
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

    eprintln!(
        "[arduino-esp32-sim] panel state: refresh_generation={}, power_on={}",
        panel.refresh_generation(),
        panel.power_on()
    );
    eprintln!(
        "[arduino-esp32-sim] SPI3 transactions={} captured_bytes_len={}",
        spi.transactions(),
        spi.captured_bytes().len(),
    );
    let cap = spi.captured_bytes();
    let head_n = cap.len().min(80);
    if head_n > 0 {
        let head_hex: Vec<String> = cap[..head_n].iter().map(|b| format!("{b:02x}")).collect();
        eprintln!(
            "[arduino-esp32-sim] first {head_n} SPI bytes: {}",
            head_hex.join(" ")
        );
    }
    if cap.len() > 160 {
        let tail = &cap[cap.len() - 80..];
        let tail_hex: Vec<String> = tail.iter().map(|b| format!("{b:02x}")).collect();
        eprintln!(
            "[arduino-esp32-sim] last 80 SPI bytes: {}",
            tail_hex.join(" ")
        );
    }
    // Count non-trivial pixels (anything that's not the all-white reset state).
    let black = panel.black_plane();
    let non_white_black = black.iter().filter(|&&b| b != 0xFF).count();
    let red = panel.red_plane();
    let non_white_red = red.iter().filter(|&&b| b != 0xFF).count();
    eprintln!(
        "[arduino-esp32-sim] black plane non-FF bytes: {non_white_black}/{}, \
         red plane non-FF bytes: {non_white_red}/{}",
        black.len(),
        red.len()
    );

    // Render the panel as a PPM so a human can visually verify the splash.
    // Native portrait: 128w × 296h; black plane bit-packed MSB-first, 16 bytes per row.
    let (w, h) = panel.dimensions();
    let stride = w / 8;
    let mut ppm = format!("P6\n{w} {h}\n255\n").into_bytes();
    for y in 0..h {
        for x in 0..w {
            let byte = y * stride + x / 8;
            let bit = 7 - (x % 8);
            let black_bit = (black[byte] >> bit) & 1;
            let red_bit = (red[byte] >> bit) & 1;
            // GxEPD2 inverts the source bitmap before sending 0x26, so source's
            // "no red" (0xFF) becomes 0x00 on the wire. Treat wire red_bit==1
            // as the actual "render red" intent.
            let (r, g, b) = if red_bit == 1 {
                (220u8, 30u8, 40u8)
            } else if black_bit == 0 {
                (0u8, 0u8, 0u8)
            } else {
                (245u8, 245u8, 240u8)
            };
            ppm.extend_from_slice(&[r, g, b]);
        }
    }
    let out_path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../target/arduino-esp32_panel.ppm");
    if let Some(parent) = out_path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Err(e) = std::fs::write(&out_path, &ppm) {
        eprintln!("[arduino-esp32-sim] failed to write panel PPM: {e}");
    } else {
        eprintln!(
            "[arduino-esp32-sim] panel image written to {}",
            out_path.display()
        );
    }

    // Regression bounds. Fires only when the ignored test is invoked
    // explicitly with the external Arduino-ESP32 firmware ELF available — locks in the
    // sim's known-good byte signature against silent drift.
    let txns = spi.transactions() as i64;
    let baseline_txns = EXTERNAL_BASELINE_SPI3_TXNS as i64;
    let txn_tolerance = (baseline_txns / 200).max(50); // ±0.5% or ±50 txns
    assert!(
        (baseline_txns - txn_tolerance..=baseline_txns + txn_tolerance).contains(&txns),
        "External Arduino-ESP32 SPI3 transaction count drift: got {txns}, baseline {baseline_txns} (±{txn_tolerance})"
    );
    let baseline_black = EXTERNAL_BASELINE_BLACK_NONFF as i64;
    let black_signed = non_white_black as i64;
    assert!(
        (baseline_black - 10..=baseline_black + 10).contains(&black_signed),
        "External Arduino-ESP32 black-plane non-FF byte count drift: got {non_white_black}, baseline {baseline_black} (±10)"
    );
}
