// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! End-to-end waveform test for the RP2040 I²C controller: select the I²C
//! function on a pad through IO_BANK0 exactly as `gpio_set_function` does, arm
//! the in-engine logic analyzer on it, run a transfer through `IC_DATA_CMD`, and
//! assert the captured edges decode back to the bytes the transfer carried.
//!
//! This chip had nothing at all before: `IO_BANK0` was a register map that no
//! chip config wired up, so `GPIOn_CTRL.FUNCSEL` was not even in the address
//! map and nothing could know which pads carried a bus. Every assertion below
//! failed on an empty trace.

#[cfg(test)]
mod rp2040_i2c_waveform_tests {
    use crate::logic_capture::LogicEdge;
    use crate::peripherals::i2c::I2cDevice;
    use crate::peripherals::rp2040::i2c::Rp2040I2c;
    use crate::peripherals::rp2040::io_bank0::{Rp2040IoBank0, GPIO_FUNC_I2C};
    use crate::peripherals::rp2040::sio::Rp2040Sio;
    use crate::{Bus, Machine};

    const IO_BANK0_BASE: u64 = 0x4001_4000;
    const SIO_BASE: u64 = 0xD000_0000;
    const I2C0_BASE: u64 = 0x4004_4000;
    const RAM_BASE: u64 = 0x2000_0000;

    const IC_CON: u64 = 0x00;
    const IC_TAR: u64 = 0x04;
    const IC_DATA_CMD: u64 = 0x10;
    const IC_SS_SCL_HCNT: u64 = 0x14;
    const IC_SS_SCL_LCNT: u64 = 0x18;
    const IC_ENABLE: u64 = 0x6c;
    const DATA_CMD_STOP: u32 = 1 << 9;
    const ENABLE_ENABLE: u32 = 1;

    /// Pico default I²C pins: GP4 = I2C0 SDA, GP5 = I2C0 SCL.
    const SDA_PIN: u8 = 4;
    const SCL_PIN: u8 = 5;
    const CH_SCL: u32 = 0;
    const CH_SDA: u32 = 1;

    const SLAVE_ADDR: u8 = 0x3C;

    struct Sink;
    impl I2cDevice for Sink {
        fn address(&self) -> u8 {
            SLAVE_ADDR
        }
        fn read(&mut self) -> u8 {
            0
        }
        fn write(&mut self, _data: u8) {}
    }

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
        bus.add_peripheral("i2c0", I2C0_BASE, 0x1000, None, Box::new(Rp2040I2c::new()));
        bus.attach_i2c_slave("i2c0", Box::new(Sink)).unwrap();
        bus.wire_rp2040_i2c_pads();

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

    /// `gpio_set_function(pin, GPIO_FUNC_I2C)` plus `i2c_set_baudrate`.
    fn configure(machine: &mut Machine<crate::cpu::CortexM>) {
        let bus = &mut machine.bus;
        bus.write_u32(IO_BANK0_BASE + ctrl_offset(SDA_PIN), GPIO_FUNC_I2C)
            .unwrap();
        bus.write_u32(IO_BANK0_BASE + ctrl_offset(SCL_PIN), GPIO_FUNC_I2C)
            .unwrap();
        // 100 kHz from a 125 MHz clk_sys: 1250 clk_sys periods per SCL bit.
        bus.write_u32(I2C0_BASE + IC_SS_SCL_HCNT, 625).unwrap();
        bus.write_u32(I2C0_BASE + IC_SS_SCL_LCNT, 625).unwrap();
        bus.write_u32(I2C0_BASE + IC_CON, 0).unwrap(); // standard speed
        bus.write_u32(I2C0_BASE + IC_TAR, u32::from(SLAVE_ADDR))
            .unwrap();
        bus.write_u32(I2C0_BASE + IC_ENABLE, ENABLE_ENABLE).unwrap();
    }

    fn write_transfer(machine: &mut Machine<crate::cpu::CortexM>, payload: &[u8]) {
        for (i, &byte) in payload.iter().enumerate() {
            let last = i + 1 == payload.len();
            let cmd = u32::from(byte) | if last { DATA_CMD_STOP } else { 0 };
            machine.bus.write_u32(I2C0_BASE + IC_DATA_CMD, cmd).unwrap();
        }
        // Pad pushes reach the ring at an observation boundary.
        machine.step().unwrap();
    }

    /// An INDEPENDENT I²C decoder, identical rules to the STM32 and S3 gates.
    fn decode(edges: &[LogicEdge]) -> Vec<(u8, bool)> {
        let (mut scl, mut sda) = (true, true);
        let mut started = false;
        let mut bits: Vec<bool> = Vec::new();
        let mut frames = Vec::new();
        for edge in edges {
            let (prev_scl, prev_sda) = (scl, sda);
            match edge.ch {
                CH_SCL => scl = edge.value,
                CH_SDA => sda = edge.value,
                _ => continue,
            }
            if edge.ch == CH_SDA && prev_sda && !sda && scl {
                started = true;
                bits.clear();
                continue;
            }
            if edge.ch == CH_SDA && !prev_sda && sda && scl {
                started = false;
                bits.clear();
                continue;
            }
            if started && edge.ch == CH_SCL && !prev_scl && scl {
                bits.push(sda);
                if bits.len() == 9 {
                    let byte = bits[..8]
                        .iter()
                        .fold(0u8, |acc, &bit| (acc << 1) | u8::from(bit));
                    frames.push((byte, !bits[8]));
                    bits.clear();
                }
            }
        }
        frames
    }

    #[test]
    fn logic_capture_sees_a_decodable_rp2040_i2c_waveform() {
        let mut machine = machine();
        configure(&mut machine);

        let sio_idx = machine.bus.find_peripheral_index_by_name("sio").unwrap();
        let initial = machine.logic_watch(&[Some((sio_idx, SCL_PIN)), Some((sio_idx, SDA_PIN))]);
        assert_eq!(
            initial,
            vec![Some(true), Some(true)],
            "an idle open-drain I²C bus rests high on both routed pads",
        );

        // ~26 bit-times at 1250 cycles each needs history to occupy.
        for _ in 0..40_000 {
            machine.step().unwrap();
        }
        write_transfer(&mut machine, &[0x00, 0xAF]);

        let edges = machine.logic_read_edges(0).edges;
        assert!(
            !edges.is_empty(),
            "an RP2040 I²C transfer must put edges on its pads, not a flat trace",
        );
        assert_eq!(
            decode(&edges),
            vec![(SLAVE_ADDR << 1, true), (0x00, true), (0xAF, true)],
            "the wire must carry the addressed write the model performed",
        );
    }

    #[test]
    fn a_pad_left_as_plain_gpio_shows_no_bus_traffic() {
        // FUNCSEL is the whole point: a pad the firmware never handed to I²C
        // must keep reporting its own state. Selecting only SCL leaves the SDA
        // pad a plain GPIO, and it must stay silent.
        let mut machine = machine();
        let bus = &mut machine.bus;
        bus.write_u32(IO_BANK0_BASE + ctrl_offset(SCL_PIN), GPIO_FUNC_I2C)
            .unwrap();
        bus.write_u32(I2C0_BASE + IC_SS_SCL_HCNT, 625).unwrap();
        bus.write_u32(I2C0_BASE + IC_SS_SCL_LCNT, 625).unwrap();
        bus.write_u32(I2C0_BASE + IC_TAR, u32::from(SLAVE_ADDR))
            .unwrap();
        bus.write_u32(I2C0_BASE + IC_ENABLE, ENABLE_ENABLE).unwrap();

        let sio_idx = machine.bus.find_peripheral_index_by_name("sio").unwrap();
        machine.logic_watch(&[Some((sio_idx, SDA_PIN))]);
        for _ in 0..40_000 {
            machine.step().unwrap();
        }
        write_transfer(&mut machine, &[0xAF]);

        assert!(
            machine.logic_read_edges(0).edges.is_empty(),
            "a pad still owned by SIO must not show the I²C wire",
        );
    }

    #[test]
    fn scl_runs_at_the_rate_the_baudrate_registers_program() {
        let mut machine = machine();
        configure(&mut machine);
        let sio_idx = machine.bus.find_peripheral_index_by_name("sio").unwrap();
        machine.logic_watch(&[Some((sio_idx, SCL_PIN)), Some((sio_idx, SDA_PIN))]);
        for _ in 0..40_000 {
            machine.step().unwrap();
        }
        write_transfer(&mut machine, &[0x00]);

        // IC_SS_SCL_HCNT + IC_SS_SCL_LCNT = 1250 clk_sys periods per bit.
        const EXPECTED_BIT_TIME: u64 = 1250;
        let rises: Vec<u64> = machine
            .logic_read_edges(0)
            .edges
            .iter()
            .filter(|e| e.ch == CH_SCL && e.value)
            .map(|e| e.cycle)
            .collect();
        assert!(rises.len() >= 9, "at least one full frame of clocks");
        let gaps: Vec<u64> = rises.windows(2).map(|pair| pair[1] - pair[0]).collect();
        let at_rate = gaps.iter().filter(|&&g| g == EXPECTED_BIT_TIME).count();
        assert!(
            at_rate >= gaps.len() - 1,
            "SCL period should be {EXPECTED_BIT_TIME} cycles, got {gaps:?}",
        );
    }
}
