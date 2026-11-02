# Validation — EK-RX261 (R5F52618BGFP)

Claims, evidence and what is explicitly **not** claimed. **SIM-DERIVED**: there
is no RX261 bench part, and no Renode or QEMU run was diffed against (QEMU's
RX target is an RXv1 RX62N; Renode has no RX).

## Tier status

| Capability | Tier | Evidence |
| :--- | :--- | :--- |
| RXv1 integer ISA, every operand form (incl. the `06` memory-extension prefix, size-scaled displacements, `[Ri,Rb]`, `[Rn+]`/`[-Rn]`) | unit + firmware | `rx_tests` (encodings from `rx-zephyr-elf-as --misa=v3`); every Zephyr image |
| Flags: C = no borrow after SUB/CMP, O on ADD/NEG/ABS/DIV, SHLL overflow rule | unit + firmware | `compare_carry_means_no_borrow`, `add_sets_overflow_and_carry`, `shift_flags`, `division_by_zero_…` |
| RXv2/v3 additions: 3-operand XOR/FADD/FSUB/FMUL, STZ/STNZ Rs, MOVLI/MOVCO, BFMOV/BFMOVZ | unit | `movco_succeeds_once_after_movli`, `bit_field_moves`, `three_operand_sub_is_rs2_minus_rs` |
| DSP: A0/A1 72-bit accumulators, MACHI/EMACA family, MVTAC*/MVFAC* with `#imm`, RACW/RACL saturation | unit + firmware | `accumulator_moves_…`, `racw_saturates_…`; Zephyr's ISR wrappers save/restore A0 |
| Single-precision FPU (FADD/FSUB/FMUL/FDIV/FCMP/FTOI/ROUND/ITOF/UTOF/FTOU/FSQRT) | unit | `single_precision_fpu` |
| String ops (SMOVF/SMOVB/SMOVU/SSTR/SCMPU/SUNTIL/SWHILE), RMPA | unit + firmware | `smovf_copies_and_sstr_fills`, `scmpu_…`; Zephyr's reset copy and context switch use SMOVF/SMOVB/SSTR |
| PSW.U stack switching (USP/ISP), INT/BRK/RTE, undefined-instruction exception via EXTB | unit + firmware | `int_switches_to_the_interrupt_stack_and_rte_returns`, `undefined_instruction_…`; `INT #1` thread switches |
| ICU: IR/IER/IPR (RX261 low-vector IPR sharing), IPL masking, IR cleared on acceptance, SWINTR, fast interrupt | unit + firmware | `icu_takes_the_highest_enabled_request_above_ipl`, `swintr_raises_vector_27`; CMT0 tick on vector 28 |
| CMT: PCLKB/8..512 count, compare-match clear + CMI, CMSTR start/stop, one live scheduler wake | unit + firmware | `rx_cmt` tests (both scheduler modes); `synchronization`, `clock`, `common_1cpu` suites |
| SYSTEM: OSCOVFSR from the stop bits, OPCCR transition, USB/CANFD clock-switch handshake; FLASH 0xAA-keyed MEMWAITR | unit + firmware | `rx_sysc` tests; RDP `mcu_clock_setup` completes |
| WAIT idle fast-forward (pending-but-masked requests do not block it) | firmware | hello idles 63.96 M of 64 M cycles |
| Instruction support for the three images | audited | `unsupported_instruction_audit.sh`: hello to its end (11,570), sync and kernel/common 2,000,000 each, 0 unsupported; an `FF FF FF` patched into `main` reports 1 |
| Negative controls | checked | SUB carry inverted: nothing boots (both scripts FAIL). CMT compare-match never requesting: hello still passes, sync stops after the first greeting, kernel/common hangs in `boot_delay` |
| Determinism | checked | fixtures rebuild byte-identically (`build.sh`, sha256 in its output) |

## Not claimed

- **Privilege**: user mode (`PSW.PM` = 1) runs supervisor-only instructions
  without the privileged-instruction exception. Zephyr runs every thread in
  supervisor mode.
- **RXv3 register banks** (`SAVE`/`RSTR`) and the **double-precision FPU**:
  their opcodes are undefined instructions. The RX261 has no DFPU.
- **FPU fidelity**: only round-to-nearest; `FPSW` is storage (no FP
  exceptions, cause/flag bits never set, denormals not flushed).
- **ICU**: DTC transfers, group interrupts, IRQ pin edge detection (`IRQCRn`
  is storage), NMI; every request is edge-detected.
- **SCI6**: polled TX/RX only; TXI/RXI/TEI/ERI are never requested.
- **Ports/MPC, BSC, DTC, the reset-status block**: declared stubs. FLASH
  programming/erase is not modelled (only the control registers are stored).
- **Clocks**: no rate is derived from SCKCR/PLL settings; `cpu_hz` is the
  64 MHz ICLK Zephyr's board configures and the CMT counts PCLKB = 32 MHz.
- Timing: one instruction is one CPU clock.
