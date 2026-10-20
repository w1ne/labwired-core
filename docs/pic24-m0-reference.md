# PIC24 / dsPIC33 M0 Reference — Encodings, SFRs, Integration Map

**Branch:** `feat/pic24-m0` · **Status:** implementation reference (no engine wiring yet) · **Target:** dsPIC33FJ32MC204 (issue #1426)

Sources (retrieved 2026-10-18, facts only):
- **DS70000157G** — *16-Bit MCU and DSC Programmer's Reference Manual* (instruction encodings)
- **DS70283H** — *dsPIC33FJ32MC202/204 and dsPIC33FJ16MC304 Data Sheet* (memory map, SFRs, peripherals)

## 1. Target facts (DS70283H)

| Item | Value |
| :--- | :--- |
| Program flash | 32 KB; GOTO at `0x000000`, reset at `0x000004`, IVT at `0x0000FE`, user flash from `0x000200` |
| Data SRAM | 2 KB: X `0x0800–0x0BFF`, Y `0x0C00–0x0FFF`; near-data to `0x1FFF` |
| SFR space | `0x0000–0x07FF` |
| UART1 | `U1MODE 0x0220`, `U1STA 0x0222`, `U1TXREG 0x0224` (write char here) |
| Core clock | up to 40 MIPS; reset FRC — board-level `cpu_hz` to be declared |

## 2. Instruction encodings (DS70000157G, verbatim extraction)

`k` = literal, `s` = source Wn, `d` = destination Wn. 24-bit words, stored as 3 little-endian bytes per instruction in an XC16 executable section (the loader adds the internal slot pad — see §3.1).

| Instruction | Encoding |
| :--- | :--- |
| `NOP` | `0000 0000 xxxx xxxx xxxx xxxx` |
| `NOPR` | `1111 1111 xxxx xxxx xxxx xxxx` |
| `GOTO Expr` | 1st `0000 0100 nnnn nnnn nnnn nnn0` · 2nd `0000 0000 0000 0000 0nnn nnnn` |
| `GOTO Wn` | `0000 0001 0100 0000 0000 ssss` |
| `BRA Expr` | `0011 0111 nnnn nnnn nnnn nnnn` (slit16 offset) |
| `BRA Wn` | `0000 0001 0110 0000 0000 ssss` |
| `CALL Expr` | 1st `0000 0010 nnnn nnnn nnnn nnn0` · 2nd `0000 0000 0000 0000 0nnn nnnn` |
| `RCALL Expr` | `0000 0111 nnnn nnnn nnnn nnnn` |
| `RETURN` | `0000 0110 0000 0000 0000 0000` |
| `MOV #lit16, Wnd` | `0010 kkkk kkkk kkkk kkkk dddd` |
| `MOV{.B} f, Wnd` | `1000 0fff ffff ffff ffff dddd` |
| `MOV Wns, f` | `1000 1fff ffff ffff ffff ssss` |
| `MOV{.B} f {,WREG}` | `1011 1111 1BDf ffff ffff ffff` |
| `MOV{.B} WREG, f` | `1011 0111 1B1f ffff ffff ffff` |
| `MOV{.B} Ws, Wd` | `0111 1www wBhh hddd dggg ssss` (address-modifier form) |
| `MOV.B #lit8, Wnd` | `1011 0011 1100 kkkk kkkk dddd` |
| `MOV{.B} [Ws+Slit10], Wnd` | `1001 0kkk kBkk kddd dkkk ssss` |
| `MOV{.B} Wns, [Wd+Slit10]` | `1001 1kkk kBkk kddd dkkk ssss` |

> Cross-check note: the 2-word address instructions carry the target in bits spread across both
> words (bit 0 of the 1st word's field is always 0 — word-aligned). Decode with unit tests using
> hand-computed vectors; validate against MPLAB `disasm` output once a toolchain is available.

## 3. M0 smoked sequence (fixture program)

Addresses are PC units (2 per instruction), user flash as XC16 lays it out:

```
0x000200: MOV  #0x48, W0         ; 'H'
0x000202: MOV  W0, 0x0224        ; U1TXREG
0x000204: NOP ×6
0x000210: BRA  $-1               ; self-loop (PC = PC + 2 + 2*(-1))
```

The fixture ELF's entry is `0x200`; the reset trampoline at `0x0` is not part
of the fixture — the model starts at `e_entry`.

Acceptance: UART sink receives `H`; PC/SP initialization deterministic; repeat runs identical.

### 3.1 XC16 ELF layout (verified 2026-10-19, DS50002071F)

The earlier M0 note called the layout "provisional: one instruction per
4-byte slot with the PC indexing slots". The XC16 compiler guide's "Program
Counter Unit" definition settles it:

- the PC increments by **2** per instruction word (24-bit instructions);
- **in an executable section, 2 program counter units are equivalent to
  3 bytes** — the ELF stores 3 bytes per instruction;
- in a read-only (data) section, 2 PC units are 2 bytes;
- "phantom bytes" (a 4th pad byte) appear in **HEX** files, not in ELF.

The model therefore:

- keeps `pc` in PC units (`PC_UNITS_PER_INSTRUCTION = 2`; interrupt vectors
  stay `0x4 + 2*N`, as on silicon);
- expands XC16 3-byte executable segments into its internal 4-byte slots at
  load (`cpu::pic24::xc16_executable_slots`) and places them at `addr / 2`;
- rejects an executable segment whose size is not a multiple of 3 rather
  than guessing;
- skips non-executable PIC24 program-memory segments (PSV/table reads are
  not modelled).

This matches the datasheet wording "addresses are incremented or decremented
by two during code execution" (DS70283H §4.2.3).

## 4. Engine integration map (M0 wiring)

| Concern | File | Change |
| :--- | :--- | :--- |
| Arch variant | `crates/config/src/chip.rs` | add `Pic24` (+ serde `pic24`, alias `dspic33`) |
| CPU module | `crates/core/src/cpu/pic24.rs`, `cpu/mod.rs` | new module + `pub use`; implement `Cpu` trait (mirror `Avr` for CPU-internal USART + serial sink) |
| Snapshot | `crates/core/src/snapshot.rs` | add the PIC24 snapshot variant (verify enum shape) |
| Arch policy | `crates/core/src/system/arch_policy.rs` | map `Pic24` → machine family + ELF machine (verify the Microchip EM value; keep `None` if unverified) |
| Session builder | `crates/core/src/system/builder/pic24.rs` (+ mod) | new builder mirroring `avr.rs` (bus, console sink, program load) |
| CLI | `crates/cli/src/commands/{run,test}.rs` | arch dispatch arms |
| Test expectations | `crates/core/tests/{one_arch_policy,register_coverage,session_catalog}.rs` | extend arch lists/panics |
| Chip + board | `configs/chips/dspic33fj32mc204.yaml`, `configs/systems/…` | per `docs/board_onboarding_playbook.md` |
| Example package | `examples/dspic33fj32mc204/` | `system.yaml`, `README.md`, `REQUIRED_DOCS.md`, `EXTERNAL_COMPONENTS.md`, `VALIDATION.md` |

## 5. Non-goals for M0

DSP MAC/accumulators, interrupts beyond reset, timers, PWM, QEI, ADC, SPI, I2C — each is its own milestone with its own validation (see the internal spec: `docs/strategy/targets/dspic33fj32mc204-onboarding-spec.md`).

## 6. Full onboarding gate (repo policy)

Promotion to "supported" requires the complete P0–P5 package (chip + system + firmware + example docs + validation evidence + unsupported-instruction audit). Nothing here claims support before that gate passes.
