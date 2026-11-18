// LabWired - Firmware Simulation Platform
// SPDX-License-Identifier: MIT
//! RP2350B register / physical-pad contracts from the Raspberry Pi datasheet
//! and pico-sdk ee68c78d hardware_regs. A and RP2040 retain their 30 pads.

use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::bus::{DevicePins, SystemBus};
use labwired_core::peripherals::rp2040::sio::{Rp2040Sio, SioLayout};
use labwired_core::{Bus, Peripheral};
use std::path::PathBuf;

const SIO: u64 = 0xd000_0000;
const IO: u64 = 0x4002_8000;

fn descriptor(name: &str) -> (ChipDescriptor, SystemManifest) {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../configs/chips")
        .join(format!("{name}.yaml"));
    let chip = ChipDescriptor::from_file(&path).unwrap();
    let manifest =
        serde_yaml::from_str(&format!("name: rp-gpio-test\nchip: {}\n", path.display())).unwrap();
    (chip, manifest)
}
fn bus() -> SystemBus {
    // A built-in chip must also resolve every declarative block without a
    // source-tree filesystem, as required by CLI bare names and wasm.
    let chip = ChipDescriptor::resolve("rp2354b", std::path::Path::new(".")).unwrap();
    let manifest = serde_yaml::from_str("name: rp2354b-test\nchip: rp2354b\n").unwrap();
    SystemBus::from_config(&chip, &manifest).unwrap()
}
fn ctrl(pin: u8) -> u64 {
    IO + u64::from(pin) * 8 + 4
}

#[test]
fn package_select_flash_and_pad_count_match_the_named_variant() {
    let b = bus();
    assert_eq!(b.flash.data.len(), 2 * 1024 * 1024);
    assert_eq!(b.ram.data.len(), 520 * 1024);
    let adc = &b.peripherals[b.find_peripheral_index_by_name("adc").unwrap()].dev;
    assert_eq!(adc.adc_channel_count(), Some(9));
    assert_eq!(b.read_u32(0x4000_0004).unwrap(), 0, "QFN80");
    let sio = &b.peripherals[b.find_peripheral_index_by_name("sio").unwrap()].dev;
    assert_eq!(sio.pins().unwrap().pin_count(), 48);
    assert_eq!(sio.read_gpio_input(47), Some(false));
    assert_eq!(sio.read_gpio_input(48), None);
    let (chip, manifest) = descriptor("rp2350");
    let a = SystemBus::from_config(&chip, &manifest).unwrap();
    assert_eq!(
        a.read_u32(0x4000_0004).unwrap(),
        1,
        "QFN60 remains unchanged"
    );
    let sio = &a.peripherals[a.find_peripheral_index_by_name("sio").unwrap()].dev;
    assert_eq!(sio.pins().unwrap().pin_count(), 30);
    assert_eq!(sio.read_gpio_input(30), None);
}

#[test]
fn both_sio_words_and_their_set_clear_xor_aliases_keep_other_bank_state() {
    let mut b = bus();
    for pin in [30, 31, 32, 40, 47] {
        b.write_u32(ctrl(pin), 5).unwrap();
    }
    b.write_u32(SIO + 0x30, 0xc000_0000).unwrap(); // GPIO_OE
    b.write_u32(SIO + 0x34, 0x8101).unwrap(); // GPIO_HI_OE
    b.write_u32(SIO + 0x10, 0xc000_0000).unwrap();
    b.write_u32(SIO + 0x14, 0x8101).unwrap();
    assert_eq!(b.read_u32(SIO + 4).unwrap(), 0xc000_0000);
    assert_eq!(b.read_u32(SIO + 8).unwrap(), 0x8101);
    b.write_u32(SIO + 0x24, 0x8000).unwrap(); // HI_OUT_CLR
    b.write_u32(SIO + 0x2c, 0x0100).unwrap(); // HI_OUT_XOR
    b.write_u32(SIO + 0x1c, 0x8000).unwrap(); // HI_OUT_SET
    assert_eq!(b.read_u32(SIO + 0x14).unwrap(), 0x8001);
    assert_eq!(b.read_u32(SIO + 0x10).unwrap(), 0xc000_0000);
    b.write_u32(SIO + 0x44, 0x8000).unwrap(); // HI_OE_CLR
    assert_eq!(b.read_u32(SIO + 8).unwrap(), 1);
    b.write_u32(SIO + 0x4c, 0x0100).unwrap(); // HI_OE_XOR
    b.write_u32(SIO + 0x3c, 0x8000).unwrap(); // HI_OE_SET
    assert_eq!(b.read_u32(SIO + 8).unwrap(), 0x8001);
    b.write_u32(SIO + 0x14, u32::MAX).unwrap();
    assert_eq!(
        b.read_u32(SIO + 0x14).unwrap(),
        0xffff,
        "QSPI/USB pads stay outside this bank-0 profile"
    );
    b.write_u8(SIO + 0x15, 0x5a).unwrap();
    assert_eq!(b.read_u32(SIO + 0x14).unwrap(), 0x5aff);
    assert_eq!(b.read_u32(SIO + 0x10).unwrap(), 0xc000_0000);
}

#[test]
fn high_bank_input_uses_physical_pad_not_same_bit_in_low_bank() {
    let mut b = bus();
    assert_eq!(
        SystemBus::resolve_pin_idr_pub(&b, "GP47"),
        Some((SIO + 8, 15))
    );
    assert_eq!(
        SystemBus::resolve_pin_odr_pub(&b, "GPIO47"),
        Some((SIO + 0x14, 15))
    );
    assert!(SystemBus::drive_pin_input(&mut b, "GP47", true));
    assert_eq!(b.read_u32(SIO + 8).unwrap(), 0x8000);
    assert_eq!(b.read_u32(SIO + 4).unwrap(), 0, "GPIO15 must not move");
    assert_eq!(DevicePins::pad_bit(&b, SIO + 0x14, 15), Some(true));
    assert!(DevicePins::released_output_bit(&b, SIO + 0x14, 15).unwrap());
    b.write_u32(SIO + 0x34, 0x8000).unwrap(); // output overrides external high
    assert_eq!(b.read_u32(SIO + 8).unwrap(), 0);
    b.write_u32(SIO + 0x1c, 0x8000).unwrap();
    assert_eq!(DevicePins::known_pad_bit(&b, SIO + 0x14, 15), Some(true));
    assert!(!SystemBus::drive_pin_input(&mut b, "GP48", true));
    assert!(!b.drive_input_bit(SIO + 8, 16, true));
}

#[test]
fn all_six_interrupt_words_and_high_summary_preserve_latched_edge_semantics() {
    let mut b = bus();
    for pin in [30u8, 31, 32, 40, 47] {
        let word = u64::from(pin / 8);
        let shift = 4 * u32::from(pin % 8);
        let old = b.read_u32(IO + 0x248 + word * 4).unwrap();
        b.write_u32(IO + 0x248 + word * 4, old | (8 << shift))
            .unwrap();
        assert!(SystemBus::drive_pin_input(
            &mut b,
            &format!("GP{pin}"),
            true
        ));
        assert_eq!(
            b.read_u32(IO + 0x230 + word * 4).unwrap() >> shift & 0xf,
            0xa
        );
    }
    assert_eq!(b.read_u32(IO + 0x200).unwrap(), 0xc000_0000);
    assert_eq!(b.read_u32(IO + 0x204).unwrap(), 0x8101);
    let bank = &b.peripherals[b.find_peripheral_index_by_name("io_bank0").unwrap()].dev;
    assert_eq!(bank.irq_line_level(), Some(true));
    b.write_u32(IO + 0x244, 8 << 28).unwrap(); // INTR5 GPIO47 edge W1C
    assert_eq!(
        b.read_u32(IO + 0x244).unwrap() >> 28,
        2,
        "live high remains"
    );
    assert_eq!(b.read_u32(IO + 0x204).unwrap(), 0x101);
    assert!(SystemBus::drive_pin_input(&mut b, "GP47", false));
    assert_eq!(
        b.read_u32(IO + 0x244).unwrap() >> 28,
        5,
        "falling edge + live low"
    );
}

#[test]
fn rp2040_and_a_package_reject_b_only_gpio_and_interrupt_bits() {
    for layout in [SioLayout::Rp2040, SioLayout::Rp2350] {
        let mut sio = Rp2040Sio::new().with_layout(layout);
        assert!(!sio.set_gpio_input(30, true));
        assert_eq!(sio.gpio_pin_registers(32), None);
    }
    let (chip, manifest) = descriptor("rp2350");
    let mut a = SystemBus::from_config(&chip, &manifest).unwrap();
    a.write_u32(SIO + 0x10, u32::MAX).unwrap();
    a.write_u32(SIO + 0x14, u32::MAX).unwrap();
    assert_eq!(a.read_u32(SIO + 0x10).unwrap(), 0x3fff_ffff);
    assert_eq!(a.read_u32(SIO + 0x14).unwrap(), 0);
    a.write_u32(IO + 0x270, u32::MAX).unwrap(); // PROC0_INTF4
    assert_eq!(a.read_u32(IO + 0x288).unwrap(), 0); // PROC0_INTS4
    assert_eq!(a.read_u32(IO + 0x204).unwrap(), 0);
    a.write_u32(ctrl(47), 5).unwrap();
    assert_eq!(a.read_u32(ctrl(47)).unwrap(), 0);
}

#[test]
fn b_pads_reach_existing_pio_spi_pwm_and_i2c_routes() {
    let mut b = bus();
    let checks = [(31, 6, "PIO_GP31"), (43, 1, "SPI1_TX"), (40, 3, "I2C0_SDA")];
    for (pin, func, expected) in checks {
        b.write_u32(ctrl(pin), func).unwrap();
        let sio = &b.peripherals[b.find_peripheral_index_by_name("sio").unwrap()].dev;
        let routing = sio.gpio_routing(pin).unwrap();
        assert_eq!(routing.func.as_deref(), Some(expected));
    }
}

#[test]
fn malformed_or_unknown_package_profile_cannot_silently_select_rp2040() {
    for id in ["sio", "io_bank0", "adc"] {
        for value in [
            serde_yaml::Value::String("rp2354banana".into()),
            serde_yaml::Value::from(48),
        ] {
            let (mut chip, manifest) = descriptor("rp2354b");
            chip.peripherals
                .iter_mut()
                .find(|p| p.id == id)
                .unwrap()
                .config
                .insert("profile".into(), value);
            let error = SystemBus::from_config(&chip, &manifest)
                .err()
                .expect("must reject malformed profile");
            assert!(format!("{error:#}").contains("profile must be"));
        }
    }
}
