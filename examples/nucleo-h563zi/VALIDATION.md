# NUCLEO-H563ZI Demo Runbook

Run all commands from `core/`.

## Prerequisites

1. Rust toolchain available (`cargo`, required targets installed).
2. `arm-none-eabi-gcc` installed (for hardware firmware build).
3. `openocd` installed (for board flashing).
4. NUCLEO-H563ZI connected over ST-Link USB (for hardware steps).

## A) Emulator Showcase (Deterministic)

```bash
examples/nucleo-h563zi/scripts/run_full_example.sh
```

Pass criteria:
1. script exits with code `0`
2. UART smoke contains `OK`
3. IO smoke contains LED on/off lines for `PB0/PF4/PG4`
4. full-chip smoke contains `RCC=1 SYSTICK=1 UART=1` and `ALL=1`

## B) Blink+UART in Emulator + Real Board

```bash
examples/nucleo-h563zi/scripts/run_blink_uart_dual.sh --port /dev/ttyACM0
```

Pass criteria:
1. emulator phase passes `io-smoke.yaml`
2. hardware phase flashes firmware successfully
3. hardware UART output contains:
   - `H563-BLINK-UART`
   - at least one `BLINK ... PB0=1 ...`
   - at least one `BLINK ... PB0=0 ...`

## C) Hardware-Only Run

```bash
examples/nucleo-h563zi/scripts/run_blink_uart_hardware.sh --port /dev/ttyACM0
```

Optional serial autodetect:

```bash
examples/nucleo-h563zi/scripts/run_blink_uart_hardware.sh
```

## D) Presentation Flow (3-5 Minutes)

1. Run `run_full_example.sh` and show deterministic emulator pass.
2. Run `run_blink_uart_hardware.sh --port /dev/ttyACM0` with board visible.
3. Point to UART lines and physical LED blinking as proof of parity.
4. Close with `docs/NUCLEO_H563ZI_DEMO.md` for capability summary.

## E) Recording Flow (Concise Terminal Output)

```bash
examples/nucleo-h563zi/scripts/run_video_demo.sh --mode all --port /dev/ttyACM0 --keep-artifacts
```

Pass criteria:
1. `PASS: uart-smoke`
2. `PASS: io-smoke`
3. `PASS: fullchip-smoke`
4. Hardware phase prints `Hardware blink+UART check passed.`

## Troubleshooting

1. `openocd` transport errors: use defaults from script (`stlink-dap` + `dapdirect_swd`).
2. No UART output: verify port (`/dev/ttyACM*`) and ST-Link cable.
3. Permission errors on serial: add user to `dialout` group or run with proper device permissions.
