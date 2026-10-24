// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! STM32F103 timer channel outputs on their pads, RCC clock gates, and the
//! internal temperature sensor — every gate built from the COMMITTED
//! `configs/chips/stm32f103.yaml` through `SystemBus::from_config`, so it
//! measures what ships, not a hand-assembled bus.
//!
//! # What was missing
//!
//! A TIM3 channel 1 configured exactly as STM32 HAL / Arduino `analogWrite`
//! does (CCMR1 = PWM mode 1, CCER.CC1E, CCR1 = 300, ARR = 999, PA6 CNF/MODE
//! = alternate-function push-pull) left PA6 flat: the timer counted and
//! latched CC1IF, but no channel output ever reached a pad. RM0008 Rev 21
//! §15.3.9 ("PWM mode") and §9.1.11 Table 20 ("TIMx_CHx output compare:
//! alternate function push-pull") say the pad carries OC1.
//!
//! The waveform is asserted on the edges the logic analyzer records — engine
//! cycle stamps — and decoded here with no model code: period is
//! rising-to-rising, high time is rising-to-falling. With the counter clocked
//! at the core clock (`PSC + 1` cycles per count), RM0008 §15.3.9 gives
//! period `(ARR + 1) · (PSC + 1)` and, in PWM mode 1, high time
//! `CCRx · (PSC + 1)` (`CCxP = 1` inverts it).

#[cfg(test)]
mod stm32f1_timer_outputs_tests {
    use crate::bus::SystemBus;
    use crate::cpu::CortexM;
    use crate::logic_capture::{LogicEdge, LogicSource};
    use crate::{Bus, Machine};
    use labwired_config::{ChipDescriptor, SystemManifest};
    use std::path::PathBuf;

    const RAM_BASE: u64 = 0x2000_0000;
    const RCC_BASE: u64 = 0x4002_1000;
    /// RM0008 §7.3.7 RCC_APB2ENR / §7.3.8 RCC_APB1ENR.
    const APB2ENR: u64 = RCC_BASE + 0x18;
    const APB1ENR: u64 = RCC_BASE + 0x1C;
    const IOPAEN: u32 = 1 << 2;
    const IOPBEN: u32 = 1 << 3;
    const IOPCEN: u32 = 1 << 4;
    const ADC1EN: u32 = 1 << 9;
    const TIM1EN: u32 = 1 << 11;
    const SPI1EN: u32 = 1 << 12;
    const TIM2EN: u32 = 1 << 0;
    const TIM3EN: u32 = 1 << 1;
    const TIM4EN: u32 = 1 << 2;

    const GPIOA_BASE: u64 = 0x4001_0800;
    const GPIOB_BASE: u64 = 0x4001_0C00;
    const GPIOC_BASE: u64 = 0x4001_1000;
    const TIM1_BASE: u64 = 0x4001_2C00;
    const TIM2_BASE: u64 = 0x4000_0000;
    const TIM3_BASE: u64 = 0x4000_0400;
    const TIM4_BASE: u64 = 0x4000_0800;
    const ADC1_BASE: u64 = 0x4001_2400;

    // TIMx register offsets (RM0008 §15.4).
    const CR1: u64 = 0x00;
    const CCMR1: u64 = 0x18;
    const CCER: u64 = 0x20;
    const PSC: u64 = 0x28;
    const ARR: u64 = 0x2C;
    const CCR1: u64 = 0x34;
    const CCR2: u64 = 0x38;
    const BDTR: u64 = 0x44;
    const CR1_CEN: u32 = 1;
    /// OC1M = 110 (PWM mode 1) | OC1PE, the HAL's `TIM_OCMODE_PWM1` setup.
    const CCMR1_OC1_PWM1: u32 = (0b110 << 4) | (1 << 3);
    /// OC2M = 110 (PWM mode 1) | OC2PE.
    const CCMR1_OC2_PWM1: u32 = ((0b110 << 4) | (1 << 3)) << 8;
    const CCMR1_OC1_FORCE_ACTIVE: u32 = 0b101 << 4;
    const CCMR1_OC1_FORCE_INACTIVE: u32 = 0b100 << 4;
    const CC1E: u32 = 1 << 0;
    const CC2E: u32 = 1 << 4;
    const CC2P: u32 = 1 << 5;
    const BDTR_MOE: u32 = 1 << 15;

    /// CRL/CRH nibble: MODE = 11 (50 MHz) + CNF = 10, alternate-function
    /// push-pull — RM0008 §9.1.11 Table 20 for a timer output.
    const AF_PUSH_PULL: u32 = 0b1011;
    /// MODE = 01 + CNF = 00: general-purpose push-pull output.
    const GPIO_PUSH_PULL: u32 = 0b0001;

    fn repo_root(rel: &str) -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .join(rel)
    }

    fn manifest(path: &str) -> SystemManifest {
        SystemManifest {
            parts: Vec::new(),
            walk_deleted: Some(false),
            schema_version: "1.0".to_string(),
            name: "stm32f1-timer-outputs".to_string(),
            chip: path.to_string(),
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

    fn f103_bus() -> SystemBus {
        let path = repo_root("configs/chips/stm32f103.yaml");
        let chip = ChipDescriptor::from_file(&path).expect("load stm32f103.yaml");
        let abs = path.to_string_lossy().to_string();
        SystemBus::from_config(&chip, &manifest(&abs)).expect("assemble stm32f103")
    }

    /// A config-built F103 running a NOP slab (`movs r0, #0` … `b` back) from
    /// RAM, so steps advance cycles and nothing else touches a register.
    fn f103_machine() -> Machine<CortexM> {
        let mut bus = f103_bus();
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

    fn set_pin_config(machine: &mut Machine<CortexM>, port_base: u64, pin: u8, config: u32) {
        let reg = port_base + if pin < 8 { 0x00 } else { 0x04 };
        let shift = ((pin % 8) * 4) as u32;
        let old = machine.bus.read_u32(reg).unwrap();
        machine
            .bus
            .write_u32(reg, (old & !(0xF << shift)) | (config << shift))
            .unwrap();
    }

    fn port(machine: &Machine<CortexM>, name: &str) -> usize {
        machine
            .bus
            .find_peripheral_index_by_name(name)
            .unwrap_or_else(|| panic!("{name} registered by from_config"))
    }

    fn run_cycles(machine: &mut Machine<CortexM>, cycles: u64) {
        let target = machine.total_cycles + cycles;
        while machine.total_cycles < target {
            machine.step().unwrap();
        }
    }

    /// The pad's waveform as (period, high time) in cycles, decoded from the
    /// recorded edges alone. Requires at least two whole periods and checks
    /// every one of them, so a single lucky period cannot pass.
    fn period_and_high(edges: &[LogicEdge]) -> (u64, u64) {
        // Transitions only: a report that repeats the level is not an edge.
        let mut last: Option<bool> = None;
        let edges: Vec<LogicEdge> = edges
            .iter()
            .filter(|e| {
                let keep = last != Some(e.value);
                last = Some(e.value);
                keep
            })
            .copied()
            .collect();
        let edges = edges.as_slice();
        let rises: Vec<u64> = edges.iter().filter(|e| e.value).map(|e| e.cycle).collect();
        let falls: Vec<u64> = edges.iter().filter(|e| !e.value).map(|e| e.cycle).collect();
        assert!(
            rises.len() >= 3,
            "need at least three rising edges to measure two periods, got {edges:?}"
        );
        let period = rises[1] - rises[0];
        for pair in rises.windows(2) {
            assert_eq!(pair[1] - pair[0], period, "periods differ: {rises:?}");
        }
        let mut highs = Vec::new();
        for &r in &rises[..rises.len() - 1] {
            let f = falls
                .iter()
                .copied()
                .find(|&f| f > r)
                .unwrap_or_else(|| panic!("no falling edge after {r}: {edges:?}"));
            highs.push(f - r);
        }
        assert!(
            highs.iter().all(|&h| h == highs[0]),
            "high times differ: {highs:?}"
        );
        (period, highs[0])
    }

    /// Run `warmup` cycles, then record `cycles` more and return only the
    /// edges of that second window (the turn-on transient is not a period).
    fn steady_edges(m: &mut Machine<CortexM>, warmup: u64, cycles: u64) -> Vec<LogicEdge> {
        run_cycles(m, warmup);
        let cursor = m.logic_read_edges(0).cursor;
        run_cycles(m, cycles);
        m.logic_read_edges(cursor).edges
    }

    /// The audit's own configuration: TIM3 CH1 on PA6, PWM mode 1,
    /// CCR1 = 300, ARR = 999, PSC = 0 — period 1000 cycles, high 300.
    #[test]
    fn tim3_ch1_pwm_mode1_drives_pa6_at_the_programmed_period_and_duty() {
        let mut m = f103_machine();
        m.bus.write_u32(APB2ENR, IOPAEN).unwrap();
        m.bus.write_u32(APB1ENR, TIM3EN).unwrap();
        set_pin_config(&mut m, GPIOA_BASE, 6, AF_PUSH_PULL);
        let gpioa = port(&m, "gpioa");
        m.logic_watch(&[Some(LogicSource::pad(gpioa, 6))]);

        m.bus.write_u32(TIM3_BASE + PSC, 0).unwrap();
        m.bus.write_u32(TIM3_BASE + ARR, 999).unwrap();
        m.bus.write_u32(TIM3_BASE + CCR1, 300).unwrap();
        m.bus.write_u32(TIM3_BASE + CCMR1, CCMR1_OC1_PWM1).unwrap();
        m.bus.write_u32(TIM3_BASE + CCER, CC1E).unwrap();
        m.bus.write_u32(TIM3_BASE + CR1, CR1_CEN).unwrap();
        let edges = steady_edges(&mut m, 1_500, 5_000);
        assert_eq!(
            period_and_high(&edges),
            (1000, 300),
            "RM0008 §15.3.9: period (ARR+1)(PSC+1) = 1000, PWM1 high CCR1 = 300"
        );
        let routed = m.bus.peripherals[gpioa]
            .dev
            .gpio_routing(6)
            .and_then(|r| r.func);
        assert_eq!(
            routed.as_deref(),
            Some("TIM3_CH1"),
            "DS5319 Table 5: PA6 Default = TIM3_CH1"
        );
    }

    /// A TIM2 pin, with the prescaler and an active-low channel: TIM2 CH2 on
    /// PA1, PSC = 1, ARR = 499, CCR2 = 100, CC2P = 1. Period (499+1)(1+1) =
    /// 1000 cycles; OC2REF is active for 100 counts = 200 cycles, and active
    /// LOW, so the pad is high for the other 800.
    #[test]
    fn tim2_ch2_on_pa1_honours_the_prescaler_and_ccxp_polarity() {
        let mut m = f103_machine();
        m.bus.write_u32(APB2ENR, IOPAEN).unwrap();
        m.bus.write_u32(APB1ENR, TIM2EN).unwrap();
        set_pin_config(&mut m, GPIOA_BASE, 1, AF_PUSH_PULL);
        let gpioa = port(&m, "gpioa");
        m.logic_watch(&[Some(LogicSource::pad(gpioa, 1))]);

        m.bus.write_u32(TIM2_BASE + PSC, 1).unwrap();
        m.bus.write_u32(TIM2_BASE + ARR, 499).unwrap();
        m.bus.write_u32(TIM2_BASE + CCR2, 100).unwrap();
        m.bus.write_u32(TIM2_BASE + CCMR1, CCMR1_OC2_PWM1).unwrap();
        m.bus.write_u32(TIM2_BASE + CCER, CC2E | CC2P).unwrap();
        m.bus.write_u32(TIM2_BASE + CR1, CR1_CEN).unwrap();
        let edges = steady_edges(&mut m, 1_500, 5_000);
        assert_eq!(period_and_high(&edges), (1000, 800));
    }

    /// CCxE = 0 is "output off": an AF pad then carries no timer wire at all
    /// (`logic_watch` answers `None` for an AF pad nothing drives). Enabling
    /// the channel hands the pad to the timer; forced-active / forced-inactive
    /// (OCxM = 101 / 100) put a constant level on it.
    #[test]
    fn ccxe_gates_the_pad_and_forced_modes_hold_a_level() {
        let mut m = f103_machine();
        m.bus.write_u32(APB2ENR, IOPAEN).unwrap();
        m.bus.write_u32(APB1ENR, TIM3EN).unwrap();
        set_pin_config(&mut m, GPIOA_BASE, 6, AF_PUSH_PULL);
        let gpioa = port(&m, "gpioa");
        let pa6 = [Some(LogicSource::pad(gpioa, 6))];

        m.bus
            .write_u32(TIM3_BASE + CCMR1, CCMR1_OC1_FORCE_ACTIVE)
            .unwrap();
        assert_eq!(m.logic_watch(&pa6), vec![None], "CC1E = 0: not driven");

        m.bus.write_u32(TIM3_BASE + CCER, CC1E).unwrap();
        assert_eq!(
            m.logic_watch(&pa6),
            vec![Some(true)],
            "OC1M = 101: forced active"
        );
        m.bus
            .write_u32(TIM3_BASE + CCMR1, CCMR1_OC1_FORCE_INACTIVE)
            .unwrap();
        assert_eq!(
            m.logic_watch(&pa6),
            vec![Some(false)],
            "OC1M = 100: forced inactive"
        );

        // A plain GPIO output on the same pad reads its own latch, timer or not.
        m.bus
            .write_u32(TIM3_BASE + CCMR1, CCMR1_OC1_FORCE_ACTIVE)
            .unwrap();
        set_pin_config(&mut m, GPIOA_BASE, 6, GPIO_PUSH_PULL);
        assert_eq!(
            m.logic_watch(&pa6),
            vec![Some(false)],
            "CNF = 00 reads ODR, not OC1"
        );
    }

    /// PA7 is `SPI1_MOSI/TIM3_CH2` in the DS5319 Default column. With TIM3 CH2
    /// off the pad stays SPI1's; with it enabled the timer drives it.
    #[test]
    fn a_shared_default_pad_follows_the_enabled_peripheral() {
        let mut m = f103_machine();
        m.bus.write_u32(APB2ENR, IOPAEN | SPI1EN).unwrap();
        m.bus.write_u32(APB1ENR, TIM3EN).unwrap();
        set_pin_config(&mut m, GPIOA_BASE, 7, AF_PUSH_PULL);
        let gpioa = port(&m, "gpioa");
        let func = |m: &Machine<CortexM>| {
            m.bus.peripherals[gpioa]
                .dev
                .gpio_routing(7)
                .and_then(|r| r.func)
        };
        assert_eq!(func(&m).as_deref(), Some("SPI1_MOSI"));
        m.bus
            .write_u32(TIM3_BASE + CCMR1, CCMR1_OC1_FORCE_ACTIVE << 8)
            .unwrap();
        m.bus.write_u32(TIM3_BASE + CCER, CC2E).unwrap();
        assert_eq!(func(&m).as_deref(), Some("TIM3_CH2"));
        m.bus.write_u32(TIM3_BASE + CCER, 0).unwrap();
        assert_eq!(func(&m).as_deref(), Some("SPI1_MOSI"));
    }

    /// TIM1 is advanced-control: its outputs stay off until `BDTR.MOE` is set
    /// (RM0008 §14.4.18), whatever CCER says.
    #[test]
    fn tim1_outputs_wait_for_moe() {
        let mut m = f103_machine();
        m.bus.write_u32(APB2ENR, IOPAEN | TIM1EN).unwrap();
        set_pin_config(&mut m, GPIOA_BASE, 8, AF_PUSH_PULL);
        let gpioa = port(&m, "gpioa");
        m.logic_watch(&[Some(LogicSource::pad(gpioa, 8))]);

        m.bus.write_u32(TIM1_BASE + ARR, 499).unwrap();
        m.bus.write_u32(TIM1_BASE + CCR1, 125).unwrap();
        m.bus.write_u32(TIM1_BASE + CCMR1, CCMR1_OC1_PWM1).unwrap();
        m.bus.write_u32(TIM1_BASE + CCER, CC1E).unwrap();
        m.bus.write_u32(TIM1_BASE + CR1, CR1_CEN).unwrap();
        run_cycles(&mut m, 2_000);
        assert!(
            m.logic_read_edges(0).edges.is_empty(),
            "MOE = 0: TIM1 CH1 must not reach PA8"
        );

        m.bus.write_u32(TIM1_BASE + BDTR, BDTR_MOE).unwrap();
        let edges = steady_edges(&mut m, 1_000, 3_000);
        assert_eq!(period_and_high(&edges), (500, 125));
    }

    /// RM0008 §7.3.7 / §7.3.8: GPIOB, GPIOC, TIM3 and TIM4 are unclocked out
    /// of reset, so their register writes are dropped until the RCC enable bit
    /// is set — then the same write lands.
    #[test]
    fn rcc_gates_drop_gpiob_gpioc_tim3_tim4_writes_until_enabled() {
        let mut bus = f103_bus();
        let cases: [(&str, u64, u64, u32, u32); 4] = [
            ("GPIOB.ODR", GPIOB_BASE + 0x0C, APB2ENR, IOPBEN, 0x0000_A5A5),
            ("GPIOC.ODR", GPIOC_BASE + 0x0C, APB2ENR, IOPCEN, 0x0000_2000),
            ("TIM3.ARR", TIM3_BASE + ARR, APB1ENR, TIM3EN, 0x1234),
            ("TIM4.ARR", TIM4_BASE + ARR, APB1ENR, TIM4EN, 0x4321),
        ];
        for (label, addr, enr, bit, value) in cases {
            bus.write_u32(addr, value).unwrap();
            assert_ne!(
                bus.read_u32(addr).unwrap(),
                value,
                "{label}: a write with the clock off must not land"
            );
            let on = bus.read_u32(enr).unwrap() | bit;
            bus.write_u32(enr, on).unwrap();
            bus.write_u32(addr, value).unwrap();
            assert_eq!(
                bus.read_u32(addr).unwrap(),
                value,
                "{label}: with the clock on the write lands"
            );
        }
    }

    /// ADC1_IN16 / IN17 (RM0008 §11.10, DS5319 Tables 50 and 13): with
    /// `CR2.TSVREFE` set, IN16 converts V_SENSE = 1.43 V − (T − 25 °C) ·
    /// 4.3 mV/°C and IN17 converts V_REFINT = 1.20 V, against 3.3 V; without
    /// it both read 0. The temperature comes from the bus-level ambient
    /// setter, default 25 °C.
    #[test]
    fn adc1_temperature_sensor_follows_the_ambient_temperature() {
        const CR2: u64 = 0x08;
        const SQR3: u64 = 0x34;
        const DR: u64 = 0x4C;
        const ADON: u32 = 1 << 0;
        const EXTSEL_SW: u32 = 0b111 << 17;
        const SWSTART: u32 = 1 << 22;
        const TSVREFE: u32 = 1 << 23;

        let convert = |m: &mut Machine<CortexM>, channel: u32, tsvrefe: bool| -> u32 {
            let base = ADON | EXTSEL_SW | if tsvrefe { TSVREFE } else { 0 };
            m.bus.write_u32(ADC1_BASE + SQR3, channel).unwrap();
            m.bus.write_u32(ADC1_BASE + CR2, base).unwrap();
            m.bus.write_u32(ADC1_BASE + CR2, base | SWSTART).unwrap();
            run_cycles(m, 200);
            m.bus.read_u32(ADC1_BASE + DR).unwrap()
        };
        // Truncating 12-bit code of `mv` against 3.3 V.
        let code = |mv: f64| (mv * 4095.0 / 3300.0).floor() as u32;

        let mut m = f103_machine();
        m.bus.write_u32(APB2ENR, ADC1EN).unwrap();

        assert_eq!(
            convert(&mut m, 16, true),
            code(1430.0),
            "25 °C default: V25"
        );
        assert_eq!(convert(&mut m, 17, true), code(1200.0), "V_REFINT");
        assert_eq!(convert(&mut m, 16, false), 0, "TSVREFE clear: disconnected");
        assert_eq!(convert(&mut m, 17, false), 0, "TSVREFE clear: disconnected");

        assert_eq!(
            m.bus.set_ambient_temperature_c(75.0),
            2,
            "adc1 and adc2 take it"
        );
        assert_eq!(
            convert(&mut m, 16, true),
            code(1430.0 - 50.0 * 4.3),
            "75 °C: V_SENSE falls by 50 × 4.3 mV"
        );
        m.bus.set_ambient_temperature_c(-15.0);
        assert_eq!(convert(&mut m, 16, true), code(1430.0 + 40.0 * 4.3));
        assert_eq!(
            m.bus.set_ambient_temperature_c(f64::NAN),
            0,
            "a non-finite temperature is refused"
        );
        assert_eq!(convert(&mut m, 16, true), code(1430.0 + 40.0 * 4.3));
    }
}
