// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! End-to-end waveform test for an STM32 USART: route PB10 to AF7 exactly as a
//! HAL MSP init does, arm the in-engine logic analyzer on it, transmit through
//! TDR, and assert the captured edges decode back to the characters sent.
//!
//! PB10 is USART3_TX (DS10198 Table 17) — a pad a user actually probes when
//! they ask "is my board printing?". It carried nothing before: serial output
//! existed as console text only, and the pad read as the idle GPIO latch. Its
//! AF nibble also lives in AFR**H** rather than AFRL, which is the half of the
//! selector decode a pad below 8 never reaches.
//!
//! USART3 and GPIO**B** rather than instance 1 and GPIOA: the default test bus
//! already ships a `uart1` and an F1-layout `gpioa`, and a second peripheral of
//! either name is shadowed by it rather than replacing it — the pad routes bind
//! to the pre-existing instance and the new one narrates to nothing.
//!
//! The decoder shares no code with the model — it synchronises on the falling
//! start edge and samples each bit at its centre, LSB first, as a receiver
//! does. The waveform reaches it through the normal `read_gpio_pad` /
//! pad-route path; nothing is synthesized into the capture ring by the test.

#[cfg(test)]
mod stm32_uart_waveform_tests {
    use crate::cpu::CortexM;
    use crate::logic_capture::LogicEdge;
    use crate::logic_capture::LogicSource;
    use crate::peripherals::gpio::{GpioPort, GpioRegisterLayout};
    use crate::peripherals::uart::{Uart, UartRegisterLayout};
    use crate::{Bus, Machine};

    const RAM_BASE: u64 = 0x2000_0000;
    const GPIOB_BASE: u64 = 0x4001_0C00;
    const USART3_BASE: u64 = 0x4000_4800;
    const USART2_BASE: u64 = 0x4000_4400;
    const GPIOC_BASE: u64 = 0x4001_1000;
    const GPIOD_BASE: u64 = 0x4001_1400;

    const MODER: u64 = 0x00;
    const AFRL: u64 = 0x20;
    const AFRH: u64 = 0x24;

    /// USARTv2 register map: BRR holds USARTDIV, TDR transmits.
    const BRR: u64 = 0x0C;
    const TDR: u64 = 0x28;

    /// PB10 = USART3_TX on AF7 (DS10198 Table 17).
    const TX_PIN: u8 = 10;
    /// PB3 carries no USART function at all — the control case.
    const NON_UART_PIN: u8 = 3;
    const CH_TX: u32 = 0;

    /// 115200 baud from an 80 MHz PCLK with OVER16: USARTDIV = 80e6 / 115200 =
    /// 694 (0x2B6), and the divisor IS one bit period in peripheral clocks.
    const USARTDIV: u32 = 694;
    const BIT_TIME: u64 = 694;

    /// CR1 lives at 0x00 on the V2 map; OVER8 is bit 15.
    const CR1: u64 = 0x00;
    const CR1_OVER8: u32 = 1 << 15;

    fn machine() -> Machine<CortexM> {
        let mut bus = crate::bus::SystemBus::new();
        let (cpu, _nvic) = crate::system::cortex_m::configure_cortex_m(&mut bus);
        // A V2 GPIOB: the AFRL nibble is what routes the pad to the USART.
        bus.add_peripheral(
            "gpiob",
            GPIOB_BASE,
            0x400,
            None,
            Box::new(GpioPort::new_with_layout(GpioRegisterLayout::Stm32V2)),
        );
        for (name, base) in [("gpioc", GPIOC_BASE), ("gpiod", GPIOD_BASE)] {
            bus.add_peripheral(
                name,
                base,
                0x400,
                None,
                Box::new(GpioPort::new_with_layout(GpioRegisterLayout::Stm32V2)),
            );
        }
        for (name, base) in [("usart3", USART3_BASE), ("usart2", USART2_BASE)] {
            let mut uart = Uart::new_with_layout(UartRegisterLayout::Stm32V2);
            // Keep the test's own stdout clean; the pad waveform is asserted.
            uart.set_sink(None, false);
            bus.add_peripheral(name, base, 0x400, None, Box::new(uart));
        }
        bus.wire_stm32_uart_pads();

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

    /// Put PB10 in alternate-function mode on AF7 and set the baud divisor, as
    /// the HAL's MSP init plus `UART_SetConfig` do.
    fn configure(machine: &mut Machine<CortexM>, pin: u8) {
        let bus = &mut machine.bus;
        bus.write_u32(GPIOB_BASE + MODER, 0b10 << (pin * 2))
            .unwrap();
        // Pads 8..15 select their AF through AFRH, indexed from pin 8.
        bus.write_u32(GPIOB_BASE + AFRH, 7 << ((pin - 8) * 4))
            .unwrap();
        bus.write_u32(USART3_BASE + BRR, USARTDIV).unwrap();
    }

    /// Write the characters into TDR, then let the engine run long enough for
    /// the line to have carried them — ten bit periods per character.
    fn transmit(machine: &mut Machine<CortexM>, text: &[u8]) {
        for &byte in text {
            machine.bus.write_u8(USART3_BASE + TDR, byte).unwrap();
        }
        for _ in 0..text.len() as u64 * 10 * BIT_TIME + 16 {
            machine.step().unwrap();
        }
    }

    /// An INDEPENDENT asynchronous-serial decoder: sync on the falling start
    /// edge, sample each bit at its centre, LSB first.
    fn decode(edges: &[LogicEdge], bit_time: u64) -> Vec<u8> {
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

    /// Table rows reachable in this fixture: `(port, pin, instance)`.
    ///
    /// Instance 1 and every port-`a` row are absent, and not by choice — the
    /// default Cortex-M test bus already registers a `uart1` and an F1-layout
    /// `gpioa`, and a second peripheral of either name is shadowed rather than
    /// replacing it, so those rows cannot be reached from here at all. They
    /// remain untested. Kept as a second hand-entered list so a typo in the
    /// model's table is a disagreement, not a shared mistake.
    const TX_PADS: &[(&str, u64, u8, u8)] = &[
        ("gpiob", GPIOB_BASE, 10, 3),
        ("gpioc", GPIOC_BASE, 10, 3),
        ("gpiod", GPIOD_BASE, 8, 3),
        ("gpiod", GPIOD_BASE, 5, 2),
    ];

    #[test]
    fn every_reachable_pad_carries_its_own_instance_and_no_other() {
        // USART3 alone transmits. Its pads must carry it and USART2's must stay
        // silent, which pins the instance AND port columns of four rows in both
        // directions. Previously one row of seven was exercised.
        //
        // One pad at a time, deliberately: a cell's tap registration is
        // REPLACED per port, not unioned, so watching two pads of one wire
        // across two ports records only the last. Real silicon does allow
        // USART3_TX out of PB10 and PC10 at once; the analyzer would show one
        // of them. That limit is noted on `PadRoutes::clear_taps` — this gate
        // sidesteps it rather than hiding it.
        for &(port, base, pin, instance) in TX_PADS {
            let mut machine = machine();
            let afr = if pin < 8 { AFRL } else { AFRH };
            let moder = machine.bus.read_u32(base + MODER).unwrap();
            machine
                .bus
                .write_u32(base + MODER, moder | (0b10 << (pin * 2)))
                .unwrap();
            let cur = machine.bus.read_u32(base + afr).unwrap();
            machine
                .bus
                .write_u32(base + afr, cur | (7 << (u64::from(pin % 8) * 4)))
                .unwrap();
            for uart in [USART2_BASE, USART3_BASE] {
                machine.bus.write_u32(uart + BRR, USARTDIV).unwrap();
            }

            let idx = machine.bus.find_peripheral_index_by_name(port).unwrap();
            assert_eq!(
                machine.logic_watch(&[Some(LogicSource::pad(idx, pin))]),
                vec![Some(true)],
                "{port} pin {pin} must idle at mark, not at the GPIO latch",
            );
            for _ in 0..20_000 {
                machine.step().unwrap();
            }
            transmit(&mut machine, b"Hi");

            let carried = !machine.logic_read_edges(0).edges.is_empty();
            assert_eq!(
                carried,
                instance == 3,
                "{port} pin {pin} belongs to USART{instance}; USART3 was transmitting",
            );
        }
    }

    #[test]
    fn another_port_holding_the_same_wire_does_not_disarm_the_tap() {
        // A controller's wire reaches pads on SEVERAL ports — USART3 comes out
        // on PB10, PC10 or PD8 — so one PadLines cell is held by three
        // `PadRoutes`, one per port. `logic_watch` offers EVERY peripheral its
        // slice of the watch set, and the ports with nothing watched used to
        // clear the shared cell's tap, wiping the registration the watched port
        // had just installed.
        //
        // It failed silently in the worst way: the pad still READ correctly, so
        // the idle level and every level assertion held, and only the trace was
        // empty. This fixture is the minimum that reproduces it — the sibling
        // I²C gates never did, because they put a single GPIO port on the bus.
        let mut machine = machine();
        configure(&mut machine, TX_PIN);

        let gpiob_idx = machine.bus.find_peripheral_index_by_name("gpiob").unwrap();
        let gpioc_idx = machine.bus.find_peripheral_index_by_name("gpioc").unwrap();
        let gpiod_idx = machine.bus.find_peripheral_index_by_name("gpiod").unwrap();
        assert!(
            gpioc_idx > gpiob_idx && gpiod_idx > gpiob_idx,
            "the ports that hold the wire without watching it must be visited \
             AFTER the one that does, or this reproduces nothing",
        );

        machine.logic_watch(&[Some(LogicSource::pad(gpiob_idx, TX_PIN))]);
        for _ in 0..20_000 {
            machine.step().unwrap();
        }
        transmit(&mut machine, b"Hi");

        assert_eq!(
            decode(&machine.logic_read_edges(0).edges, BIT_TIME),
            b"Hi".to_vec(),
            "the tap survives ports that share the wire but watch none of it",
        );
    }

    /// STM32L0 puts USART2_TX on PA2 at **AF4**, not AF7. `wire_stm32_uart_pads`
    /// selects the AF table from each GPIO port's MMIO base: the L0 IOPORT bank
    /// is `0x5000_xxxx` (stm32l073.yaml); F4/L4/H5 AHB2 GPIO is `0x4800_xxxx`.
    /// An AF7-only V2 table leaves an L0 pad on the GPIO latch while USART TX
    /// narrates on its wire.
    ///
    /// Built on [`SystemBus::empty`] so the default F1 `gpioa` at `0x4001_0800`
    /// cannot shadow the L0 window by name (a second `gpioa` is invisible to
    /// `find_peripheral_index_by_name` and to the wire planner).
    #[test]
    fn stm32l0_usart2_tx_on_pa2_af4_carries_a_decodable_waveform() {
        use crate::memory::LinearMemory;

        const GPIOA_L0: u64 = 0x5000_0000;
        const USART2: u64 = 0x4000_4400;
        const PA2: u8 = 2;
        // Bit-asymmetric payload (not LSB-first palindromes like 0xA5/0x00).
        const PAYLOAD: &[u8] = b"Lw\x12\x34";

        let mut bus = crate::bus::SystemBus::empty();
        bus.ram = LinearMemory::new(1024 * 1024, RAM_BASE);
        let (cpu, _nvic) = crate::system::cortex_m::configure_cortex_m(&mut bus);
        bus.add_peripheral(
            "gpioa",
            GPIOA_L0,
            0x400,
            None,
            Box::new(GpioPort::new_with_layout(GpioRegisterLayout::Stm32V2)),
        );
        let mut uart = Uart::new_with_layout(UartRegisterLayout::Stm32V2);
        uart.set_sink(None, false);
        bus.add_peripheral("uart2", USART2, 0x400, None, Box::new(uart));
        bus.wire_stm32_uart_pads();

        assert_eq!(
            bus.bound_pad_functions(),
            vec!["USART2_TX"],
            "L0 IOPORT base must install the AF4 USART2_TX route, not the AF7 V2 table",
        );
        assert_eq!(
            bus.peripherals[bus.find_peripheral_index_by_name("gpioa").unwrap()].base,
            GPIOA_L0,
            "the only gpioa on this bus is the L0 window",
        );

        let mut machine = Machine::new(cpu, bus);
        for i in 0..1022u64 {
            let byte = if i % 2 == 0 { 0x00 } else { 0x20 };
            machine.bus.write_u8(RAM_BASE + i, byte).unwrap();
        }
        machine.bus.write_u8(RAM_BASE + 1022, 0xFF).unwrap();
        machine.bus.write_u8(RAM_BASE + 1023, 0xE5).unwrap();
        machine.cpu.pc = RAM_BASE as u32;

        // MODER AF mode + AFRL nibble 4 for PA2 (silicon AFRL after USART2 init).
        machine
            .bus
            .write_u32(GPIOA_L0 + MODER, 0b10 << (PA2 * 2))
            .unwrap();
        machine
            .bus
            .write_u32(GPIOA_L0 + AFRL, 4 << (PA2 * 4))
            .unwrap();
        machine.bus.write_u32(USART2 + BRR, USARTDIV).unwrap();

        let gpio_idx = machine.bus.find_peripheral_index_by_name("gpioa").unwrap();
        // Pin down WHERE a failure is before arming: a wrong AF table shows up
        // as a pad that never leaves the latch, which is indistinguishable from
        // a MODER/AFRL write that landed on the wrong window.
        assert_eq!(
            machine.bus.read_u32(GPIOA_L0 + MODER).unwrap(),
            0b10 << (PA2 * 2),
            "MODER write must land on the L0 gpioa window",
        );
        assert_eq!(
            machine.bus.read_u32(GPIOA_L0 + AFRL).unwrap(),
            4 << (PA2 * 4),
            "AFRL write must land on the L0 gpioa window",
        );
        assert_eq!(
            machine.bus.peripherals[gpio_idx].dev.read_gpio_pad(PA2),
            Some(true),
            "AF4 route must already resolve the USART TX mark before arming",
        );

        let initial = machine.logic_watch(&[Some(LogicSource::pad(gpio_idx, PA2))]);
        assert_eq!(
            initial,
            vec![Some(true)],
            "idle USART TX rests at mark through the AF4 pad route",
        );

        for _ in 0..20_000 {
            machine.step().unwrap();
        }
        for &byte in PAYLOAD {
            machine.bus.write_u8(USART2 + TDR, byte).unwrap();
        }
        for _ in 0..PAYLOAD.len() as u64 * 10 * BIT_TIME + 16 {
            machine.step().unwrap();
        }

        let edges = machine.logic_read_edges(0).edges;
        assert!(
            !edges.is_empty(),
            "L0 USART2_TX on PA2/AF4 must put edges on the pad; AF7 binding \
             leaves a flat latch while the peripheral transmits",
        );
        assert_eq!(
            decode(&edges, BIT_TIME),
            PAYLOAD.to_vec(),
            "AF4 route must carry the firmware payload, not a false flat or \
             wrong bit order",
        );
    }

    #[test]
    fn logic_capture_sees_a_decodable_stm32_uart_waveform() {
        let mut machine = machine();
        configure(&mut machine, TX_PIN);

        let gpio_idx = machine.bus.find_peripheral_index_by_name("gpiob").unwrap();
        let initial = machine.logic_watch(&[Some(LogicSource::pad(gpio_idx, TX_PIN))]);
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
            "a USART transmission must put edges on its AF-routed pad",
        );
        assert_eq!(
            decode(&edges, BIT_TIME),
            b"Hi!\n".to_vec(),
            "the wire must carry the characters the firmware transmitted",
        );
    }

    #[test]
    fn a_pad_that_carries_no_usart_function_shows_no_serial_traffic() {
        // This gates the TABLE, not the AF mode: PB3 is put into alternate
        // function 7 exactly like the real TX pad, and must still stay silent,
        // because AF7 on PB3 is not a USART function at all. Route a pad the
        // datasheet does not list and this is what catches it.
        let mut machine = machine();
        configure(&mut machine, TX_PIN);
        // MODER is one word for the whole port, so this must ADD PB3's AF mode
        // rather than write it alone — writing it alone would clear PB10's,
        // leaving the real TX pad unrouted and the assertion true for a reason
        // that has nothing to do with the table.
        machine
            .bus
            .write_u32(
                GPIOB_BASE + MODER,
                (0b10 << (TX_PIN * 2)) | (0b10 << (NON_UART_PIN * 2)),
            )
            .unwrap();
        machine
            .bus
            .write_u32(GPIOB_BASE + AFRL, 7 << (NON_UART_PIN * 4))
            .unwrap();

        let gpio_idx = machine.bus.find_peripheral_index_by_name("gpiob").unwrap();
        // Watch BOTH pads: the TX channel proves the machinery is live in this
        // very fixture, so the control channel's silence means the table, not a
        // broken setup. Without that, `is_empty` is satisfied by any failure.
        machine.logic_watch(&[
            Some(LogicSource::pad(gpio_idx, TX_PIN)),
            Some(LogicSource::pad(gpio_idx, NON_UART_PIN)),
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
            "AF7 on a pad with no USART function must not show the serial line",
        );
    }

    #[test]
    fn the_line_runs_at_the_baud_rate_brr_programs() {
        let mut machine = machine();
        configure(&mut machine, TX_PIN);
        let gpio_idx = machine.bus.find_peripheral_index_by_name("gpiob").unwrap();
        machine.logic_watch(&[Some(LogicSource::pad(gpio_idx, TX_PIN))]);
        for _ in 0..20_000 {
            machine.step().unwrap();
        }
        // 0x55 alternates on every bit, so the gaps between transitions ARE the
        // bit period.
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
            "every bit should last BRR = {BIT_TIME} cycles, got {gaps:?}",
        );
    }

    #[test]
    fn oversampling_by_eight_measures_the_same_baud_as_by_sixteen() {
        // OVER8 re-encodes the divisor: the hardware programs
        // USARTDIV = 2 x f_ck/baud, keeps USARTDIV[15:4] in BRR[15:4], puts
        // USARTDIV[3:0] >> 1 in BRR[2:0] and forces BRR[3] to zero. Taking BRR
        // at face value under OVER8 reports a bit period twice the real one —
        // a trace at half the baud the firmware asked for, with nothing to
        // flag it.
        //
        // Same 115200 on the same 80 MHz clock as the OVER16 gate, so the
        // measured period must come out identical. 80e6/115200 = 694.4;
        // USARTDIV = 1389 (0x56D) -> BRR = 0x560 | (0xD >> 1) = 0x566.
        const BRR_OVER8: u32 = 0x566;

        let mut machine = machine();
        let bus = &mut machine.bus;
        bus.write_u32(GPIOB_BASE + MODER, 0b10 << (TX_PIN * 2))
            .unwrap();
        bus.write_u32(GPIOB_BASE + AFRH, 7 << ((TX_PIN - 8) * 4))
            .unwrap();
        bus.write_u32(USART3_BASE + CR1, CR1_OVER8).unwrap();
        bus.write_u32(USART3_BASE + BRR, BRR_OVER8).unwrap();

        let gpio_idx = machine.bus.find_peripheral_index_by_name("gpiob").unwrap();
        machine.logic_watch(&[Some(LogicSource::pad(gpio_idx, TX_PIN))]);
        for _ in 0..20_000 {
            machine.step().unwrap();
        }
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
            "OVER8 must measure the same {BIT_TIME}-cycle bit as OVER16, got {gaps:?}",
        );
    }

    #[test]
    fn a_usart_whose_baud_was_never_programmed_publishes_nothing() {
        // BRR reads 0 out of reset. Narrating at a made-up rate would give a
        // trace that measures a frequency the firmware never asked for.
        let mut machine = machine();
        let bus = &mut machine.bus;
        bus.write_u32(GPIOB_BASE + MODER, 0b10 << (TX_PIN * 2))
            .unwrap();
        bus.write_u32(GPIOB_BASE + AFRH, 7 << ((TX_PIN - 8) * 4))
            .unwrap();

        let gpio_idx = machine.bus.find_peripheral_index_by_name("gpiob").unwrap();
        machine.logic_watch(&[Some(LogicSource::pad(gpio_idx, TX_PIN))]);
        for _ in 0..20_000 {
            machine.step().unwrap();
        }
        transmit(&mut machine, b"Hi!\n");

        assert!(
            machine.logic_read_edges(0).edges.is_empty(),
            "no programmed baud rate means no honest timebase, so no waveform",
        );
    }
}
