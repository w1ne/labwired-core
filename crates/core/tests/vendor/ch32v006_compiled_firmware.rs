use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::{bus::SystemBus, system::riscv::configure_riscv, Machine};

fn root() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn descriptor() -> ChipDescriptor {
    ChipDescriptor::from_file(root().join("configs/chips/ch32v006k8u6.yaml"))
        .expect("CH32V006K8U6 descriptor")
}

fn count_edges(port: &str, bit: u8, steps: usize) -> (u32, u32) {
    let chip = descriptor();
    let mut bus = SystemBus::from_config(&chip, &SystemManifest::default())
        .expect("CH32V006 peripherals and clock gates");
    let image = labwired_loader::load_elf(&root().join("tests/fixtures/ch32v006k8u6-blink.elf"))
        .expect("compiled RV32E ELF");
    let cpu = configure_riscv(&mut bus);
    let mut machine = Machine::new(cpu, bus);
    machine.load_firmware(&image).expect("load CH32V006 ELF");
    let port = machine
        .bus
        .find_peripheral_index_by_name(port)
        .expect("GPIO port registered from descriptor");

    let mut previous = None;
    let mut edges = 0;
    for _ in 0..steps {
        machine.step().expect("compiled RV32E firmware instruction");
        if let Some(state) = machine.bus.peripherals[port].dev.read_gpio_output(bit) {
            if previous.is_some_and(|prior| prior != state) {
                edges += 1;
            }
            previous = Some(state);
        }
    }
    (edges, machine.cpu.mcause)
}

#[test]
fn ch32v006_compiled_rv32e_firmware_toggles_pb0() {
    // GPIOB (0x40010C00, PB2PCENR bit 3) is the port the CH32V003 lacks.
    let (edges, mcause) = count_edges("gpiob", 0, 50_000);
    assert!(
        edges >= 2,
        "compiled firmware produced only {edges} PB0 edges"
    );
    assert_eq!(mcause, 0, "compiled firmware must not trap");

    // Negative control: the blink drives PB0 only.
    let (edges, _) = count_edges("gpioc", 0, 50_000);
    assert_eq!(edges, 0, "PC0 must stay idle");
}

fn run_uart_hello(chip: &ChipDescriptor) -> (Vec<u8>, u32) {
    let mut bus = SystemBus::from_config(chip, &SystemManifest::default())
        .expect("CH32V006 peripherals and clock gates");
    let sink = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    bus.attach_uart_tx_sink(sink.clone(), false);
    let image =
        labwired_loader::load_elf(&root().join("tests/fixtures/ch32v006k8u6-uart-hello.elf"))
            .expect("compiled RV32EC+Zmmul firmware ELF");

    let cpu = configure_riscv(&mut bus);
    let mut machine = Machine::new(cpu, bus);
    machine.load_firmware(&image).expect("load CH32V006 ELF");
    for _ in 0..20_000 {
        machine
            .step()
            .expect("compiled RV32EC+Zmmul firmware instruction");
    }
    let out = sink.lock().unwrap().clone();
    (out, machine.cpu.mcause)
}

#[test]
fn ch32v006_compiled_rv32ec_zmmul_firmware_prints_on_usart1() {
    // The firmware prints only if its MUL result matches the compile-time
    // product.
    let (out, mcause) = run_uart_hello(&descriptor());
    assert_eq!(
        String::from_utf8_lossy(&out),
        "Hello from CH32V006\r\n",
        "USART1 TX stream"
    );
    assert_eq!(mcause, 0, "compiled firmware must not trap");
}

#[test]
fn ch32v006_usart1_stays_silent_without_its_apb2_clock() {
    // Negative control: move the USART1 gate to a PB2PCENR bit the firmware
    // never sets. TXE must then never assert, so nothing reaches the sink.
    let yaml = std::fs::read_to_string(root().join("configs/chips/ch32v006k8u6.yaml"))
        .expect("read CH32V006 descriptor");
    let gate = "clock: { reg: \"apb2pcenr\", bit: 14 }";
    assert_eq!(yaml.matches(gate).count(), 1, "USART1 gate is unique");
    let chip: ChipDescriptor =
        serde_yaml::from_str(&yaml.replace(gate, "clock: { reg: \"apb2pcenr\", bit: 11 }"))
            .expect("parse modified descriptor");
    let (out, _) = run_uart_hello(&chip);
    assert!(out.is_empty(), "unclocked USART1 printed {out:?}");
}

#[test]
fn ch32v006k8u6_descriptor_matches_ds0_table_2_1_1() {
    // CH32V006DS0 V2.0 product table (K8: 62 KB flash, 8 KB SRAM, 31 GPIOs,
    // 48 MHz) and Table 2-1-1 QFN32 GPIOs.
    let chip = descriptor();
    assert_eq!(chip.core.as_deref(), Some("rv32emc"));
    assert_eq!(chip.cpu_hz, 48_000_000);
    assert_eq!(chip.flash.base, 0x0800_0000);
    assert_eq!(chip.flash.size, 62 * 1024);
    assert_eq!(chip.ram.base, 0x2000_0000);
    assert_eq!(chip.ram.size, 8 * 1024);
    let mut pins: Vec<&str> = chip.pins.keys().map(String::as_str).collect();
    pins.sort_unstable();
    let mut expected: Vec<String> = (0..8)
        .map(|b| format!("PA{b}"))
        .chain((0..7).map(|b| format!("PB{b}")))
        .chain((0..8).map(|b| format!("PC{b}")))
        .chain((0..8).map(|b| format!("PD{b}")))
        .collect();
    expected.sort_unstable();
    assert_eq!(
        pins,
        expected.iter().map(String::as_str).collect::<Vec<_>>()
    );
    assert_eq!(pins.len(), 31, "DS0: 31 GPIOs on K8 (QFN32)");
}
