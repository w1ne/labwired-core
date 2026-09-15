# Python firmware SDK

Run firmware from Python and assert on its UART output with `labwired.Sim`.
The simulator runs inside your Python process; no board or server is required.

## Quickstart

These commands build the SDK from source. This SDK is not yet published to PyPI;
`pip install labwired` is not the installation path for this version.

You need Git, Python 3.9+ with `venv`, and Rust with Cargo and a native linker.
Use a POSIX shell on Linux, macOS, or WSL2. The first build can take several minutes.
The example firmware is already committed, so no firmware cross-toolchain is needed.

### 1. Install from source

```sh
git clone https://github.com/w1ne/labwired-core.git
cd labwired-core
python3 -m venv .venv
. .venv/bin/activate
python -m pip install 'maturin>=1,<2' 'pytest>=7'
maturin build --release --manifest-path crates/python/Cargo.toml --out dist/python-sdk
python -m pip install dist/python-sdk/labwired-*.whl
```

Keep this shell open in the repository root for the examples below.

### 2. Run your first firmware

Copy this entire command. It boots the committed ARM fixture and waits for `OK`:

```sh
python - <<'PY'
from labwired import Sim

with Sim("tests/fixtures/uart-ok-thumbv7m.elf",
         chip="ci-fixture-cortex-m3-uart1", uart="uart1") as sim:
    sim.expect("OK", timeout="1ms")
    print(sim.uart_transcript(), end="")
PY
```

Expected output:

```text
OK
```

The timeout is simulated time. Execution advances only when you call methods such
as `expect()` or `run_for()`.

### 3. Turn it into a pytest test

Create a test file and run it explicitly:

```sh
cat > test_firmware_boot.py <<'PY'
def test_boot(sim):
    machine = sim("tests/fixtures/uart-ok-thumbv7m.elf", uart="uart1")
    machine.expect("OK", timeout="1ms")
PY
python -m pytest -q test_firmware_boot.py \
  --labwired-chip ci-fixture-cortex-m3-uart1 --junitxml=result.xml
```

Expect `1 passed` and a `result.xml` report. The installed package registers the
`sim` fixture automatically. It closes each simulator after the test; failed tests
include UART transcripts in the terminal and JUnit report.

### Use your own firmware

Replace the ELF path and choose the matching chip from `configs/chips/`, using its
filename without `.yaml`, or supply a board manifest:

```python
from labwired import Sim

with Sim("build/firmware.elf", system="board.yaml") as sim:
    sim.expect("ready", timeout="100ms")
```

The ELF must target the selected model. See [run firmware](getting_started_firmware.md)
and the [configuration reference](configuration_reference.md) for board setup.
Chip names describe available models; they do not guarantee support for every
physical chip feature or arbitrary firmware.

## API reference

`Sim` drives the Rust `Session` engine directly using the native simulator's
machine builder. The older `labwired.Machine` and `labwired.StopReason` imports remain available.

The wheel includes the config catalog and its device/peripheral descriptors.
Named-chip lookup works outside the checkout and does not change
`LABWIRED_CONFIG_DIR`. Building copies the repository's config tree into the wheel.
A custom manifest's `chip` reference may be a packaged catalog name or a file path
relative to that manifest.

### Construction, execution, and UART

Provide exactly one of `chip=` or `system=`. A custom board can be opened with `Sim("firmware.elf", system="board.yaml")`. `uart=` names the actual peripheral ID, not a pin or host serial device. It selects both console capture and receive routing. If omitted, the manifest's `debug_uart` is used; if neither is declared the existing board-wide console capture/RX behavior applies. Invalid UART names fail; unsupported receive routing raises `NotSupported`.

`run_for(0.001)` and `run_for("1ms")` both request a millisecond of **virtual** time. Units are `s`, `ms`, `us`, and `ns`. Positive durations round up to a nanosecond, then to engine cycles; instructions can overshoot a cycle budget. Zero `run_for` is a no-op. Negative, non-finite, malformed, or excessive durations are errors. `expect` requires a positive timeout. Host execution time depends on firmware and machine speed.

`run_for` returns a `StopReason` with `kind` (`reached`, `halted`, `breakpoint`, or `error`) and optional breakpoint `pc`. Check that result if reaching the full duration matters. `sim.time` is elapsed virtual seconds and `sim.cycles` is the actual cycle count. No execution happens between calls.

`expect(regex, timeout=...)` uses Rust byte-regex syntax, consumes through the matched bytes, and preserves later output for the next call. It checks buffered bytes before advancing. `Match.text` contains the full match and `Match.captures` contains capture groups. `Match.at` is the virtual observation time; batches can run past the exact transmit cycle. `read_uart()` drains the same unread stream as UTF-8 text with replacement for invalid bytes; `uart_transcript()` returns all output without consuming it. `ExpectTimeout` is an `AssertionError` subclass with the pattern, timeout, and last output in its message. A halt without a match also produces this error. A breakpoint without a match raises an execution error.

### Inputs and observations

```python
sim.send("status\n")                  # UTF-8, no automatic newline
sim.send_bytes(b"\x01\x02")
channels = sim.list_inputs()          # device, key, label, unit, min, max
sim.set_input("pressed", 1.0)         # example: board with one input button
sim.set_inputs({"x": 0.0, "y": 0.0}) # atomic: all channels apply or none do
sim.set_pin("user_button", True)     # board_io input binding ID, logical active
word = sim.read_u32(0x20000000)
sim.write_u32(0x20000000, word + 1)
data = sim.read_memory(0x20000000, 4) # bytes
address = sim.symbol("Reset")         # integer address or None
word = sim.read_u32("Reset")         # symbol-aware, clears Thumb function bit
frames = sim.frames()                 # list of dictionaries; drains trace cursor
```

Channel names and binding IDs depend on the board. `list_inputs()` reports the channels the runtime actually exposes; it does not assume every modeled sensor has an input control. Multiple devices exposing the same key make that key ambiguous. Unknown names and invalid values raise errors. Reads and writes use the actual bus, including MMIO side effects. `frames()` returns `seq`, `cycle`, `at` (seconds), `bus`, `summary`, and the native serialized `payload`. The core trace ring is bounded; sequence gaps reveal evicted events.

`inject_can(bus, id, data, extended=False, fd=False, bitrate_switch=False, remote=False)` delivers through the modeled CAN controller's receive path. Clock, initialization, filters and queue capacity must permit reception; rejection is an error.

`snapshot()` returns an opaque token owned by the originating Sim. `restore(token)` reconstructs and deterministically replays that session to restore memory, device state, time and stream cursors. It may be expensive and refuses another Sim's token. Tokens are not portable save files.

Use a context manager or call `close()` to release the machine. Close is idempotent. Subsequent operations fail except `closed` and the retained `uart_transcript()`, which remain useful for diagnostics.

For a one-call run:

```python
from labwired import run_firmware
result = run_firmware("firmware.elf", chip="stm32f103", duration="10ms")
print(result.uart, result.stop_reason.kind)
```

The returned `Run` contains `uart`, `stop_reason`, actual elapsed `time` and `cycles`, and `matches`. Optional `expect=["ready", "done"]` waits for patterns in order with `timeout=` per pattern, then runs the requested `duration`. A breakpoint or early halt is preserved in `stop_reason`.

## pytest details

The optional `test` extra declares pytest as a dependency. Ordinary pytest discovery does not open firmware.

The `sim` fixture is a factory accepting the same arguments as `Sim`; it closes every created instance at teardown. Failures include full UART transcripts in terminal sections and JUnit properties, including machines already closed by a context manager. An explicit `NotSupported` exception becomes a skipped test with its reason. Other errors remain failures.

There is no YAML test collection yet. This interface does not add radio networking, RTOS task inspection, logic-analyzer control, ROM/flash-image boot, or multi-machine orchestration. The Python surface exposes only implemented operations; the shared builder reports unsupported targets explicitly. Simulation checks firmware behavior against modeled peripherals; it does not establish electrical behavior or physical-device parity.

## Source archives

For a source distribution, first build a wheel from the full repository to stage the catalog, then run `maturin sdist --manifest-path crates/python/Cargo.toml`. The sdist includes that generated catalog; its wheel rebuild does not need the original checkout. A source build with neither the repository catalog nor the bundled catalog fails explicitly. This wheel-first sequence is the supported packaging path for this SDK slice.
