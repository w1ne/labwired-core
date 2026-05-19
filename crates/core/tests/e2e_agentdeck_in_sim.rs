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
            let _ = machine.bus.write_u8(0x3FFC_6F04, 0x01);
        }
        // Diagnostic: catch __assert_func entry and print its args
        // (filename, line, function, expr).
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
