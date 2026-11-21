// SPDX-License-Identifier: MIT
//! Nordic GPIO DIR/PIN_CNF aliases. Source: nRF52840 Product Specification,
//! GPIO DIRSET, DIRCLR and PIN_CNF[n] register descriptions:
//! https://docs.nordicsemi.com/r/bundle/ps_nrf52840/page/gpio.html
use super::Nrf52Gpio;

fn check(g: &Nrf52Gpio, direction: u32, config: &[u32; 32]) {
    for offset in [0x514, 0x518, 0x51c] {
        assert_eq!(g.read_reg(offset), direction, "register {offset:#x}");
    }
    for (pin, &initial_config) in config.iter().enumerate() {
        let expected = if pin < g.num_pins as usize {
            (initial_config & !1) | ((direction >> pin) & 1)
        } else {
            0
        };
        assert_eq!(g.read_reg(0x700 + 4 * pin as u64), expected, "pin {pin}");
        // The serialized fields must agree as well as the register reads.
        assert_eq!(g.pin_cnf[pin], expected);
    }
}

#[test]
fn all_direction_aliases_preserve_every_other_pin_configuration_field() {
    for count in [0, 10, 13, 16, 32] {
        let mut g = Nrf52Gpio::with_num_pins(count);
        let mut config = [0; 32];
        for (pin, pin_config) in config.iter_mut().enumerate().take(count as usize) {
            // Different INPUT/PULL/DRIVE/SENSE fields per pin.
            *pin_config = ((pin as u32 & 1) << 1)
                | (((pin as u32 >> 1) & 3) << 2)
                | ((pin as u32 & 7) << 8)
                | ((pin as u32 & 3) << 16);
            g.write_reg(0x700 + 4 * pin as u64, *pin_config);
        }
        let mask = g.pin_mask();
        for value in [0, u32::MAX, 0xaaaaaaaa, 0x55555555, 0x80000001, 0] {
            g.write_reg(0x514, value);
            let mut expected = value & mask;
            check(&g, expected, &config);
            for bits in [0, 0x80000001, 0x55aa55aa, u32::MAX, 0] {
                g.write_reg(0x518, bits);
                expected |= bits & mask;
                check(&g, expected, &config);
                g.write_reg(0x51c, bits);
                expected &= !bits;
                check(&g, expected, &config);
            }
        }
    }
}

#[test]
fn pin_configuration_writes_update_all_direction_read_aliases() {
    let mut g = Nrf52Gpio::default();
    let mut config = [0; 32];
    let mut expected = 0;
    for pin in [31, 0, 29, 1, 5, 20] {
        config[pin] = 0x2030c;
        g.write_reg(0x700 + 4 * pin as u64, config[pin] | 1);
        expected |= 1 << pin;
        check(&g, expected, &config);
    }
    for pin in [1, 31, 5, 0, 20, 29] {
        g.write_reg(0x700 + 4 * pin as u64, config[pin]);
        expected &= !(1 << pin);
        check(&g, expected, &config);
    }
}

#[test]
fn read_modify_write_pin_configuration_does_not_undo_bulk_direction() {
    let mut g = Nrf52Gpio::default();
    let cnf = 0x700 + 4 * 20;
    g.write_reg(0x518, 1 << 20);
    let original = g.read_reg(cnf);
    g.write_reg(cnf, original | (3 << 2)); // Enable pull-up, retaining DIR.
    assert_eq!(g.read_reg(0x514), 1 << 20);
    g.write_reg(0x51c, 1 << 20);
    let original = g.read_reg(cnf);
    g.write_reg(cnf, original | (1 << 8)); // Change DRIVE, retaining DIR.
    assert_eq!(g.read_reg(0x514), 0);
}

#[test]
fn direction_aliases_preserve_output_and_external_input_levels() {
    let mut g = Nrf52Gpio::default();
    let selected = 1 << 20;
    let unrelated = 1 << 5;
    g.write_reg(0x700 + 4 * 20, 3 << 2); // Input with pull-up.
    assert_eq!(g.effective_in() & selected, selected);
    g.write_reg(0x504, unrelated); // Selected output latch stays low.
    g.write_reg(0x510, unrelated);
    g.external = unrelated;
    for offset in [0x514, 0x518] {
        g.write_reg(offset, selected);
        assert_eq!(g.effective_in() & selected, 0);
        assert_eq!(g.effective_in() & unrelated, unrelated);
        assert_eq!(g.read_reg(0x504), unrelated);
        g.write_reg(0x51c, selected);
        assert_eq!(g.effective_in() & selected, selected); // Pull restored.
        assert_eq!(g.effective_in() & unrelated, unrelated);
    }
    g.external |= selected;
    g.write_reg(0x510, unrelated); // External low beats the pull-up.
    assert_eq!(g.effective_in() & selected, 0);
    g.write_reg(0x508, selected);
    g.write_reg(0x518, selected);
    assert_eq!(g.effective_in() & selected, selected); // Output latch live.
    g.write_reg(0x514, 0);
    assert_eq!(g.effective_in() & selected, 0);
}

#[test]
fn output_set_and_clear_read_the_latch_without_side_effects() {
    for count in [0, 10, 16, 32] {
        let mut g = Nrf52Gpio::with_num_pins(count);
        let mask = g.pin_mask();
        for value in [0, u32::MAX, 0x80000001, 0x55555555] {
            g.write_reg(0x504, value);
            let mut expected = value & mask;
            for (offset, bits) in [
                (0x508, 0xaaaaaaaa),
                (0x50c, 0x00000003),
                (0x508, 0),
                (0x50c, 0),
            ] {
                g.write_reg(offset, bits);
                if offset == 0x508 {
                    expected |= bits & mask;
                } else {
                    expected &= !bits;
                }
                // IN and DIR differ deliberately: aliases read OUT only.
                g.write_reg(0x510, !expected);
                for read in [0x504, 0x508, 0x50c] {
                    assert_eq!(g.read_reg(read), expected);
                    assert_eq!(g.read_reg(read), expected);
                }
                assert_eq!(g.read_reg(0x514), 0);
            }
        }
    }
}

mod port_contract {
    use crate::peripherals::gpio::{GpioPort, GpioRegisterLayout};
    use crate::Peripheral;

    #[test]
    fn public_nrf52_and_compact_nrf54_aliases_keep_register_and_pad_views_coherent() {
        for (layout, window) in [
            (GpioRegisterLayout::Nrf52, 0),
            (GpioRegisterLayout::Nrf52, 0x500),
            (GpioRegisterLayout::Nrf54l, 0),
        ] {
            let mut g = GpioPort::new_with_layout(layout).with_window_offset(window);
            let (out, dir, cnf) = if layout == GpioRegisterLayout::Nrf52 {
                (0x504, 0x514, 0x700 + 4 * 20)
            } else {
                (0, 0x10, 0x80 + 4 * 20)
            };
            let (out, dir, cnf) = (out - window, dir - window, cnf - window);
            let bit = 1 << 20;
            g.write_u32(cnf, 3 << 2).unwrap(); // Input with pull-up.
            assert_eq!(g.read_gpio_pad(20), Some(true));
            g.write_u32(dir + 4, bit).unwrap();
            for offset in [dir, dir + 4, dir + 8] {
                assert_eq!(g.read_u32(offset).unwrap(), bit);
            }
            assert_eq!(g.read_u32(cnf).unwrap(), (3 << 2) | 1);
            assert_eq!(g.read_gpio_pad(20), Some(false));
            g.write_u32(out + 4, bit).unwrap();
            for offset in [out, out + 4, out + 8] {
                assert_eq!(g.read_u32(offset).unwrap(), bit);
            }
            assert_eq!(g.read_gpio_pad(20), Some(true));
            g.write_u32(out + 8, bit).unwrap();
            assert_eq!(g.read_gpio_pad(20), Some(false));
            g.write_u32(dir + 8, bit).unwrap();
            assert_eq!(g.read_u32(cnf).unwrap(), 3 << 2);
            assert_eq!(g.read_gpio_pad(20), Some(true));
            // Subword RMW through PIN_CNF preserves the live direction bit.
            g.write_u32(dir, bit).unwrap();
            g.write(cnf + 1, 1).unwrap();
            assert_eq!(g.read_u32(dir).unwrap(), bit);
            assert_eq!(g.read_u32(cnf).unwrap(), (1 << 8) | (3 << 2) | 1);
            assert_eq!(g.read_gpio_pad(20), Some(false));
        }
    }
}

// Test the public byte and halfword paths: action-register reads must not be
// folded into a narrow write and clear untouched lanes.
mod narrow_store_contract {
    use crate::peripherals::gpio::{GpioPort, GpioRegisterLayout};
    use crate::Peripheral;

    #[test]
    fn narrow_set_and_clear_stores_affect_only_written_lanes() {
        for (layout, window) in [
            (GpioRegisterLayout::Nrf52, 0),
            (GpioRegisterLayout::Nrf52, 0x500),
            (GpioRegisterLayout::Nrf54l, 0),
        ] {
            let mut g = GpioPort::new_with_layout(layout).with_window_offset(window);
            let (out, dir) = if layout == GpioRegisterLayout::Nrf52 {
                (0x504, 0x514)
            } else {
                (0, 0x10)
            };
            for latch in [out - window, dir - window] {
                for byte in 0..4 {
                    g.write_u32(latch, u32::MAX).unwrap();
                    g.write(latch + 8 + byte, 0x81).unwrap();
                    assert_eq!(g.read_u32(latch).unwrap(), !(0x81 << (byte * 8)));
                    g.write(latch + 8 + byte, 0).unwrap();
                    assert_eq!(g.read_u32(latch).unwrap(), !(0x81 << (byte * 8)));
                    g.write_u32(latch, 0).unwrap();
                    g.write(latch + 4 + byte, 0x81).unwrap();
                    assert_eq!(g.read_u32(latch).unwrap(), 0x81 << (byte * 8));
                }
                for half in [0, 2] {
                    g.write_u32(latch, u32::MAX).unwrap();
                    g.write_u16(latch + 8 + half, 0x8181).unwrap();
                    assert_eq!(g.read_u32(latch).unwrap(), !(0x8181 << (half * 8)));
                    g.write_u32(latch, 0).unwrap();
                    g.write_u16(latch + 4 + half, 0x8181).unwrap();
                    assert_eq!(g.read_u32(latch).unwrap(), 0x8181 << (half * 8));
                }
            }
        }
    }
}
