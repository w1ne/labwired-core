// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! End-to-end waveform test for the classic-ESP32 (LX6) I²C controller: route
//! pads through the classic GPIO OUTPUT MATRIX to `I2CEXT0_SCL`/`I2CEXT0_SDA`,
//! arm the in-engine logic analyzer on them, run a command list as the
//! arduino-esp32 driver does, and assert the captured edges decode back to the
//! address and bytes the transaction carried.
//!
//! Before this the classic GPIO model did not track `FUNCn_OUT_SEL_CFG` at all,
//! so no bus could be published onto a classic pad and a probe on ANY classic
//! ESP32 pin read a flat line while the C3 and S3 worked.
//!
//! The decoder shares no code with the model: it reconstructs frames from the
//! recorded edges by I²C protocol rules alone. Everything is sampled through the
//! normal `read_gpio_pad` path; nothing is synthesized into the capture ring.

#[cfg(test)]
mod esp32_i2c_waveform_tests {
    use crate::logic_capture::LogicEdge;
    use crate::peripherals::esp32::gpio::Esp32Gpio;
    use crate::peripherals::esp32::i2c::Esp32I2c;
    use crate::peripherals::i2c::I2cDevice;
    use crate::{Bus, Machine};

    const GPIO_BASE: u64 = 0x3FF4_4000;
    const I2C0_BASE: u64 = 0x3FF5_3000;
    const RAM_BASE: u64 = 0x2000_0000;

    /// `GPIO_FUNC0_OUT_SEL_CFG_REG` array base — esp-idf `soc/gpio_reg.h`; the
    /// vendored esp32.svd agrees (addressOffset 0x530, dim 40, stride 4).
    const FUNC0_OUT_SEL_CFG: u64 = 0x530;
    const ENABLE_W1TS: u64 = 0x24;

    /// esp-idf `soc/gpio_sig_map.h` — CLASSIC indices. NOT the C3's 53/54 and
    /// NOT the S3's 89/90.
    const SIG_I2CEXT0_SCL: u32 = 29;
    const SIG_I2CEXT0_SDA: u32 = 30;
    /// `SIG_GPIO_OUT_IDX` — 256 on classic, 128 on the C3.
    const SIG_GPIO_OUT: u32 = 256;

    const REG_SCL_LOW_PERIOD: u64 = 0x00;
    const REG_CTR: u64 = 0x04;
    const REG_DATA: u64 = 0x1C;
    const REG_SCL_HIGH_PERIOD: u64 = 0x38;
    const REG_CMD0: u64 = 0x58;
    const CTR_TRANS_START: u32 = 1 << 5;

    // Classic opcodes (hal/esp32/include/hal/i2c_ll.h): 0=RSTART 1=WRITE
    // 2=READ 3=STOP 4=END — the C3/S3 renumbered these.
    const OP_RSTART: u32 = 0;
    const OP_WRITE: u32 = 1;
    const OP_STOP: u32 = 3;

    /// The arduino-esp32 default Wire pins on a WROOM-32.
    const SDA_PIN: u8 = 21;
    const SCL_PIN: u8 = 22;
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

    /// A bare bus carrying just the two models under test. The CPU is a CortexM
    /// purely to advance cycles deterministically — nothing here executes ESP32
    /// code, and the S3 waveform gate is built the same way.
    fn machine() -> Machine<crate::cpu::CortexM> {
        let mut bus = crate::bus::SystemBus::new();
        let (cpu, _nvic) = crate::system::cortex_m::configure_cortex_m(&mut bus);
        bus.add_peripheral("gpio", GPIO_BASE, 0x1000, None, Box::new(Esp32Gpio::new()));
        bus.add_peripheral("i2c0", I2C0_BASE, 0x1000, None, Box::new(Esp32I2c::new()));
        bus.attach_i2c_slave("i2c0", Box::new(Sink)).unwrap();
        bus.wire_esp32_i2c_pads();

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

    /// What `i2cInit` does: put both pads in output mode, route them through the
    /// matrix, program a real SCL period.
    ///
    /// The ENABLE writes are load-bearing on classic ESP32 — `matrix_signal`
    /// requires the output driver, exactly as `pinMode(sda, OUTPUT_OPEN_DRAIN)`
    /// precedes `pinMatrixOutAttach(sda, I2CEXT0_SDA_OUT_IDX, ...)`.
    fn configure(machine: &mut Machine<crate::cpu::CortexM>) {
        let bus = &mut machine.bus;
        bus.write_u32(
            GPIO_BASE + ENABLE_W1TS,
            (1u32 << SDA_PIN) | (1u32 << SCL_PIN),
        )
        .unwrap();
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
        {
            let bus = &mut machine.bus;
            bus.write_u32(I2C0_BASE + REG_DATA, u32::from(SLAVE_ADDR << 1))
                .unwrap();
            for &byte in payload {
                bus.write_u32(I2C0_BASE + REG_DATA, u32::from(byte))
                    .unwrap();
            }
            let n = (payload.len() + 1) as u32;
            bus.write_u32(I2C0_BASE + REG_CMD0, OP_RSTART << 11)
                .unwrap();
            bus.write_u32(I2C0_BASE + REG_CMD0 + 4, (OP_WRITE << 11) | n)
                .unwrap();
            bus.write_u32(I2C0_BASE + REG_CMD0 + 8, OP_STOP << 11)
                .unwrap();
            bus.write_u32(I2C0_BASE + REG_CTR, CTR_TRANS_START).unwrap();
        }
        // Pad pushes reach the ring at an observation boundary; firmware always
        // runs on past a transfer.
        machine.step().unwrap();
    }

    /// An INDEPENDENT I²C decoder — protocol rules only, knowing nothing about
    /// how the waveform was produced. Identical rules to the STM32/S3/C3 gates.
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
    fn logic_capture_sees_a_decodable_classic_esp32_i2c_waveform() {
        let mut machine = machine();
        configure(&mut machine);

        let gpio_idx = machine.bus.find_peripheral_index_by_name("gpio").unwrap();
        let initial = machine.logic_watch(&[Some((gpio_idx, SCL_PIN)), Some((gpio_idx, SDA_PIN))]);
        assert_eq!(
            initial,
            vec![Some(true), Some(true)],
            "an idle open-drain I²C bus rests high on both matrix-routed pads",
        );

        for _ in 0..80_000 {
            machine.step().unwrap();
        }
        write_transfer(&mut machine, &[0x00, 0xAF]);

        let edges = machine.logic_read_edges(0).edges;
        assert!(
            !edges.is_empty(),
            "a classic-ESP32 I²C transaction must put edges on its pads, not a flat trace",
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
        // phantom bus traffic. Watch the routed SCL pad alongside it, so the
        // silence means the matrix and not a broken fixture.
        let mut machine = machine();
        configure(&mut machine);
        let gpio_idx = machine.bus.find_peripheral_index_by_name("gpio").unwrap();
        const UNRELATED: u8 = 4;
        machine.logic_watch(&[Some((gpio_idx, SCL_PIN)), Some((gpio_idx, UNRELATED))]);
        for _ in 0..80_000 {
            machine.step().unwrap();
        }
        write_transfer(&mut machine, &[0x00, 0xAF]);

        let edges = machine.logic_read_edges(0).edges;
        assert!(
            edges.iter().any(|e| e.ch == 0),
            "the routed SCL pad must carry the clock, or this proves nothing",
        );
        assert!(
            !edges.iter().any(|e| e.ch == 1),
            "a pad not routed to the I²C signal must show no bus activity",
        );
    }

    #[test]
    fn a_pad_taken_back_for_plain_gpio_stops_showing_the_bus() {
        // Re-routing must follow immediately in BOTH directions. Writing the
        // SIG_GPIO_OUT sentinel back is exactly what `gpio_ll_output_disable`
        // does.
        let mut machine = machine();
        configure(&mut machine);
        let gpio_idx = machine.bus.find_peripheral_index_by_name("gpio").unwrap();
        machine.logic_watch(&[Some((gpio_idx, SCL_PIN)), Some((gpio_idx, SDA_PIN))]);
        for _ in 0..80_000 {
            machine.step().unwrap();
        }

        machine
            .bus
            .write_u32(
                GPIO_BASE + FUNC0_OUT_SEL_CFG + u64::from(SDA_PIN) * 4,
                SIG_GPIO_OUT,
            )
            .unwrap();
        // Handing the pad back is itself a real transition — SDA stops showing
        // the wire's idle high and starts showing the GPIO latch — so step once
        // to let that edge reach the ring, then acknowledge it. What follows
        // must contain the BUS, and this separates the two claims.
        machine.step().unwrap();
        // The returned cursor is what drains; reading with 0 again would
        // re-deliver the retained window, i.e. the pre-re-route waveform.
        let cursor = machine.logic_read_edges(0).cursor;

        write_transfer(&mut machine, &[0x00]);
        let edges = machine.logic_read_edges(cursor).edges;
        assert!(
            edges.iter().any(|e| e.ch == CH_SCL),
            "SCL is still routed and must still carry the clock",
        );
        assert!(
            edges.iter().all(|e| e.ch != CH_SDA),
            "SDA was handed back to plain GPIO; it must stop carrying the bus",
        );
    }

    #[test]
    fn scl_runs_at_the_rate_the_timing_registers_program() {
        let mut machine = machine();
        configure(&mut machine);
        let gpio_idx = machine.bus.find_peripheral_index_by_name("gpio").unwrap();
        machine.logic_watch(&[Some((gpio_idx, SCL_PIN)), Some((gpio_idx, SDA_PIN))]);
        for _ in 0..80_000 {
            machine.step().unwrap();
        }
        write_transfer(&mut machine, &[0x00]);

        // (SCL_LOW_PERIOD + SCL_HIGH_PERIOD) APB cycles x 3 CPU cycles per APB.
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
            "SCL period should be {EXPECTED_BIT_TIME} cycles, got {gaps:?}",
        );
    }
}
