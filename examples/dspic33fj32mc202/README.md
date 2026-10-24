# dsPIC33FJ32MC202 — LabWired Example (M0)

The 28-pin sibling of the onboarded **dsPIC33FJ32MC204**, brought up to
**M0**: boot from a real XC16-compiled ELF and transmit over UART1,
deterministically. It shares the 204's PIC24/dsPIC33F core, 32 KB flash,
2 KB SRAM, UART1 register map and instruction set.

| Item | Value |
| :--- | :--- |
| Chip | dsPIC33FJ32MC202 (16-bit DSC), 32 KB flash / 2 KB SRAM |
| Descriptor | `configs/chips/dspic33fj32mc202.yaml` |
| System | `system.yaml` → `configs/systems/dspic33fj32mc202-demo.yaml` |
| Firmware | `firmware/dspic33fj32mc202-xc16-uart-smoke.elf` (real XC16 v1.70 build) |
| Smoke | `io-smoke.yaml` — expects `HI` on UART1 within 400 steps |

## Run it

```bash
# from the repository root
cargo build --release -p labwired-cli
./target/release/labwired test --script examples/dspic33fj32mc202/io-smoke.yaml
```

## The firmware

`firmware/xc16-uart-smoke.c` configures UART1, transmits `HI`, then toggles
LATB0 forever. It exercises the compiler's real reset vector, startup code,
call/return sequences and the linker's default IVT, not a hand-written
fixture. Booted by `crates/core/tests/pic24_xc16_202_smoke.rs`.

Rebuild the ELF (Linux x86-64 container; XC16 itself is proprietary and is
not redistributed — only its output is committed):

```bash
docker run --rm -v "$PWD/examples/dspic33fj32mc202/firmware:/work" -w /work \
  fstlx/xc16:latest bash -lc '
    xc16-gcc -mcpu=33FJ32MC202 -O0 -Wall \
      -Wl,-T,/opt/microchip/xc16/support/dsPIC33F/gld/p33FJ32MC202.gld \
      -o dspic33fj32mc202-xc16-uart-smoke.elf xc16-uart-smoke.c'
```

## Scope and limitations

Modelled: reset/boot, UART1 TX, and the shared PIC24 base ISA / peripheral
models documented in [`docs/pic24-m0-reference.md`](../../docs/pic24-m0-reference.md)
(the 202 and 204 are the same silicon family; the 204's M1–M3 UART, timers,
GPIO, PWM/QEI and ADC/SPI/I²C models apply to the shared core).

Not modelled in M0: the 202 **package** differences — 28 pins instead of 44,
AN0–AN5 only (no AN6–AN8), and the smaller PWM/QEI/port pin set. The chip
descriptor declares no bus peripherals yet for the same reason: package-level
pin maps land with the 202's GPIO milestone. Do not read the 204's 44-pin
pad mask on a 202 board.

See `VALIDATION.md` for the evidence trail and exact commands.
