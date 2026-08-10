// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! Guard: the Xtensa **windowed register-overflow spill** must honour the bus
//! memory contract.
//!
//! `bus/accessors.rs` returns `Err(SimulationError::MemoryViolation(addr))` for
//! any address no memory region or peripheral window covers.
//! `XtensaLx7::spill_call_preserve_to_stack` writes the OF save areas
//! (`a0..a3` at `callee_sp - 16`, `a4..a7` at `parent_sp - 32`) through a
//! `write4` closure that used four consecutive `let _ = bus.write_u32(..)`.
//! If the save area was unmapped the spill **silently vanished** and the run
//! continued with registers that were never saved — a later WindowUnderflow
//! then reloads whatever happened to be at those addresses.
//!
//! This is the same defect `cortex_m_memory_contract.rs` pins for Cortex-M; the
//! four Xtensa sites were counted in that module's shrink-only allowlist rather
//! than fixed at the time.
//!
//! **How this fixture avoids the trap #880 hit.** `examples/ci/dummy-memory-violation.yaml`
//! reaches `memory_violation` through the *instruction-fetch* path and retires
//! **0 instructions**, so it proves nothing about any data access. Every test
//! below keeps the code in mapped IRAM and *executes real instructions*
//! (`CALL8` then `ENTRY a1, 4`), asserts they retired, and only then triggers
//! the spill. The only address that can fault is the spill save area.
//!
//! Both directions are covered:
//!   * an unmapped save area must surface the violation, and
//!   * a spill into **mapped** DRAM must still succeed, leave the run alive,
//!     and leave the save area holding exactly the values a WindowUnderflow
//!     would reload into `a0..a3`.
//!
//! A change that aborted on every spill would pass a one-directional test.

#[cfg(test)]
mod tests {
    use crate::bus::SystemBus;
    use crate::cpu::xtensa_lx7::XtensaLx7;
    use crate::cpu::xtensa_sr::{INTENABLE, INTERRUPT};
    use crate::peripherals::esp_xtensa_common::rom_thunks::xthal_window_spill_thunk;
    use crate::system::xtensa::configure_xtensa_esp32;
    use crate::{Bus, Cpu, SimulationConfig, SimulationError};

    /// Classic ESP32 IRAM (`configs/chips/esp32.yaml`: 0x4008_0000, 128 KiB).
    /// Code lives here so instruction fetch never faults.
    const CODE: u32 = 0x4008_0000;

    /// Inside the modelled `dram` window (`system::xtensa::esp32`:
    /// 0x3FFA_E000, 200 KiB → ..0x3FFE_0000). A spill from this SP lands at
    /// `SP-48`, comfortably mapped.
    const MAPPED_SP: u32 = 0x3FFB_8000;

    /// Inside the **architectural** classic-ESP32 DRAM window
    /// (0x3FF8_0000..0x4000_0000) that `spill_call_preserve_to_stack::valid_sp`
    /// accepts, but in the one hole the model leaves inside it:
    ///
    /// | window          | span                      |
    /// |-----------------|---------------------------|
    /// | `rtc_fast`      | 0x3FF8_0000..0x3FF8_2000  |
    /// | **unmapped**    | 0x3FF8_2000..0x3FF9_0000  |
    /// | `brom_data`     | 0x3FF9_0000..0x3FFA_0000  |
    /// | `brom_low_data` | 0x3FFA_0000..0x3FFA_E000  |
    /// | `dram`          | 0x3FFA_E000..0x3FFE_0000  |
    /// | `sram1`         | 0x3FFE_0000..0x4000_0000  |
    ///
    /// This is exactly the shape that made the spill vanish: the SP looks like
    /// a real stack pointer to every guard in the spill path, and the store
    /// still has nowhere to land.
    const UNMAPPED_SP: u32 = 0x3FF8_8000;

    /// The save area `write4` targets for the CALL8 frame built below:
    /// `callee_sp (= SP-32) - 16`.
    fn save_area(sp: u32) -> u32 {
        sp - 48
    }

    // Caller-frame register pattern. `a1` is the SP and is set separately.
    const A0: u32 = 0xA000_0000;
    const A2: u32 = 0xA222_2222;
    const A3: u32 = 0xA333_3333;

    /// Encode `CALL8 target` (op0=0x5, n=2). Same formula as
    /// `tests/xtensa_exec.rs::enc_call`, HW-oracle verified there.
    fn enc_call8(pc: u32, target: u32) -> u32 {
        let base = (pc.wrapping_add(4)) & !3u32;
        let imm18 = (((target.wrapping_sub(base) as i32) / 4) as u32) & 0x3_FFFF;
        0x5 | (2 << 4) | (imm18 << 6)
    }

    /// Encode `ENTRY as_, imm12` (op0=6, n=3, m=0). Stack decrement = imm12 * 8.
    fn enc_entry(as_: u32, imm12: u32) -> u32 {
        0x6 | (3 << 4) | (as_ << 8) | ((imm12 & 0xFFF) << 12)
    }

    fn write_insn(bus: &mut SystemBus, addr: u32, word: u32) {
        for i in 0..3u32 {
            bus.write_u8((addr + i) as u64, ((word >> (8 * i)) & 0xFF) as u8)
                .expect("instruction memory must be mapped — otherwise this is a fetch test");
        }
    }

    fn cfg() -> SimulationConfig {
        SimulationConfig::default()
    }

    /// Build a classic-ESP32 CPU parked immediately after
    /// `CALL8 <sub>` / `ENTRY a1, 4`, with the caller's `a1` = `sp`.
    ///
    /// That leaves exactly one entry on `call_preserve_stack` holding the
    /// caller's `a0..a7`, which is what the spill has to write out.
    ///
    /// Returns `(cpu, bus, instructions_retired)`.
    fn windowed_frame(sp: u32) -> (XtensaLx7, SystemBus, u32) {
        let mut bus = SystemBus::empty();
        let mut cpu = configure_xtensa_esp32(&mut bus);
        cpu.reset(&mut bus).expect("reset");

        // A normal running task: windows on, not in exception mode, all levels
        // unmasked so a level-1 IRQ can be taken.
        cpu.ps.set_woe(true);
        cpu.ps.set_excm(false);
        cpu.ps.set_intlevel(0);

        write_insn(&mut bus, CODE, enc_call8(CODE, CODE + 0x10));
        write_insn(&mut bus, CODE + 0x10, enc_entry(1, 4)); // 32-byte frame

        cpu.set_pc(CODE);
        cpu.regs.write_logical(0, A0);
        cpu.regs.write_logical(1, sp);
        cpu.regs.write_logical(2, A2);
        cpu.regs.write_logical(3, A3);

        let mut retired = 0;
        for i in 0..2 {
            cpu.step(&mut bus, &[], &cfg())
                .unwrap_or_else(|e| panic!("setup instruction {i} must retire, got {e:?}"));
            retired += 1;
        }

        assert_eq!(
            cpu.pc,
            CODE + 0x13,
            "PC must be just past the ENTRY — the setup really executed"
        );
        assert_eq!(
            cpu.regs.read_logical(1),
            sp.wrapping_sub(32),
            "ENTRY a1,4 must have opened a 32-byte callee frame"
        );
        (cpu, bus, retired)
    }

    /// Raise a level-1 interrupt and take one step. That is the production
    /// route into the spill: `dispatch_irq` → `spill_call_preserve_to_stack`.
    fn take_irq(cpu: &mut XtensaLx7, bus: &mut SystemBus) -> Result<(), SimulationError> {
        cpu.sr.set_raw(INTERRUPT, 1 << 0);
        cpu.sr.write(INTENABLE, 1 << 0);
        cpu.step(bus, &[], &cfg())
    }

    /// ANTI-VACUITY. If `UNMAPPED_SP`'s save area were mapped after all, every
    /// "must fault" test below would be asserting nothing.
    #[test]
    fn the_unmapped_save_area_really_is_unmapped_and_the_mapped_one_really_is_mapped() {
        let mut bus = SystemBus::empty();
        let _cpu = configure_xtensa_esp32(&mut bus);
        for off in [0u32, 4, 8, 12] {
            let a = save_area(UNMAPPED_SP) + off;
            assert!(
                bus.write_u32(a as u64, 0).is_err(),
                "0x{a:08X} must be unmapped on the classic-ESP32 model, or the \
                 unmapped-spill tests are vacuous"
            );
        }
        for off in [0u32, 4, 8, 12] {
            let a = save_area(MAPPED_SP) + off;
            bus.write_u32(a as u64, 0)
                .unwrap_or_else(|e| panic!("0x{a:08X} must be mapped DRAM: {e:?}"));
        }
    }

    /// The headline contract, via the interrupt path (`dispatch_irq`).
    #[test]
    fn window_spill_into_unmapped_memory_surfaces_the_violation() {
        let (mut cpu, mut bus, retired) = windowed_frame(UNMAPPED_SP);
        assert_eq!(retired, 2, "the fixture must actually retire instructions");

        let step = take_irq(&mut cpu, &mut bus);

        let lo = save_area(UNMAPPED_SP) as u64;
        assert!(
            matches!(step, Err(SimulationError::MemoryViolation(a)) if (lo..lo + 16).contains(&a)),
            "the windowed-overflow spill wrote a0..a3 into unmapped \
             0x{lo:08X}..0x{:08X} and the run continued as if the registers had \
             been saved. It must surface the bus MemoryViolation. got {step:?}",
            lo + 16
        );
    }

    /// The same contract via the other caller: the `xthal_window_spill` ROM
    /// thunk, which firmware reaches on an explicit FreeRTOS solicited yield.
    #[test]
    fn xthal_window_spill_thunk_surfaces_the_violation() {
        let (mut cpu, mut bus, retired) = windowed_frame(UNMAPPED_SP);
        assert_eq!(retired, 2, "the fixture must actually retire instructions");

        let r = xthal_window_spill_thunk(&mut cpu, &mut bus);

        let lo = save_area(UNMAPPED_SP) as u64;
        assert!(
            matches!(r, Err(SimulationError::MemoryViolation(a)) if (lo..lo + 16).contains(&a)),
            "xthal_window_spill's semantic spill dropped its stores into \
             unmapped 0x{lo:08X} and returned Ok. got {r:?}"
        );
    }

    /// The other direction: a spill into mapped DRAM must still succeed, and
    /// the run must stay alive. A fix that aborted on every spill would fail
    /// here.
    #[test]
    fn window_spill_into_mapped_memory_still_succeeds_and_keeps_the_run_alive() {
        let (mut cpu, mut bus, _) = windowed_frame(MAPPED_SP);

        take_irq(&mut cpu, &mut bus).expect("a spill into mapped DRAM must not abort the run");

        // The IRQ was actually taken: PC is at the level-1 user exception
        // vector (VECBASE + 0x340, PS.UM=0 here → +0x300).
        assert_ne!(
            cpu.pc,
            CODE + 0x13,
            "the level-1 IRQ must have been dispatched"
        );
    }

    /// …and the spill must have *landed*: the OF save area holds exactly the
    /// caller's `a0..a3`, which is what `_WindowUnderflow8`'s
    /// `l32e a0, a1, -16 …` reloads. This is the "restored registers hold the
    /// spilled values" half — without it, "the run stayed alive" would be
    /// satisfied by a spill that wrote nothing at all.
    #[test]
    fn a_mapped_window_spill_leaves_the_save_area_reloadable() {
        let (mut cpu, mut bus, _) = windowed_frame(MAPPED_SP);
        let base = save_area(MAPPED_SP) as u64;

        // Poison the save area first, so reading back the right values cannot
        // be an accident of zero-initialised DRAM.
        for off in [0u64, 4, 8, 12] {
            bus.write_u32(base + off, 0xDEAD_BEEF).unwrap();
        }

        take_irq(&mut cpu, &mut bus).expect("mapped spill must succeed");

        // What WindowUnderflow8 would reload from `callee_sp - 16`.
        let reloaded = [
            bus.read_u32(base).unwrap(),
            bus.read_u32(base + 4).unwrap(),
            bus.read_u32(base + 8).unwrap(),
            bus.read_u32(base + 12).unwrap(),
        ];
        assert_eq!(
            reloaded,
            [A0, MAPPED_SP, A2, A3],
            "the OF save area at 0x{base:08X} must hold the caller's a0..a3 \
             (a1 = SP) so a WindowUnderflow reload restores the real values"
        );
    }
}
