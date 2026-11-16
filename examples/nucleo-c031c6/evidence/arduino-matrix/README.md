# NUCLEO-C031C6 Arduino matrix evidence

Demand placement: **other STM32 (rank 4, 13 % measured MCU use) on a Nucleo
board (23 % of kits)**. The STM32C0 is ST's current entry series; this is the
first C0 part in the engine. It is not a top-3 demand pick (ESP32 /
ATmega328 / STM32F103 rows already exist).

Generated 2026-11-09 with stock STM32duino (`ststm32@19.7.1`, Arduino_Core_STM32
2.12, board `nucleo_c031c6`). Result: **7 pass / 2 skip / 0 fail**.

| Sketch | Result | What the model did |
|---|---:|---|
| L0 serial boot | pass | HSE 48 MHz clock switch (RCC `stm32c0`), USART2 → ST-LINK VCP on PA2; 52,470 steps |
| L1 serial loop | pass | SysTick `delay`/`millis`; 100,469 steps |
| L2 blink | pass | LD4/PA5 output: 2 pad edges; 148,567 steps |
| L3 I2C sensor | pass, exact tier | Wire on I2C1 (PB8/PB9) reads INA219 CONFIG 0x399F and BUS 0x19CA (no `LW_L3_PARTIAL_NO_RX` in a full run); 11,316,915 steps |
| L4 SPI sensor | pass | SPI1 (PA5/PA6/PA7) reads the MAX31855 frame 0x01901600; 101,691 steps |
| L5 ADC | **skipped** | `analogRead(A0)` returns `0 0`: no C0 ADC (CFGR1/CHSELR) model. The generic oracle would have passed it, so the row skips it instead |
| L6 PWM | pass | `analogWrite(LED_BUILTIN, 128)`: PA5 in AF mode as TIM1_CH1 (AF5), 60 pad edges, period 48,000 cycles = **1.000 kHz, 50.2 % duty** (128/255) |
| L7 timer | pass | `micros()` advances monotonically; 196,547 steps |
| L8 CAN | skipped | the C031 has no CAN |

`run_matrix.log` and `scoreboard.md` are the matrix-level report; each sketch
directory carries its `test.yaml`, compile log, UART, JUnit, snapshot and
`result.json` (with the L2/L6 `logic_edges` capture).

Reproduce from the repository root (LabWired CLI built from this tree):

```bash
python3 validation/arduino-matrix/run_matrix.py \
  --boards stm32c031c6 \
  --out examples/nucleo-c031c6/evidence/arduino-matrix
```

## Model boundary

- RCC `stm32c0`: the G0 register file at the G0 offsets with the C0 reset
  value (CR 0x0000_1540, HSIDIV /4) and **no PLL** (PLLON never readies,
  PLLCFGR reserved, SW=010 never lands).
- Timer outputs reach pads through a C0 AF table (`pad_map: stm32c0`, from
  stm32duino's C031 `PinMap_TIM`); complementary CHxN outputs are not routed.
- ADC1 is a register file only (see L5). DMA/DMAMUX, RTC, IWDG/WWDG and CRC
  are reused family models not exercised by this row.
- ststm32 19.7.1 lacks the `nucleo_c031c6` board JSON;
  `validation/arduino-matrix/pio-boards/nucleo_c031c6.json` is 20.0.0's file
  unchanged. (20.0.0 itself makes `SPI.begin()` a no-op, so it is not used.)
- Deterministic simulation, not a capture from physical silicon.
