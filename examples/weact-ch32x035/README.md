# WCH CH32X035 (QingKe V4C) — WeAct core board and CH32X035C8T6

The CH32X035 is WCH's 48 MHz RISC-V **RV32IMAC** (QingKe V4C) with USB
device/host and a USB Type-C PD PHY, at about $0.27 in volume. WeAct Studio's
CH32X035 core board carries the QFN20 CH32X035F8U6. Live availability:
[evidence/vendor-status.md](evidence/vendor-status.md).

Demand: this is the WCH (Tier C) vendor bet in `DEMAND-ORDERED-TARGETS.md`,
where the CH32X035 is listed downstream of the CH32V003. It is not a survey
top-5 MCU.

## What runs

All firmware is unmodified ch32fun (`build-ch32fun-examples.sh`).

| Smoke | Profile | Firmware | Checks |
|---|---|---|---|
| `ch32fun-blink-smoke.yaml` | WeAct board, CH32X035F8U6 | `examples_x035/blink` | 4/4: PB12 (board LED), PA2, PA4, PA7 toggle 12,000,010 cycles apart (250 ms at 48 MHz) |
| `ch32fun-debugprintf-smoke.yaml` | WeAct board, CH32X035F8U6 | `examples_x035/debugprintfdemo` | 5/5 on the WCH-Link debug printf channel |
| `ch32fun-exti-smoke.yaml` | bare CH32X035C8T6 | `examples_x035/exti_pin_change_isr` | 9/9: a button on PC7 drives PC14 through EXTI line 7 and PFIC 20, one edge per button edge |

```sh
cargo run -q -p labwired-cli -- test \
  --script examples/weact-ch32x035/ch32fun-exti-smoke.yaml \
  --output-dir /tmp/x035-exti
```

The EXTI smoke uses the LQFP48 part because the WeAct board's QFN20 does not
bond PC7. Negative control
(`evidence/ch32fun-exti-smoke/negative-control/`): the same run with the F1
AFIO and V20x EXTI layouts fails 7/9, and PC14 never moves.

## The engine change: X035 AFIO and EXTI

The X035 keeps the F1 offsets but changes two things that matter for pin
interrupts:

- `afio` `profile: ch32x03x`: EXTICR1 (0x08) holds sixteen 2-bit port
  fields (00 = PA, 01 = PB, 10 = PC) for lines 0..15, and EXTICR2 (0x0C)
  holds lines 16..23. CTLR (0x18) is stored only.
- `exti` `profile: ch32x03x`: 30 lines. GPIO lines 7..0 share PFIC 20
  (`EXTI7_0_IRQn`), 15..8 share 40 and 23..16 share 41, as level requests.

The rest reuses existing models: the QingKe V4 core, PFIC and 64-bit SysTick
from the CH32V203, and the F1-layout GPIO, flash interface and RCC. The X035
has no PLL and no HSE. ch32fun keeps the 48 MHz HSI, and the `ch32v20x` RCC
profile's PLL is never enabled.

## Not modeled

- GPIO pins 16..23 (CFGXR/BSXR), so EXTI lines 16..23 have no pad source.
- USART1..4, I2C, SPI, TIM1..3, ADC/Touchkey, DMA, OPA/CMP, PIOC, USBFS and
  USB PD. ch32fun's other X035 examples (`uart_rx_tx_dma`, `tim2_pwm`,
  `usbpd_sink`, ...) are not run.
- PFIC nesting and priority grouping, low-power modes and flash wait states.
- Physical-board parity: no CH32X035 board was measured.
