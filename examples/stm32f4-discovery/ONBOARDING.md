# STM32F407G-DISC1 onboarding

The current STM32F407G-DISC1 order code uses an STM32F407VGT6. ST's product
page says this order code replaces the original STM32F4DISCOVERY order code;
the LabWired profile keeps the existing `stm32f4-discovery` system id.

## Board paths in the model

| Board signal | MCU signal | Modeled behavior |
|---|---|---|
| LD4 (green) | PD12 | Active-high GPIO output; exercised |
| LD3 (orange) | PD13 | Active-high GPIO output; exercised |
| LD5 (red) | PD14 | Active-high GPIO output; exercised |
| LD6 (blue) | PD15 | Active-high GPIO output; exercised |
| USER button | PA0 | Active-high GPIO input; mapped, not exercised |
| Console header TX/RX | USART2 PA2/PA3 (P1 pins 14/13) | Simulator captures TX; exercised |

The system descriptor maps the board I/O and instantiates the STM32F407VGT6
chip model. The smoke enables RCC clocks for GPIOA, GPIOD and USART2, sends a
banner on USART2, then walks the four LEDs. Each heartbeat reads `GPIOD_ODR`
back after setting an LED so the script checks the modeled GPIO state as well
as UART output. The script also counts LD4/PD12 output edges through the
modeled board I/O binding. The USART2 mapping is cross-checked against ST's
[UM1472 user manual](https://www.st.com/resource/en/user_manual/DM00039084-.pdf),
which says PA2/PA3 are on the board header and the ST-LINK/V2-A VCP is not
connected to the target MCU by default. The model captures USART2 TX directly;
it does not simulate ST-LINK or physical UART wiring.

The committed smoke result also records the simulator's supply assumption:
the MCU runs from an ideal 3.3 V rail because no power net drives it. Power-on
and brown-out reset behavior are not simulated.

## Reproduce

```bash
scripts/onboarding_smoke.sh \
  --target-id stm32f4-discovery \
  --crate firmware-f4-discovery-smoke \
  --target thumbv7em-none-eabi \
  --script examples/stm32f4-discovery/io-smoke.yaml \
  --system configs/systems/stm32f4-discovery.yaml \
  --out-dir examples/stm32f4-discovery/evidence

python3 scripts/generate_onboarding_scoreboard.py \
  --metrics-root examples/stm32f4-discovery/evidence \
  --markdown-out examples/stm32f4-discovery/evidence/onboarding-scoreboard.md \
  --json-out examples/stm32f4-discovery/evidence/onboarding-scoreboard.json
```

The committed `evidence/` directory contains the build logs, smoke result,
UART transcript (normalized to LF), snapshot, and scoreboard. This is
deterministic simulator evidence, not a physical-board capture.

## Model boundary

The smoke covers the boot path, RCC, GPIOA/GPIOD, and USART2. Only the four
LEDs and USER button are board I/O in the system descriptor. The onboard
accelerometer, audio DAC, microphone, USB OTG and ST-LINK/debug interface are
not modeled. Ethernet, CAN, SDIO, I²C, timers and ADC are not smoke-tested. See
[`EXTERNAL_COMPONENTS.md`](EXTERNAL_COMPONENTS.md) for that boundary and
[`evidence/vendor-status.md`](evidence/vendor-status.md) for the current
vendor status and buyability check.

The F407 chip descriptor contains no explicit `stub` peripheral and none of
its declared peripheral types appear in `known_stubs.rs`; the unmodeled
board-level devices above are omitted rather than represented by zero-answer
windows. This is local core simulator evidence. The hosted LabWired catalog has
no runnable F407 target, so no hosted `model_verified` verdict or hardware
capture is claimed.
