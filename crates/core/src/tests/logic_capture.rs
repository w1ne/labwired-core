// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Tests for in-engine logic-analyzer edge capture (`crate::logic_capture`).

#[cfg(test)]
mod logic_capture_tests {
    use crate::cpu::CortexM;
    use crate::logic_capture::{LogicCapture, LogicEdge, LOGIC_RING_CAPACITY};
    use crate::peripherals::gpio::{GpioPort, GpioRegisterLayout};
    use crate::{Bus, Machine};

    const GPIO_BASE: u64 = 0x5000_0000;
    /// V2-layout register offsets used below.
    const MODER: u64 = 0x00;
    const ODR: u64 = 0x14;
    /// A NOP-equivalent Thumb instruction (`movs r0, #0`, LE bytes 00 20) the
    /// CPU chews through so `step()` advances cycles without side effects.
    const RAM_BASE: u64 = 0x2000_0000;

    /// Build a Cortex-M machine with a spare V2 GPIO wired at `GPIO_BASE`, its
    /// pin 0 configured as a push-pull output, and the CPU pointed at a slab of
    /// harmless instructions in RAM so `step()` makes deterministic progress.
    fn machine_with_gpio() -> Machine<CortexM> {
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

        // MODER pin0 = 0b01 (output) so read_gpio_pad(0) reflects ODR bit 0.
        machine
            .bus
            .write_u32(GPIO_BASE + MODER, 0x0000_0001)
            .unwrap();

        // Fill a RAM window with `movs r0, #0` and run from it.
        for i in 0..256u64 {
            let byte = if i % 2 == 0 { 0x00 } else { 0x20 };
            machine.bus.write_u8(RAM_BASE + i, byte).unwrap();
        }
        machine.cpu.pc = RAM_BASE as u32;
        machine
    }

    fn set_pin0(machine: &mut Machine<CortexM>, level: bool) {
        machine
            .bus
            .write_u32(GPIO_BASE + ODR, if level { 1 } else { 0 })
            .unwrap();
    }

    fn step_n(machine: &mut Machine<CortexM>, n: usize) {
        for _ in 0..n {
            machine.step().unwrap();
        }
    }

    /// Drive a toggle sequence through the step loop and collect the edges.
    /// Returns `(edges, dropped, now_cycle)`.
    fn run_scenario(machine: &mut Machine<CortexM>) -> (Vec<LogicEdge>, u64, u64) {
        let idx = machine
            .bus
            .find_peripheral_index_by_name("gpio_test")
            .unwrap();
        let initial = machine.logic_watch(&[Some((idx, 0))]);
        assert_eq!(initial, vec![Some(false)], "pin starts low");

        // Sampling is per-cycle, so any spacing captures each edge exactly
        // once; a few steps of gap keeps the cycle stamps visibly distinct.
        let gap = 48;
        step_n(machine, gap); // low (== initial): no edge
        set_pin0(machine, true);
        step_n(machine, gap); // -> high
        set_pin0(machine, false);
        step_n(machine, gap); // -> low
        set_pin0(machine, true);
        step_n(machine, gap); // -> high

        let batch = machine.logic_read_edges(0);
        (batch.edges, batch.dropped, machine.logic_now_cycle())
    }

    #[test]
    fn captures_gpio_transitions_with_monotonic_cycles() {
        let mut machine = machine_with_gpio();
        let (edges, dropped, now) = run_scenario(&mut machine);

        assert_eq!(dropped, 0, "no overflow in this small scenario");
        let values: Vec<bool> = edges.iter().map(|e| e.value).collect();
        assert_eq!(values, vec![true, false, true], "high, low, high");

        // All on channel 0, cycles strictly increasing and non-decreasing vs now.
        for e in &edges {
            assert_eq!(e.ch, 0);
            assert!(e.cycle <= now);
        }
        for w in edges.windows(2) {
            assert!(w[1].cycle > w[0].cycle, "cycles strictly increase");
        }
    }

    #[test]
    fn identical_runs_produce_byte_identical_edge_streams() {
        let mut a = machine_with_gpio();
        let mut b = machine_with_gpio();
        let (edges_a, _, _) = run_scenario(&mut a);
        let (edges_b, _, _) = run_scenario(&mut b);
        assert_eq!(
            edges_a, edges_b,
            "determinism: same firmware + watch => same edges"
        );
    }

    #[test]
    fn unresolvable_refs_are_never_sampled() {
        let mut machine = machine_with_gpio();
        let idx = machine
            .bus
            .find_peripheral_index_by_name("gpio_test")
            .unwrap();
        // Channel 0 unresolvable (None), channel 1 a real pad.
        let initial = machine.logic_watch(&[None, Some((idx, 0))]);
        assert_eq!(initial, vec![None, Some(false)]);

        let gap = 48;
        step_n(&mut machine, gap);
        set_pin0(&mut machine, true);
        step_n(&mut machine, gap);

        let batch = machine.logic_read_edges(0);
        assert!(
            batch.edges.iter().all(|e| e.ch == 1),
            "only channel 1 emits"
        );
        assert_eq!(batch.edges.len(), 1);
    }

    #[test]
    fn cursor_returns_only_new_edges() {
        let mut machine = machine_with_gpio();
        let idx = machine
            .bus
            .find_peripheral_index_by_name("gpio_test")
            .unwrap();
        machine.logic_watch(&[Some((idx, 0))]);
        let gap = 48;

        set_pin0(&mut machine, true);
        step_n(&mut machine, gap);
        let first = machine.logic_read_edges(0);
        assert_eq!(first.edges.len(), 1);

        set_pin0(&mut machine, false);
        step_n(&mut machine, gap);
        let second = machine.logic_read_edges(first.cursor);
        assert_eq!(second.edges.len(), 1, "only the new edge since the cursor");
        assert!(!second.edges[0].value);

        // Re-reading with the same cursor yields nothing.
        let again = machine.logic_read_edges(second.cursor);
        assert!(again.edges.is_empty());
    }

    /// Native/wasm parity: the per-channel series the CLI serializes into
    /// `result.json` (via `build_logic_edges_result`) reproduces, channel by
    /// channel, the exact `{ch, cycle, value}` edge stream the wasm
    /// `read_logic_edges` accessor surfaces for the SAME run — both drain the
    /// identical `logic_read_edges`. The only encoding difference is 0/1 vs bool.
    #[test]
    fn cli_edge_series_matches_wasm_edge_stream() {
        use crate::logic_capture::{build_logic_edges_result, LogicChannelMeta};

        let mut machine = machine_with_gpio();
        let idx = machine
            .bus
            .find_peripheral_index_by_name("gpio_test")
            .unwrap();
        let initial = machine.logic_watch(&[Some((idx, 0))]);

        let gap = 48;
        step_n(&mut machine, gap);
        set_pin0(&mut machine, true);
        step_n(&mut machine, gap);
        set_pin0(&mut machine, false);
        step_n(&mut machine, gap);
        set_pin0(&mut machine, true);
        step_n(&mut machine, gap);

        let now = machine.logic_now_cycle();
        let batch = machine.logic_read_edges(0);
        // The exact stream wasm's `read_logic_edges` would emit for this run.
        let wasm_stream: Vec<(u32, u64, u8)> = batch
            .edges
            .iter()
            .map(|e| (e.ch, e.cycle, u8::from(e.value)))
            .collect();

        let meta = vec![LogicChannelMeta {
            ch: 0,
            peripheral: "gpio_test".to_string(),
            pin: 0,
            initial: initial[0],
        }];
        let result = build_logic_edges_result(&meta, &batch, now);

        assert_eq!(result.dropped, 0);
        assert_eq!(result.now_cycle, now);
        assert_eq!(result.channels.len(), 1);
        let ch0 = &result.channels[0];
        assert_eq!(ch0.channel, "CH0");
        assert_eq!(ch0.peripheral, "gpio_test");
        assert_eq!(ch0.pin, 0);
        assert_eq!(ch0.initial, Some(0), "pad starts low");

        // Reconstruct the CLI series back into a flat (ch, cycle, value) stream
        // and assert it equals the wasm stream edge-for-edge (parity).
        let cli_stream: Vec<(u32, u64, u8)> = result
            .channels
            .iter()
            .flat_map(|c| c.transitions.iter().map(move |t| (c.ch, t.cycle, t.value)))
            .collect();
        assert_eq!(cli_stream, wasm_stream, "CLI series == wasm edge stream");
        // Sanity: the toggle sequence was high, low, high.
        assert_eq!(
            ch0.transitions.iter().map(|t| t.value).collect::<Vec<_>>(),
            vec![1, 0, 1]
        );
    }

    /// `build_logic_edges_result` routes each edge to its channel by `ch`,
    /// preserves order, encodes value/initial as 0/1, and surfaces the run-level
    /// `dropped` overflow count (the oracle's fail-loud signal).
    #[test]
    fn build_logic_edges_result_routes_and_encodes() {
        use crate::logic_capture::{
            build_logic_edges_result, EdgeTransition, LogicChannelMeta, LogicEdge, LogicEdgeBatch,
        };

        let batch = LogicEdgeBatch {
            cursor: 5,
            dropped: 2,
            edges: vec![
                LogicEdge {
                    ch: 0,
                    cycle: 10,
                    value: true,
                },
                LogicEdge {
                    ch: 1,
                    cycle: 12,
                    value: true,
                },
                LogicEdge {
                    ch: 0,
                    cycle: 15,
                    value: false,
                },
            ],
        };
        let meta = vec![
            LogicChannelMeta {
                ch: 0,
                peripheral: "gpio8".into(),
                pin: 8,
                initial: Some(false),
            },
            LogicChannelMeta {
                ch: 1,
                peripheral: "gpio9".into(),
                pin: 9,
                initial: None,
            },
        ];
        let r = build_logic_edges_result(&meta, &batch, 20);

        assert_eq!(r.dropped, 2, "overflow count surfaced for fail-loud");
        assert_eq!(r.now_cycle, 20);
        assert_eq!(r.channels[0].initial, Some(0));
        assert_eq!(r.channels[1].initial, None);
        assert_eq!(
            r.channels[0].transitions,
            vec![
                EdgeTransition {
                    cycle: 10,
                    value: 1
                },
                EdgeTransition {
                    cycle: 15,
                    value: 0
                },
            ]
        );
        assert_eq!(
            r.channels[1].transitions,
            vec![EdgeTransition {
                cycle: 12,
                value: 1
            }]
        );
        assert!(r.channels.iter().all(|c| c.gaps.is_empty()));
    }

    /// Ring-buffer overflow at the capture layer: push more edges than capacity,
    /// assert some were dropped and the newest are retained.
    #[test]
    fn ring_buffer_drops_oldest_on_overflow() {
        let mut cap = LogicCapture::new();
        cap.install(&[Some((0, 0))], &[Some(false)], &[false]);

        let overflow = 100usize;
        let total = LOGIC_RING_CAPACITY + overflow;
        // Toggle on every sample so each sample records exactly one edge.
        for i in 0..total {
            let cycle = i as u64 + 1;
            let level = i % 2 == 0;
            cap.sample(cycle, |_, _| Some(level));
        }

        let batch = cap.read_edges(0);
        assert_eq!(batch.dropped, overflow as u64, "oldest edges dropped");
        assert_eq!(
            batch.edges.len(),
            LOGIC_RING_CAPACITY,
            "ring is full, not larger"
        );
        // Newest edge is the last sample taken.
        let last = batch.edges.last().unwrap();
        assert_eq!(last.cycle, total as u64);
        assert_eq!(batch.cursor, total as u64);
    }

    #[test]
    fn acknowledged_reads_free_ring_capacity_for_continuous_capture() {
        let mut cap = LogicCapture::new();
        cap.install(&[Some((0, 0))], &[Some(false)], &[false]);

        for i in 0..LOGIC_RING_CAPACITY {
            cap.sample(i as u64 + 1, |_, _| Some(i % 2 == 0));
        }

        let first = cap.read_edges(0);
        assert_eq!(first.edges.len(), LOGIC_RING_CAPACITY);
        assert_eq!(first.dropped, 0);

        let acknowledged = cap.read_edges(first.cursor);
        assert!(acknowledged.edges.is_empty());
        assert_eq!(acknowledged.dropped, 0);

        for i in 0..LOGIC_RING_CAPACITY {
            cap.sample(LOGIC_RING_CAPACITY as u64 + i as u64 + 1, |_, _| {
                Some(i % 2 == 0)
            });
        }

        let second = cap.read_edges(first.cursor);
        assert_eq!(second.edges.len(), LOGIC_RING_CAPACITY);
        assert_eq!(second.dropped, 0, "acknowledged edges must not overflow");
    }

    /// A pad that reads back as unknown (`None`) records no edges for that
    /// channel, even across many samples.
    #[test]
    fn unknown_pad_records_no_edges() {
        let mut cap = LogicCapture::new();
        cap.install(&[Some((0, 0))], &[None], &[false]);
        for i in 0..10 {
            cap.sample(i + 1, |_, _| None);
        }
        let batch = cap.read_edges(0);
        assert!(batch.edges.is_empty());
        assert_eq!(batch.dropped, 0);
    }

    /// The no-aliasing guarantee on the `step()` path: a pad toggled on EVERY
    /// cycle produces an edge on EVERY cycle — nothing is sampled away.
    #[test]
    fn pad_toggling_every_cycle_produces_every_edge() {
        let mut machine = machine_with_gpio();
        let idx = machine
            .bus
            .find_peripheral_index_by_name("gpio_test")
            .unwrap();
        let initial = machine.logic_watch(&[Some((idx, 0))]);
        assert_eq!(initial, vec![Some(false)]);

        let toggles = 100usize;
        for i in 0..toggles {
            set_pin0(&mut machine, i % 2 == 0); // true, false, true, ...
            machine.step().unwrap();
        }

        let batch = machine.logic_read_edges(0);
        assert_eq!(
            batch.edges.len(),
            toggles,
            "every per-cycle toggle is captured — no aliasing"
        );
        for (i, e) in batch.edges.iter().enumerate() {
            assert_eq!(e.value, i % 2 == 0, "values strictly alternate");
        }
        for w in batch.edges.windows(2) {
            assert_eq!(w[1].cycle - w[0].cycle, 1, "one edge per cycle");
        }
        assert_eq!(batch.dropped, 0);
    }

    /// The no-aliasing guarantee on the batched `Machine::run` path: firmware
    /// bit-banging a pad on consecutive instructions (str/str/branch loop) has
    /// every transition captured even with a wide peripheral tick interval —
    /// push-instrumented pads report every write-site transition; polled pads
    /// get the same guarantee from the armed one-instruction batch clamp (see
    /// `tests/logic_capture_differential.rs` for the byte-equality oracle).
    #[test]
    fn run_path_captures_bitbang_toggles_without_aliasing() {
        use crate::DebugControl;

        let mut machine = machine_with_gpio();
        // Wide tick interval: without the armed batch clamp the run loop would
        // stride past intermediate toggles and alias them away.
        machine.config.peripheral_tick_interval = 8;

        // r0 = &ODR, r1 = 1, r2 = 0;  loop: str r1,[r0]; str r2,[r0]; b loop
        machine.cpu.r0 = (GPIO_BASE + ODR) as u32;
        machine.cpu.r1 = 1;
        machine.cpu.r2 = 0;
        machine.bus.write_u16(RAM_BASE, 0x6001).unwrap(); // str r1, [r0]
        machine.bus.write_u16(RAM_BASE + 2, 0x6002).unwrap(); // str r2, [r0]
        machine.bus.write_u16(RAM_BASE + 4, 0xE7FC).unwrap(); // b .-8
        machine.cpu.pc = RAM_BASE as u32;

        let idx = machine
            .bus
            .find_peripheral_index_by_name("gpio_test")
            .unwrap();
        machine.logic_watch(&[Some((idx, 0))]);

        let steps = 300u32;
        machine.run(Some(steps)).unwrap();

        let batch = machine.logic_read_edges(0);
        // 2 toggles per 3-instruction loop iteration; anything close to that is
        // alias-free (the old 16-cycle grid captured well under a tenth of it).
        assert!(
            batch.edges.len() >= 190,
            "expected ~200 edges over {steps} cycles, got {}",
            batch.edges.len()
        );
        for (i, e) in batch.edges.iter().enumerate() {
            assert_eq!(e.value, i % 2 == 0, "values strictly alternate");
        }
        for w in batch.edges.windows(2) {
            assert!(
                w[1].cycle - w[0].cycle <= 2,
                "consecutive toggles captured at consecutive boundaries"
            );
        }
    }

    /// Unarmed capture stays on the zero-overhead path: no watch installed
    /// means no samples, no edges and no ring growth, no matter how much the
    /// machine runs.
    #[test]
    fn unarmed_run_records_nothing() {
        let mut machine = machine_with_gpio();
        for i in 0..100 {
            set_pin0(&mut machine, i % 2 == 0);
            machine.step().unwrap();
        }
        let batch = machine.logic_read_edges(0);
        assert!(batch.edges.is_empty(), "no watch, no capture");
        assert_eq!(batch.cursor, 0);
        assert_eq!(batch.dropped, 0);
    }
}
