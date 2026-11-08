# WCH CH32V006F8U7 (QingKe V2C) — ch32fun CH32V00x examples

The CH32V006 is the larger 2024 member of WCH's CH32V00x family, the
CH32V003's successor line: the same 48 MHz QingKe V2 RISC-V core (RV32EC,
plus Zmmul on the V2C), 62 KB flash instead of 16 KB, 8 KB SRAM instead of
2 KB, a second USART, TIM3, a 12-bit ADC and an OPA. The QFN20 CH32V006F8U7
is about $0.15 in volume. Live availability:
[evidence/vendor-status.md](evidence/vendor-status.md).

Demand: this is the WCH (Tier C) vendor bet in `DEMAND-ORDERED-TARGETS.md`,
one more member of a family the engine already models (CH32V003). It is not a
survey top-5 MCU. It is here because the V00x line is where V003 users move
for more memory, and ch32fun supports it with its own `examples_x00x/`.

## What runs

All firmware is unmodified ch32fun (`build-ch32fun-examples.sh`,
byte-reproducible), built for `TARGET_MCU=CH32V006`.

| Smoke | Firmware | Checks |
|---|---|---|
| `ch32fun-blink-smoke.yaml` | `examples_x00x/blink` | 4/4: PD0, PD4, PD6, PC0 toggle every 250 ms |
| `ch32fun-debugprintf-smoke.yaml` | `examples/debugprintfdemo` | 5/5 on the WCH-Link debug printf channel |
| `ch32fun-dual-usart-smoke.yaml` | `examples_x00x/dual_usart` | 5/5: banner, "USARTn ready" and "tick N via USARTn" on both ports; TX edges on PD5 and PD2 |

```sh
cargo run -q -p labwired-cli -- test \
  --script examples/ch32v006f8u7/ch32fun-dual-usart-smoke.yaml \
  --output-dir /tmp/v006-dual-usart
python3 examples/ch32v006f8u7/check-dual-usart.py /tmp/v006-dual-usart/result.json
```

`check-dual-usart.py` decodes the two TX pads as 115200 8N1: PD5 carries only
USART1's text and PD2 only USART2's
([log](evidence/ch32fun-dual-usart-smoke/check-dual-usart.log)). Negative
control ([evidence/ch32fun-dual-usart-smoke/negative-control/](evidence/ch32fun-dual-usart-smoke/negative-control/README.md)):
with the CH32V003 AFIO layout the USART2 remap store is lost, PD2 stays
silent, and the smoke fails 4/5.

## The engine change: CH32V00x AFIO, RCC and USART pads

- `afio` `profile: ch32v00x`: EXTICR at 0x08 and **PCFR1 at 0x0C**. The
  CH32V003 has PCFR1 at 0x04, so the V003 layout would drop every V00x
  remap. Sources: ch32fun `ch32x00xhw.h` `AFIO_TypeDef`, ch32-data
  `afio_v00x.yaml`.
- `rcc` `profile: ch32v00x`: the CH32V003 clock tree with USART2EN (APB2PCENR
  bit 13) writable. USART2 sits at 0x4000_4400 on APB1 but is clocked from
  APB2.
- `uart` `pad_map: ch32v00x`: USART1_TX on PD5 while PCFR1.USART1_RM [9:6] =
  0, USART2_TX on PD2 while USART2_RM [22:20] = 3, each a remap-gated pad
  route (`PadRoutes::bind_gated`).

The core is the existing QingKe V2 profile; a unit test
(`qingke_v2_executes_zmmul_multiplies`) covers Zmmul `mul`/`mulhu`. None of
the three ELFs contains a multiply, so the smokes themselves do not exercise
Zmmul.

## Not modeled

- TIM1/TIM2/TIM3, ADC1 (12-bit on the V00x; the V003 ADC model is 10-bit),
  OPA/CMP, TouchKey, I2C1, SPI1, DMA1, EXTI, PWR/AWU, IWDG/WWDG. ch32fun's
  `examples_x00x/adc_polled`, `pvd_voltage_detector` and `standby_autowake`
  are not run.
- USART RX pads, CTS/RTS, the USART interrupts (PFIC 32/39) and the other
  USART1/USART2 remap rows.
- GPIOB (not bonded on QFN20), flash wait states (ACTLR is stored only),
  div/rem trapping on the V2C (the engine executes them), PFIC nesting.
- Physical-board parity: no CH32V006 was measured.
