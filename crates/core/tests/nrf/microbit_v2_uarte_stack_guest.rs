// SPDX-License-Identifier: MIT
//! Source-built ARM guest that returns from putc and reuses its transmit slot.
use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::bus::SystemBus;
use labwired_core::cpu::CortexM;
use labwired_core::system::cortex_m::configure_cortex_m;
use labwired_core::{Bus, DebugControl, Machine};
use sha2::{Digest, Sha256};
use std::path::PathBuf;
use std::process::Command;
use std::sync::{Arc, Mutex};

const STATE: u64 = 0x2000_0100;
const EXPECTED: &[u8] = b"A10 stack reuse OK\n";

#[test]
fn compiled_guest_retains_serial_after_return_and_stack_reuse() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf();
    let elf = std::env::temp_dir().join(format!("microbit-uarte-stack-{}.elf", std::process::id()));
    assert!(Command::new("arm-none-eabi-gcc")
        .current_dir(root.join("examples/microbit-v2"))
        .args([
            "-mcpu=cortex-m4",
            "-mthumb",
            "-nostdlib",
            "-Wl,-T,board-io.ld",
            "-o"
        ])
        .arg(&elf)
        .arg("uarte-stack-buffer.S")
        .status()
        .expect("ARM GCC required for executable serial proof")
        .success());
    let payload = std::fs::read(&elf).unwrap();
    let hash = format!("{:x}", Sha256::digest(&payload));
    let firmware = labwired_loader::load_elf(&elf).unwrap();
    std::fs::remove_file(&elf).unwrap();
    if let Ok(directory) = std::env::var("LABWIRED_GUEST_ARTIFACT_DIR") {
        std::fs::create_dir_all(&directory).unwrap();
        std::fs::write(
            PathBuf::from(&directory).join("microbit-uarte-stack.elf"),
            &payload,
        )
        .unwrap();
    }
    println!("MICROBIT_UARTE_STACK_GUEST_SHA256={hash}");

    // Keep the feature selection in CI, rather than adding a conditional-
    // compilation site. The feature-off walk uses interval 1; the scheduler
    // additionally exercises 64. Refuse empty or unrecognised test selections.
    let intervals: &[u32] = match std::env::var("LABWIRED_UARTE_STACK_INTERVALS").as_deref() {
        Ok("1") => &[1],
        Ok("1,64") | Err(std::env::VarError::NotPresent) => &[1, 64],
        other => panic!("invalid UARTE guest interval selection: {other:?}"),
    };
    for &interval in intervals {
        let system = root.join("configs/systems/microbit-v2.yaml");
        let mut manifest = SystemManifest::from_file(&system).unwrap();
        let chip_path = system.parent().unwrap().join(&manifest.chip);
        let chip = ChipDescriptor::from_file(&chip_path).unwrap();
        manifest.chip = chip_path.to_str().unwrap().to_string();
        let mut bus = SystemBus::from_config(&chip, &manifest).unwrap();
        let serial = Arc::new(Mutex::new(Vec::new()));
        bus.attach_uart_tx_sink(serial.clone(), false);
        let (cpu, _) = configure_cortex_m(&mut bus);
        let mut machine: Machine<CortexM> = Machine::new(cpu, bus);
        machine.config.peripheral_tick_interval = interval;
        machine.bus.config.peripheral_tick_interval = interval;
        machine.load_firmware(&firmware).unwrap();
        for _ in 0..100 {
            machine.run(Some(1_000)).unwrap();
            if machine.bus.read_u32(STATE).unwrap() != 0 {
                break;
            }
        }
        assert_eq!(
            machine.bus.read_u32(STATE).unwrap(),
            1,
            "guest must finish without timeout, interval={interval}"
        );
        assert_eq!(
            machine.bus.read_u32(STATE + 4).unwrap(),
            EXPECTED.len() as u32
        );
        assert_eq!(
            machine.bus.read_u32(STATE + 8).unwrap(),
            EXPECTED.len() as u32,
            "every transmitted stack slot was actually reused"
        );
        let bytes = serial.lock().unwrap().clone();
        assert_eq!(
            bytes, EXPECTED,
            "exact ordered guest serial, interval={interval}"
        );
        println!(
            "MICROBIT_UARTE_STACK interval={interval} serial_hex={} reused={}",
            bytes
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect::<String>(),
            EXPECTED.len()
        );
    }
}

/// The IRQ starts a new DMA byte on ENDTX before inspecting TXSTOPPED.
/// This distinguishes a completed buffer from an explicitly stopped transmitter.
#[test]
fn compiled_guest_chains_endtx_without_an_unsolicited_stop() {
    const EXPECTED_IRQ: &[u8] = b"A10 IRQ chain OK\n";
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf();
    let elf = std::env::temp_dir().join(format!("microbit-uarte-irq-{}.elf", std::process::id()));
    assert!(Command::new("arm-none-eabi-gcc")
        .current_dir(root.join("examples/microbit-v2"))
        .args([
            "-mcpu=cortex-m4",
            "-mthumb",
            "-nostdlib",
            "-Wl,-T,board-io.ld",
            "-o"
        ])
        .arg(&elf)
        .arg("uarte-irq-chain.S")
        .status()
        .expect("ARM GCC required for executable IRQ proof")
        .success());
    let payload = std::fs::read(&elf).unwrap();
    let firmware = labwired_loader::load_elf(&elf).unwrap();
    std::fs::remove_file(&elf).unwrap();
    println!(
        "MICROBIT_UARTE_IRQ_GUEST_SHA256={:x}",
        Sha256::digest(&payload)
    );
    if let Ok(directory) = std::env::var("LABWIRED_GUEST_ARTIFACT_DIR") {
        std::fs::create_dir_all(&directory).unwrap();
        std::fs::write(
            PathBuf::from(directory).join("microbit-uarte-irq.elf"),
            &payload,
        )
        .unwrap();
    }
    let intervals: &[u32] = match std::env::var("LABWIRED_UARTE_STACK_INTERVALS").as_deref() {
        Ok("1") => &[1],
        Ok("1,64") => &[1, 64],
        Ok("1,64,1024") | Err(std::env::VarError::NotPresent) => &[1, 64, 1024],
        other => panic!("invalid UARTE guest interval selection: {other:?}"),
    };
    for &interval in intervals {
        let system = root.join("configs/systems/microbit-v2.yaml");
        let mut manifest = SystemManifest::from_file(&system).unwrap();
        let chip_path = system.parent().unwrap().join(&manifest.chip);
        let chip = ChipDescriptor::from_file(&chip_path).unwrap();
        manifest.chip = chip_path.to_str().unwrap().to_string();
        let mut bus = SystemBus::from_config(&chip, &manifest).unwrap();
        let serial = Arc::new(Mutex::new(Vec::new()));
        bus.attach_uart_tx_sink(serial.clone(), false);
        let (cpu, _) = configure_cortex_m(&mut bus);
        let mut machine: Machine<CortexM> = Machine::new(cpu, bus);
        machine.config.peripheral_tick_interval = interval;
        machine.bus.config.peripheral_tick_interval = interval;
        machine.load_firmware(&firmware).unwrap();
        for _ in 0..200 {
            machine.run(Some(1_000)).unwrap();
            if machine.bus.read_u32(STATE).unwrap() != 0 {
                break;
            }
        }
        let state: Vec<_> = (0..6)
            .map(|i| machine.bus.read_u32(STATE + i * 4).unwrap())
            .collect();
        assert_eq!(
            state,
            vec![
                1,
                EXPECTED_IRQ.len() as u32,
                0,
                0,
                EXPECTED_IRQ.len() as u32,
                1
            ],
            "IRQ chain must finish only after its explicit stop, interval={interval}"
        );
        let bytes = serial.lock().unwrap().clone();
        assert_eq!(
            bytes, EXPECTED_IRQ,
            "exact ordered IRQ serial, interval={interval}"
        );
        println!("MICROBIT_UARTE_IRQ interval={interval} serial_hex={} sent={} unsolicited_stops=0 explicit_stop=1",
            bytes.iter().map(|b| format!("{b:02x}")).collect::<String>(), EXPECTED_IRQ.len());
    }
}
