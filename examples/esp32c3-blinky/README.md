# ESP32-C3 Super Mini Blinky

The canonical "hello world" for the ESP32-C3 Super Mini: bare-metal rv32imc
firmware that enables GPIO8 (the board's user LED) as an output and toggles it
forever, narrating each transition over UART0.

This is also the demo binary the LabWired Playground falls back to when a
shared or agent-generated C3 lab carries no firmware of its own, so shared
`esp32c3-supermini` labs run out of the box instead of dying with
"Cannot run: no firmware".

## Run it

```sh
labwired test --script examples/esp32c3-blinky/test-blink.yaml
```

Expected UART:

```
C3 BLINKY BOOT
LED ON
LED OFF
LED ON
...
```

## Rebuild the firmware

Requires the Espressif RISC-V GCC toolchain (`riscv32-esp-elf-gcc`, from
PlatformIO or ESP-IDF):

```sh
cd examples/esp32c3-blinky/firmware
make            # produces esp32c3_blinky.elf
```

The firmware drives the plain R/W `GPIO_OUT` / `GPIO_ENABLE` registers rather
than the `W1TS`/`W1TC` aliases: the C3's gpio block is the declarative register
file (no write-1-to-set side effects), and full-value writes are equally valid
on real silicon.

## Files

- `firmware/main.c` — GPIO8 blink + UART narration
- `firmware/startup.S`, `firmware/c3.ld` — bare-metal C3 boot (shared with the
  Leo air-quality example: `_start` sets gp/sp, copies `.data`, zeroes `.bss`)
- `firmware/c3_uart.{c,h}` — polled UART0 debug output
- `system.yaml` — C3 Super Mini system: `status_led` board_io on GPIO8
- `test-blink.yaml` — headless scenario: boot banner, LED transitions, and
  `GPIO_ENABLE` bit 8 asserted
