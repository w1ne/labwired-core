# NUCLEO-G071RB Onboarding Example

Run all commands from `core/`.

## Purpose

Deterministic bring-up smoke for the ST NUCLEO-G071RB (STM32G071RB,
Cortex-M0+) using the minimal supported subset:

1. `rcc` — the **STM32G0** RCC layout (CR/ICSCR/CFGR/PLLCFGR @0x00–0x0C,
   3-bit CFGR.SW/SWS, IOPENR@0x34 / APBENR1@0x3C enable gates)
2. `gpioa` / `gpioc` (`stm32v2` GPIO layout on the 0x50000000 IOPORT bus)
3. `usart2` — ST-LINK VCP console (PA2/PA3, AF1)
4. `systick`

The smoke firmware prints `OK\n` on USART2 and toggles LD4 (PA5). Peripherals
beyond that path are declared in the chip yaml with real bases/IRQs and are
covered by the register-vs-SVD measurement, but are **not** G0-tuned yet.

**SIM-DERIVED / L3 production-ready.** No silicon diff exists for this part.
All six tier-1 classes pass for the documented scenarios; see `VALIDATION.md`
for what is proven and `KNOWN_LIMITATIONS.md` for what is not.

## Quick run

```bash
# 1. Build the smoke firmware (Cortex-M0+ → thumbv6m-none-eabi)
rustup target add thumbv6m-none-eabi   # one-time
cargo build -p firmware-stm32g0-demo --release --target thumbv6m-none-eabi

# 2. Run it through the CLI
cargo run -q -p labwired-cli -- \
  --firmware target/thumbv6m-none-eabi/release/firmware-stm32g0-demo \
  --system configs/systems/nucleo-g071rb.yaml \
  --max-steps 200000
```

Expected UART output (USART2 → stdout):

```
OK
```

Or as an asserted script:

```bash
cargo run -q -p labwired-cli -- test \
  --script examples/nucleo-g071rb/uart-smoke.yaml \
  --output-dir out/nucleo-g071rb/uart-smoke \
  --no-uart-stdout
```

## Analog examples

[`analog-ntc-diode-system.yaml`](analog-ntc-diode-system.yaml) routes a generic
Beta NTC divider and Shockley diode netlist into the modeled PA0/PA1 GPIO inputs.
This verifies voltage solving and GPIO threshold routing; the threshold ratios
are test assumptions.

[`analog-ntc-diode-adc-system.yaml`](analog-ntc-diode-adc-system.yaml) routes
the same generic devices to explicit ADC1 IN4/IN5 endpoints.
[`analog-jfet-terminal-resistance-adc-system.yaml`](analog-jfet-terminal-resistance-adc-system.yaml)
adds matched level-1 NJF branches with RD/RS on ADC1 IN6–IN9. These fixtures
prove ADC routing and firmware-visible conversion in the model; their channel
indices are explicit co-simulation endpoints, with no PAx alias or silicon
analog capture claimed. Run the end-to-end proofs from `core/` with:

```bash
cargo test -p labwired-core --test cosim_nucleo_g071rb_analog
```

## Board pinout (UM2505 / Zephyr `nucleo_g071rb` DTS)

| Signal      | Pin  | Alt-fn | Wired to                              |
|-------------|------|--------|---------------------------------------|
| USART2_TX   | PA2  | AF1    | ST-LINK V3 Virtual COM Port           |
| USART2_RX   | PA3  | AF1    | (same)                                |
| LD4 LED     | PA5  | output | green user LED, active high           |
| B1 button   | PC13 | input  | user button, active low               |
| SWDIO       | PA13 | SWD    | ST-LINK (debug)                       |
| SWCLK       | PA14 | SWD    | ST-LINK (debug)                       |

> Cortex-M0+ has **no JTAG TAP** — the only debug transport is 2-wire SWD.

## Files

1. `system.yaml` — local board mapping for simulation runs.
2. `uart-smoke.yaml` — deterministic CLI smoke assertion (task artifact).
3. `io-smoke.yaml` — same assertion in the strict-onboarding gate's
   conventional filename (the gate builds the firmware itself).
4. `analog-ntc-diode-system.yaml` — generic MNA device netlists routed to
   modeled GPIO inputs; the fixture uses test-only threshold assumptions.
5. `REQUIRED_DOCS.md` — source-grounding references (RM0444, DS12232, UM2505).
6. `EXTERNAL_COMPONENTS.md` — external component declaration.
7. `VALIDATION.md` — reproducible validation/audit commands and evidence.
8. `KNOWN_LIMITATIONS.md` — L2-required Not modelled / Partially modelled /
   Proven at L3 boundary.
