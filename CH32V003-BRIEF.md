# TASK — WCH CH32V003 bring-up, SLICE 1 (core, pure Rust, no external toolchain)

You are working in a git worktree of `w1ne/labwired-core` (the LabWired Rust simulator).
Branch: `feat/ch32v003-rv32e-profile`. Base: `origin/main`.

REPO LAYOUT: the Rust simulator is under `crates/`. The core is `crates/core`.
Chip descriptors are YAML in `configs/chips/`. CI gates live in `scripts/ci/`.

## HARD RULES (violating any of these fails the task)
- Do NOT merge to main. Do NOT push. Do NOT open a PR. Just commit on the current branch.
- Do NOT rebase, never force-push, never amend an existing commit.
- Never weaken, delete, or skip a test or a gate. Never add `#[ignore]` to make a test pass.
- Never invent a datasheet fact. If unsure of a register address/semantic, say so in your report.
- Do NOT fake behaviour with a stub that silently falls back to a standard RV32 profile.
- Commit with `git commit -s` (DCO sign-off). One logical commit per numbered slice below is fine.
- Do NOT run `cargo build` for the whole workspace in release; use `cargo test -p <crate> --lib` on
  the narrow target. Another 5 Codex jobs share this machine: keep builds small.

## CONTEXT (measured — do NOT rebuild what exists)
- `crates/core/src/cpu/riscv.rs` (~2160 lines): RV32 core. Register file `pub x: [u32; 32]`,
  helpers `read_reg(n)`/`write_reg(n,val)` (x0 hardwired 0). Standard machine CSRs
  (mstatus/mtvec/mepc/mcause/mie/mip/mtval/mscratch). `handle_trap(cause, epc)` exists.
  `RiscVCoreProfile` enum = `{ StandardRv32, Esp32C3 }`, selected by
  `RiscVCoreProfile::for_chip(&ChipDescriptor)` (esp32* -> Esp32C3, `core: "rv32*"` -> StandardRv32).
- `crates/core/src/decoder/riscv.rs` (745 lines): decodes RV32I + the C (compressed) extension.
  I and E share instruction encodings, so the decoder needs no change for E.
- `crates/core/src/peripherals/rcc.rs`: `RccRegisterLayout` enum with 24 STM32-family variants and a
  `FromStr`. There is NO WCH layout.
- There is NO PFIC anywhere. `crates/core/src/peripherals/nvic.rs` is the ARM NVIC only. Do not reuse it.
- `configs/chips/` has 122 descriptors, none WCH (`grep -ri wch configs/` is empty).

## SLICE 1 — RV32E register-count profile (do this FIRST, it is the correctness item)
Add a new `RiscVCoreProfile::Rv32e` variant. Semantics: the base ISA is RV32E, which has only
x0..x15; any instruction that reads or writes x16..x31 is an ILLEGAL INSTRUCTION.

Implementation requirements:
1. New enum variant `Rv32e` in `crates/core/src/cpu/riscv.rs`.
2. `for_chip`: a chip whose `core` string is `rv32e` / `rv32ec` / `rv32emc` / `rv32em` (case
   insensitive, trim) selects `Rv32e`. Do NOT let it fall through to StandardRv32.
3. In the interpreter execute path (`fn step`), when `core_profile == Rv32e`, BEFORE the instruction
   has any side effect, check every register operand the instruction uses/writes. If any is >= 16,
   raise an illegal-instruction trap: `self.mtval = 0; self.handle_trap(2, self.pc);` then return
   `Ok(())` WITHOUT executing the instruction and WITHOUT advancing pc past executing it (the trap
   sets pc = mtvec). Add a small helper e.g. `fn rv32e_reg_operands(instruction: &Instruction) -> ...`
   that returns the rd/rs1/rs2 the instruction touches, or add a match. Cover at least the common
   integer ops: Lui/Auipc/Jal/Jalr/Branches/Loads/Stores/OpImm/Op/AMO-if-present, plus the C-format
   variants. Note: CR-format compressed regs (C.LW/C.SW/C.ADDI4SPN...) only encode x8..x15, so
   they are always legal — but C.LWSP/C.LDSP/C.JR/... use a full 5-bit reg field: those CAN be >=16
   and must be checked.
4. The CSR gate: `Rv32e` should behave like `StandardRv32` for the standard cycle CSR at 0xC00/0xC80
   (the QingKe exposes these; if you are not certain, treat as StandardRv32 and say so in the report).
   Wherever the code matches `RiscVCoreProfile::StandardRv32` for CSR behaviour, decide explicitly
   whether Rv32e shares it and document the choice in a code comment.
5. Unit tests (add to the existing test module style in riscv.rs, or a new
   `crates/core/src/tests/rv32e_profile.rs` if the file uses a tests module — FOLLOW THE EXISTING
   PATTERN, find it first). At minimum:
   - a 4-byte `addi x16, x0, 1` on an Rv32e core traps with mcause==2, and x16 stays 0 / unchanged;
   - the same instruction on a StandardRv32 core executes normally and sets x16;
   - a legal `addi x5, x0, 1` on Rv32e executes and sets x5==1;
   - NEGATIVE CONTROL: prove the test can fail (describe in the report how you checked, e.g. by
     temporarily removing the check and seeing the test go red — do not leave the removal in).

## SLICE 2 — PFIC (WCH Programmable Fast Interrupt Controller) peripheral model
Create `crates/core/src/peripherals/pfic.rs`. This is WCH's own controller (NOT PLIC, NOT NVIC):
- 23 peripheral interrupt sources + 4 core (systick/software) sources.
- Registers (from CH32V003RM, verify against the RM before committing; state any you could not verify):
  `PFIC_ISR1` 0xE000E000 (interrupt enable/status set 1), `PFIC_ISR2` 0xE000E004,
  `PFIC_IENR1` 0xE000E100, `PFIC_IENR2` 0xE000E104 (enable set),
  `PFIC_IRER1` 0xE000E180, `PFIC_IRER2` 0xE000E184 (enable clear),
  `PFIC_IPSR1` 0xE000E200, `PFIC_IPSR2` 0xE000E204 (pending set),
  `PFIC_IPRR1` 0xE000E280, `PFIC_IPRR2` 0xE000E284 (pending clear),
  `PFIC_IACTR1` 0xE000E300, `PFIC_IACTR2` 0xE000E304 (active),
  `PFIC_IPRIORn` 0xE000E400.. (per-source priority, 4 bits used), `PFIC_SCTLR` 0xE000ED10.
- 2-level hardware nesting (highest 2 priority levels nest in hardware), VTF vector-free entry.
- Model the register block + enable/disable/pending/active/priority semantics and expose
  `pending_source()` / `acknowledge()`-style API so the bus can raise the CPU interrupt. Keep it
  self-contained; do NOT wire it into every chip yet — a chip descriptor maps it in a later slice.
- Unit tests for the register block: set/clear enable via IENR/IRER, pending via IPSR/IPRR, priority
  via IPRIOR, and the "highest-priority pending wins" selection. Negative control as above.

## SLICE 3 — WCH clock tree (RccRegisterLayout)
Add a `Wch` variant to `RccRegisterLayout` in `crates/core/src/peripherals/rcc.rs` (and its `FromStr`,
e.g. "wch"), plus a `WchRcc` register model implementing:
`RCC_CFGR0` — HSI 24 MHz RC (default on), LSI 128 kHz, HSE 4..25 MHz, PLL x2 -> 48 MHz, AHB
prescaler (SW/HPRE/PLLSRC fields), and per-peripheral enables (`RCC_APB2PCENR`, `RCC_APB1PCENR`,
`RCC_AHBPCENR`). Model reads/writes so firmware that spins on the PLL-ready bit proceeds. Follow the
existing V2/H5/WB layout implementations as the pattern. Unit test: write SW=PLL, set PLLON, poll
ready; assert the modelled SYSCLK. Negative control as above.

## REPORT (write to /home/andrii/worktrees/labwired-ch32v003/CH32V003-SLICE1-REPORT.md)
Include: what you changed (files + line counts), the exact `cargo test` commands you ran and their
REAL pasted output (not paraphrased), every datasheet fact you could NOT verify, and what is left for
the next slice (peripheral map + descriptor + firmware fixture). Do not claim a test passed unless
you pasted its output.

## WHAT NOT TO DO
- Do NOT build the chip descriptor YAML or the firmware fixture in this slice (needs a separate step).
- Do NOT touch `crates/core/src/peripherals/nvic.rs`.
- Do NOT modify anything outside `crates/core/src/cpu/riscv.rs`,
  `crates/core/src/peripherals/{pfic.rs,rcc.rs}`, `crates/core/src/peripherals/mod.rs`,
  and a new test file if you add one.
