// GPIO edge-observer coverage beyond classic ESP32:
// ESP32-C3 and STM32 GpioPort banks fire the same install_gpio_observer path.

use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::bus::SystemBus;
use labwired_core::Bus;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

#[derive(Debug, Default)]
struct EdgeLog {
    edges: Mutex<Vec<(u8, bool, bool)>>,
}

impl labwired_core::peripherals::gpio_edge::GpioEdgeObserver for EdgeLog {
    fn on_pin_change(&self, pin: u8, from: bool, to: bool, _sim_cycle: u64) {
        self.edges.lock().unwrap().push((pin, from, to));
    }
}

#[test]
fn esp32c3_gpio_fires_observers_on_out_w1ts() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let chip = ChipDescriptor::from_file(root.join("../../configs/chips/esp32c3.yaml"))
        .expect("esp32c3 chip");
    let manifest: SystemManifest = serde_yaml::from_str(
        r#"
name: c3-edge
chip: esp32c3
board_io: []
"#,
    )
    .unwrap();
    let mut bus = SystemBus::from_config(&chip, &manifest).expect("c3 bus");
    let log = Arc::new(EdgeLog::default());
    SystemBus::install_gpio_observer(&mut bus, log.clone());

    const GPIO_BASE: u64 = 0x6000_4000;
    bus.write_u32(GPIO_BASE + 0x08, 1 << 4).expect("W1TS pin4");
    let edges = log.edges.lock().unwrap().clone();
    assert!(
        edges.iter().any(|(p, from, to)| *p == 4 && !*from && *to),
        "C3 OUT_W1TS must notify pin 4 0→1, got {edges:?}"
    );
}

#[test]
fn stm32_gpio_port_fires_observers_with_bank_offset() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let chip = ChipDescriptor::from_file(root.join("../../configs/chips/stm32f103.yaml"))
        .expect("f103 chip");
    let manifest: SystemManifest = serde_yaml::from_str(
        r#"
name: f103-edge
chip: stm32f103
board_io: []
"#,
    )
    .unwrap();
    let mut bus = SystemBus::from_config(&chip, &manifest).expect("f103 bus");
    let log = Arc::new(EdgeLog::default());
    SystemBus::install_gpio_observer(&mut bus, log.clone());

    let gpiob = bus
        .find_peripheral_index_by_name("gpiob")
        .expect("gpiob present");
    let base = bus.peripherals[gpiob].base;
    // BSRR set pin 3 (PB3) → global pin id 16+3 = 19
    bus.write_u32(base + 0x10, 1 << 3).expect("BSRR set PB3");
    let edges = log.edges.lock().unwrap().clone();
    assert!(
        edges.iter().any(|(p, from, to)| *p == 19 && !*from && *to),
        "STM32 gpiob bit 3 must notify global pin 19, got {edges:?}"
    );
}

#[test]
fn stm32_pin_label_parses_to_global_id() {
    assert_eq!(SystemBus::parse_stm32_gpio_global_pin("PA0"), Some(0));
    assert_eq!(SystemBus::parse_stm32_gpio_global_pin("PA15"), Some(15));
    assert_eq!(SystemBus::parse_stm32_gpio_global_pin("PB0"), Some(16));
    assert_eq!(SystemBus::parse_stm32_gpio_global_pin("PC7"), Some(32 + 7));
    assert_eq!(SystemBus::parse_stm32_gpio_global_pin("GPIO15"), None);
}

#[test]
fn ili9341_16bit_paints_via_stm32_gpio_edges() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let chip = ChipDescriptor::from_file(root.join("../../configs/chips/stm32f103.yaml")).unwrap();
    let yaml = r#"
name: f103-par-tft
chip: stm32f103
external_devices:
  - id: tft
    type: ili9341-16bit
    connection: gpio
    config:
      cs_pin: "PA0"
      rs_pin: "PA1"
      wr_pin: "PA2"
      rd_pin: "PA3"
      rst_pin: "PA4"
      db0_pin: "PB0"
      db1_pin: "PB1"
      db2_pin: "PB2"
      db3_pin: "PB3"
      db4_pin: "PB4"
      db5_pin: "PB5"
      db6_pin: "PB6"
      db7_pin: "PB7"
      db8_pin: "PB8"
      db9_pin: "PB9"
      db10_pin: "PB10"
      db11_pin: "PB11"
      db12_pin: "PB12"
      db13_pin: "PB13"
      db14_pin: "PB14"
      db15_pin: "PB15"
board_io: []
"#;
    let manifest: SystemManifest = serde_yaml::from_str(yaml).unwrap();
    let mut bus = SystemBus::from_config(&chip, &manifest).expect("attach kit");
    assert_eq!(bus.ili9341_parallel.len(), 1);
    let pins = bus.ili9341_parallel[0].pins();
    assert_eq!(pins.db[0], 16, "PB0 must encode as global pin 16");
    assert_eq!(pins.db[15], 31);

    // F103 GPIOA is RCC-gated (APB2ENR.IOPAEN bit 2). Writes are dropped until
    // the clock is on — same as silicon (RM0008 §7.3.7). GPIOB is currently
    // ungated in the chip yaml, but enable IOPBEN (bit 3) too for realism.
    let rcc = bus.find_peripheral_index_by_name("rcc").expect("rcc");
    let rcc_base = bus.peripherals[rcc].base;
    const APB2ENR: u64 = 0x18;
    const IOPAEN: u32 = 1 << 2;
    const IOPBEN: u32 = 1 << 3;
    let en = bus.read_u32(rcc_base + APB2ENR).unwrap();
    bus.write_u32(rcc_base + APB2ENR, en | IOPAEN | IOPBEN)
        .expect("enable GPIOA/B clocks");

    let gpioa = bus.find_peripheral_index_by_name("gpioa").unwrap();
    let gpiob = bus.find_peripheral_index_by_name("gpiob").unwrap();
    let a = bus.peripherals[gpioa].base;
    let b = bus.peripherals[gpiob].base;

    // Idle: CS/WR/RD/RST high, RS low → 0b11101 = 0x1D
    bus.write_u32(a + 0x0C, 0x001D).unwrap();
    // Select: CS low → 0x1C
    bus.write_u32(a + 0x0C, 0x001C).unwrap();

    let strobe = |bus: &mut SystemBus, rs_high: bool, db: u16| {
        let mut oa: u32 = (1 << 3) | (1 << 4) | (1 << 2); // RD|RST|WR
        if rs_high {
            oa |= 1 << 1;
        }
        bus.write_u32(a + 0x0C, oa).unwrap();
        bus.write_u32(b + 0x0C, db as u32).unwrap();
        oa &= !(1 << 2);
        bus.write_u32(a + 0x0C, oa).unwrap();
        oa |= 1 << 2;
        bus.write_u32(a + 0x0C, oa).unwrap();
    };

    strobe(&mut bus, false, 0x0029); // DISPON
    assert!(
        bus.ili9341_parallel[0].display_on(),
        "DISPON must latch via STM32 ODR edges"
    );
    strobe(&mut bus, false, 0x002C); // RAMWR
    strobe(&mut bus, true, 0xF800); // red pixel
    let ink = bus.ili9341_parallel[0].ink_bytes();
    assert!(
        ink > 0,
        "STM32 GPIO edges through kit must paint, ink={ink}"
    );
}
