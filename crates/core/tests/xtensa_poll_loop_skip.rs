// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! The CCOUNT poll-loop skip (`cpu/xtensa_lx7/poll_loop.rs`) must fire, and it
//! must not change what the firmware does.
//!
//! The program is the shape of the four-servo trajectory sketch (share
//! d26Wk2eXNiFS): a pulse loop that drops four outputs at four CCOUNT
//! deadlines (it records `dt` where a real sketch writes `GPIO_OUT_W1TC`),
//! then a frame-tail busy-wait, then two loops the skip must REFUSE: one that
//! carries a CCOUNT sum from iteration to iteration, and one that tests a bit
//! of CCOUNT (not an ordered compare). Each test runs the program twice — idle
//! fast-forward on and off — and requires identical records, CCOUNT and
//! machine cycles, and a skip that covered most of the busy-waiting.
//!
//! Single core: the core runs the program on the primary. Dual core: the
//! secondary (an ESP32 APP_CPU running Arduino `loopTask`) runs it while the
//! primary sits in WAITI. Before the skip existed, the dual-core machine
//! refused any skip while the secondary was active and the single-core one
//! had no WAITI to skip, so both `skipped` assertions fail without it.
//!
//! Busy primary: the secondary runs the program while the primary never
//! idles. It counts in a store loop, then rewrites output 1's deadline (a
//! word the secondary's pulse loop reads) at a cycle inside the pulse wait,
//! then counts on forever. The store lands at ~15 000, while the loop
//! waits for output 2 (18 000); the new deadline (15 104) is ~100 cycles
//! later, so a deferral (or a primary batch) that missed the store would run
//! past it. No machine-wide skip is possible; the secondary's
//! loop is deferred instead (`Machine::secondary_defer`) and must notice the
//! store on the cycle it lands: output 1 drops at the NEW deadline in both
//! runs. Without the deferral `deferred` is 0; without its store guard the
//! deferred loop catches up on the old deadline and the records differ.

#![cfg(feature = "event-scheduler")]

mod tests {
    use labwired_core::bus::SystemBus;
    use labwired_core::cpu::xtensa_lx7::XtensaLx7;
    use labwired_core::cpu::xtensa_sr::CCOUNT;
    use labwired_core::machine::AdvanceRequest;
    use labwired_core::system::xtensa::configure_xtensa_esp32;
    use labwired_core::{Bus, Cpu, Machine};

    const CODE: u64 = 0x4008_1000;
    const IDLE: u64 = 0x4008_2000;
    const DATA: u32 = 0x3ffb_1000;

    /// `frame.S`, assembled with xtensa-esp32-elf-as (esp-2021r2):
    ///
    /// ```text
    ///  0: rsr.ccount a2            ; t0
    ///     movi.n  a10, 15          ; four outputs pending
    ///  5: rsr.ccount a13           ; pulse: anchor
    ///     sub     a13, a13, a2     ; dt
    ///     movi a9, 0; movi a12, 4
    ///     loop    a12, 36          ; for i in 0..4
    ///     bbc     a10, a9, skip    ;   if pending & (1 << i)
    ///     l32i    a8, a6+4*i+64    ;     && dt >= end[i]
    ///     bltu    a13, a8, skip
    ///     s32i.n  a13, a6+4*i      ;   record dt, clear the bit
    ///     ...
    /// 36: bnez    a10, pulse
    ///     l32i    a4, a6, 80       ; tail deadline
    /// 3c: rsr.ccount a3; sub a3, a3, a2; bgeu a4, a3, 3c
    ///     rsr.ccount a5; s32i a5, a6, 32
    /// 50: rsr.ccount a3; add.n a7, a7, a3     ; accumulates: refuse
    ///     sub a3, a3, a2; bgeu a4, a3, 50     ; (a4 = a6[84])
    ///     s32i a7, a6, 36; rsr.ccount a5; s32i a5, a6, 40
    /// 64: rsr.ccount a3; bbci a3, 12, 64      ; bit test: refuse
    ///     rsr.ccount a5; s32i.n a5, a6, 44
    /// 6f: waiti 0; j 6f
    /// ```
    const FRAME: [u8; 117] = [
        0x20, 0xea, 0x03, 0x0c, 0xfa, 0xd0, 0xea, 0x03, 0x20, 0xdd, 0xc0, 0x92, 0xa0, 0x00, 0xc2,
        0xa0, 0x04, 0x76, 0x8c, 0x21, 0x97, 0x5a, 0x1c, 0xe0, 0x89, 0x11, 0x6a, 0x88, 0x82, 0x28,
        0x10, 0x87, 0x3d, 0x11, 0xe0, 0x89, 0x11, 0x6a, 0x88, 0xd9, 0x08, 0x0c, 0x1b, 0x00, 0x19,
        0x40, 0x00, 0xbb, 0xa1, 0xb0, 0xaa, 0x30, 0x1b, 0x99, 0x56, 0xba, 0xfc, 0x42, 0x26, 0x14,
        0x30, 0xea, 0x03, 0x20, 0x33, 0xc0, 0x37, 0xb4, 0xf6, 0x50, 0xea, 0x03, 0x52, 0x66, 0x08,
        0x0c, 0x07, 0x42, 0x26, 0x15, 0x30, 0xea, 0x03, 0x3a, 0x77, 0x20, 0x33, 0xc0, 0x37, 0xb4,
        0xf4, 0x72, 0x66, 0x09, 0x50, 0xea, 0x03, 0x52, 0x66, 0x0a, 0x30, 0xea, 0x03, 0xc7, 0x63,
        0xf9, 0x50, 0xea, 0x03, 0x59, 0xb6, 0x00, 0x70, 0x00, 0x46, 0xfe, 0xff,
    ];
    /// `waiti 0; j .-3` — a core with nothing to do.
    const PARK: [u8; 6] = [0x00, 0x70, 0x00, 0x46, 0xfe, 0xff];
    /// `busy.S`, assembled with xtensa-esp32-elf-as (esp-2021r2):
    ///
    /// ```text
    ///  0: movi a3, 1250; slli a3, a3, 2   ; 5000 iterations
    ///     movi a2, 0
    ///  9: addi a2, a2, 1; s32i a2, a6, 0x100; bne a2, a3, 9
    ///     movi a4, 944; slli a4, a4, 4    ; 15104
    ///     s32i a4, a6, 68                 ; output 1's deadline
    /// 1b: addi a2, a2, 1; s32i a2, a6, 0x100; j 1b
    /// ```
    const BUSY: [u8; 36] = [
        0x32, 0xa4, 0xe2, 0xe0, 0x33, 0x11, 0x22, 0xa0, 0x00, 0x22, 0xc2, 0x01, 0x22, 0x66, 0x40,
        0x37, 0x92, 0xf6, 0x42, 0xa3, 0xb0, 0xc0, 0x44, 0x11, 0x42, 0x66, 0x11, 0x22, 0xc2, 0x01,
        0x22, 0x66, 0x40, 0x86, 0xfd, 0xff,
    ];
    /// Output 1's deadline after the busy primary's store.
    const MOVED_END: u32 = 15_104;

    /// Pulse deadlines (cycles after t0), the frame-tail deadline, and the
    /// accumulating loop's deadline.
    const PULSE_END: [u32; 4] = [12_000, 24_000, 18_000, 30_000];
    const TAIL_END: u32 = 480_000;
    const ACC_END: u32 = 500_000;
    const RUN_CYCLES: u64 = 600_000;

    struct Outcome {
        records: Vec<u32>,
        ccount: u32,
        total_cycles: u64,
        skipped: u64,
        poll_skipped: u64,
        deferred: u64,
        batches: u64,
    }

    #[derive(Clone, Copy, PartialEq)]
    enum Primary {
        Alone,
        Parked,
        Busy,
    }

    fn load(bus: &mut SystemBus, at: u64, code: &[u8]) {
        for (i, b) in code.iter().enumerate() {
            bus.write_u8(at + i as u64, *b).unwrap();
        }
    }

    fn arm(cpu: &mut XtensaLx7, pc: u64) {
        cpu.pc = pc as u32;
        cpu.regs.write_logical(6, DATA);
        cpu.sr.write(CCOUNT, 0);
    }

    fn run(mode: Primary, fast_forward: bool) -> Outcome {
        run_on(mode, fast_forward, true)
    }

    #[derive(Debug)]
    struct Quiet;
    impl labwired_core::SimulationObserver for Quiet {}

    /// `batched`: the primary may run as one batch beside a deferred
    /// secondary (`Machine::run_primary_beside_quiet_secondary`). Otherwise an
    /// observer is attached, which keeps every window at one instruction per
    /// tick.
    fn run_on(mode: Primary, fast_forward: bool, batched: bool) -> Outcome {
        let dual = mode != Primary::Alone;
        let mut bus = SystemBus::new();
        let mut primary = configure_xtensa_esp32(&mut bus);
        load(&mut bus, CODE, &FRAME);
        load(
            &mut bus,
            IDLE,
            if mode == Primary::Busy { &BUSY } else { &PARK },
        );
        for (i, end) in PULSE_END.iter().enumerate() {
            bus.write_u32(u64::from(DATA) + 64 + 4 * i as u64, *end)
                .unwrap();
        }
        bus.write_u32(u64::from(DATA) + 80, TAIL_END).unwrap();
        bus.write_u32(u64::from(DATA) + 84, ACC_END).unwrap();
        let mut machine = if dual {
            arm(&mut primary, IDLE);
            let mut app = XtensaLx7::new_app_cpu();
            arm(&mut app, CODE);
            app.unhalt();
            Machine::new(primary, bus).with_secondary_cpu(app)
        } else {
            arm(&mut primary, CODE);
            Machine::new(primary, bus)
        };
        machine.config.idle_fast_forward_enabled = fast_forward;
        if !batched {
            machine.add_observer(std::sync::Arc::new(Quiet));
        }
        while machine.total_cycles < RUN_CYCLES {
            machine
                .advance(AdvanceRequest::run(Some(RUN_CYCLES - machine.total_cycles)))
                .unwrap();
        }
        let records = (0..12)
            .map(|i| machine.bus.read_u32(u64::from(DATA) + 4 * i).unwrap())
            .collect();
        let runner: &XtensaLx7 = if dual {
            machine.cpu_secondary.as_ref().unwrap()
        } else {
            &machine.cpu
        };
        Outcome {
            records,
            ccount: runner.sr.read(CCOUNT),
            total_cycles: machine.total_cycles,
            skipped: machine.idle_fast_forward_cycles_skipped,
            poll_skipped: machine.poll_loop_cycles_skipped,
            deferred: machine.poll_loop_cycles_deferred,
            batches: machine.primary_batches_beside_deferred,
        }
    }

    fn check(dual: bool) {
        let mode = if dual {
            Primary::Parked
        } else {
            Primary::Alone
        };
        let fast = run(mode, true);
        let reference = run(mode, false);
        assert_eq!(reference.skipped, 0);
        // The program ran to the end in the reference run.
        assert!(
            reference.records[11] != 0,
            "reference never reached the end"
        );
        assert_eq!(
            fast.records, reference.records,
            "the skip changed what the firmware recorded"
        );
        assert_eq!(fast.ccount, reference.ccount);
        assert_eq!(fast.total_cycles, reference.total_cycles);
        // Each output dropped at the first iteration with dt >= its deadline.
        for (i, end) in PULSE_END.iter().enumerate() {
            let dt = fast.records[i];
            assert!(
                dt >= *end && dt < end + 64,
                "output {i}: dt {dt} vs end {end}"
            );
        }
        // The skip covered most of the pulse and tail waiting (~480 k
        // cycles), and none of the two loops it must refuse (another ~24 k).
        assert!(
            fast.poll_skipped > 440_000,
            "poll-loop skip did not fire: skipped {} of the ~480 k busy cycles",
            fast.poll_skipped
        );
        assert!(
            fast.poll_skipped < u64::from(TAIL_END),
            "the skip ran through a loop it must refuse ({} cycles)",
            fast.poll_skipped
        );
    }

    #[test]
    fn poll_loop_skip_on_the_only_core_changes_nothing() {
        check(false);
    }

    #[test]
    fn poll_loop_skip_on_an_active_secondary_changes_nothing() {
        check(true);
    }

    #[test]
    fn deferred_poll_loop_beside_a_busy_primary_changes_nothing() {
        let fast = check_busy(false);
        assert_eq!(fast.batches, 0, "an observed machine ran a batch");
    }

    /// The same, with the primary run as batches beside the deferred loop.
    /// The batch must end on the primary's store into the deadline the loop
    /// reads (`bus::BatchStop`); without that stop the loop catches up on the
    /// old deadline and the records differ.
    #[test]
    fn deferred_poll_loop_beside_a_batched_primary_changes_nothing() {
        let fast = check_busy(true);
        assert!(fast.batches > 0, "the primary never ran as a batch");
    }

    fn check_busy(batched: bool) -> Outcome {
        let fast = run_on(Primary::Busy, true, batched);
        let reference = run_on(Primary::Busy, false, batched);
        assert_eq!(reference.skipped + reference.deferred, 0);
        assert!(
            reference.records[11] != 0,
            "reference never reached the end"
        );
        // The primary's store landed inside the wait: output 1 dropped at the
        // moved deadline, not the one the secondary's loop first read.
        let dt1 = reference.records[1];
        assert!(
            (MOVED_END..MOVED_END + 64).contains(&dt1),
            "reference output 1: dt {dt1} vs moved end {MOVED_END}"
        );
        assert_eq!(
            fast.records, reference.records,
            "the deferral changed what the firmware recorded"
        );
        assert_eq!(fast.ccount, reference.ccount);
        assert_eq!(fast.total_cycles, reference.total_cycles);
        // The primary never idles, so nothing machine-wide was skipped; the
        // secondary's pulse and tail waiting (~480 k cycles) ran deferred.
        assert_eq!(fast.skipped, 0);
        assert!(
            fast.deferred > 440_000,
            "deferral did not fire: {} of the ~480 k busy cycles",
            fast.deferred
        );
        fast
    }

    /// `ipi.S` (primary): count 1000 stores, write 1 to DPORT `FROM_CPU_1`
    /// (a5), count on forever.
    const IPI: [u8; 30] = [
        0x32, 0xa3, 0xe8, 0x22, 0xa0, 0x00, 0x22, 0xc2, 0x01, 0x22, 0x66, 0x40, 0x37, 0x92, 0xf6,
        0x42, 0xa0, 0x01, 0x42, 0x65, 0x00, 0x22, 0xc2, 0x01, 0x22, 0x66, 0x40, 0x86, 0xfd, 0xff,
    ];
    /// `handler.S` (APP kernel vector): record CCOUNT at a6[0], then count
    /// into a6[4] forever.
    const HANDLER: [u8; 18] = [
        0x20, 0xea, 0x03, 0x22, 0x66, 0x00, 0x32, 0xa0, 0x00, 0x32, 0xc3, 0x01, 0x32, 0x66, 0x01,
        0x86, 0xfd, 0xff,
    ];
    const VECBASE: u64 = 0x4008_3000;
    const APP_DATA: u32 = DATA + 0x200;
    const IPI_RUN: u64 = 20_000;

    /// A WAITI-parked APP_CPU woken by the primary's cross-core interrupt in
    /// the middle of a window: returns (handler CCOUNT, handler count, APP
    /// CCOUNT, machine cycles) after `IPI_RUN` cycles advanced in `chunk`-
    /// cycle calls.
    fn run_ipi(chunk: u64, fast_forward: bool, observed: bool) -> (u32, u32, u32, u64) {
        use labwired_core::cpu::xtensa_regs::Ps;
        use labwired_core::cpu::xtensa_sr::{INTENABLE, VECBASE as VECBASE_SR};
        let mut bus = SystemBus::new();
        let mut primary = configure_xtensa_esp32(&mut bus);
        load(&mut bus, IDLE, &IPI);
        load(&mut bus, CODE, &PARK);
        load(&mut bus, VECBASE + 0x300, &HANDLER);
        // DPORT: route FROM_CPU_1 (source 25) to APP_CPU interrupt 2 (level 1).
        bus.write_u32(0x3ff0_0208 + 25 * 4, 2).unwrap();
        arm(&mut primary, IDLE);
        primary.regs.write_logical(5, 0x3ff0_00e0);
        let mut app = XtensaLx7::new_app_cpu();
        arm(&mut app, CODE);
        app.regs.write_logical(6, APP_DATA);
        app.ps = Ps::from_raw(0);
        app.sr.write(VECBASE_SR, VECBASE as u32);
        app.sr.write(INTENABLE, 1 << 2);
        app.unhalt();
        let mut machine = Machine::new(primary, bus).with_secondary_cpu(app);
        machine.config.idle_fast_forward_enabled = fast_forward;
        if observed {
            machine.add_observer(std::sync::Arc::new(Quiet));
        }
        while machine.total_cycles < IPI_RUN {
            let n = chunk.min(IPI_RUN - machine.total_cycles);
            machine.advance(AdvanceRequest::run(Some(n))).unwrap();
        }
        let app = machine.cpu_secondary.as_ref().unwrap();
        (
            machine.bus.read_u32(u64::from(APP_DATA)).unwrap(),
            machine.bus.read_u32(u64::from(APP_DATA) + 4).unwrap(),
            app.sr.read(CCOUNT),
            machine.total_cycles,
        )
    }

    /// A parked secondary woken inside a window runs from the next cycle: the
    /// result does not depend on where `advance` calls end. The quantum-1
    /// window used to step a secondary only while it was parked, so after a
    /// mid-window wake it sat unstepped until the window ended; ending the
    /// calls at different cycles moved the handler's CCOUNT and count.
    #[test]
    fn a_parked_secondary_woken_mid_window_runs_from_the_next_cycle() {
        let reference = run_ipi(1, false, true);
        assert!(
            reference.0 > 3_000,
            "APP never took the interrupt: {reference:?}"
        );
        assert!(
            reference.1 > 1_000,
            "APP handler did not run on: {reference:?}"
        );
        // Observed and unobserved: an observer changes how a window steps
        // (no store-spin coalescing), never when the secondary wakes.
        for observed in [true, false] {
            for fast_forward in [false, true] {
                for chunk in [IPI_RUN, 1_000, 997, 64] {
                    assert_eq!(
                        run_ipi(chunk, fast_forward, observed),
                        reference,
                        "chunk {chunk}, fast-forward {fast_forward}, observed {observed}"
                    );
                }
            }
        }
    }
}
