use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::{bus::SystemBus, system::riscv::configure_riscv, Machine};

fn root() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn descriptor() -> ChipDescriptor {
    ChipDescriptor::from_file(root().join("configs/chips/ch32l103c8t6.yaml"))
        .expect("CH32L103C8T6 descriptor")
}

fn count_edges(fixture: &str, port: &str, bit: u8, steps: usize) -> (u32, u32) {
    let chip = descriptor();
    let mut bus = SystemBus::from_config(&chip, &SystemManifest::default())
        .expect("CH32L103 peripherals and clock gates");
    let image = labwired_loader::load_elf(&root().join(fixture)).expect("compiled RV32IMAC ELF");
    let cpu = configure_riscv(&mut bus);
    let mut machine = Machine::new(cpu, bus);
    machine.load_firmware(&image).expect("load CH32L103 ELF");
    let port = machine
        .bus
        .find_peripheral_index_by_name(port)
        .expect("GPIO port registered from descriptor");

    let mut previous = None;
    let mut edges = 0;
    for _ in 0..steps {
        machine
            .step()
            .expect("compiled RV32IMAC firmware instruction");
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
fn ch32l103_compiled_rv32imac_firmware_toggles_pc13() {
    // PC13 sits in GPIOC_CFGHR, the high configuration word that the L103
    // GPIO has and the CH32V003 GPIO does not.
    let (edges, mcause) = count_edges("tests/fixtures/ch32l103c8t6-blink.elf", "gpioc", 13, 50_000);
    assert!(
        edges >= 2,
        "compiled firmware produced only {edges} PC13 edges"
    );
    assert_eq!(mcause, 0, "compiled firmware must not trap");

    // Negative control: the blink drives PC13 only.
    let (edges, _) = count_edges("tests/fixtures/ch32l103c8t6-blink.elf", "gpioc", 0, 50_000);
    assert_eq!(edges, 0, "PC0 must stay idle");
}

fn run_uart_hello(chip: &ChipDescriptor) -> (Vec<u8>, u32) {
    let mut bus = SystemBus::from_config(chip, &SystemManifest::default())
        .expect("CH32L103 peripherals and clock gates");
    let sink = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    bus.attach_uart_tx_sink(sink.clone(), false);
    let image =
        labwired_loader::load_elf(&root().join("tests/fixtures/ch32l103c8t6-uart-hello.elf"))
            .expect("compiled RV32IMAC firmware ELF");
    let cpu = configure_riscv(&mut bus);
    let mut machine = Machine::new(cpu, bus);
    machine.load_firmware(&image).expect("load CH32L103 ELF");
    for _ in 0..20_000 {
        machine
            .step()
            .expect("compiled RV32IMAC firmware instruction");
    }
    let out = sink.lock().unwrap().clone();
    (out, machine.cpu.mcause)
}

#[test]
fn ch32l103_compiled_rv32imac_firmware_prints_on_usart1() {
    // The firmware prints the hello only if its MUL/DIVU/AMOADD.W check
    // matches the compile-time value, so the string also proves the M and A
    // extensions executed correctly.
    let (out, mcause) = run_uart_hello(&descriptor());
    assert_eq!(
        String::from_utf8_lossy(&out),
        "Hello from CH32L103\r\n",
        "USART1 TX stream"
    );
    assert_eq!(mcause, 0, "compiled firmware must not trap");
}

#[test]
fn ch32l103_usart1_stays_silent_without_its_apb2_clock() {
    // Negative control: move the USART1 gate to an APB2PCENR bit the firmware
    // never sets. TXE must then never assert, so nothing reaches the sink.
    let yaml = std::fs::read_to_string(root().join("configs/chips/ch32l103c8t6.yaml"))
        .expect("read CH32L103 descriptor");
    let gate = "clock: { reg: \"apb2enr\", bit: 14 }";
    assert_eq!(yaml.matches(gate).count(), 1, "USART1 gate is unique");
    let chip: ChipDescriptor =
        serde_yaml::from_str(&yaml.replace(gate, "clock: { reg: \"apb2enr\", bit: 11 }"))
            .expect("parse modified descriptor");
    let (out, _) = run_uart_hello(&chip);
    assert!(out.is_empty(), "unclocked USART1 printed {out:?}");
}

#[test]
fn ch32l103c8t6_descriptor_matches_datasheet_product_table() {
    // CH32L103 Datasheet V2.1 product table, C8T6 column, and Table 2-1-1
    // LQFP48 pin definitions.
    let chip = descriptor();
    assert_eq!(chip.core.as_deref(), Some("rv32imac"));
    assert_eq!(chip.cpu_hz, 96_000_000);
    assert_eq!(chip.flash.base, 0x0800_0000);
    assert_eq!(chip.flash.size, 64 * 1024);
    assert_eq!(chip.ram.base, 0x2000_0000);
    assert_eq!(chip.ram.size, 20 * 1024);
    let mut pins: Vec<&str> = chip.pins.keys().map(String::as_str).collect();
    pins.sort_unstable();
    let mut expected: Vec<String> = (0..16)
        .map(|b| format!("PA{b}"))
        .chain((0..16).map(|b| format!("PB{b}")))
        .chain((13..16).map(|b| format!("PC{b}")))
        .chain((0..2).map(|b| format!("PD{b}")))
        .collect();
    expected.sort_unstable();
    assert_eq!(
        pins,
        expected.iter().map(String::as_str).collect::<Vec<_>>()
    );
    assert_eq!(pins.len(), 37, "DS product table: 37 GPIOs on C8T6");
}
