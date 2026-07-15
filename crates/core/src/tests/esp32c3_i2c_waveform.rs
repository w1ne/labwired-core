// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! End-to-end waveform test for the ESP32-C3 bit-level I²C engine: arm the
//! in-engine logic analyzer on the GPIO-matrix-routed SDA/SCL pads, run an
//! SSD1306 transaction through the controller's registers, and assert the
//! captured edge stream is a valid I²C waveform whose clocked byte pattern
//! matches the byte-level bus-trace monitor. The waveform is sampled through
//! the normal `read_gpio_pad` path — nothing is synthesized into the ring.

#[cfg(test)]
mod esp32c3_i2c_waveform_tests {
    use crate::bus::bus_trace::{BusPayload, I2cSym};
    use crate::cpu::CortexM;
    use crate::logic_capture::LogicEdge;
    use crate::peripherals::components::Ssd1306;
    use crate::{Bus, Machine};
    use std::collections::BTreeMap;

    const RAM_BASE: u64 = 0x2000_0000;
    const I2C_BASE: u64 = 0x6001_3000;
    const GPIO_BASE: u64 = 0x6000_4000;

    // Board routing chosen by the "firmware": SDA on pad 5, SCL on pad 6.
    const SDA_PIN: u8 = 5;
    const SCL_PIN: u8 = 6;
    /// GPIO-matrix output signal indices (esp-idf gpio_sig_map.h).
    const SIG_I2CEXT0_SCL: u32 = 53;
    const SIG_I2CEXT0_SDA: u32 = 54;

    const ENABLE_W1TS: u64 = 0x24;
    const FUNC_IN_SEL: u64 = 0x154;
    const FUNC_OUT_SEL: u64 = 0x554;
    const MATRIX_INPUT_SELECT: u32 = 1 << 6;

    const REG_CTR: u64 = 0x04;
    const REG_DATA: u64 = 0x1C;
    const REG_INT_RAW: u64 = 0x20;
    const REG_CMD0: u64 = 0x58;
    const INT_TRANS_COMPLETE: u32 = 1 << 7;

    const CH_SDA: u32 = 0;
    const CH_SCL: u32 = 1;

    /// The wire bytes the transaction clocks: addr+W (0x3C<<1), SSD1306 data
    /// control byte, one framebuffer byte.
    const WIRE_BYTES: [u8; 3] = [0x78, 0x40, 0xA5];

    /// Build a machine whose bus carries the C3 I²C0 controller (with a traced
    /// SSD1306 slave) and the C3 GPIO with pads 5/6 matrix-routed to
    /// I2CEXT0_SDA/SCL. The CPU chews a harmless NOP loop so `step()` advances
    /// cycles deterministically.
    fn machine() -> Machine<CortexM> {
        let mut bus = crate::bus::SystemBus::new();
        let (cpu, _nvic) = crate::system::cortex_m::configure_cortex_m(&mut bus);
        bus.add_peripheral(
            "gpio",
            GPIO_BASE,
            0x1000,
            None,
            Box::new(crate::peripherals::esp32c3::gpio::Esp32c3Gpio::new()),
        );
        bus.add_peripheral(
            "i2c0",
            I2C_BASE,
            0x1000,
            None,
            Box::new(crate::peripherals::esp32c3::i2c::Esp32c3I2c::new()),
        );
        // Through the single traced choke point, same as a config-built bus.
        // The fixture's physical OLED is wired to the same matrix pads its
        // firmware setup below will select.
        let route = BTreeMap::from([
            ("sda".to_string(), format!("GPIO{SDA_PIN}")),
            ("scl".to_string(), format!("GPIO{SCL_PIN}")),
        ]);
        bus.attach_i2c_slave_with_route("i2c0", Box::new(Ssd1306::new(0x3C)), Some(&route))
            .unwrap();
        bus.wire_esp32c3_i2c_pads();

        let mut machine = Machine::new(cpu, bus);
        // NOP slab (`movs r0, #0`) with a Thumb `b` back to the start at the
        // end, so the CPU can run for tens of thousands of steps.
        for i in 0..1022u64 {
            let byte = if i % 2 == 0 { 0x00 } else { 0x20 };
            machine.bus.write_u8(RAM_BASE + i, byte).unwrap();
        }
        // b <base>: T1 encoding 0xE000 | imm11, target = addr + 4 + 2*imm11;
        // at base+1022 → imm11 = -513 = 0x5FF → 0xE5FF.
        machine.bus.write_u8(RAM_BASE + 1022, 0xFF).unwrap();
        machine.bus.write_u8(RAM_BASE + 1023, 0xE5).unwrap();
        machine.cpu.pc = RAM_BASE as u32;
        machine
    }

    /// Route pads through the GPIO output matrix and program 100 kHz-style
    /// I²C timing (what esp-hal would write for a 40 MHz source clock).
    fn configure(machine: &mut Machine<CortexM>) {
        let bus = &mut machine.bus;
        // Output drivers on + matrix routing.
        bus.write_u32(GPIO_BASE + ENABLE_W1TS, (1 << SDA_PIN) | (1 << SCL_PIN))
            .unwrap();
        bus.write_u32(
            GPIO_BASE + FUNC_OUT_SEL + (SDA_PIN as u64) * 4,
            SIG_I2CEXT0_SDA,
        )
        .unwrap();
        bus.write_u32(
            GPIO_BASE + FUNC_OUT_SEL + (SCL_PIN as u64) * 4,
            SIG_I2CEXT0_SCL,
        )
        .unwrap();
        bus.write_u32(
            GPIO_BASE + FUNC_IN_SEL + u64::from(SIG_I2CEXT0_SDA) * 4,
            MATRIX_INPUT_SELECT | u32::from(SDA_PIN),
        )
        .unwrap();
        bus.write_u32(
            GPIO_BASE + FUNC_IN_SEL + u64::from(SIG_I2CEXT0_SCL) * 4,
            MATRIX_INPUT_SELECT | u32::from(SCL_PIN),
        )
        .unwrap();
        // Timing: low = 200, high = 181 + 19 wait-high = 200 module ticks →
        // 400-tick SCL period at 40 MHz = 100 kHz.
        bus.write_u32(I2C_BASE, 199).unwrap(); // SCL_LOW_PERIOD
        bus.write_u32(I2C_BASE + 0x38, 180 | (19 << 9)).unwrap(); // SCL_HIGH_PERIOD
        bus.write_u32(I2C_BASE + 0x30, 29).unwrap(); // SDA_HOLD = 30 ticks
        bus.write_u32(I2C_BASE + 0x40, 199).unwrap(); // SCL_START_HOLD
        bus.write_u32(I2C_BASE + 0x44, 199).unwrap(); // SCL_RSTART_SETUP
        bus.write_u32(I2C_BASE + 0x4C, 199).unwrap(); // SCL_STOP_SETUP
        bus.write_u32(I2C_BASE + 0x48, 199).unwrap(); // SCL_STOP_HOLD
    }

    /// Kick RSTART; WRITE 3 (addr, control, data); STOP.
    fn kick_transaction(machine: &mut Machine<CortexM>) {
        let bus = &mut machine.bus;
        let cmd = |opcode: u32, byte_num: u32| (opcode << 11) | byte_num;
        bus.write_u32(I2C_BASE + REG_CMD0, cmd(6, 0)).unwrap(); // RSTART
        bus.write_u32(I2C_BASE + REG_CMD0 + 4, cmd(1, 3)).unwrap(); // WRITE 3
        bus.write_u32(I2C_BASE + REG_CMD0 + 8, cmd(2, 0)).unwrap(); // STOP
        for b in WIRE_BYTES {
            bus.write_u32(I2C_BASE + REG_DATA, b as u32).unwrap();
        }
        bus.write_u32(I2C_BASE + REG_CTR, 1 << 5).unwrap(); // TRANS_START
    }

    /// Pad level of `ch` at engine cycle `cycle`, reconstructed from the
    /// initial level and the recorded transitions.
    fn level_at(initial: bool, edges: &[LogicEdge], ch: u32, cycle: u64) -> bool {
        let mut level = initial;
        for e in edges {
            if e.ch == ch && e.cycle <= cycle {
                level = e.value;
            }
        }
        level
    }

    #[test]
    fn logic_capture_sees_real_i2c_waveform_matching_bus_trace() {
        let mut machine = machine();
        configure(&mut machine);

        let gpio_idx = machine
            .bus
            .find_peripheral_index_by_name("gpio")
            .expect("gpio registered");
        let initial = machine.logic_watch(&[Some((gpio_idx, SDA_PIN)), Some((gpio_idx, SCL_PIN))]);
        assert_eq!(
            initial,
            vec![Some(true), Some(true)],
            "idle open-drain bus reads high on both matrix-routed pads"
        );

        kick_transaction(&mut machine);

        // ~46k cycles of wire time at 100 kHz; run with headroom.
        let mut done = false;
        for _ in 0..200_000 {
            machine.step().unwrap();
            if machine.bus.read_u32(I2C_BASE + REG_INT_RAW).unwrap() & INT_TRANS_COMPLETE != 0 {
                done = true;
                break;
            }
        }
        assert!(done, "transaction must complete within the step budget");

        let edges = machine.logic_read_edges(0).edges;
        assert!(
            !edges.is_empty(),
            "hardware-I2C pads must carry edges, not a flat trace"
        );
        assert!(edges.iter().any(|e| e.ch == CH_SDA), "SDA edges present");
        assert!(edges.iter().any(|e| e.ch == CH_SCL), "SCL edges present");

        // ── START condition: the first SDA edge falls while SCL is high, and
        // precedes every SCL edge.
        let first_sda = edges.iter().find(|e| e.ch == CH_SDA).unwrap();
        assert!(
            !first_sda.value,
            "first SDA transition must be the START fall"
        );
        assert!(
            level_at(true, &edges, CH_SCL, first_sda.cycle),
            "SDA must fall while SCL is high (START)"
        );
        let first_scl = edges.iter().find(|e| e.ch == CH_SCL).unwrap();
        assert!(
            first_sda.cycle < first_scl.cycle,
            "START must precede all SCL activity"
        );

        // ── Clocked bits: sample SDA at every SCL rising edge. 27 data-bit
        // rises (3 bytes x 9 bits) plus the one STOP-setup rise.
        let rising: Vec<u64> = edges
            .iter()
            .filter(|e| e.ch == CH_SCL && e.value)
            .map(|e| e.cycle)
            .collect();
        assert_eq!(
            rising.len(),
            WIRE_BYTES.len() * 9 + 1,
            "3 bytes x 9 bits + STOP-setup rise"
        );
        let bits: Vec<bool> = rising[..WIRE_BYTES.len() * 9]
            .iter()
            .map(|&c| level_at(true, &edges, CH_SDA, c))
            .collect();
        let mut wire_bytes = Vec::new();
        let mut acks = Vec::new();
        for chunk in bits.chunks(9) {
            let byte = chunk[..8]
                .iter()
                .fold(0u8, |acc, &b| (acc << 1) | u8::from(b));
            wire_bytes.push(byte);
            acks.push(chunk[8]);
        }
        assert_eq!(
            wire_bytes,
            WIRE_BYTES.to_vec(),
            "the clocked SDA byte pattern must be the programmed transaction"
        );
        assert_eq!(
            acks,
            vec![false; WIRE_BYTES.len()],
            "the SSD1306 must ACK (SDA low) every byte"
        );

        // ── STOP condition: the last SDA edge rises while SCL is high, after
        // the final SCL rise.
        let last_sda = edges.iter().rfind(|e| e.ch == CH_SDA).unwrap();
        assert!(last_sda.value, "last SDA transition must be the STOP rise");
        assert!(
            level_at(true, &edges, CH_SCL, last_sda.cycle),
            "SDA must rise while SCL is high (STOP)"
        );
        assert!(
            last_sda.cycle >= *rising.last().unwrap(),
            "STOP follows the final SCL rise"
        );

        // ── Waveform and byte-level bus monitor must agree: the same bytes,
        // in the same order, on the same bus.
        let events = machine.bus.bus_trace_snapshot();
        let traced: Vec<(bool, u8)> = events
            .iter()
            .filter(|e| e.bus == "i2c0")
            .filter_map(|e| match e.payload {
                BusPayload::I2c {
                    kind: I2cSym::AddrWrite,
                    byte,
                    ..
                } => Some((true, byte)),
                BusPayload::I2c {
                    kind: I2cSym::Data,
                    byte,
                    ..
                } => Some((false, byte)),
                _ => None,
            })
            .collect();
        assert_eq!(
            traced,
            vec![(true, 0x78), (false, 0x40), (false, 0xA5)],
            "bus monitor must decode the same address + data bytes the waveform clocked"
        );
    }

    /// Determinism: identical run, identical edge stream.
    #[test]
    fn waveform_capture_is_deterministic() {
        let run = || {
            let mut machine = machine();
            configure(&mut machine);
            let gpio_idx = machine
                .bus
                .find_peripheral_index_by_name("gpio")
                .expect("gpio registered");
            machine.logic_watch(&[Some((gpio_idx, SDA_PIN)), Some((gpio_idx, SCL_PIN))]);
            kick_transaction(&mut machine);
            for _ in 0..80_000 {
                machine.step().unwrap();
            }
            machine.logic_read_edges(0).edges
        };
        assert_eq!(
            run(),
            run(),
            "same firmware + watch => byte-identical edges"
        );
    }
}
