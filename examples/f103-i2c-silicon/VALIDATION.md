# STM32F103RB I²C silicon-validation runbook

Same firmware ELF runs in the LabWired simulator and on a Nucleo-F103RB
attached over ST-Link/V2.1; both runs produce byte-for-byte identical
configuration-register fingerprints.  This is the F1 sibling of the
F407 Round 2 silicon validation.

## Run in the simulator

```bash
cd core
cargo build -p firmware-f103-i2c-demo --release --target thumbv7m-none-eabi
cargo run -q -p labwired-cli -- test \
  --script examples/f103-i2c-silicon/io-smoke.yaml
```

Expected UART trace (printed to stdout by the CLI):

    F103 I2C
    INIT
    CR1=00000001
    CR2=00000008
    CCR=00000028
    TRISE=00000009
    OAR1=00004000
    SR1=00000000
    SR2=00000000
    START
    ... (etc.)
    DONE

## Run on real silicon (Nucleo-F103RB)

```bash
ELF=target/thumbv7m-none-eabi/release/firmware-f103-i2c-demo
openocd -f interface/stlink.cfg -f target/stm32f1x.cfg \
  -c "program $ELF verify reset exit"

# Read final I²C1 fingerprint via SWD:
openocd -f interface/stlink.cfg -f target/stm32f1x.cfg \
  -c "init; reset run; sleep 2000; halt; sleep 200" \
  -c "set cr1 [mrw 0x40005400]; ... shutdown"
```

The configuration registers (CR1/CR2/CCR/TRISE/OAR1) match the
simulator's end state byte-for-byte.  The full captured fingerprint is
committed at
[`tests/fixtures/hw_traces/nucleo_f103rb_i2c_register_fingerprint.txt`](../../tests/fixtures/hw_traces/nucleo_f103rb_i2c_register_fingerprint.txt).

## Status-register divergence (bus electrical, documented)

SR1 / SR2 differ between sim and silicon because the Nucleo-F103RB has
**no external I²C pull-ups on PB6/PB7**.  Real silicon's open-drain
SDA/SCL stay LOW without pull-ups, so the START condition latches on
the chip but the bus state machine can never complete address-phase →
silicon ends with `SR1=0x0001` (SB held, transaction stuck) and
`SR2=0x0003` (MSL+BUSY).  The sim models the peripheral state machine
without bus electrical effects, so a no-slave address NACKs cleanly
and AF latches → sim ends with `SR1=0x0400` (AF) / `SR2=0x0`.

This is *not* a simulator bug.  The simulator uses the same I²C model
code that F407 silicon-validated for the no-slave-AF state machine in
Round 2 (with pull-ups on its capture board).  By transitivity F103
inherits the validation, modulo the bus-electrical caveat documented
here.  Fully reproducing F407 Round 2 on F103 silicon requires adding
external 4.7 kΩ pull-ups from PB6/PB7 to VDD on the Nucleo-F103RB.
