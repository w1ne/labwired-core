# WCH CH32X035G8U6 — WCH's official Arduino core

The CH32X035G8U6 is the QFN28 package of WCH's CH32X035: a QingKe V4C
(RV32IMAC) core at 48 MHz with 62 KB of flash and 20 KB of SRAM, plus USB
and USB PD. It is the only X035 variant that
[openwch/arduino_core_ch32](https://github.com/openwch/arduino_core_ch32)
ships (`variants/CH32X035/CH32X035G8U`). The die and register map are the same
as the LQFP48 and QFN20 profiles that run ch32fun
([`weact-ch32x035`](../weact-ch32x035/)); only the bonded pads differ.

Profile: [`configs/chips/ch32x035g8u6.yaml`](../../configs/chips/ch32x035g8u6.yaml).
Vendor status: [`evidence/vendor-status.md`](evidence/vendor-status.md)
(WCH product page lists QFN28; LCSC C7437027, 8,787 in stock on 2026-11-08).

## Stock Arduino sketches

The Arduino matrix's stock sketches are built with the core through the
community PlatformIO `ch32v` platform (`genericCH32X035G8U6`,
`-march=rv32imacxw`, the same pinned revisions as the `ch32v203` and
`ch32v307` rows) and run unmodified. The row is `ch32x035` in
[`boards.yaml`](../../validation/arduino-matrix/boards.yaml). This is a chip
row with no board LED, so it adds `-DLED_BUILTIN=PB12 -DLW_PWM_PIN=PA6`:
the variant leaves LED_BUILTIN undefined, PB12 is a plain GPIO pad, and PA6
is TIM3_CH1 at the reset mapping (variant `PinMap_TIM`, `AFIO_NONE`).

| Sketch | What it exercises | Result |
| --- | --- | --- |
| L0 / L1 | core boot, `Serial` on USART1 (TX PB10), `millis()` from the STK interrupt | ✅ ✅ |
| L2 | `digitalWrite(LED_BUILTIN)` on PB12, ≥2 edges | ✅ |
| L3 | `Wire` on I2C1 to an INA219 at 0x40: config 0x399F, bus 0x19CA | ✅ |
| L4 | `SPI` on SPI1 (PA5/6/7, CS PA4) to a MAX31855: frame 0x01901600 | ✅ |
| L5 | `analogRead(A0)` on ADC1: two completed conversions (2, 3, as on the other WCH rows) | ✅ |
| L6 | `analogWrite(PA6, 128)` on TIM3_CH1, ≥4 edges: 1500 high cycles per 48000-cycle (1 kHz) period = 3.125 %, the core's 12-bit default 128/4096 | ✅ |
| L7 | `micros()` | ⏭️ upstream bug, below |
| L8 | CAN | ⏭️ the CH32X035 has no CAN |

Evidence: [`evidence/arduino-matrix/`](evidence/arduino-matrix/). Reproduce:

```bash
cargo build -p labwired-cli
python3 validation/arduino-matrix/run_matrix.py --boards ch32x035 --labwired target/debug/labwired
```

What the engine needed:

- **USART1 TX on PB10.** The F1 USART model emits a byte only while its TX
  pad is an alternate-function output, and the F1 table puts USART1_TX on
  PA9. On the X035 it is PB10 while AFIO_PCFR1 USART1_RM [6:5] is 0
  (ch32fun `ch32x03xhw.h`, WCH SPL `GPIO_PartialRemap1_USART1`). The new
  `pad_map: ch32x03x` selects that route, gated on the remap field. Without
  it, L0 runs to the end of `setup()` and prints nothing.
- **USART1, I2C1, SPI1, ADC1 and TIM1..TIM3.** These are the F1 register
  layouts at the `ch32x035.h` bases, with `ch32x035.h` IRQ numbers and RCC
  enable bits.

Negative controls
([`evidence/arduino-matrix/negative-controls/`](evidence/arduino-matrix/negative-controls/)):
L0 without `pad_map: ch32x03x` prints nothing. L3 without the INA219 gives
`LW_L3_FAIL nack`. L6 watched on PA7 fails the pad oracle.

**L7 is skipped for an upstream core bug, the same one as on the V003, V203
and V307 rows.** `getCurrentMicros()` assumes a down-counter, but
`hw_config.c` starts the STK with `CTLR = 0xF`, which counts up. A
`Serial.print` probe on this profile measured t0=1999 t1=3999 t2=3788
t3=4999 with CTLR=F. The core's libprintf drops `%lu`, so the sketch's own
FAIL line prints empty values.

## Not modeled

- GPIO `CFGXR`/`BSXR` (pins 16..23). `pinMode` on PC16..PC19 is not
  reflected on those pads. L3's Wire runs on PC16/PC17, so L3 proves the I2C1
  register traffic and the device transaction, not the pad configuration.
- Timer output pads come from the STM32F1 default table. Only TIM3_CH1 on
  PA6 (and TIM3_CH2 on PA7) agrees with the X035 variant; the F1 rows for
  TIM1 and TIM2 are not the X035's pads and have not been corrected, and no
  TIM remap is decoded.
- USART2..4, DMA, OPA/CMP, PIOC, USBFS and USBPD; the ADC's CTLR3 clock
  divider and window registers; PFIC nesting; low-power modes.
- Physical-board parity. No CH32X035G8U6 has been measured on a bench.
