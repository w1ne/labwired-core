// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! End-to-end waveform test for a UART: select the UART function on a pad
//! through IO_BANK0 exactly as `gpio_set_function` does, arm the in-engine
//! logic analyzer on it, transmit through the data register, and assert the
//! captured edges decode back to the characters that were sent.
//!
//! The UART was the last bus on the board that a probe could not see. Serial
//! output existed only as console text and as rows in the bus trace; the pad
//! itself was whatever the SIO output latch happened to hold, so a scope on GP0
//! read a flat line while the firmware was talking.
//!
//! The decoder below shares no code with the model. It synchronises on the
//! falling start edge and samples each bit at its CENTRE, which is what a real
//! receiver does and what makes the test sensitive to bit ORDER: serial is
//! LSB-first, and narrating it MSB-first produces a trace that looks entirely
//! plausible and decodes to garbage.

#[cfg(test)]
mod rp2040_uart_waveform_tests {
    use crate::logic_capture::LogicEdge;
    use crate::logic_capture::LogicSource;
    use crate::peripherals::rp2040::io_bank0::{Rp2040IoBank0, GPIO_FUNC_UART};
    use crate::peripherals::rp2040::sio::Rp2040Sio;
    use crate::peripherals::uart::{Uart, UartRegisterLayout};
    use crate::{Bus, Machine};

    const IO_BANK0_BASE: u64 = 0x4001_4000;
    const SIO_BASE: u64 = 0xD000_0000;
    const UART0_BASE: u64 = 0x4003_4000;
    const UART1_BASE: u64 = 0x4003_8000;
    const RAM_BASE: u64 = 0x2000_0000;

    /// PL011 (ARM DDI 0183G): data register, and the two baud divisors.
    const UARTDR: u64 = 0x00;
    const UARTIBRD: u64 = 0x24;
    const UARTFBRD: u64 = 0x28;

    /// GP0 is UART0 TX on the RP2040, and the Pico's default serial pin.
    const TX_PIN: u8 = 0;
    /// GP2 is `uart0_cts` in the SVD — a UART function, but not one that
    /// carries a narrated waveform. The control case.
    const NON_UART_PIN: u8 = 2;
    const CH_TX: u32 = 0;

    /// 115200 baud from a 125 MHz clk_peri: 125e6 / (16 × 115200) = 67.8168,
    /// so IBRD = 67 and FBRD = round(0.8168 × 64) = 52. One bit is then
    /// (64 × 67 + 52) / 4 = 1085 clocks, and 125e6 / 1085 = 115207 baud.
    const IBRD: u32 = 67;
    const FBRD: u32 = 52;
    const BIT_TIME: u64 = 1085;

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
        let mut uart = Uart::new_with_layout(UartRegisterLayout::Pl011);
        // Keep the test's own stdout clean; the pad waveform is what is asserted.
        uart.set_sink(None, false);
        bus.add_peripheral("uart0", UART0_BASE, 0x1000, None, Box::new(uart));
        let mut uart1 = Uart::new_with_layout(UartRegisterLayout::Pl011);
        uart1.set_sink(None, false);
        bus.add_peripheral("uart1", UART1_BASE, 0x1000, None, Box::new(uart1));
        bus.wire_rp2040_uart_pads();

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

    /// `gpio_set_function(pin, GPIO_FUNC_UART)` plus `uart_set_baudrate`.
    fn configure(machine: &mut Machine<crate::cpu::CortexM>, pin: u8) {
        let bus = &mut machine.bus;
        bus.write_u32(IO_BANK0_BASE + ctrl_offset(pin), GPIO_FUNC_UART)
            .unwrap();
        bus.write_u32(UART0_BASE + UARTIBRD, IBRD).unwrap();
        bus.write_u32(UART0_BASE + UARTFBRD, FBRD).unwrap();
    }

    /// Write the characters into the data register, then let the engine run
    /// long enough for the line to have carried them.
    ///
    /// The second half is not test scaffolding — it is the point. Firmware
    /// hands this model a whole string within a few cycles because TX always
    /// reports empty, but the wire needs ten bit periods per character. The
    /// narration waits for that time to pass before publishing, so a test that
    /// read the trace immediately would be reading a line that has not
    /// physically finished talking.
    fn transmit(machine: &mut Machine<crate::cpu::CortexM>, text: &[u8]) {
        for &byte in text {
            machine.bus.write_u8(UART0_BASE + UARTDR, byte).unwrap();
        }
        let wire_cycles = text.len() as u64 * 10 * BIT_TIME;
        for _ in 0..wire_cycles + 16 {
            machine.step().unwrap();
        }
    }

    /// An INDEPENDENT asynchronous-serial decoder.
    ///
    /// Rebuilds the line level over time from the edges, then does what a
    /// receiver does: find a falling edge, confirm the start bit is still low
    /// half a bit later, and sample the following eight bits at their centres,
    /// LSB first.
    fn decode(edges: &[LogicEdge], bit_time: u64) -> Vec<u8> {
        let timeline: Vec<(u64, bool)> = edges
            .iter()
            .filter(|e| e.ch == CH_TX)
            .map(|e| (e.cycle, e.value))
            .collect();
        // The line idles at mark (high) before the first recorded transition.
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
            // A start bit is a falling edge on an idle line, and no falling
            // edge inside a character already being decoded counts.
            if level || cycle < cursor {
                continue;
            }
            if level_at(cycle + bit_time / 2) {
                continue; // a glitch, not a start bit
            }
            let mut byte = 0u8;
            for index in 0..8u64 {
                let sample = cycle + bit_time / 2 + bit_time * (index + 1);
                if level_at(sample) {
                    byte |= 1 << index; // LSB first
                }
            }
            // The stop bit must be mark, or this was not a character.
            if !level_at(cycle + bit_time / 2 + bit_time * 9) {
                continue;
            }
            bytes.push(byte);
            cursor = cycle + bit_time * 10;
        }
        bytes
    }

    /// Every TX pad in the routing table, with the instance the SVD assigns.
    /// Kept here rather than imported so a typo in the model's table is a
    /// DISAGREEMENT between two hand-entered lists, not a shared mistake.
    const TX_PADS: &[(u8, usize)] = &[
        (0, 0),
        (4, 1),
        (8, 1),
        (12, 0),
        (16, 0),
        (20, 1),
        (24, 1),
        (28, 0),
    ];

    #[test]
    fn every_pad_in_the_table_carries_its_own_instance_and_no_other() {
        // The whole justification for transcribing this table by hand is that
        // it cannot be derived — GP8/GP9 are UART1 where the surrounding
        // pattern says UART0. That claim was previously ungated: no test put
        // uart1 on the bus at all, so flipping any instance column stayed green.
        //
        // Here uart0 alone transmits. Every uart0 pad must carry it and every
        // uart1 pad must stay silent, which pins the instance column of all
        // eight rows in both directions.
        let mut machine = machine();
        for &(pin, _) in TX_PADS {
            machine
                .bus
                .write_u32(IO_BANK0_BASE + ctrl_offset(pin), GPIO_FUNC_UART)
                .unwrap();
        }
        for base in [UART0_BASE, UART1_BASE] {
            machine.bus.write_u32(base + UARTIBRD, IBRD).unwrap();
            machine.bus.write_u32(base + UARTFBRD, FBRD).unwrap();
        }

        let sio_idx = machine.bus.find_peripheral_index_by_name("sio").unwrap();
        let watch: Vec<Option<LogicSource>> = TX_PADS
            .iter()
            .map(|&(pin, _)| Some(LogicSource::pad(sio_idx, pin)))
            .collect();
        let initial = machine.logic_watch(&watch);
        assert!(
            initial.iter().all(|&level| level == Some(true)),
            "every routed TX pad idles at mark, not at the SIO latch: {initial:?}",
        );

        for _ in 0..20_000 {
            machine.step().unwrap();
        }
        transmit(&mut machine, b"Hi");

        let edges = machine.logic_read_edges(0).edges;
        for (channel, &(pin, instance)) in TX_PADS.iter().enumerate() {
            let saw = edges.iter().any(|e| e.ch == channel as u32);
            assert_eq!(
                saw,
                instance == 0,
                "GP{pin} belongs to uart{instance}; uart0 was the one transmitting",
            );
        }
    }

    #[test]
    fn logic_capture_sees_a_decodable_uart_waveform() {
        let mut machine = machine();
        configure(&mut machine, TX_PIN);

        let sio_idx = machine.bus.find_peripheral_index_by_name("sio").unwrap();
        let initial = machine.logic_watch(&[Some(LogicSource::pad(sio_idx, TX_PIN))]);
        assert_eq!(
            initial,
            vec![Some(true)],
            "an idle serial line rests at mark, so a start bit is a falling edge",
        );

        for _ in 0..20_000 {
            machine.step().unwrap();
        }
        transmit(&mut machine, b"Hi!\n");

        let edges = machine.logic_read_edges(0).edges;
        assert!(
            !edges.is_empty(),
            "a UART transmission must put edges on its routed pad, not a flat trace",
        );
        assert_eq!(
            decode(&edges, BIT_TIME),
            b"Hi!\n".to_vec(),
            "the wire must carry the characters the firmware transmitted",
        );
    }

    #[test]
    fn a_second_message_lands_after_the_first_and_not_over_it() {
        // Firmware prints more than once. Each burst is narrated separately, so
        // the second must begin after the first finished rather than reaching
        // back across cycles the first already painted — which would splice the
        // two into characters neither call transmitted.
        let mut machine = machine();
        configure(&mut machine, TX_PIN);
        let sio_idx = machine.bus.find_peripheral_index_by_name("sio").unwrap();
        machine.logic_watch(&[Some(LogicSource::pad(sio_idx, TX_PIN))]);
        for _ in 0..20_000 {
            machine.step().unwrap();
        }
        transmit(&mut machine, b"one");
        transmit(&mut machine, b"two");

        let edges = machine.logic_read_edges(0).edges;
        assert_eq!(
            decode(&edges, BIT_TIME),
            b"onetwo".to_vec(),
            "both messages must be on the wire, in order and intact",
        );
    }

    #[test]
    fn a_pad_that_carries_no_tx_function_shows_no_serial_traffic() {
        // This gates the TABLE, not FUNCSEL: GP2 is handed to the UART exactly
        // like the real TX pad, and must still stay silent, because GP2 is
        // uart0_CTS. Route a pad the SVD does not name TX or RX and this is
        // what catches it.
        let mut machine = machine();
        configure(&mut machine, TX_PIN);
        machine
            .bus
            .write_u32(IO_BANK0_BASE + ctrl_offset(NON_UART_PIN), GPIO_FUNC_UART)
            .unwrap();

        let sio_idx = machine.bus.find_peripheral_index_by_name("sio").unwrap();
        // Watch BOTH pads: the TX channel proves the machinery is live in this
        // very fixture, so the control channel's silence means the table, not a
        // broken setup. Without that, `is_empty` is satisfied by any failure.
        machine.logic_watch(&[
            Some(LogicSource::pad(sio_idx, TX_PIN)),
            Some(LogicSource::pad(sio_idx, NON_UART_PIN)),
        ]);
        for _ in 0..20_000 {
            machine.step().unwrap();
        }
        transmit(&mut machine, b"Hi!\n");

        let edges = machine.logic_read_edges(0).edges;
        assert!(
            edges.iter().any(|e| e.ch == 0),
            "the routed TX pad must be carrying traffic, or this proves nothing",
        );
        assert!(
            !edges.iter().any(|e| e.ch == 1),
            "the UART function on a non-TX pad must not show the serial line",
        );
    }

    #[test]
    fn the_line_runs_at_the_baud_rate_the_divisors_program() {
        let mut machine = machine();
        configure(&mut machine, TX_PIN);
        let sio_idx = machine.bus.find_peripheral_index_by_name("sio").unwrap();
        machine.logic_watch(&[Some(LogicSource::pad(sio_idx, TX_PIN))]);
        for _ in 0..20_000 {
            machine.step().unwrap();
        }
        // 0x55 alternates 1/0, so every bit boundary is a real transition and
        // the gaps between them ARE the bit period.
        transmit(&mut machine, &[0x55]);

        let cycles: Vec<u64> = machine
            .logic_read_edges(0)
            .edges
            .iter()
            .filter(|e| e.ch == CH_TX)
            .map(|e| e.cycle)
            .collect();
        assert!(
            cycles.len() >= 9,
            "0x55 alternates on every bit: {cycles:?}"
        );
        let gaps: Vec<u64> = cycles.windows(2).map(|pair| pair[1] - pair[0]).collect();
        assert!(
            gaps.iter().all(|&g| g == BIT_TIME),
            "every bit should last {BIT_TIME} cycles, got {gaps:?}",
        );
    }

    #[test]
    fn a_divisor_that_was_never_programmed_publishes_nothing() {
        // Silence beats a confident wrong answer: with no baud divisor written
        // there is no timebase, and narrating at a made-up rate would give a
        // trace that measures a frequency the firmware never asked for.
        let mut machine = machine();
        machine
            .bus
            .write_u32(IO_BANK0_BASE + ctrl_offset(TX_PIN), GPIO_FUNC_UART)
            .unwrap();
        let sio_idx = machine.bus.find_peripheral_index_by_name("sio").unwrap();
        machine.logic_watch(&[Some(LogicSource::pad(sio_idx, TX_PIN))]);
        for _ in 0..20_000 {
            machine.step().unwrap();
        }
        transmit(&mut machine, b"Hi!\n");

        assert!(
            machine.logic_read_edges(0).edges.is_empty(),
            "no programmed baud rate means no honest timebase, so no waveform",
        );
    }

    #[test]
    fn the_baud_divisors_read_back_what_firmware_wrote() {
        // pico-sdk's uart_set_baudrate writes IBRD/FBRD and then reads them to
        // report the baud it achieved; a model that dropped them returned zero.
        let mut machine = machine();
        configure(&mut machine, TX_PIN);
        assert_eq!(machine.bus.read_u32(UART0_BASE + UARTIBRD).unwrap(), IBRD);
        assert_eq!(machine.bus.read_u32(UART0_BASE + UARTFBRD).unwrap(), FBRD);
    }
}
