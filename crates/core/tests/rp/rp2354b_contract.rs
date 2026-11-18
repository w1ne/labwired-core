// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! RP2354B contract: the RP2350 die in the QFN80 package with 2 MiB of
//! stacked flash (RP2350 datasheet, Chapter 1, Table 1). The descriptor must
//! be the 48-GPIO package, not the 30-GPIO `rp2350` one under another name.
//!
//! The smoke fixture `tests/fixtures/rp2354b-smoke.elf` is built from
//! `examples/rp2354b/firmware/smoke.c` against unmodified pico-sdk 2.3.1
//! register headers (`examples/rp2354b/firmware/build.sh`, sha256 in
//! `source-and-build.json`).

use crate::common::root;

use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::bus::SystemBus;
use labwired_core::system::cortex_m::configure_cortex_m;
use labwired_core::{Bus, Machine};
use std::sync::{Arc, Mutex};

const SYSINFO_CHIP_ID: u64 = 0x4000_0000;
const SYSINFO_PACKAGE_SEL: u64 = 0x4000_0004;
const IO_BANK0: u64 = 0x4002_8000;
const ADC_CS: u64 = 0x400A_0000;
const SIO: u64 = 0xD000_0000;
const SIO_GPIO_HI_IN: u64 = SIO + 0x08;
const SIO_GPIO_OUT: u64 = SIO + 0x10;
const SIO_GPIO_HI_OUT_SET: u64 = SIO + 0x1c;
const SIO_GPIO_HI_OE_SET: u64 = SIO + 0x3c;
const GPIO_FUNC_SIO: u32 = 5;

fn chip(name: &str) -> ChipDescriptor {
    ChipDescriptor::from_file(root(&format!("configs/chips/{name}.yaml")))
        .unwrap_or_else(|e| panic!("{name} chip: {e}"))
}

fn bus(name: &str) -> SystemBus {
    let chip_path = root(&format!("configs/chips/{name}.yaml"));
    let chip = ChipDescriptor::from_file(&chip_path).expect("chip");
    let mut manifest =
        SystemManifest::from_file(root("configs/systems/rp2354b.yaml")).expect("rp2354b system");
    manifest.chip = chip_path.to_str().unwrap().to_string();
    SystemBus::from_config(&chip, &manifest).expect("bus")
}

fn sio_pad(bus: &SystemBus, pin: u8) -> Option<bool> {
    let idx = bus.find_peripheral_index_by_name("sio").expect("sio");
    bus.peripherals[idx].dev.read_gpio_pad(pin)
}

/// Package geometry: 2 MiB flash, 520 KiB SRAM, GPIO0..GPIO47 on the SIO,
/// and the package-dependent blocks on their QFN80 profiles.
#[test]
fn descriptor_is_the_qfn80_package_not_an_rp2350_alias() {
    let b = chip("rp2354b");
    let a = chip("rp2350");
    assert_eq!(b.flash.base, 0x1000_0000);
    assert_eq!(b.flash.size, 2 * 1024 * 1024);
    assert_eq!(b.ram.size, a.ram.size, "same die, same SRAM");
    assert_eq!(b.pins.len(), 48);
    for n in 0..48u8 {
        let pin = b.pins.get(&format!("GPIO{n}")).expect("GPIOn declared");
        assert_eq!((pin.gpio.as_str(), pin.bit), ("sio", n));
    }
    assert!(a.pins.is_empty(), "rp2350 declares no pad labels");
    let profile = |c: &ChipDescriptor, id: &str| {
        c.peripherals
            .iter()
            .find(|p| p.id == id)
            .and_then(|p| p.config.get("profile"))
            .and_then(|v| v.as_str())
            .map(str::to_string)
    };
    for id in ["io_bank0", "sio", "adc"] {
        assert_eq!(profile(&b, id).as_deref(), Some("rp2350b"), "{id}");
        assert_ne!(profile(&a, id).as_deref(), Some("rp2350b"), "{id}");
    }
}

/// SYSINFO: same CHIP_ID as the rp2350 die, PACKAGE_SEL 0 = QFN80 (pico-sdk
/// rp2350 `sysinfo.h`).
#[test]
fn sysinfo_reports_the_qfn80_package() {
    let b = bus("rp2354b");
    let a = bus("rp2350");
    assert_eq!(b.read_u32(SYSINFO_PACKAGE_SEL).unwrap(), 0);
    assert_eq!(a.read_u32(SYSINFO_PACKAGE_SEL).unwrap(), 1);
    assert_eq!(
        b.read_u32(SYSINFO_CHIP_ID).unwrap(),
        a.read_u32(SYSINFO_CHIP_ID).unwrap()
    );
}

/// GPIO47 through the bus: FUNCSEL SIO in GPIO47_CTRL (0x17C), then the SIO
/// GPIO_HI_OE/OUT words drive the pad and GPIO_HI_IN reads it back.
#[test]
fn gpio47_is_driven_through_the_sio_hi_words() {
    let mut b = bus("rp2354b");
    b.write_u32(IO_BANK0 + 47 * 8 + 4, GPIO_FUNC_SIO).unwrap();
    assert_eq!(b.read_u32(IO_BANK0 + 47 * 8 + 4).unwrap(), GPIO_FUNC_SIO);
    b.write_u32(SIO_GPIO_HI_OE_SET, 1 << 15).unwrap();
    assert_eq!(sio_pad(&b, 47), Some(false));
    b.write_u32(SIO_GPIO_HI_OUT_SET, 1 << 15).unwrap();
    assert_eq!(sio_pad(&b, 47), Some(true));
    assert_eq!(b.read_u32(SIO_GPIO_HI_IN).unwrap() & 0xffff, 1 << 15);
    assert_eq!(sio_pad(&b, 48), None, "there is no GPIO48");

    // The same writes on the 30-GPIO profile reach no pad.
    let mut a = bus("rp2350");
    a.write_u32(SIO_GPIO_HI_OE_SET, 1 << 15).unwrap();
    a.write_u32(SIO_GPIO_HI_OUT_SET, 1 << 15).unwrap();
    assert_eq!(sio_pad(&a, 47), None);
    assert_eq!(a.read_u32(SIO_GPIO_HI_IN).unwrap(), 0);
}

/// Label resolution: GPIO0..31 resolve to a bit of the lo GPIO_OUT word;
/// GPIO32..47 are not bits of that 32-bit word and are refused rather than
/// shifted out of it.
#[test]
fn pad_labels_resolve_inside_the_lo_word_and_refuse_beyond_it() {
    let b = bus("rp2354b");
    assert_eq!(
        SystemBus::resolve_pin_odr_pub(&b, "GPIO31"),
        Some((SIO_GPIO_OUT, 31))
    );
    assert_eq!(SystemBus::resolve_pin_odr_pub(&b, "GPIO47"), None);
    assert_eq!(SystemBus::resolve_pin_idr_pub(&b, "GPIO32"), None);
}

/// ADC: AINSEL is four bits on the QFN80 package (input 8 = temperature
/// sensor), three on the RP2040-shaped converter the rp2350 profile uses.
#[test]
fn adc_selects_the_ninth_input() {
    const CS_EN: u32 = 1;
    const AINSEL_8: u32 = 8 << 12;
    let mut b = bus("rp2354b");
    b.write_u32(ADC_CS, CS_EN | AINSEL_8).unwrap();
    assert_eq!(b.read_u32(ADC_CS).unwrap() & (0xF << 12), AINSEL_8);
    let mut a = bus("rp2350");
    a.write_u32(ADC_CS, CS_EN | AINSEL_8).unwrap();
    assert_eq!(a.read_u32(ADC_CS).unwrap() & (0xF << 12), 0);
}

/// The committed smoke runs fault-free: XOSC, PLL_SYS and CLK_PERI bring-up,
/// both UART0 banners, and sixteen GPIO47 edges from GPIO_HI_OUT_XOR.
#[test]
fn smoke_firmware_prints_and_toggles_gpio47() {
    let path = root("tests/fixtures/rp2354b-smoke.elf");
    assert!(path.is_file(), "fixture missing: {}", path.display());
    let mut b = bus("rp2354b");
    let uart = Arc::new(Mutex::new(Vec::new()));
    b.attach_uart_tx_sink(uart.clone(), false);
    let (cpu, _nvic) = configure_cortex_m(&mut b);
    let mut m = Machine::new(cpu, b);
    let image = labwired_loader::load_elf(&path).expect("load elf");
    m.load_firmware(&image).expect("load firmware");

    let mut edges = 0;
    let mut last = sio_pad(&m.bus, 47);
    for _ in 0..1_000_000 {
        m.step().expect("smoke steps without a fault");
        let now = sio_pad(&m.bus, 47);
        if now != last {
            edges += 1;
            last = now;
        }
        if String::from_utf8_lossy(&uart.lock().unwrap()).contains("GPIO47 OK\n") {
            break;
        }
    }
    let text = String::from_utf8_lossy(&uart.lock().unwrap()).into_owned();
    assert!(text.contains("RP2354B UART OK\n"), "uart: {text:?}");
    assert!(text.contains("GPIO47 OK\n"), "uart: {text:?}");
    assert_eq!(edges, 16, "GPIO47 edges");
    assert_eq!(m.bus.read_u32(0xE000_ED28).unwrap_or(0), 0, "CFSR clean");
}
