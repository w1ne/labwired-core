# PlatformIO unit tests in the LabWired simulator (nRF52840)

Run your PlatformIO **unit tests inside the LabWired deterministic simulator**
instead of on physical hardware — no board, no debug probe, no upload. This is
the same mechanism PlatformIO documents for
[Renode](https://docs.platformio.org/en/latest/advanced/unit-testing/simulators/renode.html)
and QEMU, pointed at LabWired.

```
pio test -e nrf52840_dk
```

```
Testing...
test/test_smoke/test_main.c:50: test_addition         [PASSED]
test/test_smoke/test_main.c:51: test_uart_is_enabled  [PASSED]
test/test_smoke/test_main.c:52: test_string_length    [PASSED]
-------------- nrf52840_dk:test_smoke [PASSED] Took 2.12 seconds --------------
================== 3 test cases: 3 succeeded in 00:00:02.122 ==================
```

## How it works

```
pio test
  │  1. builds Unity test firmware  ->  .pio/build/nrf52840_dk/firmware.elf
  │  2. runs `test_testing_command` (platformio.ini):
  │
  │        labwired test --script labwired.test.yaml --firmware <firmware.elf>
  │
  ▼  3. LabWired boots the ELF on a simulated nRF52840 (Cortex-M4).
       Unity writes results to UART0; LabWired streams UART0 -> stdout.
  ▼  4. PlatformIO parses stdout for Unity PASS/FAIL and reports.
```

The only glue required is the three Unity output hooks in
[`test/test_smoke/test_main.c`](test/test_smoke/test_main.c), which send Unity's
characters to UART0 — the port LabWired mirrors to stdout:

```c
void unittest_uart_begin(void)      { uart_init(); }
void unittest_uart_putchar(char c)  { uart_putc(c); }
void unittest_uart_flush(void)      {}
void unittest_uart_end(void)        {}
```

## Prerequisites

1. **PlatformIO Core** and the **nordicnrf52** platform:
   ```
   pio pkg install -g -p nordicnrf52
   ```
2. The **`labwired` CLI** on your `PATH`. Build it from the LabWired core repo:
   ```
   cargo build --release -p labwired-cli
   ln -s "$(pwd)/target/release/labwired" ~/.local/bin/labwired
   ```

## Files

| File | Role |
|---|---|
| `platformio.ini` | Wires `test_testing_command` to `labwired`. The whole integration. |
| `labwired.test.yaml` | LabWired run budget + system manifest. `inputs.firmware` is a placeholder overridden by `--firmware`. |
| `nrf52840.system.yaml` | Points at the nRF52840 chip model in `configs/chips/`. |
| `test/test_smoke/test_main.c` | Unity test suite + the UART output transport hooks. |
| `test/unity_config.h` | Custom Unity config (required for a no-framework build); wires Unity output to the UART transport. |
| `src/` | Bare-metal startup, vector table, and UART0 driver. |
| `nrf52840.ld` | Linker script (flash @ 0x0, RAM @ 0x20000000). |

## Bare-metal note: `UNITY_EXCLUDE_SETJMP_H`

`platformio.ini` builds with `-D UNITY_EXCLUDE_SETJMP_H`. Unity's default
`TEST_PROTECT()`/`TEST_ABORT()` use `setjmp`/`longjmp`, which need C-runtime
support this minimal freestanding image doesn't provide. Excluding it makes a
failing assertion record the failure and continue, rather than abort — which is
exactly what you want for a deterministic, no-libc test image. Firmware built
with a full framework (Arduino/Zephyr) does not need this flag.

## Why no framework?

The example builds a tiny bare-metal image so it boots instantly and
deterministically in the model. The same `test_testing_command` pattern works
with Arduino/Zephyr firmware — just make sure the framework's test output is
routed to the UART that the chip model mirrors to stdout (UART0 here).

## CI

The run is deterministic and headless, so it drops straight into CI:

```yaml
- run: pio test -e nrf52840_dk
```
