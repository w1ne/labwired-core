# IO-Link DI/DO IO-Link DI device (simulated)

Runs the real [`iolinki`](../../third_party/iolinki) IO-Link **device** stack as
firmware on a simulated STM32L476, modeling an IFM IO-Link DI/DO 8-port IO-Link
**digital-input** hub. A native IO-Link master peer drives the link; the device
reads 8 digital inputs from a 74HC165 shift register over SPI1 and publishes them
as cyclic process data.

```
[74HC165 inputs] --SPI1--> iolinki device stack (firmware) --USART2--> iolink-master
                                                                          (shows PD)
```

Nothing of IO-Link is re-implemented in the simulator: the `iolinki` stack runs
as the firmware-under-test via a thin `phy_labwired.c` PHY over the L476 USART2.
LabWired provides the two simulated peripherals (the `sn74hc165` shifter and the
native `iolink-master`) and carries the UART bytes.

## Build the firmware

```sh
make -C firmware            # needs arm-none-eabi-gcc; produces firmware/iolink_dido.elf
```

The firmware is plain C with its own startup/linker (no vendor SDK). It is built
at `-O0` on purpose: this toolchain (arm-none-eabi GCC 10.2) miscompiles local
aggregate initialisation at `-Os` here.

## Run headless in the simulator

```sh
cargo run --release -p labwired-cli -- test --script examples/iolink-dido/test.yaml
```

Expected output (the device walks STARTUP → … → OPERATE and reports the input
byte read from the shifter, preset to `0xA5` in `system.yaml`):

```
IOLINK DIDO BOOT
IOLINK INIT OK
STATE=01
STATE=04 OPERATE PD=A5
```

## Files

- `firmware/` — startup, linker, debug UART, `phy_labwired.c` (USART2 PHY), the DI
  app `main.c`, and the `Makefile` that compiles the vendored `iolinki` sources.
- `system.yaml` — L476 board with the `iolink-master` on `uart2` and the
  `sn74hc165` on `spi1` (its `inputs:` preset is headless demo stimulus).
- `test.yaml` — headless run + assertions (`OPERATE PD=`).

The `inputs:` preset stands in for live switch toggling, which the playground UI
adds in a later step.
