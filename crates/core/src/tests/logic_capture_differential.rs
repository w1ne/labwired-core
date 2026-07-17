// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Differential oracle for the two logic-capture modes (`crate::logic_capture`):
//! run the SAME firmware twice — once forcing the per-cycle poll path
//! (`Machine::logic_force_poll_capture`), once with event-driven push capture —
//! and assert the edge streams are BYTE-IDENTICAL (same cycles, values,
//! order). Poll is the reference semantics; if push disagrees, push is wrong.
//!
//! Push instrumentation itself does not require poll capture's armed batch
//! clamp. This Cortex-M fixture is independently clamped to one instruction
//! per boundary by SCB reset fidelity, so both modes have the same batch
//! profile; the oracle here remains byte-identical edge streams. Under the
//! `event-scheduler` feature, it also proves push keeps idle fast-forward
//! enabled while probed.

#[cfg(test)]
mod logic_capture_differential_tests {
    use crate::cpu::CortexM;
    use crate::logic_capture::LogicEdge;
    use crate::peripherals::gpio::{GpioPort, GpioRegisterLayout};
    use crate::{Bus, DebugControl, Machine, Peripheral, SimResult};

    const GPIO_BASE: u64 = 0x5000_0000;
    const RAM_BASE: u64 = 0x2000_0000;
    /// V2-layout register offsets.
    const MODER: u64 = 0x00;
    const ODR: u64 = 0x14;

    /// Build a Cortex-M machine with a V2 GPIO at `GPIO_BASE`, pins 0..2 as
    /// outputs, and a str/str/branch bit-bang loop toggling pins 0+1 together
    /// through one ODR word write per store.
    fn bitbang_machine(tick_interval: u32) -> Machine<CortexM> {
        let mut bus = crate::bus::SystemBus::new();
        let (cpu, _nvic) = crate::system::cortex_m::configure_cortex_m(&mut bus);
        bus.add_peripheral(
            "gpio_test",
            GPIO_BASE,
            0x400,
            None,
            Box::new(GpioPort::new_with_layout(GpioRegisterLayout::Stm32V2)),
        );
        let mut machine = Machine::new(cpu, bus);
        machine.config.peripheral_tick_interval = tick_interval;

        // MODER pins 0..2 = output.
        machine
            .bus
            .write_u32(GPIO_BASE + MODER, 0b01_01_01)
            .unwrap();

        // r0 = &ODR, r1 = 0b11, r2 = 0;  loop: str r1,[r0]; str r2,[r0]; b loop
        machine.cpu.r0 = (GPIO_BASE + ODR) as u32;
        machine.cpu.r1 = 0b11;
        machine.cpu.r2 = 0;
        machine.bus.write_u16(RAM_BASE, 0x6001).unwrap(); // str r1, [r0]
        machine.bus.write_u16(RAM_BASE + 2, 0x6002).unwrap(); // str r2, [r0]
        machine.bus.write_u16(RAM_BASE + 4, 0xE7FC).unwrap(); // b .-8
        machine.cpu.pc = RAM_BASE as u32;
        machine
    }

    /// Watch pins [1, 0] (reversed, so channel order != pin order — this is
    /// what exercises the same-cycle cross-channel ordering rule), run
    /// `steps`, and return the edge stream plus the (batches, instructions)
    /// profile.
    fn run_bitbang(mut machine: Machine<CortexM>, force_poll: bool, steps: u32) -> RunResult {
        machine.logic_force_poll_capture(force_poll);
        let idx = machine
            .bus
            .find_peripheral_index_by_name("gpio_test")
            .unwrap();
        let initial = machine.logic_watch(&[Some((idx, 1)), Some((idx, 0))]);
        assert_eq!(initial, vec![Some(false), Some(false)]);
        machine.reset_step_profile();
        machine.run(Some(steps)).unwrap();
        let batch = machine.logic_read_edges(0);
        RunResult {
            edges: batch.edges,
            dropped: batch.dropped,
            batches: machine.step_profile().cpu_batches,
            instructions: machine.step_profile().cpu_instructions,
        }
    }

    struct RunResult {
        edges: Vec<LogicEdge>,
        dropped: u64,
        batches: u64,
        instructions: u64,
    }

    /// THE gate: bit-banged GPIO, wide tick interval. Poll capture requests a
    /// per-instruction clamp; push capture does not, but this Cortex-M fixture
    /// is independently clamped per instruction by SCB reset fidelity. Their
    /// edge streams must remain byte-identical.
    #[test]
    fn stm32_bitbang_push_stream_is_byte_identical_to_poll() {
        for tick_interval in [1u32, 8, 64] {
            let steps = 600;
            let poll = run_bitbang(bitbang_machine(tick_interval), true, steps);
            let push = run_bitbang(bitbang_machine(tick_interval), false, steps);

            assert!(
                poll.edges.len() >= 300,
                "tick={tick_interval}: expected a dense bit-bang stream, got {}",
                poll.edges.len()
            );
            assert_eq!(
                poll.edges, push.edges,
                "tick={tick_interval}: push edges must be byte-identical to poll edges"
            );
            assert_eq!(poll.dropped, push.dropped);

            // Poll capture requests a per-instruction clamp. Push capture does
            // not, but this fixture's SCB reset-fidelity clamp independently
            // keeps push execution at one instruction per boundary too.
            assert_eq!(
                poll.batches, poll.instructions,
                "tick={tick_interval}: poll fallback runs one instruction per batch"
            );
            if tick_interval > 1 {
                // This Cortex-M fixture contains an SCB, so the machine's
                // permanent reset-fidelity clamp intentionally keeps both
                // capture modes at one instruction per boundary.
                assert_eq!(
                    push.batches, push.instructions,
                    "tick={tick_interval}: SCB-equipped push capture must remain cycle-accurate"
                );
            }
        }
    }

    /// A legacy walker that charges one tick-cost cycle per walk tick — the
    /// dedicated cost source for the fixture below. (This role used to be
    /// played by an armed SysTick, whose `cycles: 1` per enabled tick was a
    /// sim artifact; the walk-free B1 batch normalised SysTick/SCB to zero
    /// cost so the walk-on reference and the scheduler path agree
    /// cycle-for-cycle. The logic-capture cost path itself is still real —
    /// any future peripheral may charge cost — so it keeps coverage here.)
    #[derive(Debug)]
    struct CostTicker;

    impl Peripheral for CostTicker {
        fn read(&self, _offset: u64) -> SimResult<u8> {
            Ok(0)
        }
        fn write(&mut self, _offset: u64, _value: u8) -> SimResult<()> {
            Ok(())
        }
        fn tick(&mut self) -> crate::PeripheralTickResult {
            crate::PeripheralTickResult {
                cycles: 1,
                ..Default::default()
            }
        }
    }

    /// Peripheral tick-COST cycles (the CostTicker charges one cycle per walk
    /// tick) shift the observation cycle past the batch boundary — the
    /// poll loop samples only after costs are charged, so push stamps
    /// finalised at the boundary must land on the same post-cost cycle. This
    /// is the only fixture that exercises the boundary→now finalisation with
    /// `now > boundary`.
    #[test]
    fn stm32_bitbang_with_tick_costs_push_stream_is_byte_identical_to_poll() {
        let build = |tick_interval: u32| {
            let mut machine = bitbang_machine(tick_interval);
            machine.bus.add_peripheral(
                "cost_ticker",
                0x5100_0000,
                0x100,
                None,
                Box::new(CostTicker),
            );
            machine
        };
        for tick_interval in [1u32, 8] {
            let poll = run_bitbang(build(tick_interval), true, 600);
            let push = run_bitbang(build(tick_interval), false, 600);
            assert!(poll.edges.len() >= 300);
            // The CostTicker's per-tick cost must actually be in play: with it, some
            // observation cycles exceed their instruction count.
            assert!(
                poll.edges.last().unwrap().cycle > 600,
                "tick costs must push observation cycles past the step count"
            );
            assert_eq!(
                poll.edges, push.edges,
                "tick={tick_interval}: push edges must match poll under tick costs"
            );
        }
    }

    /// Externally driven input pads (sim-input / button path): levels set via
    /// `set_gpio_input` while the machine is paused between run slices must
    /// land on the same cycle in both modes.
    #[test]
    fn stm32_input_pad_push_stream_is_byte_identical_to_poll() {
        let run = |force_poll: bool| {
            let mut machine = bitbang_machine(8);
            // Pin 3 stays MODER=input (00) — the externally driven channel.
            machine.logic_force_poll_capture(force_poll);
            let idx = machine
                .bus
                .find_peripheral_index_by_name("gpio_test")
                .unwrap();
            machine.logic_watch(&[Some((idx, 3)), Some((idx, 0))]);
            for slice in 0..6 {
                let level = slice % 2 == 0;
                assert!(machine.bus.peripherals[idx].dev.set_gpio_input(3, level));
                machine.run(Some(50)).unwrap();
            }
            machine.logic_read_edges(0).edges
        };
        let poll = run(true);
        let push = run(false);
        assert!(
            poll.iter().filter(|e| e.ch == 0).count() >= 5,
            "input channel must record the paused-machine level changes"
        );
        assert_eq!(poll, push, "input-pad edges must be byte-identical");
    }

    // ── ESP32-C3 I²C waveform (bit-engine push path) ────────────────────────

    const C3_RAM_BASE: u64 = 0x2000_0000;
    const I2C_BASE: u64 = 0x6001_3000;
    const C3_GPIO_BASE: u64 = 0x6000_4000;
    const SDA_PIN: u8 = 5;
    const SCL_PIN: u8 = 6;
    const SIG_I2CEXT0_SCL: u32 = 53;
    const SIG_I2CEXT0_SDA: u32 = 54;
    const ENABLE_W1TS: u64 = 0x24;
    const FUNC_IN_SEL: u64 = 0x154;
    const FUNC_OUT_SEL: u64 = 0x554;
    const MATRIX_INPUT_SELECT: u32 = 1 << 6;
    const REG_CTR: u64 = 0x04;
    const REG_DATA: u64 = 0x1C;
    const REG_CMD0: u64 = 0x58;
    const WIRE_BYTES: [u8; 3] = [0x78, 0x40, 0xA5];

    /// The ESP32-C3 SSD1306/I²C waveform fixture from
    /// `tests/esp32c3_i2c_waveform.rs`, driven through `Machine::run` batches.
    fn c3_i2c_machine(tick_interval: u32) -> Machine<CortexM> {
        use crate::peripherals::components::Ssd1306;
        let mut bus = crate::bus::SystemBus::new();
        let (cpu, _nvic) = crate::system::cortex_m::configure_cortex_m(&mut bus);
        bus.add_peripheral(
            "gpio",
            C3_GPIO_BASE,
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
        let route = std::collections::BTreeMap::from([
            ("sda".to_string(), format!("GPIO{SDA_PIN}")),
            ("scl".to_string(), format!("GPIO{SCL_PIN}")),
        ]);
        bus.attach_i2c_slave_with_route("i2c0", Box::new(Ssd1306::new(0x3C)), Some(&route))
            .unwrap();
        bus.wire_esp32c3_i2c_pads();

        let mut machine = Machine::new(cpu, bus);
        machine.config.peripheral_tick_interval = tick_interval;
        // The bus carries its own SimulationConfig copy; the peripheral walk
        // hands `tick_elapsed` the BUS interval, so keep both in sync or the
        // bit engine would advance slower than engine time.
        machine.bus.config.peripheral_tick_interval = tick_interval;
        for i in 0..1022u64 {
            let byte = if i % 2 == 0 { 0x00 } else { 0x20 };
            machine.bus.write_u8(C3_RAM_BASE + i, byte).unwrap();
        }
        machine.bus.write_u8(C3_RAM_BASE + 1022, 0xFF).unwrap();
        machine.bus.write_u8(C3_RAM_BASE + 1023, 0xE5).unwrap();
        machine.cpu.pc = C3_RAM_BASE as u32;

        let bus = &mut machine.bus;
        bus.write_u32(C3_GPIO_BASE + ENABLE_W1TS, (1 << SDA_PIN) | (1 << SCL_PIN))
            .unwrap();
        bus.write_u32(
            C3_GPIO_BASE + FUNC_OUT_SEL + (SDA_PIN as u64) * 4,
            SIG_I2CEXT0_SDA,
        )
        .unwrap();
        bus.write_u32(
            C3_GPIO_BASE + FUNC_OUT_SEL + (SCL_PIN as u64) * 4,
            SIG_I2CEXT0_SCL,
        )
        .unwrap();
        bus.write_u32(
            C3_GPIO_BASE + FUNC_IN_SEL + u64::from(SIG_I2CEXT0_SDA) * 4,
            MATRIX_INPUT_SELECT | u32::from(SDA_PIN),
        )
        .unwrap();
        bus.write_u32(
            C3_GPIO_BASE + FUNC_IN_SEL + u64::from(SIG_I2CEXT0_SCL) * 4,
            MATRIX_INPUT_SELECT | u32::from(SCL_PIN),
        )
        .unwrap();
        bus.write_u32(I2C_BASE, 199).unwrap(); // SCL_LOW_PERIOD
        bus.write_u32(I2C_BASE + 0x38, 180 | (19 << 9)).unwrap(); // SCL_HIGH_PERIOD
        bus.write_u32(I2C_BASE + 0x30, 29).unwrap(); // SDA_HOLD
        bus.write_u32(I2C_BASE + 0x40, 199).unwrap(); // SCL_START_HOLD
        bus.write_u32(I2C_BASE + 0x44, 199).unwrap(); // SCL_RSTART_SETUP
        bus.write_u32(I2C_BASE + 0x4C, 199).unwrap(); // SCL_STOP_SETUP
        bus.write_u32(I2C_BASE + 0x48, 199).unwrap(); // SCL_STOP_HOLD
        machine
    }

    fn run_c3_i2c(tick_interval: u32, force_poll: bool) -> Vec<LogicEdge> {
        let mut machine = c3_i2c_machine(tick_interval);
        machine.logic_force_poll_capture(force_poll);
        let gpio_idx = machine.bus.find_peripheral_index_by_name("gpio").unwrap();
        let initial = machine.logic_watch(&[Some((gpio_idx, SDA_PIN)), Some((gpio_idx, SCL_PIN))]);
        assert_eq!(initial, vec![Some(true), Some(true)]);

        // Kick RSTART; WRITE 3 (addr, control, data); STOP.
        let bus = &mut machine.bus;
        let cmd = |opcode: u32, byte_num: u32| (opcode << 11) | byte_num;
        bus.write_u32(I2C_BASE + REG_CMD0, cmd(6, 0)).unwrap();
        bus.write_u32(I2C_BASE + REG_CMD0 + 4, cmd(1, 3)).unwrap();
        bus.write_u32(I2C_BASE + REG_CMD0 + 8, cmd(2, 0)).unwrap();
        for b in WIRE_BYTES {
            bus.write_u32(I2C_BASE + REG_DATA, b as u32).unwrap();
        }
        bus.write_u32(I2C_BASE + REG_CTR, 1 << 5).unwrap();

        // ~46k cycles of wire time at 100 kHz; run a fixed budget with
        // headroom so both modes execute the identical cycle count.
        for _ in 0..60 {
            machine.run(Some(1_000)).unwrap();
        }
        machine.logic_read_edges(0).edges
    }

    /// THE gate, I²C flavour: the bit-level engine's SDA/SCL waveform on
    /// matrix-routed pads, pushed from the `I2cLineLevels` cell, must be
    /// byte-identical to the per-cycle poll of `read_gpio_pad`.
    #[test]
    fn esp32c3_i2c_waveform_push_stream_is_byte_identical_to_poll() {
        for tick_interval in [1u32, 4] {
            let poll = run_c3_i2c(tick_interval, true);
            let push = run_c3_i2c(tick_interval, false);
            let scl_rises = poll.iter().filter(|e| e.ch == 1 && e.value).count();
            assert_eq!(
                scl_rises,
                WIRE_BYTES.len() * 9 + 1,
                "tick={tick_interval}: full transaction on the wire (3 bytes x 9 bits + STOP rise)"
            );
            assert_eq!(
                poll, push,
                "tick={tick_interval}: push I2C waveform must be byte-identical to poll"
            );
        }
    }

    // ── STM32 SPI waveform (bit-engine push path) ────────────────────────────

    const SPI1_BASE: u64 = 0x4001_3000;
    const SPIA_GPIO_BASE: u64 = 0x5000_0000;
    const SCK_PIN: u8 = 5;
    const MOSI_PIN: u8 = 7;
    const SPI_WIRE_BYTES: [u8; 3] = [0x21, 0x0C, 0xA5];

    /// The STM32 SPI1/PA5/PA7 waveform fixture from
    /// `tests/stm32_spi_waveform.rs`, driven through `Machine::run` batches.
    /// BR=4 (half-period 16 cycles) keeps every wire transition on a
    /// tick-interval multiple for all tested intervals.
    fn stm32_spi_machine(tick_interval: u32) -> Machine<CortexM> {
        use crate::peripherals::spi::{Spi, SpiRegisterLayout};
        let mut bus = crate::bus::SystemBus::new();
        let (cpu, _nvic) = crate::system::cortex_m::configure_cortex_m(&mut bus);
        // A V2 GPIOA at a dedicated base (the default test-bus "gpioa" is F1).
        let gpioa_idx = bus.find_peripheral_index_by_name("gpioa").unwrap();
        bus.peripherals[gpioa_idx].dev =
            Box::new(GpioPort::new_with_layout(GpioRegisterLayout::Stm32V2));
        bus.peripherals[gpioa_idx].base = SPIA_GPIO_BASE;
        bus.rebuild_peripheral_ranges();
        bus.add_peripheral(
            "spi1",
            SPI1_BASE,
            0x400,
            None,
            Box::new(Spi::new_with_layout(SpiRegisterLayout::Stm32Fifo)),
        );
        bus.wire_stm32_spi_pads();

        let mut machine = Machine::new(cpu, bus);
        machine.config.peripheral_tick_interval = tick_interval;
        machine.bus.config.peripheral_tick_interval = tick_interval;
        for i in 0..1022u64 {
            let byte = if i % 2 == 0 { 0x00 } else { 0x20 };
            machine.bus.write_u8(RAM_BASE + i, byte).unwrap();
        }
        machine.bus.write_u8(RAM_BASE + 1022, 0xFF).unwrap();
        machine.bus.write_u8(RAM_BASE + 1023, 0xE5).unwrap();
        machine.cpu.pc = RAM_BASE as u32;

        let bus = &mut machine.bus;
        bus.write_u32(
            SPIA_GPIO_BASE + MODER,
            (0b10 << (SCK_PIN * 2)) | (0b10 << (MOSI_PIN * 2)),
        )
        .unwrap();
        bus.write_u32(
            SPIA_GPIO_BASE + 0x20, // AFRL: AF5 on both pads
            (5 << (SCK_PIN * 4)) | (5 << (MOSI_PIN * 4)),
        )
        .unwrap();
        // CR1 = MSTR|SSM|SSI|BR=4 (/32)|SPE — mode 0, MSB first, 8-bit.
        bus.write_u16(
            SPI1_BASE,
            (1 << 2) | (1 << 9) | (1 << 8) | (0x4 << 3) | (1 << 6),
        )
        .unwrap();
        machine
    }

    fn run_stm32_spi(tick_interval: u32, force_poll: bool) -> Vec<LogicEdge> {
        let mut machine = stm32_spi_machine(tick_interval);
        machine.logic_force_poll_capture(force_poll);
        let gpioa_idx = machine.bus.find_peripheral_index_by_name("gpioa").unwrap();
        let initial =
            machine.logic_watch(&[Some((gpioa_idx, MOSI_PIN)), Some((gpioa_idx, SCK_PIN))]);
        assert_eq!(initial, vec![Some(false), Some(false)]);

        // Three bytes back-to-back through the TX FIFO (3 × 8-bit fits the
        // 4-frame FIFO), then a fixed cycle budget so both modes execute the
        // identical cycle count. 3 frames × 8 bits × 32 cycles ≈ 768 cycles.
        for b in SPI_WIRE_BYTES {
            machine.bus.write_u8(SPI1_BASE + 0x0C, b).unwrap();
        }
        for _ in 0..40 {
            machine.run(Some(100)).unwrap();
        }
        machine.logic_read_edges(0).edges
    }

    /// THE gate, SPI flavour: the bit-level engine's SCK/MOSI waveform on
    /// AFR-routed pads, pushed from the `SpiLineLevels` cell, must be
    /// byte-identical to the per-cycle poll of `read_gpio_pad`.
    #[test]
    fn stm32_spi_waveform_push_stream_is_byte_identical_to_poll() {
        for tick_interval in [1u32, 4] {
            let poll = run_stm32_spi(tick_interval, true);
            let push = run_stm32_spi(tick_interval, false);
            let sck_rises = poll.iter().filter(|e| e.ch == 1 && e.value).count();
            assert_eq!(
                sck_rises,
                SPI_WIRE_BYTES.len() * 8,
                "tick={tick_interval}: full byte stream on the wire (3 bytes x 8 SCK rises)"
            );
            assert_eq!(
                poll, push,
                "tick={tick_interval}: push SPI waveform must be byte-identical to poll"
            );
        }
    }

    // ── Honest fallback: non-instrumented peripherals stay on poll ──────────

    /// A minimal GPIO-ish peripheral that deliberately does NOT accept a
    /// logic tap: one output latch word, every pin an output.
    #[derive(Debug, Default)]
    struct PollOnlyGpio {
        latch: u32,
    }

    impl Peripheral for PollOnlyGpio {
        fn read(&self, offset: u64) -> SimResult<u8> {
            Ok(((self.latch >> ((offset & 3) * 8)) & 0xFF) as u8)
        }
        fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
            let shift = (offset & 3) * 8;
            self.latch = (self.latch & !(0xFFu32 << shift)) | ((value as u32) << shift);
            Ok(())
        }
        fn read_gpio_output(&self, pin: u8) -> Option<bool> {
            (pin < 32).then(|| (self.latch >> pin) & 1 != 0)
        }
    }

    /// Mixed watch set: one channel on a push-instrumented `GpioPort`, one on
    /// a non-instrumented peripheral. The fallback channel must still capture
    /// (per-cycle poll), the batch clamp must be back in force, and the whole
    /// stream must equal the fully-forced-poll run.
    #[test]
    fn non_instrumented_peripheral_falls_back_to_poll() {
        const POLL_GPIO_BASE: u64 = 0x5100_0000;
        let build = || {
            let mut machine = bitbang_machine(8);
            machine.bus.add_peripheral(
                "gpio_poll_only",
                POLL_GPIO_BASE,
                0x100,
                None,
                Box::<PollOnlyGpio>::default(),
            );
            // loop: str r1,[r0]; str r1,[r3]; str r2,[r0]; str r2,[r3]; b loop
            machine.cpu.r3 = POLL_GPIO_BASE as u32;
            machine.bus.write_u16(RAM_BASE, 0x6001).unwrap(); // str r1, [r0]
            machine.bus.write_u16(RAM_BASE + 2, 0x6019).unwrap(); // str r1, [r3]
            machine.bus.write_u16(RAM_BASE + 4, 0x6002).unwrap(); // str r2, [r0]
            machine.bus.write_u16(RAM_BASE + 6, 0x601A).unwrap(); // str r2, [r3]
            machine.bus.write_u16(RAM_BASE + 8, 0xE7FA).unwrap(); // b .-12
            machine
        };
        let run = |force_poll: bool| {
            let mut machine = build();
            machine.logic_force_poll_capture(force_poll);
            let push_idx = machine
                .bus
                .find_peripheral_index_by_name("gpio_test")
                .unwrap();
            let poll_idx = machine
                .bus
                .find_peripheral_index_by_name("gpio_poll_only")
                .unwrap();
            machine.logic_watch(&[Some((push_idx, 0)), Some((poll_idx, 0))]);
            machine.reset_step_profile();
            machine.run(Some(400)).unwrap();
            let profile = machine.step_profile();
            (
                machine.logic_read_edges(0).edges,
                profile.cpu_batches,
                profile.cpu_instructions,
            )
        };
        let (poll_edges, poll_batches, poll_instr) = run(true);
        let (push_edges, push_batches, push_instr) = run(false);

        assert!(
            poll_edges.iter().filter(|e| e.ch == 1).count() >= 100,
            "the non-instrumented channel must still record its toggles"
        );
        assert_eq!(poll_edges, push_edges, "mixed-mode stream must match poll");
        // A polled channel is armed in BOTH runs, so both keep the clamp.
        assert_eq!(poll_batches, poll_instr);
        assert_eq!(push_batches, push_instr);
    }

    // ── ingest semantics units ───────────────────────────────────────────────

    /// A pad that toggles and returns within one cycle records nothing (the
    /// last written level wins) — matching what a per-cycle boundary sampler
    /// can observe.
    #[test]
    fn intra_cycle_toggle_records_net_transition_only() {
        use crate::logic_capture::{LogicCapture, PadEvent};
        let mut cap = LogicCapture::new();
        cap.install(&[Some((0, 0))], &[Some(false)], &[true]);
        cap.ingest_push(
            &[
                PadEvent {
                    ch: 0,
                    value: true,
                    cycle: 10,
                },
                PadEvent {
                    ch: 0,
                    value: false,
                    cycle: 10,
                },
            ],
            999,
            999,
        );
        assert!(
            cap.read_edges(0).edges.is_empty(),
            "A->B->A inside one cycle is invisible to a boundary observer"
        );
    }

    /// Same-cycle transitions on several channels are emitted in ascending
    /// channel order regardless of write order — the documented deterministic
    /// rule (and exactly what the poll sampler produces).
    #[test]
    fn same_cycle_edges_are_emitted_in_ascending_channel_order() {
        use crate::logic_capture::{LogicCapture, PadEvent};
        let mut cap = LogicCapture::new();
        cap.install(
            &[Some((0, 0)), Some((0, 1))],
            &[Some(false), Some(false)],
            &[true, true],
        );
        cap.ingest_push(
            &[
                PadEvent {
                    ch: 1,
                    value: true,
                    cycle: 7,
                },
                PadEvent {
                    ch: 0,
                    value: true,
                    cycle: 7,
                },
            ],
            999,
            999,
        );
        let edges = cap.read_edges(0).edges;
        assert_eq!(edges.len(), 2);
        assert_eq!((edges[0].ch, edges[0].cycle), (0, 7));
        assert_eq!((edges[1].ch, edges[1].cycle), (1, 7));
    }

    // ── Idle fast-forward while probing (push-only watch sets) ──────────────

    /// The product win: a WFI-idling RISC-V firmware keeps its idle skip while
    /// its pads are probed through push capture, and the edge stream is still
    /// byte-identical to the (clamped, non-skipping) poll reference. Gated on
    /// `event-scheduler` because `try_idle_fast_forward` compiles to a no-op
    /// without it.
    #[cfg(feature = "event-scheduler")]
    #[test]
    fn idle_fast_forward_engages_while_probing_push_channels() {
        use crate::cpu::RiscV;
        let build = || {
            let mut bus = crate::bus::SystemBus::new();
            bus.flash.data = vec![0; 0x100].into();
            bus.add_peripheral(
                "gpio_test",
                GPIO_BASE,
                0x400,
                None,
                Box::new(GpioPort::new_with_layout(GpioRegisterLayout::Stm32V2)),
            );
            // MODER pin0 = output.
            bus.write_u32(GPIO_BASE + MODER, 0b01).unwrap();
            // lui x1, 0x50000; addi x2, x0, 1; sw x2, 0x14(x1); wfi; jal .-4
            bus.write_u32(0x0, 0x5000_00B7).unwrap();
            bus.write_u32(0x4, 0x0010_0113).unwrap();
            bus.write_u32(0x8, 0x0020_AA23).unwrap();
            bus.write_u32(0xC, 0x1050_0073).unwrap();
            bus.write_u32(0x10, 0xFFDF_F06F).unwrap();
            let mut cpu = RiscV::new();
            cpu.pc = 0x0;
            cpu.mtimecmp = u64::MAX;
            let mut machine = Machine::new(cpu, bus);
            machine.config.idle_fast_forward_enabled = true;
            machine.bus.legacy_walk_disabled = true;
            machine
        };
        let run = |force_poll: bool| {
            let mut machine = build();
            machine.logic_force_poll_capture(force_poll);
            let idx = machine
                .bus
                .find_peripheral_index_by_name("gpio_test")
                .unwrap();
            machine.logic_watch(&[Some((idx, 0))]);
            machine.reset_step_profile();
            machine.run(Some(10_000)).unwrap();
            (
                machine.logic_read_edges(0).edges,
                machine.step_profile().cpu_instructions,
                machine.total_cycles,
            )
        };
        let (poll_edges, poll_instr, poll_cycles) = run(true);
        let (push_edges, push_instr, push_cycles) = run(false);

        assert_eq!(poll_cycles, push_cycles, "identical simulated time");
        assert_eq!(
            poll_edges.len(),
            1,
            "one pad write before the firmware parks in WFI"
        );
        assert_eq!(
            poll_edges, push_edges,
            "edge stream must be exact under fast-forward"
        );
        assert_eq!(
            poll_instr, poll_cycles,
            "poll fallback chews every idle cycle (fast-forward disabled)"
        );
        assert!(
            push_instr < push_cycles / 100,
            "push capture must keep the idle skip: retired {push_instr} \
             instructions over {push_cycles} cycles"
        );
    }
}
