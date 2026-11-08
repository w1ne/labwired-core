# L6 — analogWrite(LED_BUILTIN, 128) on the Arduino Zero

Stock ArduinoCore-samd (atmelsam@9.0.0) drives D13 = PA17 from TCC2 WO[1]
(PMUX function E): WAVE = NPWM, PER = 0xFFFF, CC1 = 0x8000 (128 mapped to 16
bits), GCLK0 = 48 MHz, prescaler DIV1.

The matrix row watches the pad (`pwm_watch: gpio_a:17`), not the PORT latch.
Measured in `result.json` → `logic_edges` (45 edges before LW_L6_OK):

| | cycles | at 48 MHz |
|---|---|---|
| period | 65,536 | 732.4 Hz |
| high time | 32,768 | 50.00 % duty |

First rise at cycle 108,450 (TCC2 enabled), then high/low alternate every
32,768 cycles exactly. `gpio_pins` reports PA17 as `af` with function
`TCC2_WO1`. Before this change the TCC2 window was unmapped and the cell was
skipped.
