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

The fixture ELF carries the real reset vector: a 2-word `GOTO 0x200` at PC 0,
so the model resets to `PC = 0` and vectors through it (DS70283H Figure 7-1,
"Reset – GOTO Instruction" / "Reset – GOTO Address"). `e_entry` is `0x200`.

Acceptance: UART sink receives `H`; PC/SP initialization deterministic; repeat runs identical.

### 3.1 XC16 ELF layout (verified 2026-10-22 against real XC16 v1.70 output)

The M0 note first called the layout "provisional"; #1447 then claimed the ELF
stores **3 bytes per instruction**, reading DS50002071F's "Program Counter
Unit" glossary entry ("in an executable section, 2 program counter units are
equivalent to 3 bytes"). That is the 16-bit assembler's **conceptual address
arithmetic**, not the ELF file layout — and it was disproved by building the
committed `dspic33fj32mc204-xc16-uart-smoke.elf` with XC16 v1.70 and
inspecting it:

- every executable section (`.text`, `.reset`, `.ivt.*`, `.aivt.*`) has a
  size that is a **multiple of 4**;
- the bytes are 4-byte slots: `0f 80 20 00 | 0e ff 20 00 | …` — the 24-bit
  little-endian word plus one pad byte (the "phantom byte" of the 4-byte
  container, DS50002071F glossary);
- the disassembler decodes with a 4-byte stride while PC addresses advance
  by 2 per instruction (`0x200`, `0x202`, …), i.e. **addresses are PC
  units**, 2 per instruction;
- `.reset` holds the 2-word `GOTO` as two slots; each `.ivt.*`/`.aivt.*`
  entry is one slot holding a **24-bit ISR address as data**
  (DS70283H §7.1: "Each interrupt vector contains a 24-bit-wide address").

The model therefore:

- keeps `pc` in PC units (`PC_UNITS_PER_INSTRUCTION = 2`) and one 24-bit word
  per 4-byte flash slot (`INSTR_SLOT_BYTES = 4`) — **identical to the ELF
  layout**, so the parsers copy executable sections verbatim
  (`xc16_section_is_slot_aligned` validates the size);
- places each section at its own PC-unit address from the section table
  (program headers can span sections at non-contiguous addresses);
- loads the PC from the vector table on exception entry
  (`INTCON2.ALTIVT` selects the AIVT at `0x104`);
- skips non-executable PIC24 program-memory sections (PSV/table reads are
  not modelled).

This matches the datasheet wording "addresses are incremented or decremented
by two during code execution" (DS70283H §4.2.3) and the reset/vector figures
(DS70283H §4 and §7.1).

### 3.2 DSP MAC class (M1c, verified 2026-10-23 against DS70000157G examples and real XC16 output)

Implemented in `crates/core/src/cpu/pic24.rs`; encodings are cross-checked
against `xc16-as` output in `dsp_words_match_xc16_as`, and the semantics are
pinned by the PRM's own worked examples in the `dsp_*` unit tests.

| Instruction | Encoding (24-bit) | Notes |
| :--- | :--- | :--- |
| `MAC Wm*Wn, Acc {,AWB}` | `1100 0mmm A0xx yyii iijj jjaa` | `mmm` = Table 5-12 |
| `MPY Wm*Wn, Acc` | `1100 0mmm A0xx yyii iijj jj11` | product replaces Acc |
| `MPY.N Wm*Wn, Acc` | `1100 0mmm A1xx yyii iijj jj11` | negated product |
| `MSC Wm*Wn, Acc {,AWB}` | `1100 0mmm A1xx yyii iijj jjaa` | Acc −= product |
| `MAC/MPY Wm*Wm, Acc` | `1111 00mm A0xx yyii iijj jj0b` | square, no write-back |
| `CLR Acc {,AWB}` | `1100 0011 A0xx yyii iijj jjaa` | clears Acc and its OA/SA (OB/SB) |
| `MOVSAC Acc {,AWB}` | `1100 0111 A0xx yyii iijj jjaa` | prefetch/write-back only |
| `LAC Ws/[...], {#Slit4,} Acc` | `1100 1010 Awww wrrr rggg ssss` | 32-bit shifter |
| `SAC Acc, {#Slit4,} Wd/[...]` | `1100 1100 Awww wrrr rhhh dddd` | 40-bit shift, SATDW |
| `SAC.R Acc, {#Slit4,} Wd/[...]` | `1100 1101 Awww wrrr rhhh dddd` | + RND rounding |

**Prefetch fields** (Tables 5-7…5-10): `iiii`/`jjjj` select the X (W8/W9) and
Y (W10/W11) operation — no prefetch (`0100`), `[Wn]`, `[Wn]±2/4/6`, and the
`[W9+W12]`/`[W11+W12]` register-offset mode (16-bit signed offset, no
register update); `xx`/`yy` select the W4–W7 destinations. `aa` is the
write-back selector (Table 5-13): `W13`, `[W13]+=2`, none.

**CORCON modes**: `IF` (bit 0, fractional multiplies are left-shifted one),
`RND` (bit 1, biased vs convergent rounding), `SATDW` (bit 5),
`SATB`/`SATA` (bits 6/7), `ACCSAT` (bit 4, 9.31 vs 1.31), `US` (bit 12,
unsigned multiplies).

**Status** (SR high byte, §2.16.4): `OA`/`OB` = guard-bit overflow (set when
saturation is disabled or in super mode), `SA`/`SB` = sticky saturation,
`OAB`/`SAB` derived; a write to SR can only clear them, and clearing `SAB`
clears `SA`/`SB`. `CLR` clears the accumulator's own pair.

**Rounding** (`SAC.R`, write-back): conventional rounds up when the low word
is ≥ 0x8000; convergent rounds a 0x8000 tie to even. **SATDW** saturates a
store to 0x7FFF/0x8000 when the shifted value needs the guard bits (PRM SAC
examples 1–2).

Approximations (documented, not guessed — no PRM example covers them):
`LAC`'s shifter is 32-bit (its examples 1–2 pin this); with super saturation
and an overflowing left shift the exact 40-bit result is kept; `MPY.N`
follows its "OA, OB, OAB" status list literally (no saturation/SA/SB);
write-back applies SATDW like `SAC.R`.

Verification: 16 `dsp_*` unit tests use the PRM's MAC/MPY/MPY.N/MSC/CLR/
MOVSAC/LAC/SAC/SAC.R examples as golden vectors, plus saturation-mode,
rounding-tie, unsigned, sticky-flag and full prefetch-table coverage; a real
XC16 v1.70 firmware (`firmware/xc16-dsp-mac.c`) boots and reproduces the
example-1 sequence (`crates/core/tests/pic24_xc16_dsp.rs`).

### 3.3 Motor Control PWM + QEI (M2, implemented 2026-10-23)

Sources: DS70187E (Section 14, MCPWM), DS70208C (Section 15, QEI) and
DS70283H Tables 4-8/4-10 (SFR maps), 7-1 (vectors), 1-1 (pins) and 10-2 (PPS).

**SFRs.** PWM1: `P1TCON 0x01C0`, `P1TMR 0x01C2`, `P1TPER 0x01C4`,
`P1SECMP 0x01C6`, `PWM1CON1 0x01C8`, `PWM1CON2 0x01CA`,
`P1DTCON1 0x01CC`, `P1DTCON2 0x01CE`, `P1FLTACON 0x01D0`,
`P1OVDCON 0x01D4`, `P1DC1-3 0x01D6/0x01D8/0x01DA`. QEI1:
`QEI1CON 0x01E0`, `DFLT1CON 0x01E2`, `POS1CNT 0x01E4`, `MAX1CNT 0x01E6`.
Interrupts: `IFS3 0x008A` (FLTA1 bit 15, QEI bit 9, PWM1 bit 8),
`IEC3 0x009A`, `IPC14 0x00C0` (QEIIP 10-8, PWM1IP 6-4),
`IPC15 0x00C2` (FLTA1IP 14-12). Inputs: `RPINR12 0x0698` (FLTA1R),
`RPINR14 0x069C` (QEB1R 12-8, QEA1R 4-0), `RPINR15 0x069E` (INDX1R 4-0).
UPDN is a PPS output (function 11010).

**Pins.** PWM1H1/L1 = RB14/RB15, PWM1H2/L2 = RB12/RB13, PWM1H3/L3 =
RB10/RB11 (fixed peripheral outputs; they override the GPIO/PPS drive).
QEA/QEB/INDX are remappable inputs.

**PWM model.** Time base: free running, single event, continuous up/down and
up/down with double update; 1:1/1:4/1:16/1:64 input prescaler and 1:1…1:16
output postscaler (ignored in single-event/double-update modes, §14.9).
Edge-aligned outputs go active at TMR = 0 and inactive at the duty match;
center-aligned outputs go inactive on the up-count match and active on the
down-count match (§14.11.3). Duty 0 is inactive for the whole period and
duty > PTPER is active for the whole period. P1TPER and P1DC1-3 are
double-buffered; the buffers load at the period boundary (TMR = 0, and at
TMR = PTPER for double update) or immediately when PTEN = 0 or IUE = 1
(§14.12). Complementary pairs insert dead time from units A/B before each
pin is driven active; DTSxA/DTSxI select the unit per edge (§14.14).
P1OVDCON overrides the pins with POUT; the FLTA1 fault input (active low)
forces the FAOV states, with high-side priority in complementary mode, and
restores at the next period/half-period boundary in cycle-by-cycle and
latched modes (§14.15).

**QEI model.** x4 decoding on every edge and x2 on Phase A edges, using the
lead/lag table (§15.4); SWPAB swaps the phases. Match-reset modes roll
POS1CNT to 0 at MAXCNT and load MAXCNT at the zero underflow (§15.5.2).
Index-reset modes reset to 0 when travelling forward and load MAXCNT in
reverse, honouring POSRES and the IMV match value (§15.5.3). In index modes
POS1CNT = MAXCNT+1 (up) or 0xFFFF (down) sets CNTERR and, unless CEID is
set, raises QEIIF; further errors are ignored until CNTERR is cleared.
Timer mode (QEIM = 001) counts the internal prescaled clock or QEA edges,
with TQGATE, UDSRC and period match at MAXCNT (§15.6). The digital filter
(QEOUT/QECK) accepts a level after three equal filter clocks (§15.3).

**Interrupts.** The fabric now covers IFS3/IEC3 and IPC14/15, and selects
the highest-priority pending source (lower vector wins a tie). While wiring
it, the M1a vector table was corrected against DS70283H Table 7-1:
IC2/OC2/T2/T3 are vectors 13/14/15/16, not 12/13/14/15 (IFS0 bit 4 is
reserved, so the vector is `bit + 8`).

**Verification.** Unit tests cover the PWM time base modes, complementary
PWM with dead time, override, fault entry/release, the postscaler, the QEI
decoder in x2/x4, match/index resets, CNTERR/CEID, timer mode and the
digital filter; the real XC16 firmware `xc16-m2-pwm-qei.c` configures both
blocks and the integration test checks the register image, the PWM pads and
injected quadrature edges (`crates/core/tests/pic24_xc16_m2.rs`).

**Approximations** (documented, not guessed): active-high PWM polarity and
PWMPIN = 1 (erased-device FPOR) since configuration bits are not modelled;
center-aligned endpoint ticks keep both endpoints per Equation 14-2 (the
FRM does not pin the compare order at an endpoint); index recognition uses
the INDX rising edge plus the IMV match rather than latching a quadrature
transition (§15.5.3.1); diagonal quadrature transitions are ignored; PWM/QEI
inputs are sampled once per retired step.

### 3.4 ADC1 + SPI1 + I2C1 (M3, implemented 2026-10-23)

Sources: DS70183D (Section 16, ADC), DS70206 (Section 18, SPI),
DS70000195 (Section 19, I2C) and DS70283H Tables 4-11/4-13/4-15 (SFR maps),
7-1 (vectors) and 1-1 (pins).

**SFRs.** ADC1: `ADC1BUF0-15 0x0300-0x031E`, `AD1CON1 0x0320`,
`AD1CON2 0x0322`, `AD1CON3 0x0324`, `AD1CHS123 0x0326`,
`AD1CHS0 0x0328`, `AD1PCFGL 0x032C`, `AD1CSSL 0x0330`. SPI1:
`SPI1STAT 0x0240`, `SPI1CON1 0x0242`, `SPI1CON2 0x0244`, `SPI1BUF 0x0248`.
I2C1: `I2C1RCV 0x0200`, `I2C1TRN 0x0202`, `I2C1BRG 0x0204`,
`I2C1CON 0x0206`, `I2C1STAT 0x0208`, `I2C1ADD 0x020A`,
`I2C1MSK 0x020C`. Interrupts: `IFS1 0x0086` (MI2C1 bit 1, SI2C1 bit 0),
`IEC1 0x0096`, `IPC3 0x00AA` (AD1IP 6-4) and `IPC4 0x00AC`
(MI2C1IP 6-4, SI2C1IP 2-0); `IFS0` carries AD1IF (13), SPI1IF (10) and
SPI1EIF (9). Vectors: SPI1E 17, SPI1 18, ADC1 21, SI2C1 24, MI2C1 25.

**Pins.** AN0-AN8 = RA0, RA1, RB0-RB3, RC0-RC2 (DS70283H Table 1-1);
`AD1PCFGL` bit n = 1 makes ANn digital. SPI1 SDI/SDO/SCK/SS and I2C1
SDA/SCL are PPS-remappable inputs/outputs; this model does not route them
through pads (see approximations).

**ADC model.** `AD1CON1`: ADON, AD12B, FORM (00 unsigned integer,
01 signed integer, 10 unsigned fraction, 11 signed fraction), SSRC, SIMSAM,
ASAM, SAMP and DONE. Writing ADON 0→1 powers the ADC; sampling starts
automatically only when ASAM = 1, otherwise software sets SAMP (DS70183D
Register 16-1). With SSRC = 000 software clears SAMP to start the conversion;
SSRC = 111 uses the internal
counter (`SAMC × TAD`) and ASAM restarts sampling after each sequence.
SSRC = 010 is triggered by the TMR3 period match and SSRC = 011 by the
PWM1 period/half-period match; SSRC = 001 (INT0) is not driven because the
INT0 pin edge is not modelled. TAD = `(ADCS + 1) × TCY`; a 10-bit
conversion is 12 TAD and a 12-bit conversion 14 TAD (DS70183D §16.4.4).
FORM results are left-justified for the fractional modes. CHPS selects
1-4 channels per sequence (CH0 + CH1-3 from CH123SA); CSCNA walks the
AD1CSSL scan list for CH0; ALTS uses MUX A for the first sequence and MUX B
for the next (switching only after the sequence completes). Results
land in the 16-word buffer; BUFM splits it into two 8-word halves whose
fill status is the read-only BUFS bit. SMPI sets the interrupt period
(every SMPI+1 sequences, as DS70183D §16.4.6 defines for devices without
DMA). Negative inputs (CH0NA, CH123NA) subtract the selected ANx code,
clamped to the converter range; PCFG = 1 reads 0.

**SPI model.** Master writes to SPI1BUF start a transfer whose duration is
`bits × PPRE × SPRE` TCY (PPRE 1/4/16/64, SPRE 1-8; DS70206C Equation 18-1:
`FSCK = FCY / (PPRE × SPRE)`). MODE16 exchanges two bytes (high first). SPITBF stays set
while the word shifts; the received word lands in SPI1BUF, sets SPIRBF and
SPI1IF; an unread previous word sets SPIROV and SPI1EIF instead. A second
write while shifting is held in a one-deep TX slot. SPI1CON2 keeps the
framed-mode bits (the framing protocol itself is not driven). Slave mode
loads TXB on a SPI1BUF write and is clocked through the
`spi_slave_clock()` test seam. Attached `SpiDevice`s are called once per
byte (first device only; no SS routing), so the example declares a wire
loopback (`type: "loopback"`) that returns every byte.

**I2C model.** `I2C1CON` implements I2CEN, SEN, RSEN, PEN, RCEN and
ACKEN/ACKDT with the datasheet's hardware-clear behaviour; each operation
takes its wire time (`(BRG + 1)` TCY per SCL, nine SCL for a byte and
one for start/stop; the 1.111 MHz high-time correction term in the
DS70000195 BRG formula is omitted). A transmit byte is the address after a start/restart
(7-bit; `11110xx` starts a 10-bit prefix decode, but the `I2cDevice` slave
contract is a 7-bit `u8` address, so a 10-bit address cannot match a device
here) and data thereafter. Address matching uses
`I2cDevice::claims_address`, then `select_address`/`start`; writes go to
`write()`, RCEN reads from `read()`, and PEN calls `stop()`. ACKSTAT
reports a missing slave's NACK; RBF/TBF track the buffers; reading
I2C1RCV clears RBF; I2COV/IWCOL/BCL are clear-on-0. The TMP102 model is
attached by the session builder from the manifest's `i2c1` entries
(`type: "tmp102"`, pointer register + 12-bit left-justified temperature).
Slave mode is register-level plus test seams (`i2c_slave_address`,
`i2c_slave_write`, `i2c_slave_read`, `i2c_slave_stop`); there is no
external master model.

**Verification.** Unit tests cover all four FORM formats in 10- and 12-bit
modes, ASAM/SSRC = 111, the TMR3 trigger, the scan list, the split buffer,
SMPI, SPI byte/word transfers with the prescaler timing, SPIROV, the slave
seam, an I2C TMP102 temperature read, a missing-slave NACK, I2COV and the
slave seams, plus the IFS1/IPC3/IPC4 vectors. Booting the firmware also
added four base-ISA encodings the XC16 startup uses: 3-operand `SUB`,
`INC Ws,Wd` (C/DC/N/OV/Z), and the WREG forms of `CLR Wd`/`SETM Wd`
(no status flags; DS70283H Table 22-2 #15/#39/#69), pinned by xc16-as words. The real XC16 firmware
`xc16-m3-adc-spi-i2c.c` samples AN4, loops 0xA5 through SPI1 and reads the
TMP102; `crates/core/tests/pic24_xc16_m3.rs` checks the results,
`"M3 OK\n"` on UART1 and the register image.

**Approximations** (documented, not guessed): ADRC = 1 (internal RC)
reuses the `(ADCS + 1) × TCY` TAD formula because the RC frequency is not
characterized here; a conversion's leftover cycles at a step boundary are
not carried into the next channel; the ADC's INT0 trigger and the
simultaneous-sample (SIMSAM) sample instant are not modelled (the channels
convert sequentially); SPI SS/CS routing is absent (first device answers,
the example uses a wire loopback); I2C ACK bits are not delivered to the
slave model (the TMP102 does not need them); 10-bit addressing is decoded
but cannot match a 7-bit slave address; electrical pin mapping of
ANx/SPI/I2C is not routed through the pad model.

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
