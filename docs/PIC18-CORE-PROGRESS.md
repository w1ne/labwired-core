# PIC18 public-core vertical slice — progress

Branch: `agent/pic18-core`

Base revision: `8e3b5ee4d`

Status: **first real public PIC18 vertical slice**. This is generic public CPU
infrastructure. It does not onboard any catalogue part, does not copy private
chip data, and does not claim Proteus parity. No PIC18 chip descriptor is added.

## Landed

* Public `labwired_config::Arch::Pic18` with serde spellings `pic18` and
  `pic18f`; existing architecture variants and aliases are unchanged.
* Public `labwired_core::Arch::Pic18`.
* `MachineFamily::Pic18` and `machine_family()` mapping. `elf_arch(204)`
  recognises `EM_MCHP_PIC`, the registered ELF machine value for Microchip's
  8-bit PIC family (PIC18 is the only 8-bit PIC core modelled here).
* `cpu/pic18.rs`: reset, byte-addressed program fetch, 21-bit byte PC with
  bit 0 always clear, WREG/STATUS/BSR, banked data memory and Access Bank
  resolution, SFR window, PCL/PCLATH/PCLATU, FSR0-2, TBLPTR/TABLAT/PROD, and a
  hardware return stack.
* `Cpu` trait integration, runtime snapshot variant, trace observer events,
  register names/IDs, builder and node hooks, loader entry-point alignment
  check, and WASM/loader/CLI match arms (WASM execution is explicitly
  unavailable for PIC18 at this milestone).

## Implemented instruction semantics

Encodings follow the PIC18 standard instruction set. Any word outside this set
returns `SimulationError::DecodeError(pc)`; no opcode is guessed.

| Group | Instructions | Notes |
| :--- | :--- | :--- |
| Control | `NOP` | `0x0000` and the `1111 xxxx xxxx xxxx` form |
| Literal | `MOVLW`, `MOVLB`, `RETLW` | `MOVLB` selects `BSR<3:0>` |
| Byte move | `MOVWF`, `MOVF`, `MOVFF` | `MOVFF` is two words and uses a 12-bit absolute source/destination, ignoring banking |
| Byte ALU | `ADDWF`, `ANDWF`, `IORWF`, `XORWF`, `CLRF`, `INCF`, `DECF` | `d` and `a` bits honored |
| Bit | `BCF`, `BSF`, `BTG`, `BTFSC`, `BTFSS` | skip instructions skip a following two-word instruction as a unit |
| Relative | `BRA`, `RCALL`, `BZ`, `BNZ`, `BC`, `BNC`, `BOV`, `BNOV`, `BN`, `BNN` | signed offsets in instruction words |
| Absolute | two-word `GOTO`, two-word `CALL` | 20-bit literal is a **word** address (`k -> PC<20:1>`) |
| Stack | `RETURN`/`RETURN FAST` | `CALL FAST` saves WREG/STATUS/BSR for the matching fast return |
| FSR | `LFSR` | 12-bit literal loaded into FSR0/FSR1/FSR2 |

### Flags

`STATUS` models the classic PIC18 flag bits: `C=0`, `DC=1`, `Z=2`, `OV=3`,
`N=4`; upper bits read 0. Exact rules:

* `ADDWF`, `INCF`: carry out of bit 7, digit carry out of bit 3, signed
  overflow from same-sign operands, zero and sign of the result.
* `DECF`: borrow polarity follows PIC18 subtraction (`C`/`DC` set when no
  borrow), signed overflow from opposite-sign inputs, zero and sign.
* `ANDWF`, `IORWF`, `XORWF`, `MOVF`: `Z` and `N` only.
* `CLRF`: `Z` only.
* `MOVWF`, `MOVFF`, `BCF`, `BSF`, `BTG`: no flags.
* Instructions that affect flags do not write their result into `STATUS`; they
  update only the flag bits, matching the datasheet destination rule.

### Data memory

* Default model: 16 banks x 256 bytes (`0x000..0xFFF`) and a 64 KiB flash.
  A per-device geometry can be supplied with `Pic18::with_geometry`.
* `a = 1`: `BSR<3:0>:f`.
* `a = 0`: `f < 0x60` addresses access RAM at `f`; `f >= 0x60` addresses the
  SFR window at `0xF00 + f` (so `0xE8` is WREG at `0xFE8`).
* Modeled SFRs: WREG, STATUS, BSR, FSR0L/H, FSR1L/H, FSR2L/H, PRODL/H, TABLAT,
  TBLPTRL/H/U, PCL, PCLATH, PCLATU, STKPTR, TOSL/H/U. Other SFR addresses are
  plain bytes in the file for now.

### Program counter and return stack

* The PC is byte addressed; `set_pc`/snapshot restore clear bit 0.
* Single-word instructions advance by 2; `CALL`, `GOTO`, `MOVFF`, `LFSR`
  advance by 4.
* A write to `PCL` combines PCLATU/PCLATH/PCL and aligns the target.
* Return stack defaults to 31 levels (classic parts). Push past full sets
  `STKFUL` and overwrites the top entry; pop from empty sets `STKUNF` and
  returns 0. The pointer never wraps circularly. `STVREN` reset behavior is not
  modelled yet.

## Tests and results

Commands run from this worktree:

| Command | Result |
| :--- | :--- |
| `cargo test -p labwired-core pic18` | PASS: 17 PIC18 unit tests; other test binaries filtered to 0 |
| `cargo test -p labwired-core --lib cpu::pic18` | PASS: 17 passed; 0 failed |
| `cargo test -p labwired-core one_arch_policy` | PASS: 8 passed; 0 failed |
| `cargo test -p labwired-core --test session_catalog` | PASS: 1 passed; 0 failed |
| `cargo test -p labwired-core --test register_compliance` | PASS: 1 passed; 0 failed |
| `cargo test -p labwired-core --test register_coverage` | PASS: 1 passed; 0 failed |
| `cargo test -p labwired-config --lib --tests` | PASS: 265 passed across 9 binaries; 0 failed |
| `cargo test -p labwired-config` | Unit/integration pass; the doctest phase cannot start because this environment has no `rustdoc` binary (`failed to execute rustdoc ... No such file or directory`) |
| `cargo check -p labwired-cli -p labwired-wasm -p labwired-loader` | PASS |
| `cargo check --workspace --all-targets` | Blocked by a pre-existing unrelated failure: `firmware-semihost-smoke` uses host-invalid inline assembly registers `r0`/`r1` |

The PIC18 tests cover reset and PC alignment, Access Bank vs BSR addressing,
exact `C`/`DC`/`Z`/`OV`/`N` cases, logic/`CLRF`/`MOVF` flag rules, `MOVFF`
absolute addressing, two-word `GOTO`/`CALL`/`RETURN`, `LFSR`, conditional
branches, skip over a two-word instruction, return-stack overflow/underflow,
PCL latch writes, unknown-opcode `DecodeError`, snapshot round-trip, program
image loading, and a hand-assembled add/store/GOTO program.

Commit: `d6402f376` (`feat(core): add PIC18 vertical slice`).

## Next missing pieces

1. **Loader / firmware toolchain.** XC8 output format and address semantics
   still need verification against a pinned compiler. The current ELF path
   treats `PT_LOAD` `p_paddr` as a byte address; a real XC8 ELF/HEX/COFF path,
   section handling, and a committed fixture are the next loader work.
2. **Interrupts.** Code vectors at `0x0008` (high) and `0x0018` (low),
   `INTCON`/`INTCON2`/`INTCON3`, `PIR`/`PIE`/`IPR`, priority via `RCON.IPEN`,
   `GIE/GIEH/GIEL`, `RETFIE`, and fast-register-stack context restore.
3. **Peripherals.** GPIO (`TRIS`/`LAT`/`PORT`, pad masks, analog/PPS), oscillator
   and `CONFIG`, TMR0/1/2/3, CCP/ECCP, MSSP, EUSART, ADC, comparators, WDT,
   sleep/idle, and reset sources/`RCON`.
4. **Full ISA.** Remaining standard instructions (`ADDLW`, `SUBLW`, `ANDLW`,
   `IORLW`, `XORLW`, rotates/shifts, `COMF`, `NEGF`, `SETF`, `SWAPF`, `MULLW`,
   `MULWF`, `CPFS*`, `TSTFSZ`, `DECFSZ`, `INCFSZ`, `DAW`, `PUSH`/`POP`,
   `TBLRD`/`TBLWT`), FSR indirect modes, and the `XINST` extended set plus
   indexed-literal addressing. Each newly decoded class needs a fixture before
   it is added.
5. **Data-memory completeness.** `INDF0/1/2` and the `POSTINC`/`POSTDEC`/
   `PREINC`/`PLUSW` aliases, unmodeled SFR reset values, and field-level
   bank/SFR masks per device.
6. **Device geometry and integration.** Per-device flash size, RAM/bank count,
   stack depth, `STVREN`, chip YAMLs, console/UART wiring, and a real
   end-to-end firmware run. Interrupt and peripheral support must not be
   inferred from a family label; each device needs datasheet evidence.
