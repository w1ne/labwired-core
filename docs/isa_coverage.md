# ISA Coverage

This document is the **source of truth** for what instructions LabWired Core
actually implements. README claims must match this matrix.

Last audit: v0.11.0 (2026-04).

Convention:
- ✅ **Decoded + executed** — tested path exists.
- 🟡 **Decoded, execute stubbed** — decoder recognises the opcode but the
  executor is incomplete or unverified.
- ❌ **Not implemented** — decoder returns `Instruction::Unknown`, which will
  surface as `SimulationError::DecodeError` at runtime.

---

## ARM Cortex-M (Thumb / Thumb-2)

Target subset today: **ARMv6-M core + selected ARMv7-M data-processing
and bit-field instructions.**

We do **not** yet claim ARMv7E-M (Cortex-M4/M7) compliance. FPU (VFPv4),
DSP extension, and saturating arithmetic are not implemented; attempting
to execute them raises `DecodeError`.

### Implemented Thumb-16 / Thumb-2 ops

| Category        | Instructions                                                                 |
|-----------------|------------------------------------------------------------------------------|
| Data-proc imm   | `MOV #imm`, `ADD #imm3`, `ADD #imm8`, `SUB #imm3`, `SUB #imm8`, `CMP #imm`   |
| Data-proc reg   | `ADD Rd,Rn,Rm`, `SUB Rd,Rn,Rm`, `ADD Rd,Rm (hi)`, `MOV Rd,Rm`, `CMP Rd,Rm`   |
| Logical         | `AND`, `ORR`, `EOR`, `MVN`, `MUL` (32×32→lo32), `RSBS`                       |
| Shifts          | `LSL #imm`, `LSR #imm`, `ASR #imm`, `ASR Rd,Rm`                              |
| Stack/SP arith  | `ADD SP,#imm`, `SUB SP,#imm`, `ADD Rd,SP,#imm`, `ADR Rd,#imm`                |
| Loads           | `LDR imm`, `LDR reg`, `LDR literal`, `LDR [SP,#imm]`, `LDRB`, `LDRH`         |
| Stores          | `STR imm`, `STR [SP,#imm]`, `STRB`, `STRH`                                   |
| Multi-reg       | `PUSH`, `POP`, `LDM`, `STM`                                                  |
| Branches        | `B`, `B<cond>`, `BL`, `BX`, `CBZ`, `CBNZ`                                    |
| Control         | `NOP`, `CPSIE i`, `CPSID i`                                                  |
| Sign/zero ext.  | `UXTB`                                                                       |
| Thumb-2 bitops  | `BFI`, `BFC`, `SBFX`, `UBFX`, `CLZ`, `RBIT`, `REV`, `REV16`, `REVSH`         |
| Thumb-2 data    | `DataProc32` cluster, `MOVW`, `MOVT`                                         |
| Barriers        | `DMB`, `DSB`, `ISB` — decoded; no-ops in our single-threaded sim            |
| System regs     | `MSR` / `MRS` for PRIMASK (SYSm=0x10); other SYSm accepted but unmodeled    |
| Wide multiply   | `SMULL`, `UMULL`, `SMLAL`, `UMLAL` — 32×32 → 64-bit multiply/accumulate      |

### Known gaps (ARMv7-M that we do **not** yet implement)

| Category         | Missing                                                                     |
|------------------|-----------------------------------------------------------------------------|
| Wide multiply    | `MLA`, `MLS` (`SMULL`/`UMULL`/`SMLAL`/`UMLAL` implemented — see above)       |
| Integer divide   | `SDIV`, `UDIV`                                                              |
| Saturating arith | `QADD`, `QSUB`, `SSAT`, `USAT`                                              |
| Sign/zero ext.   | `SXTB`, `SXTH`, `UXTH`                                                      |
| ARMv7E-M DSP     | `SMLAD`, `SMUAD`, packed SIMD family — entire DSP extension                 |
| FPU (VFPv4)      | `VLDR`, `VSTR`, `VMOV`, `VADD`, `VMUL`, `VSQRT`, … — entire FPU             |
| Exclusives       | `LDREX`, `STREX`, `CLREX`                                                   |
| TT / security    | `TT`, `TTA`, ARMv8-M security extensions                                    |

**Interrupt model:** NVIC is implemented via the `nvic` peripheral; VTOR
relocation and exception entry/exit work. Faults (HardFault, MemManage,
BusFault, UsageFault) are **not** raised on invalid instructions — invalid
opcodes bubble up as `SimulationError::DecodeError`.

---

## RISC-V

Target subset today: **RV32IM base ISA + Zicsr** (`rv32im_zicsr`).

| Category     | Instructions                                                                 |
|--------------|------------------------------------------------------------------------------|
| U-type       | `LUI`, `AUIPC`                                                               |
| J/B-type     | `JAL`, `JALR`, `BEQ`, `BNE`, `BLT`, `BGE`, `BLTU`, `BGEU`                    |
| Loads        | `LB`, `LH`, `LW`, `LBU`, `LHU`                                               |
| Stores       | `SB`, `SH`, `SW`                                                             |
| I-type ALU   | `ADDI`, `SLTI`, `SLTIU`, `XORI`, `ORI`, `ANDI`, `SLLI`, `SRLI`, `SRAI`       |
| R-type ALU   | `ADD`, `SUB`, `SLL`, `SLT`, `SLTU`, `XOR`, `SRL`, `SRA`, `OR`, `AND`         |
| **M** ext.   | `MUL`, `MULH`, `MULHSU`, `MULHU`, `DIV`, `DIVU`, `REM`, `REMU` (with full per-spec semantics for div-by-zero and INT_MIN/-1 overflow) |
| **A** ext.   | `LR.W`, `SC.W`, `AMOSWAP.W`, `AMOADD.W`, `AMOXOR.W`, `AMOOR.W`, `AMOAND.W`, `AMOMIN.W`, `AMOMAX.W`, `AMOMINU.W`, `AMOMAXU.W` (single-hart: aq/rl are ignored; any store invalidates LR reservation) |
| **C** ext.   | Common subset: `C.ADDI`, `C.LI`, `C.LUI`, `C.MV`, `C.ADD`, `C.J`, `C.JAL`, `C.JR`, `C.JALR`, `C.BEQZ`, `C.BNEZ`, `C.LW`, `C.SW`, `C.LWSP`, `C.SWSP`, `C.ADDI4SPN`, `C.ADDI16SP`, `C.NOP`, `C.SLLI`, `C.SRLI`, `C.SRAI`, `C.ANDI`, `C.SUB`, `C.XOR`, `C.OR`, `C.AND`, `C.EBREAK`. Each decodes to the equivalent RV32I form — covers ~80% of GCC-emitted compressed code. Uncommon variants (C.FLD*, C.LDSP, etc.) return Unknown. |
| Zicsr        | `CSRRW`, `CSRRS`, `CSRRC`, `CSRRWI`, `CSRRSI`, `CSRRCI`                      |
| System       | `ECALL`, `EBREAK`, `FENCE`, `MRET`                                           |

### Known gaps

| Extension    | Status                                                                      |
|--------------|-----------------------------------------------------------------------------|
| **F** / **D** (FP) | ❌ Not implemented                                                        |
| Interrupts      | 🟡 Machine-mode only. Timer (MTIP via mtime/mtimecmp) and external peripheral IRQs (folded into MEIP) dispatch via `mtvec`, with `mstatus.MIE` / `mie` gating. No PLIC — every external IRQ source collapses to MEIP. No per-source priority. |
| Privilege modes | M-mode only; no S/U mode or CSR enforcement.                                 |

**Implication:** firmware compiled with `-march=rv32imac_zicsr -mabi=ilp32`
now loads and runs, covering the common GCC-emitted subset. Uncommon
compressed variants will raise `DecodeError` — file an issue with the
opcode dump if you hit one.

---

## How to extend this document

1. When adding a new instruction:
   - Add an `Instruction::X` variant to the appropriate decoder.
   - Add execute logic to the matching `cpu/*.rs`.
   - Add a unit test in `crates/core/src/tests.rs` that covers the
     opcode encoding → register / memory side-effect.
   - Move the row in this document from the **gaps** table to the
     **implemented** table in the same PR.
2. When claiming a new arch tier (e.g. M4, M7, RV32G):
   - Audit the corresponding ARM ARM / RISC-V spec section.
   - Fill in every row. No partial tiers in marketing copy.
