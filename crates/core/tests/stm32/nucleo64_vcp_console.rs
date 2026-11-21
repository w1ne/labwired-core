// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! NUCLEO console tap: the ST-LINK virtual COM port carries one USART, so each
//! board manifest names it as `debug_uart`. Without that line every run path
//! fell back to one capture buffer on all UARTs, and firmware that also drove
//! USART1 (Arduino `Serial1`) spliced that traffic into the Serial pane.
//!
//! Board facts: NUCLEO-64 (UM1724 §6.8, STM32duino `SERIAL_UART_INSTANCE 2`)
//! routes the VCP to USART2 PA2/PA3 on F103RB, F401RE, F411RE and F446RE; on
//! these parts no other U(S)ART reaches PA2/PA3. NUCLEO-C031C6 (UM2953)
//! routes it to USART2 PA2/PA3; the C031 has no LPUART.
//!
//! Firmware: tests/fixtures/nucleo-vcp-console/vcp-console.c (register-level;
//! USART2 prints `VCP_BOOT` / `VCP_DONE`, USART1 sends `AUX_LINK_DATA` one
//! byte at a time between the two console bytes).

use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::bus::SystemBus;
use labwired_core::console::HostConsole;
use labwired_core::system::cortex_m::configure_cortex_m;
use labwired_core::Machine;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

const CONSOLE: &str = "VCP_BOOT\r\nVCP_DONE\r\n";

/// Board manifest, the fixture it runs, and the VCP USART base address.
const RUNNABLE: &[(&str, &str, u64)] = &[
    ("nucleo-f103rb", "vcp-console-f103.elf", 0x4000_4400),
    ("nucleo-f103rb-epaper", "vcp-console-f103.elf", 0x4000_4400),
    ("nucleo-f401re", "vcp-console-f4.elf", 0x4000_4400),
    ("nucleo-f411re", "vcp-console-f4.elf", 0x4000_4400),
    ("nucleo-f446re", "vcp-console-f4.elf", 0x4000_4400),
];

/// Boards whose USART is the v2 register layout: the fixture does not run on
/// them, so only the declaration is checked.
const DECLARED_ONLY: &[(&str, u64)] = &[("nucleo-c031c6", 0x4000_4400)];

fn root(rel: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(rel)
}

fn load(board: &str) -> (SystemManifest, ChipDescriptor) {
    let sys_path = root(&format!("configs/systems/{board}.yaml"));
    let manifest = SystemManifest::from_file(&sys_path).expect("load system yaml");
    let chip = ChipDescriptor::from_file(sys_path.parent().unwrap().join(&manifest.chip))
        .expect("load chip");
    (manifest, chip)
}

/// Run the fixture on `board` with `attach` choosing what feeds the capture.
fn run(
    board: &str,
    elf: &str,
    until: &str,
    attach: impl FnOnce(&mut SystemBus, &SystemManifest, Arc<Mutex<Vec<u8>>>),
) -> String {
    let (manifest, chip) = load(board);
    let mut bus = SystemBus::from_config(&chip, &manifest).expect("build bus");
    let sink = Arc::new(Mutex::new(Vec::new()));
    attach(&mut bus, &manifest, sink.clone());
    let (cpu, _) = configure_cortex_m(&mut bus);
    let mut machine = Machine::new(cpu, bus);
    let image = labwired_loader::load_elf(
        &PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join(format!("tests/fixtures/nucleo-vcp-console/{elf}")),
    )
    .expect("missing fixture; run tests/fixtures/nucleo-vcp-console/build.sh");
    machine.load_firmware(&image).expect("load firmware");
    for _ in 0..2_000_000u32 {
        machine.step().expect("step");
        if String::from_utf8_lossy(&sink.lock().unwrap()).contains(until) {
            break;
        }
    }
    let text = String::from_utf8_lossy(&sink.lock().unwrap()).into_owned();
    text
}

fn declared_base(manifest: &SystemManifest, chip: &ChipDescriptor) -> Option<u64> {
    let name = manifest.debug_uart.as_deref()?;
    chip.peripherals
        .iter()
        .find(|p| p.id == name)
        .map(|p| p.base_address)
}

/// Each manifest names the UART at the VCP address, and the bus resolves it.
#[test]
fn every_nucleo_manifest_names_its_vcp_usart() {
    let boards = RUNNABLE
        .iter()
        .map(|(b, _, base)| (*b, *base))
        .chain(DECLARED_ONLY.iter().copied());
    for (board, base) in boards {
        let (manifest, chip) = load(board);
        assert_eq!(
            declared_base(&manifest, &chip),
            Some(base),
            "{board}: debug_uart {:?} is not the VCP USART at {base:#x}",
            manifest.debug_uart
        );
        let mut bus = SystemBus::from_config(&chip, &manifest).expect("build bus");
        bus.attach_host_console(
            &HostConsole::from_manifest(&manifest),
            Arc::new(Mutex::new(Vec::new())),
        )
        .unwrap_or_else(|e| panic!("{board}: {e}"));
    }
}

/// The run path: the declared console holds USART2 only, line by line.
#[test]
fn console_capture_holds_the_vcp_usart_only() {
    for (board, elf, _) in RUNNABLE {
        let text = run(board, elf, "VCP_DONE\r\n", |bus, manifest, sink| {
            bus.attach_host_console(&HostConsole::from_manifest(manifest), sink)
                .expect("attach console");
        });
        assert_eq!(text, CONSOLE, "{board}");
    }
}

/// USART1 still transmits: its bytes reach its own window, not the console.
#[test]
fn serial1_bytes_reach_usart1() {
    for (board, elf, _) in RUNNABLE {
        let (_, chip) = load(board);
        let usart1 = chip
            .peripherals
            .iter()
            .find(|p| p.r#type == "uart" && matches!(p.base_address, 0x4001_3800 | 0x4001_1000))
            .map(|p| p.id.clone())
            .expect("usart1");
        let text = run(board, elf, "AUX_LINK_DATA\r\n", |bus, _, sink| {
            assert!(bus.attach_uart_tx_sink_named(&usart1, sink, false));
        });
        assert_eq!(text, "AUX_LINK_DATA\r\n", "{board}");
    }
}

/// Control: the all-UART fallback these boards used before `debug_uart`
/// splices USART1 into the console, so `VCP_DONE` never comes out whole.
#[test]
fn all_uart_fallback_interleaves_serial1_into_the_console() {
    for (board, elf, _) in RUNNABLE {
        let text = run(board, elf, "AUX_LINK_DATA", |bus, _, sink| {
            bus.attach_uart_tx_sink(sink, false);
        });
        assert!(text.starts_with("VCP_BOOT\r\n"), "{board}: {text:?}");
        assert!(text.contains('X'), "{board}: {text:?}");
        assert!(!text.contains("VCP_DONE"), "{board}: {text:?}");
    }
}
