// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! End-to-end waveform test for the STM32 I²C controller: arm the in-engine
//! logic analyzer on the AFR-routed L4 I2C1 SCL/SDA pads (PB6/PB7 on AF4, the
//! standard Nucleo I²C wiring), run a register-level transfer exactly as a HAL
//! driver does, and assert the captured edge stream decodes back to the address
//! and data byte the transfer actually carried.
//!
//! This is the gate for the claim "you can clip the analyzer to any I²C bus in
//! any lab". The STM32 controller is transaction-level — it has no bit engine —
//! so before it narrated its waveform (see
//! [`crate::peripherals::i2c_waveform`]) these pads carried nothing at all and
//! every assertion below failed on an empty trace.
//!
//! The waveform is sampled through the normal `read_gpio_pad` / pad-route path;
//! nothing is synthesized into the capture ring by the test.

#[cfg(test)]
mod stm32_i2c_waveform_tests {
    use crate::cpu::CortexM;
    use crate::logic_capture::LogicEdge;
    use crate::logic_capture::LogicSource;
    use crate::peripherals::gpio::{GpioPort, GpioRegisterLayout};
    use crate::peripherals::i2c::{I2c, I2cDevice, I2cRegisterLayout};
    use crate::{Bus, Machine};

    const RAM_BASE: u64 = 0x2000_0000;
    const GPIOB_BASE: u64 = 0x4001_0C00;
    const I2C1_BASE: u64 = 0x4000_5400;

    const MODER: u64 = 0x00;
    const AFRL: u64 = 0x20;

    const CR1: u64 = I2C1_BASE;
    const CR2: u64 = I2C1_BASE + 0x04;
    const TIMINGR: u64 = I2C1_BASE + 0x10;
    const ISR: u64 = I2C1_BASE + 0x18;
    const TXDR: u64 = I2C1_BASE + 0x28;

    /// Nucleo wiring: PB6 = I2C1_SCL, PB7 = I2C1_SDA (AF4).
    const SCL_PIN: u8 = 6;
    const SDA_PIN: u8 = 7;
    const CH_SCL: u32 = 0;
    const CH_SDA: u32 = 1;

    /// SSD1306-style 7-bit address and one command byte (display on).
    const SLAVE_ADDR: u8 = 0x3C;
    const DATA_BYTE: u8 = 0xAF;

    struct Sink {
        written: Vec<u8>,
    }
    impl I2cDevice for Sink {
        fn address(&self) -> u8 {
            SLAVE_ADDR
        }
        fn read(&mut self) -> u8 {
            0
        }
        fn write(&mut self, data: u8) {
            self.written.push(data);
        }
    }

    /// A machine with an L4 I2C1 and a V2 GPIOB, wired through
    /// `wire_stm32_i2c_pads` exactly as a config-built bus is.
    fn machine() -> Machine<CortexM> {
        let mut bus = crate::bus::SystemBus::new();
        let (cpu, _nvic) = crate::system::cortex_m::configure_cortex_m(&mut bus);
        // The default test bus ships only an F1-layout "gpioa"; this lab needs
        // a V2 GPIOB, which is where I2C1 lives.
        bus.add_peripheral(
            "gpiob",
            GPIOB_BASE,
            0x400,
            None,
            Box::new(GpioPort::new_with_layout(GpioRegisterLayout::Stm32V2)),
        );
        bus.add_peripheral(
            "i2c1",
            I2C1_BASE,
            0x400,
            None,
            Box::new(I2c::new_with_layout(I2cRegisterLayout::Stm32L4)),
        );
        bus.attach_i2c_slave(
            "i2c1",
            Box::new(Sink {
                written: Vec::new(),
            }),
        )
        .unwrap();
        bus.wire_stm32_i2c_pads();

        let mut machine = Machine::new(cpu, bus);
        // NOP slab (`movs r0, #0`) with a Thumb `b` back to the start, so
        // `step()` advances cycles deterministically.
        for i in 0..1022u64 {
            let byte = if i % 2 == 0 { 0x00 } else { 0x20 };
            machine.bus.write_u8(RAM_BASE + i, byte).unwrap();
        }
        machine.bus.write_u8(RAM_BASE + 1022, 0xFF).unwrap();
        machine.bus.write_u8(RAM_BASE + 1023, 0xE5).unwrap();
        machine.cpu.pc = RAM_BASE as u32;
        machine
    }

    /// Route PB6/PB7 to AF4 and enable I2C1, as the HAL's MSP init does.
    fn configure(machine: &mut Machine<CortexM>) {
        let bus = &mut machine.bus;
        bus.write_u32(
            GPIOB_BASE + MODER,
            (0b10 << (SCL_PIN * 2)) | (0b10 << (SDA_PIN * 2)),
        )
        .unwrap();
        bus.write_u32(
            GPIOB_BASE + AFRL,
            (4 << (SCL_PIN * 4)) | (4 << (SDA_PIN * 4)),
        )
        .unwrap();
        // A 100 kHz-ish TIMINGR (PRESC=3, SCLL=0x13, SCLH=0x0F) — any real
        // value works; the point is that the bit time comes from the register.
        bus.write_u32(TIMINGR, 0x3000_0F13).unwrap();
        bus.write_u32(CR1, 1).unwrap(); // PE
    }

    /// One register-level write transfer: arm CR2 (SADD, NBYTES=1, AUTOEND,
    /// START), let the address phase run, then commit the data byte on TXIS.
    fn write_transfer(machine: &mut Machine<CortexM>, byte: u8) {
        let cr2 = (u32::from(SLAVE_ADDR) << 1)
            | (1 << 16) // NBYTES = 1
            | (1 << 25) // AUTOEND
            | (1 << 13); // START
        machine.bus.write_u32(CR2, cr2).unwrap();
        // Spin until the address phase resolves and TXIS asks for the byte.
        for _ in 0..100_000 {
            if machine.bus.read_u32(ISR).unwrap() & (1 << 1) != 0 {
                break;
            }
            machine.step().unwrap();
        }
        machine.bus.write_u32(TXDR, u32::from(byte)).unwrap();
        for _ in 0..200_000 {
            if machine.bus.read_u32(ISR).unwrap() & (1 << 5) != 0 {
                // Pushed pad events reach the capture ring at an observation
                // boundary, i.e. on a step; the completing write is not one.
                // Firmware always runs on past a transfer — step so the trace
                // is drained, exactly as it would be in a real run.
                machine.step().unwrap();
                return; // STOPF
            }
            machine.step().unwrap();
        }
        panic!("transfer never completed — STOPF was never set");
    }

    /// An INDEPENDENT I²C decoder over the captured edges: START on an SDA fall
    /// while SCL is high, one bit sampled per SCL rising edge, frames cut every
    /// nine bits. It knows nothing about how the waveform was produced.
    fn decode(edges: &[LogicEdge], scl_idle: bool, sda_idle: bool) -> Vec<(u8, bool)> {
        let (mut scl, mut sda) = (scl_idle, sda_idle);
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
    fn logic_capture_sees_a_decodable_stm32_i2c_waveform() {
        let mut machine = machine();
        configure(&mut machine);

        let gpiob_idx = machine
            .bus
            .find_peripheral_index_by_name("gpiob")
            .expect("gpiob registered");
        let initial = machine.logic_watch(&[
            Some(LogicSource::pad(gpiob_idx, SCL_PIN)),
            Some(LogicSource::pad(gpiob_idx, SDA_PIN)),
        ]);
        assert_eq!(
            initial,
            vec![Some(true), Some(true)],
            "an idle open-drain I²C bus rests high on both AF-routed pads",
        );

        write_transfer(&mut machine, DATA_BYTE);

        let edges = machine.logic_read_edges(0).edges;
        assert!(
            !edges.is_empty(),
            "an STM32 I²C transfer must put edges on its pads, not a flat trace",
        );
        assert!(edges.iter().any(|e| e.ch == CH_SCL), "SCL must be clocked",);
        assert!(edges.iter().any(|e| e.ch == CH_SDA), "SDA must carry data",);

        // The whole claim, end to end: what the bus carried is what a decoder
        // reading the pads recovers.
        let frames = decode(&edges, true, true);
        assert_eq!(
            frames,
            vec![((SLAVE_ADDR << 1), true), (DATA_BYTE, true)],
            "decoded {frames:?} — expected the addressed write the model performed",
        );
    }

    #[test]
    fn a_transfer_to_an_absent_slave_shows_a_nack_on_the_wire() {
        // The failure a user most needs to SEE on the analyzer: the address
        // went out and nobody answered.
        let mut machine = machine();
        configure(&mut machine);
        let gpiob_idx = machine.bus.find_peripheral_index_by_name("gpiob").unwrap();
        machine.logic_watch(&[
            Some(LogicSource::pad(gpiob_idx, SCL_PIN)),
            Some(LogicSource::pad(gpiob_idx, SDA_PIN)),
        ]);

        const ABSENT: u8 = 0x4A;
        let cr2 = (u32::from(ABSENT) << 1) | (1 << 16) | (1 << 25) | (1 << 13);
        machine.bus.write_u32(CR2, cr2).unwrap();
        for _ in 0..100_000 {
            if machine.bus.read_u32(ISR).unwrap() & (1 << 4) != 0 {
                break; // NACKF
            }
            machine.step().unwrap();
        }
        assert_ne!(
            machine.bus.read_u32(ISR).unwrap() & (1 << 4),
            0,
            "the model must NACK an absent slave",
        );

        let frames = decode(&machine.logic_read_edges(0).edges, true, true);
        assert_eq!(
            frames,
            vec![((ABSENT << 1), false)],
            "the wire must show the address frame NACKed",
        );
    }

    #[test]
    fn scl_on_the_wire_runs_at_the_rate_timingr_programs() {
        // A user measuring the bus must read the frequency the registers ask
        // for, not one invented to fill a phase window.
        let mut machine = machine();
        configure(&mut machine);
        let gpiob_idx = machine.bus.find_peripheral_index_by_name("gpiob").unwrap();
        machine.logic_watch(&[
            Some(LogicSource::pad(gpiob_idx, SCL_PIN)),
            Some(LogicSource::pad(gpiob_idx, SDA_PIN)),
        ]);
        // The narration occupies the cycles leading up to the transfer, so
        // asserting the PROGRAMMED rate requires that much history behind it —
        // which real firmware has, and which a transfer fired at power-on does
        // not (there it compresses to fit; see the narrator's tests).
        for _ in 0..40_000 {
            machine.step().unwrap();
        }
        write_transfer(&mut machine, DATA_BYTE);

        // TIMINGR 0x3000_0F13: PRESC+1 = 4, SCLL+1 = 0x14, SCLH+1 = 0x10 →
        // 4 * (0x14 + 0x10) = 144 kernel periods, × 8 core cycles = 1152.
        const EXPECTED_BIT_TIME: u64 = 1152;
        let rises: Vec<u64> = machine
            .logic_read_edges(0)
            .edges
            .iter()
            .filter(|e| e.ch == CH_SCL && e.value)
            .map(|e| e.cycle)
            .collect();
        assert!(rises.len() >= 9, "at least one full frame of clocks");
        // Consecutive rises WITHIN a frame are one bit time apart; the gap
        // between frames can be longer (the controller is between phases).
        let intra: Vec<u64> = rises.windows(2).map(|pair| pair[1] - pair[0]).collect();
        let common = intra
            .iter()
            .filter(|&&gap| gap == EXPECTED_BIT_TIME)
            .count();
        assert!(
            common >= 8,
            "SCL period should be {EXPECTED_BIT_TIME} cycles for most of a frame, got {intra:?}",
        );
    }
}
