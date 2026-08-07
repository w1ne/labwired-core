// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

#[cfg(test)]
mod scb_reset_tests {
    use crate::{Bus, Cpu, DebugControl, Machine};

    /// AIRCR address: SCB base (0xE000_ED00) + 0x0C.
    const SCB_AIRCR: u64 = 0xE000_ED0C;

    /// A real Cortex-M write to AIRCR with the correct VECTKEY (0x05FA in
    /// bits 31:16) and SYSRESETREQ (bit 2) must reboot the CPU through the
    /// vector table on the next instruction boundary: MSP reloads from
    /// vector[0] and PC from vector[1] (with the Thumb bit masked off),
    /// reusing the existing power-on reset path.
    #[test]
    fn sysresetreq_reboots_cpu_via_vector_table() {
        // Bare Cortex-M machine. `configure_cortex_m` registers the SCB at
        // 0xE000_ED00 with VTOR defaulting to 0, so the vector table lives at
        // address 0 (vector[0]=MSP, vector[1]=reset).
        let mut bus = crate::bus::SystemBus::new();
        let (cpu, _nvic) = crate::system::cortex_m::configure_cortex_m(&mut bus);
        let mut m = Machine::new(cpu, bus);

        const MSP: u32 = 0x2000_1000;
        const RESET_ADDR: u32 = 0x0800_0100;

        // Seed the vector table the same way power-on reset reads it.
        m.bus.write_u32(0x0000_0000, MSP).unwrap();
        m.bus.write_u32(0x0000_0004, RESET_ADDR | 1).unwrap(); // Thumb bit set

        // Place a harmless instruction (NOP, 0xBF00) at the current PC so the
        // step executes one full instruction before the reset latch is drained
        // — mirroring firmware whose AIRCR store completes, then the core
        // reboots at the next instruction boundary.
        const PC: u32 = 0x2000_0000;
        m.bus.write_u16(PC as u64, 0xBF00).unwrap(); // NOP
        m.cpu.set_pc(PC);
        m.cpu.set_sp(0x2000_8000);

        // Trigger the latch through the exact MMIO path firmware uses: a real
        // bus write to AIRCR. No test-only Scb setter.
        m.bus
            .write_u32(SCB_AIRCR, (0x05FA << 16) | (1 << 2))
            .unwrap();

        // One step: execute the NOP, then drain the SCB latch and reset.
        m.step().unwrap();

        assert_eq!(
            m.cpu.get_pc() & !1,
            RESET_ADDR,
            "PC must reload from vector[1] (reset vector) after SYSRESETREQ"
        );
        assert_eq!(
            m.cpu.get_register(13),
            MSP,
            "SP must reload from vector[0] (MSP) after SYSRESETREQ"
        );
    }

    /// Same SYSRESETREQ semantics, but exercised through the BATCHED execution
    /// path (`Machine::run`) instead of a single `step()`. The batched path
    /// bypasses `step()`, so it must drain the SCB reset latch on each batch
    /// boundary — mirroring the flash-op drain — or the firmware-requested
    /// reboot never fires. This regression guard fails without that drain
    /// (PC keeps spinning in the post-AIRCR loop) and passes with it.
    #[test]
    fn sysresetreq_reboots_cpu_via_run() {
        let mut bus = crate::bus::SystemBus::new();
        let (cpu, _nvic) = crate::system::cortex_m::configure_cortex_m(&mut bus);
        let mut m = Machine::new(cpu, bus);

        const MSP: u32 = 0x2000_1000;
        // Keep the reset vector in the same region the start PC executes from so
        // the post-reboot self-loop is fetchable on the bare test bus.
        const RESET_ADDR: u32 = 0x2000_0100;

        // Vector table at address 0 (VTOR defaults to 0).
        m.bus.write_u32(0x0000_0000, MSP).unwrap();
        m.bus.write_u32(0x0000_0004, RESET_ADDR | 1).unwrap(); // Thumb bit set

        // After the first instruction boundary drains the reset, the remaining
        // seven scheduling quanta execute from the reset vector. Eight NOPs make the
        // final PC prove both the reset boundary and the adapter's continued
        // execution, rather than hiding either behind a self-loop.
        for offset in (0..16).step_by(2) {
            m.bus
                .write_u16((RESET_ADDR + offset) as u64, 0xBF00)
                .unwrap();
        }

        // Firmware's pre-reset code: a NOP, then it would spin. We seed a NOP
        // self-loop at the start PC too, but the AIRCR latch is already armed,
        // so the first batch boundary must reboot before the loop matters.
        const PC: u32 = 0x2000_0000;
        m.bus.write_u16(PC as u64, 0xBF00).unwrap(); // NOP
        m.bus.write_u16((PC + 2) as u64, 0xE7FE).unwrap(); // b . (spin)
        m.cpu.set_pc(PC);
        m.cpu.set_sp(0x2000_8000);
        m.config.peripheral_tick_interval = 64;

        // Arm the reset latch via the exact MMIO path firmware uses.
        m.bus
            .write_u32(SCB_AIRCR, (0x05FA << 16) | (1 << 2))
            .unwrap();

        // Drive the run adapter. A small step budget is enough: the first
        // instruction boundary must drain the latch and reboot.
        m.run(Some(8)).unwrap();

        assert_eq!(
            m.cpu.get_pc() & !1,
            RESET_ADDR + 14,
            "seven post-reset NOPs must retire after the first batch drains SYSRESETREQ"
        );
        assert_eq!(
            m.cpu.get_register(13),
            MSP,
            "SP must reload from vector[0] (MSP) after SYSRESETREQ on the batched run path"
        );
        // The two asserts above are the fidelity contract, and they are
        // unchanged: the reset drains after exactly ONE retired instruction, so
        // seven post-reset NOPs land the PC at RESET_ADDR + 14 out of an
        // 8-instruction budget. What changed is only how many CPU batches that
        // costs. SCB presence used to pin the quantum to 1 for the life of the
        // bus (8 instructions ⇒ 8 batches); the latch shared with the core now
        // cuts exactly the batch that retires the AIRCR write, and the seven
        // NOPs after the reboot come back as one batch.
        assert!(
            m.step_profile().cpu_batches < 8,
            "the run must batch now, not spend one batch per instruction (got {})",
            m.step_profile().cpu_batches
        );
    }

    /// An AIRCR write missing the VECTKEY must NOT reset the CPU: the latch is
    /// never set, so `step()` leaves PC/SP advancing normally.
    #[test]
    fn aircr_without_vectkey_does_not_reboot() {
        let mut bus = crate::bus::SystemBus::new();
        let (cpu, _nvic) = crate::system::cortex_m::configure_cortex_m(&mut bus);
        let mut m = Machine::new(cpu, bus);

        const MSP: u32 = 0x2000_1000;
        const RESET_ADDR: u32 = 0x0800_0100;
        m.bus.write_u32(0x0000_0000, MSP).unwrap();
        m.bus.write_u32(0x0000_0004, RESET_ADDR | 1).unwrap();

        const PC: u32 = 0x2000_0000;
        m.bus.write_u16(PC as u64, 0xBF00).unwrap(); // NOP
        m.cpu.set_pc(PC);
        m.cpu.set_sp(0x2000_8000);

        // SYSRESETREQ bit set but no VECTKEY — silicon ignores it.
        m.bus.write_u32(SCB_AIRCR, 1 << 2).unwrap();

        m.step().unwrap();

        assert_ne!(
            m.cpu.get_pc() & !1,
            RESET_ADDR,
            "PC must not jump to the reset vector without the VECTKEY"
        );
        assert_eq!(
            m.cpu.get_register(13),
            0x2000_8000,
            "SP must be untouched without a valid reset request"
        );
    }

    /// The plan used to pin the CPU quantum to 1 for the whole life of any bus
    /// carrying an SCB — i.e. every Cortex-M board — purely so a SYSRESETREQ
    /// could never retire an instruction past its boundary. The quantum is
    /// batched now, and the boundary is held by the latch `configure_cortex_m`
    /// shares with the core (`CortexM::sysreset_signal`): a wide batch stops
    /// dead on the instruction that latched the request.
    ///
    /// Asserted at the CPU level because that is where the guarantee lives —
    /// the machine's run loop keeps going after the reboot, which would only
    /// obscure the one number that matters here.
    #[test]
    fn sysresetreq_cuts_a_wide_cpu_batch_after_one_instruction() {
        use crate::{Cpu, SimulationConfig};

        let mut bus = crate::bus::SystemBus::new();
        let (mut cpu, _nvic) = crate::system::cortex_m::configure_cortex_m(&mut bus);

        const PC: u32 = 0x2000_0000;
        for i in 0..64u64 {
            bus.write_u16(PC as u64 + i * 2, 0xBF00).unwrap(); // NOPs
        }
        cpu.set_pc(PC);
        cpu.set_sp(0x2000_8000);

        let config = SimulationConfig::default();
        assert!(config.batch_mode_enabled, "fixture assumes the batched arm");

        // Baseline: with no request latched, a 64-instruction window is taken
        // whole — the SCB no longer stands in the way of batching.
        let clean = cpu.step_batch(&mut bus, &[], &config, 64).unwrap();
        assert_eq!(clean, 64, "an SCB-equipped core must take the whole window");

        // Now latch SYSRESETREQ through the exact MMIO path firmware uses and
        // re-offer the same wide window.
        cpu.set_pc(PC);
        bus.write_u32(SCB_AIRCR, (0x05FA << 16) | (1 << 2)).unwrap();
        let cut = cpu.step_batch(&mut bus, &[], &config, 64).unwrap();

        assert_eq!(
            cut, 1,
            "a latched SYSRESETREQ must end the batch after one instruction, so \
             the machine boundary drains the reset exactly where a \
             one-instruction quantum would have"
        );
    }

    /// The batch break must not fire on a bus with no reset request: a plain
    /// Cortex-M run has to actually collect its multi-instruction window, which
    /// is the entire point of removing the SCB pin.
    #[test]
    fn no_reset_request_leaves_the_window_batched() {
        use crate::AdvanceRequest;

        let mut bus = crate::bus::SystemBus::new();
        let (cpu, _nvic) = crate::system::cortex_m::configure_cortex_m(&mut bus);
        let mut m = Machine::new(cpu, bus);
        m.config.peripheral_tick_interval = 512;
        m.bus.config.peripheral_tick_interval = 512;

        const PC: u32 = 0x2000_0000;
        for i in 0..64u64 {
            m.bus.write_u16(PC as u64 + i * 2, 0xBF00).unwrap();
        }
        m.cpu.set_pc(PC);
        m.cpu.set_sp(0x2000_8000);

        let report = m.advance(AdvanceRequest::run(Some(64))).unwrap();

        assert_eq!(report.primary_steps, 64, "all 64 NOPs must retire");
        assert!(
            report.cpu_batches < 64,
            "64 NOPs must not cost 64 CPU batches — the SCB pin is gone, so the \
             plan is free to batch (got {} batches)",
            report.cpu_batches
        );
    }
}
