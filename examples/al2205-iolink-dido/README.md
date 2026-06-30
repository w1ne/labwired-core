# AL2205-style IO-Link DI device (simulated)

Runs the real [`iolinki`](../../third_party/iolinki) IO-Link **device** stack as
firmware on a simulated STM32L476, modeling IFM AL2205-style IO-Link
**digital-input** links. Two native IO-Link master peers drive independent
firmware device contexts on separate UARTs; each device reads its own 8 digital
inputs from a 74HC165 shift register and publishes them as cyclic process data.

```
[74HC165 inputs A5] --SPI1--> iolinki device stack PORT2 --USART2--> iolink-master
[74HC165 inputs 3C] --SPI2--> iolinki device stack PORT3 --USART3--> iolink-master
                                                                            (shows PD)
```

Nothing of IO-Link is re-implemented in the simulator: the `iolinki` stack runs
as the firmware-under-test via a thin `phy_labwired.c` PHY over the L476 USART
registers. LabWired provides the simulated peripherals (the `sn74hc165` shifter
and two native `iolink-master` peers) and carries the UART bytes.

The headless CLI scenario runs a two-port firmware topology inside one simulated
MCU and proves isolated IO-Link stack contexts, separate UART links, and separate
field-input stimulus. The CI conformance lane also runs
`crates/core/tests/iolink_multi_node_world.rs`, which places two independent
STM32L476 firmware machines in a LabWired `World`; each node has its own bus,
CPU, reset domain, UART log, native C `iolinki-master` peers, and IO-Link device
firmware image.

## Build the firmware

```sh
make -C firmware            # needs arm-none-eabi-gcc; produces firmware/al2205_dido.elf
```

The firmware is plain C with its own startup/linker (no vendor SDK). It is built
at `-O0` on purpose: this toolchain (arm-none-eabi GCC 10.2) miscompiles local
aggregate initialisation at `-Os` here.

## Run headless in the simulator

```sh
cargo run --release -p labwired-cli -- test --script examples/al2205-iolink-dido/test.yaml
```

Expected output (each port walks STARTUP -> OPERATE and reports the byte read
from its own shifter, preset to `0xA5` and `0x3C` in `system.yaml`):

```
AL2205 BOOT
IOLINK INIT OK
PORT2 STATE=01
PORT3 STATE=01
PORT2 STATE=04 OPERATE PD=A5
PORT3 STATE=04 OPERATE PD=3C
```

## Files

- `firmware/` — startup, linker, debug UART, `phy_labwired.c` (UART-backed PHY),
  the DI app `main.c`, and the `Makefile` that compiles the vendored `iolinki`
  sources.
- `system.yaml` — L476 board with native `iolink-master` peers on `uart2` and
  `uart3`, plus `sn74hc165` shifters on `spi1` and `spi2` with distinct
  headless input stimulus.
- `test.yaml` — headless run + assertions for both ports reaching OPERATE with
  their own process-data bytes; exits once assertions pass.

The `inputs:` preset stands in for live switch toggling, which the playground UI
adds in a later step.

## Run the multi-chip world test

After building the firmware:

```sh
IOLINKI_MASTER_DIR=/path/to/iolinki-master \
cargo test -p labwired-core --features iolink-native --test iolink_multi_node_world -- --nocapture
```
