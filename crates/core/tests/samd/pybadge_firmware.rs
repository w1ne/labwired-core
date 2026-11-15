// SPDX-License-Identifier: MIT

//! Machine-run proof for the PyBadge io-smoke firmware
//! (`crates/firmware-atsamd51-pybadge-demo`, fixture
//! `tests/fixtures/atsamd51-pybadge-smoke.elf`): the twin boots the image from
//! 0x4000, the Feather UART prints `OK`, and the red D13 LED (PA23) is driven
//! as an output and toggles.

use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::{bus::SystemBus, cpu::cortex_m::CortexM, Bus, Machine};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

fn root(path: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(path)
}

const PORTA_DIR: u64 = 0x4100_8000;
const PORTA_OUT: u64 = 0x4100_8010;
const LED_D13: u32 = 1 << 23;

#[test]
fn pybadge_firmware_prints_ok_and_toggles_d13() {
    let chip = ChipDescriptor::from_file(root("configs/chips/atsamd51-pybadge.yaml")).unwrap();
    let manifest = SystemManifest::from_file(root("configs/systems/pybadge.yaml")).unwrap();
    let mut bus = SystemBus::from_config(&chip, &manifest).expect("PyBadge bus builds");
    let uart = Arc::new(Mutex::new(Vec::new()));
    bus.attach_uart_tx_sink(uart.clone(), false);

    let mut machine = Machine::new(CortexM::new(), bus);
    let image = labwired_loader::load_elf(&root("tests/fixtures/atsamd51-pybadge-smoke.elf"))
        .expect("load PyBadge fixture");
    machine.load_firmware(&image).expect("load firmware");
    // Reset vectors come from the 0x4000 application table, not address 0.
    assert!(machine.cpu.pc >= 0x4000, "pc {:#x}", machine.cpu.pc);

    let (mut high, mut low, mut edges) = (false, false, 0u32);
    let mut prev: Option<bool> = None;
    for _ in 0..100_000 {
        machine.step().expect("step");
        let out = machine.bus.read_u32(PORTA_OUT).unwrap();
        let level = out & LED_D13 != 0;
        if level {
            high = true;
        } else {
            low = true;
        }
        if prev.is_some_and(|p| p != level) {
            edges += 1;
        }
        prev = Some(level);
    }

    let dir = machine.bus.read_u32(PORTA_DIR).unwrap();
    assert!(dir & LED_D13 != 0, "PA23 must be configured as output");
    assert!(
        high && low && edges >= 4,
        "D13 did not toggle: edges={edges}"
    );
    let text = String::from_utf8_lossy(&uart.lock().unwrap()).to_string();
    assert!(text.contains("OK\n"), "uart: {text:?}");
}
