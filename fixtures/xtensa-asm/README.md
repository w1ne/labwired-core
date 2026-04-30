# Hand-assembled Xtensa fixture programs

These fixtures are built from human-written Xtensa assembly and committed as
pre-built binaries so that CI can run oracle tests without requiring the full
ESP toolchain.

## Files

| File | Description |
|------|-------------|
| `fibonacci.s` | Xtensa assembly: compute fib(10) = 55, terminate with `break 1, 15` |
| `linker.ld` | Minimal linker script; places `.text.entry` at `0x40370000` (ESP32-S3 IRAM start) |
| `Makefile` | Drives `xtensa-esp32s3-elf-as` + `xtensa-esp32s3-elf-ld` |
| `fibonacci.elf` | **Committed binary** — PT_LOAD at VAddr `0x40370000`, entry `0x40370000` |
| `fibonacci.bin` | **Committed binary** — raw bytes of the `.text` section (33 bytes) |

## Rebuilding from source

Requires `xtensa-esp32s3-elf-*` tools from the ESP-IDF toolchain (installed
by `espup` at `~/.rustup/toolchains/esp/`):

```sh
cd fixtures/xtensa-asm
make
```

To verify the disassembly:

```sh
make disasm
```

## Design notes

- `ENTRY` was removed from `fibonacci.s`.  The fixture is not called from
  another windowed frame, so `PS.CALLINC = 0` at `_start` and `ENTRY` would
  not rotate the register window.  The computation is identical without it.
- The program terminates with `BREAK 1, 15` (bytes `0xF0 0x41 0x00`), which
  raises `BreakpointHit` in the simulator and halts the ESP32-S3 hardware
  debug unit via JTAG.
