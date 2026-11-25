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

/// The descriptor's `profile: wch_v00x` must reach the model: 10 channels
/// (IN0..IN9, DS0 Table 3-2), not the STM32F1's 19. The IN16 answer is covered
/// by `peripherals::adc::tests::wch_v00x_adc_has_no_f1_internal_channels`.
#[test]
fn ch32v003_adc1_is_the_ten_channel_wch_layout() {
    let chip = load_chip("ch32v003f4p6");
    let bus = SystemBus::from_config(&chip, &SystemManifest::default())
        .expect("CH32V003 peripherals and clock gates");
    let adc1 = bus.find_peripheral_index_by_name("adc1").expect("ADC1");
    assert_eq!(
        bus.peripherals[adc1].dev.adc_channel_count(),
        Some(10),
        "CH32V00x ADC converts IN0..IN9"
    );
}

#[test]
fn ch32v003_package_variants_match_ds0_bonding_on_the_f4p6_die() {
    let strip = |mut v: serde_yaml::Value| {
        let map = v.as_mapping_mut().expect("descriptor is a mapping");
        // `analog_pins` is a pin-level map like `pins`: a package that bonds
        // fewer pads names fewer channels on the same die.
        for key in ["name", "package", "pin_count", "pins", "analog_pins"] {
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
        for pad in yaml["analog_pins"]
            .as_mapping()
            .expect("analog_pins map")
            .keys()
        {
            let pad = pad.as_str().expect("pad name");
            assert!(
                bonded.contains(&pad),
                "{name} routes analog input from {pad}, which it does not bond"
            );
        }
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
        assert_no_fault(mcause);
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
        assert_no_fault(mcause);
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

const ADC_IRQ_FIXTURE: &str = "ch32v003f4p6-noneos-sdk-adc-irq.elf";

/// Run the noneos-sdk `ADC1_IRQHandler` fixture with PA2 (ADC1_IN0) held at
/// 1650 mV against the 3.3 V reference. The handler is the only reader of
/// STATR: if the ADC never raises ADC_IRQn (29) through the PFIC, `eoc_count`
/// stays 0 and the firmware prints `ADC_IRQ_EOC 0 code 0` after its 2 ms wait.
fn run_adc_irq_fixture(with_irq_line: bool) -> String {
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut yaml = chip_yaml("ch32v003f4p6");
    if !with_irq_line {
        let adc = yaml["peripherals"]
            .as_sequence_mut()
            .expect("peripherals list")
            .iter_mut()
            .find(|p| p["id"].as_str() == Some("adc1"))
            .expect("adc1 block");
        adc.as_mapping_mut().expect("adc block").remove("irq");
    }
    let chip: ChipDescriptor = serde_yaml::from_value(yaml).expect("descriptor");
    let mut bus = SystemBus::from_config(&chip, &SystemManifest::default())
        .expect("CH32V003 peripherals and clock gates");
    let sink = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    bus.attach_uart_tx_sink(sink.clone(), false);
    assert!(bus.set_analog_pad_mv("PA2", 1650), "PA2 drives ADC1_IN0");
    let image = labwired_loader::load_elf(&root.join("tests/fixtures").join(ADC_IRQ_FIXTURE))
        .expect("noneos-sdk ADC IRQ ELF");
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
        if step % 20_000 == 0 {
            let out = text(&sink);
            if out.contains("ADC_IRQ_DONE") {
                break;
            }
        }
    }
    text(&sink)
}

/// Stock noneos-sdk `ADC_SoftwareStartConvCmd` + `ADC_IT_EOC`: STATR.EOC must
/// raise ADC_IRQn (29) through the PFIC, and `NVIC_DisableIRQ` must stop it.
/// The firmware (ip `docs/audits/ch32v003-adc-irq/main.c`) never polls STATR.
#[test]
fn ch32v003_adc_eoc_interrupt_enters_the_handler() {
    let out = run_adc_irq_fixture(true);
    assert_eq!(
        out, "ADC_IRQ_BOOT\r\nADC_IRQ_EOC 1 code 511\r\nADC_IRQ_MASKED ok\r\nADC_IRQ_DONE\r\n",
        "USART1 TX stream"
    );
}

#[test]
fn ch32v003_adc_eoc_interrupt_never_enters_without_the_irq_line() {
    let out = run_adc_irq_fixture(false);
    assert_eq!(
        out, "ADC_IRQ_BOOT\r\nADC_IRQ_EOC 0 code 0\r\nADC_IRQ_MASKED ok\r\nADC_IRQ_DONE\r\n",
        "without adc1 irq the handler never runs: count 0 after the 2 ms wait"
    );
}

/// Run the openwch `analogRead` fixture with PA2 (ADC1_IN0) held at 1650 mV
/// and PD4 (ADC1_IN7) at 3000 mV against the 3.3 V reference.
fn run_adc_fixture(chip: &ChipDescriptor) -> String {
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut bus = SystemBus::from_config(chip, &SystemManifest::default())
        .expect("CH32V003 peripherals and clock gates");
    let sink = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    bus.attach_uart_tx_sink(sink.clone(), false);
    // Through the descriptor's `analog_pins:` map, the same path a user drives:
    // PA2 is ADC1_IN0, PD4 is ADC1_IN7. No test reaches into the concrete model.
    assert!(bus.set_analog_pad_mv("PA2", 1650), "PA2 drives ADC1_IN0");
    assert!(bus.set_analog_pad_mv("PD4", 3000), "PD4 drives ADC1_IN7");
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

const PWM_FIXTURE: &str = "ch32v003f4p6-arduino-openwch-pwm.elf";

/// Step the openwch `analogWrite` fixture until setup finishes, then record
/// `cycles` of steady-state pad edges on each `(port, pin)`. Returns the USART1
/// text, one edge list per pad (watch-set order), the pad's routed function
/// name after the sketch ran, and the trap cause.
fn run_pwm_fixture(
    chip: &ChipDescriptor,
    pads: &[(&str, u8)],
    cycles: u64,
) -> (
    String,
    Vec<Vec<labwired_core::logic_capture::LogicEdge>>,
    Vec<Option<String>>,
    u32,
) {
    use labwired_core::logic_capture::LogicSource;

    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut bus = SystemBus::from_config(chip, &SystemManifest::default())
        .expect("CH32V003 peripherals and clock gates");
    let sink = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    bus.attach_uart_tx_sink(sink.clone(), false);
    let image = labwired_loader::load_elf(&root.join("tests/fixtures").join(PWM_FIXTURE))
        .expect("openwch analogWrite ELF");

    let cpu = configure_riscv(&mut bus);
    let mut machine = Machine::new(cpu, bus);
    machine
        .load_firmware(&image)
        .expect("load CH32V003 PWM ELF");

    let mut sources = Vec::new();
    let mut routed = Vec::new();
    for (name, pin) in pads {
        let idx = machine
            .bus
            .find_peripheral_index_by_name(name)
            .unwrap_or_else(|| panic!("{name} registered from descriptor"));
        sources.push(Some(LogicSource::pad(idx, *pin)));
        routed.push((idx, *pin));
    }
    machine.logic_watch(&sources);

    let done = b"CH32V003_PWM_DONE";
    let reached_done = |out: &[u8]| out.windows(done.len()).any(|w| w == done);
    let mut elapsed = 0u32;
    while elapsed < 400_000 {
        machine.step().expect("Arduino PWM firmware instruction");
        elapsed += 1;
        if reached_done(&sink.lock().unwrap()) {
            break;
        }
    }
    // The setup turn-on transient is not a period: acknowledge every edge so
    // far, then measure only the steady-state window.
    let cursor = machine.logic_read_edges(0).cursor;
    let target = machine.total_cycles + cycles;
    while machine.total_cycles < target {
        machine.step().expect("PWM steady-state instruction");
    }
    let batch = machine.logic_read_edges(cursor);
    let mut per_pad = vec![Vec::new(); pads.len()];
    for edge in batch.edges {
        per_pad[edge.ch as usize].push(edge);
    }

    // Read USART1 after the steady-state window: the newline that follows
    // PWM_DONE is one more transmitted byte, not part of setup.
    let out = String::from_utf8_lossy(&sink.lock().unwrap()).to_string();
    assert!(
        reached_done(out.as_bytes()),
        "sketch stopped before PWM_DONE: {out:?}"
    );

    let funcs = routed
        .iter()
        .map(|&(idx, pin)| {
            machine.bus.peripherals[idx]
                .dev
                .gpio_routing(pin)
                .and_then(|r| r.func)
        })
        .collect();
    (out, per_pad, funcs, machine.cpu.mcause)
}

/// The pad waveform as (period, high time) in engine cycles, decoded from the
/// recorded edges alone. Requires at least two whole periods and checks every
/// one of them, so one lucky period cannot pass.
fn period_and_high(edges: &[labwired_core::logic_capture::LogicEdge]) -> (u64, u64) {
    let mut last: Option<bool> = None;
    let edges: Vec<labwired_core::logic_capture::LogicEdge> = edges
        .iter()
        .filter(|e| {
            let keep = last != Some(e.value);
            last = Some(e.value);
            keep
        })
        .copied()
        .collect();
    let rises: Vec<u64> = edges.iter().filter(|e| e.value).map(|e| e.cycle).collect();
    let falls: Vec<u64> = edges.iter().filter(|e| !e.value).map(|e| e.cycle).collect();
    assert!(
        rises.len() >= 3,
        "need at least three rising edges to measure two periods, got {edges:?}"
    );
    let period = rises[1] - rises[0];
    for pair in rises.windows(2) {
        assert_eq!(pair[1] - pair[0], period, "periods differ: {rises:?}");
    }
    let mut highs = Vec::new();
    for &r in &rises[..rises.len() - 1] {
        let f = falls
            .iter()
            .copied()
            .find(|&f| f > r)
            .unwrap_or_else(|| panic!("no falling edge after {r}: {edges:?}"));
        highs.push(f - r);
    }
    assert!(
        highs.iter().all(|&h| h == highs[0]),
        "high times differ: {highs:?}"
    );
    (period, highs[0])
}

/// The openwch Arduino core enables the 1 kHz STK for `millis()`, so a taken
/// SysTick leaves `mcause` at `0x8000_000C` (the same value
/// `ch32v003_stock_sdk_systick_interrupt_runs_through_the_pfic` asserts). That
/// code is a tick, not a fault; anything else is.
fn assert_no_fault(mcause: u32) {
    assert!(
        mcause == 0 || mcause == 0x8000_000C,
        "unexpected trap mcause {mcause:#010x}"
    );
}

/// The stock openwch `analogWrite` path on two different timers. `pwm_start`
/// asks for `PWM_FREQUENCY` 1000 Hz at `PWM_RESOLUTION` 12 bits, so at the
/// 48 MHz HCLK the model must produce a 48 000-cycle period; 2048/4095 of the
/// range is `floor(48000*2048/4095) = 24 005` high cycles on PD4
/// (TIM2_CH1), and 1024/4095 is `12 002` on PD2 (TIM1_CH1).
#[test]
fn ch32v003_openwch_analog_write_pwms_tim2_ch1_and_tim1_ch1() {
    let chip = load_chip("ch32v003f4p6");
    let (out, edges, funcs, mcause) =
        run_pwm_fixture(&chip, &[("gpiod", 4), ("gpiod", 2)], 250_000);
    assert_eq!(
        out, "CH32V003_PWM_BOOT\r\nCH32V003_PWM_SET\r\nCH32V003_PWM_DONE\r\n",
        "USART1 TX stream"
    );
    assert_no_fault(mcause);
    assert_eq!(
        funcs,
        vec![Some("TIM2_CH1".to_string()), Some("TIM1_CH1".to_string())],
        "pinmap_pinout must hand each pad to the timer channel it names"
    );
    assert_eq!(
        period_and_high(&edges[0]),
        (48_000, 24_005),
        "PD4 TIM2_CH1: 1 kHz, 12-bit 2048 = 50.01 %"
    );
    assert_eq!(
        period_and_high(&edges[1]),
        (48_000, 12_002),
        "PD2 TIM1_CH1: 1 kHz, 12-bit 1024 = 25.00 %"
    );
}

/// Negative control: move TIM2's gate to an APB1PCENR bit the firmware never
/// sets. `analogWrite(PD4, …)` then writes an unclocked register file, so the
/// channel never starts and PD4 stays flat, while TIM1 on APB2 keeps its PWM.
/// This proves the pad edges come from a clocked timer, not from the GPIO
/// pinmap write or from the sketch's own pins.
#[test]
fn ch32v003_analog_write_needs_the_tim2_apb1_clock() {
    let mut yaml = chip_yaml("ch32v003f4p6");
    let tim2 = yaml["peripherals"]
        .as_sequence_mut()
        .expect("peripherals list")
        .iter_mut()
        .find(|p| p["id"].as_str() == Some("tim2"))
        .expect("tim2 block");
    assert_eq!(
        tim2["clock"]["reg"].as_str(),
        Some("apb1pcenr"),
        "TIM2 gate register"
    );
    assert_eq!(tim2["clock"]["bit"].as_u64(), Some(0), "TIM2EN bit");
    tim2["clock"]["bit"] = 5.into();
    let chip: ChipDescriptor =
        serde_yaml::from_value(yaml).expect("descriptor with an unclocked TIM2");

    let (out, edges, _, mcause) = run_pwm_fixture(&chip, &[("gpiod", 4), ("gpiod", 2)], 150_000);
    assert!(out.contains("CH32V003_PWM_DONE"), "USART1 stream: {out:?}");
    assert_no_fault(mcause);
    assert!(
        edges[0].is_empty(),
        "an unclocked TIM2 must leave PD4 undriven, got {:?}",
        edges[0]
    );
    assert_eq!(
        period_and_high(&edges[1]),
        (48_000, 12_002),
        "TIM1 on APB2 must keep its 25 % PWM"
    );
}

/// Every package variant carries the same die, so PD4 (TIM2_CH1) must pulse on
/// all four. PD2 is not bonded on the SOP-16/SOP-8, so the sketch's second
/// `analogWrite` has no package pin there; only PD4 is checked.
#[test]
fn ch32v003_package_variants_pwm_tim2_ch1_on_the_bonded_pd4() {
    for (name, _, _, bonded) in PACKAGE_VARIANTS {
        assert!(bonded.contains(&"PD4"), "{name} bonds PD4 (TIM2_CH1)");
        let (out, edges, funcs, mcause) =
            run_pwm_fixture(&load_chip(name), &[("gpiod", 4)], 150_000);
        assert!(out.contains("CH32V003_PWM_DONE"), "{name}: {out:?}");
        assert_no_fault(mcause);
        assert_eq!(
            funcs,
            vec![Some("TIM2_CH1".to_string())],
            "{name}: PD4 must be routed to TIM2_CH1"
        );
        assert_eq!(
            period_and_high(&edges[0]),
            (48_000, 24_005),
            "{name}: PD4 TIM2_CH1"
        );
    }
}

const TONE_FIXTURE: &str = "ch32v003f4p6-arduino-openwch-tone.elf";

/// Step the openwch `tone()` fixture until setup finishes, then record
/// `cycles` of steady-state pad edges. Returns the USART1 text, the PD4 edge
/// list, and the trap cause. The sketch never touches PD4 after `tone()`
/// returns, so every edge in the window is the TIM2 update ISR toggling it.
fn run_tone_fixture(
    chip: &ChipDescriptor,
    cycles: u64,
) -> (String, Vec<labwired_core::logic_capture::LogicEdge>, u32) {
    use labwired_core::logic_capture::LogicSource;

    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut bus = SystemBus::from_config(chip, &SystemManifest::default())
        .expect("CH32V003 peripherals and clock gates");
    let sink = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    bus.attach_uart_tx_sink(sink.clone(), false);
    let image = labwired_loader::load_elf(&root.join("tests/fixtures").join(TONE_FIXTURE))
        .expect("openwch tone ELF");

    let cpu = configure_riscv(&mut bus);
    let mut machine = Machine::new(cpu, bus);
    machine
        .load_firmware(&image)
        .expect("load CH32V003 tone ELF");

    let pd4 = machine
        .bus
        .find_peripheral_index_by_name("gpiod")
        .expect("GPIOD registered from descriptor");
    machine.logic_watch(&[Some(LogicSource::pad(pd4, 4))]);

    let done = b"CH32V003_TONE_DONE";
    let reached_done = |out: &[u8]| out.windows(done.len()).any(|w| w == done);
    let mut elapsed = 0u32;
    while elapsed < 400_000 {
        machine.step().expect("Arduino tone firmware instruction");
        elapsed += 1;
        if reached_done(&sink.lock().unwrap()) {
            break;
        }
    }
    // The turn-on transient is not a period: acknowledge every edge so far,
    // then measure only the steady-state window.
    let cursor = machine.logic_read_edges(0).cursor;
    let target = machine.total_cycles + cycles;
    while machine.total_cycles < target {
        machine.step().expect("tone steady-state instruction");
    }
    let batch = machine.logic_read_edges(cursor);
    let out = String::from_utf8_lossy(&sink.lock().unwrap()).to_string();
    assert!(
        reached_done(out.as_bytes()),
        "sketch stopped before TONE_DONE: {out:?}"
    );
    (out, batch.edges, machine.cpu.mcause)
}

/// The steady toggle spacing of the pad waveform: consecutive edges alternate
/// high/low, so `steps` is the half period the TIM2 update ISR must produce.
/// Every step must be equal; one lucky edge cannot pass.
fn toggle_interval(edges: &[labwired_core::logic_capture::LogicEdge]) -> u64 {
    let mut last: Option<bool> = None;
    let edges: Vec<_> = edges
        .iter()
        .filter(|e| {
            let keep = last != Some(e.value);
            last = Some(e.value);
            keep
        })
        .copied()
        .collect();
    assert!(
        edges.len() >= 6,
        "need at least six edges to measure five toggles, got {edges:?}"
    );
    let step = edges[1].cycle - edges[0].cycle;
    for pair in edges.windows(2) {
        assert_eq!(
            pair[1].cycle - pair[0].cycle,
            step,
            "toggle spacing differs: {edges:?}"
        );
    }
    step
}

/// The stock openwch `tone(PD4, 1000)` path. `Tone.cpp` selects
/// `TIMER_TONE = TIM2`, sets the overflow to `2 * frequency` (2000 Hz), and
/// attaches a callback that toggles PD4 on every update interrupt, so the pad
/// must alternate once per update. `HardwareTimer` computes ARR from the timer
/// clock: 48 MHz / 2000 = 24 000 counts.
///
/// The timer model freezes the counter while an enabled status flag is
/// latched (documented pinned limitation, `peripherals::timer` type docs), so
/// each half period is 24 000 cycles **plus** the ISR latency that clears
/// UIF. The test therefore asserts a constant spacing in a narrow band above
/// 24 000 rather than an exact 24 000, and the exact latency is recorded in
/// the round report.
#[test]
fn ch32v003_openwch_tone_toggles_pd4_from_the_tim2_update_isr() {
    let chip = load_chip("ch32v003f4p6");
    let (out, edges, mcause) = run_tone_fixture(&chip, 200_000);
    assert_eq!(
        out, "CH32V003_TONE_BOOT\r\nCH32V003_TONE_SET\r\nCH32V003_TONE_DONE\r\n",
        "USART1 TX stream"
    );
    // mcause keeps the last trap cause after mret, so a returned timer
    // interrupt (38, TIM2) and the SysTick interrupt (12) are not faults. Any
    // exception (mcause without the interrupt bit) still fails.
    assert!(
        mcause == 0 || mcause == 0x8000_000C || mcause == 0x8000_0026,
        "unexpected exception mcause {mcause:#010x}"
    );
    let step = toggle_interval(&edges);
    assert!(
        (24_000..24_200).contains(&step),
        "TIM2 update toggle spacing {step} cycles, expected 24 000 + ISR latency"
    );
}

/// Negative control: drop TIM2's `irq` line from the descriptor. The timer
/// still counts and still latches UIF, but no PFIC interrupt is delivered, so
/// the Tone callback never runs and PD4 stays flat after `tone()` configures
/// it as a plain output. This proves the edges above come from the interrupt,
/// not from the timer's counter or the pin write.
#[test]
fn ch32v003_tone_needs_the_tim2_irq_line() {
    let mut yaml = chip_yaml("ch32v003f4p6");
    let tim2 = yaml["peripherals"]
        .as_sequence_mut()
        .expect("peripherals list")
        .iter_mut()
        .find(|p| p["id"].as_str() == Some("tim2"))
        .expect("tim2 block");
    assert_eq!(tim2["irq"].as_u64(), Some(38), "TIM2_IRQn");
    tim2.as_mapping_mut().expect("tim2 mapping").remove("irq");
    let chip: ChipDescriptor =
        serde_yaml::from_value(yaml).expect("descriptor without a TIM2 irq line");

    let (out, edges, mcause) = run_tone_fixture(&chip, 100_000);
    assert!(out.contains("CH32V003_TONE_DONE"), "USART1 stream: {out:?}");
    assert_no_fault(mcause);
    assert!(
        edges.is_empty(),
        "a TIM2 without an irq line must leave PD4 undriven, got {edges:?}"
    );
}

const PULSE_FIXTURE: &str = "ch32v003f4p6-arduino-openwch-pulse.elf";

/// High widths, in HCLK cycles at 48 MHz, of the pulses the harness puts on
/// the pad: 100 us and 250 us.
const PULSE_WIDTHS_CYCLES: [u64; 2] = [4_800, 12_000];

/// Run the pulse-width fixture and drive `pad` from the harness only. The
/// sketch never drives an output: it waits for PD3 to rise, reads the STK
/// counter, and waits for PD3 to fall.
///
/// Each pulse is placed on exact cycle boundaries. The pad is held high for
/// `PULSE_WIDTHS_CYCLES[i]`, then released; the sketch prints one `WIDTH`
/// line per pulse and the test compares the reported microseconds with the
/// cycles the harness actually drove.
fn run_pulse_fixture(chip: &ChipDescriptor, pad: u8) -> String {
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut bus = SystemBus::from_config(chip, &SystemManifest::default())
        .expect("CH32V003 peripherals and clock gates");
    let sink = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    bus.attach_uart_tx_sink(sink.clone(), false);
    let image = labwired_loader::load_elf(&root.join("tests/fixtures").join(PULSE_FIXTURE))
        .expect("openwch pulse ELF");
    let cpu = configure_riscv(&mut bus);
    let mut machine = Machine::new(cpu, bus);
    machine
        .load_firmware(&image)
        .expect("load CH32V003 pulse ELF");
    let gpiod = machine
        .bus
        .find_peripheral_index_by_name("gpiod")
        .expect("GPIOD registered from descriptor");

    let text = |sink: &std::sync::Mutex<Vec<u8>>| {
        String::from_utf8_lossy(&sink.lock().unwrap()).into_owned()
    };
    let width_lines = |out: &str| out.lines().filter(|l| l.starts_with("WIDTH ")).count();

    // Setup prints BOOT, then calls pinMode and enters the PD3 wait loop. A
    // 0.4 ms settle after the line arrives is ample for those instructions.
    let mut guard = 0u32;
    while guard < 2_000_000 && !text(&sink).contains("CH32V003_PULSE_BOOT") {
        if machine.step().is_err() {
            break;
        }
        guard += 1;
    }
    let settle = machine.total_cycles + 20_000;
    while machine.total_cycles < settle {
        if machine.step().is_err() {
            break;
        }
    }

    for (index, width) in PULSE_WIDTHS_CYCLES.iter().enumerate() {
        machine.bus.set_peripheral_gpio_input(gpiod, pad, true);
        let fall = machine.total_cycles + width;
        while machine.total_cycles < fall {
            if machine.step().is_err() {
                break;
            }
        }
        machine.bus.set_peripheral_gpio_input(gpiod, pad, false);
        // Let the sketch report this pulse and return to its wait loop before
        // the next one starts; otherwise the rising edge would arrive while
        // the previous line is still in the UART FIFO.
        let mut guard = 0u32;
        while guard < 2_000_000 && width_lines(&text(&sink)) <= index {
            if machine.step().is_err() {
                break;
            }
            guard += 1;
        }
        let settle = machine.total_cycles + 20_000;
        while machine.total_cycles < settle {
            if machine.step().is_err() {
                break;
            }
        }
    }

    // pulseIn() needs a pulse of its own. The sketch prints PULSEIN_WAIT and
    // calls pulseIn() immediately after the line. Measured, the call returns
    // on its own timeout after 28 us, before this settle ends and before the
    // edge is driven, so a missed edge cannot explain the result.
    let mut guard = 0u32;
    while guard < 2_000_000 && !text(&sink).contains("PULSEIN_WAIT") {
        if machine.step().is_err() {
            break;
        }
        guard += 1;
    }
    let settle = machine.total_cycles + 20_000;
    while machine.total_cycles < settle {
        if machine.step().is_err() {
            break;
        }
    }
    machine.bus.set_peripheral_gpio_input(gpiod, pad, true);
    let fall = machine.total_cycles + PULSE_WIDTHS_CYCLES[0];
    while machine.total_cycles < fall {
        if machine.step().is_err() {
            break;
        }
    }
    machine.bus.set_peripheral_gpio_input(gpiod, pad, false);

    let mut guard = 0u32;
    while guard < 2_000_000 && !text(&sink).contains("CH32V003_PULSE_DONE") {
        if machine.step().is_err() {
            break;
        }
        guard += 1;
    }
    text(&sink)
}

fn printed_numbers(out: &str, prefix: &str) -> Vec<u32> {
    let mut values = Vec::new();
    for line in out.lines() {
        if let Some(rest) = line.strip_prefix(prefix) {
            values.extend(
                rest.split_whitespace()
                    .filter_map(|n| n.parse::<u32>().ok()),
            );
        }
    }
    values
}

/// The QingKe V2 STK is an up-counter that reloads from 0 at `CMP`
/// (`SysTick->CMP = 48 000 - 1`, `CTLR = 0xF` from the core's `hw_config.c`),
/// so elapsed microseconds are `CNT * 1000 / (CMP + 1)`. Polling the pad and
/// reading `CNT` must recover the width the harness drove, to the polling
/// granularity.
#[test]
fn ch32v003_pad_pulse_width_reads_microseconds_from_the_stk_counter() {
    let chip = load_chip("ch32v003f4p6");
    let out = run_pulse_fixture(&chip, 3);
    assert!(
        out.contains("CH32V003_PULSE_DONE"),
        "sketch stopped before PULSE_DONE: {out:?}"
    );

    let widths = printed_numbers(&out, "WIDTH ");
    assert_eq!(widths.len(), 2, "one WIDTH line per pulse: {out:?}");
    let expected = [100u32, 250];
    for (measured, want) in widths.iter().zip(expected) {
        assert!(
            measured.abs_diff(want) <= 2,
            "pulse of {want} us measured as {measured} us: {widths:?}"
        );
    }
    let span = widths[1] - widths[0];
    assert!(
        span.abs_diff(150) <= 2,
        "100 us and 250 us pulses differ by {span} us, not 150: {widths:?}"
    );
}

/// Negative control: drive PD2 instead of PD3. The sketch polls PD3 only, so
/// no `WIDTH` line can appear. This shows the printed widths come from the
/// pad the harness drives, not from the sketch's own instruction timing.
#[test]
fn ch32v003_pulse_width_needs_the_pad_the_sketch_polls() {
    let chip = load_chip("ch32v003f4p6");
    let out = run_pulse_fixture(&chip, 2);
    // The sketch printed its banner, then blocked in `while (!pd3())` for the
    // whole run: no width, no micros() samples, no pulseIn() call.
    assert_eq!(
        out, "CH32V003_PULSE_BOOT\r\n",
        "PD2 pulses must not reach a sketch that polls PD3: {out:?}"
    );
}

/// Characterization test for the stock core's `micros()` on this part.
/// `getCurrentMicros()` (ch32/clock.c, the CH32V00x branch) computes
/// `(CMP + 1 - CNT) * 1000 / (CMP + 1)`. That is the remaining time in the
/// millisecond when `CNT` counts up, so three consecutive samples are not
/// increasing. `pulseIn()` (cores/arduino/wiring_pulse.cpp) compares
/// `micros()` differences against its timeout on every pass, so the call
/// returns 0 in tens of microseconds, before the 100 us pulse the harness
/// drives after PULSEIN_WAIT could end. The sketch times the call with the
/// STK counter and prints both the result and the duration.
///
/// The model and the core disagree on the counter direction. This test does
/// not settle which is right against silicon. It records that the model
/// follows WCH's own noneos-sdk `Delay_Us` (per this project's reading of
/// `Debug/ch32v00x/debug.c`, which writes `CNT = 0` and waits for
/// `SR.CNTIF` at `CMP`, a sequence that needs an up-counter), and that the
/// model's direction explains the stock core's output.
///
/// This test asserts the current, defective behaviour. If the vendor core is
/// fixed (or a patched core is added), invert it: `micros()` must increase
/// and `pulseIn()` must report the driven width.
#[test]
fn ch32v003_shipped_micros_runs_backwards_and_pulse_in_cannot_measure() {
    let chip = load_chip("ch32v003f4p6");
    let out = run_pulse_fixture(&chip, 3);
    assert!(
        out.contains("CH32V003_PULSE_DONE"),
        "sketch stopped before PULSE_DONE: {out:?}"
    );

    let samples = printed_numbers(&out, "MICROS ");
    assert_eq!(samples.len(), 3, "three micros() samples: {out:?}");
    assert!(
        samples[1] < samples[0] || samples[2] < samples[1],
        "micros() must not increase across consecutive calls: {samples:?}"
    );

    let pulse_in = printed_numbers(&out, "PULSEIN ");
    assert_eq!(pulse_in.len(), 2, "PULSEIN <width> <call us>: {out:?}");
    assert_eq!(
        pulse_in[0], 0,
        "pulseIn() must return 0 on this core: {out:?}"
    );
    // The harness holds the pad high for 4 800 cycles (100 us) after the
    // sketch prints PULSEIN_WAIT. A call that ended inside that window bailed
    // out on its own timeout before the pulse could finish; a call that
    // waited for the pulse would last at least the pulse width.
    assert!(
        pulse_in[1] < 100,
        "pulseIn() took {} us, which is not shorter than the 100 us pulse: {out:?}",
        pulse_in[1]
    );
}

const I2C_FIXTURE: &str = "ch32v003f4p6-arduino-openwch-i2c.elf";

/// Runs the stock openwch `Wire` fixture against a BME280 placed on `i2c1`,
/// until it prints the complete `CH32V003_I2C_DONE` line or `max_steps` pass.
fn run_i2c_fixture(
    chip: &ChipDescriptor,
    device_address: u8,
    max_steps: u64,
) -> (String, u64, u32) {
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let yaml = format!(
        "name: \"ch32v003-i2c\"\n\
         chip: \"{}\"\n\
         external_devices:\n\
         \x20 - id: \"bme\"\n\
         \x20   type: \"bme280\"\n\
         \x20   connection: \"i2c1\"\n\
         \x20   route:\n\
         \x20     sda: \"PC1\"\n\
         \x20     scl: \"PC2\"\n\
         \x20   config:\n\
         \x20     i2c_address: 0x{device_address:02x}\n",
        root.join("configs/chips/ch32v003f4p6.yaml").display()
    );
    let manifest: SystemManifest = serde_yaml::from_str(&yaml).expect("system manifest");
    let mut bus =
        SystemBus::from_config(chip, &manifest).expect("CH32V003 peripherals plus the I2C slave");
    let sink = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    bus.attach_uart_tx_sink(sink.clone(), false);
    let image = labwired_loader::load_elf(&root.join("tests/fixtures").join(I2C_FIXTURE))
        .expect("openwch Arduino Wire ELF");
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
            if String::from_utf8_lossy(&out).contains("CH32V003_I2C_DONE") && out.ends_with(b"\n") {
                break;
            }
        }
    }
    let out = String::from_utf8_lossy(&sink.lock().unwrap()).into_owned();
    (out, steps, machine.cpu.mcause)
}

#[test]
fn ch32v003_openwch_wire_reads_a_bme280_chip_id_over_i2c1() {
    // Stock openwch arduino_core_ch32 `Wire` on the default SDA=PC1 / SCL=PC2
    // pads. The core enables the I2C1 clock (APB1PCENR bit 21), programs
    // CKCFGR, and runs the EV/ER interrupt path in twi.c. A pointer write of
    // 0xD0 followed by a one-byte read must return the BME280 chip id 0x60.
    let chip = load_chip("ch32v003f4p6");
    let (out, steps, mcause) = run_i2c_fixture(&chip, 0x76, 4_000_000);
    assert_eq!(
        out,
        "CH32V003_I2C_BOOT\r\nerr=0\r\ncount=1\r\nid=0x60\r\n\
         CH32V003_I2C_PASS\r\nCH32V003_I2C_DONE\r\n",
        "USART1 TX stream after {steps} steps"
    );
    assert_no_fault(mcause);
}

#[test]
fn ch32v003_i2c_read_nacks_a_device_at_the_wrong_address() {
    // Negative control: the same firmware addresses 0x76 while the BME280 is
    // wired at 0x77, so the address phase must NACK (err != 0) and no chip id
    // may come back.
    let chip = load_chip("ch32v003f4p6");
    let (out, _, mcause) = run_i2c_fixture(&chip, 0x77, 2_000_000);
    assert!(out.starts_with("CH32V003_I2C_BOOT\r\n"), "{out:?}");
    assert!(
        !out.contains("id=0x60"),
        "wrong-address read returned the id: {out:?}"
    );
    assert!(out.contains("CH32V003_I2C_FAIL"), "{out:?}");
    assert_no_fault(mcause);
}

#[test]
fn ch32v003_package_variants_read_the_bme280_over_i2c1() {
    // F4P6 (TSSOP20), F4U6 (QFN20), A4M6 (SOP16) and J4M6 (SOP8) all bond
    // PC1/PC2 and declare the same I2C1 block. The stock openwch `Wire`
    // fixture finds the BME280 at 0x76 and reads CHIP_ID 0x60 on each.
    for name in [
        "ch32v003f4p6",
        "ch32v003f4u6",
        "ch32v003a4m6",
        "ch32v003j4m6",
    ] {
        let chip = load_chip(name);
        let (out, _, mcause) = run_i2c_fixture(&chip, 0x76, 4_000_000);
        assert!(out.contains("CH32V003_I2C_PASS"), "{name}: {out:?}");
        assert_no_fault(mcause);
    }
}

const SPI_FIXTURE: &str = "ch32v003f4p6-arduino-openwch-spi.elf";

/// Runs the stock openwch `SPI` fixture against `manifest`.
fn run_spi_fixture(
    chip: &ChipDescriptor,
    manifest: &SystemManifest,
    max_steps: u64,
) -> (String, u64, u32) {
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut bus =
        SystemBus::from_config(chip, manifest).expect("CH32V003 peripherals plus the SPI slave");
    let sink = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    bus.attach_uart_tx_sink(sink.clone(), false);
    let image = labwired_loader::load_elf(&root.join("tests/fixtures").join(SPI_FIXTURE))
        .expect("openwch Arduino SPI ELF");
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
            if String::from_utf8_lossy(&out).contains("CH32V003_SPI_DONE") && out.ends_with(b"\n") {
                break;
            }
        }
    }
    let out = String::from_utf8_lossy(&sink.lock().unwrap()).into_owned();
    (out, steps, machine.cpu.mcause)
}

/// The shipped CH32V003EVT + MAX31855 profile.
fn max31855_manifest() -> SystemManifest {
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let path = root.join("configs/systems/wch-ch32v003evt-max31855.yaml");
    let text = std::fs::read_to_string(&path).expect("read the shipped CH32V003 SPI profile");
    serde_yaml::from_str(&text).expect("parse the shipped CH32V003 SPI profile")
}

#[test]
fn ch32v003_openwch_spi_reads_a_max31855_frame_over_spi1() {
    // Stock openwch arduino_core_ch32 `SPI` on the default SCK=PC5 / MOSI=PC6
    // / MISO=PC7 pads. `SPI.begin()` enables RCC_SPI1EN (APB2PCENR bit 12),
    // programs CTLR1 for master mode, and `SPI.transfer()` polls
    // STATR.TXE/RXNE/BSY exactly as `spi_com.c` does. The MAX31855 answers a
    // read-only 32-bit frame with no command byte: 25.00 degC hot junction
    // (100 quarter-degrees) and 22.00 degC cold junction (352 sixteenths),
    // fault-free, i.e. 0x01901600.
    let chip = load_chip("ch32v003f4p6");
    let (out, steps, mcause) = run_spi_fixture(&chip, &max31855_manifest(), 4_000_000);
    assert_eq!(
        out,
        "CH32V003_SPI_BOOT\r\nraw=0x1901600\r\ntc_milli=25000\r\ncj_milli=22000\r\n\
         CH32V003_SPI_PASS\r\nCH32V003_SPI_DONE\r\n",
        "USART1 TX stream after {steps} steps"
    );
    assert_no_fault(mcause);
}

#[test]
fn ch32v003_spi_transfer_without_a_slave_reports_no_frame() {
    // Negative control: the same firmware and pads, no device on the bus. The
    // master still completes its four frames (the model answers 0x00), so the
    // sketch must see an all-zero frame and refuse to call it a reading.
    let chip = load_chip("ch32v003f4p6");
    let empty: SystemManifest =
        serde_yaml::from_str("name: \"ch32v003-spi\"\nchip: \"../chips/ch32v003f4p6.yaml\"\n")
            .expect("system manifest without a device");
    let (out, _, mcause) = run_spi_fixture(&chip, &empty, 2_000_000);
    assert!(out.starts_with("CH32V003_SPI_BOOT\r\n"), "{out:?}");
    assert!(out.contains("raw=0x0\r\n"), "no-slave frame: {out:?}");
    assert!(out.contains("CH32V003_SPI_FAIL"), "{out:?}");
    assert_no_fault(mcause);
}

#[test]
fn ch32v003_spi1_is_on_every_package_but_the_pads_only_where_bonded() {
    // One die, four packages: the SPI1 block is present in every descriptor
    // (the package-consistency test requires it). SPI1 needs SCK, MOSI and
    // MISO: the F4P6 (TSSOP20) and F4U6 (QFN20) bond PC5/PC6/PC7, the A4M6
    // (SOP16) drops PC5, and the J4M6 (SOP8) drops all three, so only the two
    // 20-pin packages may claim SPI pin functions.
    for (name, pins) in [
        (
            "ch32v003f4p6",
            vec![("PC5", "sck"), ("PC6", "mosi"), ("PC7", "miso")],
        ),
        (
            "ch32v003f4u6",
            vec![("PC5", "sck"), ("PC6", "mosi"), ("PC7", "miso")],
        ),
        ("ch32v003a4m6", vec![]),
        ("ch32v003j4m6", vec![]),
    ] {
        let chip = load_chip(name);
        let bus = SystemBus::from_config(&chip, &SystemManifest::default())
            .unwrap_or_else(|e| panic!("{name}: {e}"));
        assert!(
            bus.find_peripheral_index_by_name("spi1").is_some(),
            "{name} keeps the die's SPI1 block"
        );

        let yaml = chip_yaml(name);
        let mut claimed = Vec::new();
        for (pad, spec) in yaml["pins"].as_mapping().expect("pins map") {
            let pad = pad.as_str().expect("pad name");
            let Some(functions) = spec.get("functions") else {
                continue;
            };
            for function in functions.as_sequence().expect("functions list") {
                if function["type"].as_str() == Some("spi") {
                    assert_eq!(function["peripheral"].as_str(), Some("spi1"));
                    claimed.push((
                        pad,
                        function["role"].as_str().expect("SPI role").to_string(),
                    ));
                }
            }
        }
        claimed.sort();
        let mut expected: Vec<(&str, String)> = pins
            .into_iter()
            .map(|(pad, role)| (pad, role.to_string()))
            .collect();
        expected.sort();
        assert_eq!(claimed, expected, "{name} SPI pad claims");
    }
}
