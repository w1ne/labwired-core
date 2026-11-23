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
    let chip = ChipDescriptor::from_file(root.join("configs/chips/ch32v003f4p6.yaml"))
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

// CH32V003 package variants. One die in four packages (WCH CH32V003DS0 V1.8,
// Table 2-1 "Pin definitions"): each variant descriptor is the F4P6 one with
// a different `package`, `pin_count` and bonded `pins` map.
const PACKAGE_VARIANTS: &[(&str, &str, u32, &[&str])] = &[
    (
        "ch32v003f4u6",
        "QFN20",
        20,
        &[
            "PA1", "PA2", "PC0", "PC1", "PC2", "PC3", "PC4", "PC5", "PC6", "PC7", "PD0", "PD1",
            "PD2", "PD3", "PD4", "PD5", "PD6", "PD7",
        ],
    ),
    (
        "ch32v003a4m6",
        "SOP16",
        16,
        &[
            "PA1", "PA2", "PC0", "PC1", "PC2", "PC3", "PC4", "PC6", "PC7", "PD1", "PD4", "PD5",
            "PD6", "PD7",
        ],
    ),
    (
        "ch32v003j4m6",
        "SOP8",
        8,
        &[
            "PA1", "PA2", "PC1", "PC2", "PC4", "PD1", "PD4", "PD5", "PD6",
        ],
    ),
];

fn chip_yaml(name: &str) -> serde_yaml::Value {
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let text = std::fs::read_to_string(root.join(format!("configs/chips/{name}.yaml")))
        .unwrap_or_else(|e| panic!("read {name}: {e}"));
    serde_yaml::from_str(&text).unwrap_or_else(|e| panic!("parse {name}: {e}"))
}

fn load_chip(name: &str) -> ChipDescriptor {
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    ChipDescriptor::from_file(root.join(format!("configs/chips/{name}.yaml")))
        .unwrap_or_else(|e| panic!("{name} descriptor: {e}"))
}

/// Steps `fixture` on `chip` and counts output edges on GPIOC bit `bit`.
fn gpioc_edges(chip: &ChipDescriptor, fixture: &str, bit: u8) -> (u32, u32) {
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut bus = SystemBus::from_config(chip, &SystemManifest::default())
        .expect("CH32V003 peripherals and clock gates");
    let image = labwired_loader::load_elf(&root.join("tests/fixtures").join(fixture))
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
        if let Some(state) = machine.bus.peripherals[gpioc].dev.read_gpio_output(bit) {
            if previous.is_some_and(|prior| prior != state) {
                edges += 1;
            }
            previous = Some(state);
            if edges >= 2 {
                break;
            }
        }
    }
    (edges, machine.cpu.mcause)
}

#[test]
fn ch32v003_package_variants_match_ds0_bonding_on_the_f4p6_die() {
    let strip = |mut v: serde_yaml::Value| {
        let map = v.as_mapping_mut().expect("descriptor is a mapping");
        for key in ["name", "package", "pin_count", "pins"] {
            map.remove(key);
        }
        v
    };
    let die = strip(chip_yaml("ch32v003f4p6"));
    for (name, package, pin_count, bonded) in PACKAGE_VARIANTS {
        let yaml = chip_yaml(name);
        assert_eq!(yaml["name"].as_str(), Some(*name));
        assert_eq!(yaml["package"].as_str(), Some(*package), "{name} package");
        assert_eq!(yaml["pin_count"].as_u64(), Some(u64::from(*pin_count)));
        let pins: Vec<&str> = yaml["pins"]
            .as_mapping()
            .expect("pins map")
            .keys()
            .map(|k| k.as_str().expect("pin name"))
            .collect();
        assert_eq!(&pins, bonded, "{name} bonded GPIOs (DS0 Table 2-1)");
        assert_eq!(
            strip(yaml),
            die,
            "{name} must keep the F4P6 die: memory, clock and peripherals"
        );
    }
}

#[test]
fn ch32v003_package_variants_print_on_usart1_pd5() {
    // PD5 (USART1 TX, default mapping) is bonded on every package.
    for (name, _, _, bonded) in PACKAGE_VARIANTS {
        assert!(bonded.contains(&"PD5"), "{name} bonds PD5");
        let (out, mcause) = run_uart_hello(&load_chip(name));
        assert_eq!(
            String::from_utf8_lossy(&out),
            "Hello from CH32V003\r\n",
            "{name} USART1 TX stream"
        );
        assert_eq!(mcause, 0, "{name}: compiled firmware must not trap");
    }
}

#[test]
fn ch32v003_package_variants_toggle_a_bonded_gpioc_pin() {
    // F4U6 and A4M6 bond PC0, so the F4P6 blink runs unchanged. The SOP-8
    // J4M6 has no PC0 pad; its own fixture blinks PC4 (package pin 7).
    for (name, fixture, bit) in [
        ("ch32v003f4u6", "ch32v003f4p6-blink.elf", 0u8),
        ("ch32v003a4m6", "ch32v003f4p6-blink.elf", 0),
        ("ch32v003j4m6", "ch32v003j4m6-blink-pc4.elf", 4),
    ] {
        let bonded = PACKAGE_VARIANTS
            .iter()
            .find(|v| v.0 == name)
            .expect("variant listed")
            .3;
        assert!(
            bonded.contains(&format!("PC{bit}").as_str()),
            "{name} bonds PC{bit}"
        );
        let (edges, mcause) = gpioc_edges(&load_chip(name), fixture, bit);
        assert!(edges >= 2, "{name}: only {edges} PC{bit} edges");
        assert_eq!(mcause, 0, "{name}: compiled firmware must not trap");
    }
}

#[test]
fn ch32v003j4m6_blink_fixture_leaves_unbonded_pc0_alone() {
    // Negative control for the PC4 fixture: PC0 must not toggle, so the
    // edge count above is not satisfied by an unbonded pin.
    let (edges, _) = gpioc_edges(&load_chip("ch32v003j4m6"), "ch32v003j4m6-blink-pc4.elf", 0);
    assert_eq!(edges, 0, "PC4 fixture toggled PC0 {edges} times");
}

/// Runs the stock WCH noneos-sdk fixture (PlatformIO `ch32v`,
/// `-march=rv32ecxw`) until it prints `CH32V003_DONE` or `max_steps` pass.
fn run_noneos_sdk(chip: &ChipDescriptor, max_steps: u64) -> (String, u64, u32) {
    run_sdk_fixture(chip, "ch32v003f4p6-noneos-sdk.elf", max_steps)
}

/// Runs a stock noneos-sdk `fixture` until its output holds a complete `DONE`
/// line or `max_steps` pass.
fn run_sdk_fixture(chip: &ChipDescriptor, fixture: &str, max_steps: u64) -> (String, u64, u32) {
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut bus = SystemBus::from_config(chip, &SystemManifest::default())
        .expect("CH32V003 peripherals and clock gates");
    let sink = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    bus.attach_uart_tx_sink(sink.clone(), false);
    let image = labwired_loader::load_elf(&root.join("tests/fixtures").join(fixture))
        .expect("noneos-sdk ELF");
    let cpu = configure_riscv(&mut bus);
    let mut machine = Machine::new(cpu, bus);
    machine.load_firmware(&image).expect("load CH32V003 ELF");
    let mut steps = 0;
    while steps < max_steps {
        if machine.step().is_err() {
            break;
        }
        steps += 1;
        if steps % 10_000 == 0 {
            let out = sink.lock().unwrap();
            if String::from_utf8_lossy(&out).contains("DONE") && out.ends_with(b"\n") {
                break;
            }
        }
    }
    let out = String::from_utf8_lossy(&sink.lock().unwrap()).into_owned();
    (out, steps, machine.cpu.mcause)
}

#[test]
fn ch32v003_stock_noneos_sdk_firmware_boots_delays_and_prints() {
    // Vendor startup (QingKe INTSYSCR CSR 0x804), SystemInit (FLASH ACTLR,
    // VENDOR_CFG0 trim byte, HSI PLL to 48 MHz), XW compressed loads, and
    // Delay_Ms(10) x 4 polling STK SR.CNTIF between USART1 printf lines.
    let chip = load_chip("ch32v003f4p6");
    let (out, steps, mcause) = run_noneos_sdk(&chip, 4_000_000);
    assert_eq!(
        out,
        "CH32V003_BOOT clk=48000000\r\ntick 0\r\ntick 1\r\ntick 2\r\ntick 3\r\nCH32V003_DONE\r\n",
        "USART1 TX stream"
    );
    assert_eq!(mcause, 0, "stock firmware must not trap");
    // Four 10 ms delays at 48 MHz are 1,920,000 cycles; the stop check runs
    // every 10,000 steps.
    assert!(
        (1_920_000..2_000_000).contains(&steps),
        "4 x Delay_Ms(10) took {steps} steps"
    );
}

#[test]
fn ch32v003_noneos_sdk_delay_stalls_without_the_stk_block() {
    // Negative control: drop the STK block. Delay_Ms then reads an unmapped
    // 0xE000F004 and the firmware never reaches its first tick line.
    let mut yaml = chip_yaml("ch32v003f4p6");
    let peripherals = yaml["peripherals"]
        .as_sequence_mut()
        .expect("peripherals list");
    let before = peripherals.len();
    peripherals.retain(|p| p["id"].as_str() != Some("stk"));
    assert_eq!(
        peripherals.len(),
        before - 1,
        "descriptor has one stk block"
    );
    let chip: ChipDescriptor = serde_yaml::from_value(yaml).expect("descriptor without stk");
    let (out, _, _) = run_noneos_sdk(&chip, 4_000_000);
    assert_eq!(out, "CH32V003_BOOT clk=48000000\r\n");
}

const STK_IRQ_FIXTURE: &str = "ch32v003f4p6-noneos-sdk-stk-irq.elf";

#[test]
fn ch32v003_stock_sdk_systick_interrupt_runs_through_the_pfic() {
    // mtvec = _start | 3 (absolute vector table), INTSYSCR = HWSTKEN |
    // INESTEN, NVIC_EnableIRQ(SysTicK_IRQn) and a 1 kHz STK match with STIE.
    // The `WCH-Interrupt-fast` handler clobbers a5 without saving it; the
    // hardware prologue/epilogue keeps main's loop registers intact.
    let chip = load_chip("ch32v003f4p6");
    let (out, steps, mcause) = run_sdk_fixture(&chip, STK_IRQ_FIXTURE, 2_000_000);
    assert_eq!(
        out, "STK_IRQ_BOOT\r\nSTK_IRQ_TICKS 5\r\nSTK_IRQ_MASKED ok\r\nSTK_IRQ_DONE 1\r\n",
        "USART1 TX stream"
    );
    assert_eq!(mcause, 0x8000_000C, "last trap was SysTick_IRQn");
    // Five 1 ms ticks (240,000 cycles) plus the 3 ms masked delay (144,000).
    assert!(
        (384_000..700_000).contains(&steps),
        "five ticks and Delay_Ms(3) took {steps} steps"
    );
}

#[test]
fn ch32v003_systick_interrupt_never_enters_without_the_stk_irq_line() {
    // Negative control: drop `irq: 12` from the STK. The match still sets
    // CNTIF, but nothing pends the PFIC, so `ticks` stays 0 and the wait
    // loop never ends.
    let mut yaml = chip_yaml("ch32v003f4p6");
    let stk = yaml["peripherals"]
        .as_sequence_mut()
        .expect("peripherals list")
        .iter_mut()
        .find(|p| p["id"].as_str() == Some("stk"))
        .expect("stk block");
    assert_eq!(
        stk["irq"].as_u64(),
        Some(12),
        "descriptor wires SysTick_IRQn"
    );
    stk.as_mapping_mut().unwrap().remove("irq");
    let chip: ChipDescriptor = serde_yaml::from_value(yaml).expect("descriptor without irq");
    let (out, _, mcause) = run_sdk_fixture(&chip, STK_IRQ_FIXTURE, 1_000_000);
    assert_eq!(out, "STK_IRQ_BOOT\r\n");
    assert_eq!(mcause, 0, "no trap");
}

const USART_IRQ_FIXTURE: &str = "ch32v003f4p6-noneos-sdk-usart-irq.elf";

/// Run the USART1 RX interrupt fixture with `rx` queued on USART1's input.
fn run_usart_irq_fixture(chip: &ChipDescriptor, rx: &[u8], max_steps: u64) -> (String, u32) {
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut bus = SystemBus::from_config(chip, &SystemManifest::default())
        .expect("CH32V003 peripherals and clock gates");
    let sink = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    bus.attach_uart_tx_sink(sink.clone(), false);
    let source = bus
        .attach_uart_rx_source_named("usart1")
        .expect("usart1 rx source");
    source.lock().unwrap().extend(rx);
    let image = labwired_loader::load_elf(&root.join("tests/fixtures").join(USART_IRQ_FIXTURE))
        .expect("noneos-sdk ELF");
    let cpu = configure_riscv(&mut bus);
    let mut machine = Machine::new(cpu, bus);
    machine.load_firmware(&image).expect("load CH32V003 ELF");
    for step in 1..=max_steps {
        if machine.step().is_err() {
            break;
        }
        if step % 10_000 == 0 {
            let out = sink.lock().unwrap();
            if String::from_utf8_lossy(&out).contains("DONE") && out.ends_with(b"\n") {
                break;
            }
        }
    }
    let out = String::from_utf8_lossy(&sink.lock().unwrap()).into_owned();
    (out, machine.cpu.mcause)
}

#[test]
fn ch32v003_stock_sdk_usart1_rx_interrupt_runs_through_the_pfic() {
    // USART_ITConfig(USART1, USART_IT_RXNE) + NVIC_EnableIRQ(USART1_IRQn).
    // The bytes are queued before reset, so the four interrupts land inside
    // the SDK printf of the boot line. The `WCH-Interrupt-fast` handler calls
    // USART_GetITStatus and USART_ReceiveData without saving ra; HPE
    // restores it on mret.
    let chip = load_chip("ch32v003f4p6");
    let (out, mcause) = run_usart_irq_fixture(&chip, b"wch!", 2_000_000);
    assert_eq!(
        out, "USART_IRQ_BOOT\r\nUSART_IRQ_RX wch!\r\nUSART_IRQ_DONE\r\n",
        "USART1 TX stream"
    );
    assert_eq!(mcause, 0x8000_0020, "last trap was USART1_IRQn");
}

#[test]
fn ch32v003_usart1_rx_interrupt_never_enters_without_the_irq_line() {
    // Negative control: drop `irq: 32` from USART1. RXNE still sets, but
    // nothing pends the PFIC, so the handler never reads DATAR.
    let mut yaml = chip_yaml("ch32v003f4p6");
    let usart = yaml["peripherals"]
        .as_sequence_mut()
        .expect("peripherals list")
        .iter_mut()
        .find(|p| p["id"].as_str() == Some("usart1"))
        .expect("usart1 block");
    assert_eq!(
        usart["irq"].as_u64(),
        Some(32),
        "descriptor wires USART1_IRQn"
    );
    usart.as_mapping_mut().unwrap().remove("irq");
    let chip: ChipDescriptor = serde_yaml::from_value(yaml).expect("descriptor without irq");
    let (out, mcause) = run_usart_irq_fixture(&chip, b"wch!", 1_000_000);
    assert_eq!(out, "USART_IRQ_BOOT\r\n");
    assert_eq!(mcause, 0, "no trap");
}

const ARDUINO_FIXTURE: &str = "ch32v003f4p6-arduino-openwch.elf";

#[test]
fn ch32v003_official_openwch_arduino_sketch_runs_unmodified() {
    // Stock openwch arduino_core_ch32 build: Serial on USART1, pinMode /
    // digitalWrite / digitalRead on PC0, and delay()/millis() counted by the
    // core's `WCH-Interrupt-fast` SysTick_Handler through the PFIC. The
    // sketch prints PASS only when millis() measures 4 x delay(10) as
    // 40..42 ms.
    let chip = load_chip("ch32v003f4p6");
    let (out, steps, mcause) = run_sdk_fixture(&chip, ARDUINO_FIXTURE, 4_000_000);
    assert_eq!(
        out,
        "CH32V003_ARDUINO_BOOT\r\n\
         tick 0 pc0=0\r\ntick 1 pc0=1\r\ntick 2 pc0=0\r\ntick 3 pc0=1\r\n\
         elapsed_ms=40\r\nCH32V003_ARDUINO_PASS\r\nCH32V003_ARDUINO_DONE\r\n",
        "USART1 TX stream"
    );
    assert_eq!(mcause, 0x8000_000C, "last trap was SysTick_IRQn");
    // Four 10 ms delays at 48 MHz are 1,920,000 cycles.
    assert!(
        (1_920_000..2_200_000).contains(&steps),
        "4 x delay(10) took {steps} steps"
    );
}

#[test]
fn ch32v003_arduino_delay_never_returns_without_the_stk_irq_line() {
    // Negative control: drop `irq: 12` from the STK. msTick never advances,
    // so the first delay(10) spins for ever after the boot line.
    let mut yaml = chip_yaml("ch32v003f4p6");
    let stk = yaml["peripherals"]
        .as_sequence_mut()
        .expect("peripherals list")
        .iter_mut()
        .find(|p| p["id"].as_str() == Some("stk"))
        .expect("stk block");
    stk.as_mapping_mut().unwrap().remove("irq");
    let chip: ChipDescriptor = serde_yaml::from_value(yaml).expect("descriptor without irq");
    let (out, _, mcause) = run_sdk_fixture(&chip, ARDUINO_FIXTURE, 1_000_000);
    assert_eq!(out, "CH32V003_ARDUINO_BOOT\r\n");
    assert_eq!(mcause, 0, "no trap");
}

const EXTI_FIXTURE: &str = "ch32v003f4p6-arduino-openwch-exti.elf";

/// Run the openwch `attachInterrupt` fixture: hold PD3 high, give three
/// falling pulses after the boot line, then type `wch\n` on USART1 once the
/// sketch reports its count.
fn run_exti_fixture(chip: &ChipDescriptor) -> String {
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut bus = SystemBus::from_config(chip, &SystemManifest::default())
        .expect("CH32V003 peripherals and clock gates");
    let sink = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    bus.attach_uart_tx_sink(sink.clone(), false);
    let source = bus
        .attach_uart_rx_source_named("usart1")
        .expect("usart1 rx source");
    let gpiod = bus.find_peripheral_index_by_name("gpiod").expect("GPIOD");
    bus.set_peripheral_gpio_input(gpiod, 3, true);
    let image = labwired_loader::load_elf(&root.join("tests/fixtures").join(EXTI_FIXTURE))
        .expect("openwch Arduino ELF");
    let cpu = configure_riscv(&mut bus);
    let mut machine = Machine::new(cpu, bus);
    machine.load_firmware(&image).expect("load CH32V003 ELF");
    let text = |sink: &std::sync::Mutex<Vec<u8>>| {
        String::from_utf8_lossy(&sink.lock().unwrap()).into_owned()
    };
    let mut pulses = 0;
    let mut typed = false;
    for step in 1..=6_000_000u64 {
        if machine.step().is_err() {
            break;
        }
        if step % 5_000 != 0 {
            continue;
        }
        let out = text(&sink);
        if out.contains("BOOT\r\n") && pulses < 6 {
            // Low on even slots, high on odd: three falling edges.
            machine
                .bus
                .set_peripheral_gpio_input(gpiod, 3, pulses % 2 == 1);
            pulses += 1;
        }
        if !typed && out.contains("falls=") && out.ends_with('\n') {
            source.lock().unwrap().extend(b"wch\n");
            typed = true;
        }
        if out.contains("DONE") && out.ends_with('\n') {
            break;
        }
    }
    text(&sink)
}

#[test]
fn ch32v003_openwch_attach_interrupt_and_serial_read_run_unmodified() {
    // Stock openwch arduino_core_ch32: attachInterrupt(PD3, IPU, FALLING)
    // writes AFIO EXTICR (2 bits per line, GPIOD = 3) and EXTI INTENR /
    // FTENR, and enables EXTI7_0_IRQn (20). The core's `WCH-Interrupt-fast`
    // EXTI7_0_IRQHandler clears INTFR and calls the sketch callback.
    // Serial.read() polls USART1 RXNE.
    let chip = load_chip("ch32v003f4p6");
    let out = run_exti_fixture(&chip);
    assert_eq!(
        out,
        "CH32V003_EXTI_BOOT\r\nfalls=3\r\nrx=wch\r\n\
         CH32V003_EXTI_PASS\r\nCH32V003_EXTI_DONE\r\n",
        "USART1 TX stream"
    );
}

#[test]
fn ch32v003_attach_interrupt_misses_pd3_with_the_stm32f1_afio_mux() {
    // Negative control: the STM32F1 AFIO reads EXTICR as 4-bit fields, so
    // the V00x write `3 << 6` (line 3 = GPIOD) selects port A for line 3 and
    // no PD3 edge reaches EXTI.
    let mut yaml = chip_yaml("ch32v003f4p6");
    let afio = yaml["peripherals"]
        .as_sequence_mut()
        .expect("peripherals list")
        .iter_mut()
        .find(|p| p["id"].as_str() == Some("afio"))
        .expect("afio block");
    assert_eq!(afio["config"]["profile"].as_str(), Some("wch_v00x"));
    afio["config"]["profile"] = "stm32f1".into();
    let chip: ChipDescriptor = serde_yaml::from_value(yaml).expect("descriptor with F1 AFIO");
    let out = run_exti_fixture(&chip);
    assert!(
        out.starts_with("CH32V003_EXTI_BOOT\r\nfalls=0\r\n"),
        "{out}"
    );
    assert!(out.contains("CH32V003_EXTI_FAIL"), "{out}");
}

const ADC_FIXTURE: &str = "ch32v003f4p6-arduino-openwch-adc.elf";

/// Run the openwch `analogRead` fixture with PA2 (ADC1_IN0) held at 1650 mV
/// and PD4 (ADC1_IN7) at 3000 mV against the 3.3 V reference.
fn run_adc_fixture(chip: &ChipDescriptor) -> String {
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut bus = SystemBus::from_config(chip, &SystemManifest::default())
        .expect("CH32V003 peripherals and clock gates");
    let sink = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    bus.attach_uart_tx_sink(sink.clone(), false);
    let adc1 = bus.find_peripheral_index_by_name("adc1").expect("ADC1");
    let adc = bus.peripherals[adc1]
        .dev
        .as_any_mut()
        .and_then(|any| any.downcast_mut::<labwired_core::peripherals::adc::Adc>())
        .expect("ADC1 is the F1-layout model");
    adc.set_channel_input(0, 1650);
    adc.set_channel_input(7, 3000);
    let image = labwired_loader::load_elf(&root.join("tests/fixtures").join(ADC_FIXTURE))
        .expect("openwch Arduino ELF");
    let cpu = configure_riscv(&mut bus);
    let mut machine = Machine::new(cpu, bus);
    machine.load_firmware(&image).expect("load CH32V003 ELF");
    let text = |sink: &std::sync::Mutex<Vec<u8>>| {
        String::from_utf8_lossy(&sink.lock().unwrap()).into_owned()
    };
    for step in 1..=2_000_000u64 {
        if machine.step().is_err() {
            break;
        }
        if step % 5_000 == 0 {
            let out = text(&sink);
            if out.contains("DONE") && out.ends_with('\n') {
                break;
            }
        }
    }
    text(&sink)
}

#[test]
fn ch32v003_openwch_analog_read_returns_10_bit_codes() {
    // Stock openwch arduino_core_ch32: analogRead() enables RCC_ADC1EN, sets
    // ADON, writes RSQR3 rank 1, sets CTLR2.SWSTART, polls STATR.EOC and
    // returns RDATAR unchanged (variant ADC_RESOLUTION 10).
    // 1650 mV -> floor(1650 * 4095 / 3300) >> 2 = 511;
    // 3000 mV -> floor(3000 * 4095 / 3300) >> 2 = 930.
    let chip = load_chip("ch32v003f4p6");
    let out = run_adc_fixture(&chip);
    assert_eq!(
        out,
        "CH32V003_ADC_BOOT\r\npa2=511\r\npd4=930\r\n\
         CH32V003_ADC_PASS\r\nCH32V003_ADC_DONE\r\n",
        "USART1 TX stream"
    );
}

#[test]
fn ch32v003_analog_read_overflows_10_bits_with_the_stm32f1_adc() {
    // Negative control: the plain STM32F1 profile converts at 12 bits, so
    // RDATAR exceeds the 10-bit range the openwch V00x core returns as is.
    let mut yaml = chip_yaml("ch32v003f4p6");
    let adc = yaml["peripherals"]
        .as_sequence_mut()
        .expect("peripherals list")
        .iter_mut()
        .find(|p| p["id"].as_str() == Some("adc1"))
        .expect("adc1 block");
    assert_eq!(adc["config"]["profile"].as_str(), Some("wch_v00x"));
    adc["config"]["profile"] = "stm32f1".into();
    let chip: ChipDescriptor = serde_yaml::from_value(yaml).expect("descriptor with F1 ADC");
    let out = run_adc_fixture(&chip);
    assert_eq!(
        out,
        "CH32V003_ADC_BOOT\r\npa2=2047\r\npd4=3722\r\n\
         CH32V003_ADC_FAIL\r\nCH32V003_ADC_DONE\r\n",
        "USART1 TX stream"
    );
}
