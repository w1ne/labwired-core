use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::Machine;
use labwired_loader::load_elf;
use std::path::PathBuf;

#[test]
fn test_demo_blinky_gpio_toggle() {
    // Load the demo-blinky firmware
    let firmware_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../target/thumbv7m-none-eabi/release/demo-blinky");

    if !firmware_path.exists() {
        // Skip if not built, but ideally it should be built by pre-test task
        // For local development it's better to fail and inform
        panic!(
            "Firmware not found at {:?}. Run 'cargo build -p demo-blinky --release' first.",
            firmware_path
        );
    }

    let program = load_elf(&firmware_path).expect("Failed to load ELF");

    // Create machine with STM32F103 configuration
    let system_path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../system.yaml");

    let manifest = SystemManifest::from_file(&system_path).expect("Failed to load system manifest");

    let chip_path = system_path.parent().unwrap().join(&manifest.chip);

    let chip = ChipDescriptor::from_file(&chip_path).expect("Failed to load chip descriptor");

    let mut bus =
        labwired_core::bus::SystemBus::from_config(&chip, &manifest).expect("Failed to build bus");

    let (cpu, _nvic) = labwired_core::system::cortex_m::configure_cortex_m(&mut bus);
    let mut machine = Machine::new(cpu, bus);

    machine
        .load_firmware(&program)
        .expect("Failed to load firmware");

    // Run for enough cycles to see GPIO toggles
    let mut odr_values = Vec::new();

    // We want to detect writes to GPIOC_ODR (PC13 is the LED)
    for _ in 0..5_000_000 {
        machine.step().expect("Execution failed");

        // Peek at GPIOC state
        if let Some(gpio_val) = machine.peek_peripheral("gpioc") {
            // In the snapshot, fields are lower-case
            if let Some(odr) = gpio_val.get("odr").and_then(|v| v.as_u64()) {
                let odr_u32 = odr as u32;
                if odr_values.last() != Some(&odr_u32) {
                    odr_values.push(odr_u32);
                    // tracing::info!("ODR Changed: {:#x}", odr_u32);
                }
            }
        }

        // Stop early if we have enough toggles
        if odr_values.len() >= 3 {
            break;
        }
    }

    // PC13 is bit 13 (0x2000)
    // We expect at least one toggle
    assert!(
        odr_values.len() > 1,
        "Expected at least one LED state change, but got sequence: {:?}",
        odr_values
    );

    // Verify that bit 13 actually changed
    let bit_13_states: Vec<bool> = odr_values.iter().map(|&v| (v & 0x2000) != 0).collect();
    let mut changes = 0;
    for i in 0..bit_13_states.len() - 1 {
        if bit_13_states[i] != bit_13_states[i + 1] {
            changes += 1;
        }
    }

    assert!(
        changes >= 1,
        "PC13 (LED) did not toggle. ODR log: {:?}",
        odr_values
    );
    println!(
        "SUCCESS: Detected PC13 toggled {} times. ODR sequence: {:?}",
        changes, odr_values
    );
}
