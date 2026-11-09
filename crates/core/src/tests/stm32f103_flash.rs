// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! STM32F103 FPEC program/erase gate on the committed chip yamls.
//!
//! The tests build `configs/chips/stm32f103rb.yaml` (NUCLEO-F103RB, 128 KiB)
//! and `stm32f103.yaml` (C8, 64 KiB) through [`SystemBus::from_config`] and
//! drive the HAL_FLASH sequence with real MMIO stores: KEYR unlock, PER + AR +
//! STRT page erase, PG half-word program, LOCK, unlock again. Register map:
//! RM0008 §3.4 (KEYR 0x04, SR 0x0C, CR 0x10, AR 0x14).

#[cfg(test)]
mod stm32f103_flash_tests {
    use crate::bus::SystemBus;
    use crate::cpu::CortexM;
    use crate::system::cortex_m::configure_cortex_m;
    use crate::{Bus, Machine};
    use labwired_config::{ChipDescriptor, SystemManifest};
    use std::path::PathBuf;

    const FLASH_IF: u64 = 0x4002_2000;
    const KEYR: u64 = FLASH_IF + 0x04;
    const SR: u64 = FLASH_IF + 0x0C;
    const CR: u64 = FLASH_IF + 0x10;
    const AR: u64 = FLASH_IF + 0x14;
    const KEY1: u32 = 0x4567_0123;
    const KEY2: u32 = 0xCDEF_89AB;
    const CR_PG: u32 = 1 << 0;
    const CR_PER: u32 = 1 << 1;
    const CR_STRT: u32 = 1 << 6;
    const CR_LOCK: u32 = 1 << 7;
    const SR_PGERR: u32 = 1 << 2;
    const SR_EOP: u32 = 1 << 5;

    fn machine(chip: &str) -> Machine<CortexM> {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(|p| p.parent())
            .expect("repo root")
            .join("configs/chips")
            .join(chip);
        let descriptor = ChipDescriptor::from_file(&path).expect("load committed chip yaml");
        let manifest = SystemManifest {
            parts: Vec::new(),
            walk_deleted: Some(false),
            schema_version: "1.0".to_string(),
            name: "f103-flash".to_string(),
            chip: path.to_string_lossy().into_owned(),
            cpu_hz: None,
            external_devices: vec![],
            cosim_models: Vec::new(),
            motor_models: Vec::new(),
            board_io: vec![],
            debug_uart: None,
            wifi_ap: None,
            peripherals: vec![],
            memory_overrides: Default::default(),
        };
        let mut bus = SystemBus::from_config(&descriptor, &manifest).expect("build bus");
        let (cpu, _nvic) = configure_cortex_m(&mut bus);
        Machine::new(cpu, bus)
    }

    fn unlock(m: &mut Machine<CortexM>) {
        m.bus.write_u32(KEYR, KEY1).unwrap();
        m.bus.write_u32(KEYR, KEY2).unwrap();
        assert_eq!(m.bus.read_u32(CR).unwrap() & CR_LOCK, 0, "unlock failed");
    }

    fn erase_page(m: &mut Machine<CortexM>, page_addr: u32) {
        m.bus.write_u32(CR, CR_PER).unwrap();
        m.bus.write_u32(AR, page_addr).unwrap();
        m.bus.write_u32(CR, CR_PER | CR_STRT).unwrap();
        m.bus.write_u32(CR, 0).unwrap();
    }

    fn program(m: &mut Machine<CortexM>, addr: u64, value: u16) {
        m.bus.write_u32(CR, CR_PG).unwrap();
        m.bus.write_u16(addr, value).unwrap();
        m.bus.write_u32(CR, 0).unwrap();
    }

    /// The STM32duino EEPROM.h flush on a NUCLEO-F103RB: the last 1 KiB page
    /// (0x0801_FC00) is erased, then reprogrammed with new data, twice, with
    /// a HAL_FLASH_Lock / HAL_FLASH_Unlock between the two passes.
    #[test]
    fn rb_last_page_erases_and_reprograms_across_relock() {
        let mut m = machine("stm32f103rb.yaml");
        let page = 0x0801_FC00u64;
        for (pass, value) in [(0, 0x1142u16), (1, 0xC0FE)] {
            unlock(&mut m);
            erase_page(&mut m, page as u32);
            assert_eq!(m.bus.read_u16(page).unwrap(), 0xFFFF, "pass {pass}: erased");
            assert_eq!(m.bus.read_u16(page + 0x3FE).unwrap(), 0xFFFF);
            program(&mut m, page, value);
            assert_eq!(
                m.bus.read_u16(page).unwrap(),
                value,
                "pass {pass}: programmed"
            );
            assert_eq!(m.bus.read_u32(SR).unwrap() & (SR_EOP | SR_PGERR), SR_EOP);
            m.bus.write_u32(SR, SR_EOP).unwrap();
            m.bus.write_u32(CR, CR_LOCK).unwrap();
        }
    }

    #[test]
    fn page_erase_is_one_kib_and_leaves_neighbours() {
        let mut m = machine("stm32f103rb.yaml");
        unlock(&mut m);
        program(&mut m, 0x0801_F7FE, 0x1234);
        program(&mut m, 0x0801_FC00, 0x5678);
        erase_page(&mut m, 0x0801_F800);
        assert_eq!(m.bus.read_u16(0x0801_F7FE).unwrap(), 0x1234);
        assert_eq!(m.bus.read_u16(0x0801_FC00).unwrap(), 0x5678);
    }

    #[test]
    fn programming_over_data_sets_pgerr_and_keeps_the_data() {
        let mut m = machine("stm32f103rb.yaml");
        unlock(&mut m);
        erase_page(&mut m, 0x0801_FC00);
        program(&mut m, 0x0801_FC10, 0x00F0);
        program(&mut m, 0x0801_FC10, 0x0F0F);
        assert_eq!(m.bus.read_u16(0x0801_FC10).unwrap(), 0x00F0);
        assert_eq!(m.bus.read_u32(SR).unwrap() & SR_PGERR, SR_PGERR);
    }

    #[test]
    fn locked_flash_is_not_programmed() {
        let mut m = machine("stm32f103rb.yaml");
        unlock(&mut m);
        erase_page(&mut m, 0x0801_FC00);
        m.bus.write_u32(CR, CR_LOCK).unwrap();
        m.bus.write_u32(CR, CR_PG).unwrap();
        m.bus.write_u16(0x0801_FC00, 0x1234).unwrap();
        assert_eq!(m.bus.read_u16(0x0801_FC00).unwrap(), 0xFFFF);
    }

    /// The C8 maps 64 KiB (DS5319 Table 2). Its last page is 0x0800_FC00.
    #[test]
    fn c8_last_datasheet_page_erases_and_programs() {
        let mut m = machine("stm32f103.yaml");
        unlock(&mut m);
        erase_page(&mut m, 0x0800_FC00);
        program(&mut m, 0x0800_FC00, 0xBEEF);
        assert_eq!(m.bus.read_u16(0x0800_FC00).unwrap(), 0xBEEF);
    }
}
