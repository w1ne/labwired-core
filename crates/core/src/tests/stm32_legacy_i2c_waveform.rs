// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! End-to-end waveform gate for the LEGACY (F1/F2/F4) STM32 I²C controller.
//!
//! The modern L4-generation controller has narrated its pads since
//! [`crate::peripherals::i2c_waveform`] landed; the legacy CR1/CR2/DR/SR1/SR2
//! controller had no wire model at all. Every F4 part in the fleet
//! (stm32f401, f401cdu6, f405, f407, f411ceu6) therefore ran real I²C traffic
//! while a probe on PB6/PB7 read the GPIO output latch — a flat line — and
//! `docs/coverage/bus-visibility.md` recorded a `—` in their I2C column.
//!
//! ⚠️ These tests build the chip through `SystemBus::from_config` on the REAL
//! `configs/chips/stm32f401.yaml`, not a hand-assembled bus. A gate that
//! registers its own peripherals and calls `wire_stm32_i2c_pads` itself proves
//! the narration works, not that anything ships it — which is exactly how a
//! chip can stay dark in production with a green waveform test. The wiring here
//! is whatever `from_config` really does.
//!
//! The fix is deliberately NOT `profile: "stm32v2"` on the F4 chip yamls. The
//! F4 carries the legacy I²C IP (RM0090 §27: CR1/CR2/DR/SR1/SR2/CCR/TRISE), not
//! the v2 IP the L4 has (TIMINGR/ISR/TXDR/RXDR); switching the profile would
//! turn the board green by modelling the wrong silicon.

#[cfg(test)]
mod stm32_legacy_i2c_waveform_tests {
    use crate::bus::SystemBus;
    use crate::cpu::CortexM;
    use crate::logic_capture::LogicEdge;
    use crate::peripherals::i2c::I2cDevice;
    use crate::{Bus, Machine};
    use labwired_config::{ChipDescriptor, SystemManifest};
    use std::path::PathBuf;

    // Addresses from configs/chips/stm32f401.yaml (RM0368 memory map).
    const RAM_BASE: u64 = 0x2000_0000;
    const GPIOB_BASE: u64 = 0x4002_0400;
    const I2C1_BASE: u64 = 0x4000_5400;

    // GPIO v2 register offsets.
    const MODER: u64 = 0x00;
    const AFRL: u64 = 0x20;

    // Legacy I²C register offsets (RM0090 §27.6).
    const CR1: u64 = I2C1_BASE;
    const DR: u64 = I2C1_BASE + 0x10;
    const SR1: u64 = I2C1_BASE + 0x14;
    const SR2: u64 = I2C1_BASE + 0x18;
    const CCR: u64 = I2C1_BASE + 0x1C;

    const CR1_PE: u32 = 0x0001;
    const CR1_START: u32 = 0x0100;
    const CR1_STOP: u32 = 0x0200;
    const CR1_ACK: u32 = 0x0400;

    const SR1_SB: u32 = 0x0001;
    const SR1_ADDR: u32 = 0x0002;
    const SR1_RXNE: u32 = 0x0040;
    const SR1_TXE: u32 = 0x0080;
    const SR1_AF: u32 = 0x0400;

    /// STM32F401 datasheet DS10086 Rev 5, Table 9 (page 46): PB6/AF4 =
    /// I2C1_SCL, PB7/AF4 = I2C1_SDA — the standard Nucleo/BlackPill wiring.
    const SCL_PIN: u8 = 6;
    const SDA_PIN: u8 = 7;
    const AF_I2C: u32 = 4;
    const CH_SCL: u32 = 0;
    const CH_SDA: u32 = 1;

    /// CCR for ~100 kHz standard mode: one SCL period is `2 × CCR` PCLK1
    /// periods (RM0090 §27.6.8), so 210 gives a 420-cycle bit time.
    const CCR_VALUE: u32 = 210;
    const BIT_TIME: u64 = 2 * CCR_VALUE as u64;

    const SLAVE_ADDR: u8 = 0x3C;
    const DATA_BYTE: u8 = 0xAF;
    const REG_PTR: u8 = 0x32;

    /// A slave that records what it was written and answers a fixed script, so
    /// a read gate knows exactly which bytes should appear on the wire.
    struct Sink {
        answers: Vec<u8>,
        next: usize,
    }
    impl I2cDevice for Sink {
        fn address(&self) -> u8 {
            SLAVE_ADDR
        }
        fn read(&mut self) -> u8 {
            let byte = self.answers[self.next.min(self.answers.len() - 1)];
            self.next += 1;
            byte
        }
        fn write(&mut self, _data: u8) {}
    }

    fn repo_root(rel: &str) -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .join(rel)
    }

    /// The same minimal manifest `crates/core/tests/bus_visibility.rs` and
    /// `chip_conformance.rs` use, so this gate measures the construction path
    /// those boards measure.
    fn dummy_manifest(path: &str) -> SystemManifest {
        SystemManifest {
            parts: Vec::new(),
            walk_deleted: Some(false),
            schema_version: "1.0".to_string(),
            name: "legacy-i2c-waveform".to_string(),
            chip: path.to_string(),
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

    fn bus_for(chip_name: &str) -> SystemBus {
        let path = repo_root(&format!("configs/chips/{chip_name}.yaml"));
        let chip =
            ChipDescriptor::from_file(&path).unwrap_or_else(|e| panic!("load {chip_name}: {e}"));
        let abs = path.to_string_lossy().to_string();
        SystemBus::from_config(&chip, &dummy_manifest(&abs))
            .unwrap_or_else(|e| panic!("assemble {chip_name}: {e}"))
    }

    /// A real stm32f401 machine with `answers` scripted into an attached slave.
    fn machine(answers: Vec<u8>) -> Machine<CortexM> {
        let mut bus = bus_for("stm32f401");
        bus.attach_i2c_slave("i2c1", Box::new(Sink { answers, next: 0 }))
            .expect("attach slave to the config-built i2c1");
        let (cpu, _nvic) = crate::system::cortex_m::configure_cortex_m(&mut bus);
        let mut machine = Machine::new(cpu, bus);
        // NOP slab (`movs r0, #0`) ending in a Thumb `b` back to the start, so
        // `step()` advances cycles deterministically — same shape as the L4
        // waveform gate next door.
        for i in 0..1022u64 {
            let byte = if i % 2 == 0 { 0x00 } else { 0x20 };
            machine.bus.write_u8(RAM_BASE + i, byte).unwrap();
        }
        machine.bus.write_u8(RAM_BASE + 1022, 0xFF).unwrap();
        machine.bus.write_u8(RAM_BASE + 1023, 0xE5).unwrap();
        machine.cpu.pc = RAM_BASE as u32;
        machine
    }

    /// Route PB6/PB7 to AF4 and program CCR + PE, as the HAL's MSP init does.
    fn configure(machine: &mut Machine<CortexM>) {
        let bus = &mut machine.bus;
        bus.write_u32(
            GPIOB_BASE + MODER,
            (0b10 << (SCL_PIN * 2)) | (0b10 << (SDA_PIN * 2)),
        )
        .unwrap();
        bus.write_u32(
            GPIOB_BASE + AFRL,
            (AF_I2C << (SCL_PIN * 4)) | (AF_I2C << (SDA_PIN * 4)),
        )
        .unwrap();
        bus.write_u32(CCR, CCR_VALUE).unwrap();
        bus.write_u32(CR1, CR1_PE).unwrap();
    }

    fn arm(machine: &mut Machine<CortexM>) {
        let gpiob = machine
            .bus
            .find_peripheral_index_by_name("gpiob")
            .expect("gpiob registered by from_config");
        machine.logic_watch(&[Some((gpiob, SCL_PIN)), Some((gpiob, SDA_PIN))]);
    }

    /// Spin the CPU until `flag` shows up in SR1, so the narration has real
    /// history behind it (the plan is anchored to END at the present cycle).
    fn wait_sr1(machine: &mut Machine<CortexM>, flag: u32) {
        for _ in 0..200_000 {
            if machine.bus.read_u32(SR1).unwrap() & flag != 0 {
                return;
            }
            machine.step().unwrap();
        }
        panic!("SR1 flag {flag:#06x} never set");
    }

    /// START + address phase. Returns nothing; the caller asserts on SR1.
    fn start_and_address(machine: &mut Machine<CortexM>, addr_byte: u8) {
        let cr1 = machine.bus.read_u32(CR1).unwrap();
        machine.bus.write_u32(CR1, cr1 | CR1_START).unwrap();
        wait_sr1(machine, SR1_SB);
        machine.bus.write_u32(DR, u32::from(addr_byte)).unwrap();
    }

    /// An INDEPENDENT I²C decoder over the captured edges: START on an SDA fall
    /// while SCL is high, STOP on an SDA rise while SCL is high, one bit
    /// sampled per SCL rising edge, frames cut every nine bits. It knows
    /// nothing about how the waveform was produced — protocol rules only.
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

    /// The scoreboard's own claim, asserted on the shipping construction path:
    /// the F4 binds its I²C wire to a pad, and the F103 — same legacy
    /// controller, F1-layout GPIO ports this table does not route — does NOT.
    ///
    /// The second half is the ordering hazard `wire_stm32_i2c_pads` documents:
    /// creating the pad cell for a controller no route reaches would switch the
    /// whole narration machinery on to publish into something nothing reads.
    #[test]
    fn the_config_built_f4_binds_its_i2c_pads_and_the_f103_does_not() {
        let f401 = bus_for("stm32f401").bound_pad_functions();
        assert!(
            f401.contains(&"I2C1_SCL") && f401.contains(&"I2C1_SDA"),
            "stm32f401 must bind I2C1 SCL+SDA through from_config, got {f401:?}",
        );

        let f103 = bus_for("stm32f103").bound_pad_functions();
        assert!(
            !f103.iter().any(|f| f.starts_with("I2C")),
            "stm32f103's GPIO ports carry the F1 register layout, which no I²C \
             AF table routes — binding anything there would be a pad cell no \
             route reaches, got {f103:?}",
        );
    }

    /// The wrong-silicon guard: an F4 pad the datasheet leaves EMPTY at AF4
    /// must not carry an I²C wire.
    ///
    /// STM32L476 DS10198 Table 17 puts I2C3_SCL on PA7/AF4; STM32F401 DS10086
    /// Rev 5 Table 9 (page 45) shows `-` in the AF04 column for PA7 — that pad
    /// has no I²C function on this silicon at all. Routing the L4 table to the
    /// legacy controller would publish I2C3's idle-high open-drain wire onto
    /// PA7, so a probe would read HIGH on a pad firmware had driven LOW.
    ///
    /// Asserted through the pad read, which is what the analyzer samples:
    /// `logic_watch` answers `None` for an AF pad no peripheral wire is bound
    /// to and `Some(level)` for one that is, so the two cases are directly
    /// distinguishable without reaching into the routing table.
    #[test]
    fn an_f4_pad_the_datasheet_leaves_empty_at_af4_carries_no_i2c_wire() {
        // stm32f401cdu6 is the F4 in the fleet that actually instantiates i2c3.
        let mut bus = bus_for("stm32f401cdu6");
        let (cpu, _nvic) = crate::system::cortex_m::configure_cortex_m(&mut bus);
        let mut machine = Machine::new(cpu, bus);
        const GPIOA_BASE: u64 = 0x4002_0000;
        const PA7: u8 = 7;
        const PA8: u8 = 8;
        const AFRH: u64 = 0x24;
        // PA7 and PA8 both to AF4.
        machine
            .bus
            .write_u32(
                GPIOA_BASE + MODER,
                (0b10 << (PA7 * 2)) | (0b10 << (PA8 * 2)),
            )
            .unwrap();
        machine
            .bus
            .write_u32(GPIOA_BASE + AFRL, AF_I2C << (PA7 * 4))
            .unwrap();
        machine
            .bus
            .write_u32(GPIOA_BASE + AFRH, AF_I2C << ((PA8 - 8) * 4))
            .unwrap();
        let gpioa = machine
            .bus
            .find_peripheral_index_by_name("gpioa")
            .expect("gpioa registered by from_config");
        assert_eq!(
            machine.logic_watch(&[Some((gpioa, PA7)), Some((gpioa, PA8))]),
            // PA7: no wire. PA8: I2C3_SCL, an idle open-drain bus, so high.
            vec![None, Some(true)],
            "DS10086 Rev 5 Table 9 (page 45): AF04 on PA7 is unassigned and AF04 \
             on PA8 is I2C3_SCL. A wire on PA7 means the L4 table leaked onto \
             legacy silicon; no wire on PA8 means this gate is measuring nothing",
        );
    }

    #[test]
    fn logic_capture_sees_a_decodable_legacy_i2c_waveform() {
        let mut machine = machine(vec![0]);
        configure(&mut machine);
        arm(&mut machine);

        // Give the narration room: the plan ends at the present cycle and
        // reaches back over its own span, which real firmware always has.
        for _ in 0..40_000 {
            machine.step().unwrap();
        }

        start_and_address(&mut machine, SLAVE_ADDR << 1);
        wait_sr1(&mut machine, SR1_ADDR);
        machine.bus.read_u32(SR1).unwrap();
        machine.bus.read_u32(SR2).unwrap(); // ADDR clear sequence
        wait_sr1(&mut machine, SR1_TXE);
        machine.bus.write_u32(DR, u32::from(DATA_BYTE)).unwrap();
        wait_sr1(&mut machine, SR1_TXE);
        let cr1 = machine.bus.read_u32(CR1).unwrap();
        machine.bus.write_u32(CR1, cr1 | CR1_STOP).unwrap();
        // Pushed pad events reach the capture ring at an observation boundary,
        // i.e. on a step; the completing write is not one.
        machine.step().unwrap();

        let edges = machine.logic_read_edges(0).edges;
        assert!(
            !edges.is_empty(),
            "a legacy STM32 I²C transfer must put edges on its pads, not a flat trace",
        );
        assert_eq!(
            decode(&edges),
            vec![((SLAVE_ADDR << 1), true), (DATA_BYTE, true)],
            "the decoder must recover the addressed write the model performed",
        );
    }

    #[test]
    fn a_transfer_to_an_absent_slave_shows_a_nacked_address_on_the_wire() {
        // The failure a user most needs to SEE: the address went out and
        // nobody answered.
        const ABSENT: u8 = 0x4A;
        let mut machine = machine(vec![0]);
        configure(&mut machine);
        arm(&mut machine);
        for _ in 0..40_000 {
            machine.step().unwrap();
        }

        start_and_address(&mut machine, ABSENT << 1);
        wait_sr1(&mut machine, SR1_AF);
        let cr1 = machine.bus.read_u32(CR1).unwrap();
        machine.bus.write_u32(CR1, cr1 | CR1_STOP).unwrap();
        machine.step().unwrap();

        assert_eq!(
            decode(&machine.logic_read_edges(0).edges),
            vec![((ABSENT << 1), false)],
            "the wire must show the address frame NACKed",
        );
    }

    #[test]
    fn a_repeated_start_read_decodes_as_every_frame_that_crossed_the_bus() {
        // The standard sensor shape, and the one the legacy controller narrates
        // and the L4 one cannot: write the register pointer, repeated START,
        // read two bytes back, ACK the first and NACK the last.
        //
        // The second data byte is the regression this test exists for: a legacy
        // master-receive pulls it out of the slave on the `&self` DR read path,
        // which a `&mut`-only recorder cannot see at all — the trace would
        // decode one byte SHORT and look entirely plausible.
        const B0: u8 = 0xE5;
        const B1: u8 = 0x1D;
        let mut machine = machine(vec![B0, B1]);
        configure(&mut machine);
        arm(&mut machine);
        for _ in 0..80_000 {
            machine.step().unwrap();
        }

        start_and_address(&mut machine, SLAVE_ADDR << 1);
        wait_sr1(&mut machine, SR1_ADDR);
        machine.bus.read_u32(SR1).unwrap();
        machine.bus.read_u32(SR2).unwrap();
        wait_sr1(&mut machine, SR1_TXE);
        machine.bus.write_u32(DR, u32::from(REG_PTR)).unwrap();
        wait_sr1(&mut machine, SR1_TXE);

        // Repeated START, this time addressing for a read, with ACK armed.
        let cr1 = machine.bus.read_u32(CR1).unwrap();
        machine.bus.write_u32(CR1, cr1 | CR1_ACK).unwrap();
        start_and_address(&mut machine, (SLAVE_ADDR << 1) | 1);
        wait_sr1(&mut machine, SR1_ADDR);
        machine.bus.read_u32(SR1).unwrap();
        machine.bus.read_u32(SR2).unwrap();
        wait_sr1(&mut machine, SR1_RXNE);
        assert_eq!(
            machine.bus.read_u32(DR).unwrap() as u8,
            B0,
            "first received byte",
        );

        // Clear ACK before the final byte, exactly as a driver does to tell the
        // slave to stop driving — and assert the wire shows that NACK.
        let cr1 = machine.bus.read_u32(CR1).unwrap();
        machine.bus.write_u32(CR1, cr1 & !CR1_ACK).unwrap();
        assert_eq!(
            machine.bus.read_u32(DR).unwrap() as u8,
            B1,
            "second received byte",
        );
        let cr1 = machine.bus.read_u32(CR1).unwrap();
        machine.bus.write_u32(CR1, cr1 | CR1_STOP).unwrap();
        machine.step().unwrap();

        assert_eq!(
            decode(&machine.logic_read_edges(0).edges),
            vec![
                ((SLAVE_ADDR << 1), true),
                (REG_PTR, true),
                ((SLAVE_ADDR << 1) | 1, true),
                (B0, true),
                (B1, false),
            ],
            "every frame that crossed the bus must appear, in order, with its \
             real ACK — including the byte the `&self` read path pulled",
        );
    }

    /// The FIRST received byte carries the master's real ACK too.
    ///
    /// A single-byte read is the one shape where firmware clears CR1.ACK before
    /// the only data frame, so the wire must show that frame NACKed. Without
    /// this, the ACK on the byte the DataPending phase latches could be
    /// hardcoded `true` and every other gate here would still pass — the
    /// sibling test above only ever clears ACK for its SECOND byte, which is
    /// pulled on a different code path.
    #[test]
    fn a_single_byte_read_narrates_the_nack_the_master_actually_drove() {
        const ONLY: u8 = 0x7B;
        let mut machine = machine(vec![ONLY]);
        configure(&mut machine); // CR1 = PE, so ACK is CLEAR
        arm(&mut machine);
        for _ in 0..60_000 {
            machine.step().unwrap();
        }

        start_and_address(&mut machine, (SLAVE_ADDR << 1) | 1);
        wait_sr1(&mut machine, SR1_ADDR);
        machine.bus.read_u32(SR1).unwrap();
        machine.bus.read_u32(SR2).unwrap();
        wait_sr1(&mut machine, SR1_RXNE);
        assert_eq!(machine.bus.read_u32(DR).unwrap() as u8, ONLY);
        let cr1 = machine.bus.read_u32(CR1).unwrap();
        assert_eq!(cr1 & CR1_ACK, 0, "precondition: the master is NOT ACKing");
        machine.bus.write_u32(CR1, cr1 | CR1_STOP).unwrap();
        machine.step().unwrap();

        assert_eq!(
            decode(&machine.logic_read_edges(0).edges),
            vec![((SLAVE_ADDR << 1) | 1, true), (ONLY, false)],
            "the address ACKs, and the lone data byte must show the NACK CR1.ACK \
             says the master drove",
        );
    }

    #[test]
    fn scl_on_the_wire_runs_at_the_rate_ccr_programs() {
        // A user measuring the bus must read the frequency the registers ask
        // for, not one invented to fill a phase window.
        let mut machine = machine(vec![0]);
        configure(&mut machine);
        arm(&mut machine);
        for _ in 0..60_000 {
            machine.step().unwrap();
        }

        start_and_address(&mut machine, SLAVE_ADDR << 1);
        wait_sr1(&mut machine, SR1_ADDR);
        machine.bus.read_u32(SR1).unwrap();
        machine.bus.read_u32(SR2).unwrap();
        wait_sr1(&mut machine, SR1_TXE);
        machine.bus.write_u32(DR, u32::from(DATA_BYTE)).unwrap();
        wait_sr1(&mut machine, SR1_TXE);
        let cr1 = machine.bus.read_u32(CR1).unwrap();
        machine.bus.write_u32(CR1, cr1 | CR1_STOP).unwrap();
        machine.step().unwrap();

        let rises: Vec<u64> = machine
            .logic_read_edges(0)
            .edges
            .iter()
            .filter(|e| e.ch == CH_SCL && e.value)
            .map(|e| e.cycle)
            .collect();
        assert!(
            rises.len() >= 18,
            "two frames of clocks, got {}",
            rises.len()
        );
        let gaps: Vec<u64> = rises.windows(2).map(|pair| pair[1] - pair[0]).collect();
        let at_rate = gaps.iter().filter(|&&gap| gap == BIT_TIME).count();
        assert!(
            at_rate >= 16,
            "SCL period should be {BIT_TIME} cycles across the transfer, got {gaps:?}",
        );
    }
}
