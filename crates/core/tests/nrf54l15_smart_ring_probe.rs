// LabWired - Firmware Simulation Platform
// SPDX-License-Identifier: MIT

//! Integration test: the bare-metal nRF54L15 smart-ring firmware genuinely
//! drives the four I²C sensors over TWIM21 in-sim.
//!
//! This boots the real `nrf54l15-smart-ring.elf` fixture against
//! `configs/systems/smart-ring.yaml` (which attaches BMI270/MAX30102/TMP117/
//! DRV2605 as `external_devices` on TWIM21) and asserts that the firmware's
//! WHO_AM_I read of EACH sensor returns that model's real identity byte(s):
//!
//!   BMI270   (0x68) CHIP_ID   0x00 -> 0x24
//!   MAX30102 (0x57) PART_ID   0xFF -> 0x15
//!   TMP117   (0x48) DEVICE_ID 0x0F -> 0x0117 (16-bit big-endian)
//!   DRV2605  (0x5A) STATUS    0x00 -> 0xE0   (DEVICE_ID = 7 in bits[7:5])
//!
//! Those bytes are produced only by the sensor MODELS answering real EasyDMA
//! I²C transactions — the firmware seeds its RX buffer with 0xEE, so a byte
//! that comes back matching the datasheet is proof the transaction reached the
//! modelled slave (not a stub, not the firmware echoing a constant).
//!
//! The firmware itself stamps `[OK]`/`[MISMATCH]` and `ack=Y`/`ack=N` per line;
//! this test additionally asserts, from the outside, that the returned bytes
//! are correct and that nothing NACKed or mismatched.

use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::bus::SystemBus;
use labwired_core::system::cortex_m::configure_cortex_m;
use labwired_core::Machine;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf()
}

/// Boot the fixture against smart-ring.yaml and return the captured UART bytes.
fn run_and_capture_uart() -> Vec<u8> {
    let root = workspace_root();
    let firmware_path = root.join("tests/fixtures/nrf54l15-smart-ring.elf");
    assert!(
        firmware_path.exists(),
        "firmware fixture not found: {firmware_path:?} — run `make publish` in \
         examples/nrf54l15-smart-ring"
    );

    let sys_path = root.join("configs/systems/smart-ring.yaml");
    let mut manifest =
        SystemManifest::from_file(&sys_path).expect("failed to load smart-ring.yaml");
    // Resolve the chip path relative to the system manifest.
    manifest.chip = sys_path
        .parent()
        .unwrap()
        .join(&manifest.chip)
        .to_str()
        .unwrap()
        .to_string();
    let chip = ChipDescriptor::from_file(&manifest.chip).expect("failed to load nrf54l15 chip");

    let mut bus =
        SystemBus::from_config(&chip, &manifest).expect("failed to build SystemBus from config");
    let uart_sink = Arc::new(Mutex::new(Vec::new()));
    bus.attach_uart_tx_sink(uart_sink.clone(), false);

    let (cpu, _nvic) = configure_cortex_m(&mut bus);
    let mut machine = Machine::new(cpu, bus);

    let image = labwired_loader::load_elf(&firmware_path)
        .unwrap_or_else(|e| panic!("failed to load ELF {firmware_path:?}: {e}"));
    machine
        .load_firmware(&image)
        .expect("failed to load firmware into machine");

    // The probe of all four sensors + banner completes well within this budget;
    // the firmware then spins in an idle loop.
    for _ in 0..300_000 {
        machine.step().expect("simulation step faulted");
    }

    let bytes = uart_sink.lock().unwrap().clone();
    bytes
}

#[test]
fn smart_ring_firmware_reads_all_four_sensor_ids_over_i2c() {
    let uart = run_and_capture_uart();
    let text = String::from_utf8_lossy(&uart);
    let text = text.as_ref();

    // Each line carries the ACTUAL byte(s) read back over TWIM. Asserting the
    // full `-> id=... ack=Y [OK]` tail proves the model responded AND matched.
    let expected_lines = [
        ("BMI270", "imu    BMI270    addr=0x68 reg=0x00 -> id=0x24 ack=Y [OK]"),
        ("MAX30102", "ppg    MAX30102  addr=0x57 reg=0xff -> id=0x15 ack=Y [OK]"),
        ("TMP117", "temp   TMP117   addr=0x48 reg=0x0f -> id=0x0117 ack=Y [OK]"),
        ("DRV2605", "haptic DRV2605   addr=0x5a reg=0x00 -> id=0xe0 ack=Y [OK]"),
    ];

    for (sensor, line) in expected_lines {
        assert!(
            text.contains(line),
            "{sensor}: expected TWIM read line not found.\n\
             expected substring: {line:?}\n\
             full UART output:\n{text}"
        );
    }

    // Nothing may NACK or mismatch: every sensor must have genuinely responded.
    assert!(
        !text.contains("[MISMATCH]"),
        "a sensor returned the wrong ID:\n{text}"
    );
    assert!(
        !text.contains("ack=N"),
        "a sensor NACKed its address (not attached / wrong bus):\n{text}"
    );
    // And the sentinel must never survive: 0xEE would mean the model never
    // wrote the RX buffer, i.e. no real transaction reached the slave.
    assert!(
        !text.contains("id=0xee"),
        "an RX buffer kept its 0xEE sentinel — the model did not respond:\n{text}"
    );

    assert!(
        text.contains("probe done"),
        "firmware did not reach the end of the probe sequence:\n{text}"
    );
}
