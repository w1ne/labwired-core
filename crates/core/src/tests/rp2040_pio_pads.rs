// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! RP2040 PIO pin outputs reach their pads. Load pico-examples' stock
//! `ws2812.pio` (T1 2, T2 5, T3 3, `.side_set 1`) and configure the state
//! machine the way `ws2812_program_init(pio, sm, offset, 16, 800000, false)`
//! does at clk_sys 125 MHz: side-set base GP16, OUT shift left with autopull
//! at 24 bits, TX FIFO joined, CLKDIV 15.625, `set pindirs, 1` through
//! `SM0_INSTR`, and `FUNCSEL = 6` on GP16. Push three GRB pixels, arm the
//! logic analyzer on the pad and decode what it captured.
//!
//! Before this, the PIO model had no pins at all (no side-set, SET/OUT PINS
//! or PINDIRS, no SM_INSTR/PINCTRL, no autopull, OUT always shifted right),
//! so a NeoPixel on a Pico drove nothing.

#[cfg(test)]
mod rp2040_pio_pads_tests {
    use crate::logic_capture::{LogicSource, PadDrive};
    use crate::peripherals::pio::Pio;
    use crate::peripherals::rp2040::io_bank0::{Rp2040IoBank0, GPIO_FUNC_PIO0, GPIO_FUNC_SIO};
    use crate::peripherals::rp2040::sio::Rp2040Sio;
    use crate::{Bus, Machine};

    const IO_BANK0_BASE: u64 = 0x4001_4000;
    const SIO_BASE: u64 = 0xD000_0000;
    const PIO0_BASE: u64 = 0x5020_0000;
    const RAM_BASE: u64 = 0x2000_0000;

    const CTRL: u64 = 0x000;
    const TXF0: u64 = 0x010;
    const INSTR_MEM0: u64 = 0x048;
    const SM0_CLKDIV: u64 = 0x0c8;
    const SM0_EXECCTRL: u64 = 0x0cc;
    const SM0_SHIFTCTRL: u64 = 0x0d0;
    const SM0_INSTR: u64 = 0x0d8;
    const SM0_PINCTRL: u64 = 0x0dc;

    const PIN: u8 = 16;

    /// `pioasm ws2812.pio` (pico-examples, T1 2 / T2 5 / T3 3):
    /// `out x, 1 side 0 [2]`, `jmp !x 3 side 1 [1]`, `jmp 0 side 1 [4]`,
    /// `nop side 0 [4]`; wrap 3 -> 0.
    const WS2812: [u16; 4] = [0x6221, 0x1123, 0x1400, 0xa442];
    /// One bit = T1 + T2 + T3 = 10 PIO cycles at CLKDIV 15.625.
    const BIT_CYCLES: f64 = 156.25;

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
        bus.add_peripheral("pio0", PIO0_BASE, 0x1000, None, Box::new(Pio::new()));
        bus.wire_rp2040_pio_pads();

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

    /// `ws2812_program_init(pio0, 0, 0, PIN, 800000, false)` register for
    /// register, then `pio_sm_set_enabled(pio0, 0, true)`.
    fn ws2812_init(machine: &mut Machine<crate::cpu::CortexM>, funcsel: u32) {
        let bus = &mut machine.bus;
        for (i, op) in WS2812.iter().enumerate() {
            bus.write_u32(PIO0_BASE + INSTR_MEM0 + 4 * i as u64, u32::from(*op))
                .unwrap();
        }
        // pio_gpio_init: the pad goes to PIO0 (or, for the control, SIO).
        bus.write_u32(IO_BANK0_BASE + ctrl_offset(PIN), funcsel)
            .unwrap();
        // pio_sm_set_consecutive_pindirs: SET_BASE = PIN, SET_COUNT = 1,
        // exec `set pindirs, 1`.
        bus.write_u32(PIO0_BASE + SM0_PINCTRL, (1 << 26) | (u32::from(PIN) << 5))
            .unwrap();
        bus.write_u32(PIO0_BASE + SM0_INSTR, 0xe081).unwrap();
        // pio_sm_init: SIDESET_COUNT 1, SIDESET_BASE PIN.
        bus.write_u32(PIO0_BASE + SM0_PINCTRL, (1 << 29) | (u32::from(PIN) << 10))
            .unwrap();
        bus.write_u32(PIO0_BASE + SM0_EXECCTRL, 3 << 12).unwrap();
        // OUT shift left, autopull, PULL_THRESH 24, FJOIN_TX (IN_SHIFTDIR
        // keeps its reset 1).
        bus.write_u32(
            PIO0_BASE + SM0_SHIFTCTRL,
            (1 << 30) | (24 << 25) | (1 << 18) | (1 << 17),
        )
        .unwrap();
        // 125 MHz / (800 kHz * 10) = 15.625 = 15 + 160/256.
        bus.write_u32(PIO0_BASE + SM0_CLKDIV, (15 << 16) | (160 << 8))
            .unwrap();
        bus.write_u32(PIO0_BASE + CTRL, 1).unwrap();
    }

    fn watch(machine: &mut Machine<crate::cpu::CortexM>, pin: u8) {
        let sio = machine.bus.find_peripheral_index_by_name("sio").unwrap();
        machine.logic_watch(&[Some(LogicSource::pad(sio, pin))]);
    }

    fn run(machine: &mut Machine<crate::cpu::CortexM>, cycles: u64) {
        for _ in 0..cycles {
            machine.step().unwrap();
        }
    }

    /// (high time, rise-to-rise) of every pulse channel 0 captured.
    fn pulses(machine: &mut Machine<crate::cpu::CortexM>) -> Vec<(u64, Option<u64>)> {
        let edges = machine.logic_read_edges(0).edges;
        let rises: Vec<usize> = (0..edges.len()).filter(|&i| edges[i].value).collect();
        rises
            .iter()
            .enumerate()
            .filter_map(|(k, &r)| {
                let fall = edges.get(r + 1).filter(|e| !e.value)?;
                let next = rises.get(k + 1).map(|&n| edges[n].cycle - edges[r].cycle);
                Some((fall.cycle - edges[r].cycle, next))
            })
            .collect()
    }

    #[test]
    fn ws2812_program_on_gp16_carries_three_grb_pixels() {
        let mut m = machine();
        watch(&mut m, PIN);
        ws2812_init(&mut m, GPIO_FUNC_PIO0);
        // put_pixel(): pixel_grb << 8, MSB first.
        let pixels = [0x00ff_00u32, 0x8001_7e, 0x0f_f0_a5];
        for p in pixels {
            m.bus.write_u32(PIO0_BASE + TXF0, p << 8).unwrap();
        }
        run(&mut m, (BIT_CYCLES * 24.0 * 3.0) as u64 + 2_000);

        let pulses = pulses(&mut m);
        assert_eq!(pulses.len(), 72, "3 pixels x 24 bits: {pulses:?}");
        // T0H = T1 = 2 PIO cycles (31.25), T1H = T1 + T2 = 7 (109.4).
        let mut decoded = [0u32; 3];
        for (bit, &(high, _)) in pulses.iter().enumerate() {
            let one = match high {
                31 | 32 => false,
                109 | 110 => true,
                h => panic!("bit {bit}: high {h} cycles is neither T0H nor T1H"),
            };
            decoded[bit / 24] = (decoded[bit / 24] << 1) | u32::from(one);
        }
        assert_eq!(decoded, pixels, "GRB words on the wire");
        // Inside a frame every bit is 10 PIO cycles; the 15.625 divider
        // lands each one on 156 or 157 sys cycles.
        for (bit, &(_, period)) in pulses.iter().enumerate().take(71) {
            let p = period.unwrap();
            assert!(p == 156 || p == 157, "bit {bit}: period {p}");
        }
        // The FIFO ran dry: `out x, 1 side 0` stalls with the pad low
        // (the WS2812 latch/reset level).
        let sio = m.bus.find_peripheral_index_by_name("sio").unwrap();
        assert_eq!(m.bus.peripherals[sio].dev.read_gpio_pad(PIN), Some(false));
    }

    #[test]
    fn a_pad_left_on_sio_does_not_carry_the_pio_wire() {
        let mut m = machine();
        watch(&mut m, PIN);
        ws2812_init(&mut m, GPIO_FUNC_SIO);
        m.bus.write_u32(PIO0_BASE + TXF0, 0xffff_ff00).unwrap();
        run(&mut m, 5_000);
        assert!(
            pulses(&mut m).is_empty(),
            "FUNCSEL SIO: the PIO side-set must not reach GP16"
        );
    }

    #[test]
    fn the_pio_drives_nothing_until_pindirs_enable_the_output() {
        let mut m = machine();
        m.bus
            .write_u32(IO_BANK0_BASE + ctrl_offset(PIN), GPIO_FUNC_PIO0)
            .unwrap();
        // `set pins, 1` with the output still disabled: the pad is undriven.
        m.bus
            .write_u32(PIO0_BASE + SM0_PINCTRL, (1 << 26) | (u32::from(PIN) << 5))
            .unwrap();
        m.bus.write_u32(PIO0_BASE + SM0_INSTR, 0xe001).unwrap();
        let sio = m.bus.find_peripheral_index_by_name("sio").unwrap();
        let driver =
            |m: &Machine<crate::cpu::CortexM>| m.bus.peripherals[sio].dev.read_gpio_pad_drive(PIN);
        assert_eq!(driver(&m), Some(PadDrive::HighZ));
        // `set pindirs, 1`: now it drives the latched 1.
        m.bus.write_u32(PIO0_BASE + SM0_INSTR, 0xe081).unwrap();
        assert_eq!(driver(&m), Some(PadDrive::Driven));
        assert_eq!(m.bus.peripherals[sio].dev.read_gpio_pad(PIN), Some(true));
    }
}
