// Compiled-firmware contracts shared by the CH32 chips whose tests used to be
// five line-for-line per-chip copies (V005, V006, V203, X035, L103). The chips
// differ only in DATA, which lives in `CHIPS`; each `#[test]` below is one
// contract looped over every row, with the chip name in every assertion
// message. `ch32v003_compiled_firmware.rs` is a different shape and stays
// separate.
use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::{bus::SystemBus, system::riscv::configure_riscv, Machine};

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
        // GPIOs.
        core: "rv32imac",
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
