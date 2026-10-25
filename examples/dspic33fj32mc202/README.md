# dsPIC33FJ32MC202 — LabWired Example (M0 + M1 + M2)

The 28-pin sibling of the onboarded **dsPIC33FJ32MC204**, brought up to
**M0 + M1 + M2**: boot from real XC16-compiled ELFs, transmit over UART1,
drive the 28-pin GPIO package (RA0-RA4, RB0-RB15, no PORTC), and run the
shared T1/PWM1/QEI1 peripherals on 202 package pins, deterministically.
It shares the 204's PIC24/dsPIC33F core, 32 KB flash, 2 KB SRAM, UART1
register map and instruction set.

| Item | Value |
| :--- | :--- |
| Chip | dsPIC33FJ32MC202 (16-bit DSC), 32 KB flash / 2 KB SRAM |
| Descriptor | `configs/chips/dspic33fj32mc202.yaml` |
| System | `system.yaml` → `configs/systems/dspic33fj32mc202-demo.yaml` |
| Firmware | `firmware/dspic33fj32mc202-xc16-uart-smoke.elf` (M0), `...-m1-gpio.elf` (M1) and `...-m2-pwm-qei.elf` (M2), real XC16 v1.70 builds |
| Smoke | `io-smoke.yaml` — expects `HI` on UART1 within 400 steps |

## Run it

```bash
# from the repository root
cargo build --release -p labwired-cli
./target/release/labwired test --script examples/dspic33fj32mc202/io-smoke.yaml
```

## The firmware

- **M0** — `firmware/xc16-uart-smoke.c` configures UART1, transmits `HI`,
  then toggles LATB0 forever. Booted by
  `crates/core/tests/pic24_xc16_202_smoke.rs`.
- **M1** — `firmware/xc16-m1-gpio.c` transmits `G1`, makes PORTA/PORTB
  outputs and toggles RA0/RB0 forever. Booted by
  `crates/core/tests/pic24_xc16_202_m1.rs`, which also checks the 28-pin
  masks (`TRISA = 0x001F`, `TRISB = 0xFFFF`, `TRISC = 0`) and the pad levels.
- **M2** — `firmware/xc16-m2-pwm-qei.c` waits for T1IF, configures PWM1
  (complementary pair on RB14/RB15, 2 TCY dead time) and QEI1 (x4 index
  reset, QEA/QEB/INDX on RB0/RB1/RB2), then writes `0xBEEF` to 0x0E00.
  Booted by `crates/core/tests/pic24_xc16_202_m2.rs`, which injects
  quadrature edges, checks the PWM pads and the direction-aware index reset.

Rebuild the ELF (Linux x86-64 container; XC16 itself is proprietary and is
not redistributed — only its output is committed):

```bash
docker run --rm -v "$PWD/examples/dspic33fj32mc202/firmware:/work" -w /work \
  fstlx/xc16:latest bash -lc '
    xc16-gcc -mcpu=33FJ32MC202 -O0 -Wall \
      -Wl,-T,/opt/microchip/xc16/support/dsPIC33F/gld/p33FJ32MC202.gld \
      -o dspic33fj32mc202-xc16-uart-smoke.elf xc16-uart-smoke.c
    xc16-gcc -mcpu=33FJ32MC202 -O0 -Wall \
      -Wl,-T,/opt/microchip/xc16/support/dsPIC33F/gld/p33FJ32MC202.gld \
      -o dspic33fj32mc202-xc16-m1-gpio.elf xc16-m1-gpio.c'
```

## Scope and limitations

Modelled: reset/boot, UART1 TX, and the shared PIC24 base ISA / peripheral
models documented in [`docs/pic24-m0-reference.md`](../../docs/pic24-m0-reference.md)
(the 202 and 204 are the same silicon family; the 204's M1–M3 UART, timers,
GPIO, PWM/QEI and ADC/SPI/I²C models apply to the shared core).

M1 models the 202 **package masks**: PORTA presents RA0-RA4, PORTB RB0-RB15
and PORTC is absent (`TRISC` reads 0). The chip descriptor's three
`pic24_gpio` pads carry those masks, and the CPU derives its TRIS/LAT/ODC
masks from them. M2 validates the shared T1/PWM1/QEI1 models on the pins the
28-pin package actually has (RB14/RB15 for the PWM pair; RB0/RB1/RB2 for
QEA/QEB/INDX). The AN0–AN5 analog set (no AN6–AN8) and ADC/SPI/I²C remain
for the next milestone. Do not read the 204's 44-pin pad mask on a 202 board.

See `VALIDATION.md` for the evidence trail and exact commands.
