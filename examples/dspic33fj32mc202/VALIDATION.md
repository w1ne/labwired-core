# Validation — dsPIC33FJ32MC202 (M0 + M1)

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
| M1 28-pin package/GPIO | ✅ | pads RA0-4/RB0-15/no RC, real XC16 GPIO firmware; `crates/core/tests/pic24_xc16_202_m1.rs` |
| CLI / strict-onboarding path | ✅ | `labwired test --script examples/dspic33fj32mc202/io-smoke.yaml` |
| ADC/SPI/I²C/PWM/QEI on 202 pins | ⚠ shared core models, pin subset not validated | next milestone |
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

# 2b. M1 package/GPIO: masks, direction registers and pad levels
cargo test -p labwired-core --test pic24_xc16_202_m1
#   test result: ok. 2 passed; 0 failed

# 3. Conformance row + catalogue build
cargo test -p labwired-core --test chip_conformance
#   test result: ok. 8 passed; 0 failed
cargo test -p labwired-core --test session_catalog
#   test result: ok. 1 passed; 0 failed (52-chip catalogue incl. 202)
```

Runtime facts: the M0 ELF resets to PC 0 and vectors through the XC16 GOTO,
UART1 emits `HI` and the main loop toggles `LATB` bit 0. The M1 ELF emits
`G1`, sets PORTA/PORTB to outputs and toggles RA0/RB0; the test checks
`TRISA = 0x001F`, `TRISB = 0xFFFF`, `TRISC = 0` and that the RA0 pad level
follows the latch. There is no silicon capture, so no reset-oracle match is
claimed.

## Scope and limitations

- The M1 descriptor models the 28-pin package masks (RA0-4, RB0-15, no
  PORTC); the AN0–AN5 analog set and the smaller PWM/QEI pin set are the
  next milestone. The 204's 44-pin pad mask and AN6–AN8 must not be used
  for a 202 board.
- UART1 and the other controllers remain CPU-internal; the three pads are
  the only bus peripherals.
- No external devices are attached (`EXTERNAL_COMPONENTS.md`).

Sources: DS70283H (202/204/16MC304 data sheet), DS70000157G (16-bit MCU/DSC
PRM). Shared models: `docs/pic24-m0-reference.md`.
