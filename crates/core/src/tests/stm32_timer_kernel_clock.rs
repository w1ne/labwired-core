// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! STM32 timers count TIMxCLK, not HCLK: the APB clock, doubled whenever the
//! APB prescaler is not 1 (RM0090 §7.2). Built from the real
//! `configs/chips/stm32f407.yaml`, because the STM32F4-Discovery's Arduino
//! clock tree (HCLK 168 MHz, APB1 /4, APB2 /2) is the case that put TIM4 at
//! twice its silicon rate: a 1 kHz `analogWrite` on PD12 came out at 2 kHz.

#[cfg(test)]
mod stm32_timer_kernel_clock_tests {
    use crate::bus::SystemBus;
    use crate::cpu::CortexM;
    use crate::{Bus, Machine};
    use labwired_config::{ChipDescriptor, SystemManifest};
    use std::path::PathBuf;

    const RAM_BASE: u64 = 0x2000_0000;
    /// RM0090 §7.3.3 RCC_CFGR: PPRE1 bits 12:10, PPRE2 bits 15:13.
    const RCC_CFGR: u64 = 0x4002_3808;
    /// RM0090 §7.3.14 RCC_APB2ENR.TIM1EN: the chip yaml gates TIM1 on it.
    const RCC_APB2ENR: u64 = 0x4002_3844;
    const PPRE_DIV2: u32 = 0b100;
    const PPRE_DIV4: u32 = 0b101;
    const PPRE_DIV16: u32 = 0b111;
    /// TIM4 on APB1, TIM1 on APB2 (RM0090 Table 1).
    const TIM4_BASE: u64 = 0x4000_0800;
    const TIM1_BASE: u64 = 0x4001_0000;
    const CR1: u64 = 0x00;
    const CNT: u64 = 0x24;
    const ARR: u64 = 0x2C;

    fn repo_root(rel: &str) -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .join(rel)
    }

    fn manifest(path: &str) -> SystemManifest {
        SystemManifest {
            max_peripheral_tick_interval: None,
            parts: Vec::new(),
            walk_deleted: Some(false),
            schema_version: "1.0".to_string(),
            name: "stm32-timer-kernel-clock".to_string(),
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

    /// A config-built F407 running a NOP slab from RAM, so steps advance
    /// cycles and nothing else touches a register.
    fn f407_machine() -> Machine<CortexM> {
        chip_machine("configs/chips/stm32f407.yaml")
    }

    fn chip_machine(rel: &str) -> Machine<CortexM> {
        let path = repo_root(rel);
        let chip = ChipDescriptor::from_file(&path).expect("load chip yaml");
        let abs = path.to_string_lossy().to_string();
        let mut bus = SystemBus::from_config(&chip, &manifest(&abs)).expect("assemble chip");
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

    fn set_ppre(machine: &mut Machine<CortexM>, ppre1: u32, ppre2: u32) {
        let cfgr = machine.bus.read_u32(RCC_CFGR).unwrap() & !(0x3F << 10);
        machine
            .bus
            .write_u32(RCC_CFGR, cfgr | (ppre1 << 10) | (ppre2 << 13))
            .unwrap();
    }

    fn start_free_running(machine: &mut Machine<CortexM>, base: u64) {
        machine.bus.write_u32(base + ARR, 0xFFFF).unwrap();
        machine.bus.write_u32(base + CNT, 0).unwrap();
        machine.bus.write_u32(base + CR1, 1).unwrap();
    }

    /// How far `base`'s counter moves while the core runs `cycles` cycles.
    fn counts_in(machine: &mut Machine<CortexM>, base: u64, cycles: u64) -> u64 {
        let before = machine.bus.read_u32(base + CNT).unwrap();
        let start = machine.total_cycles;
        while machine.total_cycles < start + cycles {
            machine.step().unwrap();
        }
        let after = machine.bus.read_u32(base + CNT).unwrap();
        let elapsed = machine.total_cycles - start;
        let moved = u64::from(after.wrapping_sub(before) & 0xFFFF);
        // Scale to exactly `cycles`: a step can overshoot by a few cycles.
        moved * cycles / elapsed
    }

    fn assert_near(got: u64, want: u64, what: &str) {
        assert!(
            got.abs_diff(want) <= want / 100 + 2,
            "{what}: {got} counts, want ~{want}"
        );
    }

    #[test]
    fn disco_clock_tree_runs_apb1_timers_at_half_hclk() {
        let mut m = f407_machine();
        // The stm32duino Disco F407 SystemClock_Config: AHB /1, APB1 /4,
        // APB2 /2. TIM4 counts 2 x 42 MHz = 84 MHz, TIM1 2 x 84 = 168 MHz.
        set_ppre(&mut m, PPRE_DIV4, PPRE_DIV2);
        m.bus.write_u32(RCC_APB2ENR, 1).unwrap();
        start_free_running(&mut m, TIM4_BASE);
        start_free_running(&mut m, TIM1_BASE);
        assert_near(
            counts_in(&mut m, TIM4_BASE, 20_000),
            10_000,
            "TIM4 at APB1 /4",
        );
        assert_near(
            counts_in(&mut m, TIM1_BASE, 20_000),
            20_000,
            "TIM1 at APB2 /2",
        );
    }

    #[test]
    fn reset_clock_tree_counts_every_hclk_cycle() {
        // RCC_CFGR resets to 0: both APB at /1, so TIMxCLK = HCLK.
        let mut m = f407_machine();
        start_free_running(&mut m, TIM4_BASE);
        assert_near(
            counts_in(&mut m, TIM4_BASE, 20_000),
            20_000,
            "TIM4 at reset",
        );
    }

    #[test]
    fn prescaler_change_mid_run_retimes_a_running_timer() {
        let mut m = f407_machine();
        start_free_running(&mut m, TIM4_BASE);
        assert_near(counts_in(&mut m, TIM4_BASE, 16_000), 16_000, "APB1 /1");
        set_ppre(&mut m, PPRE_DIV16, 0);
        assert_near(counts_in(&mut m, TIM4_BASE, 16_000), 2_000, "APB1 /16");
        set_ppre(&mut m, PPRE_DIV2, 0);
        assert_near(counts_in(&mut m, TIM4_BASE, 16_000), 16_000, "APB1 /2");
    }

    // STM32H723 (RM0468). The engine counts CPU cycles, and on the H7 the
    // CPU clock is SYSCLK / D1CPRE while the bus matrix runs on HCLK, a
    // further / HPRE.
    const H7_RCC: u64 = 0x5802_4400;
    const H7_CFGR: u64 = H7_RCC + 0x10;
    const H7_D1CFGR: u64 = H7_RCC + 0x18;
    const H7_D2CFGR: u64 = H7_RCC + 0x1C;
    const H7_APB1LENR: u64 = H7_RCC + 0xE8;
    const H7_APB2ENR: u64 = H7_RCC + 0xF0;
    /// TIM2 on APB1 (D2), TIM1 on APB2 (D2).
    const H7_TIM2_BASE: u64 = 0x4000_0000;
    const H7_TIM1_BASE: u64 = 0x4001_0000;
    /// HPRE / D1CPRE encoding 1000 = /2; D2PPRE 100 = /2, 101 = /4.
    const H7_AHB_DIV2: u32 = 0b1000;

    fn h723_machine(d1cfgr: u32, d2cfgr: u32, timpre: bool) -> Machine<CortexM> {
        let mut m = chip_machine("configs/chips/stm32h723zg.yaml");
        m.bus.write_u32(H7_APB1LENR, 1).unwrap(); // TIM2EN
        m.bus.write_u32(H7_APB2ENR, 1).unwrap(); // TIM1EN
        m.bus.write_u32(H7_D1CFGR, d1cfgr).unwrap();
        m.bus.write_u32(H7_D2CFGR, d2cfgr).unwrap();
        if timpre {
            let cfgr = m.bus.read_u32(H7_CFGR).unwrap();
            m.bus.write_u32(H7_CFGR, cfgr | (1 << 15)).unwrap();
        }
        start_free_running(&mut m, H7_TIM2_BASE);
        start_free_running(&mut m, H7_TIM1_BASE);
        m
    }

    #[test]
    fn h723_arduino_clock_tree_runs_timers_at_half_the_cpu_clock() {
        // stm32duino NUCLEO_H723ZG SystemClock_Config: CPU 550 MHz
        // (D1CPRE /1), HCLK 275 MHz (HPRE /2), D2PPRE1 = D2PPRE2 = /2. TIMPRE
        // is 0, so TIMxCLK = 2 x PCLK = HCLK: one count per two CPU cycles.
        // Before the H7 path TIM2 counted every CPU cycle and a 1 kHz
        // analogWrite on PA0 came out at 2 kHz.
        let mut m = h723_machine(H7_AHB_DIV2, (0b100 << 4) | (0b100 << 8), false);
        assert_near(counts_in(&mut m, H7_TIM2_BASE, 20_000), 10_000, "TIM2");
        assert_near(counts_in(&mut m, H7_TIM1_BASE, 20_000), 10_000, "TIM1");
    }

    #[test]
    fn h723_timpre_quadruples_a_slow_apb() {
        // HCLK = CPU / 2, D2PPRE1 /4. TIMPRE = 0: TIMxCLK = 2 x PCLK1 =
        // HCLK / 2, one count per four CPU cycles. TIMPRE = 1: TIMxCLK =
        // HCLK while D2PPRE1 is /4 or less (RM0468 §8.5.4).
        let mut m = h723_machine(H7_AHB_DIV2, 0b101 << 4, false);
        assert_near(counts_in(&mut m, H7_TIM2_BASE, 20_000), 5_000, "TIMPRE 0");
        let mut m = h723_machine(H7_AHB_DIV2, 0b101 << 4, true);
        assert_near(counts_in(&mut m, H7_TIM2_BASE, 20_000), 10_000, "TIMPRE 1");
    }

    #[test]
    fn h723_core_prescaler_slows_timers_in_cpu_cycles() {
        // D1CPRE /2 alone: the CPU and HCLK both run at SYSCLK / 2, so the
        // timer still counts once per CPU cycle.
        let mut m = h723_machine(H7_AHB_DIV2 << 8, 0, false);
        assert_near(counts_in(&mut m, H7_TIM2_BASE, 20_000), 20_000, "D1CPRE /2");
    }
}
