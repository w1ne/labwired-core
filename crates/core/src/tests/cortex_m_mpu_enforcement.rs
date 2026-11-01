// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! Guard: an enabled Armv8-M MPU (`armv8m_mpu`) is enforced. A store to a
//! read-only region and a fetch from an execute-never region raise MemManage
//! (ARMv8-M ARM B3.5.4, ARMv7-M ARM B3.2.15 for the MMFSR bits), or HardFault
//! with HFSR.FORCED when MemManage is not enabled; the PPB and code running at
//! a negative priority with HFNMIENA clear see the default memory map. Zephyr
//! `tests/kernel/mem_protect/protection` on the NUCLEO-L552ZE-Q is the
//! firmware-level proof (examples/nucleo-l552ze-q/zephyr-protection.yaml).

#[cfg(test)]
mod tests {
    use crate::cpu::CortexM;
    use crate::peripherals::armv8m_mpu::Armv8mMpu;
    use crate::{Bus, Cpu, Machine};

    const SHCSR: u64 = 0xE000_ED24;
    const CFSR: u64 = 0xE000_ED28;
    const HFSR: u64 = 0xE000_ED2C;
    const MMFAR: u64 = 0xE000_ED34;
    const MPU_CTRL: u64 = 0xE000_ED94;
    const MPU_RNR: u64 = 0xE000_ED98;
    const MPU_RBAR: u64 = 0xE000_ED9C;
    const MPU_RLAR: u64 = 0xE000_EDA0;

    const SHCSR_MEMFAULTENA: u32 = 1 << 16;
    const IACCVIOL: u32 = 1 << 0;
    const DACCVIOL: u32 = 1 << 1;
    const MMARVALID: u32 = 1 << 7;
    const HFSR_FORCED: u32 = 1 << 30;
    const CTRL_ENABLE: u32 = 1;
    const CTRL_HFNMIENA: u32 = 2;
    const CTRL_PRIVDEFENA: u32 = 4;

    const CODE: u32 = 0x1000;
    const HARDFAULT_HANDLER: u32 = 0x2000;
    const MEMMANAGE_HANDLER: u32 = 0x3000;
    /// Read-only, executable: where the "rodata" a test writes lives.
    const RO_DATA: u32 = 0x0000_8000;
    /// Read-write, execute-never SRAM.
    const XN_RAM: u32 = 0x2000_0400;
    const STACK_TOP: u32 = 0x2000_8000;

    const STR_R0_R1: u16 = 0x6008;
    const LDR_R0_R1: u16 = 0x6808;
    const BLX_R1: u16 = 0x4788;
    const B_SELF: u16 = 0xE7FE;
    const NOP: u16 = 0xBF00;

    fn region(m: &mut Machine<CortexM>, n: u32, base: u32, limit: u32, ap: u32, xn: bool) {
        m.bus.write_u32(MPU_RNR, n).unwrap();
        m.bus
            .write_u32(MPU_RBAR, base | (ap << 1) | u32::from(xn))
            .unwrap();
        m.bus.write_u32(MPU_RLAR, (limit & !0x1F) | 1).unwrap();
    }

    /// A machine with an 8-region PMSAv8 MPU at 0xE000_ED90, HardFault and
    /// MemManage handlers, `insn` at `CODE`, and the Zephyr-style map: flash
    /// 0..0x000F_FFFF RO+X, SRAM RW+XN, background for privileged code.
    fn mpu_machine(insn: u16) -> Machine<CortexM> {
        let mut bus = crate::bus::SystemBus::new();
        bus.add_peripheral("mpu", 0xE000_ED90, 0x38, None, Box::new(Armv8mMpu::new(8)));
        let (cpu, _nvic) = crate::system::cortex_m::configure_cortex_m(&mut bus);
        bus.write_u32(0x0C, HARDFAULT_HANDLER | 1).unwrap(); // exception 3
        bus.write_u32(0x10, MEMMANAGE_HANDLER | 1).unwrap(); // exception 4
        bus.write_u16(HARDFAULT_HANDLER as u64, B_SELF).unwrap();
        bus.write_u16(MEMMANAGE_HANDLER as u64, B_SELF).unwrap();
        bus.write_u16(CODE as u64, insn).unwrap();
        bus.write_u16((CODE + 2) as u64, B_SELF).unwrap();
        bus.write_u16(XN_RAM as u64, NOP).unwrap();
        let mut m = Machine::new(cpu, bus);
        m.cpu.set_faults_enabled(true);
        m.cpu.set_pc(CODE);
        m.cpu.set_sp(STACK_TOP);
        region(&mut m, 0, 0x0000_0000, 0x000F_FFFF, 0b10, false); // RO priv, X
        region(&mut m, 1, 0x2000_0000, 0x2000_FFFF, 0b00, true); // RW priv, XN
        m.bus
            .write_u32(MPU_CTRL, CTRL_ENABLE | CTRL_PRIVDEFENA)
            .unwrap();
        m
    }

    #[test]
    fn store_to_read_only_region_takes_memmanage_with_mmfar() {
        let mut m = mpu_machine(STR_R0_R1);
        m.bus.write_u32(SHCSR, SHCSR_MEMFAULTENA).unwrap();
        m.cpu.set_register(0, 0xDEAD_BEEF);
        m.cpu.set_register(1, RO_DATA);
        m.step().unwrap(); // the STR faults
        assert_eq!(m.cpu.get_pc(), CODE, "a denied store does not retire");
        m.step().unwrap(); // MemManage is taken
        assert_eq!(m.cpu.get_pc(), MEMMANAGE_HANDLER);
        assert_eq!(m.bus.read_u32(CFSR).unwrap(), DACCVIOL | MMARVALID);
        assert_eq!(m.bus.read_u32(MMFAR).unwrap(), RO_DATA);
        assert_eq!(m.bus.read_u32(HFSR).unwrap(), 0);
        assert_ne!(
            m.bus.read_u32(RO_DATA as u64).unwrap(),
            0xDEAD_BEEF,
            "the denied store must not reach memory"
        );
        // The stacked return address is the faulting STR (B1.5.6).
        let sp = m.cpu.get_register(13);
        assert_eq!(m.bus.read_u32(u64::from(sp) + 24).unwrap(), CODE);
    }

    #[test]
    fn load_from_read_only_region_is_allowed() {
        let mut m = mpu_machine(LDR_R0_R1);
        m.bus.write_u32(SHCSR, SHCSR_MEMFAULTENA).unwrap();
        m.cpu.set_register(1, RO_DATA);
        m.step().unwrap();
        m.step().unwrap();
        assert_eq!(m.cpu.get_pc(), CODE + 2);
        assert_eq!(m.bus.read_u32(CFSR).unwrap(), 0);
    }

    #[test]
    fn fetch_from_execute_never_region_takes_memmanage_without_mmfar() {
        let mut m = mpu_machine(BLX_R1);
        m.bus.write_u32(SHCSR, SHCSR_MEMFAULTENA).unwrap();
        m.cpu.set_register(1, XN_RAM | 1);
        m.step().unwrap(); // BLX: pc -> XN_RAM
        assert_eq!(m.cpu.get_pc(), XN_RAM);
        m.step().unwrap(); // the fetch faults
        assert_eq!(m.cpu.get_pc(), XN_RAM);
        m.step().unwrap();
        assert_eq!(m.cpu.get_pc(), MEMMANAGE_HANDLER);
        assert_eq!(m.bus.read_u32(CFSR).unwrap(), IACCVIOL);
        assert_eq!(m.bus.read_u32(MMFAR).unwrap(), 0);
    }

    #[test]
    fn memmanage_disabled_escalates_to_hardfault_forced() {
        let mut m = mpu_machine(STR_R0_R1);
        m.cpu.set_register(1, RO_DATA);
        m.step().unwrap();
        m.step().unwrap();
        assert_eq!(m.cpu.get_pc(), HARDFAULT_HANDLER);
        assert_eq!(m.bus.read_u32(HFSR).unwrap(), HFSR_FORCED);
        assert_eq!(m.bus.read_u32(CFSR).unwrap(), DACCVIOL | MMARVALID);
    }

    /// With HFNMIENA clear the MPU is off at a negative execution priority:
    /// the HardFault handler may write what the MPU denies elsewhere. With it
    /// set the same store faults, which is LOCKUP from HardFault.
    #[test]
    fn hardfault_handler_bypasses_the_mpu_unless_hfnmiena() {
        let mut m = mpu_machine(STR_R0_R1);
        m.cpu.set_register(1, RO_DATA);
        m.cpu.set_register(0, 0x1234_5678);
        m.cpu.faultmask = true; // execution priority -1, like HardFault
        m.step().unwrap();
        assert_eq!(m.cpu.get_pc(), CODE + 2);
        assert_eq!(m.bus.read_u32(RO_DATA as u64).unwrap(), 0x1234_5678);
        assert_eq!(m.bus.read_u32(CFSR).unwrap(), 0);

        let mut m = mpu_machine(STR_R0_R1);
        m.bus
            .write_u32(MPU_CTRL, CTRL_ENABLE | CTRL_PRIVDEFENA | CTRL_HFNMIENA)
            .unwrap();
        m.cpu.set_register(1, RO_DATA);
        m.cpu.faultmask = true;
        assert!(m.step().is_err(), "MemManage from priority -1 is LOCKUP");
    }

    /// Unprivileged thread code gets no background region: an address in no
    /// region faults, and a privileged-only region denies it.
    #[test]
    fn unprivileged_code_gets_no_background_region() {
        let mut m = mpu_machine(LDR_R0_R1);
        m.bus.write_u32(SHCSR, SHCSR_MEMFAULTENA).unwrap();
        // Code readable by everyone, so only the data access is in question.
        region(&mut m, 0, 0x0000_0000, 0x000F_FFFF, 0b11, false);
        m.cpu.control |= 1; // nPRIV
        m.cpu.set_register(1, 0x6000_0000); // in no region
        m.step().unwrap();
        m.step().unwrap();
        assert_eq!(m.cpu.get_pc(), MEMMANAGE_HANDLER);
        assert_eq!(m.bus.read_u32(MMFAR).unwrap(), 0x6000_0000);

        let mut m = mpu_machine(LDR_R0_R1);
        m.bus.write_u32(SHCSR, SHCSR_MEMFAULTENA).unwrap();
        region(&mut m, 0, 0x0000_0000, 0x000F_FFFF, 0b11, false);
        m.cpu.control |= 1;
        m.cpu.set_register(1, XN_RAM); // privileged-only RW
        m.step().unwrap();
        m.step().unwrap();
        assert_eq!(m.cpu.get_pc(), MEMMANAGE_HANDLER);
    }

    /// A region change after the core cached a decision takes effect on the
    /// next access: an RBAR/RLAR write alone (no CTRL write) must invalidate
    /// the cached span through the MPU generation.
    #[test]
    fn reprogramming_a_region_invalidates_the_cached_decision() {
        let mut m = mpu_machine(STR_R0_R1);
        m.bus.write_u32(SHCSR, SHCSR_MEMFAULTENA).unwrap();
        m.cpu.set_register(1, XN_RAM);
        m.step().unwrap();
        assert_eq!(m.cpu.get_pc(), CODE + 2, "privileged RW region: allowed");
        // Same region, now read-only.
        region(&mut m, 1, 0x2000_0000, 0x2000_FFFF, 0b10, true);
        m.cpu.set_pc(CODE);
        m.step().unwrap();
        m.step().unwrap();
        assert_eq!(m.cpu.get_pc(), MEMMANAGE_HANDLER);
        assert_eq!(m.bus.read_u32(MMFAR).unwrap(), XN_RAM);
    }

    /// Unprivileged code touching the PPB (here MPU_CTRL, what Zephyr
    /// userspace `test_disable_mmu_mpu` does) is a precise BusFault, not
    /// MemManage, and the write does not land.
    #[test]
    fn unprivileged_ppb_access_is_a_busfault() {
        const SHCSR_BUSFAULTENA: u32 = 1 << 17;
        const PRECISERR_BFARVALID: u32 = (1 << 9) | (1 << 15);
        let mut m = mpu_machine(STR_R0_R1);
        m.bus
            .write_u32(SHCSR, SHCSR_MEMFAULTENA | SHCSR_BUSFAULTENA)
            .unwrap();
        m.bus.write_u32(0x14, MEMMANAGE_HANDLER | 1).unwrap(); // BusFault -> same B .
        region(&mut m, 0, 0x0000_0000, 0x000F_FFFF, 0b11, false);
        m.cpu.control |= 1;
        m.cpu.set_register(0, 0);
        m.cpu.set_register(1, MPU_CTRL as u32);
        m.step().unwrap();
        m.step().unwrap();
        assert_eq!(m.cpu.get_pc(), MEMMANAGE_HANDLER);
        assert_eq!(m.bus.read_u32(CFSR).unwrap(), PRECISERR_BFARVALID);
        assert_eq!(
            m.bus.read_u32(MPU_CTRL).unwrap(),
            CTRL_ENABLE | CTRL_PRIVDEFENA,
            "the MPU stays on"
        );
    }

    /// MSR from unprivileged Thread mode cannot clear CONTROL.nPRIV or set
    /// the masks (Zephyr userspace `test_write_control`).
    #[test]
    fn unprivileged_msr_is_ignored() {
        // MSR CONTROL, r0 = F380 8814; MSR PRIMASK, r0 = F380 8810.
        let mut m = mpu_machine(0xF380);
        m.bus.write_u16((CODE + 2) as u64, 0x8814).unwrap();
        m.bus.write_u16((CODE + 4) as u64, 0xF380).unwrap();
        m.bus.write_u16((CODE + 6) as u64, 0x8810).unwrap();
        m.bus.write_u16((CODE + 8) as u64, B_SELF).unwrap();
        region(&mut m, 0, 0x0000_0000, 0x000F_FFFF, 0b11, false);
        m.cpu.control = 1;
        m.cpu.set_register(0, 1 << 1); // nPRIV clear, SPSEL set
        m.step().unwrap();
        assert_eq!(m.cpu.control, 1, "CONTROL write ignored");
        m.cpu.set_register(0, 1);
        m.step().unwrap();
        assert!(!m.cpu.primask, "PRIMASK write ignored");
        assert_eq!(m.cpu.get_pc(), CODE + 8);
    }

    /// The PPB is outside the MPU: with the background region off, privileged
    /// code still reaches the SCB.
    #[test]
    fn ppb_is_always_default_map() {
        let mut m = mpu_machine(LDR_R0_R1);
        m.bus.write_u32(MPU_CTRL, CTRL_ENABLE).unwrap();
        m.cpu.set_register(1, SHCSR as u32);
        m.step().unwrap();
        assert_eq!(m.cpu.get_pc(), CODE + 2);
    }

    /// MPU disabled: nothing is enforced except the default map's XN.
    #[test]
    fn disabled_mpu_enforces_nothing() {
        let mut m = mpu_machine(STR_R0_R1);
        m.bus.write_u32(MPU_CTRL, 0).unwrap();
        m.cpu.set_register(1, RO_DATA);
        m.step().unwrap();
        assert_eq!(m.cpu.get_pc(), CODE + 2);
        assert_eq!(m.bus.read_u32(CFSR).unwrap(), 0);
    }
    /// MPU disabled: the default map's execute-never blocks still fault an
    /// instruction fetch (Peripheral, 0x4000_0000), and SRAM still executes.
    #[test]
    fn disabled_mpu_keeps_the_default_map_xn() {
        let mut m = mpu_machine(BLX_R1);
        m.bus.write_u32(MPU_CTRL, 0).unwrap();
        m.bus.write_u32(SHCSR, SHCSR_MEMFAULTENA).unwrap();
        m.cpu.set_register(1, 0x4000_0000 | 1);
        m.step().unwrap(); // BLX: pc -> the Peripheral block
        m.step().unwrap(); // the fetch faults
        m.step().unwrap();
        assert_eq!(m.cpu.get_pc(), MEMMANAGE_HANDLER);
        assert_eq!(m.bus.read_u32(CFSR).unwrap(), IACCVIOL);

        // SRAM (block 1) is executable in the default map: the XN region
        // programmed in `mpu_machine` no longer applies.
        let mut m = mpu_machine(BLX_R1);
        m.bus.write_u32(MPU_CTRL, 0).unwrap();
        m.cpu.set_register(1, XN_RAM | 1);
        m.step().unwrap();
        m.step().unwrap(); // NOP at XN_RAM
        assert_eq!(m.cpu.get_pc(), XN_RAM + 2);
        assert_eq!(m.bus.read_u32(CFSR).unwrap(), 0);
    }

    /// MPU disabled: unprivileged code still may not touch the PPB, and may
    /// read and write everything else.
    #[test]
    fn disabled_mpu_keeps_the_unprivileged_ppb_rule() {
        const SHCSR_BUSFAULTENA: u32 = 1 << 17;
        const PRECISERR_BFARVALID: u32 = (1 << 9) | (1 << 15);
        let mut m = mpu_machine(STR_R0_R1);
        m.bus.write_u32(MPU_CTRL, 0).unwrap();
        m.bus
            .write_u32(SHCSR, SHCSR_MEMFAULTENA | SHCSR_BUSFAULTENA)
            .unwrap();
        m.bus.write_u32(0x14, MEMMANAGE_HANDLER | 1).unwrap(); // BusFault -> same B .
        m.cpu.control |= 1;
        m.cpu.set_register(0, CTRL_ENABLE);
        m.cpu.set_register(1, MPU_CTRL as u32);
        m.step().unwrap();
        m.step().unwrap();
        assert_eq!(m.cpu.get_pc(), MEMMANAGE_HANDLER);
        assert_eq!(m.bus.read_u32(CFSR).unwrap(), PRECISERR_BFARVALID);
        assert_eq!(
            m.bus.read_u32(MPU_CTRL).unwrap(),
            0,
            "the write did not land"
        );

        let mut m = mpu_machine(STR_R0_R1);
        m.bus.write_u32(MPU_CTRL, 0).unwrap();
        m.cpu.control |= 1;
        m.cpu.set_register(0, 0x55);
        m.cpu.set_register(1, XN_RAM + 0x100);
        m.step().unwrap();
        assert_eq!(m.cpu.get_pc(), CODE + 2);
        assert_eq!(m.bus.read_u32(u64::from(XN_RAM + 0x100)).unwrap(), 0x55);
    }
}
