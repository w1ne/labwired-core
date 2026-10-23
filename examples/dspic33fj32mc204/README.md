# dsPIC33FJ32MC204 — LabWired Example (M0 … M3)

The first 16-bit PIC/dsPIC target in LabWired, brought up to **M0 + M1a +
M1b + M1c + M2 + M3**: boot from a committed ELF, transmit over UART1, run
Timers T1/T2/T3 with the interrupt path, drive/sense the real GPIO pads with
input capture and output compare, run the DSP MAC class, the Motor Control
PWM/QEI pair, and the ADC/SPI/I2C peripherals with a TMP102 on the I²C bus —
deterministically.

| Item | Value |
| :--- | :--- |
| Chip | dsPIC33FJ32MC204 (16-bit DSC), 32 KB flash / 2 KB SRAM |
| Descriptor | `configs/chips/dspic33fj32mc204.yaml` |
| System | `system.yaml` → `configs/systems/dspic33fj32mc204-demo.yaml` |
| Firmware | `firmware/dspic33fj32mc204-uart-smoke.elf` (hand-assembled fixture) and `firmware/dspic33fj32mc204-xc16-uart-smoke.elf` (real XC16 v1.70 build) |
| Smoke | `io-smoke.yaml` — expects `H` on UART1 within 64 steps |

## Run it

```bash
# from the repository root
cargo build --release -p labwired-cli
./target/release/labwired test --script examples/dspic33fj32mc204/io-smoke.yaml
```

## The firmware

Five committed ELFs exercise the chip:

1. **Hand-assembled fixture** (`firmware/make-fixture-elf.py`, DS70000157G
   encodings) — the minimal M0 smoke: `MOV #'H', W0` / `MOV W0, 0x0224`
   (U1TXREG) / `NOP ×6` / `BRA $-1`, with the standard reset vector
   (`GOTO 0x200` at PC 0). Regenerate with
   `python3 firmware/make-fixture-elf.py`.
2. **Real XC16 build** (`firmware/xc16-uart-smoke.c` →
   `dspic33fj32mc204-xc16-uart-smoke.elf`) — compiled with Microchip XC16
   v1.70 and the stock `p33FJ32MC204.gld` linker script. It configures UART1,
   transmits `HI`, then toggles LATB0 forever; it exercises the compiler's
   real startup (`__reset`/`__psv_init`/`__crt_start_mode`), call/return
   sequences, the IVT and the reset GOTO. Booted by
   `crates/core/tests/pic24_xc16_elf_boot.rs`.
3. **Real XC16 DSP build** (`firmware/xc16-dsp-mac.c` →
   `dspic33fj32mc204-xc16-dsp-mac.elf`) — the same toolchain, a fixed
   MAC-class sequence taken from the DS70000157G worked examples
   (`LAC`/`MAC` with X/Y prefetch, `MPY`/`MSC` with write-back, `SAC`,
   `SAC.R`, normal-saturation clamp). Booted by
   `crates/core/tests/pic24_xc16_dsp.rs`, which checks the accumulators,
   W13, the prefetch registers and the SR flags.
4. **Real XC16 M2 build** (`firmware/xc16-m2-pwm-qei.c` →
   `dspic33fj32mc204-xc16-m2-pwm-qei.elf`) — configures an edge-aligned
   complementary PWM1 pair with a 2 TCY dead time and a x4 index-reset
   QEI1. Booted by `crates/core/tests/pic24_xc16_m2.rs`, which checks the
   register image, the PWM pads and injected quadrature edges.
5. **Real XC16 M3 build** (`firmware/xc16-m3-adc-spi-i2c.c` →
   `dspic33fj32mc204-xc16-m3-adc-spi-i2c.elf`) — samples AN4, shifts 0xA5
   through SPI1 (the manifest's `loopback` device), reads the TMP102
   temperature over I2C1 and prints `M3 OK` on UART1. Booted by
   `crates/core/tests/pic24_xc16_m3.rs`; the `system.yaml` declares the
   TMP102 on `i2c1` and the SPI loopback on `spi1`.

Rebuild the XC16 ELFs (Linux x86-64 container; XC16 itself is proprietary
and not redistributed — only its output is committed):

```bash
docker run --rm -v "$PWD/examples/dspic33fj32mc204/firmware:/work" -w /work \
  fstlx/xc16:latest bash -lc '
    xc16-gcc -mcpu=33FJ32MC204 -O0 -Wall \
      -Wl,-T,/opt/microchip/xc16/support/dsPIC33F/gld/p33FJ32MC204.gld \
      -o dspic33fj32mc204-xc16-uart-smoke.elf xc16-uart-smoke.c
    xc16-gcc -mcpu=33FJ32MC204 -O0 -Wall \
      -Wl,-T,/opt/microchip/xc16/support/dsPIC33F/gld/p33FJ32MC204.gld \
      -o dspic33fj32mc204-xc16-dsp-mac.elf xc16-dsp-mac.c
    xc16-gcc -mcpu=33FJ32MC204 -O0 -Wall \
      -Wl,-T,/opt/microchip/xc16/support/dsPIC33F/gld/p33FJ32MC204.gld \
      -o dspic33fj32mc204-xc16-m2-pwm-qei.elf xc16-m2-pwm-qei.c
    xc16-gcc -mcpu=33FJ32MC204 -O0 -Wall \
      -Wl,-T,/opt/microchip/xc16/support/dsPIC33F/gld/p33FJ32MC204.gld \
      -o dspic33fj32mc204-xc16-m3-adc-spi-i2c.elf xc16-m3-adc-spi-i2c.c'
```

(The explicit `-T` matters: this container's `-p` device selection silently
loads a dsPIC30F script, which fails on dsPIC33F SFR symbols.)

The ELF layout is the XC16 one **verified against this build**:
PC-unit addresses (2 per instruction), executable sections stored as 4-byte
slots (24-bit word + pad byte), the reset GOTO at PC 0, and the IVT holding
24-bit ISR addresses as data. See `docs/pic24-m0-reference.md` §3.1.

## Scope and limitations

Modelled: reset/boot (PC = 0 → reset GOTO), the base integer ISA used by
real XC16 code (`NOP`; `MOV #lit16`/`MOV f,Wn`/`MOV Wns,f`; `MOV.B #lit8`;
`MOV{.B} Ws,Wd` with indirect/post/pre modes; `ADD`/`AND`/`IOR` in the
3-operand and literal forms; `CLR`/`SETM`; `CP0`; `BSET`/`BCLR`/`BTG`;
`SE`; `LSR #lit4`; `BRA` (all conditions)/`RCALL`/`CALL`/`GOTO`;
`RETURN`/`RETLW`/`LNK`/`ULNK`/`RETFIE`), the DSP MAC class (`MAC`/`MPY`/
`MPY.N`/`MSC` including the square forms, `CLR`, `MOVSAC`, `LAC`, `SAC`,
`SAC.R`, X/Y prefetches with the W12 register-offset mode, accumulator
write-back, CORCON fractional/integer, signed/unsigned, rounding and
saturation modes, OA/OB/SA/SB/OAB/SAB), Motor Control PWM 1 (edge-aligned,
single-event and center-aligned time base, three duty generators with
buffering/IUE, complementary/independent pairs with dead time, P1OVDCON
override, FLTA1 fault states, PWM1IF postscaling) and QEI1 (x2/x4 decoding,
POS1CNT/MAX1CNT, match/index resets with IMV, UPDN, CNTERR/CEID, digital
filter, 16-bit timer mode), ADC1 (10/12-bit, FORM formats, SAMP/DONE/ASAM,
manual/TMR3/PWM1/internal-counter triggers, CHPS sequences, CSCNA scan,
BUFM split buffer, SMPI interrupts, TAD/SAMC timing), SPI1 (master/slave
registers, 8/16-bit, SPRE/PPRE timing, SPIROV, attached devices with the
loopback harness) and I2C1 (master start/restart/stop, address/data,
ACKSTAT, BRG timing, TMP102 attach, slave register/test seams), UART1 TX
(`U1MODE`/`U1STA`/`U1TXREG`), Timers T1/T2/T3 (`TMR`/`PR`/`TCON`, TCKPS
prescaler, 32-bit T2/T3 pair), interrupts (`IFS0`/`IEC0`/`IPC0-2`,
`IFS1`/`IEC1`/`IPC3-4`,
DISI/NSTDIS, IVT/AIVT vector fetch of the 24-bit ISR address, two-word
exception frame), GPIO ports A/B/C (`TRIS`/`PORT`/`LAT`/`ODC`), the PPS
slice for IC1/IC2 + OC1/OC2 (`RPINR7`, `RPOR0-12`), input capture
(edge/prescaler modes, 4-deep FIFO, ICI, ICOV/ICBNE), output compare
(one-shot/toggle/delayed/continuous pulse/PWM driving the routed pin),
`ACCA*`/`ACCB*` SFR storage, 2 KB data SRAM, SFR shadow latch.

Not modelled (each has its own milestone): `TBLRD*`/`TBLW*` (PSV), `ADDC`/
`SUBB`/`CPB` sticky-Z forms, register-offset `[Ws+Wb]` addressing,
`REPEAT`/`DO`, `PUSH`/`POP`, `RESET`/`BREAK`, CNPU pull-ups, the DSP
accumulator arithmetic (`ADD`/`SUB`/`MAX`/`MIN`/`SFTAC`/`FBCL` on
ACCA/ACCB), PSV/EDS windows.
Unsupported opcodes return a decode error — they are never skipped silently.

Known M1c approximations (everything else is pinned by a DS70000157G
example — see the unit tests): `LAC`'s shifter is 32-bit (its PRM examples
1–2 pin that) and with super saturation enabled an overflowing left shift
keeps the exact 40-bit result (no PRM example covers that combination);
`MPY.N` lists only OA/OB/OAB as affected and is followed literally (no
saturation, no SA/SB); accumulator write-back applies SATDW like `SAC.R`
(no write-back example saturates).

Known M3 approximations (documented, not guessed): ADRC = 1 reuses the
`(ADCS + 1) × TCY` TAD formula (the internal RC frequency is not
characterized); leftover conversion cycles at a step boundary are not
carried into the next channel; the INT0 ADC trigger and the simultaneous
(SIMSAM) sample instant are not modelled (channels convert sequentially);
SPI has no SS/CS routing (the first device answers; the example uses a wire
loopback); I²C ACK bits are not delivered to the slave model; 10-bit
addressing is decoded but cannot match a 7-bit slave address; ANx/SPI/I²C
pin electrical mapping is not routed through the pad model.

Known M2 approximations (documented, not guessed): PWM polarity is
active-high with PWMPIN = 1 (the erased-device FPOR values; configuration
bits are not modelled); center-aligned counting keeps both endpoint ticks
per Equation 14-2 and the compare uses the phase direction before the
endpoint flip (the FRM does not pin that order); QEI index recognition uses
the INDX rising edge plus the IMV match instead of latching a quadrature
transition (DS70208C §15.5.3.1); diagonal quadrature transitions are
ignored; PWM/QEI inputs are sampled once per retired step.

See `VALIDATION.md` for the evidence and `REQUIRED_DOCS.md` for sources.
