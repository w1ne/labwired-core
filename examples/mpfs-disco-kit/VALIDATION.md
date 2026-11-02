# Validation — PolarFire SoC Discovery Kit (MPFS095T)

Claims, evidence and what is explicitly **not** claimed. **SIM-DERIVED**: there
is no MPFS bench part, and no Renode or QEMU run was diffed against.

## Tier status

| Capability | Tier | Evidence |
| :--- | :--- | :--- |
| RV64I incl. `*W` ops, 6-bit shifts, `LD`/`SD`/`LWU` | unit + firmware | `riscv64_tests` (hand-encoded, spec values); every Zephyr image |
| M extension incl. division by zero / overflow results | unit + firmware | `m_extension_corner_cases_follow_the_spec_table`; `byteorder`, `clock` suites |
| A extension: LR/SC (reservation broken by a store), AMOs `.W`/`.D` | unit + firmware | `lr_sc_pair_…`, `amos_return_the_old_value_…`; `atomic` suite 3/3 |
| RV64C | unit + firmware | `compressed_rv64_forms`; every image is RVC |
| M-mode traps: ECALL, illegal instruction (`mtval` = bits), misaligned load/store, MRET | unit + firmware | `misaligned_load_traps…`, `time_csr_and_fp_opcodes_are_illegal…`; `irq_offload` suite (ECALL path) |
| CLINT `mtime`/`mtimecmp`/`msip`, MTI vectored/direct, `mepc` = next instruction | unit + firmware | `clint_timer_interrupt_vectors_and_mret_restores_mie`; `synchronization`, `clock`, `common_1cpu` suites |
| WFI idle fast-forward to the timer compare | unit + firmware | `wfi_idle_budget_runs_to_the_timer_compare`; hello idles at 0.07 s wall |
| MMUART 16550: LCR.DLAB selects DLL/DLM (divisor never reaches THR) | unit + firmware | `uart16550_dlab_routes_divisor_writes_to_the_latches`; console output starts clean |
| Instruction support for the three images | audited | `unsupported_instruction_audit.sh`: 2,000,000 instructions each, 0 unsupported; a patched FP opcode reports 1 |
| Negative control | checked | AMOs returning the new value: `atomic` suite FAIL 2/3, `PROJECT EXECUTION FAILED` |
| Determinism | checked | fixtures rebuild byte-identically (`build.sh`, sha256 in its output) |

## Not claimed

- **F/D extensions**: any FP opcode is an illegal instruction. The Zephyr
  images are built with `CONFIG_FPU=n` (soft-float ABI).
- **S/U modes, Sv39 MMU, `satp`, delegation**: absent; `misa` reports only
  I, M, A, C, and `mstatus` keeps MPP = M.
- **PLIC**: declared stub; `MEIP` is never set, so no peripheral interrupt
  reaches the hart (Zephyr's console is polled).
- **Other harts**: the E51 and U54 2-4 do not exist; their CLINT `msip` /
  `mtimecmp` words read zero.
- **PMP**: 16 entries are WARL storage only, never enforced.
- **`time` CSR**: traps, as on the U54 (no hardware `rdtime`).
- `mvendorid`/`marchid`/`mimpid` read 0.
- Boot: images are loaded straight into the DDR window at the ELF entry; the
  HSS, eNVM boot and DDR training are not modelled.
