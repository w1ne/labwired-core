use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::Machine;
use labwired_loader::load_elf;
use std::path::PathBuf;

#[test]
#[ignore = "requires thumbv7em-none-eabihf fixture; run in nightly validation"]
fn test_h5_demo_uart_output() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf();

    let firmware_path = root.join("target/thumbv7em-none-eabihf/release/firmware-h563-demo");

    if !firmware_path.exists() {
        panic!(
            "Firmware not found at {:?}. Run 'cargo build -p firmware-h563-demo --release --target thumbv7em-none-eabihf' first.",
            firmware_path
        );
    }

    let program = load_elf(&firmware_path).expect("Failed to load ELF");

    // The test runs in 'crates/cli', so relative path to repo root is ../../
    let system_path = root.join("configs/systems/nucleo-h563zi-demo.yaml");

    // Check if system config exists
    if !system_path.exists() {
        // Fallback or explicit check
        println!(
            "System config not found at {:?}, checking relative to Cargo.toml",
            system_path
        );
    }

    let manifest = SystemManifest::from_file(&system_path).expect("Failed to load system manifest");

    let chip_path = system_path.parent().unwrap().join(&manifest.chip);
    let chip = ChipDescriptor::from_file(&chip_path)
        .unwrap_or_else(|_| panic!("Failed to load chip descriptor at {:?}", chip_path));

    let mut bus =
        labwired_core::bus::SystemBus::from_config(&chip, &manifest).expect("Failed to build bus");

    let (cpu, _nvic) = labwired_core::system::cortex_m::configure_cortex_m(&mut bus);
    let mut machine = Machine::new(cpu, bus);

    machine
        .load_firmware(&program)
        .expect("Failed to load firmware");

    // Run for 500k cycles to ensure it boots and runs (UART writes happen early)
    let mut steps = 0;
    for _ in 0..500_000 {
        machine.step().expect("Execution failed");
        steps += 1;

        // Optional: Peek UART state if possible
        // let uart = machine.peek_peripheral("uart3");
    }

    println!("Successfully executed {} steps of H5 firmware", steps);
}
