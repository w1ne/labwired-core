# nRF54LM20A + RM67162 AMOLED — Snake

Snake running on a Nordic **nRF54LM20A** driving a **Raydium RM67162**
240×536 AMOLED over `SPIM22`, steered by the nRF54LM20 DK's own four buttons.

```
make            build the ELF
make publish    build and copy to tests/fixtures/ (the committed fixture)

labwired test --script .labwired/lab.yaml    # the lab
labwired test --script io-smoke.yaml         # the strict-onboarding gate
```

## What this lab is evidence for

The chip profile, the nRF54L SPIM model and the RM67162 panel model each have
unit tests. None of them proves the others. This is the end-to-end run that
reaches all three through `from_config` on a booted machine:

| exercised | how |
|---|---|
| chip boot | vector table at RRAM 0x0, `.data` copy, `.bss` zero |
| UARTE20 | EasyDMA TX at 115200 → the DK console |
| SPIM22 | EasyDMA → panel init → a pixel stream per cell |
| GPIO P1 | four LEDs, plus three of the four buttons |
| GPIO P0 | the fourth button — the ports are genuinely mixed |
| RM67162 | DCS init, `CASET`/`RASET` windowing, `RAMWR` |

## Two things worth reading the source for

**Hardware D/C.** Every byte to the panel is one EasyDMA transfer whose first
byte is the command and whose remainder is data, with `DCXCNT = 1`. The
controller holds the panel's D/C line low for that first byte and high for the
rest, from `PSEL.DCX`. There is no D/C GPIO in this firmware and no pin write
between a command and its data — a pixel burst is a single bus transaction
rather than a toggle sandwich. That is new on the nRF54L generation; an
nRF52-era board wires D/C to a GPIO the firmware toggles.

**Brightness.** `WRDISBV` (0x51) is written explicitly. On a backlit TFT this
command does not exist and brightness is a separate backlight pin. On an
emissive panel it resets to `0x00`, so deleting that one line leaves every
other assertion in the lab passing and the panel black. The panel model
reports `lit` (DISPON **and** awake **and** brightness > 0) separately from
`display_on` for exactly this reason.

## The assertions are about the picture, not the transaction

Every UART line here would print with **nothing** on the far side of the SPI
bus: `EVENTS_END` fires whether or not a panel is listening. The
`display_region` assertions measure what landed in frame memory — ink in the
240×520 play area, and the never-addressed 16-row strip below it required to
be *exactly* black.

Verified to discriminate: setting `DCXCNT` to 0 in the firmware, so no byte is
ever framed as a command, leaves all five UART assertions passing and takes
the play area to **0.0% inked**.

## Playing it

With no button pressed the snake heads for the food and refuses suicidal
moves, so an unattended run keeps playing — it reaches score 24 before boxing
itself in, then restarts. The four buttons (`btn_up`, `btn_down`, `btn_left`,
`btn_right`) are drivable as stimuli and override the autopilot while held.

## Sources

Addresses come from the Nordic MDK vendor SVD (`nrf54lm20a_application.svd`,
vendored at `tests/fixtures/real_world/nrf54lm20a.svd`) and the Zephyr
devicetree `dts/vendor/nordic/nrf54lm20_a_b.dtsi`. Pin assignments come from
`boards/nordic/nrf54lm20dk`.

**Do not copy addresses from the nRF54L15 profile.** This is not that part with
more memory: `TAMPC` moved to 0x500EF000, `RRAMC` to 0x5004E000, `CLOCK`'s IRQ
to 270, P1 grew from 17 pins to 32, and there is a fourth GPIO port.
