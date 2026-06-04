# PlatformIO Integration

Run PlatformIO unit tests inside the LabWired deterministic simulator instead of
on physical hardware — no board, no debug probe, no upload. This is the same
[external-simulator mechanism](https://docs.platformio.org/en/latest/advanced/unit-testing/simulators/renode.html)
PlatformIO documents for Renode and QEMU, pointed at LabWired.

A complete, runnable example lives in
[`examples/platformio/nrf52840-unity/`](../examples/platformio/nrf52840-unity/).

## How it works

PlatformIO's `pio test` builds a Unity test firmware, then — when
`test_testing_command` is set — runs that command instead of uploading, and
parses the command's **stdout** for Unity results (`file:line:name:PASS|FAIL`
and the `N Tests M Failures K Ignored` summary).

LabWired plugs straight into that contract:

```
pio test
  │  1. builds Unity test firmware  ->  ${platformio.build_dir}/<env>/firmware.elf
  │  2. runs test_testing_command:
  │
  │        labwired test --no-key --script labwired.test.yaml --firmware <firmware.elf>
  │
  ▼  3. LabWired boots the ELF on the simulated MCU. Unity writes results to the
  │     UART; LabWired mirrors UART -> stdout.
  ▼  4. PlatformIO parses stdout for Unity PASS/FAIL and reports per-test results.
```

## platformio.ini

```ini
[platformio]              ; required so ${platformio.*} variables resolve

[env:nrf52840_dk]
platform = nordicnrf52
board = nrf52840_dk

test_testing_command =
    labwired
    test
    --no-key
    --script
    labwired.test.yaml
    --firmware
    ${platformio.build_dir}/${this.__env__}/firmware.elf
```

The `labwired` CLI must be on `PATH`. The `--script` is a normal LabWired test
script; its `inputs.firmware` is a non-empty placeholder that `--firmware`
overrides with the freshly-built ELF, so the same script is reusable.

## Routing test output to the mirrored UART

The only firmware-side requirement is that Unity's output reaches the UART that
the LabWired chip model mirrors to stdout (e.g. UART0 on nRF52840). With the
Unity framework, implement the standard transport hooks:

```c
void unittest_uart_begin(void)     { uart_init(); }
void unittest_uart_putchar(char c) { uart_putc(c); }   // writes UART0 TXD
void unittest_uart_flush(void)     {}
void unittest_uart_end(void)       {}
```

## Notes for no-framework (bare-metal) test images

The example builds with no framework for an instant, deterministic boot. Two
things are needed that a full framework would otherwise provide:

- **`test/unity_config.h`** — PlatformIO requires a custom Unity config when
  `framework` is empty. It wires `UNITY_OUTPUT_CHAR` to the transport hooks
  above.
- **`-D UNITY_EXCLUDE_SETJMP_H`** — Unity's default `TEST_PROTECT()`/`TEST_ABORT()`
  use `setjmp`/`longjmp`, which need C-runtime support a minimal freestanding
  image lacks. Excluding it makes a failing assertion record-and-continue.

Firmware built with Arduino/Zephyr/mbed needs neither — just route that
framework's test serial output to the mirrored UART.

## CI

The run is deterministic and headless, so it drops into any CI runner:

```yaml
- run: pio test -e nrf52840_dk
```
