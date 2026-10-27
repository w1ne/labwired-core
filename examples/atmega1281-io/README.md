# ATmega1281 I/O smoke

The bare 64-pin ATmega1281 uses the exact 128 KB flash profile and two-byte
program counter in LabWired's public AVR interpreter. The 54 package GPIO pads,
USART0 and eight bonded ADC inputs are declared from Microchip DS40002211A.

The AVR-GCC smoke program is in `src/main.c`. Its committed ELF is compiled for
`atmega1281`, starts at the device reset vector, prints `1281 OK` through
USART0, and drives PB5 high. The same ELF is loaded by the core integration
test, the `labwired test` script, and the Playground bare-chip starter.

```bash
# from core/
examples/atmega1281-io/ci/build.sh
examples/atmega1281-io/ci/test.sh
```

This target is simulation-only in Playground. The source is visible and the
committed AVR-GCC image runs; the hosted builder has no AVR source compiler
profile. Editing the C source requires rebuilding the ELF with AVR-GCC.

## Model boundary

- GPIO on ports A..F and PG0..PG5, USART0 TX, and ADC0..ADC7 are declared.
- EEPROM, USART1, Timer3 and PWM behavior are not modeled.
- PG6/PG7, ports H..L, ADC8..ADC15 and Timer4/5 belong to other package or
  sibling profiles and are not exposed here.
- No physical-board or silicon-capture acceptance is claimed.

The sources and validation record are in `REQUIRED_DOCS.md`,
`EXTERNAL_COMPONENTS.md`, and `VALIDATION.md`.
