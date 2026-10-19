# dsPIC33FJ32MC204 — LabWired Example (M0 + M1a)

The first 16-bit PIC/dsPIC target in LabWired, brought up to **M0 + M1a**: boot
from a committed ELF, transmit over UART1, and run Timers T1/T2/T3 with the
interrupt path (IFS0/IEC0/IPC, vector frame, RETFIE) — deterministically.

| Item | Value |
| :--- | :--- |
| Chip | dsPIC33FJ32MC204 (16-bit DSC), 32 KB flash / 2 KB SRAM |
| Descriptor | `configs/chips/dspic33fj32mc204.yaml` |
| System | `system.yaml` → `configs/systems/dspic33fj32mc204-demo.yaml` |
| Firmware | `firmware/dspic33fj32mc204-uart-smoke.elf` (assembled by hand, see below) |
| Smoke | `io-smoke.yaml` — expects `H` on UART1 within 64 steps |

## Run it

```bash
# from the repository root
cargo build --release -p labwired-cli
./target/release/labwired test --script examples/dspic33fj32mc204/io-smoke.yaml
```

## The firmware

No open-source PIC24 assembler exists in CI, so the fixture is assembled in
`firmware/make-fixture-elf.py` from the DS70000157G encodings:

```
MOV  #'H', W0        ; 0020 0480
MOV  W0, 0x0224      ; 8811 20    → U1TXREG
NOP ×6
BRA  $-1             ; 0037 FFFF  → run ends at max_steps
```

Regenerate it with `python3 firmware/make-fixture-elf.py`. The ELF is a minimal
ELF32 (`e_machine = 118`, `EM_DSPIC30F`) with one `PT_LOAD` segment. The
"one instruction per 4-byte little-endian slot" layout is provisional until a
real XC16 toolchain is available — see `docs/pic24-m0-reference.md`.

## Scope and limitations

Modelled: reset/boot, `NOP`, `MOV #lit16`, `MOV Wns, f`, `MOV f, Wnd`,
`BRA Expr`, `RETFIE`, UART1 TX (`U1MODE`/`U1STA`/`U1TXREG`), Timers T1/T2/T3
(`TMR`/`PR`/`TCON`, TCKPS prescaler, 32-bit T2/T3 pair), interrupts
(`IFS0`/`IEC0`/`IPC0-2`, DISI/NSTDIS, two-word exception frame), `ACCA*`/`ACCB*`
SFR storage, 2 KB data SRAM, SFR shadow latch. Not modelled (each has its own
milestone): IC/OC (needs a pin model, M1b), DSP MAC (M1c), PWM, QEI, ADC, SPI,
I2C, PSV/EDS windows. Unsupported opcodes return a decode error — they are
never skipped silently.

See `VALIDATION.md` for the evidence and `REQUIRED_DOCS.md` for sources.
