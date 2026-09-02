# External Components (BRD2709A / xG26-EK2709A)

No required external simulated components for the minimal deterministic smoke test.

## Parts that DO attach on this board

Gated by `crates/core/tests/efr32_onboarded_parts.rs`, which builds each one
through `SystemBus::from_config` against the real `efr32mg26.yaml` — the same
path a lab takes, so a chip descriptor that lost a peripheral would fail here
rather than at someone's desk.

| Part | Connection | Notes |
|------|-----------|-------|
| `st7789-170x320` | `spi0` | USART0 in SPI mode. See `st7789-system.yaml`. |
| `inmp441` | `spi0` | The SAME block in I2S mode — EFR32 has no separate I2S peripheral. |
| `slide-potentiometer` | `iadc0` | Reaches the shared potentiometer kit through TYPE_ALIASES. |
| `toggle-switch`, `button-module`, `rotary-encoder` | `board_io` | GPIO. |

⚠️ **The breakout pads are not spare pins.** UG594 Table 3.1 (p.10): "pins may
be shared between the breakout pads and other functions". Of the 28 pads,
**PD03 (pad 6) is the only GPIO with no shared feature** — everything else is
also a mikroBUS or Qwiic signal, or a debug/PTI line. Wiring anything to the
pads means keeping the mikroBUS socket empty.

Pins that are NOT available: PA01/PA02/PA03 (SWD — the on-board J-Link uses
them throughout a debug session), PB02/PB03 (VCOM console), PB00/PB01
(BTN0/BTN1), PC08/PC09 (LED0/LED1), PD04/PD05 (PTI).

⚠️ **3.3 V only.** The xG26's GPIO is not 5 V tolerant — its datasheet gives
`VDIGPIN` abs max as `VIOVDD + 0.3 V`. Pad 7 is the board's 5 V USB rail and
must not reach any MCU pin. Power 3.3 V modules from VMCU (pad 8).

The onboarding path uses on-chip peripherals only:

1. USART1 (VCOM console, TX @ 0x400A4038)
2. SysTick (declared; not exercised by the smoke firmware)
3. NVIC (declared; not exercised by the smoke firmware)

On-board hardware not modelled (documented for completeness, not needed at L1):

- mikroBUS socket and Qwiic connector (expansion headers; nothing wired by default)
- On-board J-Link OB debugger (host-side; the sim replaces it)
- Board controller (VCOM routing is implicit in the UART model's TX sink)
