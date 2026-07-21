// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! End-to-end waveform test for the STM32 bit-level SPI engine: arm the
//! in-engine logic analyzer on the AFR-routed L4 SPI1 SCK/MOSI pads (PA5/PA7
//! on AF5, the nokia5110 lab wiring), run a PCD8544-style byte stream through
//! the controller's registers exactly as the firmware driver does (poll TXE,
//! write DR, poll BSY), and assert the captured edge stream is a valid mode-0
//! SPI waveform whose decoded MOSI byte pattern matches the byte-level
//! bus-trace monitor byte for byte. The waveform is sampled through the
//! normal `read_gpio_pad` path — nothing is synthesized into the ring.

#[cfg(test)]
mod stm32_spi_waveform_tests {
    use crate::bus::bus_trace::BusPayload;
    use crate::cpu::CortexM;
    use crate::logic_capture::LogicEdge;
    use crate::peripherals::gpio::{GpioPort, GpioRegisterLayout};
    use crate::peripherals::spi::{Spi, SpiDevice, SpiRegisterLayout};
    #[cfg(feature = "event-scheduler")]
    use crate::DebugControl;
    use crate::{Bus, Machine};

    const RAM_BASE: u64 = 0x2000_0000;
    /// The default test-bus gpioa entry (its dev is swapped for a V2 port).
    const GPIOA_BASE: u64 = 0x4001_0800;
    const SPI1_BASE: u64 = 0x4001_3000;

    const MODER: u64 = 0x00;
    const AFRL: u64 = 0x20;
    const CR1: u64 = SPI1_BASE;
    const SR: u64 = SPI1_BASE + 0x08;
    const DR: u64 = SPI1_BASE + 0x0C;

    /// nokia5110-invaders-lab wiring: PA5 = SPI1_SCK, PA7 = SPI1_MOSI (AF5).
    const SCK_PIN: u8 = 5;
    const MOSI_PIN: u8 = 7;
    const CH_MOSI: u32 = 0;
    const CH_SCK: u32 = 1;

    /// PCD8544-style stream: extended-mode init (0x21 0xBF 0x04 0x14),
    /// basic mode + normal video (0x20 0x0C), one framebuffer byte.
    const WIRE_BYTES: [u8; 7] = [0x21, 0xBF, 0x04, 0x14, 0x20, 0x0C, 0xA5];

    /// Write-only display-style device (PCD8544 never drives MISO).
    struct PanelSink;
    impl SpiDevice for PanelSink {
        fn transfer(&mut self, _mosi: u8) -> u8 {
            0
        }
        fn cs_pin(&self) -> &str {
            "PB6"
        }
    }

    /// Build a machine whose bus carries an L4-style SPI1 (FIFO layout, with a
    /// traced write-only panel) and a V2 GPIOA wired through
    /// `wire_stm32_spi_pads`. The CPU chews a NOP loop so `step()` advances
    /// cycles deterministically.
    fn machine() -> Machine<CortexM> {
        let mut bus = crate::bus::SystemBus::new();
        let (cpu, _nvic) = crate::system::cortex_m::configure_cortex_m(&mut bus);
        // The default test bus ships an F1-layout "gpioa"; this lab is an L4
        // (V2 registers), so swap the device in place.
        let gpioa_idx = bus.find_peripheral_index_by_name("gpioa").unwrap();
        bus.peripherals[gpioa_idx].dev =
            Box::new(GpioPort::new_with_layout(GpioRegisterLayout::Stm32V2));
        bus.add_peripheral(
            "spi1",
            SPI1_BASE,
            0x400,
            None,
            Box::new(Spi::new_with_layout(SpiRegisterLayout::Stm32Fifo)),
        );
        // Through the single traced choke point, same as a config-built bus.
        bus.attach_spi_device("spi1", Box::new(PanelSink)).unwrap();
        bus.wire_stm32_spi_pads();

        let mut machine = Machine::new(cpu, bus);
        // NOP slab (`movs r0, #0`) with a Thumb `b` back to the start.
        for i in 0..1022u64 {
            let byte = if i % 2 == 0 { 0x00 } else { 0x20 };
            machine.bus.write_u8(RAM_BASE + i, byte).unwrap();
        }
        machine.bus.write_u8(RAM_BASE + 1022, 0xFF).unwrap();
        machine.bus.write_u8(RAM_BASE + 1023, 0xE5).unwrap();
        machine.cpu.pc = RAM_BASE as u32;
        machine
    }

    /// Route the pads and bring up SPI1 exactly as the nokia firmware does:
    /// PA5/PA7 → AF mode, AFRL nibbles = 5; CR1 = MSTR|SSM|SSI|BR=/4|SPE
    /// (mode 0, MSB first, 8-bit frames via the CR2 reset 0x0700).
    fn configure(machine: &mut Machine<CortexM>) {
        let bus = &mut machine.bus;
        bus.write_u32(
            GPIOA_BASE + MODER,
            (0b10 << (SCK_PIN * 2)) | (0b10 << (MOSI_PIN * 2)),
        )
        .unwrap();
        bus.write_u32(
            GPIOA_BASE + AFRL,
            (5 << (SCK_PIN * 4)) | (5 << (MOSI_PIN * 4)),
        )
        .unwrap();
        bus.write_u16(CR1, (1 << 2) | (1 << 9) | (1 << 8) | (0x1 << 3) | (1 << 6))
            .unwrap();
    }

    /// Drive one byte the way `spi_write` in the lab firmware does: wait TXE,
    /// write DR (8-bit access — one frame on FIFO parts), wait BSY clear.
    fn spi_write(machine: &mut Machine<CortexM>, byte: u8) {
        for _ in 0..10_000 {
            if machine.bus.read_u16(SR).unwrap() & (1 << 1) != 0 {
                break;
            }
            machine.step().unwrap();
        }
        machine.bus.write_u8(DR, byte).unwrap();
        for _ in 0..10_000 {
            if machine.bus.read_u16(SR).unwrap() & (1 << 7) == 0 {
                return;
            }
            machine.step().unwrap();
        }
        panic!("BSY never cleared — the bit engine's wire time is wrong");
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
    fn logic_capture_sees_real_spi_waveform_matching_bus_trace() {
        let mut machine = machine();
        configure(&mut machine);

        let gpioa_idx = machine
            .bus
            .find_peripheral_index_by_name("gpioa")
            .expect("gpioa registered");
        let initial =
            machine.logic_watch(&[Some((gpioa_idx, MOSI_PIN)), Some((gpioa_idx, SCK_PIN))]);
        assert_eq!(
            initial,
            vec![Some(false), Some(false)],
            "idle mode-0 wire reads low (SCK = CPOL = 0) on both AFR-routed pads"
        );

        for b in WIRE_BYTES {
            spi_write(&mut machine, b);
        }

        let edges = machine.logic_read_edges(0).edges;
        assert!(
            !edges.is_empty(),
            "hardware-SPI pads must carry edges, not a flat trace"
        );
        assert!(edges.iter().any(|e| e.ch == CH_SCK), "SCK edges present");
        assert!(edges.iter().any(|e| e.ch == CH_MOSI), "MOSI edges present");

        // ── Clock structure: mode 0 samples on the rising edge; 8 rises per
        // byte, and SCK returns low (CPOL idle) after every frame.
        let rising: Vec<u64> = edges
            .iter()
            .filter(|e| e.ch == CH_SCK && e.value)
            .map(|e| e.cycle)
            .collect();
        assert_eq!(
            rising.len(),
            WIRE_BYTES.len() * 8,
            "8 SCK rising edges per byte"
        );
        let falling = edges.iter().filter(|e| e.ch == CH_SCK && !e.value).count();
        assert_eq!(
            falling,
            WIRE_BYTES.len() * 8,
            "every SCK pulse returns to the CPOL idle level"
        );

        // ── Decode MOSI at the mode-0 sample edges, MSB first.
        let bits: Vec<bool> = rising
            .iter()
            .map(|&c| level_at(false, &edges, CH_MOSI, c))
            .collect();
        let decoded: Vec<u8> = bits
            .chunks(8)
            .map(|chunk| chunk.iter().fold(0u8, |acc, &b| (acc << 1) | u8::from(b)))
            .collect();
        assert_eq!(
            decoded,
            WIRE_BYTES.to_vec(),
            "the clocked MOSI byte pattern must be the programmed stream"
        );

        // ── Waveform and byte-level bus monitor must agree: the same bytes,
        // in the same order, on the same bus.
        let traced: Vec<u8> = machine
            .bus
            .bus_trace_snapshot()
            .iter()
            .filter(|e| e.bus == "spi1")
            .filter_map(|e| match e.payload {
                BusPayload::Spi { mosi, .. } => Some(mosi),
                _ => None,
            })
            .collect();
        assert_eq!(
            traced,
            WIRE_BYTES.to_vec(),
            "bus monitor must record the same bytes the waveform clocked"
        );
        assert_eq!(
            decoded, traced,
            "waveform decode and bus-trace monitor must agree byte for byte"
        );
    }

    /// Analytic wire time through the machine: with BR=/4 an 8-bit frame is
    /// exactly 32 engine cycles on the wire — BSY reads busy for the whole
    /// window and clear right after.
    #[test]
    fn frame_wire_time_is_exact_through_the_machine() {
        let mut machine = machine();
        configure(&mut machine);
        machine.bus.write_u8(DR, 0xA5).unwrap();
        let mut busy_cycles = 0u64;
        while machine.bus.read_u16(SR).unwrap() & (1 << 7) != 0 {
            machine.step().unwrap();
            busy_cycles += 1;
            assert!(busy_cycles < 10_000, "frame never completed");
        }
        assert_eq!(
            busy_cycles, 32,
            "8 bits × 2^(BR+1) = 32 cycles at BR=1 — firmware-visible wire time"
        );
    }

    /// Phase 1.6 no-stretch gate, exact flavour: with the scheduler timebase
    /// in absolute cycles, a 32-cycle frame is STILL exactly 32 firmware-
    /// visible cycles at `peripheral_tick_interval = 64` when drains run per
    /// cycle (`step()`). Before the cycle-exact conversion the tick-index
    /// timebase reinterpreted the engine's cycle delays as tick counts and the
    /// same frame took ~2048 cycles (×interval stretch).
    #[cfg(feature = "event-scheduler")]
    #[test]
    fn frame_wire_time_does_not_stretch_at_interval_64() {
        let mut machine = machine();
        configure(&mut machine);
        machine.config.peripheral_tick_interval = 64;
        machine.bus.config.peripheral_tick_interval = 64;
        machine.bus.write_u8(DR, 0xA5).unwrap();
        let mut busy_cycles = 0u64;
        while machine.bus.read_u16(SR).unwrap() & (1 << 7) != 0 {
            machine.step().unwrap();
            busy_cycles += 1;
            assert!(busy_cycles < 10_000, "frame never completed");
        }
        assert_eq!(
            busy_cycles, 32,
            "cycle-denominated frame time must not scale with the tick interval"
        );
    }

    /// Phase 1.6 no-stretch gate, batched flavour: through the real
    /// `Machine::run` batch loop at interval 64, frame completion is applied
    /// at the first scheduler drain at/after its exact cycle — the
    /// firmware-visible wire time is within `N..N+interval` of the exact
    /// 32-cycle time, never `N×interval`.
    #[cfg(feature = "event-scheduler")]
    #[test]
    fn batched_frame_wire_time_within_one_interval_at_64() {
        let mut machine = machine();
        configure(&mut machine);
        machine.config.peripheral_tick_interval = 64;
        machine.bus.config.peripheral_tick_interval = 64;
        machine.bus.write_u8(DR, 0xA5).unwrap();
        let start = machine.total_cycles;
        let observed = loop {
            machine.run(Some(64)).unwrap();
            if machine.bus.read_u16(SR).unwrap() & (1 << 7) == 0 {
                break machine.total_cycles;
            }
            assert!(
                machine.total_cycles - start < 10_000,
                "frame never completed"
            );
        };
        let busy = observed - start;
        assert!(
            (32..=32 + 64).contains(&busy),
            "batched wire time must be the exact 32 cycles plus at most one \
             interval of drain quantisation, got {busy}"
        );
    }

    /// Phase 1.6: the captured SCK/MOSI waveform — every edge cycle — is
    /// byte-identical at tick interval 64 and interval 1 when observed
    /// per-cycle. Scheduled wire transitions land at their exact cycles at any
    /// interval, so batching cannot move them.
    #[cfg(feature = "event-scheduler")]
    #[test]
    fn waveform_edges_identical_across_tick_intervals() {
        let run = |interval: u32| {
            let mut machine = machine();
            configure(&mut machine);
            machine.config.peripheral_tick_interval = interval;
            machine.bus.config.peripheral_tick_interval = interval;
            let gpioa_idx = machine.bus.find_peripheral_index_by_name("gpioa").unwrap();
            machine.logic_watch(&[Some((gpioa_idx, MOSI_PIN)), Some((gpioa_idx, SCK_PIN))]);
            for b in WIRE_BYTES {
                spi_write(&mut machine, b);
            }
            machine.logic_read_edges(0).edges
        };
        assert_eq!(
            run(1),
            run(64),
            "per-cycle-observed SPI edges must not depend on the tick interval"
        );
    }

    /// The pads report the SPI wire ONLY while MODER/AFR route it: flipping
    /// PA5 back to a plain GPIO output detaches the pad from the engine.
    #[test]
    fn pad_follows_afr_routing() {
        let mut machine = machine();
        configure(&mut machine);
        let gpioa_idx = machine.bus.find_peripheral_index_by_name("gpioa").unwrap();
        let routing = machine.bus.peripherals[gpioa_idx]
            .dev
            .gpio_routing(SCK_PIN)
            .unwrap();
        assert_eq!(routing.func.as_deref(), Some("SPI1_SCK"));

        // Re-route PA5 to plain output with ODR bit set: the pad must now
        // read the GPIO latch (high), not the idle SPI clock (low).
        machine
            .bus
            .write_u32(GPIOA_BASE + 0x14, 1 << SCK_PIN)
            .unwrap();
        machine
            .bus
            .write_u32(
                GPIOA_BASE + MODER,
                (0b01 << (SCK_PIN * 2)) | (0b10 << (MOSI_PIN * 2)),
            )
            .unwrap();
        let pad = machine.bus.peripherals[gpioa_idx]
            .dev
            .read_gpio_pad(SCK_PIN);
        assert_eq!(pad, Some(true), "output-mode pad reads the ODR latch");
        let routing = machine.bus.peripherals[gpioa_idx]
            .dev
            .gpio_routing(SCK_PIN)
            .unwrap();
        assert!(routing.func.is_none(), "no SPI func on a plain output");
    }

    /// Determinism: identical run, identical edge stream.
    #[test]
    fn waveform_capture_is_deterministic() {
        let run = || {
            let mut machine = machine();
            configure(&mut machine);
            let gpioa_idx = machine.bus.find_peripheral_index_by_name("gpioa").unwrap();
            machine.logic_watch(&[Some((gpioa_idx, MOSI_PIN)), Some((gpioa_idx, SCK_PIN))]);
            for b in WIRE_BYTES {
                spi_write(&mut machine, b);
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
