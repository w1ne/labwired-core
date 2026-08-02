//! ESP32-classic aids stability (PR-I).
//!
//! Repro/regression for the playground path:
//! `install_arduino_esp32_quirks` + dual-core + `AdvanceRequest::run` (batch /
//! idle FF) on the labwired-ereader Arduino-ESP32 ELF.
//!
//! The shipped wasm `step_with_esp32_aids` dual-core path currently falls back
//! to N× `AdvanceRequest::single()` (idle FF disabled). That hides idle-FF
//! bugs and is also why browser busy MIPS sit ~2. When aids routes through
//! `AdvanceRequest::run`, idle FF must not hit unmapped-memory faults, and a
//! fault must leave the machine in a state that is still safely droppable.
//!
//! ELF resolution (first hit wins):
//!   - `$LABWIRED_EREADER_ELF`
//!   - monorepo playground demo (relative to `core/crates/core` or `core/`)
//!
//! Skip quietly when no ELF is present so CI without the playground tree stays green.

use labwired_core::bus::SystemBus;
use labwired_core::cpu::xtensa_lx7::XtensaLx7;
use labwired_core::peripherals::esp_xtensa_common::rom_thunks;
use labwired_core::system::xtensa::configure_xtensa_esp32;
use labwired_core::{AdvanceRequest, Cpu, Machine};
use std::path::PathBuf;
use std::time::Instant;

fn ereader_elf() -> Option<PathBuf> {
    let mut candidates: Vec<PathBuf> = Vec::new();
    if let Ok(p) = std::env::var("LABWIRED_EREADER_ELF") {
        candidates.push(PathBuf::from(p));
    }
    // `cargo test -p labwired-core` CWD is typically `core/crates/core`.
    candidates.push(PathBuf::from(
        "../../../packages/playground/public/wasm/demo-labwired-ereader.elf",
    ));
    // Running from monorepo `core/`.
    candidates.push(PathBuf::from(
        "../packages/playground/public/wasm/demo-labwired-ereader.elf",
    ));
    candidates.push(PathBuf::from(
        "packages/playground/public/wasm/demo-labwired-ereader.elf",
    ));
    candidates.push(PathBuf::from(
        "/tmp/labwired-ereader/build/labwired-ereader.ino.elf",
    ));
    candidates.into_iter().find(|p| p.exists())
}

/// Mirror `WasmSimulator::install_arduino_esp32_quirks` thunk set (dual-core).
fn install_wasm_like_quirks(machine: &mut Machine<XtensaLx7>, elf_bytes: &[u8]) {
    machine.cpu.set_sp(0x3FFE_0000);
    if let Some(cpu1) = machine.cpu_secondary.as_mut() {
        cpu1.set_sp(0x3FFD_8000);
    }

    let symbol_addrs = labwired_loader::extract_arduino_esp32_thunks(elf_bytes);
    // Real dual-core: no handshake forge / loopTask repin.
    rom_thunks::set_appcpu_up_flags(Vec::new());

    if let Some(&addr) = symbol_addrs.get("pxCurrentTCB") {
        rom_thunks::PX_CURRENT_TCB_ADDR.with(|s| s.set(Some(addr)));
    }

    let mut thunks: Vec<(u32, rom_thunks::RomThunkFn)> = Vec::new();
    let mut push_named = |sym: &str, f: rom_thunks::RomThunkFn| {
        if let Some(&pc) = symbol_addrs.get(sym) {
            thunks.push((pc, f));
        }
    };

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
        "_ZN14HardwareSerial5writeEh",
        "_ZN14HardwareSerial5writeEPKhj",
        "_ZN14HardwareSerial9availableEv",
        "_ZN14HardwareSerial5flushEv",
        "_ZN14HardwareSerial9readBytesEPcj",
        "_ZN14HardwareSerial9readBytesEPhj",
        "_ZN14HardwareSerial5beginEmjaabmh",
        "_get_effective_baudrate",
        "uartAvailable",
        "uartAvailableForWrite",
        "uartWrite",
        "uartWriteBuf",
        "_Z14serialEventRunv",
        "vListInsert",
        "spi_flash_init_lock",
        "spi_flash_op_lock",
        "spi_flash_op_unlock",
    ] {
        push_named(sym, rom_thunks::nop_return_zero);
    }

    push_named(
        "esp_ota_get_running_partition",
        rom_thunks::nop_return_fake_ptr,
    );
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
        push_named(sym, rom_thunks::nop_return_fake_ptr);
    }

    push_named("esp_chip_info", rom_thunks::esp_chip_info_stub);
    push_named("__getreent", rom_thunks::getreent_dram_fake_ptr);
    push_named(
        "esp_timer_impl_get_counter_reg",
        rom_thunks::monotonic_counter_32,
    );
    push_named("esp_clk_cpu_freq", rom_thunks::esp_clk_cpu_freq_240mhz);
    push_named(
        "xQueueCreateMutexStatic",
        rom_thunks::x_queue_create_mutex_static_echo,
    );
    push_named(
        "xTaskGetCurrentTaskHandle",
        rom_thunks::x_task_get_current_task_handle,
    );
    push_named("xQueueSemaphoreTake", rom_thunks::return_pd_true);
    push_named("xQueueGenericSend", rom_thunks::return_pd_true);
    push_named("ulTaskGenericNotifyTake", rom_thunks::return_pd_true);
    push_named("spiStartBus", rom_thunks::spi_start_bus_fake);
    push_named(
        "_ZN8SPIClass16beginTransactionE11SPISettings",
        rom_thunks::spi_class_begin_transaction,
    );
    push_named(
        "xthal_window_spill_nw",
        rom_thunks::xthal_window_spill_thunk,
    );

    // Overwrite abort family with noreturn halt (same order as wasm install).
    for sym in &[
        "panic_abort",
        "__assert_func",
        "abort",
        "__assert",
        "__cxa_pure_virtual",
        "__cxa_throw",
    ] {
        push_named(sym, rom_thunks::abort_halt);
    }

    for (pc, f) in thunks {
        machine
            .bus
            .install_flash_thunk(pc, f)
            .unwrap_or_else(|e| panic!("install thunk @{pc:#x}: {e}"));
    }
}

fn build_machine(elf_path: &std::path::Path) -> Machine<XtensaLx7> {
    // Mirror wasm construct: clear process-global aids state between sessions.
    rom_thunks::reset_esp32_session_state();
    let elf_bytes = std::fs::read(elf_path).expect("read elf");
    let image = labwired_loader::load_elf(elf_path).expect("parse elf");
    let mut bus = SystemBus::new();
    let cpu = configure_xtensa_esp32(&mut bus);
    let manifest: labwired_config::SystemManifest = serde_yaml::from_str(
        r#"
name: esp32-wroom-epaper
chip: esp32
external_devices:
  - id: epaper
    type: uc8151d_tricolor_290
    connection: spi3
    config:
      cs_pin: GPIO5
      dc_pin: GPIO17
"#,
    )
    .expect("manifest");
    labwired_core::system::xtensa::attach_esp32_external_devices(&mut bus, &manifest)
        .expect("attach panel");
    bus.refresh_peripheral_index();

    let mut machine = Machine::new(cpu, bus).with_secondary_cpu(XtensaLx7::new_app_cpu());
    machine.load_firmware(&image).expect("load");
    machine.cpu.set_pc(image.entry_point as u32);
    install_wasm_like_quirks(&mut machine, &elf_bytes);
    machine.config.peripheral_tick_interval = 512;
    machine.bus.config.peripheral_tick_interval = 512;
    machine
}

fn dump_cores(machine: &Machine<XtensaLx7>, label: &str) {
    let pc0 = machine.cpu.get_pc();
    let parked0 = machine.cpu.is_parked_idle();
    let (pc1, parked1) = machine
        .cpu_secondary
        .as_ref()
        .map(|c| (c.get_pc(), c.is_parked_idle()))
        .unwrap_or((0, false));
    eprintln!(
        "{label}: pc0={pc0:#010x} parked0={parked0} pc1={pc1:#010x} parked1={parked1} skipped={} total={}",
        machine.idle_fast_forward_cycles_skipped, machine.total_cycles
    );
    for i in 0..16u8 {
        eprint!("  a{i}={:#010x}", machine.cpu.get_register(i));
        if i % 4 == 3 {
            eprintln!();
        }
    }
    if let Some(sec) = machine.cpu_secondary.as_ref() {
        for i in 0..16u8 {
            eprint!("  sec.a{i}={:#010x}", sec.get_register(i));
            if i % 4 == 3 {
                eprintln!();
            }
        }
    }
}

/// Primary regression: dual-core + idle FF + batched advance must not fault
/// for several million cycles of the ereader bring-up / idle path.
#[test]
fn esp32_classic_ereader_idle_ff_batch_does_not_fault() {
    let Some(elf) = ereader_elf() else {
        eprintln!("[skip] no ereader elf");
        return;
    };
    eprintln!("using elf {elf:?}");

    let mut machine = build_machine(&elf);
    machine.config.idle_fast_forward_enabled = true;

    let target = 5_000_000u64;
    let batch = 50_000u64;
    let t0 = Instant::now();
    let mut advanced = 0u64;

    while advanced < target {
        let n = batch.min(target - advanced);
        match machine.advance(AdvanceRequest::run(Some(n))) {
            Ok(rep) => {
                // Prefer elapsed device time (includes idle FF skips).
                let step = rep.elapsed_cycles.max(rep.fuel_consumed).max(1);
                advanced += step;
            }
            Err(e) => {
                dump_cores(
                    &machine,
                    &format!("FAIL idle-ff advanced={advanced} err={e}"),
                );
                panic!("idle FF batch fault: {e}");
            }
        }
    }

    let wall = t0.elapsed().as_secs_f64();
    let mips = (advanced as f64 / wall) / 1e6;
    eprintln!(
        "OK idle-ff: advanced={advanced} wall={wall:.3}s mips={mips:.3} skipped={}",
        machine.idle_fast_forward_cycles_skipped
    );
    dump_cores(&machine, "final");
    // Machine must remain droppable after a long run (dispose-safety smoke).
    drop(machine);
}

/// Busy path (FF off) smoke + MIPS sample.
#[test]
fn esp32_classic_ereader_busy_batch_smoke() {
    let Some(elf) = ereader_elf() else {
        eprintln!("[skip] no ereader elf");
        return;
    };
    let mut machine = build_machine(&elf);
    machine.config.idle_fast_forward_enabled = false;

    let target = 2_000_000u64;
    let t0 = Instant::now();
    let mut advanced = 0u64;
    while advanced < target {
        let n = 50_000u64.min(target - advanced);
        match machine.advance(AdvanceRequest::run(Some(n))) {
            Ok(rep) => advanced += rep.elapsed_cycles.max(1),
            Err(e) => {
                dump_cores(&machine, &format!("FAIL busy advanced={advanced} err={e}"));
                panic!("busy batch fault: {e}");
            }
        }
    }
    let wall = t0.elapsed().as_secs_f64();
    eprintln!(
        "OK busy: advanced={advanced} wall={wall:.3}s mips={:.3} skipped={}",
        (advanced as f64 / wall) / 1e6,
        machine.idle_fast_forward_cycles_skipped
    );
    drop(machine);
}

/// Mirrors shipped wasm `step_with_esp32_aids` dual-core path: N× single.
#[test]
fn esp32_classic_ereader_single_step_aids_path_does_not_fault() {
    let Some(elf) = ereader_elf() else {
        eprintln!("[skip] no ereader elf");
        return;
    };
    let mut machine = build_machine(&elf);
    machine.config.idle_fast_forward_enabled = true;
    let target = 1_500_000u64;
    let t0 = Instant::now();
    for i in 0..target {
        if let Err(e) = machine.advance(AdvanceRequest::single()) {
            dump_cores(&machine, &format!("FAIL single i={i} err={e}"));
            panic!("single-step aids path fault: {e}");
        }
        if i > 0 && i % 250_000 == 0 {
            dump_cores(&machine, &format!("progress i={i}"));
        }
    }
    let wall = t0.elapsed().as_secs_f64();
    eprintln!(
        "OK single-step aids: cycles={target} wall={wall:.3}s mips={:.3}",
        (target as f64 / wall) / 1e6
    );
    drop(machine);
}

/// Sequential sessions must not inherit the prior session's fake timer /
/// APPCPU TLS (the wasm worker re-run bug: session A OK, session B faults at
/// ~0x33xxxx).
#[test]
fn esp32_classic_sequential_sessions_do_not_fault() {
    let Some(elf) = ereader_elf() else {
        eprintln!("[skip] no ereader elf");
        return;
    };
    for label in ["A", "B", "C"] {
        let mut machine = build_machine(&elf);
        machine.config.idle_fast_forward_enabled = true;
        let target = 2_000_000u64;
        let mut advanced = 0u64;
        while advanced < target {
            let n = 50_000u64.min(target - advanced);
            match machine.advance(AdvanceRequest::run(Some(n))) {
                Ok(rep) => advanced += rep.elapsed_cycles.max(1),
                Err(e) => {
                    dump_cores(
                        &machine,
                        &format!("FAIL session={label} advanced={advanced} err={e}"),
                    );
                    panic!("session {label} fault: {e}");
                }
            }
        }
        eprintln!(
            "OK session {label}: advanced={advanced} skipped={}",
            machine.idle_fast_forward_cycles_skipped
        );
        drop(machine);
    }
}

/// After a forced unmapped access error path, dropping the machine must not
/// panic (wasm dispose / free safety).
#[test]
fn machine_drop_is_safe_after_step_error() {
    let mut bus = SystemBus::new();
    let cpu = configure_xtensa_esp32(&mut bus);
    // Point PC at unmapped low RAM so the first fetch faults.
    let mut machine = Machine::new(cpu, bus);
    machine.cpu.set_pc(0x0033_9529);
    let err = machine
        .advance(AdvanceRequest::single())
        .expect_err("expected unmapped fetch");
    let msg = format!("{err}");
    assert!(
        msg.contains("Memory access violation"),
        "unexpected err: {msg}"
    );
    // Must not unwind / double-free.
    drop(machine);
}
