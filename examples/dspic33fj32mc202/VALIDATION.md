# Validation — dsPIC33FJ32MC202 (M0 + M1 + M2 + M3)

Evidence for the 28-pin sibling of the dsPIC33FJ32MC204. The 202 shares the
204's core, memory and register map; the track now claims **M0–M3** — boot +
UART1 TX, the 28-pin GPIO package, the shared T1/PWM1/QEI1 models on 202
package pins, and the shared ADC1/SPI1/I2C1 models on the 202's AN0-AN5 analog
set and serial pins. There is no silicon capture.

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
| M3 ADC1/SPI1/I2C1 on 202 pins | ✅ | real XC16 M3 ELF; `crates/core/tests/pic24_xc16_202_m3.rs` (AN4=RB2, AN0-AN5 only; SPI1 0xA5 loopback; TMP102 at 0x48; `M3 OK`, marker 0xBEEF) |
| M3 demo system | ✅ | `system-m3.yaml` — TMP102 on `i2c1`, loopback on `spi1` |
| M0-M3 scope | ✅ complete | all four milestones evidenced below; nothing remains in M0-M3 |
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

# 2d. M3 ADC1/SPI1/I2C1 on 202 pins
cargo test -p labwired-core --test pic24_xc16_202_m3
#   test result: ok
#   passes when the M3 ELF reaches 0xBEEF with ADC1BUF0 = 0x2AA (AN4),
#   SPI1 = 0xA5 (loopback), TMP102 = 0x19/0x00 (25.0 °C) and "M3 OK\n" on UART1

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
UPDN and the direction-aware index reset.

The M3 ELF programs ADC1 in 10-bit unsigned-integer mode, samples AN4 (RB2)
with the harness code 0x2AA, shifts 0xA5 through the SPI1 loopback, and reads
the TMP102 temperature register at 0x48 (`0x19`/`0x00` = 25.0 °C) over I2C1;
it then prints `M3 OK\n` on UART1 and writes `0xBEEF` to 0x0E00. The test also
checks the programmed register image (`ADON`, `AD1PCFGL = 0x01EF`,
`CH0SA = AN4`, `SPI1CON1 = 0x0137`, `SPIEN`). The analog set is AN0-AN5 only
(AN0=RA0, AN1=RA1, AN2=RB0, AN3=RB1, AN4=RB2, AN5=RB3); AN6-AN8 are RC0-RC2
and absent on the 28-pin package. The TMP102 and SPI loopback are attached
from `system-m3.yaml` through the same factory calls the session builder
uses; the base `system.yaml` stays device-free for the CLI `test` path. There
is no silicon capture, so no reset-oracle match is claimed.

## Scope and limitations

- The M1 descriptor models the 28-pin package masks (RA0-4, RB0-15, no
  PORTC). M2 validates the shared T1/PWM1/QEI1 models only on the pins the
  28-pin package has. M3 validates the shared ADC1/SPI1/I2C1 models on the
  202's AN0-AN5 analog set and serial pins (AN4 is RB2). AN6-AN8 are RC0-RC2
  and absent on the 28-pin package; the 204's 44-pin pad mask and AN6-AN8
  must not be used for a 202 board. The 202 track covers M0-M3, with no
  further milestone remaining in that range.
- M3 shares the 204's documented approximations: ADRC = 1 reuses the
  `(ADCS + 1) × TCY` TAD formula because the internal RC frequency is not
  characterized; leftover conversion cycles at a step boundary are not
  carried into the next channel; the INT0 ADC trigger and the simultaneous
  (SIMSAM) sample instant are not modelled (channels convert sequentially);
  SPI has no SS/CS routing (the first device answers; the example uses a wire
  loopback); I²C ACK bits are not delivered to the slave model; 10-bit
  addressing is decoded but cannot match a 7-bit slave address; ANx/SPI/I²C
  pin electrical mapping is not routed through the pad model.
- UART1 and the other controllers remain CPU-internal; the three pads are
  the only bus peripherals.
- No external devices are attached in `system.yaml`; `system-m3.yaml`
  declares the TMP102 and SPI loopback for the M3 session path.
- No silicon capture exists for this part; the results are simulation-derived
  and no hardware match is claimed.

Sources: DS70283H (202/204/16MC304 data sheet), DS70000157G (16-bit MCU/DSC
PRM), DS70183D (ADC), DS70206 (SPI), DS70000195 (I²C). Shared models:
`docs/pic24-m0-reference.md`.
