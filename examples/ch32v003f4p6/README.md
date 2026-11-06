# CH32V003F4P6 GPIO blink

This is a small bare-metal C example for the WCH CH32V003F4P6. It enables the
GPIOC clock, configures PC0 as push-pull output, then toggles PC0 through the
WCH set/reset registers. The build targets RV32E instructions, a subset of the
chip's RV32EC CPU; it does not depend on compressed instructions.

Build the ELF with a RISC-V GCC that supports `-march=rv32e -mabi=ilp32e`:

```sh
./examples/ch32v003f4p6/build.sh
```

Run the committed firmware against the local CH32V003 model:

```sh
cargo run -q -p labwired-cli -- test \
  --script examples/ch32v003f4p6/io-smoke.yaml \
  --output-dir out/boards/ch32v003f4p6/io-smoke --no-uart-stdout
```

The smoke requires at least two PC0 output edges. The bare chip model has no
external LED attached; on a physical part, wire PC0 through a current-limiting
resistor to an LED and ground.

This fixture exercises the local simulator's RV32E, reset/flash loading, RCC,
and GPIO paths. CH32V003 is not yet a hosted LabWired verifier target, so this
is not a hosted `model_verified` result or hardware confirmation.
