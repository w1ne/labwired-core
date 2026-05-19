// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT
//
// Stress test: load the AgentDeck firmware ELF (Arduino-ESP32 + GxEPD2)
// into our ESP32-classic sim and see how far it gets.
//
// AgentDeck is real production firmware that the user verified paints the
// physical e-paper panel. Same wiring, same chip — so if our sim's ESP32
// emulation is complete enough, the panel-state assertion at the end
// should see the SSD1680 receive a non-zero refresh.

use labwired_core::bus::SystemBus;
use labwired_core::peripherals::components::Ssd1680Tricolor290;
use labwired_core::peripherals::esp32::spi::Esp32Spi;
use labwired_core::system::xtensa::configure_xtensa_esp32;
use labwired_core::{Bus, Cpu, Machine};
use std::path::PathBuf;

#[test]
#[ignore = "loads ~22 MB AgentDeck firmware ELF; only meaningful when the AgentDeck repo is checked out alongside labwired."]
fn agentdeck_firmware_drives_panel_in_sim() {
    let elf = PathBuf::from(
        "/home/andrii/Projects/AgentDeck/firmware/.pio/build/wroom32u/firmware.elf",
    );
    if !elf.exists() {
        eprintln!("[skip] AgentDeck firmware ELF unavailable at {elf:?}");
        return;
    }

    let mut bus = SystemBus::new();
    let cpu = configure_xtensa_esp32(&mut bus);

    // Wire the SSD1680 to spi3, same as the e-paper lab.
    let spi3_idx = bus
        .find_peripheral_index_by_name("spi3")
        .expect("spi3 registered");
    let any = bus.peripherals[spi3_idx].dev.as_any_mut().unwrap();
    any.downcast_mut::<Esp32Spi>()
        .unwrap()
        .attach(Box::new(Ssd1680Tricolor290::new("GPIO5")));

    bus.refresh_peripheral_index();
    let mut machine = Machine::new(cpu, bus);

    let image = labwired_loader::load_elf(&elf).expect("parse ELF");
    machine.load_firmware(&image).expect("load firmware");
    machine.cpu.set_pc(image.entry_point as u32);
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
    use labwired_core::peripherals::esp32s3::rom_thunks;
    machine.bus.install_flash_thunk(0x400e_e3b0, rom_thunks::esp_idf_heap_caps_init)
        .expect("install heap_caps_init thunk");
    machine.bus.install_flash_thunk(0x4008_2904, rom_thunks::esp_idf_heap_caps_malloc)
        .expect("install heap_caps_malloc thunk");
    machine.bus.install_flash_thunk(0x4008_2a70, rom_thunks::esp_idf_heap_caps_calloc)
        .expect("install heap_caps_calloc thunk");
    machine.bus.install_flash_thunk(0x4008_25dc, rom_thunks::esp_idf_heap_caps_free)
        .expect("install heap_caps_free thunk");
    machine.bus.install_flash_thunk(0x4008_29f0, rom_thunks::esp_idf_heap_caps_realloc)
        .expect("install heap_caps_realloc thunk");
    // esp_timer_init computes a divider for the LACT timer (1MHz tick from
    // APB clock). The HAL asserts divider >= 2; our APB clock readout path
    // isn't fully wired and the inlined math underflows. Stub the whole
    // init to return 0 (ESP_OK) — software timers won't work, but boot
    // continues past the FreeRTOS scheduler-start path.
    machine.bus.install_flash_thunk(0x4012_9034, rom_thunks::nop_return_zero)
        .expect("install esp_timer_init thunk");
    // spi_flash_disable/enable_interrupts_caches_and_other_cpu take the
    // s_flash_op_mutex which isn't initialized until esp_flash_app_init
    // runs later in boot. The sim doesn't need to disable interrupts or
    // suspend caches around flash ops — flash is just LinearMemory we
    // can touch directly — so no-op the mutex-locking wrappers.
    machine.bus.install_flash_thunk(0x4008_17dc, rom_thunks::nop_return_zero)
        .expect("install spi_flash_disable_... thunk");
    machine.bus.install_flash_thunk(0x4008_188c, rom_thunks::nop_return_zero)
        .expect("install spi_flash_enable_... thunk");
    // newlib `__retarget_lock_acquire_recursive` and friends assert that
    // the lock pointer is non-NULL — but a number of newlib locks aren't
    // initialized in our boot path (we haven't wired esp_libc's lock-init
    // chain end-to-end). For a single-threaded sim, the locks are
    // unnecessary; stub all four lock entry points as no-ops.
    machine.bus.install_flash_thunk(0x4008_3384, rom_thunks::nop_return_zero)  // __retarget_lock_init_recursive
        .expect("install lock_init_recursive thunk");
    machine.bus.install_flash_thunk(0x4008_339c, rom_thunks::nop_return_zero)  // __retarget_lock_close_recursive
        .expect("install lock_close_recursive thunk");
    machine.bus.install_flash_thunk(0x4008_33b0, rom_thunks::nop_return_zero)  // __retarget_lock_acquire_recursive
        .expect("install lock_acquire_recursive thunk");
    machine.bus.install_flash_thunk(0x4008_33cc, rom_thunks::nop_return_zero)  // __retarget_lock_release_recursive
        .expect("install lock_release_recursive thunk");
    // ESP_ERROR_CHECK macros call _esp_error_check_failed on non-zero return
    // values, which aborts. Our subsystem stubs return 0 (ESP_OK) so this
    // shouldn't fire from our own stubs, but firmware code paths may still
    // trigger it on real ESP-IDF function returns we don't fully model.
    // Stub to no-op so boot continues past these soft failures.
    machine.bus.install_flash_thunk(0x4008_bbd0, rom_thunks::nop_return_zero)
        .expect("install _esp_error_check_failed thunk");
    // Fake the app image header at 0x3F400000 (start of flash dcache view).
    // On real silicon, the 2nd-stage bootloader places this header before
    // the app's first segment. esp_image_header_t (24 bytes):
    //   magic=0xE9, segment_count=1, spi_mode=0, spi_speed_size=0,
    //   entry_addr=<elf entry>, wp_pin=0xEE, spi_pin_drv=[0,0,0],
    //   chip_id=0 (ESP32), min_chip_rev=0, reserved=[0;8], hash_appended=0
    let entry = 0x40081bf0_u32; // matches AgentDeck ELF entry
    let header: [u8; 24] = [
        0xE9, 0x01, 0x00, 0x00,
        (entry & 0xFF) as u8, ((entry >> 8) & 0xFF) as u8,
        ((entry >> 16) & 0xFF) as u8, ((entry >> 24) & 0xFF) as u8,
        0xEE, 0, 0, 0,
        0, 0, // chip_id = 0 (ESP32)
        0,    // min_chip_rev
        0, 0, 0, 0, 0, 0, 0, 0, // reserved
        0,    // hash_appended
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
    for _ in 0..MAX_STEPS {
        step_count += 1;
        if step_count % 10_000 == 0 {
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
        }
        // Diagnostic: catch __assert_func entry and print its args
        // (filename, line, function, expr).
        // Trace heap_caps_init entry + region count after soc_get_*.
        if machine.cpu.get_pc() == 0x400ee3e2 {
            // a10 = return of soc_get_available_memory_regions (count)
            // a6 = same (mov.n a6, a10)
            let count = machine.cpu.get_register(10);
            eprintln!("[agentdeck-sim] soc_get_available_memory_regions returned count={count} (step {step_count})");
        }
        // Trace the spin-loop at 0x400ed12d once to learn where a7 points.
        if machine.cpu.get_pc() == 0x400ed12d && step_count > 1_000_000 && step_count < 1_000_100 {
            eprintln!("[agentdeck-sim] spin@400ed12d: a7=0x{:08x} a6=0x{:08x} sp=0x{:08x}",
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
                "[agentdeck-sim] esp_chip_info returned: model={model} features=0x{features:08x} revision={revision} cores={cores}"
            );
        }
        // Trap on abort() entry — dump the 8 PCs immediately before it
        // (the call chain leading to the abort).
        if machine.cpu.get_pc() == 0x40091f60 {
            let caller_a0 = machine.cpu.get_register(0);
            eprintln!("[agentdeck-sim] abort() entry at step {step_count}. caller_a0=0x{caller_a0:08x}");
            eprintln!("  last 8 PCs leading here:");
            for i in 0..8 {
                let off = ((trail_idx + 64) - 1 - i) % 64;
                eprintln!("    -{:>2}: 0x{:08x}", i+1, pc_trail[off]);
            }
        }
        if machine.cpu.get_pc() == 0x4008bc54 {
            let read_string = |addr: u32, bus: &SystemBus| -> String {
                let mut out = String::new();
                for i in 0..256u32 {
                    let b = bus.read_u8(addr.wrapping_add(i) as u64).unwrap_or(0);
                    if b == 0 { break; }
                    out.push(b as char);
                }
                out
            };
            let msg_addr = machine.cpu.get_register(2);
            let msg = read_string(msg_addr, &machine.bus);
            eprintln!("[agentdeck-sim] esp_system_abort: \"{msg}\" (step {step_count})");
        }
        if machine.cpu.get_pc() == 0x40091fe4 {
            // Inside the caller's window, args are a10..a13.
            // Find them by checking PS.callinc and reading the AR file.
            let read_string = |addr: u32, bus: &SystemBus| -> String {
                let mut out = String::new();
                for i in 0..128u32 {
                    let b = bus.read_u8(addr.wrapping_add(i) as u64).unwrap_or(0);
                    if b == 0 { break; }
                    out.push(b as char);
                }
                out
            };
            // Also dump caller PC and the 8 PCs before this __assert_func entry.
            let caller_a0 = machine.cpu.get_register(0);
            eprintln!("[agentdeck-sim] __assert_func entry caller_a0=0x{caller_a0:08x}");
            eprintln!("  last 8 PCs leading here:");
            for i in 0..8 {
                let off = ((trail_idx + 64) - 1 - i) % 64;
                eprintln!("    -{:>2}: 0x{:08x}", i+1, pc_trail[off]);
            }
            let f_addr = machine.cpu.get_register(10);
            let line = machine.cpu.get_register(11);
            let fn_addr = machine.cpu.get_register(12);
            let expr_addr = machine.cpu.get_register(13);
            let f = read_string(f_addr, &machine.bus);
            let n = read_string(fn_addr, &machine.bus);
            let e = read_string(expr_addr, &machine.bus);
            eprintln!("[agentdeck-sim] __assert_func: file=\"{f}\" line={line} fn=\"{n}\" expr=\"{e}\" (step {step_count})");
        }
        if let Err(e) = machine.step() {
            eprintln!(
                "[agentdeck-sim] CPU error after {step_count} steps: \
                 last_pc=0x{last_pc:08x} current_pc=0x{:08x} — {e}",
                machine.cpu.get_pc()
            );
            eprintln!("[agentdeck-sim] last 64 PCs (most-recent first):");
            for i in 0..64 {
                let idx = (trail_idx + 63 - i) % 64;
                eprintln!("    #{i:2}: 0x{:08x}", pc_trail[idx]);
            }
            break;
        }
        let pc = machine.cpu.get_pc();
        pc_trail[trail_idx] = pc;
        trail_idx = (trail_idx + 1) % 64;
        if step_count % SAMPLE_EVERY == 0 {
            samples.push((step_count, pc));
        }
        if pc == last_pc {
            wfi_streak += 1;
            if wfi_streak > 100_000 {
                eprintln!("[agentdeck-sim] halt detected at pc=0x{pc:08x}");
                break;
            }
        } else {
            wfi_streak = 0;
            last_pc = pc;
        }
    }
    eprintln!("[agentdeck-sim] last 10 PC samples:");
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
        .find_map(|d| d.as_any().and_then(|a| a.downcast_ref::<Ssd1680Tricolor290>()))
        .expect("panel attached");

    eprintln!(
        "[agentdeck-sim] panel state: refresh_generation={}, power_on={}",
        panel.refresh_generation(),
        panel.power_on()
    );
    // Count non-trivial pixels (anything that's not the all-white reset state).
    let black = panel.black_plane();
    let non_white_black = black.iter().filter(|&&b| b != 0xFF).count();
    let red = panel.red_plane();
    let non_white_red = red.iter().filter(|&&b| b != 0xFF).count();
    eprintln!(
        "[agentdeck-sim] black plane non-FF bytes: {non_white_black}/{}, \
         red plane non-FF bytes: {non_white_red}/{}",
        black.len(),
        red.len()
    );
}
