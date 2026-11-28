// Compiled-firmware contracts shared by the CH32 chips whose tests used to be
// five line-for-line per-chip copies (V005, V006, V203, X035, L103). The chips
// differ only in DATA, which lives in `CHIPS`; each `#[test]` below is one
// contract looped over every row, with the chip name in every assertion
// message. `ch32v003_compiled_firmware.rs` is a different shape and stays
// separate.
use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::{bus::SystemBus, system::riscv::configure_riscv, Bus, Machine};

/// Which pin numbers of a port the datasheet bonds out.
enum Bits {
    Range(u8, u8),
    List(&'static [u8]),
}

struct Row {
    /// Chip name used in every assertion message, e.g. "CH32V005".
    name: &'static str,
    /// Descriptor basename under configs/chips/, also the fixture prefix.
    part: &'static str,
    /// Descriptor expect text, e.g. "CH32V005F6P6".
    part_label: &'static str,
    /// ISA named in the blink firmware's step expect text.
    blink_isa: &'static str,
    /// ISA named in the uart firmware's ELF/step expect text.
    uart_isa: &'static str,
    /// Blink target: (port peripheral, bit, label such as "PC0").
    blink: (&'static str, u8, &'static str),
    /// Negative control: a pin the blink must NOT drive.
    idle: (&'static str, u8, &'static str),
    /// Text USART1 must print on the 20 000-step uart-hello run.
    uart_text: &'static str,
    /// Clock-enable register whose bit 14 gates USART1 in the descriptor.
    gate_reg: &'static str,
    /// Datasheet core string.
    core: &'static str,
    cpu_hz: u64,
    flash_size: u64,
    ram_size: u64,
    /// GPIO pins the datasheet package bonds out, per port.
    pins: &'static [(&'static str, Bits)],
    pin_count: usize,
    pin_count_msg: &'static str,
}

const CHIPS: &[Row] = &[
    Row {
        name: "CH32V005",
        part: "ch32v005f6p6",
        part_label: "CH32V005F6P6",
        blink_isa: "RV32E",
        uart_isa: "RV32EC+Zmmul",
        // PC0 is TSSOP20 pin 10 (DS0 CH32V005F6P6 pinout). The blink drives
        // PC0 only.
        blink: ("gpioc", 0, "PC0"),
        idle: ("gpiod", 0, "PD0"),
        // The firmware prints only if its MUL result matches the compile-time
        // product.
        uart_text: "Hello from CH32V005\r\n",
        // Negative control moves the USART1 gate to a PB2PCENR bit the
        // firmware never sets.
        gate_reg: "apb2pcenr",
        // CH32V006/005 DS0 V2.0 product table (V005F6P6: 32 KB flash, 6 KB
        // SRAM, 18 GPIOs, TSSOP20, 48 MHz) and the CH32V005F6P6 pinout.
        core: "rv32emc",
        cpu_hz: 48_000_000,
        flash_size: 32 * 1024,
        ram_size: 6 * 1024,
        pins: &[
            ("PA", Bits::Range(1, 3)),
            ("PC", Bits::Range(0, 8)),
            ("PD", Bits::Range(0, 8)),
        ],
        pin_count: 18,
        pin_count_msg: "DS0: 18 GPIOs on F6P6 (TSSOP20)",
    },
    Row {
        name: "CH32V006",
        part: "ch32v006k8u6",
        part_label: "CH32V006K8U6",
        blink_isa: "RV32E",
        uart_isa: "RV32EC+Zmmul",
        // GPIOB (0x40010C00, PB2PCENR bit 3) is the port the CH32V003 lacks.
        // The blink drives PB0 only.
        blink: ("gpiob", 0, "PB0"),
        idle: ("gpioc", 0, "PC0"),
        // The firmware prints only if its MUL result matches the compile-time
        // product.
        uart_text: "Hello from CH32V006\r\n",
        // Negative control moves the USART1 gate to a PB2PCENR bit the
        // firmware never sets.
        gate_reg: "apb2pcenr",
        // CH32V006DS0 V2.0 product table (K8: 62 KB flash, 8 KB SRAM, 31
        // GPIOs, 48 MHz) and Table 2-1-1 QFN32 GPIOs.
        core: "rv32emc",
        cpu_hz: 48_000_000,
        flash_size: 62 * 1024,
        ram_size: 8 * 1024,
        pins: &[
            ("PA", Bits::Range(0, 8)),
            ("PB", Bits::Range(0, 7)),
            ("PC", Bits::Range(0, 8)),
            ("PD", Bits::Range(0, 8)),
        ],
        pin_count: 31,
        pin_count_msg: "DS0: 31 GPIOs on K8 (QFN32)",
    },
    Row {
        name: "CH32V203",
        part: "ch32v203c8t6",
        part_label: "CH32V203C8T6",
        blink_isa: "RV32IMAC",
        uart_isa: "RV32IMAC",
        // PC13 sits in GPIOC_CFGHR, the high configuration word that the V20x
        // GPIO has and the CH32V003 GPIO does not. The blink drives PC13 only.
        blink: ("gpioc", 13, "PC13"),
        idle: ("gpioc", 0, "PC0"),
        // The firmware prints the hello only if its MUL/DIVU/AMOADD.W check
        // matches the compile-time value, so the string also proves the M and
        // A extensions executed correctly.
        uart_text: "Hello from CH32V203\r\n",
        // Negative control moves the USART1 gate to an APB2PCENR bit the
        // firmware never sets.
        gate_reg: "apb2enr",
        // CH32V203DS0 V3.0 Table 2-1, C8x6 column, and Table 3-1-1 LQFP48
        // GPIOs. The core string is the ch32-rs/ch32-data name of the
        // QingKe V4B (RV32IMAC), which selects the PFIC and CORECFGR.
        core: "qingke-v4b",
        cpu_hz: 144_000_000,
        flash_size: 64 * 1024,
        ram_size: 20 * 1024,
        pins: &[
            ("PA", Bits::Range(0, 16)),
            ("PB", Bits::Range(0, 16)),
            ("PC", Bits::Range(13, 16)),
            ("PD", Bits::Range(0, 2)),
        ],
        pin_count: 37,
        pin_count_msg: "DS0 Table 2-1: 37 GPIOs on C8x6",
    },
    Row {
        name: "CH32X035",
        part: "ch32x035c8t6",
        part_label: "CH32X035C8T6",
        blink_isa: "RV32IMAC",
        uart_isa: "RV32IMAC",
        // PB12 sits in GPIOB_CFGHR, the F1 high configuration word that the
        // X035 GPIO keeps and the CH32V003 GPIO does not. The blink drives
        // PB12 only.
        blink: ("gpiob", 12, "PB12"),
        idle: ("gpiob", 0, "PB0"),
        // The firmware prints the hello only if its MUL/DIVU/AMOADD.W check
        // matches the compile-time value, so the string also proves the M and
        // A extensions executed correctly.
        uart_text: "Hello from CH32X035\r\n",
        // Negative control moves the USART1 gate to an APB2PCENR bit the
        // firmware never sets.
        gate_reg: "apb2enr",
        // CH32X035/X033 DS V2.3 product table (C8T6 column) and Table 2-1
        // (LQFP48). Only bits 0..15 are modelled: PA16..PA23 and PC16..PC19
        // use CFGXR/BSXR, which the F1 GPIO model lacks.
        core: "rv32imac",
        cpu_hz: 48_000_000,
        flash_size: 62 * 1024,
        ram_size: 20 * 1024,
        pins: &[
            ("PA", Bits::Range(0, 16)),
            ("PB", Bits::Range(0, 14)),
            ("PC", Bits::List(&[6, 7, 14, 15])),
        ],
        // 46 LQFP48 GPIOs minus PA16..PA23 and PC16..PC19.
        pin_count: 46 - 8 - 4,
        pin_count_msg: "46 LQFP48 GPIOs minus PA16..PA23 and PC16..PC19",
    },
    Row {
        name: "CH32L103",
        part: "ch32l103c8t6",
        part_label: "CH32L103C8T6",
        blink_isa: "RV32IMAC",
        uart_isa: "RV32IMAC",
        // PC13 sits in GPIOC_CFGHR, the high configuration word that the L103
        // GPIO has and the CH32V003 GPIO does not. The blink drives PC13 only.
        blink: ("gpioc", 13, "PC13"),
        idle: ("gpioc", 0, "PC0"),
        // The firmware prints the hello only if its MUL/DIVU/AMOADD.W check
        // matches the compile-time value, so the string also proves the M and
        // A extensions executed correctly.
        uart_text: "Hello from CH32L103\r\n",
        // Negative control moves the USART1 gate to an APB2PCENR bit the
        // firmware never sets.
        gate_reg: "apb2enr",
        // CH32L103 Datasheet V2.1 product table, C8T6 column, and Table 2-1-1
        // LQFP48 pin definitions.
        core: "rv32imac",
        cpu_hz: 96_000_000,
        flash_size: 64 * 1024,
        ram_size: 20 * 1024,
        pins: &[
            ("PA", Bits::Range(0, 16)),
            ("PB", Bits::Range(0, 16)),
            ("PC", Bits::Range(13, 16)),
            ("PD", Bits::Range(0, 2)),
        ],
        pin_count: 37,
        pin_count_msg: "DS product table: 37 GPIOs on C8T6",
    },
];

fn root() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn descriptor_path(row: &Row) -> std::path::PathBuf {
    root().join(format!("configs/chips/{}.yaml", row.part))
}

fn descriptor(row: &Row) -> ChipDescriptor {
    ChipDescriptor::from_file(descriptor_path(row))
        .unwrap_or_else(|e| panic!("{} descriptor {}: {e}", row.name, row.part_label))
}

fn count_edges(row: &Row, port: &str, bit: u8, steps: usize) -> (u32, u32) {
    let chip = descriptor(row);
    let mut bus = SystemBus::from_config(&chip, &SystemManifest::default())
        .unwrap_or_else(|e| panic!("{} peripherals and clock gates: {e}", row.name));
    let image =
        labwired_loader::load_elf(&root().join(format!("tests/fixtures/{}-blink.elf", row.part)))
            .unwrap_or_else(|e| panic!("{} compiled {} ELF: {e}", row.name, row.blink_isa));
    let cpu = configure_riscv(&mut bus);
    let mut machine = Machine::new(cpu, bus);
    machine
        .load_firmware(&image)
        .unwrap_or_else(|e| panic!("load {} ELF: {e}", row.name));
    let port = machine
        .bus
        .find_peripheral_index_by_name(port)
        .unwrap_or_else(|| panic!("{} GPIO port registered from descriptor", row.name));

    let mut previous = None;
    let mut edges = 0;
    for _ in 0..steps {
        machine.step().unwrap_or_else(|e| {
            panic!(
                "{} compiled {} firmware instruction: {e}",
                row.name, row.blink_isa
            )
        });
        if let Some(state) = machine.bus.peripherals[port].dev.read_gpio_output(bit) {
            if previous.is_some_and(|prior| prior != state) {
                edges += 1;
            }
            previous = Some(state);
        }
    }
    (edges, machine.cpu.mcause)
}

fn run_uart_hello(row: &Row, chip: &ChipDescriptor) -> (Vec<u8>, u32) {
    let mut bus = SystemBus::from_config(chip, &SystemManifest::default())
        .unwrap_or_else(|e| panic!("{} peripherals and clock gates: {e}", row.name));
    let sink = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    bus.attach_uart_tx_sink(sink.clone(), false);
    let image = labwired_loader::load_elf(
        &root().join(format!("tests/fixtures/{}-uart-hello.elf", row.part)),
    )
    .unwrap_or_else(|e| panic!("{} compiled {} firmware ELF: {e}", row.name, row.uart_isa));

    let cpu = configure_riscv(&mut bus);
    let mut machine = Machine::new(cpu, bus);
    machine
        .load_firmware(&image)
        .unwrap_or_else(|e| panic!("load {} ELF: {e}", row.name));
    for _ in 0..20_000 {
        machine.step().unwrap_or_else(|e| {
            panic!(
                "{} compiled {} firmware instruction: {e}",
                row.name, row.uart_isa
            )
        });
    }
    let out = sink.lock().unwrap().clone();
    (out, machine.cpu.mcause)
}

#[test]
fn every_ch32_compiled_blink_toggles_its_pin_and_only_its_pin() {
    for row in CHIPS {
        let (port, bit, label) = row.blink;
        let (edges, mcause) = count_edges(row, port, bit, 50_000);
        assert!(
            edges >= 2,
            "{}: compiled firmware produced only {edges} {label} edges",
            row.name
        );
        assert_eq!(mcause, 0, "{}: compiled firmware must not trap", row.name);

        // Negative control: the blink drives only its own pin.
        let (port, bit, label) = row.idle;
        let (edges, _) = count_edges(row, port, bit, 50_000);
        assert_eq!(edges, 0, "{}: {label} must stay idle", row.name);
    }
}

#[test]
fn every_ch32_compiled_uart_hello_prints_on_usart1() {
    for row in CHIPS {
        let (out, mcause) = run_uart_hello(row, &descriptor(row));
        assert_eq!(
            String::from_utf8_lossy(&out),
            row.uart_text,
            "{}: USART1 TX stream",
            row.name
        );
        assert_eq!(mcause, 0, "{}: compiled firmware must not trap", row.name);
    }
}

/// Runs the CH32V203 USART1 RXNE fixture with bytes already waiting on USART1
/// RX, returning the USART1 TX stream and the last QingKe trap cause.
fn run_ch32v203_usart1_rx_irq(chip: &ChipDescriptor, bytes: &[u8]) -> (String, u32) {
    let mut bus = SystemBus::from_config(chip, &SystemManifest::default())
        .expect("CH32V203 peripherals and clock gates");
    let sink = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    bus.attach_uart_tx_sink(sink.clone(), false);
    bus.attach_uart_rx_source_named("usart1")
        .expect("USART1 RX source")
        .lock()
        .unwrap()
        .extend(bytes.iter().copied());

    let image =
        labwired_loader::load_elf(&root().join("examples/ch32v203c8t6/ch32fun/usart_rx_irq.elf"))
            .expect("CH32V203 USART RX IRQ ELF");
    let cpu = configure_riscv(&mut bus);
    let mut machine = Machine::new(cpu, bus);
    machine
        .load_firmware(&image)
        .expect("load USART RX IRQ ELF");

    for _ in 0..2_000_000 {
        machine.step().expect("CH32V203 USART RX IRQ instruction");
        let out = String::from_utf8_lossy(&sink.lock().unwrap()).into_owned();
        if out.contains("USART1_IRQ_RX=wch!\r\n") || out.contains("USART1_IRQ_TIMEOUT\r\n") {
            break;
        }
    }

    let out = String::from_utf8_lossy(&sink.lock().unwrap()).into_owned();
    (out, machine.cpu.mcause)
}

#[test]
fn ch32v203_usart1_rxne_reaches_qingke_pfic_irq_53() {
    let row = CHIPS
        .iter()
        .find(|r| r.name == "CH32V203")
        .expect("CH32V203 row");
    let chip = descriptor(row);
    let (out, mcause) = run_ch32v203_usart1_rx_irq(&chip, b"wch!");
    assert_eq!(out, "USART1_IRQ_RX=wch!\r\n");
    assert_eq!(mcause, 0x8000_0035, "last trap was USART1_IRQn 53");
}

#[test]
fn ch32v203_usart1_rxne_stays_unhandled_without_irq_53_wired() {
    let row = CHIPS
        .iter()
        .find(|r| r.name == "CH32V203")
        .expect("CH32V203 row");
    let text = std::fs::read_to_string(descriptor_path(row)).expect("read CH32V203 descriptor");
    let mut yaml: serde_yaml::Value = serde_yaml::from_str(&text).expect("parse chip descriptor");
    let peripherals = yaml["peripherals"]
        .as_sequence_mut()
        .expect("peripherals list");
    let usart1 = peripherals
        .iter_mut()
        .find(|p| p["id"].as_str() == Some("usart1"))
        .expect("usart1 block");
    assert_eq!(usart1["irq"].as_u64(), Some(53), "USART1_IRQn is wired");
    usart1
        .as_mapping_mut()
        .unwrap()
        .remove(serde_yaml::Value::String("irq".to_owned()));
    let chip: ChipDescriptor = serde_yaml::from_value(yaml).expect("descriptor without USART1 IRQ");

    let (out, mcause) = run_ch32v203_usart1_rx_irq(&chip, b"wch!");
    assert_eq!(out, "USART1_IRQ_TIMEOUT\r\n");
    assert_eq!(mcause, 0, "unwired USART1 must not enter the PFIC");
}

/// Runs the CH32V203 TIM2 update / TIM1 CC1 interrupt fixture, returning the
/// USART1 TX stream and the last QingKe trap cause.
fn run_ch32v203_tim_irq(chip: &ChipDescriptor) -> (String, u32) {
    let mut bus = SystemBus::from_config(chip, &SystemManifest::default())
        .expect("CH32V203 peripherals and clock gates");
    let sink = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    bus.attach_uart_tx_sink(sink.clone(), false);

    let image =
        labwired_loader::load_elf(&root().join("examples/ch32v203c8t6/ch32fun/tim_irq.elf"))
            .expect("CH32V203 TIM IRQ ELF");
    let cpu = configure_riscv(&mut bus);
    let mut machine = Machine::new(cpu, bus);
    machine.load_firmware(&image).expect("load TIM IRQ ELF");

    for _ in 0..8_000_000 {
        machine.step().expect("CH32V203 TIM IRQ instruction");
        if sink.lock().unwrap().ends_with(b"\r\n") {
            break;
        }
    }

    let out = String::from_utf8_lossy(&sink.lock().unwrap()).into_owned();
    (out, machine.cpu.mcause)
}

#[test]
fn ch32v203_tim2_update_and_tim1_cc1_reach_qingke_pfic_irqs_44_and_43() {
    let row = CHIPS
        .iter()
        .find(|r| r.name == "CH32V203")
        .expect("CH32V203 row");
    let (out, mcause) = run_ch32v203_tim_irq(&descriptor(row));
    // 10 TIM2 update periods at PSC 143 / ATRLR 999 measure 10 ms of
    // SysTick; TIM1 CH1 compares at 0.25 ms past each TIM1 update.
    assert_eq!(out, "TIM2_UP=11 ms=10 TIM1_CC1=11\r\n");
    assert_eq!(mcause, 0x8000_002C, "last trap was TIM2_IRQn 44");
}

#[test]
fn ch32v203_tim2_update_stays_unhandled_without_irq_44_wired() {
    let row = CHIPS
        .iter()
        .find(|r| r.name == "CH32V203")
        .expect("CH32V203 row");
    let text = std::fs::read_to_string(descriptor_path(row)).expect("read CH32V203 descriptor");
    let mut yaml: serde_yaml::Value = serde_yaml::from_str(&text).expect("parse chip descriptor");
    let tim2 = yaml["peripherals"]
        .as_sequence_mut()
        .expect("peripherals list")
        .iter_mut()
        .find(|p| p["id"].as_str() == Some("tim2"))
        .expect("tim2 block");
    assert_eq!(tim2["irq"].as_u64(), Some(44), "TIM2_IRQn is wired");
    tim2.as_mapping_mut()
        .unwrap()
        .remove(serde_yaml::Value::String("irq".to_owned()));
    let chip: ChipDescriptor = serde_yaml::from_value(yaml).expect("descriptor without TIM2 IRQ");

    let (out, _) = run_ch32v203_tim_irq(&chip);
    assert!(
        out.starts_with("TIM_TIMEOUT up=0 cc1="),
        "no TIM2 handler ran: {out:?}"
    );
}

/// The CH32V203 descriptor as YAML with PA0 (ADC1 IN0) held at 1650 mV and
/// PA1 (ADC2 IN1) at 825 mV, the pad voltages of `adc-system.yaml`.
fn ch32v203_adc_yaml() -> serde_yaml::Value {
    let row = CHIPS
        .iter()
        .find(|r| r.name == "CH32V203")
        .expect("CH32V203 row");
    let text = std::fs::read_to_string(descriptor_path(row)).expect("read CH32V203 descriptor");
    let mut yaml: serde_yaml::Value = serde_yaml::from_str(&text).expect("parse chip descriptor");
    for (id, channel, mv) in [("adc1", 0u64, 1650u64), ("adc2", 1, 825)] {
        let adc = yaml["peripherals"]
            .as_sequence_mut()
            .expect("peripherals list")
            .iter_mut()
            .find(|p| p["id"].as_str() == Some(id))
            .expect("adc block");
        let mut input_mv = serde_yaml::Mapping::new();
        input_mv.insert(channel.into(), mv.into());
        adc["config"]
            .as_mapping_mut()
            .expect("adc config")
            .insert("input_mv".into(), input_mv.into());
    }
    yaml
}

/// Runs the CH32V203 ADC1 EOC interrupt / ADC2 polled fixture, returning the
/// USART1 TX stream and the last QingKe trap cause.
fn run_ch32v203_adc_irq(chip: &ChipDescriptor) -> (String, u32) {
    let mut bus = SystemBus::from_config(chip, &SystemManifest::default())
        .expect("CH32V203 peripherals and clock gates");
    let sink = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    bus.attach_uart_tx_sink(sink.clone(), false);

    let image =
        labwired_loader::load_elf(&root().join("examples/ch32v203c8t6/ch32fun/adc_irq.elf"))
            .expect("CH32V203 ADC IRQ ELF");
    let cpu = configure_riscv(&mut bus);
    let mut machine = Machine::new(cpu, bus);
    machine.load_firmware(&image).expect("load ADC IRQ ELF");

    for _ in 0..8_000_000 {
        machine.step().expect("CH32V203 ADC IRQ instruction");
        if sink.lock().unwrap().ends_with(b"\r\n") {
            break;
        }
    }

    let out = String::from_utf8_lossy(&sink.lock().unwrap()).into_owned();
    (out, machine.cpu.mcause)
}

#[test]
fn ch32v203_adc1_eoc_reaches_qingke_pfic_irq_34_and_adc2_converts() {
    let chip: ChipDescriptor =
        serde_yaml::from_value(ch32v203_adc_yaml()).expect("descriptor with pad voltages");
    let (out, mcause) = run_ch32v203_adc_irq(&chip);
    // 12-bit codes against 3.3 V: 1650 mV -> 2047, VREFINT 1.2 V -> 1489,
    // 825 mV -> 1023. Two ADC1 conversions, two interrupts.
    assert_eq!(out, "ADC1_IN0=2047 VREF=1489 ADC2_IN1=1023 irqs=2\r\n");
    assert_eq!(mcause, 0x8000_0022, "last trap was ADC_IRQn 34");
}

#[test]
fn ch32v203_adc1_eoc_stays_unhandled_without_irq_34_wired() {
    let mut yaml = ch32v203_adc_yaml();
    let adc1 = yaml["peripherals"]
        .as_sequence_mut()
        .expect("peripherals list")
        .iter_mut()
        .find(|p| p["id"].as_str() == Some("adc1"))
        .expect("adc1 block");
    assert_eq!(adc1["irq"].as_u64(), Some(34), "ADC_IRQn is wired");
    adc1.as_mapping_mut()
        .unwrap()
        .remove(serde_yaml::Value::String("irq".to_owned()));
    let chip: ChipDescriptor = serde_yaml::from_value(yaml).expect("descriptor without ADC1 IRQ");

    let (out, _) = run_ch32v203_adc_irq(&chip);
    // No handler runs, so ADC1 reports a timeout; ADC2 is polled and still
    // converts.
    assert_eq!(out, "ADC_TIMEOUT in0=0 VREF=0 ADC2_IN1=1023 irqs=0\r\n");
}

/// Runs the CH32V203 I2C fixture against the two BME280s of
/// `examples/ch32v203c8t6/i2c-system.yaml`, returning the USART1 TX stream
/// and the last QingKe trap cause.
fn run_ch32v203_i2c_bme280(chip: &ChipDescriptor) -> (String, u32) {
    let manifest = SystemManifest::from_file(root().join("examples/ch32v203c8t6/i2c-system.yaml"))
        .expect("CH32V203 I2C system manifest");
    let mut bus =
        SystemBus::from_config(chip, &manifest).expect("CH32V203 peripherals and BME280s");
    let sink = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    bus.attach_uart_tx_sink(sink.clone(), false);

    let image =
        labwired_loader::load_elf(&root().join("examples/ch32v203c8t6/ch32fun/i2c_bme280.elf"))
            .expect("CH32V203 I2C ELF");
    let cpu = configure_riscv(&mut bus);
    let mut machine = Machine::new(cpu, bus);
    machine.load_firmware(&image).expect("load I2C ELF");

    for _ in 0..8_000_000 {
        machine.step().expect("CH32V203 I2C instruction");
        if sink.lock().unwrap().ends_with(b"\r\n") {
            break;
        }
    }

    let out = String::from_utf8_lossy(&sink.lock().unwrap()).into_owned();
    (out, machine.cpu.mcause)
}

#[test]
fn ch32v203_i2c1_polled_and_i2c2_event_irq_49_read_the_bme280s() {
    let row = CHIPS
        .iter()
        .find(|r| r.name == "CH32V203")
        .expect("CH32V203 row");
    let chip = descriptor(row);
    let (out, mcause) = run_ch32v203_i2c_bme280(&chip);
    // BME280 chip id 0x60 on both buses; dig_T1 = 28528 and the dig_T2 low
    // byte (26435 = 0x6743) from the model's calibration set. The I2C2 read
    // takes eight entries of the event handler.
    assert_eq!(
        out,
        "I2C1 id=60 dig_T1=28528 T2l=43 I2C2 id=60 ev_irqs=8\r\n"
    );
    assert_eq!(mcause, 0x8000_0031, "last trap was I2C2_EV_IRQn 49");
}

#[test]
fn ch32v203_i2c2_read_times_out_without_irq_49_wired() {
    let row = CHIPS
        .iter()
        .find(|r| r.name == "CH32V203")
        .expect("CH32V203 row");
    let text = std::fs::read_to_string(descriptor_path(row)).expect("read CH32V203 descriptor");
    let mut yaml: serde_yaml::Value = serde_yaml::from_str(&text).expect("parse chip descriptor");
    let i2c2 = yaml["peripherals"]
        .as_sequence_mut()
        .expect("peripherals list")
        .iter_mut()
        .find(|p| p["id"].as_str() == Some("i2c2"))
        .expect("i2c2 block");
    assert_eq!(i2c2["irq"].as_u64(), Some(49), "I2C2_EV_IRQn is wired");
    i2c2.as_mapping_mut()
        .unwrap()
        .remove(serde_yaml::Value::String("irq".to_owned()));
    let chip: ChipDescriptor = serde_yaml::from_value(yaml).expect("descriptor without I2C2 IRQ");

    let (out, _) = run_ch32v203_i2c_bme280(&chip);
    // No handler runs, so the I2C2 read never leaves its START; the polled
    // I2C1 reads are not affected.
    assert_eq!(
        out,
        "I2C1 id=60 dig_T1=28528 T2l=43 I2C2_TIMEOUT id=00 ev_irqs=0\r\n"
    );
}

/// Runs the CH32V203 SPI2 TX DMA fixture, returning the USART1 TX stream and
/// the last QingKe trap cause.
fn run_ch32v203_spi2_dma(chip: &ChipDescriptor) -> (String, u32) {
    let mut bus = SystemBus::from_config(chip, &SystemManifest::default())
        .expect("CH32V203 peripherals and clock gates");
    let sink = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    bus.attach_uart_tx_sink(sink.clone(), false);

    let image =
        labwired_loader::load_elf(&root().join("examples/ch32v203c8t6/ch32fun/spi_dma_tx.elf"))
            .expect("CH32V203 SPI DMA ELF");
    let cpu = configure_riscv(&mut bus);
    let mut machine = Machine::new(cpu, bus);
    machine.load_firmware(&image).expect("load SPI DMA ELF");

    for _ in 0..4_000_000 {
        machine.step().expect("CH32V203 SPI DMA instruction");
        if sink.lock().unwrap().ends_with(b"\r\n") {
            break;
        }
    }

    let out = String::from_utf8_lossy(&sink.lock().unwrap()).into_owned();
    (out, machine.cpu.mcause)
}

#[test]
fn ch32v203_spi2_tx_dma_reaches_qingke_pfic_irq_31() {
    let row = CHIPS
        .iter()
        .find(|r| r.name == "CH32V203")
        .expect("CH32V203 row");
    let chip = descriptor(row);
    let (out, mcause) = run_ch32v203_spi2_dma(&chip);
    assert_eq!(out, "SPI2_DMA events=HTHTHT laps=3 cntr=8\r\n");
    assert_eq!(mcause, 0x8000_001F, "last trap was DMA1_Channel5_IRQn 31");
}

#[test]
fn ch32v203_spi2_tx_dma_stays_unhandled_without_channel_irqs() {
    let row = CHIPS
        .iter()
        .find(|r| r.name == "CH32V203")
        .expect("CH32V203 row");
    let text = std::fs::read_to_string(descriptor_path(row)).expect("read CH32V203 descriptor");
    let mut yaml: serde_yaml::Value = serde_yaml::from_str(&text).expect("parse chip descriptor");
    let dma1 = yaml["peripherals"]
        .as_sequence_mut()
        .expect("peripherals list")
        .iter_mut()
        .find(|p| p["id"].as_str() == Some("dma1"))
        .expect("dma1 block");
    let config = dma1["config"].as_mapping_mut().expect("dma1 config");
    assert!(
        config
            .remove(serde_yaml::Value::String("channel_irqs".to_owned()))
            .is_some(),
        "DMA1 channel lines are wired"
    );
    let chip: ChipDescriptor = serde_yaml::from_value(yaml).expect("descriptor without DMA IRQs");

    let (out, mcause) = run_ch32v203_spi2_dma(&chip);
    assert!(
        out.starts_with("SPI2_DMA events= laps=0 cntr="),
        "no handler ran: {out:?}"
    );
    assert_eq!(mcause, 0, "unwired DMA1 must not enter the PFIC");
}

#[test]
fn every_ch32_usart1_stays_silent_without_its_apb2_clock() {
    // Negative control: move the USART1 gate to a clock-enable bit the
    // firmware never sets. TXE must then never assert, so nothing reaches the
    // sink.
    for row in CHIPS {
        let yaml = std::fs::read_to_string(descriptor_path(row))
            .unwrap_or_else(|e| panic!("{}: read descriptor: {e}", row.name));
        let gate = format!("clock: {{ reg: \"{}\", bit: 14 }}", row.gate_reg);
        assert_eq!(
            yaml.matches(&gate).count(),
            1,
            "{}: USART1 gate is unique",
            row.name
        );
        let moved = format!("clock: {{ reg: \"{}\", bit: 11 }}", row.gate_reg);
        let chip: ChipDescriptor = serde_yaml::from_str(&yaml.replace(&gate, &moved))
            .unwrap_or_else(|e| panic!("{}: parse modified descriptor: {e}", row.name));
        let (out, _) = run_uart_hello(row, &chip);
        assert!(
            out.is_empty(),
            "{}: unclocked USART1 printed {out:?}",
            row.name
        );
    }
}

#[test]
fn every_ch32_descriptor_matches_its_datasheet_table() {
    for row in CHIPS {
        let name = row.name;
        let chip = descriptor(row);
        assert_eq!(chip.core.as_deref(), Some(row.core), "{name}: core");
        assert_eq!(chip.cpu_hz, row.cpu_hz, "{name}: cpu_hz");
        assert_eq!(chip.flash.base, 0x0800_0000, "{name}: flash base");
        assert_eq!(chip.flash.size, row.flash_size, "{name}: flash size");
        assert_eq!(chip.ram.base, 0x2000_0000, "{name}: ram base");
        assert_eq!(chip.ram.size, row.ram_size, "{name}: ram size");
        let mut pins: Vec<&str> = chip.pins.keys().map(String::as_str).collect();
        pins.sort_unstable();
        let mut expected: Vec<String> = row
            .pins
            .iter()
            .flat_map(|(port, bits)| {
                let bits: Vec<u8> = match bits {
                    Bits::Range(lo, hi) => (*lo..*hi).collect(),
                    Bits::List(list) => list.to_vec(),
                };
                bits.into_iter().map(move |b| format!("{port}{b}"))
            })
            .collect();
        expected.sort_unstable();
        assert_eq!(
            pins,
            expected.iter().map(String::as_str).collect::<Vec<_>>(),
            "{name}: pin set"
        );
        assert_eq!(pins.len(), row.pin_count, "{name}: {}", row.pin_count_msg);
    }
}

#[test]
fn ch32x035_usart1_needs_the_x035_pad_map() {
    // Negative control: without `pad_map: wch_x035` the F1 table gates the
    // console on PA9, which this firmware never configures, so the PB10
    // output must not reach the sink. An unknown pad map must be rejected.
    let row = CHIPS
        .iter()
        .find(|r| r.name == "CH32X035")
        .expect("CH32X035 row");
    let yaml = std::fs::read_to_string(descriptor_path(row)).expect("read CH32X035 descriptor");
    let key = "pad_map: \"wch_x035\"";
    assert_eq!(yaml.matches(key).count(), 1, "USART1 pad map is unique");
    let chip: ChipDescriptor = serde_yaml::from_str(&yaml.replace(key, "pad_map: \"none\""))
        .expect("parse modified descriptor");
    let (out, _) = run_uart_hello(row, &chip);
    assert!(out.is_empty(), "F1 pad table printed {out:?}");

    let chip: ChipDescriptor = serde_yaml::from_str(&yaml.replace(key, "pad_map: \"bogus\""))
        .expect("parse modified descriptor");
    assert!(
        SystemBus::from_config(&chip, &SystemManifest::default()).is_err(),
        "unknown UART pad_map must fail the build"
    );
}

/// Unmodified ch32fun `examples/systick_irq` built for the CH32V203
/// (examples/ch32v203c8t6/build-ch32fun-examples.sh). It prints through the
/// WCH-LinkE debug channel, busy-waits Delay_Ms(100) on the 64-bit SysTick,
/// then enables SysTick_IRQn (12) in the PFIC; each handler entry adds
/// 144,000 to the 64-bit CMP and increments `systick_millis`. 15.5 M
/// instructions reach about 7 ms past the enable.
#[test]
fn ch32v203_ch32fun_systick_irq_counts_milliseconds_through_the_pfic() {
    let row = CHIPS
        .iter()
        .find(|r| r.name == "CH32V203")
        .expect("CH32V203 row");
    let chip = descriptor(row);
    let mut bus = SystemBus::from_config(&chip, &SystemManifest::default())
        .expect("CH32V203 peripherals and clock gates");
    let elf = root().join("examples/ch32v203c8t6/ch32fun/systick_irq.elf");
    let image = labwired_loader::load_elf(&elf).expect("ch32fun systick_irq ELF");
    let cpu = configure_riscv(&mut bus);
    let mut machine = Machine::new(cpu, bus);
    machine.load_firmware(&image).expect("load systick_irq ELF");
    for _ in 0..15_500_000 {
        machine.step().expect("ch32fun systick_irq instruction");
    }
    assert_eq!(
        machine.cpu.mcause & 0x7FFF_FFFF,
        12,
        "last trap was SysTick"
    );

    let logs = machine
        .bus
        .peripheral_logs("debug_printf")
        .expect("debug_printf block registered");
    let terminal = logs
        .iter()
        .find(|l| l.name == "terminal")
        .expect("terminal log")
        .lines()
        .join("\n");
    for line in [
        "systick_irq example",
        "initializing systick...done.",
        "Beginning Loop...",
    ] {
        assert!(terminal.contains(line), "missing {line:?} in {terminal:?}");
    }

    // `systick_millis` is ch32fun's first .bss word (0x20000000). Every
    // handler entry moved CMP on by one millisecond from its first value,
    // DELAY_MS_TIME - 1, so CMP and the count must agree.
    let millis = machine.bus.read_u32(0x2000_0000).expect("systick_millis");
    let cmp = u64::from(machine.bus.read_u32(0xE000_F010).unwrap())
        | (u64::from(machine.bus.read_u32(0xE000_F014).unwrap()) << 32);
    assert!(
        (5..=9).contains(&millis),
        "about 7 ms of 1 ms interrupts, got {millis}"
    );
    assert_eq!(
        cmp,
        143_999 + 144_000 * u64::from(millis),
        "one CMP step per counted interrupt"
    );
}

/// Runs unmodified ch32fun `examples_v20x/exti_blink` (PA1 rising edge on
/// EXTI1 toggles PA10 in the handler) and returns PA10 after boot and after
/// each of press, release, press on PA1, plus the final EXTI INTFR.
fn run_ch32v203_exti_blink(chip: &ChipDescriptor) -> ([bool; 4], u32) {
    let mut bus = SystemBus::from_config(chip, &SystemManifest::default())
        .expect("CH32V203 peripherals and clock gates");
    let gpioa = bus.find_peripheral_index_by_name("gpioa").expect("GPIOA");
    let elf = root().join("examples/ch32v203c8t6/ch32fun/exti_blink.elf");
    let image = labwired_loader::load_elf(&elf).expect("ch32fun exti_blink ELF");
    let cpu = configure_riscv(&mut bus);
    let mut machine = Machine::new(cpu, bus);
    machine.load_firmware(&image).expect("load exti_blink ELF");
    let mut pa10 = [false; 4];
    for (i, press) in [None, Some(true), Some(false), Some(true)]
        .into_iter()
        .enumerate()
    {
        if let Some(level) = press {
            machine.bus.set_peripheral_gpio_input(gpioa, 1, level);
        }
        for _ in 0..200_000 {
            machine.step().expect("ch32fun exti_blink instruction");
        }
        // GPIOA_OUTDR bit 10.
        pa10[i] = machine.bus.read_u32(0x4001_080C).unwrap() & (1 << 10) != 0;
    }
    (pa10, machine.bus.read_u32(0x4001_0414).unwrap())
}

#[test]
fn ch32v203_ch32fun_exti_blink_toggles_pa10_from_the_exti1_handler() {
    // EXTI1_IRQn is PFIC interrupt 23 (ch32v20xhw.h): the F1 vector 7 plus
    // the 16 QingKe core slots. Only rising edges toggle; INTFR is acked.
    let row = CHIPS
        .iter()
        .find(|r| r.name == "CH32V203")
        .expect("CH32V203 row");
    let (pa10, intfr) = run_ch32v203_exti_blink(&descriptor(row));
    assert_eq!(
        pa10,
        [true, false, false, true],
        "PA10 after boot/press/release/press"
    );
    assert_eq!(intfr, 0, "handler cleared INTFR");
}

#[test]
fn ch32v203_exti_blink_gets_no_interrupt_with_the_stm32f1_vectors() {
    // Negative control: the F1 layout pends vector 7 for line 1, which is a
    // QingKe core slot, so EXTI1_IRQHandler never runs and INTFR stays set.
    let row = CHIPS
        .iter()
        .find(|r| r.name == "CH32V203")
        .expect("CH32V203 row");
    let text = std::fs::read_to_string(descriptor_path(row)).expect("read CH32V203 descriptor");
    assert!(text.contains("profile: \"wch_v20x\""));
    let chip: ChipDescriptor =
        serde_yaml::from_str(&text.replace("profile: \"wch_v20x\"", "profile: \"stm32f1\""))
            .expect("descriptor with F1 EXTI vectors");
    let (pa10, intfr) = run_ch32v203_exti_blink(&chip);
    assert_eq!(pa10, [true; 4], "PA10 never toggles");
    assert_eq!(intfr, 1 << 1, "line 1 stays pending");
}
