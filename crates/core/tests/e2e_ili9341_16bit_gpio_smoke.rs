// Proof: Ili9341Parallel paints when driven through REAL classic-ESP32 GPIO
// registers (OUT / OUT1 W1TS/W1TC), with the panel attached via the same
// `SystemBus::from_config` path diagrams and system.yaml use.
//
// This is the Phase-2 smoke lab for LCDWiki MRB3205-class 16-bit parallel
// (`ili9341-16bit`). It is the register-level twin of firmware bit-bang:
// CS/RS/WR/RST + DB[15:0] edges reach `Ili9341Parallel` through
// `Esp32Gpio` observers — no direct `on_gpio_edge` injection on the panel.
//
// Lab fixture: `examples/ili9341-16bit-lab/system.yaml`.
//
// Run:
//   cargo test -p labwired-core --test e2e_ili9341_16bit_gpio_smoke -- --nocapture

use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::bus::SystemBus;
use labwired_core::Bus;
use std::path::PathBuf;

// Classic ESP32 GPIO (TRM §4.10) at 0x3FF4_4000.
const GPIO_BASE: u64 = 0x3FF4_4000;
const GPIO_OUT_W1TS: u64 = 0x08;
const GPIO_OUT_W1TC: u64 = 0x0C;
const GPIO_OUT1_W1TS: u64 = 0x14;
const GPIO_OUT1_W1TC: u64 = 0x18;
const GPIO_ENABLE_W1TS: u64 = 0x24;
const GPIO_ENABLE1_W1TS: u64 = 0x30;

// Pin map — keep in lockstep with examples/ili9341-16bit-lab/system.yaml.
const CS: u8 = 15;
const RS: u8 = 2;
const WR: u8 = 4;
const RD: u8 = 5;
const RST: u8 = 33;
const DB: [u8; 16] = [12, 13, 14, 16, 17, 18, 19, 21, 22, 23, 25, 26, 27, 32, 0, 3];

fn lab_paths() -> (PathBuf, PathBuf) {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let chip = root.join("../../configs/chips/esp32.yaml");
    let system = root.join("../../examples/ili9341-16bit-lab/system.yaml");
    (chip, system)
}

fn bank0_mask(pins: &[u8]) -> u32 {
    let mut m = 0u32;
    for &p in pins {
        if p < 32 {
            m |= 1u32 << p;
        }
    }
    m
}

fn bank1_mask(pins: &[u8]) -> u32 {
    let mut m = 0u32;
    for &p in pins {
        if (32..40).contains(&p) {
            m |= 1u32 << (p - 32);
        }
    }
    m
}

fn set_pin(bus: &mut SystemBus, pin: u8, high: bool) {
    if pin < 32 {
        let mask = 1u32 << pin;
        let off = if high { GPIO_OUT_W1TS } else { GPIO_OUT_W1TC };
        bus.write_u32(GPIO_BASE + off, mask).expect("gpio out bank0");
    } else {
        let mask = 1u32 << (pin - 32);
        let off = if high { GPIO_OUT1_W1TS } else { GPIO_OUT1_W1TC };
        bus.write_u32(GPIO_BASE + off, mask).expect("gpio out bank1");
    }
}

/// Present `value` on DB[15:0] (DB0 = LSB).
fn set_db(bus: &mut SystemBus, value: u16) {
    for i in 0..16 {
        let high = (value >> i) & 1 != 0;
        set_pin(bus, DB[i], high);
    }
}

/// One 8080 write cycle: sample bus on WR falling edge while CS is low.
fn bus_write(bus: &mut SystemBus, rs_high: bool, value: u16) {
    set_pin(bus, RS, rs_high);
    set_db(bus, value);
    set_pin(bus, WR, true);
    set_pin(bus, WR, false); // falling edge → panel samples
    set_pin(bus, WR, true);
}

fn cmd(bus: &mut SystemBus, c: u8) {
    bus_write(bus, false, c as u16);
}

fn data8(bus: &mut SystemBus, d: u8) {
    bus_write(bus, true, d as u16);
}

fn data16(bus: &mut SystemBus, d: u16) {
    bus_write(bus, true, d);
}

fn idle_high(bus: &mut SystemBus) {
    // Enable every pin we drive so pad readback matches drive (optional for
    // observers — edges fire on OUT changes either way).
    let all: Vec<u8> = [CS, RS, WR, RD, RST]
        .into_iter()
        .chain(DB.iter().copied())
        .collect();
    let b0 = bank0_mask(&all);
    let b1 = bank1_mask(&all);
    if b0 != 0 {
        bus.write_u32(GPIO_BASE + GPIO_ENABLE_W1TS, b0)
            .expect("enable bank0");
    }
    if b1 != 0 {
        bus.write_u32(GPIO_BASE + GPIO_ENABLE1_W1TS, b1)
            .expect("enable bank1");
    }
    // Idle: CS/WR/RD/RST high, RS low, data zero.
    set_pin(bus, CS, true);
    set_pin(bus, RS, false);
    set_pin(bus, WR, true);
    set_pin(bus, RD, true);
    set_pin(bus, RST, true);
    set_db(bus, 0);
}

fn hw_reset(bus: &mut SystemBus) {
    set_pin(bus, RST, false);
    set_pin(bus, RST, true);
}

/// Minimal init + solid red band — same command set the SPI lab uses, over 8080.
fn paint_red_band(bus: &mut SystemBus) {
    idle_high(bus);
    hw_reset(bus);

    set_pin(bus, CS, false); // select

    cmd(bus, 0x01); // SWRESET
    cmd(bus, 0x28); // DISPOFF
    cmd(bus, 0x3A); // COLMOD
    data8(bus, 0x55); // 16-bit
    cmd(bus, 0x36); // MADCTL
    data8(bus, 0x00);
    cmd(bus, 0x29); // DISPON

    // Window: full width, rows 0..15 (240×16 pixels)
    cmd(bus, 0x2A); // CASET
    data8(bus, 0x00);
    data8(bus, 0x00);
    data8(bus, 0x00);
    data8(bus, 0xEF); // 239
    cmd(bus, 0x2B); // PASET
    data8(bus, 0x00);
    data8(bus, 0x00);
    data8(bus, 0x00);
    data8(bus, 0x0F); // 15
    cmd(bus, 0x2C); // RAMWR
    // RGB565 red = 0xF800 (big-endian on wire: one 16-bit bus write per pixel)
    const RED: u16 = 0xF800;
    for _ in 0..(240 * 16) {
        data16(bus, RED);
    }

    set_pin(bus, CS, true);
}

#[test]
fn ili9341_16bit_lab_system_yaml_attaches_and_paints_over_real_gpio() {
    let (chip_path, system_path) = lab_paths();
    assert!(
        system_path.is_file(),
        "lab system.yaml missing at {}",
        system_path.display()
    );

    let chip = ChipDescriptor::from_file(&chip_path).expect("esp32 chip yaml");
    let yaml = std::fs::read_to_string(&system_path).expect("read system.yaml");
    let manifest: SystemManifest = serde_yaml::from_str(&yaml).expect("parse system.yaml");

    let mut bus = SystemBus::from_config(&chip, &manifest)
        .expect("from_config must attach ili9341-16bit on classic ESP32");
    assert_eq!(bus.ili9341_parallel.len(), 1, "exactly one parallel panel");
    assert_eq!(bus.ili9341_parallel[0].id(), "tft");
    assert_eq!(bus.ili9341_parallel[0].ink_bytes(), 0);

    paint_red_band(&mut bus);

    let panel = &bus.ili9341_parallel[0];
    // RGB565 red 0xF800: only the high byte is non-zero, so ink_bytes (non-zero
    // count) is one per pixel for a solid-red band.
    let ink = panel.ink_bytes();
    assert!(
        ink >= 240 * 16,
        "red band must ink the 240×16 window (got ink_bytes={ink})"
    );
    assert!(panel.display_on(), "DISPON must leave the panel on");

    // Spot-check first pixel is RGB565 red (BE).
    let fb = panel.framebuffer();
    assert_eq!(fb[0], 0xF8, "pixel0 hi");
    assert_eq!(fb[1], 0x00, "pixel0 lo");
    // Last pixel of the 240×16 band
    let last = (240 * 16 - 1) * 2;
    assert_eq!(fb[last], 0xF8, "last band pixel hi");
    assert_eq!(fb[last + 1], 0x00, "last band pixel lo");

    // Inspect seam: same artifact path the playground / oracle use.
    let devices = bus.inspect_devices(None, &labwired_core::inspect::InspectOpts::default());
    let tft = devices
        .iter()
        .find(|d| d.id == "tft")
        .expect("inspect must list tft");
    assert_eq!(tft.device_type.as_deref(), Some("ili9341-16bit"));
    let art = tft
        .artifacts
        .iter()
        .find(|a| a.kind == "framebuffer")
        .expect("framebuffer artifact");
    let painted = art.meta["painted_bytes"]
        .as_u64()
        .or_else(|| art.meta["ink_bytes"].as_u64())
        .unwrap_or(0);
    assert!(
        painted > 0,
        "inspect meta must report painted/ink bytes, got meta={:?}",
        art.meta
    );
}

#[test]
fn lab_pin_map_matches_attached_panel() {
    let (chip_path, system_path) = lab_paths();
    let chip = ChipDescriptor::from_file(&chip_path).unwrap();
    let yaml = std::fs::read_to_string(&system_path).unwrap();
    let manifest: SystemManifest = serde_yaml::from_str(&yaml).unwrap();
    let bus = SystemBus::from_config(&chip, &manifest).unwrap();
    let pins = bus.ili9341_parallel[0].pins();
    assert_eq!(pins.cs, CS);
    assert_eq!(pins.rs, RS);
    assert_eq!(pins.wr, WR);
    assert_eq!(pins.rd, RD);
    assert_eq!(pins.rst, RST);
    assert_eq!(pins.db, DB);
}
