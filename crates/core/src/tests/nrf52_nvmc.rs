// LabWired - Firmware Simulation Platform
// SPDX-License-Identifier: MIT
//
// nRF52 NVMC fidelity: write-enable gating + 1→0 program semantics on the
// flash-region write path, and boundary-drained erase ops (page / all /
// UICR).

#[cfg(test)]
mod nrf52_nvmc_tests {
    use crate::memory::LinearMemory;
    use crate::peripherals::nrf52::nvmc::Nrf52Nvmc;
    use crate::peripherals::nrf52::uicr::Nrf52Uicr;
    use crate::{Bus, Cpu, Machine};

    const NVMC_BASE: u64 = 0x4001_E000;
    const UICR_BASE: u64 = 0x1000_1000;
    const OFF_READY: u64 = 0x400;
    const OFF_CONFIG: u64 = 0x504;
    const OFF_ERASEPAGE: u64 = 0x508;
    const OFF_ERASEALL: u64 = 0x50C;
    const OFF_ERASEUICR: u64 = 0x514;
    const WEN: u32 = 1;
    const EEN: u32 = 2;

    fn nrf52_bus() -> crate::bus::SystemBus {
        nrf52_bus_with_lead_padding(0)
    }

    /// The same nRF52 bus, but with `pad` inert stub peripherals pushed AHEAD
    /// of the NVMC so it does not land at index 0.
    ///
    /// The boundary erase drain resolves the NVMC through a bus index cached
    /// once in `Machine::new`. Every other test here happens to push the NVMC
    /// first, so they would still pass against a cache that always answered
    /// "index 0". This shape is what discriminates a correctly resolved index
    /// from a lucky one.
    fn nrf52_bus_with_lead_padding(pad: usize) -> crate::bus::SystemBus {
        let mut bus = crate::bus::SystemBus::empty();
        bus.flash = LinearMemory::new_erased(0x10000, 0x0);
        bus.ram = LinearMemory::new(0x1000, 0x2000_0000);
        for i in 0..pad {
            bus.peripherals.push(crate::bus::PeripheralEntry {
                name: format!("pad{i}"),
                base: 0x4002_0000 + (i as u64) * 0x1000,
                size: 0x1000,
                irq: None,
                dev: Box::new(crate::peripherals::stub::StubPeripheral::new(0)),
                ticks_remaining: 0,
                clock_gate: None,
            });
        }
        bus.peripherals.push(crate::bus::PeripheralEntry {
            name: "nvmc".to_string(),
            base: NVMC_BASE,
            size: 0x1000,
            irq: None,
            dev: Box::new(Nrf52Nvmc::new()),
            ticks_remaining: 0,
            clock_gate: None,
        });
        bus.peripherals.push(crate::bus::PeripheralEntry {
            name: "uicr".to_string(),
            base: UICR_BASE,
            size: 0x1000,
            irq: None,
            dev: Box::new(Nrf52Uicr::new()),
            ticks_remaining: 0,
            clock_gate: None,
        });
        bus.rebuild_peripheral_ranges();
        bus
    }

    fn machine_with_nvmc() -> Machine<crate::cpu::CortexM> {
        machine_with_nvmc_at_offset(0)
    }

    fn machine_with_nvmc_at_offset(pad: usize) -> Machine<crate::cpu::CortexM> {
        let mut bus = nrf52_bus_with_lead_padding(pad);
        let (cpu, _nvic) = crate::system::cortex_m::configure_cortex_m(&mut bus);
        Machine::new(cpu, bus)
    }

    /// One NOP step so the machine boundary drains the latched erase op.
    fn step_once(m: &mut Machine<crate::cpu::CortexM>) {
        m.bus.write_u16(0x2000_0000, 0xBF00).unwrap(); // NOP in RAM
        m.cpu.set_pc(0x2000_0000);
        m.cpu.set_sp(0x2000_0800);
        m.step().unwrap();
    }

    #[test]
    fn flash_write_dropped_without_wen() {
        let mut bus = nrf52_bus();
        bus.write_u8(0x100, 0x42).unwrap();
        assert_eq!(
            bus.read_u8(0x100).unwrap(),
            0xFF,
            "program store without CONFIG.Wen must be dropped (silicon ignores it)"
        );
    }

    #[test]
    fn flash_write_with_wen_commits_and_semantics() {
        let mut bus = nrf52_bus();
        bus.write_u32(NVMC_BASE + OFF_CONFIG, WEN).unwrap();
        bus.write_u8(0x100, 0xF0).unwrap();
        assert_eq!(bus.read_u8(0x100).unwrap(), 0xF0, "Wen set ⇒ store commits");
        // Second store can only clear bits (1→0), never set them back.
        bus.write_u8(0x100, 0x0F).unwrap();
        assert_eq!(
            bus.read_u8(0x100).unwrap(),
            0x00,
            "flash bits only flip 1→0: 0xF0 & 0x0F = 0x00"
        );
        bus.write_u8(0x100, 0xFF).unwrap();
        assert_eq!(
            bus.read_u8(0x100).unwrap(),
            0x00,
            "1→0 transitions are lost"
        );
    }

    #[test]
    fn erasepage_blanks_4kib_page_at_boundary() {
        let mut m = machine_with_nvmc();
        // Program two bytes in different pages first.
        m.bus.write_u32(NVMC_BASE + OFF_CONFIG, WEN).unwrap();
        m.bus.write_u8(0x0800, 0x11).unwrap();
        m.bus.write_u8(0x1800, 0x22).unwrap();
        assert_eq!(m.bus.read_u8(0x0800).unwrap(), 0x11);

        m.bus.write_u32(NVMC_BASE + OFF_CONFIG, EEN).unwrap();
        m.bus.write_u32(NVMC_BASE + OFF_ERASEPAGE, 0x0800).unwrap();
        step_once(&mut m);

        assert_eq!(m.bus.read_u8(0x0800).unwrap(), 0xFF, "page blanked");
        assert_eq!(m.bus.read_u8(0x1800).unwrap(), 0x22, "other page untouched");
    }

    #[test]
    fn eraseall_blanks_entire_flash() {
        let mut m = machine_with_nvmc();
        m.bus.write_u32(NVMC_BASE + OFF_CONFIG, WEN).unwrap();
        m.bus.write_u8(0x0800, 0x11).unwrap();
        m.bus.write_u8(0xF000, 0x22).unwrap();

        m.bus.write_u32(NVMC_BASE + OFF_CONFIG, EEN).unwrap();
        m.bus.write_u32(NVMC_BASE + OFF_ERASEALL, 1).unwrap();
        step_once(&mut m);

        assert_eq!(m.bus.read_u8(0x0800).unwrap(), 0xFF);
        assert_eq!(m.bus.read_u8(0xF000).unwrap(), 0xFF);
    }

    #[test]
    fn eraseuicr_resets_uicr_to_erased() {
        let mut m = machine_with_nvmc();
        // Provision a UICR customer word (UICR writes AND directly, no NVMC
        // gating in the model — matches nrf52_uicr's own contract).
        m.bus.write_u32(UICR_BASE + 0x080, 0x1234_5678).unwrap();
        assert_eq!(m.bus.read_u32(UICR_BASE + 0x080).unwrap(), 0x1234_5678);

        m.bus.write_u32(NVMC_BASE + OFF_CONFIG, EEN).unwrap();
        m.bus.write_u32(NVMC_BASE + OFF_ERASEUICR, 1).unwrap();
        step_once(&mut m);

        assert_eq!(
            m.bus.read_u32(UICR_BASE + 0x080).unwrap(),
            0xFFFF_FFFF,
            "ERASEUICR restores the erased state"
        );
    }

    #[test]
    fn erase_without_een_is_ignored() {
        let mut m = machine_with_nvmc();
        m.bus.write_u32(NVMC_BASE + OFF_CONFIG, WEN).unwrap();
        m.bus.write_u8(0x0800, 0x11).unwrap();
        m.bus.write_u32(NVMC_BASE + OFF_CONFIG, 0).unwrap();

        m.bus.write_u32(NVMC_BASE + OFF_ERASEALL, 1).unwrap();
        step_once(&mut m);

        assert_eq!(
            m.bus.read_u8(0x0800).unwrap(),
            0x11,
            "erase without CONFIG.Een must be ignored"
        );
    }

    /// Erase must still land when the NVMC is NOT the first peripheral on the
    /// bus, and must still be a clean-boundary effect (nothing erased until
    /// the instruction commits). Guards the cached-index resolution in
    /// `Machine::new` against an off-by-anything: with the index pinned to 0
    /// this fails on all three op kinds while every other test here passes.
    #[test]
    fn erase_ops_land_when_nvmc_is_not_the_first_peripheral() {
        for pad in [1usize, 5] {
            // ERASEPAGE
            let mut m = machine_with_nvmc_at_offset(pad);
            assert!(
                m.bus.peripherals[0].name.starts_with("pad"),
                "padding must sit ahead of the NVMC (pad={pad})"
            );
            m.bus.write_u32(NVMC_BASE + OFF_CONFIG, WEN).unwrap();
            m.bus.write_u8(0x0800, 0x11).unwrap();
            m.bus.write_u8(0x1800, 0x22).unwrap();
            m.bus.write_u32(NVMC_BASE + OFF_CONFIG, EEN).unwrap();
            m.bus.write_u32(NVMC_BASE + OFF_ERASEPAGE, 0x0800).unwrap();
            assert_eq!(
                m.bus.read_u8(0x0800).unwrap(),
                0x11,
                "erase is latched, not applied at the store (pad={pad})"
            );
            step_once(&mut m);
            assert_eq!(
                m.bus.read_u8(0x0800).unwrap(),
                0xFF,
                "ERASEPAGE must land with the NVMC at index {pad}"
            );
            assert_eq!(m.bus.read_u8(0x1800).unwrap(), 0x22, "other page untouched");

            // ERASEALL
            let mut m = machine_with_nvmc_at_offset(pad);
            m.bus.write_u32(NVMC_BASE + OFF_CONFIG, WEN).unwrap();
            m.bus.write_u8(0xF000, 0x22).unwrap();
            m.bus.write_u32(NVMC_BASE + OFF_CONFIG, EEN).unwrap();
            m.bus.write_u32(NVMC_BASE + OFF_ERASEALL, 1).unwrap();
            step_once(&mut m);
            assert_eq!(
                m.bus.read_u8(0xF000).unwrap(),
                0xFF,
                "ERASEALL must land with the NVMC at index {pad}"
            );

            // ERASEUICR
            let mut m = machine_with_nvmc_at_offset(pad);
            m.bus.write_u32(UICR_BASE + 0x080, 0x1234_5678).unwrap();
            m.bus.write_u32(NVMC_BASE + OFF_CONFIG, EEN).unwrap();
            m.bus.write_u32(NVMC_BASE + OFF_ERASEUICR, 1).unwrap();
            step_once(&mut m);
            assert_eq!(
                m.bus.read_u32(UICR_BASE + 0x080).unwrap(),
                0xFFFF_FFFF,
                "ERASEUICR must land with the NVMC at index {pad}"
            );
        }
    }

    #[test]
    fn nvmc_ready_always_reads_one() {
        let bus = nrf52_bus();
        assert_eq!(bus.read_u32(NVMC_BASE + OFF_READY).unwrap(), 1);
    }
}
