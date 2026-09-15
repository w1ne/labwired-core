# ATSAMD21G18A (Microchip SAM D21) — Arduino Zero / Feather M0 class

A 48-pin Cortex-M0+ with 256 KB flash at `0x0` and 32 KB SRAM at `0x2000_0000`,
running at 48 MHz once the DFLL closes its loop. It is the silicon behind the
Arduino Zero and MKR line, the Adafruit Feather M0 / QT Py / Trinket M0 family,
and CircuitPython's original target.

This is the **first Microchip part in the engine**, and two things about the
family are unlike anything already onboarded — both places where a wrong model
fails *silently* rather than loudly:

1. **SERCOM is one block that becomes three peripherals.** `CTRLA.MODE` decides
   whether an instance is a USART, an SPI controller or an I²C controller. This
   profile models USART mode; a SERCOM put into another mode goes **inert**
   rather than answering an SPI driver out of a USART register map.
2. **PORT's SET/CLR/TGL registers are aliases, not state.** Silicon reads all
   four DIR aliases back as `DIR` and all four OUT aliases back as `OUT`. A
   model that stored them separately reads back the last write *mask*, so a
   driver that sets a pin through `DIRSET` and then read-modify-writes `DIR`
   loses every other pin on the port — with no error.

The register truth here is Microchip's own `ATSAMD21G18A.svd`, which ships under
**Apache-2.0** and is vendored at `tests/fixtures/real_world/atsamd21g18a.svd`.
Every base address and IRQ number in the chip yaml is checked against it by
`svd_conformance`, with zero justified deviations.

## Status at a glance

| Aspect             | Status                                                                                    |
|--------------------|-------------------------------------------------------------------------------------------|
| Chip yaml          | [`configs/chips/atsamd21g18a.yaml`](../../configs/chips/atsamd21g18a.yaml)                  |
| System yaml        | [`configs/systems/arduino-zero.yaml`](../../configs/systems/arduino-zero.yaml)              |
| Reference firmware | [`crates/firmware-samd21-demo/`](../../crates/firmware-samd21-demo/) (bare-metal, no HAL)   |
| Committed ELF      | `tests/fixtures/samd21-smoke.elf` — runs from a clone plus the CLI, no toolchain             |
| Validation         | `examples/samd21-smoke/io-smoke.yaml` (7 checks) + `atsamd21_peripheral_estate` (4)          |
| Tier               | **sim-validated** — real PORT/SERCOM models, green CI, **no silicon diff**                   |

## What is proven

A bare-metal firmware performing the **real SAM D21 bring-up in datasheet
order** — deliberately not through a HAL, so what passes is a driver's actual
register traffic rather than one convenience wrapper:

1. `NVMCTRL.CTRLB` wait states, before the clock rises.
2. The `SYSCTRL.PCLKSR` ready-flag poll.
3. GCLK generator 0 from OSC8M, then `SERCOM0_CORE` routed to it — three
   `SYNCBUSY` spins.
4. `PM.APBCMASK` — SERCOM0's APB clock is genuinely **off** at reset (APBCMASK
   resets to `0x0001_0000`, ADC only).
5. One `PORT.WRCONFIG` store muxing PA10/PA11 to peripheral function **C**
   (SERCOM0 PAD[2]/PAD[3]), plus PA17 as a plain output.
6. SERCOM0 as a USART — `CTRLB` before `CTRLA.ENABLE`, as the datasheet
   requires.
7. Console output, then the LED pad driven.

The gate is **not** the banner. Three of the seven checks are `memory_value`
reads of the PORT registers the firmware configured — `DIR` bit 17,
`PINCFG[10]` = `PMUXEN|INEN`, and `PMUX[5]`'s low nibble = 2 — which printed
text cannot fake. Non-vacuity was checked by flipping the expected PMUX nibble:
the run fails with `expected 0x3, got 0x22` while every UART line still passes.

## What is NOT proven

Stated plainly, because a fidelity table that only lists successes is marketing:

- **No silicon diff.** Nothing here has been compared against a real SAM D21
  over SWD. Every ✅ below means *the model satisfies the firmware we have run*,
  not *the model matches silicon*. The board is `NOT_SHIPPED` in the coverage
  ratchet for exactly this reason.
- **Nothing is clock-gated.** `PM` is a register bank, not a clock-gate
  controller. A peripheral answers whether or not its `APBCMASK` bit is set, so
  firmware that forgets to unmask a SERCOM works here and fails on hardware.
  This is permissive, and it is recorded rather than quietly accepted.
- **`SYNCBUSY` always reads 0** and **`INTFLAG.DRE` is derived from
  `CTRLA.ENABLE`.** Synchronisation and transmission are instantaneous in this
  model. Those are *modelling* truths, not silicon ones: firmware that measures
  a sync delay or a baud-paced byte will not see it.
- **No narration wire.** TX bytes reach the console sink and the bus trace, but
  no `PadLines` cell is published, so a logic-analyzer **wire** probe on a
  SERCOM has nothing to bind to.
- **Throughput.** 2246.0 Ir/step, the highest in the fleet, with batch width
  1.0 — this bus is not walk-free, because the SERCOM's IRQ edge-detection
  needs the per-cycle walk and walk-deletion is all-or-nothing per bus.

## Support matrix

| Block | State | Notes |
|-------|-------|-------|
| Cortex-M0+ core, NVIC, SysTick | ✅ | Shared Arm models |
| PORT (GROUP 0/1) | ✅ behavioural | DIR/OUT + aliases, IN, WRCONFIG, PMUX, PINCFG |
| SERCOM0–5, USART mode | ✅ behavioural | 8N1, TX/RX, DRE/TXC/RXC, IRQ on the enabled-flag edge |
| SERCOM SPI / I²C modes | ❌ | Instance goes inert rather than answering wrongly |
| SYSCTRL, GCLK, PM, WDT, NVMCTRL | ⚠️ declarative | Register banks; ready flags modelled, no clock tree |
| EIC (`attachInterrupt`) | ❌ | Window unmapped — an access is reported, not absorbed |
| USB | ❌ | A sketch whose console is `SerialUSB` prints nothing yet |
| TCC0–2, TC3–5, ADC, DAC, AC, I²S, DMAC, EVSYS, RTC, DSU | ❌ | Declared not modelled in the chip yaml |

## Pins

`configs/chips/atsamd21g18a.yaml` declares an explicit 64-entry `pins:` map
(`PA0`–`PA31`, `PB0`–`PB31`). It is declared rather than left to the label
parse because the parse is **wrong** for this part twice over: it rejects any
bit above 15, and `PA17` is `LED_BUILTIN`; and it builds port names as
`gpio<letter>`, while these ports are `gpio_a`/`gpio_b` because one LabWired
port is one PORT GROUP.

The map says which port register and bit a label resolves to. It does **not**
assert package bond-out — the G18A is a 48-pin part and does not bring all 64
pads out.

| Board label | Pin | Role |
|-------------|-----|------|
| D13 / `LED_BUILTIN` | PA17 | User LED (`board_io` `led0`) |
| D1 / TX | PA10 | SERCOM0 PAD[2], PMUX C |
| D0 / RX | PA11 | SERCOM0 PAD[3], PMUX C |

## How to run

```bash
# From a clone, with the released CLI — no toolchain needed:
labwired run \
  --firmware tests/fixtures/samd21-smoke.elf \
  --chip configs/chips/atsamd21g18a.yaml

# The full gate, including the PORT register assertions:
labwired test --script examples/samd21-smoke/io-smoke.yaml
```

To rebuild the firmware, build **from the crate directory**, not the workspace
root — its `-Tlink.x` comes from its own `.cargo/config.toml`, and a root build
links entry `0x0` and boots nowhere while the build succeeds:

```bash
cd crates/firmware-samd21-demo && cargo build --release
```

## Related

- [Board onboarding playbook](../board_onboarding_playbook.md)
- [Onboarding candidates](../onboarding-candidates.md) — SAMD51/SAME54 are the
  cheap follow-ons now that the SAM family exists
- [Validation status](VALIDATION_STATUS.md)
