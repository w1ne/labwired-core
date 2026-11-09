// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Classic-ESP32 PCNT fed by the GPIO input matrix, on the production
//! builder (`configure_xtensa_esp32`): routing is `GPIO_FUNCn_IN_SEL_CFG`
//! only, the stimulus is a pad level, and the result is read back over MMIO
//! exactly as `pcnt_get_counter_value` reads it.

#[cfg(test)]
mod tests {
    use crate::Bus;

    const GPIO: u64 = 0x3FF4_4000;
    const GPIO_OUT_W1TS: u64 = GPIO + 0x08;
    const GPIO_OUT_W1TC: u64 = GPIO + 0x0C;
    const GPIO_ENABLE_W1TS: u64 = GPIO + 0x24;
    const FUNC_IN_SEL: u64 = GPIO + 0x130;
    const PCNT: u64 = 0x3FF5_7000;
    const PCNT_CTRL: u64 = PCNT + 0xB0;
    const SIG_IN_SEL: u32 = 1 << 7;

    fn bus() -> (crate::bus::SystemBus, usize) {
        let mut bus = crate::bus::SystemBus::new();
        let _cpu = crate::system::xtensa::configure_xtensa_esp32(&mut bus);
        let gpio = bus.find_peripheral_index_by_name("gpio").unwrap();
        (bus, gpio)
    }

    fn count(bus: &crate::bus::SystemBus, unit: u64) -> i16 {
        bus.read_u32(PCNT + 0x60 + 4 * unit).unwrap() as u16 as i16
    }

    /// Route `pad` to input signal `sig` the way BROM `gpio_matrix_in` does.
    fn route(bus: &mut crate::bus::SystemBus, sig: u64, pad: u32) {
        bus.write_u32(FUNC_IN_SEL + 4 * sig, pad | SIG_IN_SEL)
            .unwrap();
    }

    /// `ESP32Encoder::attachFullQuad(a, b)` on unit 0, register for register:
    /// ch0 = (sig A, ctrl B), ch1 = (sig B, ctrl A), counts released.
    fn full_quad_unit0(bus: &mut crate::bus::SystemBus, a: u32, b: u32) {
        route(bus, 39, a); // PCNT_SIG_CH0_IN0
        route(bus, 41, b); // PCNT_CTRL_CH0_IN0
        route(bus, 40, b); // PCNT_SIG_CH1_IN0
        route(bus, 42, a); // PCNT_CTRL_CH1_IN0
        let ch0 = 1 | (2 << 2) | (1 << 6);
        let ch1 = 2 | (1 << 2) | (1 << 6);
        bus.write_u32(PCNT, 0x3C10 | (ch0 << 16) | (ch1 << 24))
            .unwrap();
        bus.write_u32(PCNT + 8, 32766 | (((-32766i16) as u16 as u32) << 16))
            .unwrap();
        let ctrl = bus.read_u32(PCNT_CTRL).unwrap();
        bus.write_u32(PCNT_CTRL, ctrl | 1).unwrap();
        bus.write_u32(PCNT_CTRL, ctrl & !0b11).unwrap();
    }

    /// One Gray step per call on pads `a`/`b` (A leads B for `dir` = +1).
    fn turn(
        bus: &mut crate::bus::SystemBus,
        gpio: usize,
        a: u8,
        b: u8,
        phase: &mut i32,
        steps: i32,
    ) {
        for _ in 0..steps.abs() {
            *phase += steps.signum();
            let ph = phase.rem_euclid(4);
            bus.set_peripheral_gpio_input(gpio, a, ph == 1 || ph == 2);
            bus.set_peripheral_gpio_input(gpio, b, ph == 2 || ph == 3);
        }
    }

    #[test]
    fn production_classic_builder_has_a_real_pcnt() {
        let (bus, _) = bus();
        assert_eq!(bus.read_u32(PCNT + 0xFC).unwrap(), 0x1412_2600, "PCNT DATE");
        assert_eq!(bus.read_u32(PCNT_CTRL).unwrap(), 0x5555, "CTRL reset");
        assert_eq!(bus.read_u32(PCNT).unwrap(), 0x3C10, "U0_CONF0 reset");
    }

    #[test]
    fn a_quadrature_encoder_on_routed_pads_counts_both_ways() {
        let (mut bus, gpio) = bus();
        // Idle encoder: both lines released high would be phase 0 low here.
        bus.set_peripheral_gpio_input(gpio, 25, false);
        bus.set_peripheral_gpio_input(gpio, 26, false);
        full_quad_unit0(&mut bus, 25, 26);
        let mut phase = 0;
        turn(&mut bus, gpio, 25, 26, &mut phase, 20);
        let fwd = count(&bus, 0);
        assert_eq!(fwd.abs(), 20, "full quad: one count per Gray step");
        turn(&mut bus, gpio, 25, 26, &mut phase, -32);
        assert_eq!(count(&bus, 0), fwd - 32 * fwd.signum(), "reverses");
    }

    #[test]
    fn an_unrouted_pad_counts_nothing() {
        let (mut bus, gpio) = bus();
        full_quad_unit0(&mut bus, 25, 26);
        let mut phase = 0;
        // Same waveform, two pads the matrix does not select.
        turn(&mut bus, gpio, 27, 14, &mut phase, 20);
        assert_eq!(count(&bus, 0), 0);
    }

    #[test]
    fn the_pads_own_output_is_counted_like_silicon() {
        // Input and output enabled on one pad: the input matrix samples the
        // pad, so a pulse train the firmware drives itself is counted
        // (the esp-idf `pcnt_event_example` loopback).
        let (mut bus, _) = bus();
        route(&mut bus, 39, 18);
        // ch0 pos = INC, unit 0 released.
        bus.write_u32(PCNT, 0x3C10 | (1 << 18)).unwrap();
        bus.write_u32(PCNT_CTRL, 0x5555 & !1).unwrap();
        bus.write_u32(GPIO_ENABLE_W1TS, 1 << 18).unwrap();
        for _ in 0..7 {
            bus.write_u32(GPIO_OUT_W1TS, 1 << 18).unwrap();
            bus.write_u32(GPIO_OUT_W1TC, 1 << 18).unwrap();
        }
        assert_eq!(count(&bus, 0), 7);
    }

    #[test]
    fn unit_5_uses_the_second_signal_block() {
        // Units 5..7 resume at matrix signal 71, not 59.
        let (mut bus, gpio) = bus();
        route(&mut bus, 71, 4); // PCNT_SIG_CH0_IN5
        bus.write_u32(PCNT + 5 * 12, 0x3C10 | (1 << 18)).unwrap();
        bus.write_u32(PCNT_CTRL, 0x5555 & !(1 << 10)).unwrap();
        for level in [true, false, true, false, true] {
            bus.set_peripheral_gpio_input(gpio, 4, level);
        }
        assert_eq!(count(&bus, 5), 3);
        assert_eq!(count(&bus, 0), 0);
    }
}
