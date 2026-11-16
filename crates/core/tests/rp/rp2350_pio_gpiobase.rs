// LabWired - Firmware Simulation Platform
// SPDX-License-Identifier: MIT

//! RP2350 PIO GPIOBASE (datasheet Release 8, section 11.7 register list:
//! GPIOBASE at 0x168, "Relocate GPIO 0 (from PIO's point of view) in the
//! system GPIO numbering, to access more than 32 GPIOs from PIO. Only the
//! values 0 and 16 are supported (only bit 4 is writable)"). pico-sdk
//! `pio_set_gpio_base`; Betaflight's RP2350B DShot sets base 16 for motor
//! pins above GPIO31. PIO pin n then drives GPIO (n + base).
//!
//! Also the PIO TX DREQs (DREQ_PIOx_TXn = 8x + n, pico-sdk dreq.h): a DMA
//! channel feeding TXFn is throttled by the FIFO's free space.

use crate::common::root;

use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::bus::SystemBus;
use labwired_core::system::cortex_m::configure_cortex_m;
use labwired_core::Bus;

const IO_BANK0: u64 = 0x4002_8000;
const PIO2: u64 = 0x5040_0000;
const DMA: u64 = 0x5000_0000;
const GPIO_FUNC_PIO2: u32 = 8;
const SM0_PINCTRL: u64 = 0x0DC;
const SM0_INSTR: u64 = 0x0D8;
const GPIOBASE: u64 = 0x168;
const FLEVEL: u64 = 0x00C;
const FDEBUG: u64 = 0x008;

fn bus(chip: &str, system: &str) -> SystemBus {
    let chip_path = root(chip);
    let c = ChipDescriptor::from_file(&chip_path).expect("chip");
    let mut manifest = SystemManifest::from_file(root(system)).expect("system");
    manifest.chip = chip_path.to_str().unwrap().to_string();
    let mut b = SystemBus::from_config(&c, &manifest).expect("bus");
    let _ = configure_cortex_m(&mut b);
    b
}

fn pad(b: &SystemBus, pin: u8) -> Option<bool> {
    let idx = b.find_peripheral_index_by_name("sio").expect("sio");
    b.peripherals[idx].dev.read_gpio_pad(pin)
}

/// `set pindirs, 1` then `set pins, v` on SM0 with SET_BASE `pio_pin`.
fn drive(b: &mut SystemBus, pio_pin: u32, v: u32) {
    b.write_u32(PIO2 + SM0_PINCTRL, (1 << 26) | (pio_pin << 5))
        .unwrap();
    b.write_u32(PIO2 + SM0_INSTR, 0xE081).unwrap(); // set pindirs, 1
    b.write_u32(PIO2 + SM0_INSTR, 0xE000 | v).unwrap(); // set pins, v
}

#[test]
fn gpiobase_16_puts_pio_pin_24_on_gpio40() {
    let mut b = bus("configs/chips/rp2354b.yaml", "configs/systems/rp2354b.yaml");
    b.write_u32(IO_BANK0 + 40 * 8 + 4, GPIO_FUNC_PIO2).unwrap();
    b.write_u32(PIO2 + GPIOBASE, 16).unwrap();
    assert_eq!(b.read_u32(PIO2 + GPIOBASE).unwrap(), 16);
    drive(&mut b, 24, 1);
    assert_eq!(pad(&b, 40), Some(true), "PIO pin 24 + base 16 = GPIO40");
    drive(&mut b, 24, 0);
    assert_eq!(pad(&b, 40), Some(false));
    // Only bit 4 is writable: 0 and 16 are the only bases.
    b.write_u32(PIO2 + GPIOBASE, 0xFFFF_FFFF).unwrap();
    assert_eq!(b.read_u32(PIO2 + GPIOBASE).unwrap(), 16);
    b.write_u32(PIO2 + GPIOBASE, 0).unwrap();
    assert_eq!(b.read_u32(PIO2 + GPIOBASE).unwrap(), 0);
}

#[test]
fn dma_into_pio_tx_fifo_is_paced_by_dreq_pio2_tx0() {
    let mut b = bus(
        "configs/chips/rp2350.yaml",
        "configs/systems/rp2350-zero.yaml",
    );
    for i in 0..8u64 {
        b.write_u32(0x2001_0000 + 4 * i, 0x1000 + i as u32).unwrap();
    }
    // Word DMA RAM -> PIO2 TXF0 on DREQ 16; the SM is not running, so the
    // 4-deep FIFO fills and the channel must wait with 4 words left.
    b.write_u32(DMA, 0x2001_0000).unwrap();
    b.write_u32(DMA + 0x04, (PIO2 + 0x010) as u32).unwrap();
    b.write_u32(DMA + 0x08, 8).unwrap();
    b.write_u32(DMA + 0x0C, 1 | (2 << 2) | (1 << 4) | (16 << 17))
        .unwrap();
    for _ in 0..200 {
        let _ = b.tick_peripherals_fully();
    }
    assert_eq!(b.read_u32(PIO2 + FLEVEL).unwrap() & 0xF, 4, "TX0 level");
    assert_eq!(
        b.read_u32(PIO2 + FDEBUG).unwrap() & (1 << 16),
        0,
        "no TXOVER"
    );
    assert_ne!(b.read_u32(DMA + 0x0C).unwrap() & (1 << 26), 0, "still busy");
    assert_eq!(
        b.read_u32(DMA + 0x08).unwrap() & 0x0FFF_FFFF,
        4,
        "4 words left"
    );
}
