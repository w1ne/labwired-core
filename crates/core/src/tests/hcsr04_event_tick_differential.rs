// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Differential oracle for the HC-SR04 echo-waveform migration
//! (`perf/hcsr04-event-tick`): run the SAME busy-poll firmware twice — once on
//! the legacy per-cycle `service_hcsr04` path (`hcsr04_scheduling_disabled =
//! true`, which keeps `requires_cycle_accurate()` true and pins the batch to
//! one instruction), once on the event-scheduled ECHO-edge path — and assert
//! the two runs are BYTE-IDENTICAL: the ECHO input pad transitions at exactly
//! the same cycles, and the full machine snapshot + cycle count agree.
//!
//! Per-tick is the reference semantics; if the scheduled path disagrees, the
//! scheduled path is wrong. The firmware pulses TRIG, then busy-polls ECHO in
//! a tight `ldr/tst/b` loop — precisely the "an edge landing mid-batch must not
//! be observed late" case the migration has to preserve.
//!
//! The idle fast-forward is DISABLED here so the two paths execute the same
//! instruction stream one cycle at a time and can be compared step-for-step —
//! lifting the HC-SR04 cycle-accurate pin also re-enables idle fast-forward,
//! which would otherwise let the scheduled path skip the busy-poll and diverge
//! in instruction count (that optimisation is exercised by other tests).

#![cfg(feature = "event-scheduler")]

#[cfg(test)]
mod hcsr04_event_tick_differential_tests {
    use crate::cpu::CortexM;
    use crate::peripherals::gpio::{GpioPort, GpioRegisterLayout};
    use crate::peripherals::hc_sr04::HcSr04;
    use crate::{Bus, DebugControl, Machine};

    const GPIO_BASE: u64 = 0x4800_0000; // stm32v2: IDR @0x10, ODR @0x14
    const IDR: u64 = 0x10;
    const ODR: u64 = 0x14;
    const RAM_BASE: u64 = 0x2000_0000;
    const TRIG_BIT: u8 = 8; // PA8 (TRIG output)
    const ECHO_BIT: u8 = 9; // PA9 (ECHO input)
    const CPU_HZ: u64 = 1_000_000; // 1 cycle per µs → short, fast window

    /// Build a Cortex-M machine with a V2 GPIO, one HC-SR04 wired TRIG=PA8 /
    /// ECHO=PA9, on a walk-deleted bus (the config class the scheduled path
    /// serves). `scheduling_disabled` selects the legacy per-tick path.
    ///
    /// Firmware (Thumb) at RAM_BASE, r0=&ODR r1=1<<TRIG r2=&IDR r3=1<<ECHO:
    ///   str  r1,[r0]        ; TRIG high → arm the echo window
    ///   poll_high: ldr r4,[r2]; tst r4,r3; beq poll_high   ; wait ECHO high
    ///   poll_low:  ldr r4,[r2]; tst r4,r3; bne poll_low     ; wait ECHO low
    ///   end: b end
    fn build_machine(
        tick_interval: u32,
        scheduling_disabled: bool,
        distance_cm: f32,
    ) -> Machine<CortexM> {
        let mut bus = crate::bus::SystemBus::new();
        let (cpu, _nvic) = crate::system::cortex_m::configure_cortex_m(&mut bus);
        bus.add_peripheral(
            "gpioa",
            GPIO_BASE,
            0x400,
            None,
            Box::new(GpioPort::new_with_layout(GpioRegisterLayout::Stm32V2)),
        );
        bus.hcsr04.push(HcSr04::new(
            "dist".into(),
            GPIO_BASE + ODR,
            TRIG_BIT,
            GPIO_BASE + IDR,
            ECHO_BIT,
            CPU_HZ,
            distance_cm,
        ));
        // The scheduled ECHO-edge path is gated on a walk-deleted bus.
        bus.legacy_walk_disabled = true;
        bus.hcsr04_scheduling_disabled = scheduling_disabled;

        let mut machine = Machine::new(cpu, bus);
        machine.config.peripheral_tick_interval = tick_interval;
        // Fair A/B: keep both paths executing the same instruction stream (see
        // module docs). Idle fast-forward is validated separately.
        machine.config.idle_fast_forward_enabled = false;

        machine.cpu.r0 = (GPIO_BASE + ODR) as u32;
        machine.cpu.r1 = 1 << TRIG_BIT;
        machine.cpu.r2 = (GPIO_BASE + IDR) as u32;
        machine.cpu.r3 = 1 << ECHO_BIT;
        // 0x00 str r1,[r0]
        machine.bus.write_u16(RAM_BASE, 0x6001).unwrap();
        // 0x02 ldr r4,[r2]  (poll_high)
        machine.bus.write_u16(RAM_BASE + 2, 0x6814).unwrap();
        // 0x04 tst r4,r3
        machine.bus.write_u16(RAM_BASE + 4, 0x421C).unwrap();
        // 0x06 beq poll_high  (target 0x02)
        machine.bus.write_u16(RAM_BASE + 6, 0xD0FC).unwrap();
        // 0x08 ldr r4,[r2]  (poll_low)
        machine.bus.write_u16(RAM_BASE + 8, 0x6814).unwrap();
        // 0x0A tst r4,r3
        machine.bus.write_u16(RAM_BASE + 10, 0x421C).unwrap();
        // 0x0C bne poll_low  (target 0x08)
        machine.bus.write_u16(RAM_BASE + 12, 0xD1FC).unwrap();
        // 0x0E b end  (self)
        machine.bus.write_u16(RAM_BASE + 14, 0xE7FE).unwrap();
        machine.cpu.pc = RAM_BASE as u32;
        machine
    }

    fn echo_high(machine: &Machine<CortexM>) -> bool {
        (machine.bus.read_u32(GPIO_BASE + IDR).unwrap() >> ECHO_BIT) & 1 != 0
    }

    /// Single-step the machine cycle-by-cycle, recording every `(cycle, level)`
    /// transition of the ECHO input pad. The exact cycle each rise/fall is
    /// driven — through the shared choke point on either path — is what this
    /// pins. `step()` ticks/drains once per cycle, so both the per-tick service
    /// and the event drain are exercised at their real cycle.
    fn echo_edges(
        mut machine: Machine<CortexM>,
        cycles: u64,
    ) -> (Vec<(u64, bool)>, serde_json::Value) {
        let mut edges = Vec::new();
        let mut prev = echo_high(&machine);
        for _ in 0..cycles {
            machine.step().unwrap();
            let now = echo_high(&machine);
            if now != prev {
                edges.push((machine.total_cycles, now));
                prev = now;
            }
        }
        (edges, serde_json::to_value(machine.snapshot()).unwrap())
    }

    /// THE gate: legacy per-tick ECHO service vs event-scheduled ECHO edges must
    /// be byte-identical — the ECHO pad transitions at exactly the same cycles
    /// and the machine ends in an identical state. Runs at tick interval 2 (the
    /// scheduled path activates only for interval > 1), cycle-stepped so the
    /// edge cycle of every rise/fall is captured on both paths.
    #[test]
    fn scheduled_echo_edge_cycles_match_per_tick() {
        for distance_cm in [2.0f32, 30.0, 100.0, 300.0] {
            let cycles = 30_000;
            let (per_tick, snap_a) = echo_edges(build_machine(2, true, distance_cm), cycles);
            let (scheduled, snap_b) = echo_edges(build_machine(2, false, distance_cm), cycles);

            assert_eq!(
                per_tick.len(),
                2,
                "distance={distance_cm}: expected exactly one ECHO rise + one fall, got {per_tick:?}"
            );
            assert_eq!(
                per_tick, scheduled,
                "distance={distance_cm}: ECHO edge cycles must be byte-identical \
                 (per-tick {per_tick:?} vs scheduled {scheduled:?})"
            );
            assert_eq!(
                snap_a, snap_b,
                "distance={distance_cm}: full machine snapshot must be byte-identical"
            );
        }
    }

    /// The batch-clamp path (tick interval > 1, where the scheduled path runs
    /// multi-instruction batches ended exactly at the next ECHO edge) must stay
    /// byte-identical to the per-tick reference, which quantises ECHO to the
    /// same tick boundaries via its one-instruction batches.
    #[test]
    fn scheduled_echo_matches_per_tick_across_tick_intervals() {
        for tick_interval in [2u32, 8, 64] {
            let cycles = 30_000;
            let (per_tick, snap_a) = run_state(build_machine(tick_interval, true, 30.0), cycles);
            let (scheduled, snap_b) = run_state(build_machine(tick_interval, false, 30.0), cycles);
            assert_eq!(
                per_tick, scheduled,
                "tick={tick_interval}: total_cycles must match after the ECHO window"
            );
            assert_eq!(
                snap_a, snap_b,
                "tick={tick_interval}: full machine snapshot must be byte-identical"
            );
        }
    }

    /// Run via the batched `Machine::run` to a cycle budget and return
    /// `(total_cycles, snapshot)`. Exercises the batch clamp on the scheduled
    /// path (wide batches ended at the ECHO edge) and the one-instruction clamp
    /// on the per-tick path.
    fn run_state(mut machine: Machine<CortexM>, cycles: u64) -> (u64, serde_json::Value) {
        while machine.total_cycles < cycles {
            let remaining = (cycles - machine.total_cycles).min(u32::MAX as u64) as u32;
            machine.run(Some(remaining)).unwrap();
        }
        (
            machine.total_cycles,
            serde_json::to_value(machine.snapshot()).unwrap(),
        )
    }
}
