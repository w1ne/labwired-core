# Arduino × LabWired board matrix

Runs stock-style Arduino sketches on **every LabWired-supported chip** via
PlatformIO compile + `labwired test`, then writes a scoreboard of what boots,
what fails compile, and what hits unmodeled paths.

## Sketches

| ID | Marker | Intent |
|----|--------|--------|
| `L0_serial_boot` | `LW_L0_OK` | `setup()` + Serial after core boot |
| `L1_serial_loop` | `LW_L1_OK` | `loop()` + `delay`/`millis` scheduling |
| `L2_blink_serial` | `LW_L2_OK` | `digitalWrite(LED_BUILTIN)` + serial |

## Boards

See `boards.yaml` — currently all chips with a `configs/chips/*.yaml` model that
has an Arduino PlatformIO profile (ESP32 family, nRF52, RP2040, STM32 set).

## Run

```bash
cd core
cargo build -p labwired-cli --release   # once
# optional: pio pkg install -g -p raspberrypi   # for rp2040

python3 validation/arduino-matrix/run_matrix.py
python3 validation/arduino-matrix/run_matrix.py --boards stm32f103,esp32
python3 validation/arduino-matrix/run_matrix.py --sketches L0_serial_boot
```

Outputs:

- `out/scoreboard.md` — human matrix
- `out/results.json` — machine-readable
- `docs/coverage/arduino-scoreboard.md` — published copy
- `out/<board>/<sketch>/` — compile logs, ELF, uart.log, result.json

## Interpreting status

| Status | Meaning |
|--------|---------|
| `pass` | UART contained the sketch marker |
| `compile_fail` | PlatformIO/Arduino core rejected the sketch |
| `toolchain_missing` | PlatformIO platform/board package not installed |
| `boot_fail` | Sim ran but produced no UART (hang, fault, wrong entry) |
| `oracle_fail` | UART present but marker missing (wrong UART, partial boot) |
| `unmodeled` | Sim/loader reported unimplemented peripheral/instruction |
| `timeout` | Compile or run exceeded budget |

This is complementary to **Tier-1 fixtures** (bare-metal peripheral rubric):
Tier-1 proves models under hand-written drivers; this matrix proves **Arduino
cores** (the path Architect/proto.cat generate).
