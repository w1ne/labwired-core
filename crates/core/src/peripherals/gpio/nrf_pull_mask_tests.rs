// SPDX-License-Identifier: MIT
//! Differential proof against the pre-optimization pull scan, not a hardware claim.
use super::Nrf52Gpio;

fn reference(g: &Nrf52Gpio) -> u32 {
    let mut apply = 0;
    let mut level = 0;
    for pin in 0..g.num_pins.min(32) as usize {
        match (g.pin_cnf[pin] >> 2) & 3 {
            1 => apply |= 1 << pin,
            3 => {
                apply |= 1 << pin;
                level |= 1 << pin;
            }
            _ => {}
        }
    }
    let undriven = !g.dir;
    let from_pull = undriven & apply & !g.external;
    (g.odr & g.dir) | (level & from_pull) | (g.idr & undriven & !from_pull)
}

fn check(g: &Nrf52Gpio) {
    assert_eq!(g.effective_in(), reference(g));
    assert_eq!(g.pull_apply & !g.pin_mask(), 0);
    assert_eq!(g.pull_level & !g.pull_apply, 0);
}

#[test]
fn every_pin_pull_direction_drive_and_latch_combination_matches_old_scan() {
    for count in [0, 13, 16, 32] {
        let mut g = Nrf52Gpio::with_num_pins(count);
        for pin in 0..32 {
            let bit = 1u32 << pin;
            for pull in 0..4 {
                for output in [false, true] {
                    for external in [false, true] {
                        for latch in [false, true] {
                            for out in [false, true] {
                                g.write_reg(0x700 + 4 * pin, pull << 2 | u32::from(output));
                                g.write_reg(0x504, if out { bit } else { 0 });
                                g.write_reg(0x510, if latch { bit } else { 0 });
                                g.external = if external { bit } else { 0 };
                                check(&g);
                                // Releasing external drive reveals the live pull.
                                g.external = 0;
                                check(&g);
                            }
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn randomized_mixed_register_writes_match_old_scan() {
    for count in [0, 13, 16, 32] {
        let mut g = Nrf52Gpio::with_num_pins(count);
        let mut rng = 0x6b8c239du32;
        for i in 0..8192 {
            rng ^= rng << 13;
            rng ^= rng >> 17;
            rng ^= rng << 5;
            let offset = match i % 12 {
                0..=3 => 0x700 + 4 * u64::from((rng >> 16) & 31),
                4 => 0x504,
                5 => 0x508,
                6 => 0x50c,
                7 => 0x514,
                8 => 0x518,
                9 => 0x51c,
                10 => super::NRF52_GPIO_PAD_LATCH,
                _ => 0x510,
            };
            g.write_reg(offset, rng);
            g.external = rng.rotate_left(11);
            check(&g);
            g.external = 0;
            check(&g);
        }
    }
}

#[test]
fn invalid_bank_pins_cannot_change_masks_and_reserved_pull_removes_previous_pull() {
    for count in [0, 13, 16, 32] {
        let mut g = Nrf52Gpio::with_num_pins(count);
        for pin in 0..32 {
            g.write_reg(0x700 + 4 * pin, 12);
        }
        assert_eq!(g.pull_apply, g.pin_mask());
        assert_eq!(g.pull_level, g.pin_mask());
        for pin in 0..32 {
            assert_eq!(
                g.read_reg(0x700 + 4 * pin),
                if pin < u64::from(count) { 12 } else { 0 }
            );
            g.write_reg(0x700 + 4 * pin, 8); // reserved PULL=2
        }
        assert_eq!((g.pull_apply, g.pull_level), (0, 0));
        // IN latches historically include missing-pin bits; preserve that
        // model behavior instead of silently changing the register contract.
        g.write_reg(0x510, u32::MAX);
        assert_eq!(g.effective_in(), u32::MAX);
    }
}

#[test]
fn bulk_direction_and_pad_latch_changes_use_existing_cached_resistors() {
    let mut g = Nrf52Gpio::default();
    g.write_reg(0x700, 12);
    g.write_reg(0x704, 4);
    let masks = (g.pull_apply, g.pull_level);
    g.write_reg(0x514, 3);
    g.write_reg(0x504, 2);
    assert_eq!(g.effective_in() & 3, 2);
    g.write_reg(0x51c, 3);
    assert_eq!(g.effective_in() & 3, 1);
    g.external = 3;
    g.write_reg(super::NRF52_GPIO_PAD_LATCH, 0);
    g.write_reg(super::NRF52_GPIO_PAD_LATCH, 1 | (1 << 8));
    assert_eq!(g.effective_in() & 3, 2);
    g.external = 0;
    assert_eq!(g.effective_in() & 3, 1);
    assert_eq!((g.pull_apply, g.pull_level), masks);
    check(&g);
}

#[test]
fn default_and_zero_pin_constructors_have_no_cached_pulls() {
    for count in [0, 13, 16, 32] {
        let g = Nrf52Gpio::with_num_pins(count);
        assert_eq!((g.pull_apply, g.pull_level, g.effective_in()), (0, 0, 0));
        check(&g);
    }
}

#[test]
fn no_pull_fast_path_survives_pull_release_and_full_word_input_changes() {
    for count in [0, 13, 16, 32] {
        let mut g = Nrf52Gpio::with_num_pins(count);
        for pin in 0..32 {
            g.write_reg(0x700 + 4 * pin, 12);
        }
        check(&g);
        for disabled in [0, 8] {
            for pin in 0..32 {
                g.write_reg(0x700 + 4 * pin, disabled);
            }
            assert_eq!(g.pull_apply, 0);
            let mut rng = 0xa7959928u32;
            for _ in 0..4096 {
                rng ^= rng << 13;
                rng ^= rng >> 17;
                rng ^= rng << 5;
                g.write_reg(0x504, rng);
                g.write_reg(0x510, rng.rotate_left(7));
                g.write_reg(0x514, rng.rotate_left(19));
                for external in [0, u32::MAX, rng.rotate_left(11)] {
                    g.external = external;
                    check(&g);
                    assert_eq!(g.effective_in(), (g.odr & g.dir) | (g.idr & !g.dir));
                }
            }
            // Re-enable the last valid pin, including bit 31 on full ports.
            if count != 0 {
                g.write_reg(0x700 + 4 * u64::from(count - 1), 4);
                assert_ne!(g.pull_apply, 0);
                check(&g);
            }
        }
    }
}

mod port_contract {
    use super::super::{GpioPort, GpioRegisterLayout};
    use crate::Peripheral;

    #[test]
    fn subword_writes_and_nrf54_translation_keep_pull_masks_coherent() {
        for layout in [GpioRegisterLayout::Nrf52, GpioRegisterLayout::Nrf54l] {
            let mut g = GpioPort::new_with_layout(layout);
            let (cnf, input) = if layout == GpioRegisterLayout::Nrf52 {
                (0x700, 0x510)
            } else {
                (0x80, 0xc)
            };
            g.write(cnf, 12).unwrap();
            assert_eq!(g.read_u32(input).unwrap() & 1, 1);
            g.write(cnf + 1, 0xa5).unwrap(); // unrelated high byte must not lose PULL
            assert_eq!(g.read_u32(input).unwrap() & 1, 1);
            g.write_u16(cnf, 4).unwrap();
            assert_eq!(g.read_u32(input).unwrap() & 1, 0);
            g.write_u32(cnf, 8).unwrap(); // reserved -> latched IN
            assert!(g.set_gpio_input(0, true));
            assert_eq!(g.read_u32(input).unwrap() & 1, 1);
        }
    }

    #[test]
    fn derived_masks_do_not_change_snapshot_register_schema() {
        let mut g = GpioPort::new_nrf52(13);
        g.write_u32(0x700, 12).unwrap();
        let snapshot = g.snapshot();
        assert_eq!(snapshot.as_object().unwrap().len(), 6);
        assert!(snapshot.get("pull_apply").is_none());
        assert!(snapshot.get("pull_level").is_none());
        assert_eq!(snapshot["pin_cnf"][0], 12);
        assert_eq!(snapshot["num_pins"], 13);
    }
}
