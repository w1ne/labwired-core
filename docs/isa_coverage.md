# ISA Coverage

Source of truth for what CPU architectures and instructions LabWired Core
actually decodes and executes. README or marketing claims must match this
matrix. Last sync: `merge/surgical-additions` on `main`.

Convention:
- ✅ **Decoded + executed** — tested path exists.
- 🟡 **Decoded, execute stubbed** — decoder recognises the opcode but the
  executor is incomplete or unverified.
- ❌ **Not implemented** — decoder returns `Instruction::Unknown*`, which
  surfaces as `SimulationError::DecodeError` at runtime.

---

## ARM Cortex-M (Thumb / Thumb-2)

Target subset today: **ARMv6-M core + a substantial ARMv7-M data-processing
and bit-field subset**, including the wide multiply, bit-field, and
conditional-execution clusters. We do **not** yet claim ARMv7E-M (Cortex-M4/M7)
compliance: FPU (VFPv4), the DSP extension, saturating arithmetic, and most
load-exclusive ops remain unimplemented; attempting to execute them raises
`DecodeError`.

### Implemented

| Category             | Instructions                                                                 |
|----------------------|------------------------------------------------------------------------------|
| Data-proc imm        | `MOV #imm`, `ADD/SUB #imm3`, `ADD/SUB #imm8`, `CMP #imm`, `ADC`, `ADR`       |
| Data-proc reg        | `ADD/SUB Rd,Rn,Rm`, `ADD Rd,Rm (hi)`, `MOV Rd,Rm`, `CMP/CMN/TST Rd,Rm`       |
| Logical              | `AND`, `BIC`, `ORR`, `EOR`, `MVN`, `MUL`, `RSB/RSBS`                         |
| Shifts               | `LSL/LSR/ASR #imm`, `LSL/LSR/ASR Rd,Rm`, `ROR`                               |
| Stack / SP arith     | `ADD/SUB SP,#imm`, `ADD Rd,SP,#imm`, `ADR Rd,#imm`                           |
| Loads                | `LDR imm/reg/literal`, `LDR [SP,#imm]`, `LDRB`, `LDRH`, `LDRSB`, `LDRSH`, `LDRD` |
| Stores               | `STR imm/reg`, `STR [SP,#imm]`, `STRB`, `STRH`                               |
| Multi-reg            | `PUSH`, `POP`, `LDM`, `STM`, `LDMIA` (T2)                                    |
| Branches             | `B`, `B<cond>`, `BL`, `BX`, `BLX (reg)`, `CBZ`, `CBNZ`, `TBB`, `TBH`         |
| Control              | `NOP`, `CPSIE i`, `CPSID i`, `IT/ITE/ITT/…`                                  |
| Extension            | `UXTB`                                                                       |
| Thumb-2 bitops       | `BFI`, `BFC`, `SBFX`, `UBFX`, `CLZ`, `RBIT`, `REV`, `REV16`, `REVSH`         |
| Thumb-2 data         | `DataProc32`, `DataProcImm32`, `MOVW`, `MOVT`                                |
| **Barriers**         | `DMB`, `DSB`, `ISB` — decoded; no-ops on single-threaded sim                 |
| **System regs**      | `MSR` / `MRS` for PRIMASK (SYSm=0x10); other SYSm accepted but unmodelled    |
| **Wide multiply**    | `SMULL`, `UMULL`, `SMLAL`, `UMLAL` — 32×32 → 64-bit                          |
| **Mul-accumulate**   | `MLA`, `MLS` — 32-bit `Rd = Ra ± (Rn*Rm)`                                    |
| Breakpoint           | `BKPT` (halts simulation with `SimulationError::Halt`)                       |

**Interrupt model:** NVIC is implemented via the `nvic` peripheral; VTOR
relocation and exception entry / exit work. Faults (HardFault, MemManage,
BusFault, UsageFault) are **not** raised on invalid instructions — invalid
opcodes bubble up as `SimulationError::DecodeError`.

### Known gaps (ARMv7-M still missing)

| Category          | Missing                                                                     |
|-------------------|-----------------------------------------------------------------------------|
| Integer divide    | `SDIV`, `UDIV`                                                              |
| Saturating arith  | `QADD`, `QSUB`, `SSAT`, `USAT`                                              |
| Extension         | `SXTB`, `SXTH`, `UXTH`                                                      |
| ARMv7E-M DSP      | `SMLAD`, `SMUAD`, packed SIMD family — entire DSP extension                 |
| FPU (VFPv4)       | `VLDR`, `VSTR`, `VMOV`, `VADD`, `VMUL`, `VSQRT`, … — entire FPU             |
| Exclusives        | `LDREX`, `STREX`, `CLREX`                                                   |
| TT / security     | `TT`, `TTA`, ARMv8-M security extensions                                    |

---

## RISC-V

Target subset today: **RV32IMAC + Zicsr** (`rv32imac_zicsr`).

| Category     | Instructions                                                                 |
|--------------|------------------------------------------------------------------------------|
| U-type       | `LUI`, `AUIPC`                                                               |
| J/B-type     | `JAL`, `JALR`, `BEQ`, `BNE`, `BLT`, `BGE`, `BLTU`, `BGEU`                    |
| Loads        | `LB`, `LH`, `LW`, `LBU`, `LHU`                                               |
| Stores       | `SB`, `SH`, `SW`                                                             |
| I-type ALU   | `ADDI`, `SLTI`, `SLTIU`, `XORI`, `ORI`, `ANDI`, `SLLI`, `SRLI`, `SRAI`       |
| R-type ALU   | `ADD`, `SUB`, `SLL`, `SLT`, `SLTU`, `XOR`, `SRL`, `SRA`, `OR`, `AND`         |
| **M** ext.   | `MUL`, `MULH`, `MULHSU`, `MULHU`, `DIV`, `DIVU`, `REM`, `REMU` (full per-spec semantics for div-by-zero and INT_MIN/-1 overflow) |
| **A** ext.   | `LR.W`, `SC.W`, `AMOSWAP.W`, `AMOADD.W`, `AMOXOR.W`, `AMOOR.W`, `AMOAND.W`, `AMOMIN.W`, `AMOMAX.W`, `AMOMINU.W`, `AMOMAXU.W` (single-hart: aq/rl ignored; any store invalidates LR reservation) |
| **C** ext.   | Common GCC-emitted subset: `C.ADDI`, `C.LI`, `C.LUI`, `C.MV`, `C.ADD`, `C.J`, `C.JAL`, `C.JR`, `C.JALR`, `C.BEQZ`, `C.BNEZ`, `C.LW`, `C.SW`, `C.LWSP`, `C.SWSP`, `C.ADDI4SPN`, `C.ADDI16SP`, `C.NOP`, `C.SLLI`. Uncommon variants (`C.FLD*`, `C.LDSP`, etc.) return `Unknown`. |
| Zicsr        | `CSRRW`, `CSRRS`, `CSRRC`, `CSRRWI`, `CSRRSI`, `CSRRCI`                      |
| System       | `ECALL`, `EBREAK`, `FENCE`, `MRET`                                           |

### Known gaps

| Extension        | Status                                                                      |
|------------------|-----------------------------------------------------------------------------|
| **F** / **D** FP | ❌ Not implemented                                                           |
| Interrupts       | 🟡 Machine-mode only. Timer (MTIP via mtime/mtimecmp) and external peripheral IRQs (folded into MEIP) dispatch via `mtvec`, with `mstatus.MIE` / `mie` gating. No PLIC — every external IRQ source collapses to MEIP. No per-source priority. |
| Privilege modes  | M-mode only; no S/U mode or CSR enforcement.                                |

**Implication:** firmware compiled with `-march=rv32imac_zicsr -mabi=ilp32`
loads and runs, covering the common GCC-emitted subset plus atomics (which
Rust's `core::sync` and std atomics lower to even when compiled from
single-threaded sources).

---

## Xtensa (ESP32-S3 backend)

Target subset: **Xtensa LX7 narrow (16-bit) + wide (24-bit) general-purpose
instructions**, covering the subset the ESP-IDF bootrom and `hello_world`
firmware exercise. See `crates/core/src/cpu/xtensa.rs` for the full decoder
tree. No FPU, no Vector SIMD, no privileged-mode features.

This tier is experimental — enough to run the `esp32s3-zero` demo firmware
but not claimed for production use.

---

## How to extend this document

1. When adding a new instruction:
   - Add an `Instruction::X` variant to the appropriate decoder.
   - Add execute logic to the matching `cpu/*.rs`.
   - Add a unit test that covers encoding → register / memory side-effect.
   - Move the row in this document from the **gaps** table to the
     **implemented** table in the same PR.
2. When claiming a new arch tier (e.g. M4, M7, RV32G):
   - Audit the corresponding ARM ARM / RISC-V spec section.
   - Fill in every row. No partial tiers in marketing copy.
