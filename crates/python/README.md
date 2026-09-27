# labwired (Python)

Run microcontroller firmware in the LabWired simulator and test it with pytest.
Time is virtual: execution advances only inside `run_for` and `expect`.

## Install

```sh
pip install labwired
```

Wheels are abi3: one wheel per platform covers CPython 3.9 and newer
(Linux x86_64/aarch64, macOS arm64/x86_64, Windows x64).

## Test firmware with pytest

The package installs a pytest plugin with a `sim` fixture:

```python
def test_boot_banner(sim):
    with sim("firmware.elf", chip="stm32f103") as s:
        s.expect(r"boot ok", timeout="10ms")
        s.send("ping\n")
        assert s.expect("pong", timeout="5ms").text == "pong"
```

Run it with `pytest`. Give `--labwired-chip <name>` to set a default chip for
every `sim(...)` call. On a failure, the report shows the UART transcript.

## API summary

- `Sim(elf, chip=... | system=..., uart=None)`: one ELF on one machine.
- `run_for(duration)`, `expect(pattern, timeout)`: advance virtual time.
- `read_uart()` returns text (invalid UTF-8 becomes U+FFFD).
  `read_uart_bytes()` returns the raw bytes.
- `send(text)`, `send_bytes(data)`, `read_u32`, `write_u32`, `read_memory`,
  `snapshot()`, `restore(snap)`, `set_input`, `set_pin`, `frames()`.
- `run_firmware(elf, chip=..., duration="1s", expect=...)`: one-shot helper.
