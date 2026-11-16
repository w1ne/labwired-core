use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::{bus::SystemBus, system::riscv::configure_riscv, Machine};

fn root() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn descriptor() -> ChipDescriptor {
    ChipDescriptor::from_file(root().join("configs/chips/ch32x035c8t6.yaml"))
        .expect("CH32X035C8T6 descriptor")
}

fn count_edges(fixture: &str, port: &str, bit: u8, steps: usize) -> (u32, u32) {
    let chip = descriptor();
    let mut bus = SystemBus::from_config(&chip, &SystemManifest::default())
        .expect("CH32X035 peripherals and clock gates");
    let image = labwired_loader::load_elf(&root().join(fixture)).expect("compiled RV32IMAC ELF");
    let cpu = configure_riscv(&mut bus);
    let mut machine = Machine::new(cpu, bus);
    machine.load_firmware(&image).expect("load CH32X035 ELF");
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
fn ch32x035_compiled_rv32imac_firmware_toggles_pb12() {
    // PB12 sits in GPIOB_CFGHR, the F1 high configuration word that the X035
    // GPIO keeps and the CH32V003 GPIO does not.
    let (edges, mcause) = count_edges("tests/fixtures/ch32x035c8t6-blink.elf", "gpiob", 12, 50_000);
    assert!(
        edges >= 2,
        "compiled firmware produced only {edges} PB12 edges"
    );
    assert_eq!(mcause, 0, "compiled firmware must not trap");

    // Negative control: the blink drives PB12 only.
    let (edges, _) = count_edges("tests/fixtures/ch32x035c8t6-blink.elf", "gpiob", 0, 50_000);
    assert_eq!(edges, 0, "PB0 must stay idle");
}

fn run_uart_hello(chip: &ChipDescriptor) -> (Vec<u8>, u32) {
    let mut bus = SystemBus::from_config(chip, &SystemManifest::default())
        .expect("CH32X035 peripherals and clock gates");
    let sink = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    bus.attach_uart_tx_sink(sink.clone(), false);
    let image =
        labwired_loader::load_elf(&root().join("tests/fixtures/ch32x035c8t6-uart-hello.elf"))
            .expect("compiled RV32IMAC firmware ELF");
    let cpu = configure_riscv(&mut bus);
    let mut machine = Machine::new(cpu, bus);
    machine.load_firmware(&image).expect("load CH32X035 ELF");
    for _ in 0..20_000 {
        machine
            .step()
            .expect("compiled RV32IMAC firmware instruction");
    }
    let out = sink.lock().unwrap().clone();
    (out, machine.cpu.mcause)
}

#[test]
fn ch32x035_compiled_rv32imac_firmware_prints_on_usart1() {
    // The firmware prints the hello only if its MUL/DIVU/AMOADD.W check
    // matches the compile-time value, so the string also proves the M and A
    // extensions executed correctly.
    let (out, mcause) = run_uart_hello(&descriptor());
    assert_eq!(
        String::from_utf8_lossy(&out),
        "Hello from CH32X035\r\n",
        "USART1 TX stream"
    );
    assert_eq!(mcause, 0, "compiled firmware must not trap");
}

#[test]
fn ch32x035_usart1_stays_silent_without_its_apb2_clock() {
    // Negative control: move the USART1 gate to an APB2PCENR bit the firmware
    // never sets. TXE must then never assert, so nothing reaches the sink.
    let yaml = std::fs::read_to_string(root().join("configs/chips/ch32x035c8t6.yaml"))
        .expect("read CH32X035 descriptor");
    let gate = "clock: { reg: \"apb2enr\", bit: 14 }";
    assert_eq!(yaml.matches(gate).count(), 1, "USART1 gate is unique");
    let chip: ChipDescriptor =
        serde_yaml::from_str(&yaml.replace(gate, "clock: { reg: \"apb2enr\", bit: 11 }"))
            .expect("parse modified descriptor");
    let (out, _) = run_uart_hello(&chip);
    assert!(out.is_empty(), "unclocked USART1 printed {out:?}");
}

#[test]
fn ch32x035c8t6_descriptor_matches_ds_v2_3() {
    // CH32X035/X033 DS V2.3 product table (C8T6 column) and Table 2-1
    // (LQFP48). Only bits 0..15 are modelled: PA16..PA23 and PC16..PC19 use
    // CFGXR/BSXR, which the F1 GPIO model lacks.
    let chip = descriptor();
    assert_eq!(chip.core.as_deref(), Some("rv32imac"));
    assert_eq!(chip.cpu_hz, 48_000_000);
    assert_eq!(chip.flash.base, 0x0800_0000);
    assert_eq!(chip.flash.size, 62 * 1024);
    assert_eq!(chip.ram.base, 0x2000_0000);
    assert_eq!(chip.ram.size, 20 * 1024);
    let mut pins: Vec<&str> = chip.pins.keys().map(String::as_str).collect();
    pins.sort_unstable();
    let mut expected: Vec<String> = (0..16)
        .map(|b| format!("PA{b}"))
        .chain((0..14).map(|b| format!("PB{b}")))
        .chain([6, 7, 14, 15].map(|b| format!("PC{b}")))
        .collect();
    expected.sort_unstable();
    assert_eq!(
        pins,
        expected.iter().map(String::as_str).collect::<Vec<_>>()
    );
    // 46 LQFP48 GPIOs minus PA16..PA23 and PC16..PC19.
    assert_eq!(pins.len(), 46 - 8 - 4);
}

#[test]
fn ch32x035_usart1_needs_the_x035_pad_map() {
    // Negative control: without `pad_map: wch_x035` the F1 table gates the
    // console on PA9, which this firmware never configures, so the PB10
    // output must not reach the sink. An unknown pad map must be rejected.
    let yaml = std::fs::read_to_string(root().join("configs/chips/ch32x035c8t6.yaml"))
        .expect("read CH32X035 descriptor");
    let key = "pad_map: \"wch_x035\"";
    assert_eq!(yaml.matches(key).count(), 1, "USART1 pad map is unique");
    let chip: ChipDescriptor = serde_yaml::from_str(&yaml.replace(key, "pad_map: \"none\""))
        .expect("parse modified descriptor");
    let (out, _) = run_uart_hello(&chip);
    assert!(out.is_empty(), "F1 pad table printed {out:?}");

    let chip: ChipDescriptor = serde_yaml::from_str(&yaml.replace(key, "pad_map: \"bogus\""))
        .expect("parse modified descriptor");
    assert!(
        SystemBus::from_config(&chip, &SystemManifest::default()).is_err(),
        "unknown UART pad_map must fail the build"
    );
}
