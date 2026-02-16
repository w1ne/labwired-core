# Demo Blinky - GPIO LED Toggle

> Part of the [LabWired Demos](../../../DEMOS.md) suite.

Simple firmware demonstrating GPIO control on STM32F103.

## Features
- Toggles LED on GPIO pin
- Uses SysTick for delays
- Minimal dependencies

## Building

```bash
cd examples/demo-blinky
make
```

## Running in LabWired

```bash
cargo run -p labwired-cli -- \
  --firmware examples/demo-blinky/build/demo-blinky.elf \
  --system system.yaml
```

## Expected Output

```
GPIO: Write to GPIOC_ODR: 0x00002000 (LED ON)
GPIO: Write to GPIOC_ODR: 0x00000000 (LED OFF)
GPIO: Write to GPIOC_ODR: 0x00002000 (LED ON)
...
```
