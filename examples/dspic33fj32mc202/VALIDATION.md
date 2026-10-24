# Validation — dsPIC33FJ32MC202 (M0)

Evidence for the 28-pin sibling of the dsPIC33FJ32MC204. The 202 shares the
204's core, memory and UART1; this milestone claims **M0 only** (boot +
UART1 TX). The 204's M1–M3 peripheral models are available to the shared
silicon but are not re-validated on 202 package pins here.

## Status

| Area | State | Evidence |
| :--- | :--- | :--- |
| Chip descriptor | ✅ | `configs/chips/dspic33fj32mc202.yaml` (DS70283H memory/UART facts) |
| System manifest | ✅ | `configs/systems/dspic33fj32mc202-demo.yaml` |
| Example package | ✅ | `examples/dspic33fj32mc202/{system.yaml,README.md,REQUIRED_DOCS.md,EXTERNAL_COMPONENTS.md,VALIDATION.md}` |
| M0 boot + UART1 | ✅ | real XC16 v1.70 ELF; `crates/core/tests/pic24_xc16_202_smoke.rs` |
| CLI / strict-onboarding path | ✅ | `labwired test --script examples/dspic33fj32mc202/io-smoke.yaml` |
| GPIO/PWM/QEI/ADC/SPI/I²C on 202 pins | ⚠ shared core models, package not validated | next milestone |
| Silicon capture / oracle | ❌ none | simulation-derived only |

## Evidence

```bash
# 1. Real XC16 firmware boots and transmits HI; LATB0 toggles
cargo test -p labwired-core --test pic24_xc16_202_smoke
#   test result: ok. 2 passed; 0 failed

# 2. The CLI path used by strict onboarding
cargo run -q -p labwired-cli -- test \
  --script examples/dspic33fj32mc202/io-smoke.yaml --no-uart-stdout
#   PASS  2/2 checks · io-smoke · 400 steps

# 3. Conformance row + catalogue build
cargo test -p labwired-core --test chip_conformance
#   test result: ok. 8 passed; 0 failed
cargo test -p labwired-core --test session_catalog
#   test result: ok. 1 passed; 0 failed (52-chip catalogue incl. 202)
```

Runtime facts: the ELF resets to PC 0 and vectors through the XC16 GOTO;
UART1 emits `HI`; the main loop toggles `LATB` bit 0. There is no silicon
capture, so no reset-oracle match is claimed.

## Scope and limitations

- The 202 package has 28 pins and exposes AN0–AN5 only; the 204's 44-pin
  pad mask and AN6–AN8 must not be used for a 202 board. Package pin maps
  are the next milestone.
- The descriptor intentionally has no bus peripherals at M0: UART1 and the
  other controllers are CPU-internal.
- No external devices are attached (`EXTERNAL_COMPONENTS.md`).

Sources: DS70283H (202/204/16MC304 data sheet), DS70000157G (16-bit MCU/DSC
PRM). Shared models: `docs/pic24-m0-reference.md`.
