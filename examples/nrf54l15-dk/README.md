# nRF54L15-DK — bare-metal C smoke firmware

Reference firmware for the [nRF54L15 board profile](../../docs/boards/nrf54l15.md).
Proves the chip profile boots and that UARTE20 EasyDMA and GPIO reach their
destinations.

**No Zephyr, no nRF Connect SDK, no CMSIS.** The only requirement is
`arm-none-eabi-gcc`. That is deliberate: the less code between reset and the
first UARTE byte, the more precisely a failure localises to the simulator rather
than to a vendor HAL.

## Build

```sh
make            # build build/nrf54l15-smoke.elf
make publish    # build and copy to ../../tests/fixtures/ (the committed fixture)
make clean
```

Toolchain on macOS: `brew install arm-none-eabi-gcc`.

## Run

```sh
cargo build -p labwired-cli --release
./target/release/labwired \
    --firmware tests/fixtures/nrf54l15-smoke.elf \
    --system configs/systems/nrf54l15dk.yaml \
    --max-steps 200000
```

Expected output:

```
Initial PC: 0x44, SP: 0x20040000
nRF54L15 boot OK
core=cortex-m33 rram=1524K ram=256K
uarte20@0x500C6000 gpio2@0x50050400
```

`SP: 0x20040000` is worth reading as an assertion, not decoration: it is
`0x20000000 + 256 KB`, so a wrong RAM size in the chip profile shows up right
here rather than as mysterious stack corruption later.

## What it exercises

| Step | Exercises |
|---|---|
| Reset → vector table at RRAM `0x0` | RRAM based at 0, ELF load, initial SP/PC |
| `.data` copy / `.bss` zero | RRAM→RAM reads and RAM writes |
| GPIO P2 DIRSET/OUTSET (LED0 = P2.09) | GPIO model, per-port widths, base-address convention |
| UARTE20 EasyDMA TX ×3 | PSEL/BAUDRATE/ENABLE, TXD.PTR/MAXCNT, TASKS_STARTTX → EVENTS_ENDTX |

## Two details that are easy to get wrong

**The TX buffer must live in RAM.** EasyDMA reads it over the bus, so a string
literal in RRAM would fault on real silicon. `main.c` copies into a `static char
tx_buf[]` in `.data` for exactly this reason — a simulator that allowed the RRAM
version would be modelling the part too leniently.

**GPIO base ≠ devicetree address.** A Nordic GPIO DT node points at the OUT
register (`peripheral_base + 0x500`). `nrf54l15.h` therefore defines
`GPIO_P2_BASE` as `0x5004FF00`, not the DT's `0x50050400`, and uses
peripheral-relative offsets. Mixing the two conventions is silent: UART keeps
working and only the LED stays dark. See
[the board doc](../../docs/boards/nrf54l15.md#the-gpio-base-address-trap).

## Files

```
Makefile          arm-none-eabi build (with -MMD -MP header deps)
nrf54l15.ld       RRAM 1524K @ 0x0, RAM 256K @ 0x20000000
src/startup.c     ARMv8-M vector table, .data/.bss init, Reset_Handler
src/nrf54l15.h    hand-checked register subset (not generated CMSIS)
src/main.c        LED + UARTE banner
```

## Validation

- `crates/core/tests/nrf54l15_boot.rs` — 7 bus-level conformance tests
- `firmware_survival::test_nrf54l15_smoke_survival` — boots this ELF, asserts the banner
- `firmware_survival::test_nrf54l15_lights_dk_led0` — boots this ELF, asserts
  DIR and OUT **at the pin**, because the banner alone does not catch a GPIO
  base-address error
