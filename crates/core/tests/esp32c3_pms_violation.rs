// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! The ESP32-C3 permission-control unit (PMS) must catch what silicon catches.
//!
//! # The gap this pins
//!
//! A real ESP32-C3 running the published "BLE Pong" sketch panicked with
//! `Guru Meditation Error: Core 0 panic'ed (Memory protection fault)` —
//! `mcause = 26` = `ETS_MEMPROT_ERR_INUM`. The same image ran clean in the
//! twin, because the twin did not model the PMS at all: `SENSITIVE`'s
//! permission registers were inert storage, so a stray write into
//! write-protected IRAM text, or a jump into the IRAM view of the data region
//! through a corrupted function pointer, was silently permitted. An entire bug
//! class was invisible in the browser and only findable on a bench.
//!
//! # What is asserted, and why it is these observables
//!
//! Not "the model has a struct". The four things a user can actually see:
//!
//! 1. **The access is blocked.** A store into IRAM text does not land.
//! 2. **The status registers say what happened.** Read back through the exact
//!    field decode ESP-IDF's `esp_memprot_get_violate_addr/world/operation`
//!    uses, so a twin fault is *diagnosable*, not merely present.
//! 3. **The interrupt reaches the CPU.** Through the real interrupt matrix, as
//!    `ETS_MEMPROT_ERR_INUM` (26) — whose vector-table slot in IDF is
//!    `_panic_handler`. That is what makes the firmware's own panic run.
//! 4. **Nothing else changes.** The reset configuration grants full
//!    permissions on every area, so firmware that never enables memory
//!    protection must be untouched. Over-strictness would fault labs that work
//!    on silicon, which is worse than the gap being closed.
//!
//! Every number here is derived from ESP-IDF v5.3.1
//! (`soc/esp32c3/include/soc/memprot_defs.h`,
//! `hal/esp32c3/include/hal/memprot_ll.h`,
//! `esp_hw_support/port/esp32c3/esp_memprot.c`) and configured through MMIO,
//! the way firmware configures it — nothing here reaches into the model.

use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::bus::SystemBus;
use labwired_core::peripherals::esp32c3::pms::PmsPort;
use labwired_core::{Bus, Cpu};
use std::path::PathBuf;

// ── SENSITIVE (0x600C_1000) PMS registers ────────────────────────────────────
const SENSITIVE: u64 = 0x600C_1000;
const SPLIT_MAIN_I_D: u64 = 0x094;
const SPLIT_IRAM_0: u64 = 0x098;
const SPLIT_IRAM_1: u64 = 0x09C;
const SPLIT_DRAM_0: u64 = 0x0A0;
const SPLIT_DRAM_1: u64 = 0x0A4;
const IRAM0_PMS_CONSTRAIN_2: u64 = 0x0B0;
const IRAM0_PMS_MONITOR_1: u64 = 0x0B8;
const IRAM0_PMS_MONITOR_2: u64 = 0x0BC;
const DRAM0_PMS_CONSTRAIN_1: u64 = 0x0C4;
const DRAM0_PMS_MONITOR_1: u64 = 0x0CC;
const DRAM0_PMS_MONITOR_2: u64 = 0x0D0;
const DRAM0_PMS_MONITOR_3: u64 = 0x0D4;

// ── INTERRUPT_CORE0 (0x600C_2000) ────────────────────────────────────────────
const INTC: u64 = 0x600C_2000;
/// `ETS_CORE0_IRAM0_PMS_INTR_SOURCE` (`CORE_0_IRAM0_PMS_MONITOR_VIOLATE_INTR_MAP`
/// sits at INTC + 0xE0 = source 56 * 4 in the C3 descriptor).
const IRAM0_PMS_SOURCE: u64 = 56;
/// `ETS_CORE0_DRAM0_PMS_INTR_SOURCE` (INTC + 0xE4).
const DRAM0_PMS_SOURCE: u64 = 57;
/// `ETS_MEMPROT_ERR_INUM` — `soc/esp32c3/include/soc/soc.h`.
const MEMPROT_INUM: u32 = 26;

// ── C3 SRAM geometry (`memprot_defs.h`) ──────────────────────────────────────
const IRAM0_SRAM_LOW: u32 = 0x4038_0000;
const DRAM0_SRAM_LOW: u32 = 0x3FC8_0000;
const I_D_SRAM_SEGMENT_SIZE: u32 = 0x2_0000;

/// The REAL split address of the image that faulted on silicon, not an invented
/// one. Derived from the app segments of the arduino-esp32 2.0.17 / IDF v4.4.7
/// BLE Pong build (`elf_sha256 7c431e3d…`): IRAM text spans
/// `0x4038_0000..0x4039_0A84`, `.data` starts at `0x3FC9_0C00`, and `gp`
/// (= `.data + 0x800` per the RISC-V small-data model) independently confirms
/// that base. `_iram_text_end` rounds up to the 512-byte memprot alignment
/// (`CONFIG_ESP_SYSTEM_MEMPROT_MEM_ALIGN_SIZE`), giving `0x4039_0C00` ≡
/// `0x3FC9_0C00` — and `MAP_IRAM_TO_DRAM` maps one onto the other exactly.
const IRAM_TEXT_END: u32 = 0x4039_0C00;
/// First byte of `.data` on that image. A store BELOW this is the real-world
/// signature of the fault: an underflow off the bottom of the lowest DRAM
/// object, or a gp-relative store with a negative offset.
const DATA_START: u32 = 0x3FC9_0C00;
/// Inside IRAM area 0 (R|X under the IDF default) — "instruction memory".
const IRAM_TEXT_ADDR: u32 = 0x4038_1000;
/// Where the test's own code lives; also IRAM area 0, so it stays executable.
const CODE_ADDR: u32 = 0x4038_2000;
/// At or above the split line: IRAM area 3, permissions NONE under the IDF
/// default. This is the IRAM view of the data region — "data memory" as the
/// instruction bus sees it, and where a corrupted function pointer lands.
const IRAM_DATA_ADDR: u32 = 0x4039_4000;

/// `MAP_IRAM_TO_DRAM`.
fn map_iram_to_dram(addr: u32) -> u32 {
    addr - IRAM0_SRAM_LOW + DRAM0_SRAM_LOW
}

fn root(rel: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(rel)
}

/// The shipped `esp32c3-devkit` bus with matrix routing on — the same shape
/// the browser's ROM-boot entry builds.
fn bus_esp32c3() -> SystemBus {
    let chip = ChipDescriptor::from_file(root("configs/chips/esp32c3.yaml")).expect("chip yaml");
    let system_path = root("configs/systems/esp32c3-devkit.yaml");
    let mut manifest = SystemManifest::from_file(&system_path).expect("system yaml");
    let anchored = system_path.parent().expect("parent").join(&manifest.chip);
    manifest.chip = anchored.to_string_lossy().into_owned();
    manifest.walk_deleted = None;
    let mut bus = SystemBus::from_config(&chip, &manifest).expect("build c3 bus");
    let _ = labwired_core::system::riscv::configure_riscv(&mut bus);
    bus.esp32c3_irq_routing = true;
    bus.recompute_walk_deletable();
    bus
}

/// Encode a split-line configuration register the way
/// `memprot_ll_set_iram0_split_line` does: an 8-bit 512-byte-granular offset
/// plus a category triple naming the 128 KiB segment it is relative to.
fn encode_split_line(addr: u32, base: u32) -> u32 {
    assert_eq!(addr % 0x200, 0, "PMS split lines are 512B aligned");
    let seg = (addr - base) / I_D_SRAM_SEGMENT_SIZE;
    let cat: [u32; 3] = match seg {
        0 => [2, 3, 3],
        1 => [0, 2, 3],
        _ => [0, 0, 2],
    };
    (((addr >> 9) & 0xFF) << 14) | cat[0] | (cat[1] << 2) | (cat[2] << 4)
}

/// Program the PMS exactly as `esp_mprot_set_prot()` does with IDF defaults:
/// monitors off, all split lines at `_iram_text_end`, IRAM areas 0..2 = R|X and
/// area 3 = none, DRAM area 0 = none and areas 1..3 = R|W, then clear and
/// re-enable the monitors. Every write is an ordinary MMIO store.
fn configure_memprot_like_idf(bus: &mut SystemBus) {
    // 1. Disable the monitors before touching anything (IDF does this first).
    bus.write_u32(SENSITIVE + IRAM0_PMS_MONITOR_1, 0).unwrap();
    bus.write_u32(SENSITIVE + DRAM0_PMS_MONITOR_1, 0).unwrap();

    // 2. Split lines: IRAM line 1, line 0, main I/D, then the two DRAM DMA
    //    lines at the mapped address. IDF writes the same address to all five.
    let i_enc = encode_split_line(IRAM_TEXT_END, IRAM0_SRAM_LOW);
    let d_enc = encode_split_line(map_iram_to_dram(IRAM_TEXT_END), DRAM0_SRAM_LOW);
    for off in [SPLIT_IRAM_1, SPLIT_IRAM_0, SPLIT_MAIN_I_D] {
        bus.write_u32(SENSITIVE + off, i_enc).unwrap();
    }
    for off in [SPLIT_DRAM_0, SPLIT_DRAM_1] {
        bus.write_u32(SENSITIVE + off, d_enc).unwrap();
    }

    // 3. Permissions. `REG_SET_FIELD` is read-modify-write, so the cache-data
    //    -array and ROM fields keep their reset values.
    //    IRAM: R|W|F are bits 0|1|2 of each 3-bit area field.
    const R: u32 = 0x1;
    const W: u32 = 0x2;
    const F: u32 = 0x4;
    let cur = bus.read_u32(SENSITIVE + IRAM0_PMS_CONSTRAIN_2).unwrap();
    let rx = R | F;
    let iram = (cur & !0xFFF) | rx | (rx << 3) | (rx << 6); // area 3 -> none
    bus.write_u32(SENSITIVE + IRAM0_PMS_CONSTRAIN_2, iram)
        .unwrap();

    // DRAM: R|W are bits 0|1 of each 2-bit area field; area 0 -> none.
    let cur = bus.read_u32(SENSITIVE + DRAM0_PMS_CONSTRAIN_1).unwrap();
    let rw = R | W;
    let dram = (cur & !0xFF) | (rw << 2) | (rw << 4) | (rw << 6);
    bus.write_u32(SENSITIVE + DRAM0_PMS_CONSTRAIN_1, dram)
        .unwrap();

    // 4. Clear any stale violation, then re-enable the monitors.
    for off in [IRAM0_PMS_MONITOR_1, DRAM0_PMS_MONITOR_1] {
        bus.write_u32(SENSITIVE + off, 0x3).unwrap(); // VIOLATE_CLR | VIOLATE_EN
        bus.write_u32(SENSITIVE + off, 0x2).unwrap(); // VIOLATE_EN
    }

    assert!(
        bus.esp32c3_pms_armed(),
        "precondition: the IDF default configuration must arm the PMS"
    );
}

/// `esp_mprot_set_intr_matrix`: route the IRAM0 PMS source to
/// `ETS_MEMPROT_ERR_INUM`, give the line medium priority and enable it.
fn route_pms_to_memprot_inum(bus: &mut SystemBus) {
    // IDF routes BOTH PMS sources to the same INUM.
    bus.write_u32(INTC + IRAM0_PMS_SOURCE * 4, MEMPROT_INUM)
        .unwrap();
    bus.write_u32(INTC + DRAM0_PMS_SOURCE * 4, MEMPROT_INUM)
        .unwrap();
    bus.write_u32(INTC + 0x114 + u64::from(MEMPROT_INUM) * 4, 7)
        .unwrap();
    bus.write_u32(INTC + 0x104, 1 << MEMPROT_INUM).unwrap();
    bus.write_u32(INTC + 0x194, 1).unwrap();
}

/// Decode `SENSITIVE_CORE_0_IRAM0_PMS_MONITOR_2` the way IDF's
/// `memprot_ll_iram0_get_monitor_status_*` helpers do.
struct Iram0Status {
    intr: u32,
    wr: u32,
    loadstore: u32,
    world: u32,
    addr: u32,
}

fn read_iram0_status(bus: &SystemBus) -> Iram0Status {
    let w = bus.read_u32(SENSITIVE + IRAM0_PMS_MONITOR_2).unwrap();
    let field = (w >> 5) & 0x00FF_FFFF;
    Iram0Status {
        intr: w & 0x1,
        wr: (w >> 1) & 0x1,
        loadstore: (w >> 2) & 0x1,
        world: (w >> 3) & 0x3,
        // memprot_ll_iram0_get_monitor_status_fault_addr()
        addr: if field > 0 {
            (field << 2) + 0x4000_0000
        } else {
            0
        },
    }
}

/// A RISC-V core parked at `pc` with vectored `mtvec` and interrupts enabled —
/// the state IDF leaves the C3 in (`_vector_table` is installed with
/// `MODE = Vectored`, and slot 26 is `_panic_handler`).
fn cpu_at(pc: u32, mtvec_base: u32) -> labwired_core::cpu::riscv::RiscV {
    let mut cpu = labwired_core::cpu::riscv::RiscV::new();
    cpu.pc = pc;
    cpu.mtvec = mtvec_base | 1; // vectored
    cpu.mstatus |= 1 << 3; // MIE
    cpu
}

fn step(
    cpu: &mut labwired_core::cpu::riscv::RiscV,
    bus: &mut SystemBus,
) -> labwired_core::SimResult<()> {
    let cfg = bus.config.clone();
    cpu.step(bus, &[], &cfg)
}

// ─────────────────────────────────────────────────────────────────────────────

/// A store into write-protected IRAM text must be BLOCKED, and the violation
/// must be readable exactly where real IDF reads it.
///
/// Before the PMS model existed this store landed and every assertion below
/// failed: `read_u32` returned the stored word, `MONITOR_2` stayed 0, and no
/// interrupt line was ever asserted.
#[test]
fn store_into_write_protected_iram_is_blocked_and_reported() {
    let mut bus = bus_esp32c3();
    // Seed a known word while protection is still off, so "blocked" is proven
    // by the memory being UNCHANGED rather than by an absence of evidence.
    bus.write_u32(u64::from(IRAM_TEXT_ADDR), 0xA5A5_A5A5)
        .unwrap();
    configure_memprot_like_idf(&mut bus);
    route_pms_to_memprot_inum(&mut bus);

    assert_eq!(
        bus.esp32c3_pms_violations(),
        0,
        "precondition: no violation before the offending store"
    );

    // The stray store.
    bus.write_u32(u64::from(IRAM_TEXT_ADDR), 0xDEAD_BEEF)
        .unwrap();

    assert_eq!(
        bus.read_u32(u64::from(IRAM_TEXT_ADDR)).unwrap(),
        0xA5A5_A5A5,
        "PERMITTED: the store into write-protected IRAM text landed. \
         On silicon the PMS blocks it and panics."
    );
    assert_eq!(bus.esp32c3_pms_violations(), 1);

    // What IDF's panic handler reads.
    let st = read_iram0_status(&bus);
    assert_eq!(st.intr, 1, "IRAM0 VIOLATE_INTR must be latched");
    assert_eq!(st.wr, 1, "the violating operation was a write");
    assert_eq!(
        st.loadstore, 1,
        "LOADSTORE=1 -> esp_mprot_get_violate_operation() reports MEMPROT_OP_WRITE"
    );
    assert_eq!(st.world, 1, "MEMP_HAL_WORLD_0");
    assert_eq!(
        st.addr, IRAM_TEXT_ADDR,
        "esp_mprot_get_violate_addr() must name the faulting address"
    );

    // And the matrix must have raised ETS_MEMPROT_ERR_INUM.
    assert_ne!(
        bus.external_irq_lines() & (1 << MEMPROT_INUM),
        0,
        "UNDELIVERED: IRAM0 PMS source {IRAM0_PMS_SOURCE} never reached CPU line \
         {MEMPROT_INUM}; riscv_irq_lines={:#x}",
        bus.external_irq_lines()
    );
}

/// The CPU must take the trap, at the vector IDF wired to `_panic_handler`.
///
/// # On the two forms of `mcause`
///
/// A coredump from this fault shows `mcause = 0x0000_001A` — 26 with the
/// interrupt bit CLEAR — and that is what `panic_arch.c` compares against
/// `ETS_MEMPROT_ERR_INUM`. That value is not what the CSR holds. IDF's
/// `_call_panic_handler` in `components/riscv/vectors.S` branches on the high
/// bit (`li t0, 0x80000000; bgeu a1, t0, _call_panic_handler`) to tell an
/// interrupt from an exception, and only THEN strips it (`not t0, t0;
/// and a1, a1, t0`) before storing it in the exception frame.
///
/// So the CSR must carry `0x8000_001A` and the guest produces `0x1A` itself.
/// Delivering 26 with the bit already clear would send IDF down the
/// *exception* path and it would never classify the fault as memory
/// protection. This assertion is on the CSR, deliberately.
#[test]
fn cpu_takes_memprot_interrupt_after_a_protected_store() {
    let mut bus = bus_esp32c3();
    const MTVEC: u32 = 0x4038_8000;

    // Two instructions in IRAM area 0 (stays executable): materialise the
    // protected address, then store to it.
    //   lui  x5, 0x40381        -> 0x403812B7
    //   sw   x6, 0(x5)          -> 0x0062A023
    bus.write_u32(u64::from(CODE_ADDR), 0x4038_12B7).unwrap();
    bus.write_u32(u64::from(CODE_ADDR) + 4, 0x0062_A023)
        .unwrap();
    bus.write_u32(u64::from(IRAM_TEXT_ADDR), 0xA5A5_A5A5)
        .unwrap();

    configure_memprot_like_idf(&mut bus);
    route_pms_to_memprot_inum(&mut bus);

    let mut cpu = cpu_at(CODE_ADDR, MTVEC);
    step(&mut cpu, &mut bus).expect("lui must retire");
    assert_eq!(cpu.pc, CODE_ADDR + 4, "precondition: lui advanced the PC");

    step(&mut cpu, &mut bus).expect("the store must not stop the simulation");

    assert_eq!(
        cpu.mcause,
        0x8000_0000 | MEMPROT_INUM,
        "mcause must be an asynchronous interrupt on ETS_MEMPROT_ERR_INUM (26) — \
         what IDF's panic_arch.c turns into \"Memory protection fault\""
    );
    assert_eq!(
        cpu.pc,
        MTVEC + MEMPROT_INUM * 4,
        "vectored mtvec must dispatch to slot 26, which IDF fills with _panic_handler"
    );
    assert_eq!(
        bus.read_u32(u64::from(IRAM_TEXT_ADDR)).unwrap(),
        0xA5A5_A5A5,
        "the blocked store must not have landed"
    );
}

/// Executing from the IRAM view of the data region — where a corrupted
/// function pointer or a smashed return address lands — must fault.
///
/// Without the model the core happily executed the instruction sitting there,
/// which is the assertion that flips: `x7` would hold 0x123 and no trap taken.
#[test]
fn execute_from_data_region_is_blocked() {
    let mut bus = bus_esp32c3();
    const MTVEC: u32 = 0x4038_8000;

    // A perfectly valid instruction, in memory the instruction bus must refuse
    // to fetch from: `addi x7, x0, 0x123`.
    let addi_x7 = (0x123u32 << 20) | (7 << 7) | 0x13;
    bus.write_u32(u64::from(IRAM_DATA_ADDR), addi_x7).unwrap();

    configure_memprot_like_idf(&mut bus);
    route_pms_to_memprot_inum(&mut bus);

    let mut cpu = cpu_at(IRAM_DATA_ADDR, MTVEC);
    step(&mut cpu, &mut bus).expect("the blocked fetch must not stop the simulation");

    assert_eq!(
        cpu.x[7], 0,
        "EXECUTED: the instruction in the non-executable data region ran (x7={:#x}). \
         On silicon the fetch is blocked and the PMS panics.",
        cpu.x[7]
    );
    assert_eq!(
        cpu.mcause,
        0x8000_0000 | MEMPROT_INUM,
        "the blocked fetch must raise ETS_MEMPROT_ERR_INUM"
    );
    assert_eq!(cpu.pc, MTVEC + MEMPROT_INUM * 4);

    let st = read_iram0_status(&bus);
    assert_eq!(st.intr, 1);
    assert_eq!(
        st.loadstore, 0,
        "LOADSTORE=0 -> esp_mprot_get_violate_operation() reports MEMPROT_OP_EXEC"
    );
    assert_eq!(st.addr, IRAM_DATA_ADDR);
    assert_eq!(
        bus.esp32c3_pms_latched(PmsPort::Iram0)
            .expect("a violation must be latched")
            .addr,
        IRAM_DATA_ADDR
    );
}

/// `VIOLATE_CLR` must drop the latch, the status words AND the interrupt line.
/// A model that latches forever swaps a silent bug for an ISR storm.
#[test]
fn violate_clr_releases_the_latched_fault() {
    let mut bus = bus_esp32c3();
    configure_memprot_like_idf(&mut bus);
    route_pms_to_memprot_inum(&mut bus);

    bus.write_u32(u64::from(IRAM_TEXT_ADDR), 0xDEAD_BEEF)
        .unwrap();
    assert_ne!(bus.external_irq_lines() & (1 << MEMPROT_INUM), 0);

    // esp_mprot_monitor_clear_intr(): pulse VIOLATE_CLR.
    bus.write_u32(SENSITIVE + IRAM0_PMS_MONITOR_1, 0x3).unwrap();
    bus.write_u32(SENSITIVE + IRAM0_PMS_MONITOR_1, 0x2).unwrap();

    assert_eq!(read_iram0_status(&bus).intr, 0, "status must clear");
    assert!(bus.esp32c3_pms_latched(PmsPort::Iram0).is_none());
    assert_eq!(
        bus.external_irq_lines() & (1 << MEMPROT_INUM),
        0,
        "LATCHED: the memprot line stayed asserted after VIOLATE_CLR"
    );
}

/// **The regression rail.** Firmware that never enables memory protection must
/// be completely unaffected: every `..._PMS_CONSTRAIN_*` reset value grants
/// full permissions, so nothing may be blocked and the checker must not even
/// arm. If this fails, working labs are about to start faulting.
#[test]
fn reset_configuration_blocks_nothing() {
    let mut bus = bus_esp32c3();
    assert!(
        !bus.esp32c3_pms_armed(),
        "the reset PMS configuration must not arm the checker"
    );

    for addr in [
        IRAM_TEXT_ADDR,
        IRAM_DATA_ADDR,
        map_iram_to_dram(IRAM_TEXT_END) + 0x1000,
        DRAM0_SRAM_LOW + 0x100,
    ] {
        bus.write_u32(u64::from(addr), 0x1234_5678)
            .unwrap_or_else(|e| panic!("store to {addr:#x} must be permitted at reset: {e}"));
        assert_eq!(
            bus.read_u32(u64::from(addr)).unwrap(),
            0x1234_5678,
            "store to {addr:#x} must land at reset"
        );
    }
    assert_eq!(bus.esp32c3_pms_violations(), 0);

    // Fetching from anywhere is permitted too.
    let addi_x7 = (0x123u32 << 20) | (7 << 7) | 0x13;
    bus.write_u32(u64::from(IRAM_DATA_ADDR), addi_x7).unwrap();
    let mut cpu = cpu_at(IRAM_DATA_ADDR, 0x4038_8000);
    step(&mut cpu, &mut bus).expect("fetch must be permitted at reset");
    assert_eq!(cpu.x[7], 0x123, "the instruction must have executed");
    assert_eq!(cpu.mcause, 0, "no trap may be taken at reset");
}

/// Reconfiguration must not fault: `esp_mprot_set_prot` disables the monitors,
/// rewrites split lines and permissions, and only then re-enables — a model
/// that enforces during that window would fault IDF's own startup.
#[test]
fn no_enforcement_while_the_monitor_is_disabled() {
    let mut bus = bus_esp32c3();
    configure_memprot_like_idf(&mut bus);

    bus.write_u32(SENSITIVE + IRAM0_PMS_MONITOR_1, 0).unwrap();
    bus.write_u32(SENSITIVE + DRAM0_PMS_MONITOR_1, 0).unwrap();
    assert!(!bus.esp32c3_pms_armed());

    bus.write_u32(u64::from(IRAM_TEXT_ADDR), 0xC0FF_EE00)
        .unwrap();
    assert_eq!(
        bus.read_u32(u64::from(IRAM_TEXT_ADDR)).unwrap(),
        0xC0FF_EE00
    );
    assert_eq!(bus.esp32c3_pms_violations(), 0);
}

/// **The real-world signature.** `0x3FC9_0C00` is the first byte of `.data` on
/// the image that panicked on silicon, and the PMS split line sits exactly
/// there. A store that runs off the bottom of the lowest DRAM object — or a
/// gp-relative store with an underflowed offset — lands in DRAM0 area 0, whose
/// IDF permission is NONE, and faults on the DRAM0 port.
///
/// This is the case the coredump actually represents, so it is asserted against
/// the derived address rather than a synthetic one.
#[test]
fn store_below_data_start_faults_on_the_dram0_port() {
    let mut bus = bus_esp32c3();
    let below = DATA_START - 4;
    bus.write_u32(u64::from(below), 0xA5A5_A5A5).unwrap();
    configure_memprot_like_idf(&mut bus);
    route_pms_to_memprot_inum(&mut bus);

    // Sanity: the split line derived from the real image maps I <-> D exactly.
    assert_eq!(map_iram_to_dram(IRAM_TEXT_END), DATA_START);
    // The first word OF `.data` is fine...
    bus.write_u32(u64::from(DATA_START), 0x1111_1111).unwrap();
    assert_eq!(bus.read_u32(u64::from(DATA_START)).unwrap(), 0x1111_1111);
    assert_eq!(bus.esp32c3_pms_violations(), 0);

    // ...one word below it is not.
    bus.write_u32(u64::from(below), 0xDEAD_BEEF).unwrap();
    assert_eq!(
        bus.read_u32(u64::from(below)).unwrap(),
        0xA5A5_A5A5,
        "PERMITTED: a store one word below .data start ({below:#x}) landed. \
         That underflow is exactly what panicked on silicon."
    );
    assert_eq!(bus.esp32c3_pms_violations(), 1);

    let v = bus
        .esp32c3_pms_latched(PmsPort::Dram0)
        .expect("the DRAM0 port must latch it");
    assert_eq!(v.addr, below);

    // CORE_0_DRAM0_PMS_MONITOR_2 / _3, decoded as
    // memprot_ll_dram0_get_monitor_status_* do.
    let w2 = bus.read_u32(SENSITIVE + DRAM0_PMS_MONITOR_2).unwrap();
    let w3 = bus.read_u32(SENSITIVE + DRAM0_PMS_MONITOR_3).unwrap();
    assert_eq!(w2 & 0x1, 1, "DRAM0 VIOLATE_INTR must be latched");
    assert_eq!((w2 >> 2) & 0x3, 1, "MEMP_HAL_WORLD_0");
    let field = (w2 >> 4) & 0x00FF_FFFF;
    assert_eq!(
        (field << 2) + 0x3C00_0000,
        below,
        "esp_mprot_get_violate_addr(MEMPROT_TYPE_DRAM0_SRAM) must name the address"
    );
    assert_eq!(w3 & 0x1, 1, "the violating operation was a write");

    assert_ne!(
        bus.external_irq_lines() & (1 << MEMPROT_INUM),
        0,
        "UNDELIVERED: DRAM0 PMS source {DRAM0_PMS_SOURCE} never reached line {MEMPROT_INUM}"
    );
}

/// `CONFIG_ESP_SYSTEM_MEMPROT_FEATURE_LOCK=y` is the default and is set on the
/// image that faulted. Once firmware locks the PMS, protection CANNOT be
/// relaxed: later writes to the split lines, the permissions and the monitor
/// enables are ignored by silicon until reset.
///
/// A twin that let a stray (or malicious) write disable the monitor would
/// silently go back to being blind, which is the exact failure this whole
/// change exists to remove.
#[test]
fn locked_protection_cannot_be_turned_off() {
    let mut bus = bus_esp32c3();
    configure_memprot_like_idf(&mut bus);
    route_pms_to_memprot_inum(&mut bus);

    // esp_mprot_set_prot(..., lock_feature = true).
    bus.write_u32(SENSITIVE + 0x090, 1).unwrap(); // split lines
    bus.write_u32(SENSITIVE + 0x0A8, 1).unwrap(); // IRAM0 permissions
    bus.write_u32(SENSITIVE + 0x0B4, 1).unwrap(); // IRAM0 monitor
    bus.write_u32(SENSITIVE + 0x0C0, 1).unwrap(); // DRAM0 permissions
    bus.write_u32(SENSITIVE + 0x0C8, 1).unwrap(); // DRAM0 monitor

    // Everything an attacker (or a bug) would try, in order.
    bus.write_u32(SENSITIVE + IRAM0_PMS_MONITOR_1, 0).unwrap(); // disable monitor
    bus.write_u32(SENSITIVE + IRAM0_PMS_CONSTRAIN_2, 0x001C_7FFF)
        .unwrap(); // re-open permissions
    bus.write_u32(SENSITIVE + SPLIT_MAIN_I_D, 0).unwrap(); // erase the split
    bus.write_u32(SENSITIVE + 0x0B4, 0).unwrap(); // un-lock the lock

    assert_eq!(
        bus.read_u32(SENSITIVE + IRAM0_PMS_MONITOR_1).unwrap() & 0x2,
        0x2,
        "the monitor-enable write must have been ignored"
    );
    assert_eq!(
        bus.read_u32(SENSITIVE + 0x0B4).unwrap() & 1,
        1,
        "a lock bit is set-only; writing 0 must not clear it"
    );
    assert!(
        bus.esp32c3_pms_armed(),
        "DISARMED: locked memory protection was turned off"
    );

    // And it still fires.
    bus.write_u32(u64::from(IRAM_TEXT_ADDR), 0xDEAD_BEEF)
        .unwrap();
    assert_eq!(bus.esp32c3_pms_violations(), 1);
    assert_ne!(bus.external_irq_lines() & (1 << MEMPROT_INUM), 0);
}

/// Firmware must not be able to fake away a latched fault by writing the
/// status register directly — the hardware owns it. (`VIOLATE_CLR` in
/// `MONITOR_1` is the only way to clear it, covered above.)
#[test]
fn status_registers_are_hardware_owned() {
    let mut bus = bus_esp32c3();
    configure_memprot_like_idf(&mut bus);
    route_pms_to_memprot_inum(&mut bus);
    bus.write_u32(u64::from(IRAM_TEXT_ADDR), 0xDEAD_BEEF)
        .unwrap();
    assert_eq!(read_iram0_status(&bus).intr, 1);

    bus.write_u32(SENSITIVE + IRAM0_PMS_MONITOR_2, 0).unwrap();

    let st = read_iram0_status(&bus);
    assert_eq!(st.intr, 1, "a direct write must not clear VIOLATE_INTR");
    assert_eq!(st.addr, IRAM_TEXT_ADDR);
    assert_ne!(bus.external_irq_lines() & (1 << MEMPROT_INUM), 0);
}
