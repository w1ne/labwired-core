# NUCLEO-H563ZI Capability Showcase

This example demonstrates STM32H563 capabilities in two environments:
1. LabWired emulator
2. Real NUCLEO-H563ZI board

The goal is simple: run the same demo story in both places and prove deterministic, repeatable behavior.

For a presentation-ready narrative, see `../../../docs/NUCLEO_H563ZI_DEMO.md`.

## What This Demo Proves

1. Boot/reset flow for STM32H563 image loading.
2. UART logging through board VCP path (`USART3`).
3. Board IO behavior for LEDs (`PB0`, `PF4`, `PG4`) and button (`PC13`).
4. Full-chip smoke path for currently modeled H563 peripherals (`RCC`, `SYSTICK`, `UART`, `GPIOA..GPIOG`).
5. Hardware-in-the-loop parity: blink + UART output on a physical NUCLEO-H563ZI board.

## Quick Start

Run from `core/`.

### 1) Emulator capability check (recommended first)

```bash
examples/nucleo-h563zi/scripts/run_full_example.sh
```

This builds demo firmware and runs deterministic smoke checks.
No generated artifacts are written into the repo by default.

### 2) Emulator + real-board blink demo

```bash
examples/nucleo-h563zi/scripts/run_blink_uart_dual.sh --port /dev/ttyACM0
```

### 3) Video-ready full flow (concise output)

```bash
examples/nucleo-h563zi/scripts/run_video_demo.sh --mode all --port /dev/ttyACM0 --keep-artifacts
```

If you only want hardware:

```bash
examples/nucleo-h563zi/scripts/run_blink_uart_hardware.sh --port /dev/ttyACM0
```

## Expected Output Signals

Emulator checks:
1. `OK`
2. `H563-IO`
3. `PB0=1 PF4=1 PG4=1 ...`
4. `PB0=0 PF4=0 PG4=0 ...`
5. `RCC=1 SYSTICK=1 UART=1`
6. `ALL=1`

Command Center note:
1. `system.yaml` `board_io` is the board wiring source of truth.
2. VS Code shows LED/button state from emulated GPIO register values (no LED-specific emulation path).

Real board checks:
1. `H563-BLINK-UART`
2. `BLINK ... PB0=1 ...`
3. `BLINK ... PB0=0 ...`

## Files You Need

- `VALIDATION.md`: step-by-step reproducible runbook
- `system.yaml`: local system profile used by emulator runs
- `board_firmware/`: native C firmware for real-board blink+UART
- `scripts/run_full_example.sh`: one-command emulator showcase
- `scripts/run_blink_uart_dual.sh`: one-command emulator + hardware blink demo
- `scripts/run_blink_uart_hardware.sh`: hardware-only flash + UART check
- `scripts/run_blink_uart_emulator.sh`: emulator-only blink check
- `scripts/run_video_demo.sh`: concise recording-oriented end-to-end run

Repository hygiene note:
- Recording artifacts/logs are written to `/tmp` unless `--artifacts-dir` is provided.

## References

- Chip config: `../../configs/chips/stm32h563.yaml`
- System config: `../../configs/systems/nucleo-h563zi-demo.yaml`
- Marketing/demo narrative: `../../../docs/NUCLEO_H563ZI_DEMO.md`
