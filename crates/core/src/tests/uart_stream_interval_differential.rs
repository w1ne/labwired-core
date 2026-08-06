// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! An attached UART RX stream must not change what the guest sees when the
//! peripheral tick interval widens.
//!
//! The shared `Uart` used to hold `reschedule_delay: 1` for as long as any
//! stream was attached. That single line pinned `plan_cpu_window` to a
//! one-instruction quantum — through the next-event-deadline clamp — for every
//! lab carrying a GPS, an HC-05, a modem, an IO-Link peer or a cross-chip UART
//! link. It now wakes once per interval and replays the skipped
//! tick-equivalents (`Uart::advance_ticks`).
//!
//! The contract that buys: the byte SEQUENCE a stream pushes into RX per unit
//! of simulated time is interval-independent. Only the instant a byte becomes
//! visible is quantised, by at most one interval.

#[cfg(all(test, feature = "event-scheduler"))]
mod uart_stream_interval_tests {
    use crate::bus::SystemBus;
    use crate::peripherals::uart::{Uart, UartRegisterLayout, UartStreamDevice};
    use crate::{Bus, Peripheral};

    /// Emits an ascending byte on every poll — so the RX contents are a direct
    /// transcript of how many times the UART serviced its streams.
    #[derive(Default)]
    struct Counter(u8);

    impl UartStreamDevice for Counter {
        fn poll(&mut self, _elapsed_us: u32) -> Option<u8> {
            let b = self.0;
            self.0 = self.0.wrapping_add(1);
            Some(b)
        }
    }

    /// Drive a UART's scheduler path for `cycles` simulated cycles at the given
    /// tick interval and return everything the stream pushed into RX.
    fn rx_transcript(interval: u32, cycles: u64) -> Vec<u8> {
        let mut bus = SystemBus::new();
        bus.config.peripheral_tick_interval = interval;

        let mut uart = Uart::new_with_layout(UartRegisterLayout::Stm32F1);
        // Mirror the real registration choke: the bus hands every peripheral
        // its cycle clock, which is what opts the UART into interval pacing.
        Peripheral::attach_cycle_clock(&mut uart, bus.cycle_clock.clone());
        uart.attach_stream(Box::new(Counter::default()));

        let rx = uart.rx_buffer();
        let mut sched = crate::sched::EventScheduler::new();

        // Service the UART on its own cadence across the window, exactly as the
        // machine's scheduler would: advance the published cycle to the wake-up
        // instant, then deliver the event.
        let step = u64::from(interval.max(1));
        let mut now = 0u64;
        while now < cycles {
            now += step;
            bus.set_current_cycle(now);
            uart.on_event(0, &mut sched, &mut bus);
        }

        let guard = rx.lock().unwrap();
        guard.iter().copied().collect()
    }

    /// The whole point: same bytes, same order, whatever the interval.
    #[test]
    fn rx_byte_sequence_is_identical_at_interval_1_and_512() {
        const CYCLES: u64 = 4096;
        let exact = rx_transcript(1, CYCLES);
        let batched = rx_transcript(512, CYCLES);

        assert_eq!(
            exact.len(),
            CYCLES as usize,
            "interval 1 must poll the stream once per cycle"
        );
        assert_eq!(
            batched, exact,
            "a widened tick interval must not change the RX byte sequence — the \
             UART replays the tick-equivalents it slept through"
        );
    }

    /// Interval 128 is not special; the invariant is that poll COUNT tracks
    /// simulated cycles, not wake-ups.
    #[test]
    fn poll_count_tracks_simulated_cycles_not_wakeups() {
        const CYCLES: u64 = 2048;
        for interval in [1u32, 8, 64, 128, 512] {
            assert_eq!(
                rx_transcript(interval, CYCLES).len(),
                CYCLES as usize,
                "interval {interval} must still deliver one poll per simulated cycle"
            );
        }
    }

    /// A UART that never received a cycle clock (hand-built bus, or a host that
    /// bypasses the attach choke) must keep the exact legacy per-cycle cadence
    /// — the conservative half of the `attach_cycle_clock` contract.
    #[test]
    fn without_a_cycle_clock_the_legacy_cadence_is_preserved() {
        let mut bus = SystemBus::new();
        bus.config.peripheral_tick_interval = 512;

        let mut uart = Uart::new_with_layout(UartRegisterLayout::Stm32F1);
        // Deliberately no attach_cycle_clock.
        uart.attach_stream(Box::new(Counter::default()));
        let rx = uart.rx_buffer();
        let mut sched = crate::sched::EventScheduler::new();

        bus.set_current_cycle(512);
        let result = uart.on_event(0, &mut sched, &mut bus);

        assert_eq!(
            rx.lock().unwrap().len(),
            1,
            "no clock ⇒ one tick-equivalent per wake-up, as before"
        );
        assert_eq!(
            result.reschedule_delay,
            Some(1),
            "no clock ⇒ the UART keeps re-arming every cycle"
        );
    }

    /// And with a clock, the UART must actually stop asking to be woken every
    /// cycle — that re-arm is what the CPU plan clamps against.
    #[test]
    fn a_stream_no_longer_demands_a_wakeup_every_cycle() {
        let mut bus = SystemBus::new();
        bus.config.peripheral_tick_interval = 512;

        let mut uart = Uart::new_with_layout(UartRegisterLayout::Stm32F1);
        Peripheral::attach_cycle_clock(&mut uart, bus.cycle_clock.clone());
        uart.attach_stream(Box::new(Counter::default()));
        let mut sched = crate::sched::EventScheduler::new();

        bus.set_current_cycle(512);
        let result = uart.on_event(0, &mut sched, &mut bus);

        assert_eq!(
            result.reschedule_delay,
            Some(512),
            "a stream-only UART must re-arm at the bus tick interval, not at 1"
        );
    }
}
