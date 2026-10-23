# dsPIC33FJ32MC204 — LabWired Example (M0 + M1a + M1b)

The first 16-bit PIC/dsPIC target in LabWired, brought up to **M0 + M1a +
M1b**: boot from a committed ELF, transmit over UART1, run Timers T1/T2/T3
with the interrupt path, and drive/sense the real GPIO pads with input
capture and output compare — deterministically.

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

Two committed ELFs exercise the chip:

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

Rebuild the XC16 ELF (Linux x86-64 container; XC16 itself is proprietary and
not redistributed — only its output is committed):

```bash
docker run --rm -v "$PWD/examples/dspic33fj32mc204/firmware:/work" -w /work \
  fstlx/xc16:latest bash -lc '
    xc16-gcc -mcpu=33FJ32MC204 -O0 -Wall \
      -Wl,-T,/opt/microchip/xc16/support/dsPIC33F/gld/p33FJ32MC204.gld \
      -o dspic33fj32mc204-xc16-uart-smoke.elf xc16-uart-smoke.c'
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
`RETURN`/`RETLW`/`LNK`/`ULNK`/`RETFIE`), UART1 TX
(`U1MODE`/`U1STA`/`U1TXREG`), Timers T1/T2/T3 (`TMR`/`PR`/`TCON`, TCKPS
prescaler, 32-bit T2/T3 pair), interrupts (`IFS0`/`IEC0`/`IPC0-2`,
DISI/NSTDIS, IVT/AIVT vector fetch of the 24-bit ISR address, two-word
exception frame), GPIO ports A/B/C (`TRIS`/`PORT`/`LAT`/`ODC`), the PPS
slice for IC1/IC2 + OC1/OC2 (`RPINR7`, `RPOR0-12`), input capture
(edge/prescaler modes, 4-deep FIFO, ICI, ICOV/ICBNE), output compare
(one-shot/toggle/delayed/continuous pulse/PWM driving the routed pin),
`ACCA*`/`ACCB*` SFR storage, 2 KB data SRAM, SFR shadow latch.

Not modelled (each has its own milestone): `TBLRD*`/`TBLW*` (PSV), `ADDC`/
`SUBB`/`CPB` sticky-Z forms, register-offset `[Ws+Wb]` addressing,
`REPEAT`/`DO`, `PUSH`/`POP`, `RESET`/`BREAK`, CNPU pull-ups, DSP MAC
(M1c), Motor Control PWM/QEI (M2), ADC/SPI/I2C (M3), PSV/EDS windows.
Unsupported opcodes return a decode error — they are never skipped silently.

See `VALIDATION.md` for the evidence and `REQUIRED_DOCS.md` for sources.
