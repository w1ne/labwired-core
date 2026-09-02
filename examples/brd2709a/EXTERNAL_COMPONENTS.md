# External Components (BRD2709A / xG26-EK2709A)

No required external simulated components for the minimal deterministic smoke test.

## The agent deck

`agent-deck-system.yaml` wires every part below onto the bare breakout pads at
once, and `deck-smoke.yaml` / `firmware-mg26-deck` drive them together. That
pair is this chip's **L2 (behaviour)** evidence: the silicon reset oracle
already matches 219/219 registers, which is an L1 claim about the register
FILE and says nothing about whether a driver written against those registers
makes a panel light up.

The same assertions run in process as `efr32_deck_behavior::
the_deck_firmware_drives_every_part`, because the CLI lab runs in the
coverage-matrix workflow and that is not a required PR check — a gate that
only runs elsewhere is not holding anything.

## Parts that DO attach on this board

Gated by `crates/core/tests/efr32_onboarded_parts.rs`, which builds each one
through `SystemBus::from_config` against the real `efr32mg26.yaml` — the same
path a lab takes, so a chip descriptor that lost a peripheral would fail here
rather than at someone's desk.

| Part | Connection | Notes |
|------|-----------|-------|
| `st7789-170x320` | `spi0` | USART0 in SPI mode. See `st7789-system.yaml`. |
| `inmp441` | `spi2` | USART2 in I2S mode. NOT the same block as the panel: `I2SCTRL.EN` switches a whole USART, so SPI and I2S cannot share one. |
| `slide-potentiometer` | `iadc0` | Reaches the shared potentiometer kit through TYPE_ALIASES. |
| `toggle-switch`, `button-module`, `rotary-encoder` | `board_io` | GPIO. The encoder needs three pins: A, B and the shaft switch. |

⚠️ **The pads are shared, not spare.** UG594 Table 3.1 (p.10): "pins may be
shared between the breakout pads and other functions". Of the 28 pads,
**PD03 (pad 6) is the only GPIO with no shared feature** — everything else is
also a mikroBUS or Qwiic signal, a power rail, a VREF, a BOARD_ID line or a PTI
line. Wiring anything to the pads means keeping the mikroBUS socket empty.

**The 28 pads carry fifteen MCU GPIO** (PC00–PC07, PD02–PD05, PA04, PA05, PA07)
plus four dedicated analog inputs (AIN0–AIN3). The rest are GND ×2, 5V, VMCU,
3V3, VREFN, VREFP and the two BOARD_ID lines. `agent-deck-system.yaml` spends
all fifteen.

PD04/PD05 are the PTI pads. PTI is a debug **tap** that copies radio packets to
the J-Link — **BLE does not need it**, so spending those two as GPIO costs you
packet tracing in Network Analyzer, not the radio. Take them back if you want
tracing on a build; the deck then loses two contacts, not a device.

Pins that are NOT on the pads and so not usable: PA01/PA02/PA03 (SWD — the
on-board J-Link holds them for the whole debug session), PB02/PB03 (VCOM
console), PB00/PB01 (BTN0/BTN1), PC08/PC09 (LED0/LED1). The last four are on
the board and the deck deliberately leaves them free.

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
