// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! Guard: `WAITI` must **retire** before it waits, so the interrupt that ends
//! the wait resumes at the instruction AFTER it.
//!
//! Xtensa ISA RM, WAITI: `PS.INTLEVEL ← level`, then the core suspends. The
//! wait state sits *between* WAITI and its successor, so `EPC[level]` latches
//! the address of the following instruction and `RFI`/`RFE` resumes there.
//!
//! The model used to park *on* the WAITI (PC deliberately not advanced, so a
//! poll loop would keep seeing the same PC). That looks harmless — the core
//! still takes its interrupts — but it makes every wake re-enter the wait:
//! `dispatch_irq` latched `EPC1` = the WAITI's own address, the handler ran,
//! and the return dropped the core straight back into the wait. Code that has
//! to make forward progress *after* a wake therefore never did.
//!
//! That is exactly ESP-IDF's SMP bring-up. Core 1's FreeRTOS idle task calls
//! `esp_cpu_wait_for_intr()` from `esp_vApplicationIdleHook()`; the registered
//! idle hooks — including the one that sets `s_other_cpu_startup_done` — only
//! run on the NEXT loop iteration, i.e. after that call returns. With the PC
//! pinned, an ESP32-S3 dual-core image took hundreds of systimer ticks on core
//! 1 and still never returned from the call, so core 0 spun forever in
//! `main_task`'s `while (!s_other_cpu_startup_done)` and `app_main` never ran.
//!
//! Counting wakes is NOT enough to catch this: the old model woke on every
//! tick and dispatched a real handler. What distinguishes the two is where
//! control lands afterwards, so the assertions below are about `EPC1` and about
//! the instruction after the WAITI actually executing.

#[cfg(test)]
mod tests {
    use crate::bus::SystemBus;
    use crate::cpu::xtensa_lx7::XtensaLx7;
    use crate::cpu::xtensa_sr::{EPC1, INTENABLE};
    use crate::system::xtensa::{configure_xtensa_esp32s3, Esp32s3Opts};
    use crate::{Bus, Cpu, SimulationConfig};

    /// ESP32-S3 IRAM — instruction fetch and the test's own stores both land.
    const CODE: u32 = 0x4037_0000;

    /// `waiti 0` (RRR, 3 bytes) followed by `movi.n a2, 1` (2 bytes). The
    /// `movi.n` is the "did the core make forward progress" witness: it can
    /// only run if execution resumed *after* the WAITI.
    const WAITI_0: [u8; 3] = [0x00, 0x70, 0x00];
    const MOVI_N_A2_1: [u8; 2] = [0x0C, 0x12];

    /// Bit 6 of INTERRUPT is the internal timer-0 source (`IRQ_LEVELS[6] == 1`),
    /// the level-1 line FreeRTOS-on-Xtensa uses for its tick.
    const TIMER0_BIT: u32 = 1 << 6;

    fn machine() -> (SystemBus, XtensaLx7) {
        let mut bus = SystemBus::new();
        let wiring = configure_xtensa_esp32s3(&mut bus, &Esp32s3Opts::default());
        let mut cpu = wiring.cpu;
        for (i, b) in WAITI_0.iter().enumerate() {
            bus.write_u8(CODE as u64 + i as u64, *b).unwrap();
        }
        for (i, b) in MOVI_N_A2_1.iter().enumerate() {
            bus.write_u8(CODE as u64 + WAITI_0.len() as u64 + i as u64, *b)
                .unwrap();
        }
        cpu.set_pc(CODE);
        // Reset leaves PS.EXCM set; an interrupt can only dispatch with it
        // clear, which is the state the idle task actually runs in.
        cpu.ps.set_excm(false);
        cpu.regs.write_logical(2, 0);
        (bus, cpu)
    }

    fn step(cpu: &mut XtensaLx7, bus: &mut SystemBus) {
        cpu.step(bus, &[], &SimulationConfig::default()).unwrap();
    }

    #[test]
    fn waiti_retires_and_parks_at_the_next_instruction() {
        let (mut bus, mut cpu) = machine();
        step(&mut cpu, &mut bus);
        assert_eq!(
            cpu.get_pc(),
            CODE + WAITI_0.len() as u32,
            "WAITI must retire: the PC belongs to the instruction after it, \
             not to the WAITI itself"
        );
        assert_eq!(cpu.ps.intlevel(), 0, "WAITI 0 sets PS.INTLEVEL = 0");
        assert!(cpu.is_parked_idle(), "the core is in the wait state");
        // The wait really waits: with nothing pending, the successor must NOT
        // execute, however many cycles pass.
        for _ in 0..64 {
            step(&mut cpu, &mut bus);
        }
        assert_eq!(
            cpu.regs.read_logical(2),
            0,
            "the instruction after WAITI must not run until an interrupt arrives"
        );
        assert!(cpu.is_parked_idle(), "still waiting");
    }

    #[test]
    fn the_wake_interrupt_returns_past_the_waiti() {
        let (mut bus, mut cpu) = machine();
        step(&mut cpu, &mut bus); // retire WAITI, enter the wait
        assert!(cpu.is_parked_idle());

        cpu.sr.write(INTENABLE, TIMER0_BIT);
        cpu.sr.raise_interrupt_bits(TIMER0_BIT);
        step(&mut cpu, &mut bus); // dispatch

        assert!(!cpu.is_parked_idle(), "the interrupt ends the wait");
        assert_eq!(
            cpu.sr.read(EPC1),
            CODE + WAITI_0.len() as u32,
            "the level-1 return address must be the instruction AFTER the WAITI \
             — returning to the WAITI itself re-enters the wait forever"
        );

        // Model the handler returning (RFE: PC ← EPC1, PS.EXCM ← 0) and check
        // the core makes forward progress instead of falling back into the wait.
        let ret = cpu.sr.read(EPC1);
        cpu.set_pc(ret);
        cpu.ps.set_excm(false);
        cpu.sr.write(INTENABLE, 0);
        step(&mut cpu, &mut bus);
        assert_eq!(
            cpu.regs.read_logical(2),
            1,
            "after the wake the core must execute the instruction following the WAITI"
        );
        assert!(
            !cpu.is_parked_idle(),
            "and must not be back in the wait state"
        );
    }
}
