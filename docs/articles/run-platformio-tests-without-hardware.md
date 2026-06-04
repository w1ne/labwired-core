# Run your PlatformIO unit tests without hardware — or anything installed

You wrote firmware. You wrote unit tests. Now you need a board, a USB cable, a
debug probe, and a free hand to press reset — for every test run, on every push.

You don't.

This guide shows how to run your PlatformIO `pio test` suite inside the
**LabWired simulator** — a deterministic digital twin of your MCU. Same tests,
same Unity output, same green checkmark. No board. And in the cloud path below,
**nothing installed on your machine at all**.

It plugs into PlatformIO the same way [Renode and QEMU
do](https://docs.platformio.org/en/latest/advanced/unit-testing/simulators/renode.html):
through the built-in `test_testing_command` hook. PlatformIO doesn't need to
know anything about LabWired — it just runs a command and reads the Unity
results from its output.

---

## The 60-second version (zero install)

1. Fork [`w1ne/labwired-core`](https://github.com/w1ne/labwired-core).
2. Drop this file into `.github/workflows/firmware-tests.yml`:

   ```yaml
   name: Firmware tests (LabWired simulator)
   on: [push, pull_request]
   jobs:
     unit-tests:
       runs-on: ubuntu-latest
       steps:
         - uses: actions/checkout@v4
         - name: Install PlatformIO
           run: pipx install platformio
         - uses: dtolnay/rust-toolchain@stable
         - name: Build the LabWired CLI and put it on PATH
           run: |
             cargo build --release -p labwired-cli
             echo "$PWD/target/release" >> "$GITHUB_PATH"
         - name: Run unit tests in the simulator
           working-directory: examples/platformio/nrf52840-unity
           run: pio test -e nrf52840_dk
   ```

3. Push.

GitHub's runner installs PlatformIO, builds the firmware, runs it in the
simulator, and reports per-test PASS/FAIL. Your laptop did nothing. You get:

```
test/test_smoke/test_main.c:50: test_addition         [PASSED]
test/test_smoke/test_main.c:51: test_uart_is_enabled  [PASSED]
test/test_smoke/test_main.c:52: test_string_length    [PASSED]
================== 3 test cases: 3 succeeded in 00:00:01.4 ==================
```

> Prefer a terminal? Open the repo in a **GitHub Codespace** and run
> `cd examples/platformio/nrf52840-unity && pio test`. Still nothing installed
> locally — it runs in your browser.

The runnable project is at
[`examples/platformio/nrf52840-unity/`](../../examples/platformio/nrf52840-unity/).

---

## How it works

```
pio test
  │  1. builds the Unity test firmware  ->  firmware.elf
  │  2. instead of uploading, runs your test_testing_command:
  │
  │        labwired test --script labwired.test.yaml --firmware <firmware.elf>
  │
  ▼  3. LabWired boots the ELF on a simulated MCU (a real Cortex-M core model).
  │     Unity writes results to the UART; LabWired mirrors the UART to stdout.
  ▼  4. PlatformIO reads stdout, parses Unity PASS/FAIL, and reports each test.
```

Two deterministic, headless processes, one pipe. No flashing, no flakiness, no
"is the board plugged in?"

---

## Wiring it into your own project

### 1. The `platformio.ini` hook

```ini
[platformio]                 ; required so the ${platformio.*} vars below resolve

[env:nrf52840_dk]
platform = nordicnrf52
board = nrf52840_dk

test_testing_command =
    labwired
    test
    --script
    labwired.test.yaml
    --firmware
    ${platformio.build_dir}/${this.__env__}/firmware.elf
```

That's the whole integration. `pio test` builds `firmware.elf`, then runs this
command and parses its output.

### 2. The test script

`labwired.test.yaml` tells LabWired which MCU to simulate and how long to run:

```yaml
schema_version: "1.0"
inputs:
  firmware: "PLACEHOLDER"          # overridden by --firmware from PlatformIO
  system: "nrf52840.system.yaml"   # which chip model to load
limits:
  max_steps: 2000000
  wall_time_ms: 15000
assertions:
  - expected_stop_reason: "max_steps"
```

`--firmware` (passed by PlatformIO) overrides the placeholder, so the same
script is reusable across builds.

### 3. Route Unity output to the mirrored UART

LabWired mirrors one UART to stdout. Point Unity's output at it. With the Unity
framework you implement the standard transport hooks:

```c
void unittest_uart_begin(void)     { uart_init(); }
void unittest_uart_putchar(char c) { uart_putc(c); }   // writes UART0 TX
void unittest_uart_flush(void)     {}
void unittest_uart_end(void)       {}
```

If you're using the Arduino, Zephyr, or mbed framework, you don't write these —
just make sure the framework's test serial is the UART the model mirrors
(`Serial1` / UART0 on most boards).

---

## Running it locally (optional)

If you'd rather run on your own machine:

```bash
# PlatformIO
pipx install platformio

# The LabWired CLI (from the labwired-core checkout)
cargo build --release -p labwired-cli
export PATH="$PWD/target/release:$PATH"

# Run the tests in the simulator
cd examples/platformio/nrf52840-unity
pio test -e nrf52840_dk
```

---

## Bare-metal notes (skip if you use a framework)

The example builds a no-framework image so it boots instantly and
deterministically. Two small things a framework would otherwise hand you:

- **`test/unity_config.h`** — PlatformIO requires a custom Unity config when
  `framework` is empty; it routes `UNITY_OUTPUT_CHAR` to the UART hooks above.
- **`-D UNITY_EXCLUDE_SETJMP_H`** — Unity's default `TEST_PROTECT`/`TEST_ABORT`
  use `setjmp`/`longjmp`, which need a C runtime a minimal freestanding image
  doesn't carry. Excluding it makes a failing assertion record-and-continue.

---

## Why simulate?

- **Determinism** — same inputs, identical results, every run. No timing flake.
- **Scale** — run the matrix (every board, every config) in parallel CI jobs;
  hardware can't fork.
- **Zero hardware in the loop** — contributors and CI runners need no board.
- **Observability** — UART logs, VCD traces, and machine-readable `result.json`
  fall out of every run.

The MCU is a real Cortex-M core model with hardware-validated peripherals — not
a stub. Your firmware runs the same instructions it would on silicon.
