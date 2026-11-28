// LabWired - Firmware Simulation Platform
// SPDX-License-Identifier: MIT

//! nRF54L15 RRAMC gates CPU stores into the RRAM array.
//!
//! Sources: Nordic `nrf54l15.svd` (vendored at
//! `tests/fixtures/real_world/nrf54l15.svd`), peripheral `GLOBAL_RRAMC_S` at
//! 0x5004_B000, and the product specification sections "Writing to RRAM" and
//! "Buffered RRAM write": RRAMC writes both 0 and 1 to any bit, a store needs
//! CONFIG.WEN, and BUFSTATUS.WRITEBUFEMPTY reports uncommitted buffer data.
//!
//! The firmware test runs unmodified Zephyr v4.4.0
//! `samples/drivers/soc_flash_nrf` built for `nrf54l15dk/nrf54l15/cpuapp`
//! (fixture `nrf54l15-zephyr-soc-flash.elf`). Its `soc_flash_nrf_rram.c`
//! driver sets WEN, copies into RRAM, checks WRITEBUFEMPTY, triggers
//! COMMITWRITEBUF and clears WEN. The same image also needs two fixes outside
//! RRAMC to reach `main`: the CLOCK IRQ for an event latched before INTENSET
//! (nrfx `nrfx_clock_lfclk_start` order), and a mapped SPIM00 (the DK
//! devicetree enables it for the on-board SPI-NOR).

use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::bus::SystemBus;
use labwired_core::system::cortex_m::configure_cortex_m;
use labwired_core::{Bus, Machine};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

const RRAMC: u64 = 0x5004_B000;
const TASKS_COMMITWRITEBUF: u64 = 0x008;
const EVENTS_READY: u64 = 0x104;
const READY: u64 = 0x400;
const READYNEXT: u64 = 0x404;
const WRITEBUFEMPTY: u64 = 0x418;
const CONFIG: u64 = 0x500;
const CONFIG_WEN: u32 = 1;
/// CONFIG.WRITEBUFSIZE = 1 line (SVD field at bits 8..13).
const CONFIG_ONE_LINE_BUFFER: u32 = 1 << 8;

/// `storage_partition` of the DK devicetree; the sample writes here.
const TEST_PARTITION: u64 = 0x0017_4000;
/// A word in RRAM clear of any loaded image.
const SCRATCH: u64 = 0x0010_0000;

fn root(rel: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(rel)
}

fn nrf54l15_bus() -> SystemBus {
    let sys_path = root("configs/systems/nrf54l15dk.yaml");
    let mut manifest = SystemManifest::from_file(&sys_path).expect("load nrf54l15dk.yaml");
    manifest.chip = sys_path
        .parent()
        .unwrap()
        .join(&manifest.chip)
        .to_str()
        .unwrap()
        .to_string();
    let chip = ChipDescriptor::from_file(&manifest.chip).expect("load nrf54l15 chip");
    SystemBus::from_config(&chip, &manifest).expect("assemble nrf54l15 bus")
}

#[test]
fn idle_rramc_reads_ready_not_busy() {
    let bus = nrf54l15_bus();
    // nrfx spins on READY; a zero-filled window reports "RRAMC is busy" forever.
    assert_eq!(bus.read_u32(RRAMC + READY).unwrap(), 1);
    assert_eq!(bus.read_u32(RRAMC + READYNEXT).unwrap(), 1);
    assert_eq!(bus.read_u32(RRAMC + WRITEBUFEMPTY).unwrap(), 1);
    assert_eq!(bus.read_u32(RRAMC + CONFIG).unwrap(), 0);
}

#[test]
fn wen_gates_rram_stores_and_overwrite_needs_no_erase() {
    let mut bus = nrf54l15_bus();
    let erased = bus.read_u32(SCRATCH).unwrap();

    // WEN clear: the store does not reach the array.
    bus.write_u32(SCRATCH, 0x1234_5678).unwrap();
    assert_eq!(bus.read_u32(SCRATCH).unwrap(), erased);
    bus.write_u8(SCRATCH, 0x5A).unwrap();
    assert_eq!(bus.read_u32(SCRATCH).unwrap(), erased);

    // WEN set: the store commits as written.
    bus.write_u32(RRAMC + CONFIG, CONFIG_WEN).unwrap();
    bus.write_u32(SCRATCH, 0x1234_5678).unwrap();
    assert_eq!(bus.read_u32(SCRATCH).unwrap(), 0x1234_5678);

    // RRAM writes 0 and 1 alike: no erase, no NOR 1→0 rule.
    bus.write_u32(SCRATCH, 0x0000_0000).unwrap();
    assert_eq!(bus.read_u32(SCRATCH).unwrap(), 0);
    bus.write_u32(SCRATCH, 0xFFFF_FFFF).unwrap();
    assert_eq!(bus.read_u32(SCRATCH).unwrap(), 0xFFFF_FFFF);
    bus.write_u8(SCRATCH + 1, 0xA5).unwrap();
    assert_eq!(bus.read_u32(SCRATCH).unwrap(), 0xFFFF_A5FF);

    // WEN cleared again: the array is read-only.
    bus.write_u32(RRAMC + CONFIG, 0).unwrap();
    bus.write_u32(SCRATCH, 0).unwrap();
    assert_eq!(bus.read_u32(SCRATCH).unwrap(), 0xFFFF_A5FF);
}

#[test]
fn buffered_store_reports_uncommitted_data_until_committed() {
    let mut bus = nrf54l15_bus();
    bus.write_u32(RRAMC + CONFIG, CONFIG_WEN | CONFIG_ONE_LINE_BUFFER)
        .unwrap();
    assert_eq!(
        bus.read_u32(RRAMC + CONFIG).unwrap(),
        CONFIG_WEN | CONFIG_ONE_LINE_BUFFER
    );

    // A partial line stays in the buffer.
    bus.write_u32(SCRATCH, 0xCAFE_F00D).unwrap();
    assert_eq!(bus.read_u32(RRAMC + WRITEBUFEMPTY).unwrap(), 0);
    assert_eq!(bus.read_u32(RRAMC + EVENTS_READY).unwrap(), 0);

    bus.write_u32(RRAMC + TASKS_COMMITWRITEBUF, 1).unwrap();
    assert_eq!(bus.read_u32(RRAMC + WRITEBUFEMPTY).unwrap(), 1);
    assert_eq!(bus.read_u32(RRAMC + EVENTS_READY).unwrap(), 1);
    assert_eq!(bus.read_u32(SCRATCH).unwrap(), 0xCAFE_F00D);

    // A full 128-bit line commits without the task.
    for word in 0..4 {
        bus.write_u32(SCRATCH + 0x10 + 4 * word, word as u32)
            .unwrap();
    }
    assert_eq!(bus.read_u32(RRAMC + WRITEBUFEMPTY).unwrap(), 1);
}

#[test]
fn zephyr_soc_flash_sample_writes_and_reads_back_rram() {
    let firmware_path = root("tests/fixtures/nrf54l15-zephyr-soc-flash.elf");
    let sys_path = root("configs/systems/nrf54l15dk.yaml");
    let mut manifest = SystemManifest::from_file(&sys_path).expect("load nrf54l15dk.yaml");
    manifest.chip = sys_path
        .parent()
        .unwrap()
        .join(&manifest.chip)
        .to_str()
        .unwrap()
        .to_string();
    let chip = ChipDescriptor::from_file(&manifest.chip).expect("load nrf54l15 chip");
    let mut bus = SystemBus::from_config(&chip, &manifest).expect("assemble nrf54l15 bus");
    let uart_sink = Arc::new(Mutex::new(Vec::new()));
    bus.attach_uart_tx_sink(uart_sink.clone(), false);

    let (cpu, _nvic) = configure_cortex_m(&mut bus);
    let mut machine = Machine::new(cpu, bus);
    let image = labwired_loader::load_elf(&firmware_path).expect("load soc_flash ELF");
    machine.load_firmware(&image).expect("load firmware");

    let finished = |sink: &Arc<Mutex<Vec<u8>>>| {
        String::from_utf8_lossy(&sink.lock().unwrap()).contains("Finished!")
    };
    let mut steps = 0u32;
    while steps < 5_000_000 && !finished(&uart_sink) {
        for _ in 0..50_000 {
            machine.step().expect("simulation step faulted");
        }
        steps += 50_000;
    }

    let uart = uart_sink.lock().unwrap().clone();
    let text = String::from_utf8_lossy(&uart);
    assert!(text.contains("Finished!"), "sample did not finish:\n{text}");
    // Test 2 writes 128 bytes as eight 16-byte blocks and reads each back.
    assert_eq!(
        text.matches("Writing 16 data bytes to 0x1740").count(),
        8,
        "expected eight block writes:\n{text}"
    );
    assert_eq!(
        text.matches("Data read matches data written. Good!")
            .count(),
        1,
        "expected one write/read-back pass:\n{text}"
    );
    for bad in ["failed", "does not match", "not ready", "ERROR", "Error"] {
        assert!(!text.contains(bad), "sample reported `{bad}`:\n{text}");
    }
    // The array holds the sample's incrementing pattern (RRAM needs no erase,
    // so Tests 3 and 4 leave it in place).
    for i in 0..128u64 {
        assert_eq!(
            machine.bus.read_u8(TEST_PARTITION + i).unwrap(),
            i as u8,
            "RRAM byte {i} of the test partition"
        );
    }
    // The driver leaves the controller with write disabled and nothing
    // waiting in the write-buffer.
    assert_eq!(
        machine.bus.read_u32(RRAMC + CONFIG).unwrap() & CONFIG_WEN,
        0
    );
    assert_eq!(machine.bus.read_u32(RRAMC + WRITEBUFEMPTY).unwrap(), 1);
}
