# NUCLEO-C031C6 Arduino matrix evidence

Demand placement: **other STM32 (rank 4, 13 % measured MCU use) on a Nucleo
board (23 % of kits)**. The STM32C0 is ST's current entry series; this is the
first C0 part in the engine. It is not a top-3 demand pick (ESP32 /
ATmega328 / STM32F103 rows already exist).

Generated 2026-11-24 with stock STM32duino (`ststm32@19.7.1`, Arduino_Core_STM32
2.12, board `nucleo_c031c6`). Result: **8 pass / 1 skip / 0 fail**.

| Sketch | Result | What the model did |
|---|---:|---|
| L0 serial boot | pass | HSE 48 MHz clock switch (RCC `stm32c0`), USART2 → ST-LINK VCP on PA2 |
| L1 serial loop | pass | SysTick `delay`/`millis` |
| L2 blink | pass | LD4/PA5 output: 2 pad edges |
| L3 I2C sensor | pass, exact tier | Wire on I2C1 (PB8/PB9) reads INA219 CONFIG 0x399F and BUS 0x19CA (no `LW_L3_PARTIAL_NO_RX` in a full run) |
| L4 SPI sensor | pass | SPI1 (PA5/PA6/PA7) reads the MAX31855 frame 0x01901600 |
| L5 ADC | pass, exact tier | ADC1 on the `stm32g0` (G0/C0 CHSELR) layout: `analogRead(A0)` on PA0 = ADC1_IN0 at 1651 mV prints `LW_L5_VAL 512 512` |
| L6 PWM | pass | `analogWrite(LED_BUILTIN, 128)`: PA5 in AF mode as TIM1_CH1 (AF5), 60 pad edges, period 48,000 cycles = **1.000 kHz, 50.2 % duty** (128/255) |
| L7 timer | pass | `micros()` advances monotonically |
| L8 CAN | skipped | the C031 has no CAN |

The earlier 7/2 run skipped L5 because the chip descriptor used the L4
(L4-shaped) ADC layout, which has no CHSELR register; `analogRead` returned
`0 0` on every pin. ADC1 now carries `profile: stm32g0` (the C0 and G0 ADC are
the same IP) and the system manifest holds A0 at 1651 mV. A pass with `0 0`
would have been vacuous, so the row skipped until the conversion was exact.

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
- ADC1 is the G0/C0 CHSELR block (`profile: stm32g0`), exercised by L5.
  DMA/DMAMUX, RTC, IWDG/WWDG and CRC are reused family models not exercised
  by this row.
- ststm32 19.7.1 lacks the `nucleo_c031c6` board JSON;
  `validation/arduino-matrix/pio-boards/nucleo_c031c6.json` is 20.0.0's file
  unchanged. (20.0.0 itself makes `SPI.begin()` a no-op, so it is not used.)
- Deterministic simulation, not a capture from physical silicon.
