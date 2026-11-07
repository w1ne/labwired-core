use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::{bus::SystemBus, system::riscv::configure_riscv, Machine};

#[test]
fn ch32v003_compiled_rv32e_firmware_toggles_pc0() {
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let chip_path = root.join("configs/chips/ch32v003f4p6.yaml");
    let chip = ChipDescriptor::from_file(&chip_path).expect("CH32V003 descriptor");
    let mut bus = SystemBus::from_config(&chip, &SystemManifest::default())
        .expect("CH32V003 peripherals and clock gates");
    let image = labwired_loader::load_elf(&root.join("tests/fixtures/ch32v003f4p6-blink.elf"))
        .expect("compiled RV32E firmware ELF");

    let cpu = configure_riscv(&mut bus);
    let mut machine = Machine::new(cpu, bus);
    machine.load_firmware(&image).expect("load CH32V003 ELF");
    let gpioc = machine
        .bus
        .find_peripheral_index_by_name("gpioc")
        .expect("GPIOC registered from descriptor");

    let mut previous = None;
    let mut edges = 0;
    for _ in 0..200_000 {
        machine.step().expect("compiled RV32E firmware instruction");
        if let Some(state) = machine.bus.peripherals[gpioc].dev.read_gpio_output(0) {
            if previous.is_some_and(|prior| prior != state) {
                edges += 1;
            }
            previous = Some(state);
            if edges >= 2 {
                break;
            }
        }
    }

    assert!(
        edges >= 2,
        "compiled firmware produced only {edges} PC0 edges"
    );
    assert_eq!(machine.cpu.mcause, 0, "compiled firmware must not trap");
}

fn run_uart_hello(chip: &ChipDescriptor) -> (Vec<u8>, u32) {
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut bus = SystemBus::from_config(chip, &SystemManifest::default())
        .expect("CH32V003 peripherals and clock gates");
    let sink = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    bus.attach_uart_tx_sink(sink.clone(), false);
    let image = labwired_loader::load_elf(&root.join("tests/fixtures/ch32v003f4p6-uart-hello.elf"))
        .expect("compiled RV32EC firmware ELF");

    let cpu = configure_riscv(&mut bus);
    let mut machine = Machine::new(cpu, bus);
    machine.load_firmware(&image).expect("load CH32V003 ELF");
    for _ in 0..20_000 {
        machine
            .step()
            .expect("compiled RV32EC firmware instruction");
    }
    let out = sink.lock().unwrap().clone();
    (out, machine.cpu.mcause)
}

#[test]
fn ch32v003_compiled_rv32ec_firmware_prints_on_usart1() {
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let chip = ChipDescriptor::from_file(&root.join("configs/chips/ch32v003f4p6.yaml"))
        .expect("CH32V003 descriptor");
    let (out, mcause) = run_uart_hello(&chip);
    assert_eq!(
        String::from_utf8_lossy(&out),
        "Hello from CH32V003\r\n",
        "USART1 TX stream"
    );
    assert_eq!(mcause, 0, "compiled firmware must not trap");
}

#[test]
fn ch32v003_usart1_stays_silent_without_its_apb2_clock() {
    // Negative control: move the USART1 gate to an APB2PCENR bit the firmware
    // never sets. TXE must then never assert, so nothing reaches the sink.
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let yaml = std::fs::read_to_string(root.join("configs/chips/ch32v003f4p6.yaml"))
        .expect("read CH32V003 descriptor");
    let gate = "clock: { reg: \"apb2pcenr\", bit: 14 }";
    assert_eq!(yaml.matches(gate).count(), 1, "USART1 gate is unique");
    let chip: ChipDescriptor =
        serde_yaml::from_str(&yaml.replace(gate, "clock: { reg: \"apb2pcenr\", bit: 11 }"))
            .expect("parse modified descriptor");
    let (out, _) = run_uart_hello(&chip);
    assert!(out.is_empty(), "unclocked USART1 printed {out:?}");
}
