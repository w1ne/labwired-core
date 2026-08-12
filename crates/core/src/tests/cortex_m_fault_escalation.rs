// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! Guard: ARMv7-M **fault escalation** for a precise data-access fault.
//!
//! `crates/core/src/tests/cortex_m_memory_contract.rs` (#880) pins the
//! *contract*: a load or store that the bus cannot serve returns
//! `Err(SimulationError::MemoryViolation)` instead of fabricating state. That is
//! correct as a contract and it matches RISC-V — but it **aborts the run**.
//!
//! Silicon does not stop. A precise data-access fault takes an exception
//! (ARMv7-M ARM B1.5.14), and firmware that installs a `BusFault_Handler` is
//! entitled to handle it and carry on. Before this module, the Cortex-M model was
//! structurally incapable of raising one: a grep for `set_exception_pending(3|4|5|6)`
//! over all of `crates/core/src` returned nothing. The only production pend in the
//! execute path was `set_exception_pending(11)` — SVCall.
//!
//! The two headline guards below therefore assert what no amount of contract work
//! can give you:
//!
//! 1. `SHCSR.BUSFAULTENA` set + a `BusFault_Handler` installed → **the handler
//!    runs**, `CFSR.BFSR.PRECISERR` and `BFARVALID` are set, `BFAR` holds the
//!    faulting address, and **the run continues**.
//! 2. `SHCSR.BUSFAULTENA` clear → the fault **escalates to HardFault** with
//!    `HFSR.FORCED` set (B1.5.14: "a fault occurs and the handler for that fault
//!    is not enabled").
//!
//! Both directions are covered, because a change that faulted on *everything*
//! would pass a one-directional test:
//!
//! * with escalation **on**, a run of valid accesses must pend nothing and leave
//!   every fault status register at zero;
//! * with escalation **off** (the `LABWIRED_CORTEXM_FAULTS=0` opt-out), the
//!   #880 abort contract must be byte-for-byte what it is today —
//!   `Err(MemoryViolation)`, no exception, and the fault status registers not
//!   even served by the SCB.

#[cfg(test)]
mod tests {
    use crate::cpu::CortexM;
    use crate::{Bus, Cpu, Machine, SimulationError};

    // ---- SCS/SCB fault register addresses (ARMv7-M ARM B3.2.2, Table B3-4) ----
    /// System Handler Control and State Register.
    const SHCSR: u64 = 0xE000_ED24;
    /// Configurable Fault Status Register (MMFSR:BFSR:UFSR packed into one word).
    const CFSR: u64 = 0xE000_ED28;
    /// HardFault Status Register.
    const HFSR: u64 = 0xE000_ED2C;
    /// BusFault Address Register.
    const BFAR: u64 = 0xE000_ED38;

    /// `SHCSR.BUSFAULTENA`, bit 17 (B3.2.13).
    const SHCSR_BUSFAULTENA: u32 = 1 << 17;
    /// `CFSR.BFSR.PRECISERR` — BFSR bit 1, i.e. CFSR bit 9 (B3.2.15).
    const CFSR_PRECISERR: u32 = 1 << 9;
    /// `CFSR.BFSR.BFARVALID` — BFSR bit 7, i.e. CFSR bit 15 (B3.2.15).
    const CFSR_BFARVALID: u32 = 1 << 15;
    /// `HFSR.FORCED`, bit 30 (B3.2.16).
    const HFSR_FORCED: u32 = 1 << 30;
    /// `CFSR.BFSR.IMPRECISERR` — BFSR bit 2, i.e. CFSR bit 10 (B3.2.15). A
    /// synchronous data-access fault is precise; this bit must stay clear, and
    /// BFAR is architecturally meaningless when it is the one that is set.
    const CFSR_IMPRECISERR: u32 = 1 << 10;
    /// `CFSR.BFSR.STKERR` — BFSR bit 4, i.e. CFSR bit 12 (B3.2.15): the
    /// exception-entry stacking fault. A different fault from PRECISERR.
    const CFSR_STKERR: u32 = 1 << 12;
    /// `CFSR.BFSR.UNSTKERR` — BFSR bit 3, i.e. CFSR bit 11 (B3.2.15): the
    /// exception-return unstacking fault.
    const CFSR_UNSTKERR: u32 = 1 << 11;
    /// The exact CFSR a precise data-access fault must leave behind: PRECISERR
    /// and BFARVALID, and nothing else — no MMFSR bits (the MPU is not
    /// enforced), no UFSR bits, no IBUSERR/IMPRECISERR/STKERR/UNSTKERR.
    const CFSR_PRECISE_DATA_FAULT: u32 = CFSR_PRECISERR | CFSR_BFARVALID;

    /// Covered by no memory region and no peripheral window on a default
    /// `SystemBus` (flash 0x0000_0000..0x0010_0000, RAM 0x2000_0000..0x2010_0000,
    /// peripherals at 0x4000_C000 / 0x4001_0800 / 0x4002_1000 / 0xE000_E010 and
    /// the SCB/NVIC/DWT block). Also outside both bit-band alias windows.
    const UNMAPPED: u32 = 0x9000_0000;
    /// Mapped, writable RAM.
    const MAPPED: u32 = 0x2000_0100;
    /// Where the faulting instruction lives — always mapped, so the *fetch*
    /// never faults and only the data access can. That separation is the whole
    /// point: `examples/ci/dummy-memory-violation.yaml` reaches
    /// `memory_violation` through the fetch path and proves nothing here.
    const CODE: u32 = 0x1000;
    /// Where the fake `HardFault_Handler` lives.
    const HARDFAULT_HANDLER: u32 = 0x2000;
    /// Where the fake `BusFault_Handler` lives.
    const BUSFAULT_HANDLER: u32 = 0x3000;
    /// Top of the stack. `frame_ptr = SP - 32` stays inside RAM.
    const STACK_TOP: u32 = 0x2000_8000;

    /// `LDR r0, [r1, #0]` — T1 encoding, rn = r1, rt = r0.
    const LDR_R0_R1: u16 = 0x6808;
    /// `STR r0, [r1, #0]` — T1 encoding, rn = r1, rt = r0.
    const STR_R0_R1: u16 = 0x6008;
    /// `BX lr` — the exception-return instruction when LR holds an EXC_RETURN.
    const BX_LR: u16 = 0x4770;
    /// `B .` — an infinite self-loop, the classic default fault handler body.
    const B_SELF: u16 = 0xE7FE;

    /// A machine whose vector table has both fault handlers installed, with a
    /// single `LDR r0,[r1]` at `CODE` and a real stack.
    ///
    /// `faults_enabled` selects the model under test; it is the flag this whole
    /// change lives behind, default **off**.
    fn faulting_machine(faults_enabled: bool) -> Machine<CortexM> {
        let mut bus = crate::bus::SystemBus::new();
        let (cpu, _nvic) = crate::system::cortex_m::configure_cortex_m(&mut bus);

        // Vector table at VTOR = 0. Thumb bit set on both handler addresses, as
        // any real vector table has it.
        bus.write_u32(0x0C, HARDFAULT_HANDLER | 1).unwrap(); // exception 3
        bus.write_u32(0x14, BUSFAULT_HANDLER | 1).unwrap(); // exception 5

        // Handler bodies: `B .`
        bus.write_u16(HARDFAULT_HANDLER as u64, B_SELF).unwrap();
        bus.write_u16(BUSFAULT_HANDLER as u64, B_SELF).unwrap();
        // The faulting instruction, then a self-loop so a run that *doesn't*
        // fault still terminates cleanly instead of wandering into erased flash.
        bus.write_u16(CODE as u64, LDR_R0_R1).unwrap();
        bus.write_u16((CODE + 2) as u64, B_SELF).unwrap();

        let mut machine = Machine::new(cpu, bus);
        machine.cpu.set_faults_enabled(faults_enabled);
        machine.cpu.set_pc(CODE);
        machine.cpu.set_sp(STACK_TOP);
        machine
    }

    /// Step until the PC leaves `CODE`, or `budget` steps elapse. Returns the
    /// number of steps taken. Any `Err` fails the test loudly — the entire point
    /// of escalation is that the run does not die.
    fn run_alive(machine: &mut Machine<CortexM>, budget: u32) -> u32 {
        for i in 0..budget {
            machine.step().unwrap_or_else(|e| {
                panic!(
                    "the run must survive a handled fault, but step {i} returned {e:?} \
                     (pc = 0x{:08X})",
                    machine.cpu.get_pc()
                )
            });
        }
        budget
    }

    // -------------------------------------------------------------------------
    // GUARD 1 — BUSFAULTENA set: the BusFault handler runs, and the run lives.
    // -------------------------------------------------------------------------

    /// ARMv7-M B1.5.14. A precise data-access fault with `SHCSR.BUSFAULTENA` set
    /// and an execution priority that lets BusFault preempt takes **BusFault
    /// (exception 5)** — it does not escalate, and it does not stop the core.
    ///
    /// This is the claim the product makes and the model could not honour: a
    /// `BusFault_Handler` in the firmware never ran, because nothing ever pended
    /// exception 5.
    #[test]
    fn precise_data_fault_takes_busfault_when_enabled_and_the_run_continues() {
        let mut machine = faulting_machine(true);
        machine
            .bus
            .write_u32(SHCSR, SHCSR_BUSFAULTENA)
            .expect("SHCSR must be inside the SCB window");
        assert_eq!(
            machine.bus.read_u32(SHCSR).unwrap() & SHCSR_BUSFAULTENA,
            SHCSR_BUSFAULTENA,
            "BUSFAULTENA must read back set — otherwise this test cannot \
             distinguish the enabled case from the escalated one"
        );
        machine.cpu.set_register(1, UNMAPPED);

        // 1st step: the LDR faults. 2nd step: the exception is taken.
        run_alive(&mut machine, 2);

        assert_eq!(
            machine.cpu.get_pc(),
            BUSFAULT_HANDLER,
            "the installed BusFault_Handler must run. pc = 0x{:08X}",
            machine.cpu.get_pc()
        );
        let cfsr = machine.bus.read_u32(CFSR).unwrap();
        assert_eq!(
            cfsr & CFSR_PRECISERR,
            CFSR_PRECISERR,
            "CFSR.BFSR.PRECISERR must be set for a precise data-access fault \
             (B3.2.15). CFSR = 0x{cfsr:08X}"
        );
        assert_eq!(
            cfsr & CFSR_BFARVALID,
            CFSR_BFARVALID,
            "CFSR.BFSR.BFARVALID must be set when BFAR holds the faulting \
             address. CFSR = 0x{cfsr:08X}"
        );
        assert_eq!(
            machine.bus.read_u32(BFAR).unwrap(),
            UNMAPPED,
            "BFAR must hold the address the access faulted on"
        );
        assert_eq!(
            machine.bus.read_u32(HFSR).unwrap() & HFSR_FORCED,
            0,
            "HFSR.FORCED must stay clear: BusFault was enabled and takeable, so \
             nothing escalated"
        );

        // And it keeps running — the handler's `B .` spins forever without ever
        // returning an Err.
        run_alive(&mut machine, 50);
        assert_eq!(
            machine.cpu.get_pc(),
            BUSFAULT_HANDLER,
            "still inside the handler after 50 more steps"
        );
    }

    // -------------------------------------------------------------------------
    // GUARD 2 — BUSFAULTENA clear: escalation to HardFault with HFSR.FORCED.
    // -------------------------------------------------------------------------

    /// ARMv7-M B1.5.14, escalation rule: *"a fault occurs and the handler for
    /// that fault is not enabled"* → the fault escalates to **HardFault
    /// (exception 3)** and `HFSR.FORCED` is set. The original fault's status is
    /// still recorded, which is exactly how every `HardFault_Handler` in the wild
    /// decodes what went wrong: it reads CFSR and BFAR.
    #[test]
    fn precise_data_fault_escalates_to_hardfault_when_busfault_disabled() {
        let mut machine = faulting_machine(true);
        assert_eq!(
            machine.bus.read_u32(SHCSR).unwrap() & SHCSR_BUSFAULTENA,
            0,
            "BUSFAULTENA must be clear out of reset for this test to mean anything"
        );
        machine.cpu.set_register(1, UNMAPPED);

        run_alive(&mut machine, 2);

        assert_eq!(
            machine.cpu.get_pc(),
            HARDFAULT_HANDLER,
            "with BusFault disabled the fault must escalate to HardFault. \
             pc = 0x{:08X}",
            machine.cpu.get_pc()
        );
        assert_eq!(
            machine.bus.read_u32(HFSR).unwrap() & HFSR_FORCED,
            HFSR_FORCED,
            "HFSR.FORCED must be set on escalation (B3.2.16). HFSR = 0x{:08X}",
            machine.bus.read_u32(HFSR).unwrap()
        );
        let cfsr = machine.bus.read_u32(CFSR).unwrap();
        assert_eq!(
            cfsr & (CFSR_PRECISERR | CFSR_BFARVALID),
            CFSR_PRECISERR | CFSR_BFARVALID,
            "the escalated-from fault's status must still be recorded, or a \
             HardFault_Handler cannot tell what happened. CFSR = 0x{cfsr:08X}"
        );
        assert_eq!(
            machine.bus.read_u32(BFAR).unwrap(),
            UNMAPPED,
            "BFAR must hold the faulting address even after escalation"
        );

        run_alive(&mut machine, 50);
    }

    // -------------------------------------------------------------------------
    // THE OTHER DIRECTION — nothing must fault that should not.
    // -------------------------------------------------------------------------

    /// With escalation **on**, a valid access must pend nothing, leave every
    /// fault status register at zero, and retire normally. A change that raised a
    /// BusFault on every access would pass both guards above.
    #[test]
    fn valid_access_pends_nothing_with_faults_enabled() {
        let mut machine = faulting_machine(true);
        machine
            .bus
            .write_u32(SHCSR, SHCSR_BUSFAULTENA)
            .expect("SHCSR write");
        machine.bus.write_u32(MAPPED as u64, 0xA5A5_1234).unwrap();
        machine.cpu.set_register(1, MAPPED);

        machine.step().expect("a valid load must retire");

        assert_eq!(
            machine.cpu.get_register(0),
            0xA5A5_1234,
            "a valid load must deliver the value"
        );
        assert_eq!(
            machine.cpu.get_pc(),
            CODE + 2,
            "pc must advance past the LDR"
        );
        assert_eq!(
            machine.bus.read_u32(CFSR).unwrap(),
            0,
            "CFSR must stay clear"
        );
        assert_eq!(
            machine.bus.read_u32(HFSR).unwrap(),
            0,
            "HFSR must stay clear"
        );
        assert_eq!(
            machine.bus.read_u32(BFAR).unwrap(),
            0,
            "BFAR must stay clear"
        );

        // And a long run of valid accesses stays alive and quiet.
        run_alive(&mut machine, 200);
        assert_eq!(machine.bus.read_u32(CFSR).unwrap(), 0, "CFSR still clear");
        assert_eq!(machine.bus.read_u32(HFSR).unwrap(), 0, "HFSR still clear");
    }

    /// A store into a real peripheral window must not fault with escalation on.
    /// Guards the "peripheral legitimately returns Err on a benign access" hazard
    /// from turning every MMIO write into a HardFault.
    #[test]
    fn peripheral_window_access_does_not_fault_with_faults_enabled() {
        let mut bus = crate::bus::SystemBus::new();
        let (cpu, _nvic) = crate::system::cortex_m::configure_cortex_m(&mut bus);
        bus.write_u16(CODE as u64, 0x6008).unwrap(); // STR r0,[r1,#0]
        bus.write_u16((CODE + 2) as u64, B_SELF).unwrap();
        let mut machine = Machine::new(cpu, bus);
        machine.cpu.set_faults_enabled(true);
        machine.cpu.set_pc(CODE);
        machine.cpu.set_sp(STACK_TOP);
        machine.cpu.set_register(1, 0x4000_C000); // uart1 base
        machine.cpu.set_register(0, 0x41); // 'A'

        machine.step().expect("a peripheral store must retire");

        assert_eq!(
            machine.bus.read_u32(CFSR).unwrap(),
            0,
            "CFSR must stay clear"
        );
        assert_eq!(
            machine.bus.read_u32(HFSR).unwrap(),
            0,
            "HFSR must stay clear"
        );
    }

    // -------------------------------------------------------------------------
    // FLAG OFF — #880's contract, unchanged, including the SCB register surface.
    // -------------------------------------------------------------------------

    /// With the flag **off** (the default, and what this PR ships), the #880
    /// abort contract must be exactly what it is today: `Err(MemoryViolation)`,
    /// no exception taken, no PC change.
    #[test]
    fn faults_disabled_keeps_the_880_abort_contract() {
        let mut machine = faulting_machine(false);
        machine.cpu.set_register(1, UNMAPPED);

        let step = machine.step();

        assert!(
            matches!(step, Err(SimulationError::MemoryViolation(a)) if a == UNMAPPED as u64),
            "with escalation off, an unmapped load must still abort the run \
             exactly as #880 made it. got {step:?}"
        );
        assert_eq!(
            machine.cpu.get_pc(),
            CODE,
            "no exception may be taken with the flag off"
        );
    }

    /// With the flag off the SCB must not even *serve* the fault status
    /// registers: they read 0 and swallow writes, exactly as before this change.
    /// This is what makes "flag off changes zero labs" true by construction
    /// rather than by hope — no firmware can observe a new read-back.
    #[test]
    fn faults_disabled_leaves_the_scb_fault_registers_unmodelled() {
        let mut machine = faulting_machine(false);
        for (name, addr) in [
            ("SHCSR", SHCSR),
            ("CFSR", CFSR),
            ("HFSR", HFSR),
            ("BFAR", BFAR),
        ] {
            machine.bus.write_u32(addr, 0xFFFF_FFFF).unwrap();
            assert_eq!(
                machine.bus.read_u32(addr).unwrap(),
                0,
                "{name} must read 0 with escalation off — the register surface is \
                 part of the flag, so flag-off is byte-identical to today"
            );
        }
    }

    /// The one place where flag-off is **not** byte-identical by construction.
    ///
    /// Everything else about this feature is gated by a register surface the SCB
    /// simply does not serve, or by an escalation path that is never entered.
    /// `masked_by_primask` is different: it sits on the *shared* dispatch path
    /// that every Cortex-M lab already runs through, and it changes PRIMASK from
    /// this core's historical blanket "block everything" into ARMv7-M B1.5.4's
    /// priority boost to 0 — which by construction cannot mask HardFault (-1) or
    /// NMI (-2).
    ///
    /// With the flag **off** it must be exactly `self.primask` again. The
    /// difference is only observable for an exception of negative priority, so
    /// that is what this pins: HardFault pended under PRIMASK must NOT dispatch
    /// with the flag off (blanket block), and MUST dispatch with it on
    /// (`fault_inside_a_primask_critical_section_takes_hardfault` covers the
    /// other side). Delete the `!faults_enabled()` early-out and this fails.
    #[test]
    fn primask_stays_a_blanket_block_with_the_flag_off() {
        for (faults_enabled, expect_dispatch) in [(false, false), (true, true)] {
            let mut machine = faulting_machine(faults_enabled);
            machine.cpu.primask = true;
            // Pend HardFault directly — priority -1, i.e. above PRIMASK's boost
            // to 0. No memory fault is involved, so this isolates the dispatch
            // rule from the escalation rule.
            machine.cpu.set_exception_pending(3);

            machine.step().expect("no memory access is involved here");

            let dispatched = machine.cpu.get_pc() == HARDFAULT_HANDLER;
            assert_eq!(
                dispatched,
                expect_dispatch,
                "faults_enabled={faults_enabled}: PRIMASK must be a blanket block \
                 with the flag OFF (historical behaviour, and what makes every \
                 existing lab byte-identical) and a boost to priority 0 with it \
                 ON (B1.5.4). pc = 0x{:08X}",
                machine.cpu.get_pc()
            );
        }
    }

    /// The path labs actually run on.
    ///
    /// Every guard above drives `Machine::step`, i.e. `advance(single())`, a
    /// one-instruction quantum. Real runs go through `advance(run(..))`, which
    /// takes `CortexM::step_batch` — a different loop, with its own
    /// takeable-exception early-break that this change also touched. A fault
    /// that escalated correctly at quantum 1 and was swallowed by the batch loop
    /// would leave every test above green and every lab unchanged, which is
    /// precisely the shape of blast-radius measurement that proves nothing.
    #[test]
    fn escalation_works_on_the_batched_run_path_too() {
        use crate::AdvanceRequest;
        let mut machine = faulting_machine(true);
        machine.bus.write_u32(SHCSR, SHCSR_BUSFAULTENA).unwrap();
        machine.cpu.set_register(1, UNMAPPED);
        assert!(
            machine.config.batch_mode_enabled,
            "this test is only meaningful on the batched path"
        );

        machine
            .advance(AdvanceRequest::run(Some(64)))
            .expect("the batched run must survive a handled fault");

        assert_eq!(
            machine.cpu.get_pc(),
            BUSFAULT_HANDLER,
            "the BusFault handler must run on the batched path too. pc = 0x{:08X}",
            machine.cpu.get_pc()
        );
        assert_eq!(
            machine.bus.read_u32(BFAR).unwrap(),
            UNMAPPED,
            "BFAR must hold the faulting address"
        );
    }

    // -------------------------------------------------------------------------
    // THE ESCALATION RULE ITSELF — the priority half, not just the enable half.
    // -------------------------------------------------------------------------

    /// ARMv7-M B1.5.14: *"an exception handler causes a fault for which the
    /// priority is the same as or lower than the currently executing
    /// exception"* → escalate.
    ///
    /// BusFault is **enabled** here, so only priority can decide. `BASEPRI` is
    /// boosted to 0x10 while BusFault sits at 0x20 (SHPR1 byte 1): 0x20 is
    /// numerically larger, i.e. lower priority, so BusFault cannot preempt and
    /// the fault must escalate. Getting this half wrong is invisible to the
    /// enable-flag tests above — the handler would still run, just at a moment
    /// silicon would never run it.
    #[test]
    fn enabled_busfault_still_escalates_when_priority_does_not_permit() {
        // B1.5.14 says "the same as or lower than", so BOTH the strictly-lower
        // case and the EQUAL case must escalate. The equal case is the boundary:
        // an implementation that compared `<=` instead of `<` would take a
        // BusFault that cannot actually pre-empt, pend an exception it can never
        // dispatch, and spin on the faulting instruction.
        for (busfault_prio, basepri, case) in [
            (0x20u32, 0x10u8, "lower priority than BASEPRI"),
            (0x20u32, 0x20u8, "the SAME priority as BASEPRI"),
        ] {
            let mut machine = faulting_machine(true);
            machine.bus.write_u32(SHCSR, SHCSR_BUSFAULTENA).unwrap();
            // SHPR1 (0xE000ED18) byte 1 = BusFault priority.
            machine
                .bus
                .write_u32(0xE000_ED18, busfault_prio << 8)
                .unwrap();
            machine.cpu.basepri = basepri;
            machine.cpu.set_register(1, UNMAPPED);

            run_alive(&mut machine, 2);

            assert_eq!(
                machine.cpu.get_pc(),
                HARDFAULT_HANDLER,
                "BusFault at 0x{busfault_prio:02X} has {case} (0x{basepri:02X}), \
                 so B1.5.14 escalates it. pc = 0x{:08X} — BUSFAULT_HANDLER means \
                 the priority rule let it through, CODE means it pended an \
                 exception it cannot deliver",
                machine.cpu.get_pc()
            );
            assert_eq!(
                machine.bus.read_u32(HFSR).unwrap() & HFSR_FORCED,
                HFSR_FORCED,
                "HFSR.FORCED marks the escalation ({case})"
            );
        }
    }

    /// `PRIMASK` boosts the execution priority to 0 (B1.5.4). A BusFault at any
    /// configurable priority is therefore blocked and escalates — and the
    /// resulting HardFault, at priority -1, **must still be taken**: PRIMASK
    /// cannot mask it.
    ///
    /// This is the case that would otherwise wedge the core. The dispatch loop's
    /// historical blanket `!self.primask` guard would pend the HardFault and
    /// never dispatch it, and the faulting instruction would re-execute forever
    /// inside a `__disable_irq()` critical section.
    #[test]
    fn fault_inside_a_primask_critical_section_takes_hardfault() {
        let mut machine = faulting_machine(true);
        machine.bus.write_u32(SHCSR, SHCSR_BUSFAULTENA).unwrap();
        machine.cpu.primask = true;
        machine.cpu.set_register(1, UNMAPPED);

        run_alive(&mut machine, 2);

        assert_eq!(
            machine.cpu.get_pc(),
            HARDFAULT_HANDLER,
            "HardFault (priority -1) is not maskable by PRIMASK (priority 0). \
             pc = 0x{:08X} — CODE means the core is spinning on the faulting \
             instruction with an undeliverable pend",
            machine.cpu.get_pc()
        );
    }

    /// A fault taken *inside* the HardFault handler cannot preempt HardFault:
    /// `-1 >= -1`. On silicon that is LOCKUP (B1.5.15), which this model does not
    /// have — so the original `Err` is left to stop the run rather than pending
    /// an exception that can never dispatch.
    ///
    /// Without this fallback the core would silently re-execute the faulting
    /// instruction until `max_steps`, which is a worse lie than aborting.
    #[test]
    fn fault_inside_the_hardfault_handler_stops_the_run_instead_of_spinning() {
        let mut machine = faulting_machine(true);
        // HardFault handler body: `LDR r0,[r1]` on the same bad address.
        machine
            .bus
            .write_u16(HARDFAULT_HANDLER as u64, LDR_R0_R1)
            .unwrap();
        machine.cpu.set_register(1, UNMAPPED);

        run_alive(&mut machine, 2); // fault, then enter HardFault
        assert_eq!(machine.cpu.get_pc(), HARDFAULT_HANDLER);

        let step = machine.step();
        assert!(
            matches!(step, Err(SimulationError::MemoryViolation(a)) if a == UNMAPPED as u64),
            "a fault inside HardFault is LOCKUP on silicon; with no lockup model \
             the run must stop rather than spin. got {step:?}"
        );
    }

    /// The flag must reach the SCB too, not just the CPU: with escalation on,
    /// SHCSR round-trips. One shared state, one flag — if these ever forked, the
    /// CPU could escalate while firmware could not enable the handler.
    #[test]
    fn faults_enabled_makes_the_scb_fault_registers_round_trip() {
        let mut machine = faulting_machine(true);
        machine.bus.write_u32(SHCSR, SHCSR_BUSFAULTENA).unwrap();
        assert_eq!(
            machine.bus.read_u32(SHCSR).unwrap(),
            SHCSR_BUSFAULTENA,
            "SHCSR must round-trip once escalation is on"
        );
        // CFSR is write-1-to-clear (B3.2.15): writing the bits back clears them,
        // which is how cortex-m-rt's fault handlers acknowledge a fault.
        machine.cpu.set_register(1, UNMAPPED);
        run_alive(&mut machine, 2);
        let cfsr = machine.bus.read_u32(CFSR).unwrap();
        assert_ne!(cfsr, 0, "a fault must have been recorded");
        machine.bus.write_u32(CFSR, cfsr).unwrap();
        assert_eq!(
            machine.bus.read_u32(CFSR).unwrap(),
            0,
            "CFSR is write-1-to-clear"
        );
    }

    // -------------------------------------------------------------------------
    // THE STORE SIDE — `CortexM::store` latches too, not just `load`.
    // -------------------------------------------------------------------------

    /// Every guard above faults on an `LDR`. `CortexM::load` and
    /// `CortexM::store` latch `pending_data_fault` independently, so a `store`
    /// that propagated its `Err` without latching would keep the whole store
    /// half of the instruction set on the abort path — every guard here would
    /// still be green, and firmware writing to a bad address would still stop
    /// the run instead of taking a BusFault.
    ///
    /// ARMv7-M does not distinguish read from write for PRECISERR (B3.2.15):
    /// both are precise data-access faults and both set BFAR.
    #[test]
    fn a_faulting_store_escalates_exactly_like_a_faulting_load() {
        let mut machine = faulting_machine(true);
        // Replace the LDR at CODE with an STR to the same bad address.
        machine
            .bus
            .write_u16(CODE as u64, STR_R0_R1)
            .expect("CODE is mapped flash");
        machine.bus.write_u32(SHCSR, SHCSR_BUSFAULTENA).unwrap();
        machine.cpu.set_register(0, 0xDEAD_BEEF);
        machine.cpu.set_register(1, UNMAPPED);

        run_alive(&mut machine, 2);

        assert_eq!(
            machine.cpu.get_pc(),
            BUSFAULT_HANDLER,
            "a faulting STR must take BusFault just as a faulting LDR does. \
             pc = 0x{:08X}",
            machine.cpu.get_pc()
        );
        assert_eq!(
            machine.bus.read_u32(CFSR).unwrap(),
            CFSR_PRECISE_DATA_FAULT,
            "a faulting store is a precise data-access fault: PRECISERR + \
             BFARVALID, nothing else"
        );
        assert_eq!(
            machine.bus.read_u32(BFAR).unwrap(),
            UNMAPPED,
            "BFAR must hold the address the store faulted on"
        );
    }

    // -------------------------------------------------------------------------
    // THE EXACT STATUS WORD — not merely "the bit I expect is present".
    // -------------------------------------------------------------------------

    /// `cfsr & PRECISERR == PRECISERR` also passes for an implementation that
    /// sets every BFSR bit it can think of. ARMv7-M B3.2.15 is specific: a
    /// synchronous data-access fault sets **PRECISERR**, and BFARVALID because
    /// BFAR is written. `IMPRECISERR` in particular is the *contradictory* bit —
    /// it means the fault is asynchronous and BFAR is meaningless — and STKERR /
    /// UNSTKERR name entirely different events.
    ///
    /// Likewise HFSR: on a non-escalated BusFault it must be *zero*, not merely
    /// "FORCED clear", and on an escalated one exactly `FORCED` — not VECTTBL
    /// (a vector-table read fault) and not DEBUGEVT.
    #[test]
    fn the_fault_status_words_are_exact_not_merely_supersets() {
        // Non-escalated: BusFault enabled and takeable.
        let mut taken = faulting_machine(true);
        taken.bus.write_u32(SHCSR, SHCSR_BUSFAULTENA).unwrap();
        taken.cpu.set_register(1, UNMAPPED);
        run_alive(&mut taken, 2);
        let cfsr = taken.bus.read_u32(CFSR).unwrap();
        assert_eq!(
            cfsr, CFSR_PRECISE_DATA_FAULT,
            "CFSR must be exactly PRECISERR|BFARVALID (0x{CFSR_PRECISE_DATA_FAULT:08X}), \
             got 0x{cfsr:08X}. IMPRECISERR set here would contradict BFARVALID; \
             STKERR/UNSTKERR would name a fault that did not happen"
        );
        assert_eq!(
            cfsr & CFSR_IMPRECISERR,
            0,
            "a synchronous data-access fault is PRECISE — IMPRECISERR would say \
             BFAR is not trustworthy"
        );
        assert_eq!(
            cfsr & (CFSR_STKERR | CFSR_UNSTKERR),
            0,
            "no stacking or unstacking happened"
        );
        assert_eq!(
            taken.bus.read_u32(HFSR).unwrap(),
            0,
            "HFSR must be untouched when nothing escalated"
        );

        // Escalated: BusFault not enabled.
        let mut escalated = faulting_machine(true);
        escalated.cpu.set_register(1, UNMAPPED);
        run_alive(&mut escalated, 2);
        assert_eq!(
            escalated.bus.read_u32(CFSR).unwrap(),
            CFSR_PRECISE_DATA_FAULT,
            "escalation must not change WHICH fault happened, only who handles it"
        );
        assert_eq!(
            escalated.bus.read_u32(HFSR).unwrap(),
            HFSR_FORCED,
            "HFSR must be exactly FORCED — not VECTTBL (bit 1), which means the \
             vector-table read itself faulted, and not DEBUGEVT (bit 31)"
        );
    }

    // -------------------------------------------------------------------------
    // B1.5.6 — the stacked return address of a synchronous fault.
    // -------------------------------------------------------------------------

    /// ARMv7-M B1.5.6: for a **synchronous** fault the stacked PC is the address
    /// of the instruction that caused it, so a handler that fixes the cause can
    /// `BX LR` and have the access retried. (For an asynchronous exception it is
    /// the *next* instruction.) This is the whole reason the escalation path
    /// leaves the PC parked on the faulting instruction instead of advancing it.
    ///
    /// Nothing else in this file looks at the exception frame, so an
    /// implementation that advanced the PC past the LDR before pending would
    /// pass every other guard while silently stacking the wrong return address —
    /// and firmware that retries would skip the access instead of repeating it.
    #[test]
    fn the_stacked_return_address_is_the_faulting_instruction() {
        let mut machine = faulting_machine(true);
        machine.bus.write_u32(SHCSR, SHCSR_BUSFAULTENA).unwrap();
        machine.cpu.set_register(1, UNMAPPED);

        run_alive(&mut machine, 2);
        assert_eq!(machine.cpu.get_pc(), BUSFAULT_HANDLER);

        // Entry stacks 8 words on the preempted stack: R0,R1,R2,R3,R12,LR,PC,xPSR.
        let frame = STACK_TOP - 32;
        assert_eq!(
            machine.cpu.get_register(13),
            frame,
            "SP must have dropped by exactly one 8-word exception frame"
        );
        assert_eq!(
            machine.bus.read_u32((frame + 24) as u64).unwrap(),
            CODE,
            "the stacked PC of a SYNCHRONOUS fault is the faulting instruction \
             itself (B1.5.6), so `BX LR` retries the access. CODE+2 here would \
             mean the fault silently skipped the access"
        );
        assert_eq!(
            machine.bus.read_u32((frame + 4) as u64).unwrap(),
            UNMAPPED,
            "R1 (the address register) must be stacked as it was at the fault"
        );
        assert_eq!(
            machine.bus.read_u32((frame + 28) as u64).unwrap() & 0x1FF,
            0,
            "the stacked xPSR.IPSR must record Thread mode — the fault was taken \
             from thread context, not from another handler"
        );
    }

    // -------------------------------------------------------------------------
    // THE NON-ESCALATED FAULTS — BFAR is only meaningful when BFARVALID is set,
    // and only a PRECISE data access may set either.
    // -------------------------------------------------------------------------

    /// An **exception-entry stacking** fault is `BFSR.STKERR` (B3.2.15), not
    /// PRECISERR, and on silicon it can end in LOCKUP (B1.5.15). It must not be
    /// escalated: escalating it would re-enter the very stack that is broken,
    /// forever. It must also not touch BFAR — a handler that read BFAR after a
    /// stacking fault would be told the wrong address.
    ///
    /// This is what pins the `bus_store` (non-latching) choice at the stacking
    /// sites. Route them back through `store` and this test fails.
    #[test]
    fn an_exception_entry_stacking_fault_is_not_escalated_and_leaves_bfar_alone() {
        /// A second unmapped region, well away from `UNMAPPED`, so the two
        /// faults are distinguishable by address.
        const BAD_STACK: u32 = 0x9800_0000;

        let mut machine = faulting_machine(true);
        machine.bus.write_u32(SHCSR, SHCSR_BUSFAULTENA).unwrap();
        machine.cpu.set_register(1, UNMAPPED);
        machine.cpu.set_sp(BAD_STACK);

        // Step 1: the LDR faults and BusFault is pended (this much must work,
        // or the test would pass for the wrong reason).
        machine
            .step()
            .expect("the data fault itself must be handled");
        assert_eq!(
            machine.bus.read_u32(BFAR).unwrap(),
            UNMAPPED,
            "precondition: the data fault recorded its address"
        );

        // Step 2: taking the exception stacks onto BAD_STACK and cannot.
        let frame = BAD_STACK.wrapping_sub(32);
        let step = machine.step();
        assert!(
            matches!(step, Err(SimulationError::MemoryViolation(a)) if a == frame as u64),
            "a stacking fault is STKERR/LOCKUP territory, not a precise data \
             fault: it must surface as an abort at the frame address \
             0x{frame:08X}, never be escalated into another exception. got {step:?}"
        );
        assert_eq!(
            machine.bus.read_u32(BFAR).unwrap(),
            UNMAPPED,
            "BFAR must still hold the DATA address. Overwriting it with the \
             stack address would hand the handler the wrong fault"
        );
        assert_eq!(
            machine.bus.read_u32(CFSR).unwrap(),
            CFSR_PRECISE_DATA_FAULT,
            "the stacking fault must not add PRECISERR/BFARVALID for itself"
        );
    }

    /// The mirror case: an **exception-return unstacking** fault is
    /// `BFSR.UNSTKERR` (B3.2.15). `exception_return` reads the frame straight
    /// off the bus rather than through `CortexM::load`, so it does not latch —
    /// this guard is what stops a later "unify all the accesses" refactor from
    /// quietly turning an unstacking fault into a precise data fault with a
    /// bogus BFAR.
    #[test]
    fn an_exception_return_unstacking_fault_is_not_escalated() {
        let mut machine = faulting_machine(true);
        machine.bus.write_u32(SHCSR, SHCSR_BUSFAULTENA).unwrap();
        // The handler returns instead of spinning.
        machine
            .bus
            .write_u16(BUSFAULT_HANDLER as u64, BX_LR)
            .unwrap();
        machine.cpu.set_register(1, UNMAPPED);

        run_alive(&mut machine, 2);
        assert_eq!(machine.cpu.get_pc(), BUSFAULT_HANDLER, "in the handler");
        let cfsr_before = machine.bus.read_u32(CFSR).unwrap();
        let bfar_before = machine.bus.read_u32(BFAR).unwrap();
        assert_eq!(cfsr_before, CFSR_PRECISE_DATA_FAULT, "precondition");

        // Move the stack out from under the frame, then return.
        machine.cpu.set_sp(0x9800_0000);
        let step = machine.step();

        assert!(
            matches!(step, Err(SimulationError::MemoryViolation(_))),
            "an unstacking fault must surface as an abort, not be escalated into \
             a fresh precise data fault. got {step:?}"
        );
        assert_eq!(
            machine.bus.read_u32(CFSR).unwrap(),
            cfsr_before,
            "UNSTKERR is a different fault from PRECISERR — CFSR must not gain \
             a second precise-data-fault report"
        );
        assert_eq!(
            machine.bus.read_u32(BFAR).unwrap(),
            bfar_before,
            "BFAR must still name the original data address"
        );
    }
}
