// LabWired - Firmware Simulation Platform
// SPDX-License-Identifier: MIT

//! The RP2350 ADC raises `ADC_IRQ_FIFO` on NVIC line 35 — RP2350 datasheet
//! (Release 8) section 3.2, Table "Interrupts": ADC_IRQ_FIFO = 35; pico-sdk
//! rp2350 `intctrl.h` `ADC_IRQ_FIFO 35`. Line 22 is the RP2040 number (RP2040
//! datasheet 2.3.2); on the RP2350, NVIC 22 is IO_IRQ_BANK0_NS.
//!
//! The ADC is the RP2040 model reused; its interrupt line must come from the
//! chip descriptor (`irq: 35` in configs/chips/rp2350.yaml), not from the
//! RP2040 constant baked into the model.

use crate::common::root;

use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::bus::SystemBus;
use labwired_core::system::cortex_m::configure_cortex_m;
use labwired_core::Bus;

const RP2350_ADC: u64 = 0x400A_0000;
const RP2040_ADC: u64 = 0x4004_C000;
const INTE: u64 = 0x18;
const INTF: u64 = 0x1C;
const NVIC_ISER0: u64 = 0xE000_E100;
const NVIC_ISPR0: u64 = 0xE000_E200;

fn pending(bus: &SystemBus, line: u32) -> bool {
    let word = bus.read_u32(NVIC_ISPR0 + u64::from(line / 32) * 4).unwrap();
    word & (1 << (line % 32)) != 0
}

fn bus_for(chip_rel: &str, system_rel: &str) -> SystemBus {
    let chip_path = root(chip_rel);
    let chip = ChipDescriptor::from_file(&chip_path).expect("chip");
    let mut manifest = SystemManifest::from_file(root(system_rel)).expect("system");
    manifest.chip = chip_path.to_str().unwrap().to_string();
    let mut bus = SystemBus::from_config(&chip, &manifest).expect("bus");
    let _ = configure_cortex_m(&mut bus);
    bus
}

fn fire_adc_fifo_irq(bus: &mut SystemBus, adc: u64, line: u32) {
    // Enable the NVIC line, then force the FIFO interrupt (INTE.FIFO + INTF.FIFO).
    bus.write_u32(NVIC_ISER0 + u64::from(line / 32) * 4, 1 << (line % 32))
        .unwrap();
    bus.write_u32(adc + INTE, 1).unwrap();
    bus.write_u32(adc + INTF, 1).unwrap();
    for _ in 0..4 {
        let _ = bus.tick_peripherals_fully();
    }
}

#[test]
fn rp2350_adc_fifo_interrupt_is_nvic_35() {
    let mut bus = bus_for(
        "configs/chips/rp2350.yaml",
        "configs/systems/rp2350-zero.yaml",
    );
    fire_adc_fifo_irq(&mut bus, RP2350_ADC, 35);
    assert!(
        pending(&bus, 35),
        "ADC_IRQ_FIFO must pend NVIC 35 on the RP2350"
    );
    assert!(
        !pending(&bus, 22),
        "NVIC 22 is IO_IRQ_BANK0_NS on the RP2350, not the ADC"
    );
}

#[test]
fn rp2040_adc_fifo_interrupt_stays_nvic_22() {
    let mut bus = bus_for(
        "configs/chips/rp2040.yaml",
        "configs/systems/rp2040-pico.yaml",
    );
    fire_adc_fifo_irq(&mut bus, RP2040_ADC, 22);
    assert!(pending(&bus, 22), "ADC_IRQ_FIFO is NVIC 22 on the RP2040");
}
