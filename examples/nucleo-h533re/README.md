# ST NUCLEO-H533RE quick start

## Demand rank

The STM32H533RE is in MCU demand rank **#4**, “other STM32” (13%). STM32
Nucleo/Discovery boards are kit rank **#4** (23%). This is a current H5
Nucleo-64 board; ST lists the board Active and In Stock. See
[vendor-status.md](vendor-status.md) for the live status record.

## Copy-paste smoke

The no-std Rust smoke blinks the board's green LD2 on PA5 and checks that the
GPIO output follows each transition. Build and run it from the repository root:

```bash
cargo build -p firmware-nucleo-h533re-smoke --release --target thumbv8m.main-none-eabi
cargo run -q -p labwired-cli -- test \
  --script examples/nucleo-h533re/io-smoke.yaml \
  --output-dir out/nucleo-h533re/io-smoke --no-uart-stdout
```

The smoke requires at least eight PA5 edges. The modeled board profile includes
the H5 RCC, GPIOA/GPIOC, SysTick, LD2 and B1. It intentionally does not claim
USART/VCP, USB, crypto, timers, or other board peripherals.

The committed evidence is under [evidence/](evidence/): the local engine smoke
passed 2/2 assertions and recorded 50 PA5 transitions over 200,000 steps. Its
fidelity report also says the board power net is not routed, so VDD is assumed
ideal and power-on/brown-out behavior is not simulated. This profile uses no
known zero-answering peripheral stubs and has no physical-board capture.
