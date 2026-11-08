# NUCLEO-G071RB Onboarding Example

Run all commands from `core/`.

## Board-connected smoke lane

The committed onboarding lane now uses the board-connected USART2 VCP on PA2/PA3
and the LD4 LED on PA5. Its narrow system profile is
`configs/systems/nucleo-g071rb-smoke.yaml`; it declares only RCC, GPIOA and
USART2, with no zero-answering stubs and no claims for the board button or other
chip peripherals. The firmware prints `NUCLEO_G071RB_OK` and repeatedly toggles
LD4. Reproduce it with:

```bash
scripts/onboarding_smoke.sh \
  --target-id nucleo-g071rb \
  --crate firmware-nucleo-g071rb-smoke \
  --target thumbv6m-none-eabi \
  --script examples/nucleo-g071rb/onboarding-smoke.yaml \
  --system configs/systems/nucleo-g071rb-smoke.yaml \
  --out-dir examples/nucleo-g071rb/evidence
```

The committed build, simulation output and vendor-status receipt are in
[`evidence/`](evidence/). A separate hosted Arduino oracle smoke is recorded in
[`evidence/hosted-verification.md`](evidence/hosted-verification.md); it checks
the same USART2 marker and PA5 high state, with the hosted twin's stated ideal
VDD assumption.

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
4. `REQUIRED_DOCS.md` — source-grounding references (RM0444, DS12232, UM2505).
5. `EXTERNAL_COMPONENTS.md` — external component declaration.
6. `VALIDATION.md` — reproducible validation/audit commands and evidence.
7. `KNOWN_LIMITATIONS.md` — L2-required Not modelled / Partially modelled /
   Proven at L3 boundary.
