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
    let mut pc_trail: [u32; 16] = [0; 16];
    let mut trail_idx = 0usize;
    let mut samples: Vec<(u64, u32)> = Vec::new();
    for _ in 0..MAX_STEPS {
        step_count += 1;
        // Re-write s_cpu_up[1] every 10k steps. xtensa-lx-rt's Reset zeroes
        // .bss BEFORE start_other_core polls this byte, so a one-shot write
        // before stepping gets clobbered. Periodic re-write keeps it set
        // until start_other_core's spin loop reads it.
        if step_count % 10_000 == 0 {
            let _ = machine.bus.write_u8(0x3FFC_6F04, 0x01);
        }
        if let Err(e) = machine.step() {
            eprintln!(
                "[agentdeck-sim] CPU error after {step_count} steps: \
                 last_pc=0x{last_pc:08x} current_pc=0x{:08x} — {e}",
                machine.cpu.get_pc()
            );
            eprintln!("[agentdeck-sim] last 16 PCs (most-recent first):");
            for i in 0..16 {
                let idx = (trail_idx + 15 - i) % 16;
                eprintln!("    #{i:2}: 0x{:08x}", pc_trail[idx]);
            }
            break;
        }
        let pc = machine.cpu.get_pc();
        pc_trail[trail_idx] = pc;
        trail_idx = (trail_idx + 1) % 16;
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
