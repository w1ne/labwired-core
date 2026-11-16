// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Opt-in probe: boots an external firmware (LABWIRED_PROBE_ELF) on
//! examples/ramps14-marlin/system.yaml and prints ADC register state and
//! selected SRAM bytes (LABWIRED_PROBE_SRAM="0x12cb:1,0x1061:16").

use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::bus::SystemBus;
use labwired_core::cpu::Avr;
use labwired_core::{Bus, Machine};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

#[test]
#[ignore]
fn probe() {
    let Ok(elf_path) = std::env::var("LABWIRED_PROBE_ELF") else {
        return;
    };
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let yaml = root.join("examples/ramps14-marlin/system.yaml");
    let manifest = SystemManifest::from_file(&yaml).expect("system.yaml");
    let chip = ChipDescriptor::from_file(yaml.parent().unwrap().join(&manifest.chip)).unwrap();
    let bus = SystemBus::from_config(&chip, &manifest).expect("bus");
    let image = labwired_loader::load_elf_bytes(&std::fs::read(elf_path).unwrap()).unwrap();
    let mut cpu = Avr::for_chip_name(&chip.name);
    cpu.load_program_image(&image);
    let serial = Arc::new(Mutex::new(Vec::new()));
    cpu.set_serial_sink(serial.clone());
    let mut m = Machine::new(cpu, bus);
    let mv: u16 = std::env::var("LABWIRED_PROBE_MV")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(4775);
    let spots: Vec<(u16, usize)> = std::env::var("LABWIRED_PROBE_SRAM")
        .unwrap_or_default()
        .split(',')
        .filter(|s| !s.is_empty())
        .map(|s| {
            let (a, n) = s.split_once(':').unwrap();
            (
                u16::from_str_radix(a.trim_start_matches("0x"), 16).unwrap(),
                n.parse().unwrap(),
            )
        })
        .collect();
    let secs: f64 = std::env::var("LABWIRED_PROBE_SECS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(3.0);
    let base = 0x0001_0200u64;
    for ch in 0..16u64 {
        let _ = m.bus.write_u8(base + ch * 2, (mv & 0xFF) as u8);
        let _ = m.bus.write_u8(base + ch * 2 + 1, (mv >> 8) as u8);
    }
    let mut next = 0u64;
    let end = (secs * 16e6) as u64;
    let mut adsc_set = 0u64;
    let watch: Vec<u32> = std::env::var("LABWIRED_PROBE_PCS")
        .unwrap_or_default()
        .split(',')
        .filter(|s| !s.is_empty())
        .map(|s| u32::from_str_radix(s.trim_start_matches("0x"), 16).unwrap())
        .collect();
    let mut hits = vec![0u64; watch.len()];
    // LABWIRED_PROBE_UART="72000000:G28\n;73600000:M114\n" (cycles:text).
    let mut uart: Vec<(u64, Vec<u8>)> = std::env::var("LABWIRED_PROBE_UART")
        .unwrap_or_default()
        .split(';')
        .filter(|s| !s.is_empty())
        .map(|s| {
            let (c, t) = s.split_once(':').unwrap();
            (c.parse().unwrap(), t.replace("\\n", "\n").into_bytes())
        })
        .collect();
    uart.reverse();
    let mut ring = std::collections::VecDeque::with_capacity(256);
    let mut steps = 0u64;
    while m.total_cycles < end {
        if let Some((at, _)) = uart.last() {
            if m.total_cycles >= *at {
                let (_, bytes) = uart.pop().unwrap();
                m.bus
                    .deliver_uart_rx_named("usart0", &bytes)
                    .expect("inject");
            }
        }
        if ring.len() == 256 {
            ring.pop_front();
        }
        ring.push_back((m.cpu.pc, m.cpu.sp, m.total_cycles));
        if let Err(e) = m.step() {
            eprintln!("step error at cycle {}: {e:?}", m.total_cycles);
            for (pc, sp, c) in &ring {
                eprintln!("  pc={pc:#07x} sp={sp:#06x} cyc={c}");
            }
            let sp = m.cpu.sp as usize;
            let lo = sp.saturating_sub(0x200).max(0x200);
            eprintln!(
                "stack {:#x}..: {:02x?}",
                sp,
                &m.cpu.sram[sp + 1 - 0x200..(sp + 17 - 0x200).min(m.cpu.sram.len())]
            );
            let _ = lo;
            break;
        }
        steps += 1;
        let pc_bytes = m.cpu.pc * 2;
        for (i, &w) in watch.iter().enumerate() {
            if pc_bytes == w || m.cpu.pc == w {
                hits[i] += 1;
            }
        }
        if m.cpu.adcsra & 0x40 != 0 {
            adsc_set += 1;
        }
        if m.total_cycles >= next {
            next += 16_000_000 / 4;
            let sram = &m.cpu.sram;
            let dump: Vec<String> = spots
                .iter()
                .map(|&(a, n)| {
                    let i = (a - 0x200) as usize;
                    format!("{a:#06x}={:02x?}", &sram[i..i + n])
                })
                .collect();
            eprintln!(
                "t={:.2}s pc={:#x} ADCSRA={:#04x} ADMUX={:#04x} io[ADCSRB]={:#04x} adsc_steps={adsc_set}/{steps} TIMSK0={:#04x} OCR0A={:#04x} TCCR0B={:#04x} hits={hits:?} {}",
                m.total_cycles as f64 / 16e6,
                m.cpu.pc * 2,
                m.cpu.adcsra,
                m.cpu.admux,
                m.cpu.io[0x7B - 0x20],
                m.cpu.io[0x6E - 0x20],
                m.cpu.io[0x47 - 0x20],
                m.cpu.io[0x45 - 0x20],
                dump.join(" ")
            );
        }
    }
    eprintln!(
        "serial tail: {}",
        String::from_utf8_lossy(&serial.lock().unwrap())
            .chars()
            .rev()
            .take(300)
            .collect::<String>()
            .chars()
            .rev()
            .collect::<String>()
    );
}
