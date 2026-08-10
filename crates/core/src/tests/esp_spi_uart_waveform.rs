// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! End-to-end waveform gates for the Espressif SPI and UART pads — ESP32-C3,
//! ESP32-S3 and classic ESP32 (LX6).
//!
//! Before this, both buses ran and neither could be measured on either part: the
//! GP-SPI transaction engine moves a whole `SPI_CMD.USR` launch inside one MMIO
//! write and the UART hands a byte to its sink with no bit engine at all, so a
//! probe clipped to `FSPICLK` or a remapped `U0TXD` read the GPIO output latch
//! while the firmware was clocking bytes out.
//!
//! # What makes these gates rather than mirrors
//!
//! The decoders below share NO code with the models. They replay the captured
//! edges, sample MOSI on the edge CPOL/CPHA select, cut SPI frames on chip
//! select, and recover UART characters by sampling the middle of each bit period
//! after a falling start edge — which is what a real receiver does. That is what
//! makes them sensitive to bit ORDER and to the mode bits: narrating LSB-first,
//! or reading `CK_IDLE_EDGE` out of the wrong register, produces a trace that
//! looks entirely plausible and decodes to garbage.
//!
//! # The per-chip index trap, gated explicitly
//!
//! GPIO-matrix signal indices are PER CHIP. `FSPICLK` is 63 on the C3 and 101 on
//! the S3; 63 on the S3 is not a SPI signal at all. Borrowing a sibling's
//! constant fails silently in BOTH directions — a plain pad decodes as routed
//! and a routed pad as plain — so
//! [`the_matrix_indices_are_per_chip_and_do_not_cross`] routes each part's pads
//! with the OTHER part's numbers and asserts nothing lights up.
//!
//! ⚠️ Classic ESP32 makes the trap worse, not better: its VSPI indices are 63 /
//! 65 / 68, byte for byte the C3's FSPI numbers. That is a COLLISION of two
//! index spaces, not evidence of a shared one — the classic UART indices in the
//! same header are 14 / 17 / 198 against the C3's 6 / 9 / (none), and on the S3
//! 63 is not a SPI signal. So the classic part is crossed against the S3 (where
//! the numbers differ and must not light) and against the C3 on the UART indices
//! (where the SPI numbers coincide and the UART ones cannot).
//!
//! # The register geometry is not shared either
//!
//! The classic SPI's `SPI_CLOCK` is at 0x18, its CPHA bit is `SPI_USER` bit 7
//! (not 9), and its CPOL bit is in `SPI_PIN` at 0x34 because the part has no
//! `SPI_MISC`. The classic constants below are hand-entered from
//! `soc/esp32/include/soc/spi_reg.h` for the same reason the signal indices are:
//! a change to a model constant should be a DISAGREEMENT between two lists, not
//! a shared mistake.
//!
//! # Production, not just possible
//!
//! Four tests assert on the builders real labs are made from — `from_config`
//! for the C3, `configure_xtensa_esp32s3` for the S3, and BOTH
//! `configure_xtensa_esp32` and `from_config` for the classic part — because a
//! waveform gate that constructs its own wiring proves the narration works, not
//! that anything ships it. That is exactly how the S3's I²C binding stayed green
//! in test while being dark in every real lab.
//!
//! Classic ESP32 needs both because its two builders disagree:
//! `configure_xtensa_esp32` registers the peripheral bank in Rust and is what
//! every real lab runs, while `configs/chips/esp32.yaml` is what
//! `crates/core/tests/bus_visibility.rs` scores the fleet on.

#[cfg(test)]
mod esp_spi_uart_waveform_tests {
    use crate::cpu::CortexM;
    use crate::logic_capture::LogicEdge;
    use crate::logic_capture::LogicSource;
    use crate::peripherals::esp32c3::gpio::Esp32c3Gpio;
    use crate::peripherals::esp32c3::spi::Esp32c3Spi;
    use crate::peripherals::esp32s3::gpio::Esp32s3Gpio;
    use crate::peripherals::esp32s3::gpspi::Esp32s3Spi;
    use crate::{Bus, Machine};

    const RAM_BASE: u64 = 0x2000_0000;
    const GPIO_BASE: u64 = 0x6000_4000;
    const SPI2_BASE: u64 = 0x6002_4000;
    const UART0_BASE: u64 = 0x6000_0000;

    // --- GPIO matrix registers, identical offsets on both parts -------------
    const ENABLE_W1TS: u64 = 0x24;
    const FUNC_OUT_SEL: u64 = 0x554;

    // --- GP-SPI registers (same offsets on C3 and S3, `spi_reg.h`) ----------
    const SPI_CMD: u64 = 0x00;
    const SPI_CLOCK: u64 = 0x0C;
    const SPI_USER: u64 = 0x10;
    const SPI_MS_DLEN: u64 = 0x1C;
    const SPI_MISC: u64 = 0x20;
    const SPI_W0: u64 = 0x98;
    const SPI_USR: u32 = 1 << 24;
    /// `SPI_CK_OUT_EDGE` (USER bit 9) — CPHA.
    const CK_OUT_EDGE: u32 = 1 << 9;
    /// `SPI_CK_IDLE_EDGE` (MISC bit 29) — CPOL.
    const CK_IDLE_EDGE: u32 = 1 << 29;

    // --- Espressif UART registers ------------------------------------------
    const UART_FIFO: u64 = 0x00;
    const UART_CLKDIV: u64 = 0x14;

    // --- Matrix signal indices, from esp-idf `gpio_sig_map.h` ---------------
    //
    // Hand-entered here rather than imported from the models, so a change to a
    // model's constant is a DISAGREEMENT between two lists instead of a shared
    // mistake — the mirror-test failure mode.
    /// C3 `FSPICLK_OUT_IDX` :104 / `FSPID_OUT_IDX` :108 / `FSPICS0_OUT_IDX` :114.
    const C3_SIG_SCK: u32 = 63;
    const C3_SIG_MOSI: u32 = 65;
    const C3_SIG_CS: u32 = 68;
    /// C3 `U0TXD_OUT_IDX` :29.
    const C3_SIG_U0TXD: u32 = 6;
    /// S3 `FSPICLK_OUT_IDX` :194 / `FSPID_OUT_IDX` :198 / `FSPICS0_OUT_IDX` :212.
    const S3_SIG_SCK: u32 = 101;
    const S3_SIG_MOSI: u32 = 103;
    const S3_SIG_CS: u32 = 110;
    /// S3 `U0TXD_OUT_IDX` :39.
    const S3_SIG_U0TXD: u32 = 12;

    // Pads the "firmware" picks. Any pad works — the matrix routes anything
    // anywhere — which is itself part of what is being asserted.
    const SCK_PIN: u8 = 6;
    const MOSI_PIN: u8 = 7;
    const CS_PIN: u8 = 10;
    const TX_PIN: u8 = 5;

    /// Channel order matters: MOSI is watched FIRST so that when a data setup
    /// and a clock edge land on the same cycle (CPHA=1's leading edge) the
    /// cycle-then-channel sort settles MOSI before SCK, which is what a setup
    /// window means physically.
    const CH_MOSI: u32 = 0;
    const CH_SCK: u32 = 1;
    const CH_CS: u32 = 2;
    const CH_TX: u32 = 0;

    /// 1 MHz from the 80 MHz APB — a divide by 80. `CLKCNT_N` [17:12] is only
    /// six bits, so 79 does not fit and the divisor is a PAIR:
    /// `CLKDIV_PRE` [21:18] = 1 (÷2) and `CLKCNT_N` = 39 (÷40).
    /// `CLK_EQU_SYSCLK` (bit 31) must stay CLEAR or the divisors are bypassed.
    const SPI_CLOCK_1MHZ: u32 = (1 << 18) | (39 << 12);
    /// One bit period at that setting: 80 APB ticks × (cpu/apb).
    const C3_SPI_BIT: u64 = 160; // 160 MHz core
    const S3_SPI_BIT: u64 = 240; // 240 MHz core
    /// 8 clocked bits + one period of CS setup/hold + one idle period. Mirrors
    /// `SpiFraming::frame_bits`, hand-entered so a change to the pacing rule is
    /// a disagreement between two lists.
    const SPI_FRAME_BITS: u64 = 10;

    /// `CLKDIV` for 115200 baud from the 80 MHz UART source clock — and the
    /// register's reset value, so this is also what a UART that was never
    /// configured shifts at.
    const UART_CLKDIV_115200: u32 = 694;
    const C3_UART_BIT: u64 = 694 * 160 / 80; // 1388 CPU cycles per bit
    const S3_UART_BIT: u64 = 694 * 240 / 80; // 2082
    /// start + 8 data + stop.
    const UART_FRAME_BITS: u64 = 10;

    #[derive(Clone, Copy, PartialEq, Eq, Debug)]
    struct Mode {
        cpol: bool,
        cpha: bool,
    }
    const MODE0: Mode = Mode {
        cpol: false,
        cpha: false,
    };
    const MODE3: Mode = Mode {
        cpol: true,
        cpha: true,
    };

    /// A NOP slab so `step()` advances the cycle axis deterministically. The
    /// waveform is placed on absolute cycles, so a bus that never advances has
    /// nowhere to put one — running the CPU is part of the measurement, not
    /// scaffolding.
    fn finish(bus: crate::bus::SystemBus) -> Machine<CortexM> {
        let mut bus = bus;
        let (cpu, _nvic) = crate::system::cortex_m::configure_cortex_m(&mut bus);
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

    fn c3_spi_machine() -> Machine<CortexM> {
        let mut bus = crate::bus::SystemBus::new();
        bus.add_peripheral(
            "gpio",
            GPIO_BASE,
            0x1000,
            None,
            Box::new(Esp32c3Gpio::new()),
        );
        bus.add_peripheral(
            "spi2",
            SPI2_BASE,
            0x1000,
            None,
            Box::new(Esp32c3Spi::new(19)),
        );
        bus.wire_esp32c3_spi_pads();
        finish(bus)
    }

    fn s3_spi_machine() -> Machine<CortexM> {
        let mut bus = crate::bus::SystemBus::new();
        bus.add_peripheral(
            "gpio",
            GPIO_BASE,
            0x1000,
            None,
            Box::new(Esp32s3Gpio::new()),
        );
        bus.add_peripheral(
            "spi2_s3",
            SPI2_BASE,
            0x1000,
            None,
            Box::new(Esp32s3Spi::new(21)),
        );
        bus.wire_esp32s3_spi_pads();
        finish(bus)
    }

    fn c3_uart_machine() -> Machine<CortexM> {
        let mut bus = crate::bus::SystemBus::new();
        bus.add_peripheral(
            "gpio",
            GPIO_BASE,
            0x1000,
            None,
            Box::new(Esp32c3Gpio::new()),
        );
        bus.add_peripheral(
            "uart0",
            UART0_BASE,
            0x100,
            None,
            Box::new(crate::peripherals::esp32c3::uart::new(false, 21)),
        );
        bus.wire_esp32c3_uart_pads();
        finish(bus)
    }

    fn s3_uart_machine() -> Machine<CortexM> {
        let mut bus = crate::bus::SystemBus::new();
        bus.add_peripheral(
            "gpio",
            GPIO_BASE,
            0x1000,
            None,
            Box::new(Esp32s3Gpio::new()),
        );
        bus.add_peripheral(
            "uart0_s3",
            UART0_BASE,
            0x100,
            None,
            Box::new(crate::peripherals::esp_uart::EspUart::new(false, 27)),
        );
        bus.wire_esp32s3_uart_pads();
        finish(bus)
    }

    /// `gpio_matrix_out(pin, signal, false, false)` plus the output-driver
    /// enable, which is what `esp_rom_gpio_connect_out_signal` +
    /// `gpio_set_direction` do.
    fn route(machine: &mut Machine<CortexM>, pin: u8, signal: u32) {
        machine
            .bus
            .write_u32(GPIO_BASE + ENABLE_W1TS, 1u32 << pin)
            .unwrap();
        machine
            .bus
            .write_u32(GPIO_BASE + FUNC_OUT_SEL + u64::from(pin) * 4, signal)
            .unwrap();
    }

    /// Program the transfer rate and mode, then launch a `SPI_CMD.USR`
    /// transaction carrying `payload`, then run the engine long enough for the
    /// wire to have carried it.
    fn spi_transfer(machine: &mut Machine<CortexM>, payload: &[u8], mode: Mode, bit_time: u64) {
        {
            let bus = &mut machine.bus;
            bus.write_u32(SPI2_BASE + SPI_CLOCK, SPI_CLOCK_1MHZ)
                .unwrap();
            bus.write_u32(
                SPI2_BASE + SPI_MISC,
                if mode.cpol { CK_IDLE_EDGE } else { 0 },
            )
            .unwrap();
            bus.write_u32(
                SPI2_BASE + SPI_USER,
                if mode.cpha { CK_OUT_EDGE } else { 0 },
            )
            .unwrap();
            bus.write_u32(SPI2_BASE + SPI_MS_DLEN, (payload.len() as u32 * 8) - 1)
                .unwrap();
            for (w, chunk) in payload.chunks(4).enumerate() {
                let mut word = 0u32;
                for (b, &byte) in chunk.iter().enumerate() {
                    word |= u32::from(byte) << (8 * b);
                }
                bus.write_u32(SPI2_BASE + SPI_W0 + (w as u64) * 4, word)
                    .unwrap();
            }
            bus.write_u32(SPI2_BASE + SPI_CMD, SPI_USR).unwrap();
        }
        // The model completes the transaction inside that last write; the WIRE
        // needs ten bit periods per byte. Running that time out is the point.
        let wire = payload.len() as u64 * SPI_FRAME_BITS * bit_time;
        for _ in 0..wire + 256 {
            machine.step().unwrap();
        }
    }

    /// Push characters into the TX FIFO and let the shift register drain them
    /// at the programmed baud rate.
    fn uart_send(machine: &mut Machine<CortexM>, base: u64, bytes: &[u8], bit_time: u64) {
        {
            let bus = &mut machine.bus;
            bus.write_u32(base + UART_CLKDIV, UART_CLKDIV_115200)
                .unwrap();
            for &b in bytes {
                bus.write_u32(base + UART_FIFO, u32::from(b)).unwrap();
            }
        }
        let wire = bytes.len() as u64 * UART_FRAME_BITS * bit_time;
        for _ in 0..wire + 256 {
            machine.step().unwrap();
        }
    }

    /// An INDEPENDENT SPI decoder. Knows the protocol, nothing about the model.
    fn decode_spi(edges: &[LogicEdge], mode: Mode) -> Vec<u8> {
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
                // Chip select released: a partial word is not a frame.
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
                    if count == 8 {
                        out.push(acc as u8);
                        acc = 0;
                        count = 0;
                    }
                }
            }
        }
        out
    }

    /// An INDEPENDENT asynchronous-serial decoder: find a falling start edge,
    /// then sample the MIDDLE of each of the next nine bit periods — eight data
    /// bits LSB-first and a stop bit that MUST read high, exactly as a UART
    /// receiver synchronises.
    ///
    /// Sampling mid-bit rather than on transitions is what makes this sensitive
    /// to the programmed baud rate: a waveform drawn at the wrong bit period
    /// walks off the character and the stop-bit check rejects it.
    fn decode_uart(edges: &[LogicEdge], bit_time: u64) -> Vec<u8> {
        let mut ev: Vec<(u64, bool)> = edges
            .iter()
            .filter(|e| e.ch == CH_TX)
            .map(|e| (e.cycle, e.value))
            .collect();
        ev.sort_by_key(|&(cycle, _)| cycle);
        if ev.is_empty() {
            return Vec::new();
        }
        // Level of the line at an arbitrary cycle, replayed from the edges.
        let level_at = |t: u64| -> bool {
            let mut level = true; // idle mark
            for &(cycle, value) in &ev {
                if cycle <= t {
                    level = value;
                } else {
                    break;
                }
            }
            level
        };

        let mut out = Vec::new();
        let mut cursor = 0u64;
        for &(start, value) in &ev {
            if value || start < cursor {
                continue; // not a falling edge, or inside a character
            }
            let mut byte = 0u8;
            for bit in 0..8u64 {
                let t = start + bit_time + bit * bit_time + bit_time / 2;
                if level_at(t) {
                    byte |= 1 << bit; // LSB first
                }
            }
            let stop = start + 9 * bit_time + bit_time / 2;
            if !level_at(stop) {
                continue; // framing error — not a character
            }
            out.push(byte);
            // Re-arm just after the stop-bit sample, which is what a receiver
            // does — NOT at a full ten bit periods from this start edge. The two
            // differ by half a bit, and that slack is load-bearing: the event
            // scheduler arms its first wakeup one cycle later than the per-cycle
            // walk does, so the character train sits one cycle earlier relative
            // to the first start edge under `--features event-scheduler`. A
            // cursor pinned to an exact multiple rejects the next start edge by
            // that one cycle and then decodes the REST of the message from a
            // mid-character transition, which reads as a model bug and is not
            // one. Data transitions all fall before 9.5 bit periods, so nothing
            // inside a character is admitted either.
            cursor = start + 9 * bit_time + bit_time / 2;
        }
        out
    }

    fn watch_spi(machine: &mut Machine<CortexM>) {
        let gpio = machine.bus.find_peripheral_index_by_name("gpio").unwrap();
        machine.logic_watch(&[
            Some(LogicSource::pad(gpio, MOSI_PIN)),
            Some(LogicSource::pad(gpio, SCK_PIN)),
            Some(LogicSource::pad(gpio, CS_PIN)),
        ]);
    }

    fn watch_tx(machine: &mut Machine<CortexM>) {
        let gpio = machine.bus.find_peripheral_index_by_name("gpio").unwrap();
        machine.logic_watch(&[Some(LogicSource::pad(gpio, TX_PIN))]);
    }

    // ================= C3 =================

    #[test]
    fn c3_spi_bytes_reach_the_matrix_routed_pads() {
        let mut m = c3_spi_machine();
        route(&mut m, SCK_PIN, C3_SIG_SCK);
        route(&mut m, MOSI_PIN, C3_SIG_MOSI);
        route(&mut m, CS_PIN, C3_SIG_CS);
        watch_spi(&mut m);

        // ⚠️ Every byte here is bit-ASYMMETRIC on purpose. 0xA5, 0x5A, 0x3C,
        // 0xFF and 0x00 are all bit-palindromes, so a payload of those decodes
        // identically MSB- and LSB-first and a gate built on them cannot see a
        // reversed narration at all. 0x01/0xB2/0x0F reverse to 0x80/0x4D/0xF0.
        let payload = [0xA5u8, 0x01, 0xB2, 0x0F];
        spi_transfer(&mut m, &payload, MODE0, C3_SPI_BIT);

        let edges = m.logic_read_edges(0).edges;
        assert!(
            !edges.is_empty(),
            "GP-SPI2 clocked {} bytes and the routed pads recorded NOTHING — the \
             wire is not reaching the C3 output matrix",
            payload.len()
        );
        assert_eq!(
            decode_spi(&edges, MODE0),
            payload.to_vec(),
            "the captured waveform must decode back to exactly what the W buffer \
             carried, MSB-first"
        );
    }

    /// Mode 3 is what an SSD1306/ILI9341 panel is usually driven at, and it is
    /// the case a model that hardcoded mode 0 would still pass the test above.
    ///
    /// Asserted on the two things the mode bits physically CHANGE, not on
    /// "decodes differently": modes 0 and 3 genuinely sample on the same
    /// physical edge, so a decode comparison would pass on a model that ignored
    /// both bits. What actually differs is the clock's resting level (CPOL) and
    /// whether the data setup shares a cycle with the leading edge (CPHA).
    #[test]
    fn c3_spi_honours_the_mode_bits_in_misc_and_user() {
        let payload = [0x12u8, 0x34]; // bit-asymmetric, see above

        let mut m = c3_spi_machine();
        route(&mut m, SCK_PIN, C3_SIG_SCK);
        route(&mut m, MOSI_PIN, C3_SIG_MOSI);
        route(&mut m, CS_PIN, C3_SIG_CS);
        watch_spi(&mut m);
        spi_transfer(&mut m, &payload, MODE3, C3_SPI_BIT);
        let mode3 = m.logic_read_edges(0).edges;
        assert_eq!(
            decode_spi(&mode3, MODE3),
            payload.to_vec(),
            "a mode-3 transfer must decode as mode 3"
        );

        let mut m = c3_spi_machine();
        route(&mut m, SCK_PIN, C3_SIG_SCK);
        route(&mut m, MOSI_PIN, C3_SIG_MOSI);
        route(&mut m, CS_PIN, C3_SIG_CS);
        watch_spi(&mut m);
        spi_transfer(&mut m, &payload, MODE0, C3_SPI_BIT);
        let mode0 = m.logic_read_edges(0).edges;

        // CPOL (`SPI_MISC.CK_IDLE_EDGE`): the level SCK rests at once the burst
        // is over. A narration that never read MISC parks it at 0 either way.
        let rest = |edges: &[LogicEdge]| edges.iter().rfind(|e| e.ch == CH_SCK).map(|e| e.value);
        assert_eq!(rest(&mode3), Some(true), "CPOL=1 must idle SCK HIGH");
        assert_eq!(rest(&mode0), Some(false), "CPOL=0 must idle SCK LOW");

        // CPHA (`SPI_USER.CK_OUT_EDGE`): the phase decides chip-select framing.
        // Under CPHA=0 every word gets its own CS pulse, so a two-byte burst
        // asserts and releases twice; under CPHA=1 CS is held LOW across the
        // whole burst and asserts once. A slave that latches on the CS edge
        // (every shift register, every display) sees a completely different bus
        // between the two, so this is behaviour, not cosmetics.
        let cs_edges = |edges: &[LogicEdge]| edges.iter().filter(|e| e.ch == CH_CS).count();
        assert_eq!(
            cs_edges(&mode0),
            2 * payload.len(),
            "CPHA=0 must pulse chip select once per word"
        );
        assert_eq!(
            cs_edges(&mode3),
            2,
            "CPHA=1 must HOLD chip select low ACROSS the burst — one assert at \
             the start and one release when the burst closes, whatever its \
             length. More than that means the narration ignored CK_OUT_EDGE and \
             re-framed every word."
        );
    }

    #[test]
    fn c3_uart_characters_reach_a_matrix_routed_tx_pad() {
        let mut m = c3_uart_machine();
        route(&mut m, TX_PIN, C3_SIG_U0TXD);
        watch_tx(&mut m);

        let msg = b"Hi!";
        uart_send(&mut m, UART0_BASE, msg, C3_UART_BIT);

        let edges = m.logic_read_edges(0).edges;
        assert!(
            !edges.is_empty(),
            "UART0 shifted {} characters and the routed TX pad recorded NOTHING",
            msg.len()
        );
        assert_eq!(
            decode_uart(&edges, C3_UART_BIT),
            msg.to_vec(),
            "the captured waveform must decode as 8N1 at the programmed baud rate"
        );
    }

    // ================= S3 =================

    #[test]
    fn s3_spi_bytes_reach_the_matrix_routed_pads() {
        let mut m = s3_spi_machine();
        route(&mut m, SCK_PIN, S3_SIG_SCK);
        route(&mut m, MOSI_PIN, S3_SIG_MOSI);
        route(&mut m, CS_PIN, S3_SIG_CS);
        watch_spi(&mut m);

        // ⚠️ Every byte here is bit-ASYMMETRIC on purpose. 0xA5, 0x5A, 0x3C,
        // 0xFF and 0x00 are all bit-palindromes, so a payload of those decodes
        // identically MSB- and LSB-first and a gate built on them cannot see a
        // reversed narration at all. 0x01/0xB2/0x0F reverse to 0x80/0x4D/0xF0.
        let payload = [0xA5u8, 0x01, 0xB2, 0x0F];
        spi_transfer(&mut m, &payload, MODE0, S3_SPI_BIT);

        let edges = m.logic_read_edges(0).edges;
        assert!(
            !edges.is_empty(),
            "GP-SPI2 clocked {} bytes and the routed pads recorded NOTHING — the \
             wire is not reaching the S3 output matrix",
            payload.len()
        );
        assert_eq!(
            decode_spi(&edges, MODE0),
            payload.to_vec(),
            "the captured waveform must decode back to the W-buffer payload. \
             ⚠️ The S3's non-DMA path OVERWRITES W0..W15 with the idle-MISO \
             0xFF; a narration taken after that fill decodes to all-ones"
        );
    }

    #[test]
    fn s3_uart_characters_reach_a_matrix_routed_tx_pad() {
        let mut m = s3_uart_machine();
        route(&mut m, TX_PIN, S3_SIG_U0TXD);
        watch_tx(&mut m);

        let msg = b"S3";
        uart_send(&mut m, UART0_BASE, msg, S3_UART_BIT);

        let edges = m.logic_read_edges(0).edges;
        assert!(!edges.is_empty(), "the routed S3 TX pad recorded NOTHING");
        assert_eq!(
            decode_uart(&edges, S3_UART_BIT),
            msg.to_vec(),
            "the S3 UART runs at 240 MHz, so the same CLKDIV is a different \
             number of engine cycles per bit than on the C3"
        );
    }

    // ================= classic ESP32 (LX6) =================
    //
    // Its own bases, its own register offsets, its own signal indices. Nothing
    // in this block may be expressed in terms of the C3/S3 constants above —
    // that reuse IS the bug class being gated.

    /// `DR_REG_GPIO_BASE` / `DR_REG_SPI3_BASE` (VSPI) / `DR_REG_UART_BASE`.
    const CL_GPIO_BASE: u64 = 0x3FF4_4000;
    const CL_SPI3_BASE: u64 = 0x3FF6_5000;
    const CL_UART0_BASE: u64 = 0x3FF4_0000;

    /// `GPIO_FUNC0_OUT_SEL_CFG_REG` — classic `gpio_reg.h`; 0x530, NOT the
    /// C3/S3's 0x554.
    const CL_FUNC_OUT_SEL: u64 = 0x530;

    // --- classic SPI registers (`soc/esp32/include/soc/spi_reg.h`) ----------
    const CL_SPI_CMD: u64 = 0x00;
    /// `SPI_CLOCK_REG` :328 — 0x18, not 0x0C.
    const CL_SPI_CLOCK: u64 = 0x18;
    /// `SPI_USER_REG` :364.
    const CL_SPI_USER: u64 = 0x1C;
    /// `SPI_MOSI_DLEN_REG` — sizes the MOSI stream in bits minus one.
    const CL_SPI_MOSI_DLEN: u64 = 0x28;
    /// `SPI_PIN_REG` :587. Classic has no `SPI_MISC`.
    const CL_SPI_PIN: u64 = 0x34;
    /// `SPI_W0` — the 64-byte FIFO starts at 0x80 here, not 0x98.
    const CL_SPI_W0: u64 = 0x80;
    /// `SPI_USR` :116 — CMD bit 18, not the C3/S3's bit 24.
    const CL_SPI_USR: u32 = 1 << 18;
    /// `SPI_USR_MOSI` — USER bit 27.
    const CL_USER_USR_MOSI: u32 = 1 << 27;
    /// `SPI_CK_OUT_EDGE` :506 — USER bit **7** (CPHA). The C3/S3 use bit 9.
    const CL_CK_OUT_EDGE: u32 = 1 << 7;
    /// `SPI_CK_IDLE_EDGE` :599 — `SPI_PIN` bit 29 (CPOL).
    const CL_CK_IDLE_EDGE: u32 = 1 << 29;

    // --- classic matrix signal indices (`gpio_sig_map.h`) -------------------
    /// `VSPICLK_OUT_IDX` :138 / `VSPID_OUT_IDX` :142 / `VSPICS0_OUT_IDX` :148.
    /// ⚠️ Numerically identical to the C3's FSPI trio, and meaningless on the
    /// S3 — see the module header.
    const CL_SIG_SCK: u32 = 63;
    const CL_SIG_MOSI: u32 = 65;
    const CL_SIG_CS: u32 = 68;
    /// `U0TXD_OUT_IDX` :46. The C3's is 6 and the S3's is 12.
    const CL_SIG_U0TXD: u32 = 14;
    /// `SIG_GPIO_OUT_IDX` :399 — the matrix-bypass sentinel, 256 on classic.
    const CL_SIG_GPIO_OUT: u32 = 256;

    /// The WROOM-32 VSPI pin-out (`variants/esp32/pins_arduino.h`: SCK 18,
    /// MOSI 23, SS 5) and GPIO4 for TX. Real pads rather than the C3 block's
    /// 6/7/10, which are the classic part's flash pins.
    const CL_SCK_PIN: u8 = 18;
    const CL_MOSI_PIN: u8 = 23;
    const CL_CS_PIN: u8 = 5;
    const CL_TX_PIN: u8 = 4;

    /// 1 MHz from the 80 MHz APB, spelled with a divisor pair that DOES NOT FIT
    /// the C3/S3's four-bit `CLKDIV_PRE`: `pre_reg` = 19 (÷20) with `CLKCNT_N` =
    /// 3 (÷4) gives ÷80. Classic's field is [30:18], THIRTEEN bits (`spi_reg.h`
    /// :341), so a model masking it with the sibling's 0xF reads `pre_reg` = 3
    /// and narrates at five times the programmed rate — carrying the SAME bytes,
    /// because an edge-driven decoder is rate-blind. Hence the separate rate
    /// assertion in the test below.
    const CL_SPI_CLOCK_1MHZ: u32 = (19 << 18) | (3 << 12);
    /// One bit period at that setting: 80 APB ticks × (240/80).
    const CL_SPI_BIT: u64 = 240;
    /// 694 × 240 / 80 — `Esp32Uart::cycles_per_byte` / 10.
    const CL_UART_BIT: u64 = 694 * 240 / 80;

    fn classic_spi_machine() -> Machine<CortexM> {
        use crate::peripherals::esp32::gpio::Esp32Gpio;
        use crate::peripherals::esp32::spi::Esp32Spi;
        let mut bus = crate::bus::SystemBus::new();
        bus.add_peripheral(
            "gpio",
            CL_GPIO_BASE,
            0x1000,
            None,
            Box::new(Esp32Gpio::new()),
        );
        bus.add_peripheral(
            "spi3",
            CL_SPI3_BASE,
            0x1000,
            None,
            Box::new(Esp32Spi::new()),
        );
        bus.wire_esp32_spi_pads();
        finish(bus)
    }

    fn classic_uart_machine() -> Machine<CortexM> {
        use crate::peripherals::esp32::gpio::Esp32Gpio;
        use crate::peripherals::esp32::uart::Esp32Uart;
        let mut bus = crate::bus::SystemBus::new();
        bus.add_peripheral(
            "gpio",
            CL_GPIO_BASE,
            0x1000,
            None,
            Box::new(Esp32Gpio::new()),
        );
        bus.add_peripheral(
            "uart0",
            CL_UART0_BASE,
            0x100,
            None,
            Box::new(Esp32Uart::new(false, 34)),
        );
        bus.wire_esp32_uart_pads();
        finish(bus)
    }

    /// `gpio_matrix_out(pin, signal, false, false)` plus the output-driver
    /// enable — the classic `pinMatrixOutAttach` after a `pinMode(_, OUTPUT)`.
    /// The ENABLE write is load-bearing: `Esp32Gpio::matrix_signal` refuses a
    /// pad whose driver is off, exactly as `i2cInit` sets direction first.
    fn classic_route(machine: &mut Machine<CortexM>, pin: u8, signal: u32) {
        machine
            .bus
            .write_u32(CL_GPIO_BASE + ENABLE_W1TS, 1u32 << pin)
            .unwrap();
        machine
            .bus
            .write_u32(CL_GPIO_BASE + CL_FUNC_OUT_SEL + u64::from(pin) * 4, signal)
            .unwrap();
    }

    fn classic_spi_transfer(machine: &mut Machine<CortexM>, payload: &[u8], mode: Mode) {
        {
            let bus = &mut machine.bus;
            bus.write_u32(CL_SPI3_BASE + CL_SPI_CLOCK, CL_SPI_CLOCK_1MHZ)
                .unwrap();
            bus.write_u32(
                CL_SPI3_BASE + CL_SPI_PIN,
                if mode.cpol { CL_CK_IDLE_EDGE } else { 0 },
            )
            .unwrap();
            bus.write_u32(
                CL_SPI3_BASE + CL_SPI_USER,
                CL_USER_USR_MOSI | if mode.cpha { CL_CK_OUT_EDGE } else { 0 },
            )
            .unwrap();
            bus.write_u32(
                CL_SPI3_BASE + CL_SPI_MOSI_DLEN,
                (payload.len() as u32 * 8) - 1,
            )
            .unwrap();
            for (w, chunk) in payload.chunks(4).enumerate() {
                let mut word = 0u32;
                for (b, &byte) in chunk.iter().enumerate() {
                    word |= u32::from(byte) << (8 * b);
                }
                bus.write_u32(CL_SPI3_BASE + CL_SPI_W0 + (w as u64) * 4, word)
                    .unwrap();
            }
            bus.write_u32(CL_SPI3_BASE + CL_SPI_CMD, CL_SPI_USR)
                .unwrap();
        }
        // The model completes the whole transaction inside that last write; the
        // WIRE needs ten bit periods per byte. Running that time out is the
        // point — it is what makes this a measurement and not a mirror.
        let wire = payload.len() as u64 * SPI_FRAME_BITS * CL_SPI_BIT;
        for _ in 0..wire + 256 {
            machine.step().unwrap();
        }
    }

    fn classic_uart_send(machine: &mut Machine<CortexM>, bytes: &[u8]) {
        {
            let bus = &mut machine.bus;
            bus.write_u32(CL_UART0_BASE + UART_CLKDIV, UART_CLKDIV_115200)
                .unwrap();
            for &b in bytes {
                bus.write_u32(CL_UART0_BASE + UART_FIFO, u32::from(b))
                    .unwrap();
            }
        }
        let wire = bytes.len() as u64 * UART_FRAME_BITS * CL_UART_BIT;
        for _ in 0..wire + 256 {
            machine.step().unwrap();
        }
    }

    fn watch_classic_spi(machine: &mut Machine<CortexM>) {
        let gpio = machine.bus.find_peripheral_index_by_name("gpio").unwrap();
        machine.logic_watch(&[
            Some(LogicSource::pad(gpio, CL_MOSI_PIN)),
            Some(LogicSource::pad(gpio, CL_SCK_PIN)),
            Some(LogicSource::pad(gpio, CL_CS_PIN)),
        ]);
    }

    fn watch_classic_tx(machine: &mut Machine<CortexM>) {
        let gpio = machine.bus.find_peripheral_index_by_name("gpio").unwrap();
        machine.logic_watch(&[Some(LogicSource::pad(gpio, CL_TX_PIN))]);
    }

    #[test]
    fn classic_spi_bytes_reach_the_matrix_routed_pads() {
        let mut m = classic_spi_machine();
        classic_route(&mut m, CL_SCK_PIN, CL_SIG_SCK);
        classic_route(&mut m, CL_MOSI_PIN, CL_SIG_MOSI);
        classic_route(&mut m, CL_CS_PIN, CL_SIG_CS);
        watch_classic_spi(&mut m);

        // ⚠️ Bit-ASYMMETRIC on purpose. 0xA5, 0x5A, 0x3C, 0xFF and 0x00 are all
        // bit-palindromes: a payload of those decodes identically MSB- and
        // LSB-first, so a gate built on them cannot see a reversed narration at
        // all. 0x01/0xB2/0x0F reverse to 0x80/0x4D/0xF0.
        let payload = [0x01u8, 0xB2, 0x0F, 0x80];
        classic_spi_transfer(&mut m, &payload, MODE0);

        let edges = m.logic_read_edges(0).edges;
        assert!(
            !edges.is_empty(),
            "VSPI clocked {} bytes and the routed pads recorded NOTHING — the \
             wire is not reaching the classic-ESP32 output matrix",
            payload.len()
        );
        assert_eq!(
            decode_spi(&edges, MODE0),
            payload.to_vec(),
            "the captured waveform must decode back to exactly what the W buffer \
             carried, MSB-first"
        );

        // ⚠️ And at the RATE the firmware programmed. `decode_spi` replays edges
        // and is deliberately rate-BLIND, so a divisor read out of the wrong
        // register (classic `SPI_CLOCK` is 0x18, the C3/S3's is 0x0C) or masked
        // to the wrong width (thirteen bits here, four there) decodes to exactly
        // the right bytes at a frequency nothing on the bus ever ran at.
        // Measured back off the capture the way a user reading the analyzer
        // would: the tightest spacing between two clock edges IS one half
        // period.
        let mut sck: Vec<u64> = edges
            .iter()
            .filter(|e| e.ch == CH_SCK)
            .map(|e| e.cycle)
            .collect();
        sck.sort_unstable();
        let tightest = sck.windows(2).map(|w| w[1] - w[0]).min();
        assert_eq!(
            tightest,
            Some(CL_SPI_BIT / 2),
            "the trace measures {tightest:?} cycles between the closest pair of \
             SCK edges; one half of the programmed {CL_SPI_BIT}-cycle period is \
             {}. A trace at the wrong frequency carrying the right bytes is what \
             a divisor decoded from a sibling part's register geometry looks \
             like.",
            CL_SPI_BIT / 2
        );
    }

    /// The classic mode bits sit at register positions the C3/S3 do not share:
    /// CPHA is `SPI_USER` bit 7 (theirs is bit 9) and CPOL is `SPI_PIN` bit 29
    /// (they have no `SPI_PIN` and keep it in `SPI_MISC`). Reading either from a
    /// sibling's position yields a trace at the right rate framed on the wrong
    /// phase — plausible, and garbage.
    #[test]
    fn classic_spi_honours_the_mode_bits_in_pin_and_user() {
        let payload = [0x12u8, 0x34]; // bit-asymmetric: reverses to 0x48/0x2C

        let mut m = classic_spi_machine();
        classic_route(&mut m, CL_SCK_PIN, CL_SIG_SCK);
        classic_route(&mut m, CL_MOSI_PIN, CL_SIG_MOSI);
        classic_route(&mut m, CL_CS_PIN, CL_SIG_CS);
        watch_classic_spi(&mut m);
        classic_spi_transfer(&mut m, &payload, MODE3);
        let mode3 = m.logic_read_edges(0).edges;
        assert_eq!(
            decode_spi(&mode3, MODE3),
            payload.to_vec(),
            "a mode-3 transfer must decode as mode 3"
        );

        let mut m = classic_spi_machine();
        classic_route(&mut m, CL_SCK_PIN, CL_SIG_SCK);
        classic_route(&mut m, CL_MOSI_PIN, CL_SIG_MOSI);
        classic_route(&mut m, CL_CS_PIN, CL_SIG_CS);
        watch_classic_spi(&mut m);
        classic_spi_transfer(&mut m, &payload, MODE0);
        let mode0 = m.logic_read_edges(0).edges;

        // CPOL (`SPI_PIN.CK_IDLE_EDGE`): the level SCK rests at once the burst
        // is over. A narration that read USER instead parks it at 0 either way.
        let rest = |edges: &[LogicEdge]| edges.iter().rfind(|e| e.ch == CH_SCK).map(|e| e.value);
        assert_eq!(rest(&mode3), Some(true), "CPOL=1 must idle SCK HIGH");
        assert_eq!(rest(&mode0), Some(false), "CPOL=0 must idle SCK LOW");

        // CPHA (`SPI_USER.CK_OUT_EDGE`, bit 7 here): the phase decides
        // chip-select framing. A model reading bit 9 sees CPHA=0 for both modes
        // and pulses CS per word in each.
        let cs_edges = |edges: &[LogicEdge]| edges.iter().filter(|e| e.ch == CH_CS).count();
        assert_eq!(
            cs_edges(&mode0),
            2 * payload.len(),
            "CPHA=0 must pulse chip select once per word"
        );
        assert_eq!(
            cs_edges(&mode3),
            2,
            "CPHA=1 must HOLD chip select low ACROSS the burst — one assert and \
             one release, whatever its length. More than that means the \
             narration read CK_OUT_EDGE from the wrong bit and re-framed every \
             word."
        );
    }

    #[test]
    fn classic_uart_characters_reach_a_matrix_routed_tx_pad() {
        let mut m = classic_uart_machine();
        classic_route(&mut m, CL_TX_PIN, CL_SIG_U0TXD);
        watch_classic_tx(&mut m);

        // ⚠️ Bit-asymmetric characters: 'L' (0x4C) reverses to 0x32, 'W' (0x57)
        // to 0xEA. A palindromic message would decode the same LSB- and
        // MSB-first and could not see a reversed frame.
        let msg = b"LW";
        classic_uart_send(&mut m, msg);

        let edges = m.logic_read_edges(0).edges;
        assert!(
            !edges.is_empty(),
            "UART0 shifted {} characters and the routed TX pad recorded NOTHING",
            msg.len()
        );
        assert_eq!(
            decode_uart(&edges, CL_UART_BIT),
            msg.to_vec(),
            "the captured waveform must decode as 8N1 at the programmed baud \
             rate — 694 CLKDIV against an 80 MHz APB on a 240 MHz core"
        );
    }

    /// A pad the firmware kept for plain GPIO must keep reading the output
    /// latch. The control case for a wire bound to every output-capable pad:
    /// binding is only correct if the selector gates it.
    #[test]
    fn a_classic_pad_left_at_the_bypass_sentinel_still_reads_the_gpio_latch() {
        let mut m = classic_spi_machine();
        classic_route(&mut m, CL_SCK_PIN, CL_SIG_SCK);
        classic_route(&mut m, CL_MOSI_PIN, CL_SIG_MOSI);
        // CS enabled as an output but left at SIG_GPIO_OUT — a plain GPIO.
        classic_route(&mut m, CL_CS_PIN, CL_SIG_GPIO_OUT);
        watch_classic_spi(&mut m);
        classic_spi_transfer(&mut m, &[0x01, 0xB2], MODE0);

        let edges = m.logic_read_edges(0).edges;
        assert!(
            edges.iter().any(|e| e.ch == CH_SCK),
            "the routed clock pad must still carry the bus"
        );
        assert!(
            !edges.iter().any(|e| e.ch == CH_CS),
            "a pad left at SIG_GPIO_OUT (256 on classic) must show its latch, \
             never the controller's chip select"
        );
    }

    /// ⚠️ The classic half of the per-chip index trap.
    ///
    /// VSPI's 63/65/68 collide numerically with the C3's FSPI trio, so the SPI
    /// half is crossed against the **S3** — where 101/103/110 mean FSPI and 63
    /// is not a SPI signal at all. The UART half is crossed against the C3,
    /// where the numbers genuinely differ (classic `U0TXD` = 14, C3 = 6) and a
    /// borrowed constant would be silent in both directions.
    #[test]
    fn the_classic_matrix_indices_are_its_own() {
        let mut m = classic_spi_machine();
        classic_route(&mut m, CL_SCK_PIN, S3_SIG_SCK); // 101 — nothing on classic
        classic_route(&mut m, CL_MOSI_PIN, S3_SIG_MOSI);
        classic_route(&mut m, CL_CS_PIN, S3_SIG_CS);
        watch_classic_spi(&mut m);
        classic_spi_transfer(&mut m, &[0x01, 0xB2], MODE0);
        assert!(
            m.logic_read_edges(0).edges.is_empty(),
            "classic ESP32 routed the S3's FSPI indices (101/103/110) and still \
             showed traffic — the classic GPIO is decoding an index space that \
             is not its own"
        );

        let mut m = classic_uart_machine();
        classic_route(&mut m, CL_TX_PIN, C3_SIG_U0TXD); // 6 — the C3's U0TXD
        watch_classic_tx(&mut m);
        classic_uart_send(&mut m, b"LW");
        assert!(
            m.logic_read_edges(0).edges.is_empty(),
            "classic ESP32 routed the C3's U0TXD index (6) and still showed \
             traffic; classic U0TXD is 14"
        );

        // …and the mirror image: the C3, routed with the classic index.
        let mut m = c3_uart_machine();
        route(&mut m, TX_PIN, CL_SIG_U0TXD); // 14 — classic's U0TXD
        watch_tx(&mut m);
        uart_send(&mut m, UART0_BASE, b"LW", C3_UART_BIT);
        assert!(
            m.logic_read_edges(0).edges.is_empty(),
            "the C3 routed the classic part's U0TXD index (14) and still showed \
             traffic; 14 is not U0TXD on the C3"
        );
    }

    // ================= the traps =================

    /// ⚠️ The single most dangerous failure mode on this family: matrix signal
    /// indices are PER CHIP, and a borrowed constant is silent in BOTH
    /// directions.
    ///
    /// Route each part's pads with the OTHER part's numbers and assert nothing
    /// is captured. If either half of this ever passes traffic, some model is
    /// decoding a sibling's index space.
    #[test]
    fn the_matrix_indices_are_per_chip_and_do_not_cross() {
        let mut m = c3_spi_machine();
        route(&mut m, SCK_PIN, S3_SIG_SCK); // 101 — meaningless on the C3
        route(&mut m, MOSI_PIN, S3_SIG_MOSI);
        route(&mut m, CS_PIN, S3_SIG_CS);
        watch_spi(&mut m);
        spi_transfer(&mut m, &[0xA5, 0x01], MODE0, C3_SPI_BIT);
        assert!(
            m.logic_read_edges(0).edges.is_empty(),
            "the C3 routed the S3's FSPI indices (101/103/110) and still showed \
             traffic — the C3 GPIO is decoding an index space that is not its own"
        );

        let mut m = s3_spi_machine();
        route(&mut m, SCK_PIN, C3_SIG_SCK); // 63 — not a SPI signal on the S3
        route(&mut m, MOSI_PIN, C3_SIG_MOSI);
        route(&mut m, CS_PIN, C3_SIG_CS);
        watch_spi(&mut m);
        spi_transfer(&mut m, &[0xA5, 0x01], MODE0, S3_SPI_BIT);
        assert!(
            m.logic_read_edges(0).edges.is_empty(),
            "the S3 routed the C3's FSPI indices (63/65/68) and still showed \
             traffic — 63 is not a SPI signal on this part"
        );
    }

    /// A pad the firmware kept for plain GPIO must keep reading the output
    /// latch, not the bus. The control case for every binding above: a wire
    /// bound to every pad is only correct if the selector gates it.
    #[test]
    fn an_unrouted_pad_still_reads_the_gpio_latch() {
        let mut m = c3_spi_machine();
        // SCK and MOSI routed; CS deliberately left as a plain GPIO output.
        route(&mut m, SCK_PIN, C3_SIG_SCK);
        route(&mut m, MOSI_PIN, C3_SIG_MOSI);
        m.bus
            .write_u32(GPIO_BASE + ENABLE_W1TS, 1u32 << CS_PIN)
            .unwrap();
        watch_spi(&mut m);
        spi_transfer(&mut m, &[0xA5, 0x01], MODE0, C3_SPI_BIT);

        let edges = m.logic_read_edges(0).edges;
        assert!(
            edges.iter().any(|e| e.ch == CH_SCK),
            "the routed clock pad must still carry the bus"
        );
        assert!(
            !edges.iter().any(|e| e.ch == CH_CS),
            "a pad left at SIG_GPIO_OUT must show its latch, never the \
             controller's chip select — a route that ignores the selector makes \
             every plain GPIO on the chip report bus traffic"
        );
    }

    // ================= production paths =================

    /// The bus a REAL C3 lab is built from must have its SPI and UART pads
    /// bound.
    ///
    /// Every test above hand-builds a bus and calls the wiring itself, which is
    /// exactly how the S3's I²C binding stayed green while being dark in
    /// production. This one asserts on `from_config` — the path
    /// `configs/chips/esp32c3.yaml` really takes — and reads the routes through
    /// the same `bound_pad_functions` hook the bus-visibility scoreboard uses,
    /// so the two can never disagree about whether a chip can show its bus.
    #[test]
    fn the_production_c3_builder_routes_the_spi_and_uart_pads() {
        use labwired_config::{ChipDescriptor, SystemManifest};
        let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .join("configs/chips/esp32c3.yaml");
        let chip = ChipDescriptor::from_file(&path).expect("load esp32c3.yaml");
        // The same minimal manifest `crates/core/tests/bus_visibility.rs` uses,
        // so this test and the scoreboard measure the same construction path.
        let manifest = SystemManifest {
            parts: Vec::new(),
            walk_deleted: Some(false),
            schema_version: "1.0".to_string(),
            name: "esp-spi-uart-waveform".to_string(),
            chip: path.to_string_lossy().to_string(),
            external_devices: vec![],
            cosim_models: Vec::new(),
            motor_models: Vec::new(),
            board_io: vec![],
            debug_uart: None,
            wifi_ap: None,
            peripherals: vec![],
            memory_overrides: Default::default(),
        };
        let bus = crate::bus::SystemBus::from_config(&chip, &manifest)
            .expect("assemble the production C3 bus");
        let bound = bus.bound_pad_functions();
        for want in ["SPI2_SCK", "SPI2_MOSI", "SPI2_CS", "UART0_TX", "UART1_TX"] {
            assert!(
                bound.contains(&want),
                "SystemBus::from_config must bind {want} on the C3, or every C3 \
                 lab reads a flat line while the bus is busy; bound functions \
                 were {bound:?}"
            );
        }
    }

    /// The same question for classic ESP32, asked of `configure_xtensa_esp32` —
    /// the programmatic builder every real classic lab is made from, which
    /// registers its peripheral bank in Rust and bypasses the chip yaml.
    ///
    /// ⚠️ This is the gate that matters. Every waveform test above hand-builds a
    /// bus and calls the wiring itself, which proves the narration works and NOT
    /// that anything ships it — exactly how the S3's I²C binding stayed green in
    /// test while being dark in every production lab.
    #[test]
    fn the_production_classic_esp32_builder_routes_the_spi_and_uart_pads() {
        let mut bus = crate::bus::SystemBus::new();
        let _cpu = crate::system::xtensa::configure_xtensa_esp32(&mut bus);
        let bound = bus.bound_pad_functions();
        for want in [
            "SPI3_SCK",
            "SPI3_MOSI",
            "SPI3_CS",
            "UART0_TX",
            "UART1_TX",
            "UART2_TX",
        ] {
            assert!(
                bound.contains(&want),
                "configure_xtensa_esp32 must bind {want}, or every classic-ESP32 \
                 lab reads a flat line while the bus is busy; bound functions \
                 were {bound:?}"
            );
        }
    }

    /// And of `from_config`, which is the path
    /// `crates/core/tests/bus_visibility.rs` scores the fleet on — so the board
    /// and this file can never disagree about whether the classic part can show
    /// its buses.
    ///
    /// ⚠️ `UART0_TX` is deliberately absent from the list.
    /// `configs/chips/esp32.yaml` declares `uart0` as the vendor-neutral `uart`
    /// type (the STM32 register map at a classic base), not `esp32_uart`, so a
    /// `from_config` build has no classic UART0 model to route. UART1/UART2 are
    /// `esp32_uart` and do bind, which is what turns the board's UART cell. The
    /// programmatic builder gated above covers all three.
    #[test]
    fn the_production_classic_from_config_build_routes_the_spi_and_uart_pads() {
        use labwired_config::{ChipDescriptor, SystemManifest};
        let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .join("configs/chips/esp32.yaml");
        let chip = ChipDescriptor::from_file(&path).expect("load esp32.yaml");
        let manifest = SystemManifest {
            parts: Vec::new(),
            walk_deleted: Some(false),
            schema_version: "1.0".to_string(),
            name: "esp-spi-uart-waveform".to_string(),
            chip: path.to_string_lossy().to_string(),
            external_devices: vec![],
            cosim_models: Vec::new(),
            motor_models: Vec::new(),
            board_io: vec![],
            debug_uart: None,
            wifi_ap: None,
            peripherals: vec![],
            memory_overrides: Default::default(),
        };
        let bus = crate::bus::SystemBus::from_config(&chip, &manifest)
            .expect("assemble the production classic-ESP32 bus");
        let bound = bus.bound_pad_functions();
        for want in ["SPI3_SCK", "SPI3_MOSI", "SPI3_CS", "UART1_TX", "UART2_TX"] {
            assert!(
                bound.contains(&want),
                "SystemBus::from_config must bind {want} on classic ESP32; bound \
                 functions were {bound:?}"
            );
        }
    }

    /// The same question for the S3, asked of `configure_xtensa_esp32s3`.
    ///
    /// ⚠️ NOT `from_config`, and that is deliberate: `configs/chips/esp32s3.yaml`
    /// is an address-map stub — `gpio` is `type: "declarative"`, `uart0` is the
    /// vendor-neutral `uart` type, and there is no `spi2` entry at all — so a
    /// `from_config` build has no S3 model to route and this programmatic
    /// builder is what every real S3 lab is made from. Gating on `from_config`
    /// here would assert on a bus nobody runs.
    #[test]
    fn the_production_s3_builder_routes_the_spi_and_uart_pads() {
        let mut bus = crate::bus::SystemBus::new();
        let _wiring = crate::system::xtensa::configure_xtensa_esp32s3(
            &mut bus,
            &crate::system::xtensa::Esp32s3Opts::default(),
        );
        let bound = bus.bound_pad_functions();
        for want in [
            "SPI2_SCK",
            "SPI2_MOSI",
            "SPI2_CS",
            "UART0_TX",
            "UART1_TX",
            "UART2_TX",
        ] {
            assert!(
                bound.contains(&want),
                "configure_xtensa_esp32s3 must bind {want}, or every S3 lab reads \
                 a flat line while the bus is busy; bound functions were {bound:?}"
            );
        }
    }
}
