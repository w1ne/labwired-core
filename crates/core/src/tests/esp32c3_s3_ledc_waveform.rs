// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! LEDC duty on the pad for the ESP32-C3 and ESP32-S3: the counterpart of
//! `esp32_ledc_waveform` for the two RISC-V/LX7 parts.
//!
//! Each gate programs the LEDC the way `ledcSetup(ch, 5000, 10)` +
//! `ledcWrite(ch, 256)` does (APB source, Q8 divider, 10-bit resolution,
//! DUTY_START), routes the channel to a pad through `FUNCn_OUT_SEL_CFG`, and
//! measures the pad with the logic analyzer. Frequency and duty are checked
//! against values computed here from the register fields, not read back from
//! the model.

#[cfg(test)]
mod esp32c3_s3_ledc_waveform_tests {
    use crate::cpu::CortexM;
    use crate::logic_capture::{LogicEdge, LogicSource};
    use crate::{Bus, Machine};

    const RAM_BASE: u64 = 0x2000_0000;
    const GPIO_BASE: u64 = 0x6000_4000;
    const LEDC_BASE: u64 = 0x6001_9000;
    const ENABLE_W1TS: u64 = 0x24;
    /// C3 and S3 both put `FUNC0_OUT_SEL_CFG` at GPIO + 0x554.
    const FUNC_OUT_SEL: u64 = 0x554;
    /// `LEDC_LS_SIG_OUT0_IDX`: 45 on the C3, 73 on the S3 (`gpio_sig_map.h`).
    const C3_SIG_LEDC0: u32 = 45;
    const S3_SIG_LEDC0: u32 = 73;

    // `ledc_reg.h` (C3 and S3 share the geometry).
    const CH_STRIDE: u64 = 0x14;
    const CH_CONF0: u64 = 0x00;
    const CH_DUTY: u64 = 0x08;
    const CH_CONF1: u64 = 0x0C;
    const TIMER0_CONF: u64 = 0xA0;
    const LEDC_CONF: u64 = 0xD0;
    const SIG_OUT_EN: u32 = 1 << 2;
    const DUTY_START: u32 = 1 << 31;
    const APB_CLK_SEL_APB: u32 = 1;

    const APB_HZ: u64 = 80_000_000;
    const DUTY_RES: u32 = 10;
    /// 80 MHz / (5 kHz * 2^10) = 15.625 → Q8 4000.
    const DIV_Q8: u32 = 4000;
    const PWM_HZ: f64 = 5_000.0;

    fn finish(mut bus: crate::bus::SystemBus) -> Machine<CortexM> {
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

    fn c3_machine() -> Machine<CortexM> {
        use crate::peripherals::esp32c3::gpio::Esp32c3Gpio;
        use crate::peripherals::esp32c3::ledc::{Esp32c3Ledc, LEDC_INTR_SOURCE_ID};
        let mut bus = crate::bus::SystemBus::new();
        bus.cpu_hz = 160_000_000;
        bus.add_peripheral(
            "gpio",
            GPIO_BASE,
            0x1000,
            None,
            Box::new(Esp32c3Gpio::new()),
        );
        bus.add_peripheral(
            "ledc",
            LEDC_BASE,
            0x400,
            None,
            Box::new(Esp32c3Ledc::new(LEDC_INTR_SOURCE_ID)),
        );
        bus.wire_esp32c3_ledc_pads();
        finish(bus)
    }

    fn s3_machine() -> Machine<CortexM> {
        use crate::peripherals::esp32s3::gpio::Esp32s3Gpio;
        use crate::peripherals::esp32s3::ledc::Esp32s3Ledc;
        let mut bus = crate::bus::SystemBus::new();
        bus.cpu_hz = 240_000_000;
        bus.add_peripheral(
            "gpio",
            GPIO_BASE,
            0x1000,
            None,
            Box::new(Esp32s3Gpio::new()),
        );
        bus.add_peripheral(
            "ledc",
            LEDC_BASE,
            0x1000,
            None,
            Box::new(Esp32s3Ledc::new()),
        );
        bus.wire_esp32s3_ledc_pads();
        finish(bus)
    }

    fn route(m: &mut Machine<CortexM>, pin: u8, signal: u32) {
        m.bus
            .write_u32(GPIO_BASE + ENABLE_W1TS, 1u32 << pin)
            .unwrap();
        m.bus
            .write_u32(GPIO_BASE + FUNC_OUT_SEL + u64::from(pin) * 4, signal)
            .unwrap();
    }

    /// `ledcSetup` + `ledcWrite`: APB source, timer 0 at 5 kHz / 10 bit (RST
    /// released), channel `ch` on timer 0 with `duty` committed by DUTY_START.
    fn start_channel(m: &mut Machine<CortexM>, ch: u64, duty: u32) {
        m.bus
            .write_u32(LEDC_BASE + LEDC_CONF, APB_CLK_SEL_APB)
            .unwrap();
        m.bus
            .write_u32(LEDC_BASE + TIMER0_CONF, DUTY_RES | (DIV_Q8 << 4))
            .unwrap();
        let base = LEDC_BASE + ch * CH_STRIDE;
        m.bus.write_u32(base + CH_CONF0, SIG_OUT_EN).unwrap();
        m.bus.write_u32(base + CH_DUTY, duty << 4).unwrap();
        m.bus.write_u32(base + CH_CONF1, DUTY_START).unwrap();
    }

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

    /// Run `periods` PWM periods with `pin` on the analyzer and check every
    /// whole period's frequency and duty against the register-derived values.
    fn measure(m: &mut Machine<CortexM>, pin: u8, cpu_hz: u64, duty: u32, chip: &str) {
        let gpio = m.bus.find_peripheral_index_by_name("gpio").unwrap();
        m.logic_watch(&[Some(LogicSource::pad(gpio, pin))]);
        let period_cycles = (cpu_hz as f64 / PWM_HZ) as u64;
        for _ in 0..(4 * period_cycles + period_cycles / 2) {
            m.step().unwrap();
        }
        let p = pulses(&m.logic_read_edges(0).edges, 0);
        assert!(
            p.len() >= 3,
            "{chip} GPIO{pin} must show at least 3 whole PWM pulses, saw {p:?}"
        );
        let want_duty = f64::from(duty) / f64::from(1u32 << DUTY_RES);
        // Counter clock check: one period is 2^res ticks of APB / (Q8 / 256).
        let ticks_hz = APB_HZ as f64 * 256.0 / f64::from(DIV_Q8);
        assert!(
            (ticks_hz / f64::from(1u32 << DUTY_RES) - PWM_HZ).abs() < 1e-6,
            "test constants must describe 5 kHz"
        );
        for w in p.windows(2) {
            let period = (w[1].0 - w[0].0) as f64;
            let hz = cpu_hz as f64 / period;
            assert!(
                (hz - PWM_HZ).abs() / PWM_HZ < 1e-3,
                "{chip} GPIO{pin}: {hz:.2} Hz, want {PWM_HZ} Hz (pulses {p:?})"
            );
            let high = (w[0].1 - w[0].0) as f64;
            assert!(
                (high / period - want_duty).abs() < 2e-3,
                "{chip} GPIO{pin}: duty {:.4}, want {want_duty:.4} (pulses {p:?})",
                high / period
            );
        }
    }

    #[test]
    fn c3_ledc_duty_and_frequency_reach_the_routed_pad() {
        let mut m = c3_machine();
        route(&mut m, 2, C3_SIG_LEDC0 + 1);
        start_channel(&mut m, 1, 256);
        measure(&mut m, 2, 160_000_000, 256, "C3");
    }

    #[test]
    fn s3_ledc_duty_and_frequency_reach_the_routed_pad() {
        let mut m = s3_machine();
        route(&mut m, 5, S3_SIG_LEDC0 + 3);
        start_channel(&mut m, 3, 768);
        measure(&mut m, 5, 240_000_000, 768, "S3");
    }

    /// A pad left on plain GPIO (signal 128 on the C3, 256 on the S3) stays
    /// flat while a sibling pad carries the PWM.
    #[test]
    fn an_unrouted_pad_stays_flat() {
        for (mut m, sig0, plain) in [
            (c3_machine(), C3_SIG_LEDC0, 128u32),
            (s3_machine(), S3_SIG_LEDC0, 256u32),
        ] {
            route(&mut m, 4, sig0);
            route(&mut m, 6, plain);
            start_channel(&mut m, 0, 512);
            let gpio = m.bus.find_peripheral_index_by_name("gpio").unwrap();
            m.logic_watch(&[
                Some(LogicSource::pad(gpio, 4)),
                Some(LogicSource::pad(gpio, 6)),
            ]);
            for _ in 0..80_000 {
                m.step().unwrap();
            }
            let edges = m.logic_read_edges(0).edges;
            assert!(
                edges.iter().any(|e| e.ch == 0),
                "routed pad carries LEDC ch0"
            );
            assert!(
                !edges.iter().any(|e| e.ch == 1),
                "plain-GPIO pad stays flat"
            );
        }
    }

    fn manifest_for(path: &std::path::Path) -> labwired_config::SystemManifest {
        labwired_config::SystemManifest {
            parts: Vec::new(),
            walk_deleted: Some(false),
            schema_version: "1.0".to_string(),
            name: "esp-ledc-waveform".to_string(),
            chip: path.to_string_lossy().to_string(),
            cpu_hz: None,
            external_devices: vec![],
            cosim_models: Vec::new(),
            motor_models: Vec::new(),
            board_io: vec![],
            debug_uart: None,
            wifi_ap: None,
            peripherals: vec![],
            memory_overrides: Default::default(),
        }
    }

    fn from_config_bound(chip_yaml: &str) -> Vec<&'static str> {
        let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .join("configs/chips")
            .join(chip_yaml);
        let chip = labwired_config::ChipDescriptor::from_file(&path).expect("load chip yaml");
        let bus = crate::bus::SystemBus::from_config(&chip, &manifest_for(&path))
            .expect("assemble the production bus");
        bus.bound_pad_functions()
    }

    /// The production builders bind the outputs: `from_config` for the C3 and
    /// `configure_xtensa_esp32s3` for the S3.
    #[test]
    fn the_production_c3_and_s3_builders_bind_the_ledc_outputs() {
        let bound = from_config_bound("esp32c3.yaml");
        for want in ["LEDC_LS_CH0", "LEDC_LS_CH5"] {
            assert!(
                bound.contains(&want),
                "C3 from_config must bind {want}: {bound:?}"
            );
        }
        let mut bus = crate::bus::SystemBus::new();
        let _ = crate::system::xtensa::configure_xtensa_esp32s3(
            &mut bus,
            &crate::system::xtensa::Esp32s3Opts::default(),
        );
        let bound = bus.bound_pad_functions();
        for want in ["LEDC_LS_CH0", "LEDC_LS_CH7"] {
            assert!(
                bound.contains(&want),
                "configure_xtensa_esp32s3 must bind {want}: {bound:?}"
            );
        }
    }

    /// The C6 reuses the C3 LEDC and GPIO models, but its matrix numbers 45/46
    /// as I²C — LEDC must not claim them there.
    #[test]
    fn the_c6_does_not_bind_c3_ledc_signal_numbers() {
        let bound = from_config_bound("esp32c6.yaml");
        assert!(
            !bound.iter().any(|f| f.starts_with("LEDC_")),
            "C6 from_config must not bind LEDC outputs: {bound:?}"
        );
    }
}
