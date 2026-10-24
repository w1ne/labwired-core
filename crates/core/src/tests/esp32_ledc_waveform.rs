// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! End-to-end gate for classic-ESP32 LEDC PWM on GPIO pads.
//!
//! Before this, the classic LEDC model was a register bank: a sketch that ran
//! `ledcSetup` + `ledcAttachPin` + `ledcWrite` showed SIG_OUT_EN=1 and the
//! right DUTY_R, and every attached pad recorded ZERO edges, so a servo or
//! LED driven the normal way could not be measured at all.
//!
//! These tests drive the registers the way arduino-esp32 does, route the pads
//! through the GPIO output matrix with the signal indices from esp-idf
//! `soc/esp32/include/soc/gpio_sig_map.h` (hand-entered here, not imported
//! from the model, so a wrong model constant is a disagreement and not a
//! shared mistake), watch them with the in-engine logic analyzer, and measure
//! period and high time from the captured edges.

#[cfg(test)]
mod esp32_ledc_waveform_tests {
    use crate::cpu::CortexM;
    use crate::logic_capture::{LogicEdge, LogicSource};
    use crate::{Bus, Machine};

    const RAM_BASE: u64 = 0x2000_0000;
    const GPIO_BASE: u64 = 0x3FF4_4000;
    const LEDC_BASE: u64 = 0x3FF5_9000;
    /// `GPIO_ENABLE_W1TS_REG` / `GPIO_ENABLE1_W1TS_REG` (pads 32..39).
    const ENABLE_W1TS: u64 = 0x24;
    const ENABLE1_W1TS: u64 = 0x30;
    /// `GPIO_FUNC0_OUT_SEL_CFG_REG`, stride 4.
    const FUNC_OUT_SEL: u64 = 0x530;
    /// `LEDC_HS_SIG_OUT0_IDX` (71) and `LEDC_LS_SIG_OUT0_IDX` (79).
    const SIG_LEDC_HS0: u32 = 71;
    const SIG_LEDC_LS0: u32 = 79;
    /// `SIG_GPIO_OUT_IDX` — the matrix bypass.
    const SIG_GPIO_OUT: u32 = 256;

    // `ledc_reg.h`: HSTIMERx_CONF at 0x140 + 8x, HSCHn block at 0x14 * n.
    const HSTIMER0_CONF: u64 = 0x140;
    const LSTIMER0_CONF: u64 = 0x160;
    const LSCH0: u64 = 0xA0;
    const CH_CONF0: u64 = 0x00;
    const CH_DUTY: u64 = 0x08;
    const CH_CONF1: u64 = 0x0C;
    const LEDC_CONF: u64 = 0x190;
    const SIG_OUT_EN: u32 = 1 << 2;
    const DUTY_START: u32 = 1 << 31;
    const TICK_SEL_APB: u32 = 1 << 25;

    /// 10-bit, DIV_NUM = 256 (/1) on the 80 MHz APB: 1024 ticks x 3 CPU
    /// cycles = 3072 cycles a frame on the 240 MHz core. Short enough to step.
    const FAST_CONF: u32 = 10 | (256 << 5) | TICK_SEL_APB;
    const FAST_PERIOD: u64 = 1024 * 3;

    /// A NOP slab so `step()` advances the cycle axis one cycle at a time.
    fn machine() -> Machine<CortexM> {
        use crate::peripherals::esp32::gpio::Esp32Gpio;
        use crate::peripherals::esp32::ledc::Ledc;
        let mut bus = crate::bus::SystemBus::new();
        bus.cpu_hz = 240_000_000;
        bus.add_peripheral("gpio", GPIO_BASE, 0x1000, None, Box::new(Esp32Gpio::new()));
        bus.add_peripheral(
            "ledc",
            LEDC_BASE,
            0x1000,
            None,
            Box::new(Ledc::new(Ledc::BASE)),
        );
        bus.wire_esp32_ledc_pads();
        let (cpu, _nvic) = crate::system::cortex_m::configure_cortex_m(&mut bus);
        let mut machine = Machine::new(cpu, bus);
        for i in 0..1022u64 {
            let byte = if i % 2 == 0 { 0x00 } else { 0x20 };
            machine.bus.write_u8(RAM_BASE + i, byte).unwrap();
        }
        machine.bus.write_u8(RAM_BASE + 1022, 0xFF).unwrap();
        machine.bus.write_u8(RAM_BASE + 1023, 0xE5).unwrap();
        machine.cpu.pc = RAM_BASE as u32;
        machine
    }

    /// `pinMode(pin, OUTPUT)` + `gpio_matrix_out(pin, signal, false, false)`.
    fn route(m: &mut Machine<CortexM>, pin: u8, signal: u32) {
        let (reg, bit) = if pin < 32 {
            (ENABLE_W1TS, pin)
        } else {
            (ENABLE1_W1TS, pin - 32)
        };
        m.bus.write_u32(GPIO_BASE + reg, 1u32 << bit).unwrap();
        m.bus
            .write_u32(GPIO_BASE + FUNC_OUT_SEL + u64::from(pin) * 4, signal)
            .unwrap();
    }

    /// `ledcWrite(ch, duty)` on an HS channel bound to timer `ch / 2`.
    fn hs_channel(m: &mut Machine<CortexM>, ch: u64, duty: u32) {
        let base = LEDC_BASE + ch * 0x14;
        m.bus
            .write_u32(base + CH_CONF0, SIG_OUT_EN | (ch / 2) as u32)
            .unwrap();
        m.bus.write_u32(base + CH_DUTY, duty << 4).unwrap();
        m.bus.write_u32(base + CH_CONF1, DUTY_START).unwrap();
    }

    fn run(m: &mut Machine<CortexM>, cycles: u64) {
        for _ in 0..cycles {
            m.step().unwrap();
        }
    }

    /// (rise, fall) cycle pairs of channel `ch`, complete pulses only.
    fn pulses(edges: &[LogicEdge], ch: u32) -> Vec<(u64, u64)> {
        let mut out = Vec::new();
        let mut rise = None;
        for e in edges.iter().filter(|e| e.ch == ch) {
            match (e.value, rise) {
                (true, _) => rise = Some(e.cycle),
                (false, Some(r)) => {
                    out.push((r, e.cycle));
                    rise = None;
                }
                (false, None) => {}
            }
        }
        out
    }

    #[test]
    fn four_servo_pins_carry_ledc_pwm_at_their_duties() {
        // The drawing-robot pin map: GPIO25/26/27/33 on HS channels 0..3.
        let mut m = machine();
        let pins = [25u8, 26, 27, 33];
        let duties = [256u32, 384, 512, 320];
        for t in 0..2u64 {
            m.bus
                .write_u32(LEDC_BASE + HSTIMER0_CONF + 8 * t, FAST_CONF)
                .unwrap();
        }
        for (ch, (&pin, &duty)) in pins.iter().zip(duties.iter()).enumerate() {
            route(&mut m, pin, SIG_LEDC_HS0 + ch as u32);
            hs_channel(&mut m, ch as u64, duty);
        }
        let gpio = m.bus.find_peripheral_index_by_name("gpio").unwrap();
        let watch: Vec<_> = pins
            .iter()
            .map(|&p| Some(LogicSource::pad(gpio, p)))
            .collect();
        m.logic_watch(&watch);
        run(&mut m, 5 * FAST_PERIOD + 100);

        let edges = m.logic_read_edges(0).edges;
        for (ch, &duty) in duties.iter().enumerate() {
            let p = pulses(&edges, ch as u32);
            assert!(
                p.len() >= 3,
                "GPIO{} (HS ch{ch}) must show >= 3 complete pulses, saw {p:?}",
                pins[ch]
            );
            for w in p.windows(2) {
                assert_eq!(w[1].0 - w[0].0, FAST_PERIOD, "GPIO{} period", pins[ch]);
            }
            for &(r, f) in &p {
                assert_eq!(f - r, u64::from(duty) * 3, "GPIO{} high time", pins[ch]);
            }
        }
    }

    #[test]
    fn a_low_speed_channel_reaches_its_pad_through_signal_79() {
        let mut m = machine();
        m.bus.write_u32(LEDC_BASE + LEDC_CONF, 1).unwrap(); // SLOW_CLK = APB
        m.bus
            .write_u32(LEDC_BASE + LSTIMER0_CONF, FAST_CONF)
            .unwrap();
        route(&mut m, 4, SIG_LEDC_LS0);
        m.bus
            .write_u32(LEDC_BASE + LSCH0 + CH_CONF0, SIG_OUT_EN)
            .unwrap();
        m.bus
            .write_u32(LEDC_BASE + LSCH0 + CH_DUTY, 100 << 4)
            .unwrap();
        m.bus
            .write_u32(LEDC_BASE + LSCH0 + CH_CONF1, DUTY_START)
            .unwrap();
        let gpio = m.bus.find_peripheral_index_by_name("gpio").unwrap();
        m.logic_watch(&[Some(LogicSource::pad(gpio, 4))]);
        run(&mut m, 4 * FAST_PERIOD);
        let p = pulses(&m.logic_read_edges(0).edges, 0);
        assert!(p.len() >= 2, "LS ch0 on GPIO4: {p:?}");
        assert!(p.iter().all(|&(r, f)| f - r == 300));
    }

    /// The control case: a pad left on the GPIO latch must not pick up a
    /// running LEDC channel, and a pad routed to a different channel shows
    /// that channel, not channel 0.
    #[test]
    fn only_the_pad_routed_to_a_channel_carries_it() {
        let mut m = machine();
        m.bus
            .write_u32(LEDC_BASE + HSTIMER0_CONF, FAST_CONF)
            .unwrap();
        hs_channel(&mut m, 0, 256);
        route(&mut m, 25, SIG_LEDC_HS0);
        route(&mut m, 26, SIG_GPIO_OUT);
        route(&mut m, 27, SIG_LEDC_HS0 + 1); // HS ch1: never configured
        let gpio = m.bus.find_peripheral_index_by_name("gpio").unwrap();
        m.logic_watch(&[
            Some(LogicSource::pad(gpio, 25)),
            Some(LogicSource::pad(gpio, 26)),
            Some(LogicSource::pad(gpio, 27)),
        ]);
        run(&mut m, 3 * FAST_PERIOD);
        let edges = m.logic_read_edges(0).edges;
        assert!(edges.iter().any(|e| e.ch == 0), "GPIO25 carries HS ch0");
        assert!(!edges.iter().any(|e| e.ch == 1), "GPIO26 is a plain GPIO");
        assert!(
            !edges.iter().any(|e| e.ch == 2),
            "GPIO27 carries the idle HS ch1"
        );
    }

    /// The production builder every real classic lab is made from binds the
    /// LEDC outputs — a waveform gate on a hand-built bus proves the model, not
    /// that anything ships it.
    #[test]
    fn the_production_classic_esp32_builder_binds_the_ledc_outputs() {
        let mut bus = crate::bus::SystemBus::new();
        let _cpu = crate::system::xtensa::configure_xtensa_esp32(&mut bus);
        let bound = bus.bound_pad_functions();
        for want in [
            "LEDC_HS_SIG_OUT0",
            "LEDC_HS_SIG_OUT7",
            "LEDC_LS_SIG_OUT0",
            "LEDC_LS_SIG_OUT7",
        ] {
            assert!(
                bound.contains(&want),
                "configure_xtensa_esp32 must bind {want}; bound functions were {bound:?}"
            );
        }
    }
}
