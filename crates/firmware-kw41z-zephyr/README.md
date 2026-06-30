# firmware-kw41z-zephyr

Reproducer for the two **unmodified upstream Zephyr v3.7** fixture ELFs that the
`firmware_survival` tests boot on the NXP **MKW41Z4** (KW41Z, Cortex-M0+):

| fixture (`tests/fixtures/`)       | Zephyr sample              | what it proves |
|-----------------------------------|---------------------------|----------------|
| `kw41z-zephyr-hello.elf`          | `samples/hello_world`     | stock Zephyr boots through the Kinetis MCG FEE clock bring-up and prints over LPUART0 |
| `kw41z-zephyr-fxos8700.elf`       | `samples/sensor/fxos8700` | the stock Zephyr `fxos8700` sensor driver probes + streams the on-board accelerometer/magnetometer over I2C1 — a CowManager-style livestock **activity** node |

Neither ELF is patched. The point is that LabWired runs the firmware an NXP
customer would actually flash:

- The hello fixture exercises the behavioural MCG / RSIM / LPUART models.
- The fxos8700 fixture additionally exercises the interrupt-driven Kinetis I2C
  master (`crates/core/src/peripherals/i2c.rs`, `KinetisI2c`, IRQ 9) talking to
  the FXOS8700 device model (`crates/core/src/peripherals/components/fxos8700.rs`)
  wired onto I2C1 in `configs/systems/frdm-kw41z.yaml`. It is built in polling
  mode (`CONFIG_FXOS8700_TRIGGER_NONE=y`) so no sensor data-ready GPIO interrupt
  is required; hybrid accel+mag and the die-temperature channel are the sample's
  own defaults.

## Build

```sh
./build.sh          # -> tests/fixtures/kw41z-zephyr-{hello,fxos8700}.elf
```

Requirements: a Zephyr v3.7.x west workspace (`$ZEPHYRPROJECT`, default
`~/zephyrproject`) and `arm-none-eabi-gcc` on `PATH` (the `gnuarmemb` toolchain
variant — no Zephyr SDK install needed).

This directory is a build wrapper only; it is not a Cargo crate.
