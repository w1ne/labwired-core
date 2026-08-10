// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! End-to-end waveform test for the RP2040 SPI: select the SPI function on a
//! pad through IO_BANK0 exactly as `gpio_set_function` does, arm the in-engine
//! logic analyzer on it, shift words through SSPDR, and assert the captured
//! edges decode back to what the firmware sent.
//!
//! SPI was the last bus on this board a probe could not see. The PL022 model
//! moves a whole word inside the `SSPDR` write — no shift counter, no bit index
//! — so the pad was whatever the SIO output latch happened to hold, and a scope
//! on GP3 read a flat line while the firmware was clocking bytes out.
//!
//! The decoder below shares no code with the model. It replays the three lines,
//! samples MOSI on the edge the MODE selects, and cuts frames on chip select —
//! which is what a real receiver does. That is what makes the test sensitive to
//! bit ORDER and to CPOL/CPHA: narrating LSB-first, or sampling on the wrong
//! edge, produces a trace that looks entirely plausible and decodes to garbage.

#[cfg(test)]
mod rp2040_spi_waveform_tests {
    use crate::logic_capture::LogicEdge;
    use crate::logic_capture::LogicSource;
    use crate::peripherals::rp2040::io_bank0::{Rp2040IoBank0, GPIO_FUNC_SPI};
    use crate::peripherals::rp2040::sio::Rp2040Sio;
    use crate::peripherals::rp2040::spi::Rp2040Spi;
    use crate::{Bus, Machine};

    const IO_BANK0_BASE: u64 = 0x4001_4000;
    const SIO_BASE: u64 = 0xD000_0000;
    // RP2040 SVD: SPI0 @ 0x4003c000, SPI1 @ 0x40040000.
    const SPI0_BASE: u64 = 0x4003_c000;
    const SPI1_BASE: u64 = 0x4004_0000;
    const RAM_BASE: u64 = 0x2000_0000;

    const SSPCR0: u64 = 0x00;
    const SSPCR1: u64 = 0x04;
    const SSPDR: u64 = 0x08;
    const SSPCPSR: u64 = 0x10;
    const CR1_SSE: u32 = 1 << 1;

    /// GP3 is `spi0_tx` (MOSI), GP2 `spi0_sclk`, GP1 `spi0_ss_n`.
    const MOSI_PIN: u8 = 3;
    const SCK_PIN: u8 = 2;
    const CS_PIN: u8 = 1;
    /// GP0 is `spi0_rx` — an SPI function, but not one anything drives. The
    /// control case.
    const NON_DRIVEN_PIN: u8 = 0;

    /// Channel order matters. MOSI is watched FIRST so that when a data setup
    /// and a clock edge land on the same cycle (CPHA=1's leading edge) the
    /// cycle-then-channel sort settles MOSI before SCK — which is what a setup
    /// window means physically.
    const CH_MOSI: u32 = 0;
    const CH_SCK: u32 = 1;
    const CH_CS: u32 = 2;

    /// What pico-sdk's `spi_set_baudrate` programs for 1 MHz from a 125 MHz
    /// clk_peri: prescale 2, postdiv 63 ⇒ SCR = 62. One bit is
    /// CPSDVSR × (1 + SCR) = 2 × 63 = 126 clk_peri cycles.
    const CPSDVSR: u32 = 2;
    const SCR: u32 = 62;
    const BIT_TIME: u64 = 126;
    /// 8 clocked bits + one period of CS setup/hold + one idle period. Mirrors
    /// `SpiFraming::frame_bits`, hand-entered here so a change to the pacing
    /// rule is a DISAGREEMENT between two lists rather than a shared mistake.
    const FRAME_CYCLES: u64 = 10 * BIT_TIME;

    #[derive(Clone, Copy)]
    struct Mode {
        cpol: bool,
        cpha: bool,
        bits: u8,
    }
    const MODE0: Mode = Mode {
        cpol: false,
        cpha: false,
        bits: 8,
    };

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
        bus.add_peripheral("spi0", SPI0_BASE, 0x1000, None, Box::new(Rp2040Spi::new()));
        bus.add_peripheral("spi1", SPI1_BASE, 0x1000, None, Box::new(Rp2040Spi::new()));
        bus.wire_rp2040_spi_pads();

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

    /// `spi_init` + `spi_set_baudrate` + `spi_set_format` + `gpio_set_function`
    /// for the three driven pads.
    fn configure(machine: &mut Machine<crate::cpu::CortexM>, base: u64, mode: Mode) {
        let cr0 = (SCR << 8)
            | (u32::from(mode.cpha) << 7)
            | (u32::from(mode.cpol) << 6)
            | u32::from(mode.bits - 1);
        let bus = &mut machine.bus;
        bus.write_u32(base + SSPCPSR, CPSDVSR).unwrap();
        bus.write_u32(base + SSPCR0, cr0).unwrap();
        bus.write_u32(base + SSPCR1, CR1_SSE).unwrap();
        for pin in [MOSI_PIN, SCK_PIN, CS_PIN] {
            bus.write_u32(IO_BANK0_BASE + ctrl_offset(pin), GPIO_FUNC_SPI)
                .unwrap();
        }
    }

    /// Shift the words, then let the engine run long enough for the wire to have
    /// carried them.
    ///
    /// The second half is not scaffolding — it is the point. This model shifts a
    /// whole buffer within a few cycles because TX always reports empty, but the
    /// wire needs ten bit periods per frame. The narration waits for that time
    /// to pass before publishing, so a test that read the trace immediately
    /// would be reading a bus that has not physically finished clocking.
    fn shift(machine: &mut Machine<crate::cpu::CortexM>, base: u64, words: &[u16]) {
        for &word in words {
            machine
                .bus
                .write_u32(base + SSPDR, u32::from(word))
                .unwrap();
        }
        let wire_cycles = words.len() as u64 * FRAME_CYCLES;
        for _ in 0..wire_cycles + 64 {
            machine.step().unwrap();
        }
    }

    /// An INDEPENDENT SPI decoder. Knows the protocol, nothing about the model.
    fn decode(edges: &[LogicEdge], mode: Mode) -> Vec<u16> {
        let mut events: Vec<(u64, u32, bool)> =
            edges.iter().map(|e| (e.cycle, e.ch, e.value)).collect();
        events.sort_by_key(|&(cycle, ch, _)| (cycle, ch));

        let (mut sck, mut mosi, mut cs) = (mode.cpol, false, true);
        let (mut acc, mut count) = (0u32, 0u8);
        let mut out = Vec::new();
        for (_, ch, value) in events {
            let previous_sck = sck;
            match ch {
                CH_SCK => sck = value,
                CH_MOSI => mosi = value,
                CH_CS => cs = value,
                _ => {}
            }
            if ch == CH_CS && value {
                // Chip select released: whatever was partial is not a frame.
                acc = 0;
                count = 0;
                continue;
            }
            if ch == CH_SCK && !cs && previous_sck != sck {
                let sampling = if mode.cpha {
                    sck == mode.cpol // CPHA=1 samples on the trailing edge
                } else {
                    sck != mode.cpol // CPHA=0 samples on the leading edge
                };
                if sampling {
                    acc = (acc << 1) | u32::from(mosi); // MSB first
                    count += 1;
                    if count == mode.bits {
                        out.push(acc as u16);
                        acc = 0;
                        count = 0;
                    }
                }
            }
        }
        out
    }

    fn watch_driven(machine: &mut Machine<crate::cpu::CortexM>) -> Vec<Option<bool>> {
        let sio = machine.bus.find_peripheral_index_by_name("sio").unwrap();
        machine.logic_watch(&[
            Some(LogicSource::pad(sio, MOSI_PIN)),
            Some(LogicSource::pad(sio, SCK_PIN)),
            Some(LogicSource::pad(sio, CS_PIN)),
        ])
    }

    /// Every pad in the routing table, with the instance the SVD assigns. Kept
    /// here rather than imported so a typo in the model's table is a
    /// DISAGREEMENT between two hand-entered lists, not a shared mistake.
    const SPI_PADS: &[(u8, usize)] = &[
        (1, 0),
        (2, 0),
        (3, 0),
        (5, 0),
        (6, 0),
        (7, 0),
        (9, 1),
        (10, 1),
        (11, 1),
        (13, 1),
        (14, 1),
        (15, 1),
        (17, 0),
        (18, 0),
        (19, 0),
        (21, 0),
        (22, 0),
        (23, 0),
        (25, 1),
        (26, 1),
        (27, 1),
        (29, 1),
    ];

    // ── Gate 1: an INDEPENDENT decoder recovers the bytes ────────────────────
    #[test]
    fn logic_capture_sees_a_decodable_spi_waveform() {
        let mut machine = machine();
        configure(&mut machine, SPI0_BASE, MODE0);
        let initial = watch_driven(&mut machine);
        assert_eq!(
            initial,
            vec![Some(false), Some(false), Some(true)],
            "an idle SPI bus rests MOSI low, SCK at CPOL and chip select RELEASED",
        );
        for _ in 0..20_000 {
            machine.step().unwrap();
        }
        shift(&mut machine, SPI0_BASE, &[0xA5, 0x01, 0x80, 0xFF, 0x00]);

        let edges = machine.logic_read_edges(0).edges;
        assert!(
            !edges.is_empty(),
            "an SPI transfer must put edges on its routed pads, not a flat trace",
        );
        assert_eq!(
            decode(&edges, MODE0),
            vec![0xA5, 0x01, 0x80, 0xFF, 0x00],
            "the wire must carry the words the firmware shifted",
        );
    }

    // ── Gate 2: all four modes, both sampling edges ──────────────────────────
    #[test]
    fn every_clock_polarity_and_phase_decodes_on_its_own_sampling_edge() {
        for cpol in [false, true] {
            for cpha in [false, true] {
                let mode = Mode {
                    cpol,
                    cpha,
                    bits: 8,
                };
                let mut machine = machine();
                configure(&mut machine, SPI0_BASE, mode);
                watch_driven(&mut machine);
                for _ in 0..20_000 {
                    machine.step().unwrap();
                }
                // NOT bit-palindromes. `0x5A` and `0xC3` both read the same
                // reversed, so a mode gate built on them stays green under an
                // LSB-first narration — a blind spot this pair closes.
                shift(&mut machine, SPI0_BASE, &[0x9C, 0x1B]);
                assert_eq!(
                    decode(&machine.logic_read_edges(0).edges, mode),
                    vec![0x9C, 0x1B],
                    "mode {}{} did not survive the wire",
                    u8::from(cpol),
                    u8::from(cpha),
                );
            }
        }
    }

    // ── Gate 3: the clock rate is the one the registers program ──────────────
    #[test]
    fn sck_runs_at_the_rate_cpsdvsr_and_scr_program() {
        let mut machine = machine();
        configure(&mut machine, SPI0_BASE, MODE0);
        watch_driven(&mut machine);
        for _ in 0..20_000 {
            machine.step().unwrap();
        }
        shift(&mut machine, SPI0_BASE, &[0xA5]);

        let leading: Vec<u64> = machine
            .logic_read_edges(0)
            .edges
            .iter()
            .filter(|e| e.ch == CH_SCK && e.value != MODE0.cpol)
            .map(|e| e.cycle)
            .collect();
        assert_eq!(leading.len(), 8, "one clock per bit: {leading:?}");
        let gaps: Vec<u64> = leading.windows(2).map(|p| p[1] - p[0]).collect();
        // Asserting only "a multiple of BIT_TIME" would pass at half or double
        // the programmed rate — the error this gate exists to catch.
        assert!(
            gaps.iter().all(|&g| g == BIT_TIME),
            "every bit should last {BIT_TIME} cycles, got {gaps:?}",
        );
    }

    // ── Gate 4: chip select frames every word ────────────────────────────────
    #[test]
    fn chip_select_brackets_each_frame_and_releases_between_them() {
        let mut machine = machine();
        configure(&mut machine, SPI0_BASE, MODE0);
        watch_driven(&mut machine);
        for _ in 0..20_000 {
            machine.step().unwrap();
        }
        shift(&mut machine, SPI0_BASE, &[0x12, 0x34, 0x56]);

        let edges = machine.logic_read_edges(0).edges;
        let cs: Vec<bool> = edges
            .iter()
            .filter(|e| e.ch == CH_CS)
            .map(|e| e.value)
            .collect();
        assert_eq!(
            cs,
            vec![false, true, false, true, false, true],
            "one assert/release pair per frame: {cs:?}",
        );
        // Every clock edge must fall inside an asserted window, or a decoder
        // cutting on CS would lose bits.
        let assert_at: Vec<u64> = edges
            .iter()
            .filter(|e| e.ch == CH_CS && !e.value)
            .map(|e| e.cycle)
            .collect();
        let release_at: Vec<u64> = edges
            .iter()
            .filter(|e| e.ch == CH_CS && e.value)
            .map(|e| e.cycle)
            .collect();
        for edge in edges.iter().filter(|e| e.ch == CH_SCK) {
            assert!(
                assert_at
                    .iter()
                    .zip(&release_at)
                    .any(|(&lo, &hi)| edge.cycle > lo && edge.cycle < hi),
                "SCK moved at {} outside any chip-select window",
                edge.cycle,
            );
        }
        assert_eq!(decode(&edges, MODE0), vec![0x12, 0x34, 0x56]);
    }

    // ── Gate 5: the instance column of the pad table, both directions ────────
    #[test]
    fn every_pad_in_the_table_carries_its_own_instance_and_no_other() {
        // The whole justification for transcribing this table by hand is that
        // it cannot be derived: the ROLES repeat every four pads but the
        // INSTANCE flips every eight, so GP8-15 and GP24-29 are spi1 where the
        // surrounding four-pad pattern says nothing at all about instance.
        //
        // One pad per fixture, in a loop. The RP2040 has exactly ONE routing
        // table (SIO's) so a single `logic_watch` over all 22 pads does union
        // their channels per wire line — but a per-pad fixture also pins that
        // each pad reads the wire ON ITS OWN, which is the claim the table
        // actually makes, and it does not depend on that union behaviour.
        for &(pin, instance) in SPI_PADS {
            let mut machine = machine();
            for &(other, _) in SPI_PADS {
                machine
                    .bus
                    .write_u32(IO_BANK0_BASE + ctrl_offset(other), GPIO_FUNC_SPI)
                    .unwrap();
            }
            for base in [SPI0_BASE, SPI1_BASE] {
                configure(&mut machine, base, MODE0);
            }
            let sio = machine.bus.find_peripheral_index_by_name("sio").unwrap();
            // Watch the pad under test ALONGSIDE a pad known to carry spi0
            // (GP3, `spi0_tx`), so a spi1 pad's silence means the instance
            // column and not a fixture that produced no traffic at all.
            machine.logic_watch(&[
                Some(LogicSource::pad(sio, pin)),
                Some(LogicSource::pad(sio, MOSI_PIN)),
            ]);
            for _ in 0..20_000 {
                machine.step().unwrap();
            }
            // spi0 alone shifts.
            shift(&mut machine, SPI0_BASE, &[0xA5, 0x5A]);

            let edges = machine.logic_read_edges(0).edges;
            assert!(
                edges.iter().any(|e| e.ch == 1),
                "GP{MOSI_PIN} must carry spi0's traffic, or GP{pin} proves nothing",
            );
            let saw = edges.iter().any(|e| e.ch == 0);
            assert_eq!(
                saw,
                instance == 0,
                "GP{pin} belongs to spi{instance}; spi0 was the one shifting",
            );
        }
    }

    // ── Gate 6: the undriven control pad ─────────────────────────────────────
    #[test]
    fn a_pad_carrying_a_signal_nothing_drives_shows_no_traffic() {
        let mut machine = machine();
        configure(&mut machine, SPI0_BASE, MODE0);
        machine
            .bus
            .write_u32(IO_BANK0_BASE + ctrl_offset(NON_DRIVEN_PIN), GPIO_FUNC_SPI)
            .unwrap();

        let sio = machine.bus.find_peripheral_index_by_name("sio").unwrap();
        // Watch the MOSI pad ALONGSIDE the control pad: the MOSI channel proves
        // the machinery is live in this very fixture, so the control channel's
        // silence means the table, not a broken setup. Without that, `is_empty`
        // is satisfied by any failure at all.
        machine.logic_watch(&[
            Some(LogicSource::pad(sio, MOSI_PIN)),
            Some(LogicSource::pad(sio, NON_DRIVEN_PIN)),
        ]);
        for _ in 0..20_000 {
            machine.step().unwrap();
        }
        shift(&mut machine, SPI0_BASE, &[0xA5]);

        let edges = machine.logic_read_edges(0).edges;
        assert!(
            edges.iter().any(|e| e.ch == 0),
            "the routed MOSI pad must be carrying traffic, or this proves nothing",
        );
        assert!(
            !edges.iter().any(|e| e.ch == 1),
            "spi0_rx is driven by the DEVICE, not the MCU; a bound RX pad would \
             report a confident idle level straight through real traffic",
        );
    }

    // ── Gate 7: no prescaler, no waveform ────────────────────────────────────
    #[test]
    fn a_prescaler_that_was_never_programmed_publishes_nothing() {
        let mut machine = machine();
        // Everything EXCEPT the prescaler: format, enable, pad functions.
        machine
            .bus
            .write_u32(SPI0_BASE + SSPCR0, (SCR << 8) | 0x07)
            .unwrap();
        machine.bus.write_u32(SPI0_BASE + SSPCR1, CR1_SSE).unwrap();
        for pin in [MOSI_PIN, SCK_PIN, CS_PIN] {
            machine
                .bus
                .write_u32(IO_BANK0_BASE + ctrl_offset(pin), GPIO_FUNC_SPI)
                .unwrap();
        }
        watch_driven(&mut machine);
        for _ in 0..20_000 {
            machine.step().unwrap();
        }
        shift(&mut machine, SPI0_BASE, &[0xA5, 0x5A]);

        assert!(
            machine.logic_read_edges(0).edges.is_empty(),
            "CPSDVSR resets to 0, an invalid divisor — no honest timebase, so no waveform",
        );
    }

    // ── Gate 8: a second burst lands after the first, not over it ────────────
    #[test]
    fn a_second_burst_lands_after_the_first_and_not_over_it() {
        let mut machine = machine();
        configure(&mut machine, SPI0_BASE, MODE0);
        watch_driven(&mut machine);
        for _ in 0..20_000 {
            machine.step().unwrap();
        }
        shift(&mut machine, SPI0_BASE, &[0x11, 0x22]);
        shift(&mut machine, SPI0_BASE, &[0x33, 0x44]);

        assert_eq!(
            decode(&machine.logic_read_edges(0).edges, MODE0),
            vec![0x11, 0x22, 0x33, 0x44],
            "both bursts must be on the wire, in order and intact",
        );
    }

    // ── Gate 8b: the narration floor, on the path where it is load-bearing ───
    #[test]
    fn a_forced_narration_never_repaints_cycles_an_earlier_one_owns() {
        // ⚠️ The sequential-burst gate above does NOT pin the floor, and that is
        // worth stating rather than assuming: a PACED flush only fires once
        // `now` has reached `wave_cursor + duration`, so `available` is exactly
        // the burst's duration and therefore always ≥ its span. `emit_between`
        // and `emit_ending_at` then anchor identically and the floor is
        // unobservable. Dropping it stayed green there.
        //
        // The floor decides the FORCED flush — a burst published before the wire
        // could carry it. Here a mid-burst prescaler change forces one out with
        // only a few hundred cycles of room for a ~2400-cycle waveform. With the
        // floor it compresses into the cycles it owns; without it, it reaches
        // back across everything the previous burst already painted and splices
        // the two into frames neither transfer sent.
        let mut machine = machine();
        configure(&mut machine, SPI0_BASE, MODE0);
        watch_driven(&mut machine);
        for _ in 0..20_000 {
            machine.step().unwrap();
        }
        // Burst A, stepped only far enough to publish. Using `shift` here would
        // ALSO advance past the second burst's whole wire time, leaving room
        // the floor is not needed for — which is exactly how the first version
        // of this gate was vacuous.
        for word in [0x11u32, 0x22] {
            machine.bus.write_u32(SPI0_BASE + SSPDR, word).unwrap();
        }
        for _ in 0..32 {
            machine.step().unwrap();
        }

        let before: Vec<(u64, u32, bool)> = machine
            .logic_read_edges(0)
            .edges
            .iter()
            .map(|e| (e.cycle, e.ch, e.value))
            .collect();
        assert!(!before.is_empty(), "the first burst must have painted");
        let boundary = before.iter().map(|&(cycle, _, _)| cycle).max().unwrap();

        // A short window — far less than the second burst's ~2400-cycle wire
        // time, but comfortably more than its ~36 distinct transitions, so the
        // honest answer is a COMPRESSED waveform rather than levels-only.
        for _ in 0..200 {
            machine.step().unwrap();
        }
        for word in [0x33u32, 0x44] {
            machine.bus.write_u32(SPI0_BASE + SSPDR, word).unwrap();
        }
        // Reprogramming the prescaler mid-burst forces what is held out at the
        // rate it was actually shifted at.
        machine.bus.write_u32(SPI0_BASE + SSPCPSR, 4).unwrap();
        machine.bus.write_u32(SPI0_BASE + SSPDR, 0x55).unwrap();
        for _ in 0..64 {
            machine.step().unwrap();
        }

        let after: Vec<(u64, u32, bool)> = machine
            .logic_read_edges(0)
            .edges
            .iter()
            .map(|e| (e.cycle, e.ch, e.value))
            .collect();
        assert!(
            after.len() > before.len(),
            "the forced burst must still leave a trace, or the check below is \
             satisfied by nothing happening at all",
        );
        let below: Vec<(u64, u32, bool)> = after
            .iter()
            .copied()
            .filter(|&(cycle, _, _)| cycle <= boundary)
            .collect();
        assert_eq!(
            below, before,
            "a forced narration reached back into cycles the previous burst \
             already owns, inventing transitions that never happened",
        );
    }

    // ── Gate 9: the register read-back a RMW driver depends on ───────────────
    #[test]
    fn the_control_registers_read_back_what_firmware_wrote() {
        let mut machine = machine();
        configure(&mut machine, SPI0_BASE, MODE0);
        assert_eq!(machine.bus.read_u32(SPI0_BASE + SSPCPSR).unwrap(), CPSDVSR);
        assert_eq!(
            (machine.bus.read_u32(SPI0_BASE + SSPCR0).unwrap() >> 8) & 0xFF,
            SCR,
        );
    }
}
