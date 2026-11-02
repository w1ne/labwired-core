// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! AArch64 processing element: an Armv8.0-A Cortex-A53, one PE, with its
//! GICv3 CPU interface, redistributor, distributor and generic timer folded
//! into the core (see [`aarch64_gic`](super::aarch64_gic)).
//!
//! What is modelled (Arm Architecture Reference Manual for A-profile,
//! DDI 0487; Cortex-A53 TRM, DDI 0500):
//!
//! * **A64 integer ISA** of Armv8.0: data processing (immediate, register,
//!   shifted/extended, bitfield, extract, conditional select/compare,
//!   1/2/3-source incl. multiply-high and divide), branches, loads/stores in
//!   every addressing form, pairs, exclusives and load-acquire/store-release.
//!   CRC32/CRC32C (the A53 always has them). No LSE atomics or crypto:
//!   the A53 is Armv8.0 and `ID_AA64ISAR0_EL1` says so; libgcc's outline
//!   atomics take their LL/SC path for it.
//! * **SIMD&FP**: the 32 128-bit V registers with FPCR/FPSR, every SIMD&FP
//!   load/store form, `FMOV` between general and FP registers, `CNT` and
//!   `ADDV` (what GCC emits for popcount). Other SIMD/FP data processing is an
//!   UNDEFINED instruction, logged and trapped, never guessed.
//! * **Exception levels EL0-EL3**, all AArch64 (`SCR_EL3.RW`/`HCR_EL2.RW`
//!   are RES1 here). Reset is EL3h, as on the silicon. `SVC`/`HVC`/`SMC`/
//!   `BRK`, UNDEFINED, data/instruction aborts, PC alignment, FP access traps
//!   (`CPACR_EL1.FPEN`, `CPTR_EL2.TFP`, `CPTR_EL3.TFP`), IRQ routing by
//!   `SCR_EL3.IRQ` / `HCR_EL2.IMO`, `ERET`, banked `SP_ELx` with `SPSel`.
//!   The PE implements no Secure EL2 (`ID_AA64PFR0_EL1.SEL2` = 0), so EL2
//!   exists only in Non-secure state.
//! * **Stage-1 MMU** for the EL1&0, EL2 and EL3 regimes, 4 KB granule,
//!   levels 0-3, block and page descriptors, access-flag faults, AP[2:1]
//!   and PXN/UXN permission checks, `SCTLR.WXN`; `TLBI`, `AT S1E*` with
//!   `PAR_EL1`, `DC ZVA`. TLB entries are not ASID-tagged: every TTBR, TCR,
//!   SCTLR write and TLBI flushes the cached walks.
//! * Generic timer: physical, virtual, EL2 and secure physical channels
//!   over one system counter running at the chip's reference rate.
//!
//! Not modelled: stage 2, 16/64 KB granules, the hierarchical APTable/
//! XNTable controls, alignment faults (`SCTLR.A`) and SP alignment checks,
//! caches beyond their maintenance instructions being accepted, debug and
//! PMU registers (RAZ/WI), AArch32 at any EL, and the other three A53s.

use super::aarch64_gic::{GenericTimer, Gic};
use crate::{Bus, Cpu, SimResult, SimulationError, SimulationObserver};
use std::collections::HashMap;
use std::sync::Arc;

#[path = "aarch64_exec.rs"]
mod exec;

/// Encode an `MRS`/`MSR` system register: op0, op1, CRn, CRm, op2, exactly
/// as instruction bits [20:5].
pub const fn sr(op0: u32, op1: u32, crn: u32, crm: u32, op2: u32) -> u32 {
    (op0 << 14) | (op1 << 11) | (crn << 7) | (crm << 3) | op2
}

pub mod regs {
    use super::sr;
    pub const MIDR_EL1: u32 = sr(3, 0, 0, 0, 0);
    pub const MPIDR_EL1: u32 = sr(3, 0, 0, 0, 5);
    pub const REVIDR_EL1: u32 = sr(3, 0, 0, 0, 6);
    pub const ID_AA64PFR0_EL1: u32 = sr(3, 0, 0, 4, 0);
    pub const ID_AA64DFR0_EL1: u32 = sr(3, 0, 0, 5, 0);
    pub const ID_AA64ISAR0_EL1: u32 = sr(3, 0, 0, 6, 0);
    pub const ID_AA64MMFR0_EL1: u32 = sr(3, 0, 0, 7, 0);
    pub const CCSIDR_EL1: u32 = sr(3, 1, 0, 0, 0);
    pub const CLIDR_EL1: u32 = sr(3, 1, 0, 0, 1);
    pub const CSSELR_EL1: u32 = sr(3, 2, 0, 0, 0);
    pub const CTR_EL0: u32 = sr(3, 3, 0, 0, 1);
    pub const DCZID_EL0: u32 = sr(3, 3, 0, 0, 7);
    pub const SCTLR_EL1: u32 = sr(3, 0, 1, 0, 0);
    pub const CPACR_EL1: u32 = sr(3, 0, 1, 0, 2);
    pub const TTBR0_EL1: u32 = sr(3, 0, 2, 0, 0);
    pub const TTBR1_EL1: u32 = sr(3, 0, 2, 0, 1);
    pub const TCR_EL1: u32 = sr(3, 0, 2, 0, 2);
    pub const SPSR_EL1: u32 = sr(3, 0, 4, 0, 0);
    pub const ELR_EL1: u32 = sr(3, 0, 4, 0, 1);
    pub const SP_EL0: u32 = sr(3, 0, 4, 1, 0);
    pub const SPSEL: u32 = sr(3, 0, 4, 2, 0);
    pub const CURRENTEL: u32 = sr(3, 0, 4, 2, 2);
    pub const ICC_PMR_EL1: u32 = sr(3, 0, 4, 6, 0);
    pub const ESR_EL1: u32 = sr(3, 0, 5, 2, 0);
    pub const FAR_EL1: u32 = sr(3, 0, 6, 0, 0);
    pub const PAR_EL1: u32 = sr(3, 0, 7, 4, 0);
    pub const MAIR_EL1: u32 = sr(3, 0, 10, 2, 0);
    pub const VBAR_EL1: u32 = sr(3, 0, 12, 0, 0);
    pub const ISR_EL1: u32 = sr(3, 0, 12, 1, 0);
    pub const ICC_IAR0_EL1: u32 = sr(3, 0, 12, 8, 0);
    pub const ICC_EOIR0_EL1: u32 = sr(3, 0, 12, 8, 1);
    pub const ICC_HPPIR0_EL1: u32 = sr(3, 0, 12, 8, 2);
    pub const ICC_BPR0_EL1: u32 = sr(3, 0, 12, 8, 3);
    pub const ICC_DIR_EL1: u32 = sr(3, 0, 12, 11, 1);
    pub const ICC_RPR_EL1: u32 = sr(3, 0, 12, 11, 3);
    pub const ICC_SGI1R_EL1: u32 = sr(3, 0, 12, 11, 5);
    pub const ICC_IAR1_EL1: u32 = sr(3, 0, 12, 12, 0);
    pub const ICC_EOIR1_EL1: u32 = sr(3, 0, 12, 12, 1);
    pub const ICC_HPPIR1_EL1: u32 = sr(3, 0, 12, 12, 2);
    pub const ICC_BPR1_EL1: u32 = sr(3, 0, 12, 12, 3);
    pub const ICC_CTLR_EL1: u32 = sr(3, 0, 12, 12, 4);
    pub const ICC_SRE_EL1: u32 = sr(3, 0, 12, 12, 5);
    pub const ICC_IGRPEN0_EL1: u32 = sr(3, 0, 12, 12, 6);
    pub const ICC_IGRPEN1_EL1: u32 = sr(3, 0, 12, 12, 7);
    pub const TPIDR_EL1: u32 = sr(3, 0, 13, 0, 4);
    pub const CNTKCTL_EL1: u32 = sr(3, 0, 14, 1, 0);
    pub const NZCV: u32 = sr(3, 3, 4, 2, 0);
    pub const DAIF: u32 = sr(3, 3, 4, 2, 1);
    pub const FPCR: u32 = sr(3, 3, 4, 4, 0);
    pub const FPSR: u32 = sr(3, 3, 4, 4, 1);
    pub const TPIDR_EL0: u32 = sr(3, 3, 13, 0, 2);
    pub const TPIDRRO_EL0: u32 = sr(3, 3, 13, 0, 3);
    pub const CNTFRQ_EL0: u32 = sr(3, 3, 14, 0, 0);
    pub const CNTPCT_EL0: u32 = sr(3, 3, 14, 0, 1);
    pub const CNTVCT_EL0: u32 = sr(3, 3, 14, 0, 2);
    pub const CNTP_TVAL_EL0: u32 = sr(3, 3, 14, 2, 0);
    pub const CNTP_CTL_EL0: u32 = sr(3, 3, 14, 2, 1);
    pub const CNTP_CVAL_EL0: u32 = sr(3, 3, 14, 2, 2);
    pub const CNTV_TVAL_EL0: u32 = sr(3, 3, 14, 3, 0);
    pub const CNTV_CTL_EL0: u32 = sr(3, 3, 14, 3, 1);
    pub const CNTV_CVAL_EL0: u32 = sr(3, 3, 14, 3, 2);
    pub const SCTLR_EL2: u32 = sr(3, 4, 1, 0, 0);
    pub const HCR_EL2: u32 = sr(3, 4, 1, 1, 0);
    pub const CPTR_EL2: u32 = sr(3, 4, 1, 1, 2);
    pub const TTBR0_EL2: u32 = sr(3, 4, 2, 0, 0);
    pub const TCR_EL2: u32 = sr(3, 4, 2, 0, 2);
    pub const SPSR_EL2: u32 = sr(3, 4, 4, 0, 0);
    pub const ELR_EL2: u32 = sr(3, 4, 4, 0, 1);
    pub const SP_EL1: u32 = sr(3, 4, 4, 1, 0);
    pub const ESR_EL2: u32 = sr(3, 4, 5, 2, 0);
    pub const FAR_EL2: u32 = sr(3, 4, 6, 0, 0);
    pub const MAIR_EL2: u32 = sr(3, 4, 10, 2, 0);
    pub const VBAR_EL2: u32 = sr(3, 4, 12, 0, 0);
    pub const ICC_SRE_EL2: u32 = sr(3, 4, 12, 9, 5);
    pub const CNTVOFF_EL2: u32 = sr(3, 4, 14, 0, 3);
    pub const CNTHCTL_EL2: u32 = sr(3, 4, 14, 1, 0);
    pub const CNTHP_TVAL_EL2: u32 = sr(3, 4, 14, 2, 0);
    pub const CNTHP_CTL_EL2: u32 = sr(3, 4, 14, 2, 1);
    pub const CNTHP_CVAL_EL2: u32 = sr(3, 4, 14, 2, 2);
    pub const SCTLR_EL3: u32 = sr(3, 6, 1, 0, 0);
    pub const SCR_EL3: u32 = sr(3, 6, 1, 1, 0);
    pub const CPTR_EL3: u32 = sr(3, 6, 1, 1, 2);
    pub const TTBR0_EL3: u32 = sr(3, 6, 2, 0, 0);
    pub const TCR_EL3: u32 = sr(3, 6, 2, 0, 2);
    pub const SPSR_EL3: u32 = sr(3, 6, 4, 0, 0);
    pub const ELR_EL3: u32 = sr(3, 6, 4, 0, 1);
    pub const SP_EL2: u32 = sr(3, 6, 4, 1, 0);
    pub const ESR_EL3: u32 = sr(3, 6, 5, 2, 0);
    pub const FAR_EL3: u32 = sr(3, 6, 6, 0, 0);
    pub const MAIR_EL3: u32 = sr(3, 6, 10, 2, 0);
    pub const VBAR_EL3: u32 = sr(3, 6, 12, 0, 0);
    pub const RVBAR_EL3: u32 = sr(3, 6, 12, 0, 1);
    pub const ICC_CTLR_EL3: u32 = sr(3, 6, 12, 12, 4);
    pub const ICC_SRE_EL3: u32 = sr(3, 6, 12, 12, 5);
    pub const ICC_IGRPEN1_EL3: u32 = sr(3, 6, 12, 12, 7);
    pub const CNTPS_TVAL_EL1: u32 = sr(3, 7, 14, 2, 0);
    pub const CNTPS_CTL_EL1: u32 = sr(3, 7, 14, 2, 1);
    pub const CNTPS_CVAL_EL1: u32 = sr(3, 7, 14, 2, 2);
}

/// Registers with no behaviour beyond holding what was written: the
/// auxiliary/implementation-defined controls, thread IDs, virtualization
/// setup this model does not act on.
const PLAIN_REGS: &[u32] = &[
    sr(3, 0, 1, 0, 1),  // ACTLR_EL1
    sr(3, 0, 5, 1, 0),  // AFSR0_EL1
    sr(3, 0, 5, 1, 1),  // AFSR1_EL1
    sr(3, 0, 10, 3, 0), // AMAIR_EL1
    sr(3, 0, 13, 0, 1), // CONTEXTIDR_EL1
    sr(3, 0, 13, 0, 4), // TPIDR_EL1
    sr(3, 3, 13, 0, 2), // TPIDR_EL0
    sr(3, 3, 13, 0, 3), // TPIDRRO_EL0
    sr(3, 4, 0, 0, 0),  // VPIDR_EL2
    sr(3, 4, 0, 0, 5),  // VMPIDR_EL2
    sr(3, 4, 1, 0, 1),  // ACTLR_EL2
    sr(3, 4, 1, 1, 1),  // MDCR_EL2
    sr(3, 4, 1, 1, 3),  // HSTR_EL2
    sr(3, 4, 1, 1, 7),  // HACR_EL2
    sr(3, 4, 2, 1, 0),  // VTTBR_EL2
    sr(3, 4, 2, 1, 2),  // VTCR_EL2
    sr(3, 4, 5, 1, 0),  // AFSR0_EL2
    sr(3, 4, 5, 1, 1),  // AFSR1_EL2
    sr(3, 4, 6, 0, 4),  // HPFAR_EL2
    sr(3, 4, 10, 3, 0), // AMAIR_EL2
    sr(3, 4, 13, 0, 2), // TPIDR_EL2
    sr(3, 6, 1, 0, 1),  // ACTLR_EL3
    sr(3, 6, 1, 3, 1),  // MDCR_EL3
    sr(3, 6, 5, 1, 0),  // AFSR0_EL3
    sr(3, 6, 5, 1, 1),  // AFSR1_EL3
    sr(3, 6, 10, 3, 0), // AMAIR_EL3
    sr(3, 6, 13, 0, 2), // TPIDR_EL3
    sr(3, 1, 15, 2, 0), // CPUACTLR_EL1 (A53)
    sr(3, 1, 15, 2, 1), // CPUECTLR_EL1 (A53)
    sr(3, 1, 15, 2, 2), // CPUMERRSR_EL1 (A53)
    sr(3, 1, 11, 0, 2), // L2CTLR_EL1 (A53)
    sr(3, 1, 11, 0, 3), // L2ECTLR_EL1 (A53)
];

pub const PSTATE_N: u64 = 1 << 31;
pub const PSTATE_Z: u64 = 1 << 30;
pub const PSTATE_C: u64 = 1 << 29;
pub const PSTATE_V: u64 = 1 << 28;
const DAIF_MASK: u64 = 0xF << 6;
const PSTATE_I: u64 = 1 << 7;

const SCR_NS: u64 = 1 << 0;
const SCR_IRQ: u64 = 1 << 1;
const SCR_SMD: u64 = 1 << 7;
const SCR_HCE: u64 = 1 << 8;
const HCR_IMO: u64 = 1 << 4;
const HCR_TGE: u64 = 1 << 27;
const SCTLR_M: u64 = 1 << 0;
const SCTLR_WXN: u64 = 1 << 19;

/// Exception classes (ESR_ELx.EC).
pub mod ec {
    pub const UNKNOWN: u32 = 0x00;
    pub const WFX: u32 = 0x01;
    pub const FP_ACCESS: u32 = 0x07;
    pub const ILLEGAL_STATE: u32 = 0x0E;
    pub const SVC: u32 = 0x15;
    pub const HVC: u32 = 0x16;
    pub const SMC: u32 = 0x17;
    pub const SYSREG: u32 = 0x18;
    pub const IABT_LOWER: u32 = 0x20;
    pub const IABT_SAME: u32 = 0x21;
    pub const PC_ALIGN: u32 = 0x22;
    pub const DABT_LOWER: u32 = 0x24;
    pub const DABT_SAME: u32 = 0x25;
    pub const SP_ALIGN: u32 = 0x26;
    pub const BRK: u32 = 0x3C;
}

/// A synchronous exception raised by an instruction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Exception {
    pub ec: u32,
    pub iss: u32,
    pub far: Option<u64>,
    /// Where `ELR` points: the faulting instruction (aborts, UNDEFINED) or
    /// the one after it (`SVC`/`HVC`/`SMC`).
    pub return_next: bool,
    /// Forced target EL (FP traps to EL2/EL3, `HVC`, `SMC`), else the
    /// default for synchronous exceptions.
    pub target: Option<u8>,
}

impl Exception {
    pub fn new(ec: u32, iss: u32) -> Self {
        Self {
            ec,
            iss,
            far: None,
            return_next: false,
            target: None,
        }
    }
    pub fn undefined() -> Self {
        Self::new(ec::UNKNOWN, 0)
    }
}

/// Outcome of executing one instruction.
pub(crate) enum Flow {
    Next,
    Jump(u64),
    Exc(Exception),
}

/// Memory access kinds, for permission checks and fault syndromes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Access {
    Read,
    Write,
    Fetch,
}

#[derive(Debug, Clone, Copy)]
struct TlbEntry {
    pa_page: u64,
    /// AP[2:1] at bits 1:0, PXN bit 2, UXN bit 3, MAIR attr index bits 6:4.
    perms: u8,
    attr_index: u8,
}

/// A failed translation: the fault status code (ESR DFSC/IFSC).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct WalkFault {
    pub fsc: u32,
}

const FSC_TRANSLATION: u32 = 0x04;
const FSC_ACCESS_FLAG: u32 = 0x08;
const FSC_PERMISSION: u32 = 0x0C;

/// Per-chip PE parameters.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Aarch64Params {
    pub mpidr: u64,
    pub gicd_base: Option<u64>,
    pub gicr_base: Option<u64>,
    pub num_spis: usize,
    /// CPU clocks per system-counter tick.
    pub counter_divider: u64,
}

/// TI AM62x (AM6234 on the SK-AM62): GIC-600 distributor at 0x0180_0000,
/// redistributors from 0x0188_0000 (AM62x TRM SPRUIV7, memory map;
/// Zephyr `ti_am62x_a53.dtsi` `interrupt-controller@1800000`). The A53
/// cluster's system counter runs at 200 MHz (`CONFIG_SYS_CLOCK_HW_CYCLES_
/// PER_SEC` on every AM62x A53 board); the A53s run at 1.4 GHz on the
/// AM6234 speed grade the SK-AM62B-P1 fits. 256 SPIs (GIC-600 config).
const AM62_GICD: u64 = 0x0180_0000;
const AM62_GICR: u64 = 0x0188_0000;
const AM62_COUNTER_HZ: u64 = 200_000_000;
const AM62_SPIS: usize = 256;

impl Aarch64Params {
    pub fn for_chip(chip: &labwired_config::ChipDescriptor) -> Self {
        Self::for_chip_name(&chip.name, chip.cpu_hz)
    }

    pub fn for_chip_name(name: &str, cpu_hz: u64) -> Self {
        let name = name.to_ascii_lowercase();
        if name.starts_with("am62") {
            return Self {
                // Cluster 0, core 0; MPIDR.U = 0 (multiprocessor), bit 31 RES1.
                mpidr: 1 << 31,
                gicd_base: Some(AM62_GICD),
                gicr_base: Some(AM62_GICR),
                num_spis: AM62_SPIS,
                counter_divider: (cpu_hz / AM62_COUNTER_HZ).max(1),
            };
        }
        Self {
            mpidr: 1 << 31,
            gicd_base: None,
            gicr_base: None,
            num_spis: 0,
            counter_divider: 1,
        }
    }
}

/// True when a chip descriptor names an AArch64 core.
pub fn is_aarch64_chip(chip: &labwired_config::ChipDescriptor) -> bool {
    chip.arch == labwired_config::Arch::Aarch64
}

/// Snapshot of the architectural state of an [`Aarch64`] PE.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Aarch64CpuSnapshot {
    pub x: Vec<u64>,
    pub sp: Vec<u64>,
    pub pc: u64,
    pub nzcv: u64,
    pub daif: u64,
    pub el: u8,
    pub spsel: bool,
    /// V registers as (low, high) doublewords.
    pub v: Vec<(u64, u64)>,
    pub fpcr: u64,
    pub fpsr: u64,
    pub sctlr: Vec<u64>,
    pub vbar: Vec<u64>,
    pub elr: Vec<u64>,
    pub spsr: Vec<u64>,
    pub esr: Vec<u64>,
    pub far: Vec<u64>,
    pub tcr: Vec<u64>,
    pub ttbr0: Vec<u64>,
    pub mair: Vec<u64>,
    pub ttbr1_el1: u64,
    pub cpacr_el1: u64,
    pub cptr_el2: u64,
    pub cptr_el3: u64,
    pub hcr_el2: u64,
    pub scr_el3: u64,
    pub par_el1: u64,
    pub csselr_el1: u64,
    pub plain: Vec<(u32, u64)>,
    pub timer: GenericTimer,
    pub gic: Gic,
    pub cycles: u64,
}

#[derive(Debug)]
pub struct Aarch64 {
    pub x: [u64; 31],
    /// SP_EL0 .. SP_EL3.
    pub sp: [u64; 4],
    pub pc: u64,
    /// PSTATE.NZCV at bits 31:28.
    pub nzcv: u64,
    /// PSTATE.DAIF at bits 9:6.
    pub daif: u64,
    pub el: u8,
    pub spsel: bool,
    pub v: [u128; 32],
    pub fpcr: u64,
    pub fpsr: u64,

    /// Per-EL banks, index = EL (index 0 unused where the EL0 copy does not
    /// exist).
    pub sctlr: [u64; 4],
    pub vbar: [u64; 4],
    pub elr: [u64; 4],
    pub spsr: [u64; 4],
    pub esr: [u64; 4],
    pub far: [u64; 4],
    pub tcr: [u64; 4],
    pub ttbr0: [u64; 4],
    pub mair: [u64; 4],
    pub ttbr1_el1: u64,
    pub cpacr_el1: u64,
    pub cptr_el2: u64,
    pub cptr_el3: u64,
    pub hcr_el2: u64,
    pub scr_el3: u64,
    pub par_el1: u64,
    pub csselr_el1: u64,
    plain: HashMap<u32, u64>,

    pub timer: GenericTimer,
    pub gic: Gic,
    pub params: Aarch64Params,

    /// Local exclusive monitor: (physical address, size) of the last
    /// load-exclusive, cleared by a store-exclusive, `CLREX`, exception
    /// entry and `ERET`.
    pub exclusive: Option<(u64, u64)>,
    pub cycles: u64,
    waiting_for_interrupt: bool,
    tlb: HashMap<(u8, u64), TlbEntry>,
    /// Direct-mapped front of `tlb`: (regime, VA page, entry) by VA page.
    utlb: Box<[Option<(u8, u64, TlbEntry)>; UTLB_SIZE]>,
}

const UTLB_SIZE: usize = 256;

impl Default for Aarch64 {
    fn default() -> Self {
        Self::new(Aarch64Params::for_chip_name("", 1))
    }
}

/// `MIDR_EL1` of a Cortex-A53 r0p4 (A53 TRM, 4.3.1).
const MIDR_A53: u64 = 0x410F_D034;
/// EL0-EL3 AArch64 only, FP and AdvSIMD implemented, GICv3 system registers.
const ID_AA64PFR0: u64 = 0x0100_1111;
/// 4 KB granule supported (TGran4 = 0), 16 KB and 64 KB not, 40-bit PA, 16
/// ASID bits.
const ID_AA64MMFR0: u64 = (0xF << 24) | (0xF << 20) | (2 << 4) | 2;
/// Cache geometry the A53 reports: L1 I, L1 D, unified L2 (A53 TRM 4.3.22).
const CLIDR: u64 = 0x0A20_0023;
/// 64-byte lines in both caches; DminLine/IminLine = log2(16 words).
const CTR: u64 = 0x8444_C004;
/// `DC ZVA` permitted, 64-byte block (2^4 words).
const DCZID: u64 = 4;
const ZVA_BLOCK: u64 = 64;

impl Aarch64 {
    pub fn new(params: Aarch64Params) -> Self {
        let mut s = Self {
            x: [0; 31],
            sp: [0; 4],
            pc: 0,
            nzcv: 0,
            daif: DAIF_MASK,
            el: 3,
            spsel: true,
            v: [0; 32],
            fpcr: 0,
            fpsr: 0,
            sctlr: [0; 4],
            vbar: [0; 4],
            elr: [0; 4],
            spsr: [0; 4],
            esr: [0; 4],
            far: [0; 4],
            tcr: [0; 4],
            ttbr0: [0; 4],
            mair: [0; 4],
            ttbr1_el1: 0,
            cpacr_el1: 0,
            cptr_el2: 0,
            cptr_el3: 0,
            hcr_el2: 0,
            scr_el3: 0,
            par_el1: 0,
            csselr_el1: 0,
            plain: HashMap::new(),
            timer: GenericTimer::new(params.counter_divider),
            gic: Gic::new(
                params.gicd_base,
                params.gicr_base,
                params.num_spis,
                params.mpidr & 0xFF_00FF_FFFF,
            ),
            params,
            exclusive: None,
            cycles: 0,
            waiting_for_interrupt: false,
            tlb: HashMap::new(),
            utlb: Box::new([None; UTLB_SIZE]),
        };
        // SCTLR reset values with the RES1 bits the A53 reports (MMU, caches
        // and alignment checks off).
        s.sctlr[1] = 0x00C5_0838;
        s.sctlr[2] = 0x30C5_0838;
        s.sctlr[3] = 0x30C5_0838;
        s
    }

    pub fn for_chip(chip: &labwired_config::ChipDescriptor) -> Self {
        Self::new(Aarch64Params::for_chip(chip))
    }

    // ---- registers --------------------------------------------------------

    /// General register read with XZR for 31.
    #[inline]
    pub(crate) fn xr(&self, n: u32) -> u64 {
        if n == 31 {
            0
        } else {
            self.x[n as usize]
        }
    }

    /// General register read with SP for 31.
    #[inline]
    pub(crate) fn xsp(&self, n: u32) -> u64 {
        if n == 31 {
            self.sp[self.sp_index()]
        } else {
            self.x[n as usize]
        }
    }

    #[inline]
    pub(crate) fn set_xr(&mut self, n: u32, v: u64) {
        if n != 31 {
            self.x[n as usize] = v;
        }
    }

    #[inline]
    pub(crate) fn set_xsp(&mut self, n: u32, v: u64) {
        if n == 31 {
            let i = self.sp_index();
            self.sp[i] = v;
        } else {
            self.x[n as usize] = v;
        }
    }

    #[inline]
    fn sp_index(&self) -> usize {
        if self.spsel {
            self.el as usize
        } else {
            0
        }
    }

    /// PSTATE as an SPSR value (M[3:0] = EL:SPSel, AArch64).
    pub fn pstate(&self) -> u64 {
        self.nzcv | self.daif | (u64::from(self.el) << 2) | u64::from(self.spsel)
    }

    fn el2_enabled(&self) -> bool {
        // No Secure EL2: EL2 exists in Non-secure state only.
        self.scr_el3 & SCR_NS != 0
    }

    // ---- exceptions -------------------------------------------------------

    /// The EL a synchronous exception from the current EL is taken to.
    fn sync_target(&self) -> u8 {
        if self.el == 0 {
            if self.el2_enabled() && self.hcr_el2 & HCR_TGE != 0 {
                2
            } else {
                1
            }
        } else {
            self.el
        }
    }

    /// Enter `target` at vector `offset` (0 sync, 0x80 IRQ, 0x100 FIQ, 0x180
    /// SError) with `ret` in ELR.
    pub(crate) fn enter_exception(
        &mut self,
        target: u8,
        offset: u64,
        ret: u64,
        esr: Option<u64>,
        far: Option<u64>,
    ) {
        let t = target as usize;
        let base = if target > self.el {
            0x400 // lower EL, AArch64
        } else if self.spsel {
            0x200
        } else {
            0
        };
        self.spsr[t] = self.pstate();
        self.elr[t] = ret;
        if let Some(e) = esr {
            self.esr[t] = e;
        }
        if let Some(f) = far {
            self.far[t] = f;
        }
        self.el = target;
        self.spsel = true;
        self.daif = DAIF_MASK;
        self.exclusive = None;
        self.waiting_for_interrupt = false;
        self.pc = self.vbar[t].wrapping_add(base + offset);
    }

    pub(crate) fn take_sync(&mut self, e: Exception, pc: u64) {
        let target = e.target.unwrap_or_else(|| self.sync_target());
        let mut ec = e.ec;
        // Aborts encode "from a lower EL" in the class.
        if matches!(ec, ec::IABT_SAME | ec::DABT_SAME) && target > self.el {
            ec -= 1;
        }
        let esr = (u64::from(ec) << 26) | (1 << 25) | u64::from(e.iss & 0x1FF_FFFF);
        let ret = if e.return_next {
            pc.wrapping_add(4)
        } else {
            pc
        };
        self.enter_exception(target, 0, ret, Some(esr), e.far);
    }

    /// The EL a physical IRQ targets, and whether it is taken now.
    fn irq_target(&self) -> Option<u8> {
        let target = if self.scr_el3 & SCR_IRQ != 0 {
            3
        } else if self.el2_enabled() && self.hcr_el2 & (HCR_IMO | HCR_TGE) != 0 {
            2
        } else {
            1
        };
        if target < self.el {
            return None;
        }
        if target == self.el && self.daif & PSTATE_I != 0 {
            return None;
        }
        Some(target)
    }

    fn irq_line(&self) -> bool {
        self.gic.signalled(&self.timer).is_some()
    }

    pub(crate) fn eret(&mut self) -> Flow {
        let el = self.el as usize;
        let spsr = self.spsr[el];
        let target_el = ((spsr >> 2) & 3) as u8;
        // AArch32 targets and returns to a higher EL are illegal returns.
        if spsr & 0x10 != 0 || target_el > self.el || (target_el == 2 && !self.el2_enabled()) {
            return Flow::Exc(Exception::new(ec::ILLEGAL_STATE, 0));
        }
        self.nzcv = spsr & 0xF000_0000;
        self.daif = spsr & DAIF_MASK;
        self.el = target_el;
        self.spsel = spsr & 1 != 0;
        self.exclusive = None;
        Flow::Jump(self.elr[el])
    }

    /// FP/SIMD access check: `Some` when the instruction traps.
    pub(crate) fn fp_trap(&self) -> Option<Exception> {
        let fpen = (self.cpacr_el1 >> 20) & 3;
        let trap_el1 = match self.el {
            0 => fpen != 3,
            1 => fpen == 0 || fpen == 2,
            _ => false,
        };
        // ISS for an AArch64 trap: CV=1, COND=0b1110.
        let iss = 0x1E0_0000;
        if trap_el1 && !(self.el == 0 && self.el2_enabled() && self.hcr_el2 & HCR_TGE != 0) {
            let mut e = Exception::new(ec::FP_ACCESS, iss);
            e.target = Some(1);
            return Some(e);
        }
        if self.el <= 2 && self.el2_enabled() && self.cptr_el2 & (1 << 10) != 0 {
            let mut e = Exception::new(ec::FP_ACCESS, iss);
            e.target = Some(2);
            return Some(e);
        }
        if self.cptr_el3 & (1 << 10) != 0 {
            let mut e = Exception::new(ec::FP_ACCESS, iss);
            e.target = Some(3);
            return Some(e);
        }
        None
    }

    pub(crate) fn svc_hvc_smc(&mut self, op: u32, imm: u32) -> Flow {
        match op {
            // SVC
            1 => {
                let mut e = Exception::new(ec::SVC, imm);
                e.return_next = true;
                Flow::Exc(e)
            }
            // HVC: UNDEFINED at EL0, without EL2, or with SCR_EL3.HCE clear.
            2 => {
                if self.el == 0 || !self.el2_enabled() || self.scr_el3 & SCR_HCE == 0 {
                    return Flow::Exc(Exception::undefined());
                }
                let mut e = Exception::new(ec::HVC, imm);
                e.return_next = true;
                e.target = Some(self.el.max(2));
                Flow::Exc(e)
            }
            // SMC: UNDEFINED at EL0 or with SCR_EL3.SMD set.
            3 => {
                if self.el == 0 || self.scr_el3 & SCR_SMD != 0 {
                    return Flow::Exc(Exception::undefined());
                }
                let mut e = Exception::new(ec::SMC, imm);
                e.return_next = true;
                e.target = Some(3);
                Flow::Exc(e)
            }
            _ => Flow::Exc(Exception::undefined()),
        }
    }

    // ---- MMU --------------------------------------------------------------

    /// Translation regime of the current EL: 1 for EL1&0, else the EL.
    fn regime(&self) -> u8 {
        if self.el <= 1 {
            1
        } else {
            self.el
        }
    }

    pub(crate) fn flush_tlb(&mut self) {
        self.tlb.clear();
        self.utlb.fill(None);
    }

    /// Stage-1 walk for `va` in `regime`, cached.
    fn walk(
        &mut self,
        bus: &mut dyn Bus,
        regime: u8,
        va: u64,
    ) -> SimResult<Result<TlbEntry, WalkFault>> {
        let page = va >> 12;
        let slot = (page as usize) % UTLB_SIZE;
        if let Some((r, p, e)) = self.utlb[slot] {
            if r == regime && p == page {
                return Ok(Ok(e));
            }
        }
        let key = (regime, page);
        if let Some(e) = self.tlb.get(&key).copied() {
            self.utlb[slot] = Some((regime, page, e));
            return Ok(Ok(e));
        }
        let r = self.walk_uncached(bus, regime, va)?;
        if let Ok(e) = r {
            if self.tlb.len() > 8192 {
                self.tlb.clear();
            }
            self.tlb.insert(key, e);
            self.utlb[slot] = Some((regime, page, e));
        }
        Ok(r)
    }

    fn walk_uncached(
        &self,
        bus: &mut dyn Bus,
        regime: u8,
        va: u64,
    ) -> SimResult<Result<TlbEntry, WalkFault>> {
        let fault = |level: u32, kind: u32| Ok(Err(WalkFault { fsc: kind | level }));
        let tcr = self.tcr[regime as usize];
        let upper = regime == 1 && (va >> 55) & 1 != 0;
        let (tsz, epd, tg, ttbr) = if upper {
            (
                (tcr >> 16) & 0x3F,
                tcr & (1 << 23) != 0,
                match (tcr >> 30) & 3 {
                    2 => 0,
                    1 => 2,
                    3 => 1,
                    _ => 3,
                },
                self.ttbr1_el1,
            )
        } else {
            (
                tcr & 0x3F,
                regime == 1 && tcr & (1 << 7) != 0,
                (tcr >> 14) & 3,
                self.ttbr0[regime as usize],
            )
        };
        if tg != 0 {
            return Err(SimulationError::NotImplemented(format!(
                "AArch64 translation granule other than 4 KB (TCR_EL{regime} = {tcr:#x})"
            )));
        }
        let inputsize = 64 - tsz.clamp(16, 39);
        // The VA must be all-zeros (TTBR0) or all-ones (TTBR1) above inputsize.
        let top = if inputsize >= 64 { 0 } else { va >> inputsize };
        let in_range = if upper {
            top == (u64::MAX >> inputsize)
        } else {
            top == 0
        };
        if epd || !in_range {
            return fault(0, FSC_TRANSLATION);
        }
        // 4 KB granule: 9 bits per level, start level from the input size.
        let levels = (inputsize - 12).div_ceil(9);
        let mut level = 4 - levels as u32;
        let mut table = ttbr & 0x0000_FFFF_FFFF_F000;
        loop {
            let shift = 12 + 9 * (3 - level);
            let idx_bits = if level == 4 - levels as u32 {
                inputsize - u64::from(shift)
            } else {
                9
            };
            let idx = (va >> shift) & ((1 << idx_bits) - 1);
            let desc = bus.read_u64(table + idx * 8)?;
            if desc & 1 == 0 {
                return fault(level, FSC_TRANSLATION);
            }
            let is_table = desc & 2 != 0 && level < 3;
            if is_table {
                table = desc & 0x0000_FFFF_FFFF_F000;
                level += 1;
                continue;
            }
            if level == 3 && desc & 2 == 0 {
                return fault(level, FSC_TRANSLATION);
            }
            if level == 0 {
                // No level-0 blocks with a 4 KB granule.
                return fault(level, FSC_TRANSLATION);
            }
            if desc & (1 << 10) == 0 {
                return fault(level, FSC_ACCESS_FLAG);
            }
            let block_mask = (1u64 << shift) - 1;
            let oa = (desc & 0x0000_FFFF_FFFF_F000 & !block_mask) | (va & block_mask);
            let ap = ((desc >> 6) & 3) as u8;
            let pxn = ((desc >> 53) & 1) as u8;
            let uxn = ((desc >> 54) & 1) as u8;
            return Ok(Ok(TlbEntry {
                pa_page: oa >> 12,
                perms: ap | (pxn << 2) | (uxn << 3),
                attr_index: ((desc >> 2) & 7) as u8,
            }));
        }
    }

    /// Check `access` at `el` against a translation's permissions.
    fn permitted(&self, regime: u8, el: u8, e: &TlbEntry, access: Access) -> bool {
        let ap = e.perms & 3;
        let ro = ap & 2 != 0;
        let pxn = e.perms & 4 != 0;
        let uxn = e.perms & 8 != 0;
        let writable = !ro;
        // SCTLR.WXN: writable memory is never executable.
        let wxn_forbids = self.sctlr[regime as usize] & SCTLR_WXN != 0 && writable;
        if regime != 1 {
            // EL2/EL3 regimes: AP[1] is RES1; XN is bit 54.
            return match access {
                Access::Read => true,
                Access::Write => writable,
                Access::Fetch => !uxn && !wxn_forbids,
            };
        }
        let el0_access = ap & 1 != 0;
        let el0_writable = el0_access && writable;
        match (el, access) {
            (0, Access::Read) => el0_access,
            (0, Access::Write) => el0_writable,
            (0, Access::Fetch) => el0_access && !uxn && !wxn_forbids,
            (_, Access::Read) => true,
            (_, Access::Write) => writable,
            // EL1 never executes memory EL0 can write.
            (_, Access::Fetch) => !pxn && !el0_writable && !wxn_forbids,
        }
    }

    /// Translate `va` for `access` at the current EL (or `as_el`, for
    /// unprivileged and AT S1E0 accesses).
    pub(crate) fn translate(
        &mut self,
        bus: &mut dyn Bus,
        va: u64,
        access: Access,
        as_el: Option<u8>,
    ) -> SimResult<Result<u64, WalkFault>> {
        let regime = self.regime();
        if self.sctlr[regime as usize] & SCTLR_M == 0 {
            return Ok(Ok(va));
        }
        let e = match self.walk(bus, regime, va)? {
            Ok(e) => e,
            Err(f) => return Ok(Err(f)),
        };
        let el = as_el.unwrap_or(self.el);
        if !self.permitted(regime, el, &e, access) {
            // Permission faults are reported at the level that mapped it; the
            // walk does not keep the level, so report level 3 (page) - the
            // only level Zephyr's fault decoder distinguishes by class.
            return Ok(Err(WalkFault {
                fsc: FSC_PERMISSION | 3,
            }));
        }
        Ok(Ok((e.pa_page << 12) | (va & 0xFFF)))
    }

    /// `AT S1E{0,1,2,3}{R,W}`: result into `PAR_EL1`.
    pub(crate) fn address_translate(
        &mut self,
        bus: &mut dyn Bus,
        va: u64,
        as_el: u8,
        write: bool,
    ) -> SimResult<()> {
        let regime = if as_el <= 1 { 1 } else { as_el };
        if self.sctlr[regime as usize] & SCTLR_M == 0 {
            self.par_el1 = va & 0x0000_FFFF_FFFF_F000;
            return Ok(());
        }
        let access = if write { Access::Write } else { Access::Read };
        self.par_el1 = match self.walk(bus, regime, va)? {
            Ok(e) if self.permitted(regime, as_el, &e, access) => {
                let attr = (self.mair[regime as usize] >> (8 * e.attr_index)) & 0xFF;
                (attr << 56) | (e.pa_page << 12)
            }
            Ok(_) => 1 | (u64::from(FSC_PERMISSION | 3) << 1),
            Err(f) => 1 | (u64::from(f.fsc) << 1),
        };
        Ok(())
    }

    fn abort(&self, va: u64, f: WalkFault, access: Access) -> Exception {
        let (class, wnr) = match access {
            Access::Fetch => (ec::IABT_SAME, 0),
            Access::Write => (ec::DABT_SAME, 1 << 6),
            Access::Read => (ec::DABT_SAME, 0),
        };
        let mut e = Exception::new(class, f.fsc | wnr);
        e.far = Some(va);
        e
    }

    // ---- memory -----------------------------------------------------------

    fn phys_read(&mut self, bus: &mut dyn Bus, pa: u64, size: u64) -> SimResult<u64> {
        if let Some(off) = self.gic.dist_offset(pa) {
            return Ok(self.gic.dist_read(off, size, &self.timer));
        }
        if let Some(off) = self.gic.redist_offset(pa) {
            return Ok(self.gic.redist_read(off, size, &self.timer));
        }
        if pa % size != 0 {
            let mut v = 0u64;
            for i in 0..size {
                v |= u64::from(bus.read_u8(pa + i)?) << (8 * i);
            }
            return Ok(v);
        }
        Ok(match size {
            1 => u64::from(bus.read_u8(pa)?),
            2 => u64::from(bus.read_u16(pa)?),
            4 => u64::from(bus.read_u32(pa)?),
            _ => bus.read_u64(pa)?,
        })
    }

    fn phys_write(&mut self, bus: &mut dyn Bus, pa: u64, size: u64, value: u64) -> SimResult<()> {
        if let Some(off) = self.gic.dist_offset(pa) {
            self.gic.dist_write(off, size, value);
            return Ok(());
        }
        if let Some(off) = self.gic.redist_offset(pa) {
            self.gic.redist_write(off, size, value);
            return Ok(());
        }
        if pa % size != 0 {
            for i in 0..size {
                bus.write_u8(pa + i, (value >> (8 * i)) as u8)?;
            }
            return Ok(());
        }
        match size {
            1 => bus.write_u8(pa, value as u8),
            2 => bus.write_u16(pa, value as u16),
            4 => bus.write_u32(pa, value as u32),
            _ => bus.write_u64(pa, value),
        }
    }

    /// Virtual read of 1/2/4/8 bytes. An access crossing a page is split.
    pub(crate) fn read_va(
        &mut self,
        bus: &mut dyn Bus,
        va: u64,
        size: u64,
        as_el: Option<u8>,
    ) -> SimResult<Result<u64, Exception>> {
        if (va & 0xFFF) + size > 0x1000 {
            let mut v = 0u64;
            for i in 0..size {
                match self.read_va(bus, va.wrapping_add(i), 1, as_el)? {
                    Ok(b) => v |= b << (8 * i),
                    Err(e) => return Ok(Err(e)),
                }
            }
            return Ok(Ok(v));
        }
        match self.translate(bus, va, Access::Read, as_el)? {
            Ok(pa) => Ok(Ok(self.phys_read(bus, pa, size)?)),
            Err(f) => Ok(Err(self.abort(va, f, Access::Read))),
        }
    }

    pub(crate) fn write_va(
        &mut self,
        bus: &mut dyn Bus,
        va: u64,
        size: u64,
        value: u64,
        as_el: Option<u8>,
    ) -> SimResult<Result<(), Exception>> {
        if (va & 0xFFF) + size > 0x1000 {
            // Translate every byte first so a fault leaves memory unchanged.
            for i in 0..size {
                if let Err(f) = self.translate(bus, va.wrapping_add(i), Access::Write, as_el)? {
                    return Ok(Err(self.abort(va.wrapping_add(i), f, Access::Write)));
                }
            }
            for i in 0..size {
                let pa = self
                    .translate(bus, va.wrapping_add(i), Access::Write, as_el)?
                    .unwrap_or(va);
                self.phys_write(bus, pa, 1, value >> (8 * i))?;
            }
            return Ok(Ok(()));
        }
        match self.translate(bus, va, Access::Write, as_el)? {
            Ok(pa) => {
                self.phys_write(bus, pa, size, value)?;
                Ok(Ok(()))
            }
            Err(f) => Ok(Err(self.abort(va, f, Access::Write))),
        }
    }

    /// `DC ZVA`: zero the 64-byte block containing `va`.
    pub(crate) fn dc_zva(
        &mut self,
        bus: &mut dyn Bus,
        va: u64,
    ) -> SimResult<Result<(), Exception>> {
        let base = va & !(ZVA_BLOCK - 1);
        for i in (0..ZVA_BLOCK).step_by(8) {
            if let Err(e) = self.write_va(bus, base + i, 8, 0, None)? {
                return Ok(Err(e));
            }
        }
        Ok(Ok(()))
    }

    fn fetch(&mut self, bus: &mut dyn Bus) -> SimResult<Result<u32, Exception>> {
        let pc = self.pc;
        if pc & 3 != 0 {
            let mut e = Exception::new(ec::PC_ALIGN, 0);
            e.far = Some(pc);
            return Ok(Err(e));
        }
        match self.translate(bus, pc, Access::Fetch, None)? {
            Ok(pa) => Ok(Ok(bus.read_u32(pa)?)),
            Err(f) => Ok(Err(self.abort(pc, f, Access::Fetch))),
        }
    }

    // ---- system registers -------------------------------------------------

    /// Minimum EL for an `MRS`/`MSR` by its op1 field (Arm ARM D18.2).
    fn sysreg_min_el(key: u32) -> u8 {
        let op0 = key >> 14;
        let op1 = (key >> 11) & 7;
        match (op0, op1) {
            (3, 3) => 0,
            (_, 4) | (_, 5) => 2,
            (_, 6) => 3,
            _ => 1,
        }
    }

    /// EL0 access to op1=3 registers that `SCTLR_EL1`/`CNTKCTL_EL1` gate.
    fn el0_permitted(&self, key: u32) -> bool {
        use regs::*;
        match key {
            CTR_EL0 => self.sctlr[1] & (1 << 15) != 0,
            DCZID_EL0 | NZCV | FPCR | FPSR | TPIDR_EL0 | TPIDRRO_EL0 => true,
            DAIF => self.sctlr[1] & (1 << 9) != 0,
            CNTFRQ_EL0 | CNTPCT_EL0 => self.timer.cntkctl & 1 != 0,
            CNTVCT_EL0 => self.timer.cntkctl & 3 != 0,
            CNTV_CTL_EL0 | CNTV_CVAL_EL0 | CNTV_TVAL_EL0 => self.timer.cntkctl & (1 << 8) != 0,
            CNTP_CTL_EL0 | CNTP_CVAL_EL0 | CNTP_TVAL_EL0 => self.timer.cntkctl & (1 << 9) != 0,
            _ => false,
        }
    }

    pub(crate) fn sysreg_access_ok(&self, key: u32, write: bool) -> bool {
        use regs::*;
        let min = Self::sysreg_min_el(key);
        if self.el < min {
            return false;
        }
        if self.el == 0 && !self.el0_permitted(key) {
            return false;
        }
        // Read-only registers.
        if write
            && matches!(
                key,
                MIDR_EL1
                    | MPIDR_EL1
                    | REVIDR_EL1
                    | CURRENTEL
                    | CTR_EL0
                    | DCZID_EL0
                    | CLIDR_EL1
                    | CCSIDR_EL1
                    | ISR_EL1
                    | CNTPCT_EL0
                    | CNTVCT_EL0
                    | ICC_IAR0_EL1
                    | ICC_IAR1_EL1
                    | ICC_HPPIR0_EL1
                    | ICC_HPPIR1_EL1
                    | ICC_RPR_EL1
                    | RVBAR_EL3
            )
        {
            return false;
        }
        if !write
            && matches!(
                key,
                ICC_EOIR0_EL1 | ICC_EOIR1_EL1 | ICC_DIR_EL1 | ICC_SGI1R_EL1
            )
        {
            return false;
        }
        true
    }

    /// `MRS`. `None` = no such register (UNDEFINED).
    pub(crate) fn read_sysreg(&mut self, key: u32) -> Option<u64> {
        use regs::*;
        let op0 = key >> 14;
        let crn = (key >> 7) & 0xF;
        let crm = (key >> 3) & 0xF;
        let op1 = (key >> 11) & 7;
        Some(match key {
            MIDR_EL1 => MIDR_A53,
            MPIDR_EL1 => self.params.mpidr,
            REVIDR_EL1 => 0,
            ID_AA64PFR0_EL1 => ID_AA64PFR0,
            ID_AA64DFR0_EL1 => 0x1030_5106,
            // CRC32 (always present on the A53); no crypto, no LSE.
            ID_AA64ISAR0_EL1 => 0x0001_0000,
            ID_AA64MMFR0_EL1 => ID_AA64MMFR0,
            CCSIDR_EL1 => match self.csselr_el1 {
                // 32 KB 4-way L1 D, 32 KB 2-way L1 I, 512 KB 16-way L2,
                // 64-byte lines (A53 TRM 4.3.21, AM62x configuration).
                0 => 0x700F_E01A,
                1 => 0x201F_E00A,
                2 => 0x703F_E07A,
                _ => 0,
            },
            CLIDR_EL1 => CLIDR,
            CSSELR_EL1 => self.csselr_el1,
            CTR_EL0 => CTR,
            DCZID_EL0 => DCZID,
            SCTLR_EL1 => self.sctlr[1],
            SCTLR_EL2 => self.sctlr[2],
            SCTLR_EL3 => self.sctlr[3],
            CPACR_EL1 => self.cpacr_el1,
            CPTR_EL2 => self.cptr_el2,
            CPTR_EL3 => self.cptr_el3,
            HCR_EL2 => self.hcr_el2,
            SCR_EL3 => self.scr_el3,
            TTBR0_EL1 => self.ttbr0[1],
            TTBR1_EL1 => self.ttbr1_el1,
            TTBR0_EL2 => self.ttbr0[2],
            TTBR0_EL3 => self.ttbr0[3],
            TCR_EL1 => self.tcr[1],
            TCR_EL2 => self.tcr[2],
            TCR_EL3 => self.tcr[3],
            MAIR_EL1 => self.mair[1],
            MAIR_EL2 => self.mair[2],
            MAIR_EL3 => self.mair[3],
            SPSR_EL1 => self.spsr[1],
            SPSR_EL2 => self.spsr[2],
            SPSR_EL3 => self.spsr[3],
            ELR_EL1 => self.elr[1],
            ELR_EL2 => self.elr[2],
            ELR_EL3 => self.elr[3],
            ESR_EL1 => self.esr[1],
            ESR_EL2 => self.esr[2],
            ESR_EL3 => self.esr[3],
            FAR_EL1 => self.far[1],
            FAR_EL2 => self.far[2],
            FAR_EL3 => self.far[3],
            VBAR_EL1 => self.vbar[1],
            VBAR_EL2 => self.vbar[2],
            VBAR_EL3 => self.vbar[3],
            RVBAR_EL3 => 0,
            SP_EL0 => self.sp[0],
            SP_EL1 => self.sp[1],
            SP_EL2 => self.sp[2],
            SPSEL => u64::from(self.spsel),
            CURRENTEL => u64::from(self.el) << 2,
            NZCV => self.nzcv,
            DAIF => self.daif,
            FPCR => self.fpcr,
            FPSR => self.fpsr,
            PAR_EL1 => self.par_el1,
            ISR_EL1 => u64::from(self.irq_line()) << 7,
            CNTFRQ_EL0 => self.timer.cntfrq,
            CNTPCT_EL0 => self.timer.count,
            CNTVCT_EL0 => self.timer.virtual_count(),
            CNTKCTL_EL1 => self.timer.cntkctl,
            CNTHCTL_EL2 => self.timer.cnthctl,
            CNTVOFF_EL2 => self.timer.cntvoff,
            CNTP_CTL_EL0 => self.timer.phys.read_ctl(self.timer.count),
            CNTP_CVAL_EL0 => self.timer.phys.cval,
            CNTP_TVAL_EL0 => self.timer.phys.read_tval(self.timer.count),
            CNTV_CTL_EL0 => self.timer.virt.read_ctl(self.timer.virtual_count()),
            CNTV_CVAL_EL0 => self.timer.virt.cval,
            CNTV_TVAL_EL0 => self.timer.virt.read_tval(self.timer.virtual_count()),
            CNTHP_CTL_EL2 => self.timer.hyp.read_ctl(self.timer.count),
            CNTHP_CVAL_EL2 => self.timer.hyp.cval,
            CNTHP_TVAL_EL2 => self.timer.hyp.read_tval(self.timer.count),
            CNTPS_CTL_EL1 => self.timer.sec_phys.read_ctl(self.timer.count),
            CNTPS_CVAL_EL1 => self.timer.sec_phys.cval,
            CNTPS_TVAL_EL1 => self.timer.sec_phys.read_tval(self.timer.count),
            ICC_PMR_EL1 => u64::from(self.gic.pmr),
            ICC_IAR1_EL1 | ICC_IAR0_EL1 => u64::from(self.gic.acknowledge(&self.timer)),
            ICC_HPPIR1_EL1 | ICC_HPPIR0_EL1 => u64::from(
                self.gic
                    .highest_pending(&self.timer)
                    .map_or(super::aarch64_gic::SPURIOUS, |(id, _)| id),
            ),
            ICC_RPR_EL1 => u64::from(self.gic.running_priority()),
            ICC_BPR1_EL1 | ICC_BPR0_EL1 => u64::from(self.gic.bpr1),
            // PRIbits = 5 (32 levels), IDbits = 16-bit capable is 0 (16 bits
            // not supported), A3V.
            ICC_CTLR_EL1 => self.gic.icc_ctlr | (4 << 8) | (1 << 15),
            ICC_CTLR_EL3 => self.gic.icc_ctlr | (4 << 8) | (1 << 15),
            ICC_SRE_EL1 => self.gic.sre_el1 | 0b110,
            ICC_SRE_EL2 => self.gic.sre_el2 | 0b110 | (1 << 3),
            ICC_SRE_EL3 => self.gic.sre_el3 | 0b110 | (1 << 3),
            ICC_IGRPEN0_EL1 => u64::from(self.gic.igrpen0),
            ICC_IGRPEN1_EL1 | ICC_IGRPEN1_EL3 => u64::from(self.gic.igrpen1),
            k if PLAIN_REGS.contains(&k) => self.plain.get(&k).copied().unwrap_or(0),
            // Remaining ID space (AArch32 ID registers, unallocated ID regs)
            // is RAZ (Arm ARM D18.3.1).
            _ if op0 == 3 && op1 == 0 && crn == 0 && crm >= 1 => 0,
            // ICC active-priority registers: RAZ (no preemption groups).
            _ if op0 == 3 && op1 == 0 && crn == 12 && (crm == 8 || crm == 9) => 0,
            // Debug (op0 = 2) and PMU (CRn = 9) registers: not modelled, RAZ.
            _ if op0 == 2 || (op0 == 3 && crn == 9) => 0,
            _ => return None,
        })
    }

    /// `MSR`. `false` = no such register (UNDEFINED).
    pub(crate) fn write_sysreg(&mut self, key: u32, v: u64) -> bool {
        use regs::*;
        let op0 = key >> 14;
        let crn = (key >> 7) & 0xF;
        let crm = (key >> 3) & 0xF;
        let op1 = (key >> 11) & 7;
        match key {
            SCTLR_EL1 | SCTLR_EL2 | SCTLR_EL3 => {
                let el = match key {
                    SCTLR_EL1 => 1,
                    SCTLR_EL2 => 2,
                    _ => 3,
                };
                self.sctlr[el] = v;
                self.flush_tlb();
            }
            CPACR_EL1 => self.cpacr_el1 = v & (3 << 20),
            CPTR_EL2 => self.cptr_el2 = v,
            CPTR_EL3 => self.cptr_el3 = v,
            HCR_EL2 => self.hcr_el2 = v | (1 << 31), // RW: EL1 is AArch64
            SCR_EL3 => self.scr_el3 = v | (1 << 10), // RW: lower ELs AArch64
            TTBR0_EL1 => {
                self.ttbr0[1] = v;
                self.flush_tlb();
            }
            TTBR1_EL1 => {
                self.ttbr1_el1 = v;
                self.flush_tlb();
            }
            TTBR0_EL2 => {
                self.ttbr0[2] = v;
                self.flush_tlb();
            }
            TTBR0_EL3 => {
                self.ttbr0[3] = v;
                self.flush_tlb();
            }
            TCR_EL1 => {
                self.tcr[1] = v;
                self.flush_tlb();
            }
            TCR_EL2 => {
                self.tcr[2] = v;
                self.flush_tlb();
            }
            TCR_EL3 => {
                self.tcr[3] = v;
                self.flush_tlb();
            }
            MAIR_EL1 => self.mair[1] = v,
            MAIR_EL2 => self.mair[2] = v,
            MAIR_EL3 => self.mair[3] = v,
            SPSR_EL1 => self.spsr[1] = v,
            SPSR_EL2 => self.spsr[2] = v,
            SPSR_EL3 => self.spsr[3] = v,
            ELR_EL1 => self.elr[1] = v,
            ELR_EL2 => self.elr[2] = v,
            ELR_EL3 => self.elr[3] = v,
            ESR_EL1 => self.esr[1] = v,
            ESR_EL2 => self.esr[2] = v,
            ESR_EL3 => self.esr[3] = v,
            FAR_EL1 => self.far[1] = v,
            FAR_EL2 => self.far[2] = v,
            FAR_EL3 => self.far[3] = v,
            VBAR_EL1 => self.vbar[1] = v & !0x7FF,
            VBAR_EL2 => self.vbar[2] = v & !0x7FF,
            VBAR_EL3 => self.vbar[3] = v & !0x7FF,
            SP_EL0 => self.sp[0] = v,
            SP_EL1 => self.sp[1] = v,
            SP_EL2 => self.sp[2] = v,
            NZCV => self.nzcv = v & 0xF000_0000,
            DAIF => self.daif = v & DAIF_MASK,
            FPCR => self.fpcr = v & 0x07FF_9F00,
            FPSR => self.fpsr = v & 0xF800_009F,
            PAR_EL1 => self.par_el1 = v,
            CSSELR_EL1 => self.csselr_el1 = v & 0xF,
            CNTFRQ_EL0 => {
                // Writable only at the highest EL (EL3 here).
                if self.el == 3 {
                    self.timer.cntfrq = v & 0xFFFF_FFFF;
                }
            }
            CNTKCTL_EL1 => self.timer.cntkctl = v,
            CNTHCTL_EL2 => self.timer.cnthctl = v,
            CNTVOFF_EL2 => self.timer.cntvoff = v,
            CNTP_CTL_EL0 => self.timer.phys.ctl = v & 3,
            CNTP_CVAL_EL0 => self.timer.phys.cval = v,
            CNTP_TVAL_EL0 => self.timer.phys.write_tval(self.timer.count, v),
            CNTV_CTL_EL0 => self.timer.virt.ctl = v & 3,
            CNTV_CVAL_EL0 => self.timer.virt.cval = v,
            CNTV_TVAL_EL0 => {
                let c = self.timer.virtual_count();
                self.timer.virt.write_tval(c, v)
            }
            CNTHP_CTL_EL2 => self.timer.hyp.ctl = v & 3,
            CNTHP_CVAL_EL2 => self.timer.hyp.cval = v,
            CNTHP_TVAL_EL2 => self.timer.hyp.write_tval(self.timer.count, v),
            CNTPS_CTL_EL1 => self.timer.sec_phys.ctl = v & 3,
            CNTPS_CVAL_EL1 => self.timer.sec_phys.cval = v,
            CNTPS_TVAL_EL1 => self.timer.sec_phys.write_tval(self.timer.count, v),
            ICC_PMR_EL1 => self.gic.pmr = v as u8,
            ICC_EOIR1_EL1 | ICC_EOIR0_EL1 => self.gic.end_of_interrupt(v as u32 & 0xFF_FFFF),
            // EOImode 1 is not modelled, so DIR has nothing left to do.
            ICC_DIR_EL1 => {}
            ICC_SGI1R_EL1 => self.gic.write_sgi1r(v),
            ICC_BPR1_EL1 | ICC_BPR0_EL1 => self.gic.bpr1 = (v & 7) as u8,
            ICC_CTLR_EL1 | ICC_CTLR_EL3 => self.gic.icc_ctlr = v & 0x3,
            ICC_SRE_EL1 => self.gic.sre_el1 = 1,
            ICC_SRE_EL2 => self.gic.sre_el2 = 1,
            ICC_SRE_EL3 => self.gic.sre_el3 = 1,
            ICC_IGRPEN0_EL1 => self.gic.igrpen0 = v & 1 != 0,
            ICC_IGRPEN1_EL1 | ICC_IGRPEN1_EL3 => self.gic.igrpen1 = v & 1 != 0,
            k if PLAIN_REGS.contains(&k) => {
                self.plain.insert(k, v);
            }
            _ if op0 == 3 && op1 == 0 && crn == 12 && (crm == 8 || crm == 9) => {}
            _ if op0 == 2 || (op0 == 3 && crn == 9) => {}
            _ => return false,
        }
        true
    }

    /// X0-X30, PSTATE, then the standard SP, PC trailer
    /// ([`crate::SP_FROM_END`], [`crate::PC_FROM_END`]).
    fn observer_registers(&self) -> [u32; 34] {
        let mut regs = [0u32; 34];
        for (dst, src) in regs.iter_mut().zip(self.x.iter()) {
            *dst = *src as u32;
        }
        regs[31] = self.pstate() as u32;
        regs[32] = self.sp[self.sp_index()] as u32;
        regs[33] = self.pc as u32;
        regs
    }

    fn interrupt_pending(&self) -> bool {
        self.irq_target().is_some() && self.irq_line()
    }
}

impl Cpu for Aarch64 {
    fn reset(&mut self, _bus: &mut dyn Bus) -> SimResult<()> {
        let pc = self.pc;
        *self = Self::new(self.params);
        self.pc = pc;
        Ok(())
    }

    fn uses_elf_entry_point(&self) -> bool {
        true
    }

    fn step(
        &mut self,
        bus: &mut dyn Bus,
        observers: &[Arc<dyn SimulationObserver>],
        _config: &crate::SimulationConfig,
    ) -> SimResult<()> {
        self.waiting_for_interrupt = false;
        let pc = self.pc;
        let fetched = self.fetch(bus)?;
        let inst = *fetched.as_ref().unwrap_or(&0);
        for o in observers {
            o.on_step_start(pc as u32, inst);
        }
        let flow = match fetched {
            Ok(inst) => self.exec(bus, inst)?,
            Err(e) => Flow::Exc(e),
        };
        match flow {
            Flow::Next => self.pc = pc.wrapping_add(4),
            Flow::Jump(t) => self.pc = t,
            Flow::Exc(e) => {
                if e.ec == ec::UNKNOWN {
                    // The guest sees an architectural trap; the host log line
                    // is what `scripts/unsupported_instruction_audit.sh` counts.
                    tracing::warn!("Unknown instruction {:#010x} at {:#x}", inst, pc);
                }
                self.take_sync(e, pc);
            }
        }
        self.cycles = self.cycles.wrapping_add(1);
        self.timer.advance(1);
        if !observers.is_empty() {
            crate::emit_trace_event(
                observers,
                labwired_hw_trace::TraceEvent::InstructionRetired {
                    pc: pc as u32,
                    opcode: inst,
                },
            );
        }
        if self.interrupt_pending() {
            if let Some(target) = self.irq_target() {
                self.enter_exception(target, 0x80, self.pc, None, None);
            }
        }
        if !observers.is_empty() {
            let regs = self.observer_registers();
            for o in observers {
                o.on_step_end(1, &regs);
            }
        }
        Ok(())
    }

    fn set_pc(&mut self, val: u32) {
        self.pc = u64::from(val);
    }
    fn get_pc(&self) -> u32 {
        self.pc as u32
    }
    fn set_sp(&mut self, val: u32) {
        let i = self.sp_index();
        self.sp[i] = u64::from(val);
    }
    fn set_exception_pending(&mut self, exception_num: u32) {
        // A peripheral line, by GIC INTID.
        self.gic.set_pending(exception_num);
    }

    fn idle_fast_forward_budget(&self, _bus: &dyn Bus) -> Option<u64> {
        // WFI wakes on a pending interrupt even when PSTATE masks it.
        if !self.waiting_for_interrupt || self.irq_line() {
            return None;
        }
        match self.timer.cycles_until_line() {
            None => Some(u64::MAX),
            Some(c) => Some(c.saturating_sub(1)).filter(|c| *c > 0),
        }
    }

    fn fast_forward_idle_cycles(&mut self, cycles: u64) {
        self.cycles = self.cycles.wrapping_add(cycles);
        self.timer.advance(cycles);
    }

    fn get_register(&self, id: u8) -> u32 {
        match id {
            0..=30 => self.x[id as usize] as u32,
            31 => self.sp[self.sp_index()] as u32,
            32 => self.pc as u32,
            33 => self.pstate() as u32,
            _ => 0,
        }
    }
    fn set_register(&mut self, id: u8, val: u32) {
        match id {
            0..=30 => self.x[id as usize] = u64::from(val),
            31 => {
                let i = self.sp_index();
                self.sp[i] = u64::from(val)
            }
            32 => self.pc = u64::from(val),
            _ => {}
        }
    }

    fn snapshot(&self) -> crate::snapshot::CpuSnapshot {
        let mut plain: Vec<(u32, u64)> = self.plain.iter().map(|(k, v)| (*k, *v)).collect();
        plain.sort_unstable();
        crate::snapshot::CpuSnapshot::Aarch64(Box::new(Aarch64CpuSnapshot {
            x: self.x.to_vec(),
            sp: self.sp.to_vec(),
            pc: self.pc,
            nzcv: self.nzcv,
            daif: self.daif,
            el: self.el,
            spsel: self.spsel,
            v: self
                .v
                .iter()
                .map(|v| (*v as u64, (*v >> 64) as u64))
                .collect(),
            fpcr: self.fpcr,
            fpsr: self.fpsr,
            sctlr: self.sctlr.to_vec(),
            vbar: self.vbar.to_vec(),
            elr: self.elr.to_vec(),
            spsr: self.spsr.to_vec(),
            esr: self.esr.to_vec(),
            far: self.far.to_vec(),
            tcr: self.tcr.to_vec(),
            ttbr0: self.ttbr0.to_vec(),
            mair: self.mair.to_vec(),
            ttbr1_el1: self.ttbr1_el1,
            cpacr_el1: self.cpacr_el1,
            cptr_el2: self.cptr_el2,
            cptr_el3: self.cptr_el3,
            hcr_el2: self.hcr_el2,
            scr_el3: self.scr_el3,
            par_el1: self.par_el1,
            csselr_el1: self.csselr_el1,
            plain,
            timer: self.timer,
            gic: self.gic.clone(),
            cycles: self.cycles,
        }))
    }

    fn apply_snapshot(&mut self, snapshot: &crate::snapshot::CpuSnapshot) {
        let crate::snapshot::CpuSnapshot::Aarch64(s) = snapshot else {
            return;
        };
        let copy = |dst: &mut [u64], src: &[u64]| {
            for (d, s) in dst.iter_mut().zip(src.iter()) {
                *d = *s;
            }
        };
        copy(&mut self.x, &s.x);
        copy(&mut self.sp, &s.sp);
        self.pc = s.pc;
        self.nzcv = s.nzcv & 0xF000_0000;
        self.daif = s.daif & DAIF_MASK;
        self.el = s.el.min(3);
        self.spsel = s.spsel;
        for (d, (lo, hi)) in self.v.iter_mut().zip(s.v.iter()) {
            *d = u128::from(*lo) | (u128::from(*hi) << 64);
        }
        self.fpcr = s.fpcr;
        self.fpsr = s.fpsr;
        copy(&mut self.sctlr, &s.sctlr);
        copy(&mut self.vbar, &s.vbar);
        copy(&mut self.elr, &s.elr);
        copy(&mut self.spsr, &s.spsr);
        copy(&mut self.esr, &s.esr);
        copy(&mut self.far, &s.far);
        copy(&mut self.tcr, &s.tcr);
        copy(&mut self.ttbr0, &s.ttbr0);
        copy(&mut self.mair, &s.mair);
        self.ttbr1_el1 = s.ttbr1_el1;
        self.cpacr_el1 = s.cpacr_el1;
        self.cptr_el2 = s.cptr_el2;
        self.cptr_el3 = s.cptr_el3;
        self.hcr_el2 = s.hcr_el2;
        self.scr_el3 = s.scr_el3;
        self.par_el1 = s.par_el1;
        self.csselr_el1 = s.csselr_el1;
        self.plain = s.plain.iter().copied().collect();
        self.timer = s.timer;
        self.gic = s.gic.clone();
        self.gic.any_latched = self.gic.pending.iter().any(|p| *p);
        self.cycles = s.cycles;
        self.exclusive = None;
        self.flush_tlb();
    }

    fn get_register_names(&self) -> Vec<String> {
        let mut names: Vec<String> = (0..31).map(|i| format!("x{i}")).collect();
        names.push("sp".to_string());
        names.push("pc".to_string());
        names.push("cpsr".to_string());
        names
    }

    fn index_of_register(&self, name: &str) -> Option<u8> {
        let n = name.to_ascii_lowercase();
        if let Some(i) = n.strip_prefix('x') {
            i.parse().ok().filter(|i: &u8| *i < 31)
        } else {
            match n.as_str() {
                "sp" => Some(31),
                "pc" => Some(32),
                "cpsr" | "pstate" => Some(33),
                _ => None,
            }
        }
    }

    fn as_any_mut(&mut self) -> Option<&mut dyn std::any::Any> {
        Some(self)
    }
}

#[cfg(test)]
#[path = "aarch64_tests.rs"]
mod tests;
