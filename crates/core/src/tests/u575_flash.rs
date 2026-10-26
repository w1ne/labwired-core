// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! STM32U575 FLASH program/erase gate — the controller the committed
//! `configs/chips/stm32u575.yaml` declares behind the `stm32u5` profile.
//!
//! These tests build the COMMITTED chip yaml through the production
//! [`SystemBus::from_config`] path (the same build the CLI and matrices use),
//! wire a Cortex-M33, and drive unlock → page erase → quad-word program →
//! status flags through real MMIO writes, exactly as a flash driver would.
//! Erases are recorded as pending ops and applied by `Machine::step` to the
//! flash backing store; programming is validated by the peripheral's U5 gate
//! and committed by the bus.
//!
//! Offsets/fields are SVD-verified against
//! `tests/fixtures/real_world/stm32u575.svd` (see `flash_u5_regs.rs`):
//! NSKEYR@0x08, OPTKEYR@0x10, NSSR@0x20 (EOP bit 0, WRPERR bit 4, BSY bit 16),
//! NSCR@0x28 (PG bit 0, PER bit 1, PNB[9:3], BKER bit 11, STRT bit 16,
//! LOCK bit 31, OPTLOCK bit 30), OPTR@0x40. Geometry: 2 × 1 MiB banks,
//! 8 KiB pages, 128-bit quad-word program granule (RM0456 §7.1).

#[cfg(test)]
mod u575_flash_tests {
    use crate::bus::SystemBus;
    use crate::cpu::CortexM;
    use crate::peripherals::flash::u5;
    use crate::system::cortex_m::configure_cortex_m;
    use crate::{Bus, Machine};
    use labwired_config::{ChipDescriptor, SystemManifest};
    use std::path::PathBuf;

    /// FLASH register interface base (SVD / chip yaml).
    const FLASH_IF: u64 = 0x4002_2000;

    const KEY1: u32 = 0x4567_0123;
    const KEY2: u32 = 0xCDEF_89AB;
    const OPTKEY1: u32 = 0x0819_2A3B;
    const OPTKEY2: u32 = 0x4C5D_6E7F;

    fn repo_root(rel: &str) -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(|p| p.parent())
            .expect("repo root")
            .join(rel)
    }

    fn manifest_for(chip_path: &str) -> SystemManifest {
        SystemManifest {
            parts: Vec::new(),
            walk_deleted: Some(false),
            schema_version: "1.0".to_string(),
            name: "u575-flash".to_string(),
            chip: chip_path.to_string(),
            cpu_hz: None,
            external_devices: vec![],
            cosim_models: Vec::new(),
            motor_models: Vec::new(),
            board_io: vec![],
            debug_uart: None,
            wifi_ap: None,
            peripherals: vec![],
            memory_overrides: Default::default(),
        }
    }

    /// The production build: committed chip yaml → `SystemBus::from_config`.
    fn u575_bus() -> SystemBus {
        let path = repo_root("configs/chips/stm32u575.yaml");
        let descriptor =
            ChipDescriptor::from_file(&path).expect("load committed stm32u575 chip yaml");
        let abs = path.to_string_lossy().into_owned();
        SystemBus::from_config(&descriptor, &manifest_for(&abs)).expect("build stm32u575 bus")
    }

    fn u575_machine() -> Machine<CortexM> {
        let mut bus = u575_bus();
        let (cpu, _nvic) = configure_cortex_m(&mut bus);
        Machine::new(cpu, bus)
    }

    fn unlock_ns(m: &mut Machine<CortexM>) {
        m.bus.write_u32(FLASH_IF + u5::NSKEYR_OFF, KEY1).unwrap();
        m.bus.write_u32(FLASH_IF + u5::NSKEYR_OFF, KEY2).unwrap();
    }

    fn unlock_opt(m: &mut Machine<CortexM>) {
        m.bus
            .write_u32(FLASH_IF + u5::OPTKEYR_OFF, OPTKEY1)
            .unwrap();
        m.bus
            .write_u32(FLASH_IF + u5::OPTKEYR_OFF, OPTKEY2)
            .unwrap();
    }

    fn nssr(m: &Machine<CortexM>) -> u32 {
        m.bus.read_u32(FLASH_IF + u5::NSSR_OFF).unwrap()
    }

    fn nscr(m: &Machine<CortexM>) -> u32 {
        m.bus.read_u32(FLASH_IF + u5::NSCR_OFF).unwrap()
    }

    /// Plant a 32-bit word directly in the flash backing store (test setup —
    /// goes around the program gate, exactly like a loader/fixture would).
    fn plant_word(m: &mut Machine<CortexM>, addr: u64, value: u32) {
        for (i, b) in value.to_le_bytes().iter().enumerate() {
            m.bus.flash.write_u8(addr + i as u64, *b);
        }
    }

    fn read_word(m: &Machine<CortexM>, addr: u64) -> u32 {
        m.bus.flash.read_u32(addr).unwrap()
    }

    // ── 1: unlock/lock ──────────────────────────────────────────────────────

    #[test]
    fn u5_flash_unlock_locks() {
        let mut m = u575_machine();
        // SVD reset: LOCK + OPTLOCK held.
        assert_eq!(nscr(&m), u5::NSCR_RESET, "NSCR reset = LOCK|OPTLOCK");

        // Configuration writes are ignored while locked.
        m.bus
            .write_u32(FLASH_IF + u5::NSCR_OFF, u5::NSCR_PG)
            .unwrap();
        assert_eq!(nscr(&m) & u5::NSCR_PG, 0, "PG write ignored while locked");

        // NSKEYR KEY1+KEY2 clears LOCK only; OPTLOCK stays. Wrong-key
        // lockout is covered by the peripheral-level controller test. A
        // Machine reset preserves peripherals, so reset recovery is not
        // covered by the current simulator contract.
        unlock_ns(&mut m);
        assert_eq!(nscr(&m) & u5::NSCR_LOCK, 0, "NS key sequence clears LOCK");
        assert_ne!(nscr(&m) & u5::NSCR_OPTLOCK, 0, "OPTKEYR domain is separate");

        // OPTKEYR sequence clears OPTLOCK.
        unlock_opt(&mut m);
        assert_eq!(nscr(&m) & u5::NSCR_OPTLOCK, 0);

        // Software re-lock (LOCK is set-only) takes effect immediately.
        m.bus
            .write_u32(FLASH_IF + u5::NSCR_OFF, u5::NSCR_LOCK)
            .unwrap();
        assert_ne!(nscr(&m) & u5::NSCR_LOCK, 0, "re-lock via NSCR.LOCK");
        m.bus
            .write_u32(FLASH_IF + u5::NSCR_OFF, u5::NSCR_PG)
            .unwrap();
        assert_eq!(nscr(&m) & u5::NSCR_PG, 0, "PG ignored after re-lock");
    }

    // ── 2: page erase zeroes the backing store ──────────────────────────────

    #[test]
    fn u5_flash_page_erase_zeroes() {
        let mut m = u575_machine();
        let page = 2u64;
        let page_addr = u5::FLASH_BASE + page * u5::PAGE_SIZE;
        let other_page_addr = u5::FLASH_BASE + 3 * u5::PAGE_SIZE;
        let bank2_addr = u5::FLASH_BASE + u5::BANK_SIZE + page * u5::PAGE_SIZE;

        plant_word(&mut m, page_addr, 0x1234_5678);
        plant_word(&mut m, page_addr + u5::PAGE_SIZE - 4, 0xDEAD_BEEF);
        plant_word(&mut m, other_page_addr, 0xCAFE_F00D);
        plant_word(&mut m, bank2_addr, 0xAAAA_AAAA);

        unlock_ns(&mut m);
        // PER + PNB=2 + STRT, bank 1 (BKER clear). 8 KiB page.
        m.bus
            .write_u32(
                FLASH_IF + u5::NSCR_OFF,
                u5::NSCR_PER
                    | u5::NSCR_EOPIE
                    | ((page as u32) << u5::NSCR_PNB_SHIFT)
                    | u5::NSCR_STRT,
            )
            .unwrap();
        assert_ne!(
            nssr(&m) & u5::NSSR_EOP,
            0,
            "EOP set when the erase completes"
        );
        assert_eq!(nscr(&m) & u5::NSCR_STRT, 0, "STRT self-clears");
        assert_eq!(nssr(&m) & u5::NSSR_BSY, 0, "BSY self-clears");

        m.step().expect("step must not fail");
        assert_eq!(
            read_word(&m, page_addr),
            0xFFFF_FFFF,
            "erased page reads 0xFF"
        );
        assert_eq!(
            read_word(&m, page_addr + u5::PAGE_SIZE - 4),
            0xFFFF_FFFF,
            "last word of the page erased"
        );
        assert_eq!(
            read_word(&m, other_page_addr),
            0xCAFE_F00D,
            "neighbouring page untouched"
        );
        assert_eq!(
            read_word(&m, bank2_addr),
            0xAAAA_AAAA,
            "same page in bank 2 untouched"
        );

        // BKER selects bank 2 for the same page number.
        m.bus
            .write_u32(
                FLASH_IF + u5::NSCR_OFF,
                u5::NSCR_PER
                    | u5::NSCR_EOPIE
                    | u5::NSCR_BKER
                    | ((page as u32) << u5::NSCR_PNB_SHIFT)
                    | u5::NSCR_STRT,
            )
            .unwrap();
        m.step().expect("step must not fail");
        assert_eq!(read_word(&m, bank2_addr), 0xFFFF_FFFF, "BKER erases bank 2");
        assert_eq!(
            read_word(&m, page_addr),
            0xFFFF_FFFF,
            "bank-1 page stays erased (not re-dirtied)"
        );
    }

    // ── 3: quad-word program ────────────────────────────────────────────────

    #[test]
    fn u5_flash_program_sets_bits_and_eop() {
        let mut m = u575_machine();
        let qw = u5::FLASH_BASE + 4 * u5::PAGE_SIZE;
        let words = [0x1111_1111u32, 0x2222_2222, 0x3333_3333, 0x4444_4444];

        unlock_ns(&mut m);
        // Program an erased (0xFF) quad-word: erase page 4 first.
        m.bus
            .write_u32(
                FLASH_IF + u5::NSCR_OFF,
                u5::NSCR_PER | u5::NSCR_EOPIE | (4 << u5::NSCR_PNB_SHIFT) | u5::NSCR_STRT,
            )
            .unwrap();
        m.step().expect("step must not fail");

        // Enable programming and write four successive 32-bit words.
        m.bus
            .write_u32(FLASH_IF + u5::NSCR_OFF, u5::NSCR_PG | u5::NSCR_EOPIE)
            .unwrap();
        for (i, word) in words.iter().enumerate() {
            m.bus.write_u32(qw + (i as u64) * 4, *word).unwrap();
        }

        assert_ne!(nssr(&m) & u5::NSSR_EOP, 0, "EOP set on quad-word commit");
        assert_eq!(nssr(&m) & u5::NSSR_WDW, 0, "WDW clears on commit");
        assert_eq!(nssr(&m) & u5::NSSR_WRPERR, 0, "no protection error");

        for (i, word) in words.iter().enumerate() {
            assert_eq!(
                read_word(&m, qw + (i as u64) * 4),
                *word,
                "programmed word {i} reads back (program set the bits)"
            );
        }

        // Programming only flips 1→0 (AND semantics): 0x1111 & 0x0F0F = 0x0101.
        m.bus.write_u32(qw, 0x0F0F_0F0F).unwrap();
        m.bus.write_u32(qw + 4, 0x0F0F_0F0F).unwrap();
        m.bus.write_u32(qw + 8, 0x0F0F_0F0F).unwrap();
        m.bus.write_u32(qw + 12, 0x0F0F_0F0F).unwrap();
        assert_eq!(read_word(&m, qw), 0x0101_0101, "bits only clear");
        assert_eq!(read_word(&m, qw + 4), 0x0202_0202, "bits only clear");
    }

    // ── 4: locked program/erase raises WRPERR ───────────────────────────────

    #[test]
    fn u5_flash_locked_write_sets_wrperr() {
        let mut m = u575_machine();
        let addr = u5::FLASH_BASE + 5 * u5::PAGE_SIZE;
        plant_word(&mut m, addr, 0x0000_0000);
        let before = read_word(&m, addr);

        // Reset state: LOCK held. A flash-region program store is a
        // write-protection error and commits nothing.
        m.bus.write_u32(addr, 0xAAAA_AAAA).unwrap();
        assert_ne!(nssr(&m) & u5::NSSR_WRPERR, 0, "locked program sets WRPERR");
        assert_eq!(
            read_word(&m, addr),
            before,
            "locked program commits nothing"
        );

        // Clear it (W1C), then a locked erase start is a WRPERR too.
        m.bus
            .write_u32(FLASH_IF + u5::NSSR_OFF, u5::NSSR_WRPERR)
            .unwrap();
        assert_eq!(nssr(&m) & u5::NSSR_WRPERR, 0, "W1C clears WRPERR");
        m.bus
            .write_u32(FLASH_IF + u5::NSCR_OFF, u5::NSCR_PER | u5::NSCR_STRT)
            .unwrap();
        assert_ne!(
            nssr(&m) & u5::NSSR_WRPERR,
            0,
            "locked erase start sets WRPERR"
        );
    }

    // ── 5: NSSR write-1-to-clear ────────────────────────────────────────────

    #[test]
    fn u5_flash_sr_w1c() {
        let mut m = u575_machine();
        unlock_ns(&mut m);
        // Erase page 1 (not page 0) to raise EOP.
        m.bus
            .write_u32(
                FLASH_IF + u5::NSCR_OFF,
                u5::NSCR_PER | u5::NSCR_EOPIE | (1 << u5::NSCR_PNB_SHIFT) | u5::NSCR_STRT,
            )
            .unwrap();
        m.step().expect("step must not fail");
        assert_ne!(nssr(&m) & u5::NSSR_EOP, 0, "EOP after erase");

        // Writing 0 to NSSR clears nothing.
        m.bus.write_u32(FLASH_IF + u5::NSSR_OFF, 0).unwrap();
        assert_ne!(nssr(&m) & u5::NSSR_EOP, 0, "write-0 clears nothing");

        // Writing 1 clears EOP; BSY stays read-only/low.
        m.bus
            .write_u32(FLASH_IF + u5::NSSR_OFF, u5::NSSR_EOP | u5::NSSR_BSY)
            .unwrap();
        assert_eq!(nssr(&m) & u5::NSSR_EOP, 0, "write-1 clears EOP");
        assert_eq!(nssr(&m) & u5::NSSR_BSY, 0, "BSY is live status, stays low");

        // WRPERR is W1C as well; locking then storing raises it.
        m.bus
            .write_u32(FLASH_IF + u5::NSCR_OFF, u5::NSCR_LOCK)
            .unwrap();
        m.bus
            .write_u32(u5::FLASH_BASE + 6 * u5::PAGE_SIZE, 1)
            .unwrap();
        assert_ne!(nssr(&m) & u5::NSSR_WRPERR, 0);
        m.bus
            .write_u32(FLASH_IF + u5::NSSR_OFF, u5::NSSR_WRPERR)
            .unwrap();
        assert_eq!(nssr(&m) & u5::NSSR_WRPERR, 0);
    }

    // ── 6: option-byte read + OPTKEYR/OPTLOCK gate ──────────────────────────

    #[test]
    fn u5_flash_option_bytes_read_and_lock() {
        let mut m = u575_machine();
        assert_eq!(
            m.bus.read_u32(FLASH_IF + u5::OPTR_OFF).unwrap(),
            0,
            "SVD OPTR reset = 0"
        );

        // OPTLOCK (and LOCK) held: OPTR writes are ignored.
        m.bus
            .write_u32(FLASH_IF + u5::OPTR_OFF, 0x0000_0010)
            .unwrap();
        assert_eq!(m.bus.read_u32(FLASH_IF + u5::OPTR_OFF).unwrap(), 0);

        // Both unlock domains must be open for option programming: NSKEYR
        // clears LOCK (NSCR register lock), OPTKEYR clears OPTLOCK.
        unlock_ns(&mut m);
        unlock_opt(&mut m);
        m.bus
            .write_u32(FLASH_IF + u5::OPTR_OFF, 0x0000_0010)
            .unwrap();
        assert_eq!(
            m.bus.read_u32(FLASH_IF + u5::OPTR_OFF).unwrap(),
            0x0000_0010,
            "option bytes are readable back after programming"
        );

        m.bus
            .write_u32(FLASH_IF + u5::NSCR_OFF, u5::NSCR_OPTSTRT | u5::NSCR_EOPIE)
            .unwrap();
        assert_ne!(nssr(&m) & u5::NSSR_EOP, 0, "OPTSTRT completes with EOP");
    }

    // ── 7: op-modeling FLASH pins cycle-accurate execution ──────────────────

    /// A U5 op-modelling FLASH installs the batch WATCH; it no longer pins the
    /// whole run to quantum 1.
    ///
    /// This asserted the opposite — `requires_cycle_accurate()` — and the
    /// inversion is deliberate. What the contract protects is the DRAIN POINT:
    /// the pending erase must be applied at the instruction that recorded it.
    /// Forcing quantum 1 was one way to get that, and it is extremely
    /// expensive: it clamps the planned window to a single instruction for the
    /// entire firmware, which also stops the Cortex-M hot-loop fast path from
    /// ever engaging (it needs a budget of >= 8). Measured on run
    /// 35542849216 against 35539932655, that clamp costs:
    ///
    ///     stm32h563   54.6 -> 848.5 Ir/step   (15.5x)
    ///     stm32h735   54.1 -> 797.2           (14.7x)
    ///     stm32u575   54.5 -> 853.0           (15.7x)
    ///
    /// The watch reaches the same drain point at ~1/15th the cost: the
    /// Cortex-M batch probes `Flash::has_pending_op` after each instruction
    /// and ENDS the batch at the recording write, so the machine boundary —
    /// and `Machine::apply_pending_flash_op` with it — lands on exactly that
    /// instruction.
    ///
    /// ⚠️ What this test does NOT cover: that the batch actually stops there.
    /// That lives in `CortexM::step_batch` and is what must not be removed;
    /// deleting the watch while leaving this test green is the hole to watch
    /// for, and a batch-level test for it is owed.
    #[test]
    fn u5_flash_models_ops_installs_the_batch_watch() {
        let m = u575_machine();
        assert!(
            m.bus.models_flash_ops(),
            "a U5 op-modelling FLASH must advertise itself so the Cortex-M \
             batch installs its pending-op watch"
        );
        assert!(
            !m.bus.requires_cycle_accurate(),
            "FLASH alone must no longer pin the quantum — the watch ends the \
             batch at the recording write instead, at ~1/15th the cost"
        );
    }
}
