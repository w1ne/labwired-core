// LabWired - Firmware Simulation Platform
// SPDX-License-Identifier: MIT

//! Integration tests for STM32H5 FLASH pending-op drain in Machine::step.
//!
//! Tests that NSCR sector-erase fills the flash buffer with 0xFF and that
//! SWAP_BANK + OPTSTRT swaps the two 1 MB banks and reloads the CPU
//! reset vector from the new bank-1 content.
//!
//! Bus construction mirrors `h563_conformance.rs`: build a `SystemBus` from
//! the stm32h563.yaml chip descriptor + a minimal manifest, then call
//! `configure_cortex_m` to wire the CPU and shared SCB/NVIC peripherals.

use labwired_config::ChipDescriptor;
use labwired_core::peripherals::flash::h5;
use labwired_core::system::cortex_m::configure_cortex_m;
use labwired_core::{Bus, Cpu, DebugControl, Machine};

/// FLASH interface peripheral base address (RM0481, stm32h563.yaml).
const FLASH_BASE: u64 = 0x4002_2000;

/// Build a SystemBus + CortexM CPU wired to the STM32H563 chip descriptor.
fn h563_machine() -> Machine<labwired_core::cpu::CortexM> {
    let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../configs/chips/stm32h563.yaml");
    let chip = ChipDescriptor::from_file(&path).expect("load stm32h563.yaml");
    let manifest = labwired_config::SystemManifest {
        walk_deleted: Some(false),
        schema_version: "1.0".to_string(),
        name: "flash-h5-ops".to_string(),
        chip: path.to_string_lossy().to_string(),
        external_devices: vec![],
        board_io: vec![],
        debug_uart: None,
        peripherals: vec![],
        memory_overrides: Default::default(),
    };
    let mut bus = labwired_core::bus::SystemBus::from_config(&chip, &manifest).expect("build bus");
    let (cpu, _nvic) = configure_cortex_m(&mut bus);
    Machine::new(cpu, bus)
}

/// Unlock the non-secure flash key register (NSKEYR) via the bus.
fn unlock_nskeyr(m: &mut Machine<labwired_core::cpu::CortexM>) {
    m.bus
        .write_u32(FLASH_BASE + h5::NSKEYR_OFF, 0x4567_0123)
        .unwrap(); // KEY1
    m.bus
        .write_u32(FLASH_BASE + h5::NSKEYR_OFF, 0xCDEF_89AB)
        .unwrap(); // KEY2
}

/// Unlock the option-byte key register (OPTKEYR) via the bus.
fn unlock_optkeyr(m: &mut Machine<labwired_core::cpu::CortexM>) {
    m.bus.write_u32(FLASH_BASE + 0x0C, 0x0819_2A3B).unwrap(); // OPTKEY1
    m.bus.write_u32(FLASH_BASE + 0x0C, 0x4C5D_6E7F).unwrap(); // OPTKEY2
}

/// Write a 32-bit word directly into the flash buffer at an absolute address.
/// Used to plant sentinel values and vector-table entries before tests run.
fn write_flash_word(m: &mut Machine<labwired_core::cpu::CortexM>, abs_addr: u64, value: u32) {
    let bytes = value.to_le_bytes();
    for (i, b) in bytes.iter().enumerate() {
        m.bus.flash.write_u8(abs_addr + i as u64, *b);
    }
}

/// Read a 32-bit word from the flash buffer at an absolute address.
fn read_flash_word(m: &Machine<labwired_core::cpu::CortexM>, abs_addr: u64) -> u32 {
    m.bus.flash.read_u32(abs_addr).unwrap()
}

// ── Test 1: sector erase fills with 0xFF ────────────────────────────────────

/// Verify that NSCR SER+STRT (bank 0, sector 1) causes Machine::step to fill
/// the sector's flash range with 0xFF.
///
/// Sector 1 starts at offset SECTOR_SIZE (0x2000) from bank 0 base.
/// The CPU executes the 0x0000 instruction (LSL R0,R0,#0 = effective NOP)
/// from the zeroed flash buffer on every step, so `step()` always succeeds.
#[test]
fn erase_fills_sector_with_ff() {
    let mut m = h563_machine();

    // Flash base (absolute address of bank 0 sector 1).
    let bank0_base: u64 = h5::FLASH_BASE;
    let sector1_start: u64 = bank0_base + h5::SECTOR_SIZE; // offset = 0x2000

    // Plant a sentinel byte in sector 1 that is NOT 0xFF.
    write_flash_word(&mut m, sector1_start, 0x1234_5678);
    assert_eq!(
        read_flash_word(&m, sector1_start),
        0x1234_5678,
        "sentinel before erase"
    );

    // Unlock NSKEYR then trigger NSCR SER+SNB=1+STRT for bank 0 (BKSEL=0).
    unlock_nskeyr(&mut m);
    let nscr = h5::NSCR_SER | (1 << h5::NSCR_SNB_SHIFT) | h5::NSCR_STRT;
    m.bus.write_u32(FLASH_BASE + h5::NSCR_OFF, nscr).unwrap();

    // step() executes one NOP from zeroed flash, then drains the pending op.
    m.step().expect("step must not fail");

    // Sector 1 should now be all 0xFF.
    let first_word = read_flash_word(&m, sector1_start);
    assert_eq!(
        first_word, 0xFFFF_FFFF,
        "first word of sector after erase should be 0xFFFF_FFFF"
    );

    // Spot-check the last word of sector 1.
    let last_word_offset: u64 = sector1_start + h5::SECTOR_SIZE - 4;
    assert_eq!(
        read_flash_word(&m, last_word_offset),
        0xFFFF_FFFF,
        "last word of sector after erase should be 0xFFFF_FFFF"
    );

    // Sector 0 must be untouched (still zeroed — no erase was requested).
    assert_eq!(
        read_flash_word(&m, bank0_base),
        0x0000_0000,
        "sector 0 must be untouched by sector-1 erase"
    );
}

// ── Test 1b: sector erase targets bank 2 when BKSEL is set ─────────────────

/// Verify NSCR SER+BKSEL+STRT erases the sector in BANK 2 (offset
/// BANK_SIZE + sector*SECTOR_SIZE), leaving the same sector in bank 0 intact.
/// This covers the BKSEL→bank-2 path that the bank-0 test cannot, and pins the
/// invariant that bank 2 lives one BANK_SIZE above the flash base.
#[test]
fn erase_targets_bank2_with_bksel() {
    let mut m = h563_machine();
    assert_eq!(
        m.bus.flash.data.len() as u64,
        2 * h5::BANK_SIZE,
        "H563 flash buffer must be exactly 2 * BANK_SIZE"
    );

    let sector: u64 = 2;
    let bank0_addr = h5::FLASH_BASE + sector * h5::SECTOR_SIZE;
    let bank2_addr = h5::FLASH_BASE + h5::BANK_SIZE + sector * h5::SECTOR_SIZE;

    // Distinct sentinels in the same sector of each bank.
    write_flash_word(&mut m, bank0_addr, 0xAAAA_AAAA);
    write_flash_word(&mut m, bank2_addr, 0xBBBB_BBBB);

    unlock_nskeyr(&mut m);
    let nscr =
        h5::NSCR_SER | h5::NSCR_BKSEL | ((sector as u32) << h5::NSCR_SNB_SHIFT) | h5::NSCR_STRT;
    m.bus.write_u32(FLASH_BASE + h5::NSCR_OFF, nscr).unwrap();
    m.step().expect("step must not fail");

    assert_eq!(
        read_flash_word(&m, bank2_addr),
        0xFFFF_FFFF,
        "bank-2 sector should be erased to 0xFF"
    );
    assert_eq!(
        read_flash_word(&m, bank0_addr),
        0xAAAA_AAAA,
        "bank-0 sentinel must be untouched by a BKSEL (bank-2) erase"
    );
}

// ── Test 2: SWAP_BANK + OPTSTRT reboots into bank 2 ────────────────────────

/// Verify that OPTSR_PRG.SWAP_BANK + OPTCR.OPTSTRT causes Machine::step
/// to swap the two 1 MB flash banks and reload the CPU reset vector from the
/// (now-swapped) new bank 1 content.
///
/// Bank 1 vector table lives at flash[0]: SP=0x2000_0000, PC=0x0800_0009 (bank1-entry).
/// Bank 2 vector table lives at flash[BANK_SIZE]: SP=0x2001_0000, PC=0x0800_0405 (bank2-entry).
///
/// After swap_banks(BANK_SIZE), what was bank 2 is now at offset 0 of the
/// buffer → reset() reads bank2's SP and PC.
#[test]
fn swap_bank_reboots_into_bank2() {
    let mut m = h563_machine();

    // The chip yaml sizes flash as "2MiB" (binary) so the buffer is exactly
    // two architectural 1 MiB banks; bank 2 therefore lives at 0x08100000,
    // matching real silicon and the drain's swap_banks(h5::BANK_SIZE).
    assert_eq!(
        m.bus.flash.data.len() as u64,
        2 * h5::BANK_SIZE,
        "H563 flash buffer must be exactly 2 * BANK_SIZE (chip yaml must use MiB units)"
    );
    let bank1_base: u64 = h5::FLASH_BASE;
    let bank2_base: u64 = h5::FLASH_BASE + h5::BANK_SIZE;

    // Bank 1 reset vector (at flash buffer offset 0).
    let bank1_sp: u32 = 0x2000_0000;
    let bank1_pc: u32 = 0x0800_0009; // Thumb bit set = 0x0800_0008
    write_flash_word(&mut m, bank1_base, bank1_sp);
    write_flash_word(&mut m, bank1_base + 4, bank1_pc);

    // Bank 2 reset vector (at flash buffer offset BANK_SIZE).
    let bank2_sp: u32 = 0x2001_0000;
    let bank2_pc: u32 = 0x0810_0005; // Thumb bit set = 0x0810_0004
    write_flash_word(&mut m, bank2_base, bank2_sp);
    write_flash_word(&mut m, bank2_base + 4, bank2_pc);

    // Arm the swap: unlock OPTKEYR, set SWAP_BANK in OPTSR_PRG, then
    // OPTSTRT in OPTCR.  (NSKEYR is NOT required for swap — only OPTKEYR.)
    unlock_optkeyr(&mut m);

    // OPTSR_PRG: set SWAP_BANK (bit 31).
    m.bus
        .write_u32(FLASH_BASE + h5::OPTSR_PRG_OFF, h5::OPTSR_SWAP_BANK)
        .unwrap();

    // OPTCR: OPTSTRT (bit 1) → records SwapAndReset in the peripheral.
    m.bus
        .write_u32(FLASH_BASE + h5::OPTCR_OFF, h5::OPTCR_OPTSTRT)
        .unwrap();

    // step() executes one NOP (bank1 instruction at PC=0), then applies
    // swap_banks + reset.  After reset, VTOR=0 so SP/PC come from the start
    // of the flash buffer (boot alias 0 → 0x0800_0000 + 0) which now holds
    // bank 2's vector table.
    m.step().expect("step must not fail");

    // Banks are now swapped: bank2's content is at offset 0 of the buffer.
    assert_eq!(
        read_flash_word(&m, bank1_base),
        bank2_sp,
        "after swap: flash[0x08000000] should hold bank2 SP"
    );
    assert_eq!(
        read_flash_word(&m, bank1_base + 4),
        bank2_pc,
        "after swap: flash[0x08000004] should hold bank2 PC"
    );

    // CPU PC was reloaded from the swapped vector table.
    // reset() clears Thumb bit: pc = bank2_pc & !1.
    assert_eq!(
        m.cpu.get_pc(),
        bank2_pc & !1,
        "CPU PC should point to bank2 reset handler after swap+reset"
    );
}

// ── Test 3: H563 forces cycle-accurate execution ───────────────────────────

/// Lock the predicate that makes the batch/CLI run path apply FLASH ops: an
/// H5 op-modeling FLASH on the bus must force `requires_cycle_accurate()` true,
/// so the runner executes one instruction per batch and the per-instruction
/// FLASH-op drain fires. A regression that drops this would silently strand the
/// erase/swap on the shipping run path — this test fails loudly if so.
#[test]
fn h563_requires_cycle_accurate() {
    let m = h563_machine();
    assert!(
        m.bus.requires_cycle_accurate(),
        "H563 bus has an H5 op-modeling FLASH, so it must require cycle-accurate execution"
    );
}

// ── Test 4: erase applied via the batch/run path (not just step) ───────────

/// Same erase as `erase_fills_sector_with_ff`, but driven through `Machine::run`
/// — the path the CLI test runner and `Machine::run` take. Proves the op is
/// applied on the batch path (cycle-accurate clamp + per-iteration drain), not
/// only inside `step()`.
#[test]
fn erase_applied_via_run() {
    let mut m = h563_machine();

    let bank0_base: u64 = h5::FLASH_BASE;
    let sector1_start: u64 = bank0_base + h5::SECTOR_SIZE;

    write_flash_word(&mut m, sector1_start, 0x1234_5678);
    assert_eq!(
        read_flash_word(&m, sector1_start),
        0x1234_5678,
        "sentinel before erase"
    );

    unlock_nskeyr(&mut m);
    let nscr = h5::NSCR_SER | (1 << h5::NSCR_SNB_SHIFT) | h5::NSCR_STRT;
    m.bus.write_u32(FLASH_BASE + h5::NSCR_OFF, nscr).unwrap();

    // Drive via run() (the shipping batch path), not step().
    m.run(Some(2)).expect("run must not fail");

    assert_eq!(
        read_flash_word(&m, sector1_start),
        0xFFFF_FFFF,
        "first word of sector after erase via run() should be 0xFFFF_FFFF"
    );
    let last_word_offset: u64 = sector1_start + h5::SECTOR_SIZE - 4;
    assert_eq!(
        read_flash_word(&m, last_word_offset),
        0xFFFF_FFFF,
        "last word of sector after erase via run() should be 0xFFFF_FFFF"
    );
    assert_eq!(
        read_flash_word(&m, bank0_base),
        0x0000_0000,
        "sector 0 must be untouched by sector-1 erase via run()"
    );
}

// ── Test 5: bank swap + reset applied via the batch/run path ───────────────

/// Same swap as `swap_bank_reboots_into_bank2`, but driven through
/// `Machine::run`. Proves SWAP_BANK + OPTSTRT swaps banks AND resets the CPU
/// PC to bank2's reset vector on the shipping batch path.
#[test]
fn swap_applied_via_run() {
    let mut m = h563_machine();

    let bank1_base: u64 = h5::FLASH_BASE;
    let bank2_base: u64 = h5::FLASH_BASE + h5::BANK_SIZE;

    let bank1_sp: u32 = 0x2000_0000;
    let bank1_pc: u32 = 0x0800_0009;
    write_flash_word(&mut m, bank1_base, bank1_sp);
    write_flash_word(&mut m, bank1_base + 4, bank1_pc);

    let bank2_sp: u32 = 0x2001_0000;
    let bank2_pc: u32 = 0x0810_0005;
    write_flash_word(&mut m, bank2_base, bank2_sp);
    write_flash_word(&mut m, bank2_base + 4, bank2_pc);

    unlock_optkeyr(&mut m);
    m.bus
        .write_u32(FLASH_BASE + h5::OPTSR_PRG_OFF, h5::OPTSR_SWAP_BANK)
        .unwrap();
    m.bus
        .write_u32(FLASH_BASE + h5::OPTCR_OFF, h5::OPTCR_OPTSTRT)
        .unwrap();

    // Drive via run() (the shipping batch path), not step(). Limit to a single
    // instruction: the SwapAndReset op is already pending (recorded by the MMIO
    // writes above), so the first executed instruction triggers the drain →
    // swap + reset, landing the CPU exactly on bank2's reset vector. A larger
    // budget would execute further instructions from the (now bank2) reset
    // handler, advancing PC past the vector and making the landing PC fragile.
    m.run(Some(1)).expect("run must not fail");

    assert_eq!(
        read_flash_word(&m, bank1_base),
        bank2_sp,
        "after swap via run(): flash[0x08000000] should hold bank2 SP"
    );
    assert_eq!(
        read_flash_word(&m, bank1_base + 4),
        bank2_pc,
        "after swap via run(): flash[0x08000004] should hold bank2 PC"
    );
    assert_eq!(
        m.cpu.get_pc(),
        bank2_pc & !1,
        "CPU PC should point to bank2 reset handler after swap+reset via run()"
    );
}
