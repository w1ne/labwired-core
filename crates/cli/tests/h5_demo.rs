use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::Machine;
use labwired_loader::load_elf;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

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

    let system_path = root.join("configs/systems/nucleo-h563zi-demo.yaml");

    if !system_path.exists() {
        panic!("System config not found at {:?}", system_path);
    }

    let manifest = SystemManifest::from_file(&system_path).expect("Failed to load system manifest");

    let chip_path = system_path.parent().unwrap().join(&manifest.chip);
    let chip = ChipDescriptor::from_file(&chip_path)
        .unwrap_or_else(|_| panic!("Failed to load chip descriptor at {:?}", chip_path));

    let mut bus =
        labwired_core::bus::SystemBus::from_config(&chip, &manifest).expect("Failed to build bus");

    // Attach UART sink so we can assert on what the firmware writes.
    let uart_sink: Arc<Mutex<Vec<u8>>> = Arc::new(Mutex::new(Vec::new()));
    bus.attach_uart_tx_sink(uart_sink.clone(), false);

    let (cpu, _nvic) = labwired_core::system::cortex_m::configure_cortex_m(&mut bus);
    let mut machine = Machine::new(cpu, bus);

    machine
        .load_firmware(&program)
        .expect("Failed to load firmware");

    // firmware-h563-demo writes "OK\n" via USART3 early in boot.
    // Run for up to 500k steps to give it time to boot and write.
    const MAX_STEPS: usize = 500_000;
    for _ in 0..MAX_STEPS {
        if machine.step().is_err() {
            break;
        }
    }

    let uart_bytes = uart_sink.lock().unwrap().clone();
    let uart_str = String::from_utf8_lossy(&uart_bytes);
    eprintln!(
        "[h5_demo] UART output after {} steps: {:?}",
        MAX_STEPS, uart_str
    );

    assert!(
        !uart_bytes.is_empty(),
        "H563 demo produced no UART output after {MAX_STEPS} steps"
    );
    // The H563 demo writes "OK\n" via USART3 (see firmware-h563-demo/src/main.rs).
    assert!(
        uart_str.contains("OK"),
        "H563 demo UART output does not contain expected prefix 'OK'; got: {uart_str:?}"
    );
}
