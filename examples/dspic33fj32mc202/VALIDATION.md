# Validation — dsPIC33FJ32MC202 (M0 + M1 + M2)

Evidence for the 28-pin sibling of the dsPIC33FJ32MC204. The 202 shares the
204's core, memory and register map; the track now claims **M0–M2** — boot +
UART1 TX, the 28-pin GPIO package, and the shared T1/PWM1/QEI1 models on
202 package pins. ADC/SPI/I²C remain for the next milestone.

## Status

| Area | State | Evidence |
| :--- | :--- | :--- |
| Chip descriptor | ✅ | `configs/chips/dspic33fj32mc202.yaml` (DS70283H memory/UART facts) |
| System manifest | ✅ | `configs/systems/dspic33fj32mc202-demo.yaml` |
| Example package | ✅ | `examples/dspic33fj32mc202/{system.yaml,README.md,REQUIRED_DOCS.md,EXTERNAL_COMPONENTS.md,VALIDATION.md}` |
| M0 boot + UART1 | ✅ | real XC16 v1.70 ELF; `crates/core/tests/pic24_xc16_202_smoke.rs` |
| M1 28-pin package/GPIO | ✅ | pads RA0-4/RB0-15/no RC, real XC16 GPIO firmware; `crates/core/tests/pic24_xc16_202_m1.rs` |
| CLI / strict-onboarding path | ✅ | `labwired test --script examples/dspic33fj32mc202/io-smoke.yaml` |
| M2 T1/PWM1/QEI1 on 202 pins | ✅ | real XC16 M2 ELF; `crates/core/tests/pic24_xc16_202_m2.rs` (PWM pads, quadrature, index reset, T1IF) |
| ADC/SPI/I²C on 202 pins (AN0-AN5) | ⏳ shared core models, next milestone | — |
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

# 2c. M2 T1/PWM1/QEI1 on 202 pins
cargo test -p labwired-core --test pic24_xc16_202_m2
#   test result: ok. 1 passed; 0 failed

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
follows the latch. The M2 ELF waits on T1IF, configures PWM1 (P1TPER 15,
dead time 2 TCY) and QEI1 (x4 index reset, MAXCNT 199); the test drives real
pad edges on RB0-RB2, reads the PWM pads on RB14/RB15, and checks POS1CNT,
UPDN and the direction-aware index reset. There is no silicon capture, so no
reset-oracle match is claimed.

## Scope and limitations

- The M1 descriptor models the 28-pin package masks (RA0-4, RB0-15, no
  PORTC). M2 validates the shared T1/PWM1/QEI1 models only on the pins the
  28-pin package has; the AN0–AN5 analog set and ADC/SPI/I²C are the next
  milestone. The 204's 44-pin pad mask and AN6–AN8 must not be used for a
  202 board.
- UART1 and the other controllers remain CPU-internal; the three pads are
  the only bus peripherals.
- No external devices are attached (`EXTERNAL_COMPONENTS.md`).

Sources: DS70283H (202/204/16MC304 data sheet), DS70000157G (16-bit MCU/DSC
PRM). Shared models: `docs/pic24-m0-reference.md`.
