// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! End-to-end waveform test for the ESP32-S3 I²C controller: route pads through
//! the S3 GPIO output matrix to `I2CEXT0_SCL`/`I2CEXT0_SDA`, arm the in-engine
//! logic analyzer on them, run a command list exactly as esp-hal does, and
//! assert the captured edges decode back to the address and bytes the
//! transaction carried.
//!
//! The S3 controller executes its whole command list synchronously on the
//! `TRANS_START` write and charges no wire time, so before it narrated its
//! waveform (see [`crate::peripherals::i2c_waveform`]) these pads carried
//! nothing at all.
//!
//! Everything is sampled through the normal `read_gpio_pad` path; the test
//! synthesizes nothing into the capture ring.

#[cfg(test)]
mod esp32s3_i2c_waveform_tests {
    use crate::logic_capture::LogicEdge;
    use crate::peripherals::esp32s3::gpio::Esp32s3Gpio;
    use crate::peripherals::esp32s3::i2c::Esp32s3I2c;
    use crate::peripherals::i2c::I2cDevice;
    use crate::{Bus, Machine};

    const GPIO_BASE: u64 = 0x6000_4000;
    const I2C0_BASE: u64 = 0x6001_3000;

    /// `GPIO_FUNCn_OUT_SEL_CFG` array base (TRM §5.4).
    const FUNC0_OUT_SEL_CFG: u64 = 0x554;

    /// esp-idf `gpio_sig_map.h` for the S3 — NOT the C3's 53/54.
    const SIG_I2CEXT0_SCL: u32 = 89;
    const SIG_I2CEXT0_SDA: u32 = 90;

    const REG_CTR: u64 = 0x04;
    /// The TX FIFO window: `DATA` (0x1C) is the non-FIFO access, `TXFIFO_START`
    /// is where esp-hal pushes bytes.
    const REG_DATA: u64 = 0x1C;
    const REG_SCL_LOW_PERIOD: u64 = 0x00;
    const REG_SCL_HIGH_PERIOD: u64 = 0x38;
    const REG_CMD0: u64 = 0x58;
    const CTR_TRANS_START: u32 = 1 << 5;

    /// Typical S3 wiring: GPIO8 = SDA, GPIO9 = SCL.
    const SDA_PIN: u8 = 8;
    const SCL_PIN: u8 = 9;
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

    fn machine() -> Machine<crate::cpu::CortexM> {
        let mut bus = crate::bus::SystemBus::new();
        let (cpu, _nvic) = crate::system::cortex_m::configure_cortex_m(&mut bus);
        bus.add_peripheral(
            "gpio",
            GPIO_BASE,
            0x1000,
            None,
            Box::new(Esp32s3Gpio::new()),
        );
        bus.add_peripheral("i2c0", I2C0_BASE, 0x1000, None, Box::new(Esp32s3I2c::new()));
        bus.attach_i2c_slave("i2c0", Box::new(Sink)).unwrap();
        bus.wire_esp32s3_i2c_pads();

        let mut machine = Machine::new(cpu, bus);
        // NOP slab so `step()` advances cycles deterministically.
        const RAM_BASE: u64 = 0x2000_0000;
        for i in 0..1022u64 {
            let byte = if i % 2 == 0 { 0x00 } else { 0x20 };
            machine.bus.write_u8(RAM_BASE + i, byte).unwrap();
        }
        machine.bus.write_u8(RAM_BASE + 1022, 0xFF).unwrap();
        machine.bus.write_u8(RAM_BASE + 1023, 0xE5).unwrap();
        machine.cpu.pc = RAM_BASE as u32;
        machine
    }

    /// Route the two pads through the output matrix and program a real SCL
    /// period, as the driver's bus init does.
    fn configure(machine: &mut Machine<crate::cpu::CortexM>) {
        let bus = &mut machine.bus;
        bus.write_u32(
            GPIO_BASE + FUNC0_OUT_SEL_CFG + u64::from(SCL_PIN) * 4,
            SIG_I2CEXT0_SCL,
        )
        .unwrap();
        bus.write_u32(
            GPIO_BASE + FUNC0_OUT_SEL_CFG + u64::from(SDA_PIN) * 4,
            SIG_I2CEXT0_SDA,
        )
        .unwrap();
        // 100 kHz at 80 MHz APB: 400 low + 400 high.
        bus.write_u32(I2C0_BASE + REG_SCL_LOW_PERIOD, 400).unwrap();
        bus.write_u32(I2C0_BASE + REG_SCL_HIGH_PERIOD, 400).unwrap();
    }

    /// esp-hal shape: RSTART, WRITE n bytes (addr + payload from the FIFO), STOP.
    fn write_transfer(machine: &mut Machine<crate::cpu::CortexM>, payload: &[u8]) {
        let bus = &mut machine.bus;
        bus.write_u32(I2C0_BASE + REG_DATA, u32::from(SLAVE_ADDR << 1))
            .unwrap();
        for &byte in payload {
            bus.write_u32(I2C0_BASE + REG_DATA, u32::from(byte))
                .unwrap();
        }
        let n = (payload.len() + 1) as u32;
        bus.write_u32(I2C0_BASE + REG_CMD0, 6 << 11).unwrap(); // RSTART
        bus.write_u32(I2C0_BASE + REG_CMD0 + 4, (1 << 11) | n)
            .unwrap(); // WRITE
        bus.write_u32(I2C0_BASE + REG_CMD0 + 8, 2 << 11).unwrap(); // STOP
        bus.write_u32(I2C0_BASE + REG_CTR, CTR_TRANS_START).unwrap();
        // Pad pushes reach the ring at an observation boundary; firmware always
        // runs on past a transfer.
        machine.step().unwrap();
    }

    /// An INDEPENDENT I²C decoder — identical rules to the STM32 gate, knowing
    /// nothing about how the waveform was produced.
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
    fn logic_capture_sees_a_decodable_esp32s3_i2c_waveform() {
        let mut machine = machine();
        configure(&mut machine);

        let gpio_idx = machine.bus.find_peripheral_index_by_name("gpio").unwrap();
        let initial = machine.logic_watch(&[Some((gpio_idx, SCL_PIN)), Some((gpio_idx, SDA_PIN))]);
        assert_eq!(
            initial,
            vec![Some(true), Some(true)],
            "an idle open-drain I²C bus rests high on both matrix-routed pads",
        );

        // Enough history for the waveform to occupy at its true rate.
        for _ in 0..80_000 {
            machine.step().unwrap();
        }
        write_transfer(&mut machine, &[0x00, 0xAF]);

        let edges = machine.logic_read_edges(0).edges;
        assert!(
            !edges.is_empty(),
            "an S3 I²C transaction must put edges on its pads, not a flat trace",
        );
        assert_eq!(
            decode(&edges),
            vec![(SLAVE_ADDR << 1, true), (0x00, true), (0xAF, true)],
            "the wire must carry the addressed write the model performed",
        );
    }

    #[test]
    fn an_unrouted_pad_is_untouched_by_the_bus() {
        // The honest converse: a pad the matrix has NOT handed to I²C must keep
        // reporting its own GPIO state, or probing an unrelated pin would show
        // phantom bus traffic.
        let mut machine = machine();
        configure(&mut machine);
        let gpio_idx = machine.bus.find_peripheral_index_by_name("gpio").unwrap();
        const UNRELATED: u8 = 4;
        machine.logic_watch(&[Some((gpio_idx, UNRELATED))]);

        write_transfer(&mut machine, &[0x00, 0xAF]);

        assert!(
            machine.logic_read_edges(0).edges.is_empty(),
            "a pad not routed to the I²C signal must show no bus activity",
        );
    }

    #[test]
    fn scl_runs_at_the_rate_the_timing_registers_program() {
        let mut machine = machine();
        configure(&mut machine);
        let gpio_idx = machine.bus.find_peripheral_index_by_name("gpio").unwrap();
        machine.logic_watch(&[Some((gpio_idx, SCL_PIN)), Some((gpio_idx, SDA_PIN))]);
        // Let the machine run first. The narration occupies the cycles leading
        // up to the transfer, so it needs that much elapsed time behind it —
        // which any real firmware has by the time it touches a bus, but a
        // transfer fired at cycle zero does not.
        // ~20 bit-times of narration at 2400 cycles each needs ~50k cycles of
        // history behind it.
        for _ in 0..80_000 {
            machine.step().unwrap();
        }
        write_transfer(&mut machine, &[0x00]);

        // (SCL_LOW_PERIOD + SCL_HIGH_PERIOD) APB cycles × 3 CPU cycles per APB.
        const EXPECTED_BIT_TIME: u64 = (400 + 400) * 3;
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
            "SCL period should be {EXPECTED_BIT_TIME} cycles across the transaction, got {gaps:?}",
        );
    }
    #[test]
    fn a_transfer_in_the_opening_cycles_of_a_run_still_decodes() {
        // The limitation this covers, end to end. Firmware that touches a bus
        // immediately — no boot, no clock setup — leaves the narration less
        // history than it needs. The trace must still report WHAT crossed the
        // bus; before compression it collapsed into a single-cycle spike and
        // decoded to nothing.
        let mut machine = machine();
        configure(&mut machine);
        let gpio_idx = machine.bus.find_peripheral_index_by_name("gpio").unwrap();
        machine.logic_watch(&[Some((gpio_idx, SCL_PIN)), Some((gpio_idx, SDA_PIN))]);

        // Barely any history: ~3k cycles against a waveform wanting ~49k. This
        // is the case that used to render as a single-cycle spike.
        for _ in 0..3_000 {
            machine.step().unwrap();
        }
        write_transfer(&mut machine, &[0x00, 0xAF]);

        let edges = machine.logic_read_edges(0).edges;
        assert!(
            !edges.is_empty(),
            "a transfer at cycle zero must still leave a trace",
        );
        assert_eq!(
            decode(&edges),
            vec![(SLAVE_ADDR << 1, true), (0x00, true), (0xAF, true)],
            "the bytes must survive even with no history to lay the waveform in",
        );
    }
    /// The bus a REAL S3 lab is built from must have its I²C pads routed.
    ///
    /// Every other test in this file hand-builds a bus and calls
    /// `wire_esp32s3_i2c_pads` itself, which is exactly how this stayed green
    /// while the feature was dark: `configure_xtensa_esp32s3` — the builder
    /// every actual S3 lab goes through — never called it. A waveform gate that
    /// constructs its own wiring proves the narration works, not that anything
    /// ships it.
    ///
    /// So this one asserts on the PRODUCTION builder and nothing else. It reads
    /// the bound routes through the same `bound_pad_functions` hook the
    /// bus-visibility scoreboard uses, so the two can never disagree about
    /// whether a chip can show its bus.
    #[test]
    fn the_production_s3_builder_routes_the_i2c_pads() {
        let mut bus = crate::bus::SystemBus::new();
        let _wiring = crate::system::xtensa::configure_xtensa_esp32s3(
            &mut bus,
            &crate::system::xtensa::Esp32s3Opts::default(),
        );
        let bound = bus.bound_pad_functions();
        assert!(
            bound.iter().any(|f| f.contains("I2CEXT0")),
            "configure_xtensa_esp32s3 must bind I2C0's wire to the output \
             matrix, or every S3 lab reads a flat line while the bus is busy; \
             bound functions were {bound:?}",
        );
    }
}
