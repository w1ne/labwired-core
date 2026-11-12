// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! RP2040 PWM outputs reach their pads. Select `FUNCSEL = 4` on a pad through
//! IO_BANK0 the way `gpio_set_function(pin, GPIO_FUNC_PWM)` does, program the
//! slice the way Arduino-Mbed's `analogWrite` does on the Pico (GP25 = slice 4
//! B, DIV 250.0, TOP 1000, CC 502), arm the logic analyzer on the pad and
//! measure what it captured.
//!
//! Before this, the PWM model counted and wrapped but drove no level, so GP25
//! read the SIO latch and the Arduino matrix's L6 cell passed on its UART
//! marker alone.

#[cfg(test)]
mod rp2040_pwm_pads_tests {
    use crate::logic_capture::LogicSource;
    use crate::peripherals::rp2040::io_bank0::{Rp2040IoBank0, GPIO_FUNC_PWM, GPIO_FUNC_SIO};
    use crate::peripherals::rp2040::pwm::Rp2040Pwm;
    use crate::peripherals::rp2040::sio::Rp2040Sio;
    use crate::{Bus, Machine};

    const IO_BANK0_BASE: u64 = 0x4001_4000;
    const SIO_BASE: u64 = 0xD000_0000;
    const PWM_BASE: u64 = 0x4005_0000;
    const RAM_BASE: u64 = 0x2000_0000;

    const SLICE_STRIDE: u64 = 0x14;
    const CSR: u64 = 0x00;
    const DIV: u64 = 0x04;
    const CC: u64 = 0x0C;
    const TOP: u64 = 0x10;
    const CSR_EN: u32 = 1 << 0;
    const CSR_B_INV: u32 = 1 << 3;

    /// Arduino-Mbed `analogWrite(LED_BUILTIN, 128)` on the Pico, read back
    /// from the stock L6 ELF: `period_ms(2)` at clk_sys 125 MHz.
    const DIV_250: u32 = 250 << 4;
    const TOP_1000: u32 = 1000;
    const CC_B_502: u32 = 502 << 16;
    const PERIOD: u64 = 1001 * 250;
    const HIGH: u64 = 502 * 250;

    fn ctrl_offset(pin: u8) -> u64 {
        u64::from(pin) * 8 + 4
    }

    fn machine() -> Machine<crate::cpu::CortexM> {
        let mut bus = crate::bus::SystemBus::new();
        let (cpu, _nvic) = crate::system::cortex_m::configure_cortex_m(&mut bus);
        bus.add_peripheral(
            "io_bank0",
            IO_BANK0_BASE,
            0x1000,
            None,
            Box::new(Rp2040IoBank0::new()),
        );
        bus.add_peripheral("sio", SIO_BASE, 0x1000, None, Box::new(Rp2040Sio::new()));
        bus.add_peripheral("pwm", PWM_BASE, 0x1000, None, Box::new(Rp2040Pwm::new()));
        bus.wire_rp2040_pwm_pads();

        let mut machine = Machine::new(cpu, bus);
        // A Thumb NOP field ending in a backward branch, so `step()` advances
        // the cycle axis without faulting.
        for i in 0..1022u64 {
            let byte = if i % 2 == 0 { 0x00 } else { 0x20 };
            machine.bus.write_u8(RAM_BASE + i, byte).unwrap();
        }
        machine.bus.write_u8(RAM_BASE + 1022, 0xFF).unwrap();
        machine.bus.write_u8(RAM_BASE + 1023, 0xE5).unwrap();
        machine.cpu.pc = RAM_BASE as u32;
        machine
    }

    fn start_slice4(machine: &mut Machine<crate::cpu::CortexM>, csr: u32) {
        let base = PWM_BASE + 4 * SLICE_STRIDE;
        let bus = &mut machine.bus;
        bus.write_u32(base + DIV, DIV_250).unwrap();
        bus.write_u32(base + TOP, TOP_1000).unwrap();
        bus.write_u32(base + CC, CC_B_502).unwrap();
        bus.write_u32(base + CSR, csr | CSR_EN).unwrap();
    }

    /// (rise-to-rise periods, high times) of channel 0's captured edges.
    fn measure(machine: &mut Machine<crate::cpu::CortexM>) -> (Vec<u64>, Vec<u64>) {
        let edges = machine.logic_read_edges(0).edges;
        let mut periods = Vec::new();
        let mut highs = Vec::new();
        let mut last_rise = None;
        for w in edges.windows(2) {
            if w[0].value {
                highs.push(w[1].cycle - w[0].cycle);
                if let Some(r) = last_rise {
                    periods.push(w[0].cycle - r);
                }
                last_rise = Some(w[0].cycle);
            }
        }
        (periods, highs)
    }

    fn run(machine: &mut Machine<crate::cpu::CortexM>, cycles: u64) {
        for _ in 0..cycles {
            machine.step().unwrap();
        }
    }

    fn watch(machine: &mut Machine<crate::cpu::CortexM>, pin: u8) {
        let sio = machine.bus.find_peripheral_index_by_name("sio").unwrap();
        machine.logic_watch(&[Some(LogicSource::pad(sio, pin))]);
    }

    #[test]
    fn gp25_on_funcsel_pwm_carries_slice4_b_at_the_programmed_period() {
        let mut m = machine();
        m.bus
            .write_u32(IO_BANK0_BASE + ctrl_offset(25), GPIO_FUNC_PWM)
            .unwrap();
        watch(&mut m, 25);
        start_slice4(&mut m, 0);
        run(&mut m, 4 * PERIOD + 10);
        let (periods, highs) = measure(&mut m);
        assert!(periods.len() >= 3, "GP25 must toggle: {periods:?}");
        assert!(
            periods.iter().all(|&p| p == PERIOD),
            "(TOP+1)*DIV = {PERIOD} cycles: {periods:?}"
        );
        assert!(
            highs[1..].iter().all(|&h| h == HIGH),
            "high while CTR < CC = {HIGH} cycles: {highs:?}"
        );
    }

    #[test]
    fn b_inv_inverts_the_pad() {
        let mut m = machine();
        m.bus
            .write_u32(IO_BANK0_BASE + ctrl_offset(25), GPIO_FUNC_PWM)
            .unwrap();
        watch(&mut m, 25);
        start_slice4(&mut m, CSR_B_INV);
        run(&mut m, 3 * PERIOD + 10);
        let (_, highs) = measure(&mut m);
        assert!(
            highs[1..].iter().all(|&h| h == PERIOD - HIGH),
            "B_INV: high for the low part: {highs:?}"
        );
    }

    #[test]
    fn the_route_is_live_only_on_funcsel_pwm_and_only_on_its_own_slice() {
        // GP25 left on SIO: the latch, not the PWM.
        let mut m = machine();
        m.bus
            .write_u32(IO_BANK0_BASE + ctrl_offset(25), GPIO_FUNC_SIO)
            .unwrap();
        watch(&mut m, 25);
        start_slice4(&mut m, 0);
        run(&mut m, 2 * PERIOD);
        assert!(
            m.logic_read_edges(0).edges.is_empty(),
            "SIO-owned GP25 stays flat"
        );

        // GP24 on PWM is slice 4 channel A, whose CC is 0: low all period.
        let mut m = machine();
        m.bus
            .write_u32(IO_BANK0_BASE + ctrl_offset(24), GPIO_FUNC_PWM)
            .unwrap();
        watch(&mut m, 24);
        start_slice4(&mut m, 0);
        run(&mut m, 2 * PERIOD);
        assert!(
            m.logic_read_edges(0).edges.iter().all(|e| !e.value),
            "slice 4 A has CC_A = 0"
        );
    }
}
