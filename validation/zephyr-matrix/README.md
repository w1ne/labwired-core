# Zephyr × LabWired board matrix

Multi-level **unmodified-Zephyr-style** apps on every LabWired board that has a
Zephyr 3.7 west target and a `configs/systems/*.yaml` model.

This is the Zephyr counterpart to [`../arduino-matrix`](../arduino-matrix):
depth beyond the existing `firmware_survival` hello-only cases.

## Levels

| ID | Marker | Intent |
|----|--------|--------|
| `L0_hello` | `LW_Z0_OK` | `main()` + console after kernel boot |
| `L1_sleep` | `LW_Z1_OK` | `k_msleep` loop — tick / timer fidelity |
| `L2_blink` | `LW_Z2_OK` | GPIO `led0` + sleep + serial |

Samples live under `samples/` (in-tree Zephyr apps, not PlatformIO).

## Relation to `firmware_survival`

| Suite | What it proves |
|-------|----------------|
| `firmware_survival` `*_zephyr*` | Stock upstream `hello_world` (and KW41Z FXOS8700) still boots |
| **This matrix** | Same boards (plus nRF52) on L0–L2 with LabWired markers |

Survival stays the PR-gate “stock sample” bar. This matrix is the **depth** bar.

## Requirements

- Zephyr **v3.7.x** west workspace (`ZEPHYRPROJECT`, default `~/zephyrproject`)
- `arm-none-eabi-gcc` on `PATH` (`ZEPHYR_TOOLCHAIN_VARIANT=gnuarmemb`)
- Built LabWired CLI: `cargo build -p labwired-cli --release`

## Run

```bash
cd core
cargo build -p labwired-cli --release

# Full matrix (build with west + sim each cell)
python3 validation/zephyr-matrix/run_matrix.py

# Pilot
python3 validation/zephyr-matrix/run_matrix.py --boards stm32l476,stm32f103
python3 validation/zephyr-matrix/run_matrix.py --levels L0_hello,L1_sleep

# Skip west rebuild (reuse out/<board>/<level>/zephyr.elf)
python3 validation/zephyr-matrix/run_matrix.py --no-build --boards stm32l476
```

Outputs:

- `out/scoreboard.md` / `out/results.json`
- `out/<board>/<level>/{build.log,zephyr.elf,uart.log,result.json,...}`
- optional publish: `docs/coverage/zephyr-scoreboard.md`

## Interpreting status

| Status | Meaning |
|--------|---------|
| `pass` | UART contained the level marker |
| `build_fail` | `west build` failed |
| `toolchain_missing` | west / Zephyr / arm-none-eabi missing |
| `boot_fail` | Sim ran, empty UART |
| `oracle_fail` | UART present, marker missing |
| `sim_error` | Fault / decode / bus violation |
| `timeout` | Build or sim budget exceeded |

## Explicit non-goals

- ESP32 Zephyr (no in-tree ESP Zephyr survival path yet)
- Network core / BLE stacks as L3 (later)
- Patching Zephyr sources — samples are plain apps against stock Zephyr
