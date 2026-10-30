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
  resolution, SFR window, PCL/PCLATH/PCLATU, FSR0-2 (including the indirect
  aliases), TBLPTR/TABLAT/PROD, and a hardware return stack.
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
| Interrupts | `RETFIE`/`RETFIE FAST` | vectors at `0x0008`/`0x0018`, `INTCON`/`INTCON2`/`INTCON3`, `PIR1`/`PIR2`/`PIE1`/`PIE2`/`IPR1`/`IPR2`, `RCON.IPEN`, `GIE`/`GIEH`/`GIEL`/`PEIE`; entry saves and `FAST` restores the fast register stack |
| FSR literal | `LFSR` | 12-bit literal loaded into FSR0/FSR1/FSR2 |
| Indirect | `INDF0/1/2`, `POSTINC0/1/2`, `POSTDEC0/1/2`, `PREINC0/1/2`, `PLUSW0/1/2` | Virtual Access Bank registers used by byte/bit file operations; full 12-bit FSR address, independent of BSR and `a` |

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

### Interrupts

The core samples enabled interrupt sources between instructions (after the
current instruction retires) and pushes the next instruction as the return
address. Sticky flags live in the SFR file and are cleared by the ISR.

* `RCON.IPEN = 0`: every source vectors to `0x0008`; `INTCON.GIE` (bit 7) is
  the global enable and is cleared on entry. The `PIR1`/`PIR2` peripheral
  sources additionally need `INTCON.PEIE` (bit 6); the `INTCON`/`INTCON3`
  sources need only `GIE`. `RETFIE` sets `GIE` again.
* `RCON.IPEN = 1`: high-priority sources vector to `0x0008` through `GIEH`
  (bit 7), low-priority sources to `0x0018` through `GIEH` **and** `GIEL`
  (bit 6) — `GIEH = 0` disables all interrupts (DS39631 Register 9-1). Entry
  clears only the matching global bit, so a high-priority source may nest
  inside a low-priority ISR. `RETFIE` sets both `GIEH` and `GIEL`.
* Sources: `INTCON` TMR0/INT0/RB, `INTCON3` INT1/INT2, and the eight bits of
  `PIR1`/`PIE1`/`IPR1` and `PIR2`/`PIE2`/`IPR2`. INT0 is always high priority
  when `IPEN = 1`; the other source priorities come from `INTCON2`,
  `INTCON3` or the matching IPR bit. `INTCON3` follows the PIC18F4520
  layout (DS39631 Register 9-3): INT2IP=7, INT1IP=6, INT2IE=4, INT1IE=3,
  INT2IF=1, INT1IF=0.
* Interrupt entry loads WREG/STATUS/BSR into the one-level fast register
  stack (the same shadow `CALL FAST`/`RETURN FAST` use), and `RETFIE FAST`
  restores them. A high-priority entry nested in a low-priority ISR
  overwrites that save, as on silicon. Per-device vector remapping is not
  modelled.
* **No peripheral raises a flag yet.** `interrupt_request` reads the CPU's
  internal SFR array only, and no bus peripheral writes `PIRx`/`INTCONx`, so
  today the only way to pend a source is firmware (or a test) writing the
  flag bit. `Cpu::set_exception_pending` is recorded but not consumed.


* Default model: 16 banks x 256 bytes (`0x000..0xFFF`) and a 64 KiB flash.
  A per-device geometry can be supplied with `Pic18::with_geometry`.
* `a = 1`: `BSR<3:0>:f`.
* `a = 0`: `f < 0x60` addresses access RAM at `f`; `f >= 0x60` addresses the
  SFR window at `0xF00 + f` (so `0xE8` is WREG at `0xFE8`).
* Modeled SFRs: WREG, STATUS, BSR, FSR0L/H, FSR1L/H, FSR2L/H, PRODL/H, TABLAT,
  TBLPTRL/H/U, PCL, PCLATH, PCLATU, STKPTR, TOSL/H/U. Other SFR addresses are
  plain bytes in the file for now.
* Indirect aliases are the classic Access Bank virtual registers in three
  groups: `0xFDB..0xFDF` (FSR2), `0xFE3..0xFE7` (FSR1) and `0xFEB..0xFEF`
  (FSR0), each ordered `PLUSW`, `PREINC`, `POSTDEC`, `POSTINC`, `INDF`. This
  matches DS41303F §5.4.3.1/§5.4.3.3, including the INDF1 example at `0xFE7`.
  Byte-oriented and bit-oriented file operations resolve through the same
  helper.
* `INDFn` reads/writes `FSRn` without changing it. `POSTINCn`/`POSTDECn`
  access the current address and then move the pointer by one;
  `PREINCn` moves before the access. A read-modify-write instruction
  (`ADDWF`, `INCF`, `BCF`, ...) resolves the operand once, writes the original
  location, then applies the pointer update once. `PLUSWn` adds the signed
  8-bit WREG offset and changes neither WREG nor `FSRn`.
* If an indirect access resolves to any of the 15 virtual operand addresses
  above, the virtual register is not physically implemented: an indirect read
  returns `0x00`, and an indirect write is a `NOP` (DS41303F §5.4.3.3). A
  rejected write also drops the selected operand's post-action, because no
  access is committed in this model.
* If an indirect write resolves to an FSR pair (`FSR0L/H`, `FSR1L/H`,
  `FSR2L/H`), the byte is stored in the FSR, but the selected operand's
  `POSTINC`/`POSTDEC` is not applied. `PREINC` has already run before the
  target is known. This is the documented self-target interaction, e.g.
  `POSTDEC2` targeting `FSR2L` stores the value without decrementing.
* FSR addresses and updates wrap in the 12-bit data space. The direct SFR
  read/write helpers still expose the raw file bytes; indirect expansion and
  virtual-target filtering are instruction-operand features.
* `MOVFF` keeps its 12-bit absolute-address path and does not expand virtual
  `FSRn` aliases in this milestone. The public material inspected did not
  establish that behavior for `MOVFF`, so it is left explicit rather than
  guessed.
* `TOSL/H/U` remain direct hardware-stack mirrors; indirect-target semantics
  for those stack registers are not claimed. The virtual-operand rule above is
  limited to the documented 15 INDF/POST/PRE/PLUS addresses.

### Program counter and return stack

* The PC is byte addressed; `set_pc`/snapshot restore clear bit 0.
* Single-word instructions advance by 2; `CALL`, `GOTO`, `MOVFF`, `LFSR`
  advance by 4.
* A write to `PCL` combines PCLATU/PCLATH/PCL and aligns the target.
* `ADDWF PCL,f` is a computed GOTO from the instruction's own address: the
  low-byte addition does not propagate into the upper PC bytes, and reading
  `PCL` latches the current PCLATH/PCLATU. `MOVFF ... ,PCL` uses the latches.
* Return stack defaults to 31 levels (classic parts). Push past full sets
  `STKFUL` and overwrites the top entry; pop from empty sets `STKUNF` and
  returns 0. The pointer never wraps circularly. `STVREN` reset behavior is not
  modelled yet.

## Tests and results

Commands run from this worktree:

| Command | Result |
| :--- | :--- |
| `cargo test -p labwired-core pic18` | PASS: 37 PIC18 unit tests; other test binaries filtered to 0 |
| `cargo test -p labwired-core --lib cpu::pic18` | PASS: 37 passed; 0 failed |
| `cargo fmt --all -- --check` | PASS |
| XC8 local check (`command -v xc8-cc` / `xc8`, repository fixture search) | No XC8 compiler on PATH and no PIC18 ELF/HEX fixture in the worktree; no real firmware smoke is claimed |
| `cargo test -p labwired-core one_arch_policy` | PASS: 8 passed; 0 failed |
| `cargo test -p labwired-core --test session_catalog` | PASS: 1 passed; 0 failed |
| `cargo test -p labwired-core --test register_compliance` | PASS: 1 passed; 0 failed |
| `cargo test -p labwired-core --test register_coverage` | PASS: 1 passed; 0 failed |
| `cargo test -p labwired-config --lib --tests` | PASS: 265 passed across 9 binaries; 0 failed |
| `cargo test -p labwired-config` | PASS: 265 passed (including 0 doctests); 0 failed |
| `cargo test -p labwired-loader` | PASS: 57 passed across 4 binaries; 0 failed |
| `cargo check -p labwired-cli -p labwired-wasm -p labwired-loader` | PASS |
| `cargo check --workspace --all-targets` | Blocked by a pre-existing unrelated failure: `firmware-semihost-smoke` uses host-invalid inline assembly registers `r0`/`r1` |

The PIC18 tests cover reset and PC alignment, Access Bank vs BSR addressing,
`INDF0/1/2` through their FSRs with a conflicting BSR, FSR1/FSR2 post-increment,
`POSTINC` read/write and read-modify-write pointer movement, `POSTDEC`/`PREINC`
ordering, `PLUSW` positive and negative signed offsets without pointer/WREG
changes, 12-bit pointer wrap, indirect bit operations, reads/writes of all
15 virtual operand aliases (`0x00` read, `NOP` write), the FSR-pair
write/post-action suppression (including self-targeting `POSTDEC2`), exact
`C`/`DC`/`Z`/`OV`/`N` cases, logic/`CLRF`/`MOVF` flag rules, `MOVFF` absolute
addressing, two-word `GOTO`/`CALL`/`RETURN`, `LFSR`, conditional branches,
`BTG`/`BTFSS`, `RCALL`/`RETLW`, skip over a two-word instruction, return-stack
overflow/underflow, PCL latch writes, `MOVFF` to `PCL`, `ADDWF PCL` computed
jumps with and without low-byte carry, unknown-opcode `DecodeError`, snapshot
round-trip, program image loading, and a hand-assembled add/store/GOTO program.
The interrupt tests cover GIE masking, `PEIE` gating of peripheral sources
with `IPEN = 0`, the high/low vectors and `GIEH`/`GIEL` gating (including
`GIEH = 0` masking low-priority sources), `INTCON` TMR0/INT0 sources, the
`INTCON3` INT1/INT2 bits pinned to raw datasheet values, `PIR1`/`PIE1`/`IPR1`
priority, `RETFIE` re-enabling, and `RETFIE FAST` restoring the context
saved on interrupt entry (no hand-seeded shadow). `cpu_trace_conformance`
covers `Pic18` like every other core. No catalogue part is onboarded or claimed.

Commit: `d6402f376` (`feat(core): add PIC18 vertical slice`).

Follow-ups: `75f859ce9` (`fix(pic18): let MOVFF to PCL update the PC`),
`7f81f318b` (progress note), `c718f35a8`
(`test(pic18): cover PCL jumps and remaining bit/return ops`), and
`d56049d17` (`feat(pic18): implement FSR indirect addressing`), plus
`3fb236eec` (`fix(pic18): honor virtual indirect target rules`).

## Next missing pieces

1. **Loader / firmware toolchain.** XC8 output format and address semantics
   still need verification against a pinned compiler. The current ELF path
   treats `PT_LOAD` `p_paddr` as a byte address; a real XC8 ELF/HEX/COFF path,
   section handling, and a committed fixture are the next loader work.
2. **Interrupts.** The basic controller is in: vectors `0x0008`/`0x0018`,
   `INTCON`/`INTCON2`/`INTCON3`, `PIR1`/`PIR2`/`PIE1`/`PIE2`/`IPR1`/`IPR2`,
   `RCON.IPEN`, `GIE/GIEH/GIEL`/`PEIE`, `RETFIE`/`RETFIE FAST`, and the
   fast register stack saved on entry. Still missing: peripherals that
   raise `PIRx` flags, per-device vector remapping, interrupt latency/cycle accounting, and the
   `RCON` reset-source/`STVREN` behavior tied to the vector path.
3. **Peripherals.** GPIO (`TRIS`/`LAT`/`PORT`, pad masks, analog/PPS), oscillator
   and `CONFIG`, TMR0/1/2/3, CCP/ECCP, MSSP, EUSART, ADC, comparators, WDT,
   sleep/idle, and reset sources/`RCON`.
4. **Full ISA.** Remaining standard instructions (`ADDLW`, `SUBLW`, `ANDLW`,
   `IORLW`, `XORLW`, rotates/shifts, `COMF`, `NEGF`, `SETF`, `SWAPF`, `MULLW`,
   `MULWF`, `CPFS*`, `TSTFSZ`, `DECFSZ`, `INCFSZ`, `DAW`, `PUSH`/`POP`,
   `TBLRD`/`TBLWT`), and the `XINST` extended set plus indexed-literal
   addressing. Each newly decoded class needs a fixture before it is added.
5. **Data-memory completeness.** Expanded (>4 KiB) data memory and wider FSR
   pointers, direct `FSRnH` field masking (indirect paths already wrap at 12
   bits), device-specific Access Bank split (`0x60` vs `0x80`), unmodeled SFR
   reset values, indirect-target behavior for `TOSL/H/U`, and
   `MOVFF`/`MOVFFL` virtual-alias behavior that still needs authoritative
   evidence.
6. **Device geometry and integration.** Per-device flash size, RAM/bank count,
   stack depth, `STVREN`, chip YAMLs, console/UART wiring, and a real
   end-to-end firmware run. Interrupt and peripheral support must not be
   inferred from a family label; each device needs datasheet evidence.
