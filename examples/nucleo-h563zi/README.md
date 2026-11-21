# NUCLEO-H563ZI Capability Showcase

This example demonstrates STM32H563 capabilities in two environments:
1. LabWired emulator
2. Real NUCLEO-H563ZI board

The goal is simple: run the same demo story in both places and prove deterministic, repeatable behavior.

For the hardware-vs-simulation parity pipeline that backs this demo, see [`../../docs/golden_reference.md`](../../docs/golden_reference.md).

## What This Demo Proves

1. Boot/reset flow for STM32H563 image loading.
2. UART logging through board VCP path (`USART3`).
3. Board IO behavior for LEDs (`PB0`, `PF4`, `PG4`) and button (`PC13`).
4. Full-chip smoke path for currently modeled H563 peripherals (`RCC`, `SYSTICK`, `UART`, `GPIOA..GPIOG`).
5. Hardware-in-the-loop parity: blink + UART output on a physical NUCLEO-H563ZI board.
6. **Golden Reference**: Standardized determinism proof between hardware and simulation.
## Quick Start

Run from the repo root.

### VS Code (out-of-box demo run)

1. Open `examples/nucleo-h563zi` as the workspace folder.
2. Run `LabWired: Run in LabWired` (or click `Run in LabWired` in the status bar).

The example root `Makefile` builds `firmware-h563-io-demo` and writes `target/firmware`,
which is the firmware path the extension launches.
`Run in LabWired` now starts execution immediately and UART text appears in the `LabWired` output panel.

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

This prints emulator UART evidence first, then live UART from the real board.

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
- `Makefile`: VS Code `Run in LabWired` bridge for this example folder
- `board_firmware/`: native C firmware for real-board blink+UART
- `scripts/run_full_example.sh`: one-command emulator showcase
- `scripts/run_blink_uart_dual.sh`: one-command emulator + hardware blink demo
- `scripts/run_blink_uart_hardware.sh`: hardware-only flash + UART check
- `scripts/run_blink_uart_emulator.sh`: emulator-only blink check
- `scripts/run_video_demo.sh`: concise recording-oriented end-to-end run
- `golden-reference/`: determinism proof artifacts
  - `determinism_report_h563.json`: PASS/FAIL evidence
  - `hw_trace.json`: hardware instruction trace
  - `sim_trace.json`: simulation instruction trace
Repository hygiene note:
- Recording artifacts/logs are written to `/tmp` unless `--artifacts-dir` is provided.

## Mixed-signal regression fixtures

`analog-ntc-diode-adc-system.yaml` and `analog-mosfet-adc-system.yaml` route generic MNA
loads to explicit `adc.adc1.<channel>_volts` co-simulation endpoints. The core test asserts
the NTC, diode, and level-1 NMOS load lines through ADC1's modeled data-register path. These
fixtures do not define physical H563 pad aliases or claim silicon conversion parity.

`analog-bjt-recombination-adc-system.yaml` adds matched generic Gummel–Poon branches with
base-emitter recombination, base-collector recombination, both effects, and neither effect.
The core test compares the solved collector load lines against an independent reference and
reads each branch through ADC1 channels 2–5 alongside isolated NTC and diode inputs on
channels 0–1. These are explicit co-simulation routes; they do not claim physical pad aliases
or silicon ADC parity.

`analog-bjt-charge-adc-system.yaml` routes matched generic BJT branches with CJE, CJC, TF,
and combined junction/transit charge to ADC1 channels 2–6 beside the NTC and diode on
channels 0–1. The transient regression checks the first-step charge response, ADC data-register
readback, isolation, and convergence to the same DC load line. Enlarged junction capacitances
make this a solver-physics stress case at the fixture's 1 us step; it is not a device-rating
claim.

## References

- Chip config: `../../configs/chips/stm32h563.yaml`
- System config: `../../configs/systems/nucleo-h563zi-demo.yaml`
- Hardware-vs-simulation parity pipeline: [`../../docs/golden_reference.md`](../../docs/golden_reference.md)
