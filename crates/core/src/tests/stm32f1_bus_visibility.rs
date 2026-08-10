// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! End-to-end waveform gates for the STM32F103's I²C and USART pads.
//!
//! # What was dark
//!
//! `docs/coverage/bus-visibility.md` read `| stm32f103 | — | ✓ | — |`. The F103
//! is the most-used chip on the estate — every bundled I²C sensor lab
//! (bme280, mpu6050, ds3231, ina219, ads1115, adxl345, vl53l1x, ssd1306) is an
//! F103 lab — and a probe on PB6/PB7 read the GPIO output latch, a flat line,
//! while real I²C traffic crossed the model. The SPI column was already `✓`
//! because `wire_stm32_spi_pads` had grown an F1 table; I²C and USART had not.
//!
//! The controller side was already done: `F1I2c` publishes a `PadLines` cell and
//! narrates whole transactions at STOP, and `Uart` narrates 8N1 characters from
//! its F1 `BRR`. The gap was purely that no pad route reached either wire, so
//! `pad_lines_arc` was never called and the machinery stayed switched off.
//!
//! # Why these tests build from the yaml
//!
//! ⚠️ Every gate here builds through `SystemBus::from_config` on the COMMITTED
//! `configs/chips/stm32f103.yaml`. A gate that registers its own peripherals and
//! calls `wire_stm32_i2c_pads` itself proves the narration works, not that
//! anything ships it — which is exactly how a chip stays dark in production with
//! a green waveform test.
//!
//! The decoders share no code with the model. The I²C one is protocol rules
//! only: START on an SDA fall while SCL is high, STOP on an SDA rise while SCL
//! is high, one bit sampled per SCL rising edge, frames cut every nine bits.
//! The serial one syncs on the falling start edge and samples each bit at its
//! centre, LSB first, as a receiver does.

#[cfg(test)]
mod stm32f1_bus_visibility_tests {
    use crate::bus::SystemBus;
    use crate::cpu::CortexM;
    use crate::logic_capture::LogicEdge;
    use crate::logic_capture::LogicSource;
    use crate::peripherals::i2c::I2cDevice;
    use crate::peripherals::uart::Uart;
    use crate::{Bus, Machine};
    use labwired_config::{ChipDescriptor, SystemManifest};
    use std::path::PathBuf;

    // Addresses from configs/chips/stm32f103.yaml (RM0008 memory map).
    const RAM_BASE: u64 = 0x2000_0000;
    const RCC_BASE: u64 = 0x4002_1000;
    const GPIOA_BASE: u64 = 0x4001_0800;
    const GPIOB_BASE: u64 = 0x4001_0C00;
    const I2C1_BASE: u64 = 0x4000_5400;
    const USART1_BASE: u64 = 0x4001_3800;

    /// F1 RCC clock-enable registers (RM0008 §7.3.7/§7.3.8). The F103 yaml gates
    /// gpioa, i2c1 and uart1 on these, so a test that skips them writes into a
    /// dead peripheral exactly as firmware does.
    const APB2ENR: u64 = 0x18;
    const APB1ENR: u64 = 0x1C;
    const IOPAEN: u32 = 1 << 2;
    const USART1EN: u32 = 1 << 14;
    const I2C1EN: u32 = 1 << 21;

    /// F1 GPIO port configuration registers: four bits per pin, MODE[1:0] then
    /// CNF[1:0]. CRL covers pins 0-7, CRH pins 8-15.
    const CRL: u64 = 0x00;
    const CRH: u64 = 0x04;
    /// MODE=0b11 (output, 50 MHz) + CNF=0b11 (alternate function, open drain) —
    /// how a driver configures an I²C pad. DS5319 Rev 20 §5.3.16 (page 68)
    /// confirms SDA/SCL are driven open drain on this part.
    const AF_OPEN_DRAIN: u32 = 0b1111;
    /// MODE=0b11 + CNF=0b10 (alternate function, push-pull) — a USART TX pad.
    const AF_PUSH_PULL: u32 = 0b1011;
    /// MODE=0b01 (output, 10 MHz) + CNF=0b00 (general purpose push-pull) — a
    /// plain GPIO output, which must NOT read any peripheral wire.
    const GPIO_PUSH_PULL: u32 = 0b0001;

    // Legacy I²C register offsets (RM0008 §26.6).
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

    /// F1 USART register offsets (RM0008 §27.6): SR@0x00, DR@0x04, BRR@0x08.
    ///
    /// ⚠️ Never write SR: the model aliases a write at offset 0x00 to a TX push
    /// on this layout.
    const USART_DR: u64 = 0x04;
    const USART_BRR: u64 = 0x08;

    /// DS5319 Rev 20, Table 5 (page 32), Default column: PB6 = `I2C1_SCL`,
    /// PB7 = `I2C1_SDA` — the Blue Pill wiring every bundled sensor lab uses.
    const SCL_PIN: u8 = 6;
    const SDA_PIN: u8 = 7;
    /// DS5319 Rev 20, Table 5 (page 33), REMAP column: PB8 = `I2C1_SCL/CANRX`.
    /// Live only under `AFIO_MAPR.I2C1_REMAP`, which nothing decodes — so this
    /// pad must carry NO wire. Its DEFAULT function is TIM4_CH3.
    const REMAP_SCL_PIN: u8 = 8;
    /// DS5319 Rev 20, Table 5 (page 31), Default column: PA9 = `USART1_TX`.
    const TX_PIN: u8 = 9;

    const CH_SCL: u32 = 0;
    const CH_SDA: u32 = 1;
    const CH_TX: u32 = 0;

    /// CCR for ~100 kHz standard mode: one SCL period is `2 × CCR` PCLK1 periods
    /// (RM0008 §26.6.8), so 210 gives a 420-cycle bit time.
    const CCR_VALUE: u32 = 210;

    /// 115200 baud from a 72 MHz PCLK2 with the F1's fixed 16× oversampling:
    /// `BRR = 72e6 / 115200 = 625`, and the divisor IS one bit period in
    /// peripheral clocks.
    const BRR_VALUE: u32 = 625;
    const BIT_TIME: u64 = 625;

    const SLAVE_ADDR: u8 = 0x3C;

    // ⚠️ EVERY payload here is BIT-ASYMMETRIC — no value reads the same
    // reversed. A bit-palindromic byte (0x00, 0xFF, 0xA5, 0x5A, 0x3C, 0x81)
    // survives an LSB-first/MSB-first mutation silently, because the mutant
    // decodes to the same number. `0xB2` reverses to `0x4D`, `0x01` to `0x80`,
    // and the address byte `0x3C << 1 = 0x78` to `0x1E`.
    const DATA_BYTE: u8 = 0xB2;
    const REG_PTR: u8 = 0x01;
    const READ_B0: u8 = 0x01;
    const READ_B1: u8 = 0xB2;
    const SERIAL_TEXT: &[u8] = &[0xB2, 0x01];

    /// A slave that answers a fixed script, so a read gate knows exactly which
    /// bytes should appear on the wire.
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

    /// The same minimal manifest `crates/core/tests/bus_visibility.rs` uses, so
    /// this gate measures the construction path that board measures.
    fn dummy_manifest(path: &str) -> SystemManifest {
        SystemManifest {
            parts: Vec::new(),
            walk_deleted: Some(false),
            schema_version: "1.0".to_string(),
            name: "stm32f1-bus-visibility".to_string(),
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

    /// A NOP slab (`movs r0, #0`) ending in a Thumb `b` back to the start, so
    /// `step()` advances cycles deterministically.
    fn load_nop_slab(machine: &mut Machine<CortexM>) {
        for i in 0..1022u64 {
            let byte = if i % 2 == 0 { 0x00 } else { 0x20 };
            machine.bus.write_u8(RAM_BASE + i, byte).unwrap();
        }
        machine.bus.write_u8(RAM_BASE + 1022, 0xFF).unwrap();
        machine.bus.write_u8(RAM_BASE + 1023, 0xE5).unwrap();
        machine.cpu.pc = RAM_BASE as u32;
    }

    /// A real config-built F103 machine with `answers` scripted into a slave on
    /// i2c1.
    fn i2c_machine(answers: Vec<u8>) -> Machine<CortexM> {
        let mut bus = bus_for("stm32f103");
        bus.attach_i2c_slave("i2c1", Box::new(Sink { answers, next: 0 }))
            .expect("attach slave to the config-built i2c1");
        let (cpu, _nvic) = crate::system::cortex_m::configure_cortex_m(&mut bus);
        let mut machine = Machine::new(cpu, bus);
        load_nop_slab(&mut machine);
        machine
    }

    /// A real config-built F103 machine whose usart1 prints nowhere, so the gate
    /// asserts on the pad waveform and not on the test harness's stdout.
    fn uart_machine() -> Machine<CortexM> {
        let mut bus = bus_for("stm32f103");
        let idx = bus
            .find_peripheral_index_by_name("uart1")
            .expect("uart1 registered by from_config");
        if let Some(uart) = bus.peripherals[idx]
            .dev
            .as_any_mut()
            .and_then(|a| a.downcast_mut::<Uart>())
        {
            uart.set_sink(None, false);
        }
        let (cpu, _nvic) = crate::system::cortex_m::configure_cortex_m(&mut bus);
        let mut machine = Machine::new(cpu, bus);
        load_nop_slab(&mut machine);
        machine
    }

    /// Write one pin's 4-bit CRL/CRH field without disturbing the others.
    fn set_pin_config(machine: &mut Machine<CortexM>, port_base: u64, pin: u8, config: u32) {
        let reg = port_base + if pin < 8 { CRL } else { CRH };
        let shift = ((pin % 8) * 4) as u32;
        let old = machine.bus.read_u32(reg).unwrap();
        machine
            .bus
            .write_u32(reg, (old & !(0xF << shift)) | (config << shift))
            .unwrap();
    }

    /// Enable I2C1 + its port, route PB6/PB7 to alternate-function open drain,
    /// and program CCR + PE, exactly as the HAL's MSP init does.
    fn configure_i2c(machine: &mut Machine<CortexM>) {
        machine.bus.write_u32(RCC_BASE + APB1ENR, I2C1EN).unwrap();
        set_pin_config(machine, GPIOB_BASE, SCL_PIN, AF_OPEN_DRAIN);
        set_pin_config(machine, GPIOB_BASE, SDA_PIN, AF_OPEN_DRAIN);
        machine.bus.write_u32(CCR, CCR_VALUE).unwrap();
        machine.bus.write_u32(CR1, CR1_PE).unwrap();
    }

    fn arm_i2c(machine: &mut Machine<CortexM>) {
        let gpiob = machine
            .bus
            .find_peripheral_index_by_name("gpiob")
            .expect("gpiob registered by from_config");
        machine.logic_watch(&[
            Some(LogicSource::pad(gpiob, SCL_PIN)),
            Some(LogicSource::pad(gpiob, SDA_PIN)),
        ]);
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

    fn start_and_address(machine: &mut Machine<CortexM>, addr_byte: u8) {
        let cr1 = machine.bus.read_u32(CR1).unwrap();
        machine.bus.write_u32(CR1, cr1 | CR1_START).unwrap();
        wait_sr1(machine, SR1_SB);
        machine.bus.write_u32(DR, u32::from(addr_byte)).unwrap();
    }

    /// An INDEPENDENT I²C decoder over the captured edges. It knows nothing
    /// about how the waveform was produced — protocol rules only.
    fn decode_i2c(edges: &[LogicEdge]) -> Vec<(u8, bool)> {
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

    /// An INDEPENDENT asynchronous-serial decoder: sync on the falling start
    /// edge, sample each bit at its centre, LSB first.
    fn decode_serial(edges: &[LogicEdge], bit_time: u64) -> Vec<u8> {
        let timeline: Vec<(u64, bool)> = edges
            .iter()
            .filter(|e| e.ch == CH_TX)
            .map(|e| (e.cycle, e.value))
            .collect();
        let level_at = |t: u64| -> bool {
            timeline
                .iter()
                .rev()
                .find(|(cycle, _)| *cycle <= t)
                .map(|(_, level)| *level)
                .unwrap_or(true)
        };

        let mut bytes = Vec::new();
        let mut cursor = 0u64;
        for &(cycle, level) in &timeline {
            if level || cycle < cursor {
                continue;
            }
            if level_at(cycle + bit_time / 2) {
                continue; // a glitch, not a start bit
            }
            let mut byte = 0u8;
            for index in 0..8u64 {
                if level_at(cycle + bit_time / 2 + bit_time * (index + 1)) {
                    byte |= 1 << index; // LSB first
                }
            }
            if !level_at(cycle + bit_time / 2 + bit_time * 9) {
                continue; // no stop bit: not a character
            }
            bytes.push(byte);
            cursor = cycle + bit_time * 10;
        }
        bytes
    }

    // ── The board's own claim, on the shipping construction path ─────────────

    /// The F103 must BIND both I²C instances and USART1/USART2 through
    /// `from_config`, and must NOT bind USART3.
    ///
    /// The negative half is the collision this table declines to guess at:
    /// DS5319 Rev 20 Table 5 (page 30) lists PB10 as `I2C2_SCL/USART3_TX(9)` —
    /// both in the DEFAULT column. Two `selector: None` routes on one pad are
    /// indistinguishable, so binding both would hand PB10 to whichever wiring
    /// function ran first. I²C wins by decision; USART3 stays dark and says so.
    #[test]
    fn the_config_built_f103_binds_both_i2c_instances_and_usart1_and_usart2() {
        let bound = bus_for("stm32f103").bound_pad_functions();
        for func in [
            "I2C1_SCL",
            "I2C1_SDA",
            "I2C2_SCL",
            "I2C2_SDA",
            "USART1_TX",
            "USART2_TX",
        ] {
            assert!(
                bound.contains(&func),
                "stm32f103 must bind {func} through from_config, got {bound:?}",
            );
        }
        assert!(
            !bound.contains(&"USART3_TX"),
            "USART3_TX shares PB10 with I2C2_SCL in the DS5319 Default column and \
             must stay unbound until AFIO_MAPR is decoded, got {bound:?}",
        );
    }

    /// The pad every bundled F103 sensor lab probes reports the I²C wire, and
    /// the pad the datasheet gives I²C only under a REMAP does not.
    ///
    /// Asserted through the pad read the analyzer samples: `logic_watch`
    /// answers `None` for an alternate-function pad no peripheral wire is bound
    /// to and `Some(level)` for one that is, so the two cases are directly
    /// distinguishable without reaching into the routing table.
    #[test]
    fn the_default_i2c_pads_carry_a_wire_and_the_remap_only_pad_does_not() {
        let mut machine = i2c_machine(vec![0]);
        configure_i2c(&mut machine);
        // PB8 is I2C1_SCL only under AFIO_MAPR.I2C1_REMAP (DS5319 page 33); its
        // DEFAULT function is TIM4_CH3. Put it in alternate-function mode — the
        // exact state a route bound there would wrongly answer for.
        set_pin_config(&mut machine, GPIOB_BASE, REMAP_SCL_PIN, AF_OPEN_DRAIN);
        let gpiob = machine
            .bus
            .find_peripheral_index_by_name("gpiob")
            .expect("gpiob registered by from_config");
        assert_eq!(
            machine.logic_watch(&[
                Some(LogicSource::pad(gpiob, SCL_PIN)),
                Some(LogicSource::pad(gpiob, SDA_PIN)),
                Some(LogicSource::pad(gpiob, REMAP_SCL_PIN)),
            ]),
            // PB6/PB7: an idle open-drain bus, so high. PB8: no wire at all.
            vec![Some(true), Some(true), None],
            "DS5319 Rev 20 Table 5: PB6/PB7 are I2C1 in the Default column \
             (page 32) and PB8 only in the Remap column (page 33). A wire on \
             PB8 means a remap row leaked in; no wire on PB6 means this gate \
             measures nothing",
        );
    }

    /// A pad taken back for plain GPIO must stop reading the bus. The regression
    /// this guards is the mirror image of a dark bus: a general-purpose output
    /// silently showing I²C traffic.
    #[test]
    fn a_plain_gpio_output_on_pb6_does_not_read_the_i2c_wire() {
        let mut machine = i2c_machine(vec![0]);
        configure_i2c(&mut machine);
        set_pin_config(&mut machine, GPIOB_BASE, SCL_PIN, GPIO_PUSH_PULL);
        let gpiob = machine
            .bus
            .find_peripheral_index_by_name("gpiob")
            .expect("gpiob registered by from_config");
        assert_eq!(
            machine.logic_watch(&[
                Some(LogicSource::pad(gpiob, SCL_PIN)),
                Some(LogicSource::pad(gpiob, SDA_PIN))
            ]),
            // PB6 falls back to the ODR latch (reset 0); PB7 is still AF.
            vec![Some(false), Some(true)],
            "CNF < 0b10 is not an alternate function, so PB6 must read its own \
             output latch and not the bus",
        );
    }

    // ── I²C waveform ────────────────────────────────────────────────────────

    #[test]
    fn logic_capture_sees_a_decodable_f103_i2c_waveform() {
        let mut machine = i2c_machine(vec![0]);
        configure_i2c(&mut machine);
        arm_i2c(&mut machine);

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
            "an F103 I²C transfer must put edges on PB6/PB7, not a flat trace",
        );
        assert_eq!(
            decode_i2c(&edges),
            vec![((SLAVE_ADDR << 1), true), (DATA_BYTE, true)],
            "the decoder must recover the addressed write the model performed",
        );
    }

    #[test]
    fn an_f103_transfer_to_an_absent_slave_shows_a_nacked_address() {
        // The failure a user most needs to SEE: the address went out and
        // nobody answered. 0x4A << 1 = 0x94, which reverses to 0x29.
        const ABSENT: u8 = 0x4A;
        let mut machine = i2c_machine(vec![0]);
        configure_i2c(&mut machine);
        arm_i2c(&mut machine);
        for _ in 0..40_000 {
            machine.step().unwrap();
        }

        start_and_address(&mut machine, ABSENT << 1);
        wait_sr1(&mut machine, SR1_AF);
        let cr1 = machine.bus.read_u32(CR1).unwrap();
        machine.bus.write_u32(CR1, cr1 | CR1_STOP).unwrap();
        machine.step().unwrap();

        assert_eq!(
            decode_i2c(&machine.logic_read_edges(0).edges),
            vec![((ABSENT << 1), false)],
            "the wire must show the address frame NACKed",
        );
    }

    #[test]
    fn an_f103_repeated_start_read_decodes_every_frame_that_crossed_the_bus() {
        // The standard sensor shape, and the one every bundled F103 lab runs:
        // write the register pointer, repeated START, read two bytes back, ACK
        // the first and NACK the last.
        let mut machine = i2c_machine(vec![READ_B0, READ_B1]);
        configure_i2c(&mut machine);
        arm_i2c(&mut machine);
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

        let cr1 = machine.bus.read_u32(CR1).unwrap();
        machine.bus.write_u32(CR1, cr1 | CR1_ACK).unwrap();
        start_and_address(&mut machine, (SLAVE_ADDR << 1) | 1);
        wait_sr1(&mut machine, SR1_ADDR);
        machine.bus.read_u32(SR1).unwrap();
        machine.bus.read_u32(SR2).unwrap();
        wait_sr1(&mut machine, SR1_RXNE);
        assert_eq!(
            machine.bus.read_u32(DR).unwrap() as u8,
            READ_B0,
            "first received byte",
        );

        let cr1 = machine.bus.read_u32(CR1).unwrap();
        machine.bus.write_u32(CR1, cr1 & !CR1_ACK).unwrap();
        wait_sr1(&mut machine, SR1_RXNE);
        assert_eq!(
            machine.bus.read_u32(DR).unwrap() as u8,
            READ_B1,
            "second received byte",
        );
        let cr1 = machine.bus.read_u32(CR1).unwrap();
        machine.bus.write_u32(CR1, cr1 | CR1_STOP).unwrap();
        machine.step().unwrap();

        assert_eq!(
            decode_i2c(&machine.logic_read_edges(0).edges),
            vec![
                ((SLAVE_ADDR << 1), true),
                (REG_PTR, true),
                ((SLAVE_ADDR << 1) | 1, true),
                (READ_B0, true),
                (READ_B1, false),
            ],
            "the wire must show write-pointer, repeated START, then both bytes \
             read back with the last one NACKed",
        );
    }

    // ── USART waveform ──────────────────────────────────────────────────────

    #[test]
    fn logic_capture_sees_a_decodable_f103_usart1_waveform_on_pa9() {
        let mut machine = uart_machine();
        // GPIOA is clock-gated in the F103 yaml (RCC_APB2ENR.IOPAEN), and so is
        // USART1 (USART1EN) — an ungated write is dropped, exactly as on silicon.
        machine
            .bus
            .write_u32(RCC_BASE + APB2ENR, IOPAEN | USART1EN)
            .unwrap();
        set_pin_config(&mut machine, GPIOA_BASE, TX_PIN, AF_PUSH_PULL);
        machine
            .bus
            .write_u32(USART1_BASE + USART_BRR, BRR_VALUE)
            .unwrap();

        let gpioa = machine
            .bus
            .find_peripheral_index_by_name("gpioa")
            .expect("gpioa registered by from_config");
        machine.logic_watch(&[Some(LogicSource::pad(gpioa, TX_PIN))]);

        for &byte in SERIAL_TEXT {
            machine.bus.write_u8(USART1_BASE + USART_DR, byte).unwrap();
        }
        // Ten bit periods per character, plus slack for the flush boundary.
        for _ in 0..SERIAL_TEXT.len() as u64 * 10 * BIT_TIME + 64 {
            machine.step().unwrap();
        }

        let edges = machine.logic_read_edges(0).edges;
        assert!(
            !edges.is_empty(),
            "an F103 USART1 transmission must put edges on PA9, not a flat trace",
        );
        assert_eq!(
            decode_serial(&edges, BIT_TIME),
            SERIAL_TEXT.to_vec(),
            "the decoder must recover the characters USART1 transmitted",
        );
    }

    /// PA9 must carry USART1 and nothing else, and the I²C pads must not have
    /// been handed a serial wire. Names the live routing, which is what a user
    /// sees in `gpio_routing`.
    #[test]
    fn each_f103_pad_names_the_peripheral_the_datasheet_gives_it() {
        let mut machine = uart_machine();
        machine
            .bus
            .write_u32(RCC_BASE + APB2ENR, IOPAEN | USART1EN)
            .unwrap();
        machine.bus.write_u32(RCC_BASE + APB1ENR, I2C1EN).unwrap();
        set_pin_config(&mut machine, GPIOA_BASE, TX_PIN, AF_PUSH_PULL);
        set_pin_config(&mut machine, GPIOB_BASE, SCL_PIN, AF_OPEN_DRAIN);
        set_pin_config(&mut machine, GPIOB_BASE, SDA_PIN, AF_OPEN_DRAIN);

        let named = |machine: &Machine<CortexM>, port: &str, pin: u8| -> Option<String> {
            let idx = machine.bus.find_peripheral_index_by_name(port).unwrap();
            machine.bus.peripherals[idx]
                .dev
                .gpio_routing(pin)
                .and_then(|r| r.func)
        };
        assert_eq!(
            named(&machine, "gpioa", TX_PIN).as_deref(),
            Some("USART1_TX"),
            "DS5319 page 31: PA9 Default = USART1_TX",
        );
        assert_eq!(
            named(&machine, "gpiob", SCL_PIN).as_deref(),
            Some("I2C1_SCL"),
            "DS5319 page 32: PB6 Default = I2C1_SCL. USART1_TX is its REMAP \
             function and must never claim it",
        );
        assert_eq!(
            named(&machine, "gpiob", SDA_PIN).as_deref(),
            Some("I2C1_SDA"),
            "DS5319 page 32: PB7 Default = I2C1_SDA",
        );
    }
}
