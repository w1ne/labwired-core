// LabWired - Firmware Simulation Platform
// SPDX-License-Identifier: MIT

//! earlephilhower `Serial` (USB CDC) on RP2350 must be observed by the twin.
//!
//! Fixture: `tests/fixtures/rp2350-pico2w-usb-cdc-serial.elf` — sketch
//! ```c
//! void setup() { Serial.begin(115200); }
//! void loop() {
//!   if (Serial) { Serial.println("RP2350_USB_CDC_OK"); delay(50); }
//! }
//! ```
//! Built with arduino-cli `--fqbn rp2040:rp2040:rpipico2` (arduino-pico 6.3.2;
//! Pico 2 non-W so CYW43 does not gate boot). Source under
//! `tests/fixtures/src/rp2350-pico2w-usb-cdc-serial.ino`.
//!
//! The chip YAML already wires `type: rp2040_usb` on USBCTRL_IRQ 14. This test
//! is the gate that TinyUSB CDC enumeration against the simulated host actually
//! delivers `Serial` bytes into the UART capture sink (same path as RP2040
//! Mbed CDC). Print from `loop()` after `Serial` (DTR) is up — a single
//! `println` in `setup()` races the host attach/enum window.

mod common;
use common::root;

use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::system::cortex_m::configure_cortex_m;
use labwired_core::{Bus, Cpu, Machine};
use std::sync::{Arc, Mutex};

const FIXTURE: &str = "tests/fixtures/rp2350-pico2w-usb-cdc-serial.elf";
const STEP_BUDGET: u32 = 80_000_000;

static ENV_LOCK: Mutex<()> = Mutex::new(());

fn load() -> (ChipDescriptor, SystemManifest) {
    let chip_path = root("configs/chips/rp2350.yaml");
    let chip = ChipDescriptor::from_file(&chip_path).expect("rp2350 chip");
    let mut manifest =
        SystemManifest::from_file(root("configs/systems/pico2.yaml")).expect("system");
    manifest.chip = chip_path.to_str().unwrap().to_string();
    (chip, manifest)
}

#[test]
fn serial_println_observed_over_usb_cdc() {
    let _lock = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    std::env::remove_var("LABWIRED_RP2350_BOOTROM");

    let path = root(FIXTURE);
    assert!(path.is_file(), "fixture missing: {}", path.display());
    let (chip, manifest) = load();
    let mut bus = labwired_core::bus::SystemBus::from_config(&chip, &manifest).expect("bus");
    let uart = Arc::new(Mutex::new(Vec::new()));
    bus.attach_uart_tx_sink(uart.clone(), false);
    let (cpu, _nvic) = configure_cortex_m(&mut bus);
    let mut m = Machine::new(cpu, bus);
    let image = labwired_loader::load_elf(&path).expect("load elf");
    m.load_firmware(&image).expect("load firmware");

    let mut steps = 0u32;
    while steps < STEP_BUDGET {
        if m.step().is_err() {
            break;
        }
        steps += 1;
        if steps.is_multiple_of(500_000) {
            let guard = uart.lock().unwrap();
            if !guard.is_empty() {
                let text = String::from_utf8_lossy(&guard);
                if text.contains("RP2350_USB_CDC_OK") {
                    return;
                }
            }
        }
    }
    let text = String::from_utf8_lossy(&uart.lock().unwrap()).into_owned();
    let cfsr = m.bus.read_u32(0xE000_ED28).unwrap_or(0);
    panic!(
        "Serial USB CDC banner missing after {steps} steps (PC={:#x}, CFSR={:#x}): {text:?}",
        m.cpu.get_pc(),
        cfsr
    );
}
