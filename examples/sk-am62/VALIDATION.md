# Validation — SK-AM62 (TI AM6234, Cortex-A53)

Claims, evidence and what is explicitly **not** claimed. **SIM-DERIVED**:
there is no AM62x bench part, and no QEMU or Renode run was diffed against.

## Tier status

| Capability | Tier | Evidence |
| :--- | :--- | :--- |
| A64 data processing: add/sub (imm, shifted, extended) with flags, logical immediates (DecodeBitMasks), bitfield/extract, conditional select/compare, 1/2/3-source incl. SMULH/UMULH/divide-by-zero | unit + firmware | `aarch64_tests::data_processing_matches_the_pseudocode` (encodings from GNU as); every Zephyr image |
| Loads/stores in every addressing form, pairs, LDRSW/LDURSB, register offset with extend, LDXR/STXR/LDAXR/STLXR/CLREX monitor | unit + firmware | `loads_stores_and_the_exclusive_monitor`; the `atomic` suite through libgcc's outline atomics |
| Branches (B/BL/BR/BLR/RET, CBZ/TBNZ, B.cond) | unit + firmware | `branches_link_and_return` |
| Exception model: SVC/HVC/SMC/BRK/UNDEFINED, ESR/ELR/SPSR/FAR, vector offsets, ERET (illegal returns trapped), DAIF, SPSel | unit + firmware | `svc_and_undefined_…`, `eret_from_el3_…`; Zephyr's EL3 → EL1 reset path, `irq_offload` (SVC from EL1), syscalls (SVC from EL0) |
| FP/SIMD: V registers, q/d/s loads/stores, FMOV, CNT/ADDV/ADD, CPACR/CPTR FP access traps | unit + firmware | `fp_instructions_trap_until_cpacr_enables_them`; `bitarray` (popcount) — Zephyr's lazy FP sharing takes EC 0x07 from EL0 and EL1 |
| Stage-1 MMU (4 KB granule, levels 1-3), AF, AP[2:1]/PXN/UXN, TLBI, AT S1E*/PAR_EL1 | unit + firmware | `mmu_translates_and_raises_permission_faults`, `at_s1e1r_…`; Zephyr enables the MMU and runs user threads on per-thread tables (≈10 k ERETs to EL0 in kernel/common) |
| GICv3: distributor/redistributor MMIO, IAR1/EOIR1/PMR/RPR, level timer PPIs | unit + firmware | `aarch64_gic` tests, `virtual_timer_irq_…`; `synchronization` and `clock` |
| Generic timer (virtual/physical/EL2/secure channels at 200 MHz) and WFI idle fast-forward | unit + firmware | `wfi_fast_forwards_to_the_timer_compare`; hello idles 59.94 M of 60 M cycles |
| Instruction support for the three images | audited | `unsupported_instruction_audit.sh`, 2,000,000 steps each, 0 unsupported; a UDF patched into `main` reports 1 |
| Negative control | checked | CNT counting zero bits instead of one bits: `bitarray` fails 5/8, kernel-common FAILs |
| Determinism | checked | fixtures rebuild byte-identically (`build.sh`, sha256 in its output) |

## Not claimed

- **Cores 1-3** (SMP), the **M4F** and **R5F**, DMSC/TIFS, and the boot flow
  (TF-A, U-Boot): an ELF is entered at EL3 directly.
- **ISA**: Armv8.1+ (LSE atomics, PAN, ...) and crypto are UNDEFINED, as on
  an A53 without the crypto extension (`ID_AA64ISAR0_EL1` reports only CRC32,
  which every A53 has). SIMD/FP arithmetic beyond
  FMOV/CNT/ADDV/ADD/ORR/DUP/UMOV/INS is UNDEFINED (logged and trapped, never
  guessed). AArch32 at any EL.
- **MMU**: stage 2, 16/64 KB granules, the hierarchical APTable/XNTable
  controls, ASID tagging (every TTBR/TCR/SCTLR write and TLBI flushes),
  alignment faults (`SCTLR.A`) and SP alignment checks. Permission faults
  report level 3 whatever level mapped the address.
- **GIC**: groups and security are storage (every enabled interrupt is an
  IRQ through IAR1), no binary-point preemption subgroups, EOImode 1, LPIs/ITS.
- **UART**: polled TX/RX; the SPI is wired but Zephyr's console does not
  enable it. Pad configuration and the mailbox are declared stubs.
- Caches (maintenance instructions are accepted), debug and PMU registers
  (RAZ/WI). Timing: one instruction is one CPU clock.
