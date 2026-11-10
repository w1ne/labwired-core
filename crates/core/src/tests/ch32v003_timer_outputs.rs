// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! CH32V003 TIM2 channel outputs on their WCH pads, built from the COMMITTED
//! `configs/chips/ch32v003f4p6.yaml` through `SystemBus::from_config`.
//!
//! The register values are the ones WCH's official Arduino core writes for
//! `analogWrite(PD4, 128)` at its defaults: `PWM_FREQUENCY` 1000 Hz and
//! `PWM_RESOLUTION` 12 bits (arduino_core_ch32 `cores/arduino/pins_arduino.h`),
//! at HCLK = 48 MHz. `HardwareTimer::setOverflow` gives PSC = 0 and ARR =
//! 47999. `setCaptureCompare(128, 12 bit)` gives CCR1 = 128 * 48000 / 4096 =
//! 1500. PD4 is TIM2_CH1 in variant_CH32V003F4 `PinMap_TIM`. The waveform is
//! decoded from the logic-analyzer edges alone, as in `stm32f1_timer_outputs`.

#[cfg(test)]
mod ch32v003_timer_outputs_tests {
    use crate::bus::SystemBus;
    use crate::cpu::RiscV;
    use crate::logic_capture::{LogicEdge, LogicSource};
    use crate::{Bus, Machine};
    use labwired_config::{ChipDescriptor, SystemManifest};
    use std::path::PathBuf;

    const RAM_BASE: u64 = 0x2000_0000;
    // ch32v00x.h / ch32v00x_rcc.h.
    const RCC_BASE: u64 = 0x4002_1000;
    const APB2PCENR: u64 = RCC_BASE + 0x18;
    const APB1PCENR: u64 = RCC_BASE + 0x1C;
    const AFIOEN: u32 = 1 << 0;
    const IOPAEN: u32 = 1 << 2;
    const IOPDEN: u32 = 1 << 5;
    const TIM2EN: u32 = 1 << 0;
    const GPIOA_BASE: u64 = 0x4001_0800;
    const GPIOD_BASE: u64 = 0x4001_1400;
    const TIM2_BASE: u64 = 0x4000_0000;

    // TIM_TypeDef offsets: CTLR1, CHCTLR1, CCER, PSC, ATRLR, CH1CVR, CH2CVR.
    const CTLR1: u64 = 0x00;
    const CHCTLR1: u64 = 0x18;
    const CCER: u64 = 0x20;
    const PSC: u64 = 0x28;
    const ATRLR: u64 = 0x2C;
    const CH1CVR: u64 = 0x34;
    const CH2CVR: u64 = 0x38;
    const CEN: u32 = 1;
    /// OC1M = 110 (PWM mode 1) | OC1PE: `TIM_OCMode_PWM1` with preload.
    const OC1_PWM1: u32 = (0b110 << 4) | (1 << 3);
    const OC2_PWM1: u32 = OC1_PWM1 << 8;
    const CC1E: u32 = 1 << 0;
    const CC2E: u32 = 1 << 4;
    /// CFGLR nibble MODE = 11, CNF = 10: `GPIO_Mode_AF_PP`, 50 MHz.
    const AF_PUSH_PULL: u32 = 0b1011;

    const PERIOD: u64 = 48_000;
    const HIGH: u64 = 1_500;

    fn ch32_machine() -> Machine<RiscV> {
        let path =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../configs/chips/ch32v003f4p6.yaml");
        let chip = ChipDescriptor::from_file(&path).expect("load ch32v003f4p6.yaml");
        let mut bus = SystemBus::from_config(&chip, &SystemManifest::default())
            .expect("assemble ch32v003f4p6");
        let cpu = crate::system::riscv::configure_riscv(&mut bus);
        let mut machine = Machine::new(cpu, bus);
        // `j .` from RAM: steps advance cycles and touch no register.
        machine.bus.write_u32(RAM_BASE, 0x0000_006F).unwrap();
        machine.cpu.pc = RAM_BASE as u32;
        machine
    }

    fn set_af_pp(m: &mut Machine<RiscV>, port_base: u64, pin: u8) {
        // CH32V003 ports have pins 0-7 only, all in CFGLR (+0x00).
        let shift = u32::from(pin) * 4;
        let old = m.bus.read_u32(port_base).unwrap();
        m.bus
            .write_u32(port_base, (old & !(0xF << shift)) | (AF_PUSH_PULL << shift))
            .unwrap();
    }

    fn run_cycles(m: &mut Machine<RiscV>, cycles: u64) {
        let target = m.total_cycles + cycles;
        while m.total_cycles < target {
            m.step().unwrap();
        }
    }

    fn steady_edges(m: &mut Machine<RiscV>, warmup: u64, cycles: u64) -> Vec<LogicEdge> {
        run_cycles(m, warmup);
        let cursor = m.logic_read_edges(0).cursor;
        run_cycles(m, cycles);
        m.logic_read_edges(cursor).edges
    }

    /// (period, high) from transitions only; every whole period must agree.
    fn period_and_high(edges: &[LogicEdge]) -> (u64, u64) {
        let mut last = None;
        let edges: Vec<LogicEdge> = edges
            .iter()
            .filter(|e| {
                let keep = last != Some(e.value);
                last = Some(e.value);
                keep
            })
            .copied()
            .collect();
        let rises: Vec<u64> = edges.iter().filter(|e| e.value).map(|e| e.cycle).collect();
        let falls: Vec<u64> = edges.iter().filter(|e| !e.value).map(|e| e.cycle).collect();
        assert!(rises.len() >= 3, "need two whole periods, got {edges:?}");
        let period = rises[1] - rises[0];
        assert!(rises.windows(2).all(|p| p[1] - p[0] == period), "{rises:?}");
        let highs: Vec<u64> = rises[..rises.len() - 1]
            .iter()
            .map(|&r| {
                falls
                    .iter()
                    .copied()
                    .find(|&f| f > r)
                    .expect("fall after rise")
                    - r
            })
            .collect();
        assert!(highs.iter().all(|&h| h == highs[0]), "{highs:?}");
        (period, highs[0])
    }

    #[test]
    fn wch_analog_write_128_drives_pd4_at_1khz_and_128_of_4096() {
        let mut m = ch32_machine();
        m.bus.write_u32(APB2PCENR, AFIOEN | IOPDEN).unwrap();
        m.bus.write_u32(APB1PCENR, TIM2EN).unwrap();
        set_af_pp(&mut m, GPIOD_BASE, 4);
        let gpiod = m.bus.find_peripheral_index_by_name("gpiod").unwrap();
        m.logic_watch(&[Some(LogicSource::pad(gpiod, 4))]);

        m.bus.write_u32(TIM2_BASE + PSC, 0).unwrap();
        m.bus
            .write_u32(TIM2_BASE + ATRLR, (PERIOD - 1) as u32)
            .unwrap();
        m.bus.write_u32(TIM2_BASE + CH1CVR, HIGH as u32).unwrap();
        m.bus.write_u32(TIM2_BASE + CHCTLR1, OC1_PWM1).unwrap();
        m.bus.write_u32(TIM2_BASE + CCER, CC1E).unwrap();
        m.bus.write_u32(TIM2_BASE + CTLR1, CEN).unwrap();
        let edges = steady_edges(&mut m, 2 * PERIOD, 4 * PERIOD);
        assert_eq!(period_and_high(&edges), (PERIOD, HIGH));
        let routed = m.bus.peripherals[gpiod]
            .dev
            .gpio_routing(4)
            .and_then(|r| r.func);
        assert_eq!(
            routed.as_deref(),
            Some("TIM2_CH1"),
            "PinMap_TIM: PD4 = TIM2_CH1"
        );
    }

    /// Negative control: PA1 is TIM2_CH2 on an STM32F103 but TIM1_CH2 on a
    /// CH32V003. A running TIM2 CH2 must not reach it, so the F1 pad map is
    /// not what produced the PD4 waveform above.
    #[test]
    fn f1_tim2_pad_pa1_carries_nothing_on_ch32v003() {
        let mut m = ch32_machine();
        m.bus.write_u32(APB2PCENR, AFIOEN | IOPAEN).unwrap();
        m.bus.write_u32(APB1PCENR, TIM2EN).unwrap();
        set_af_pp(&mut m, GPIOA_BASE, 1);
        let gpioa = m.bus.find_peripheral_index_by_name("gpioa").unwrap();
        m.logic_watch(&[Some(LogicSource::pad(gpioa, 1))]);

        m.bus.write_u32(TIM2_BASE + ATRLR, 999).unwrap();
        m.bus.write_u32(TIM2_BASE + CH2CVR, 300).unwrap();
        m.bus.write_u32(TIM2_BASE + CHCTLR1, OC2_PWM1).unwrap();
        m.bus.write_u32(TIM2_BASE + CCER, CC2E).unwrap();
        m.bus.write_u32(TIM2_BASE + CTLR1, CEN).unwrap();
        let edges = steady_edges(&mut m, 2_000, 5_000);
        assert!(
            edges.windows(2).all(|p| p[0].value == p[1].value),
            "PA1 must not toggle: {edges:?}"
        );
        assert_eq!(
            m.bus.peripherals[gpioa]
                .dev
                .gpio_routing(1)
                .and_then(|r| r.func),
            None
        );
    }
}
