# ST NUCLEO-H533RE quick start

## Demand rank

The STM32H533RE is in MCU demand rank **#4**, “other STM32” (13%). STM32
Nucleo/Discovery boards account for **23%** of the survey's kits. This is a
peripheral-coverage extension to the already onboarded H533RE, not a new board
or MCU demand pick. ST lists this current H5 Nucleo-64 board Active and In
Stock. See
[vendor-status.md](vendor-status.md) for the live status record.

## Copy-paste smoke

The no-std Rust smoke writes `H533_USART2_OK` over the board's default
STLINK-V3EC virtual COM port (USART2 on PA2/PA3), blinks the green LD2 on PA5,
and checks that the GPIO output follows each transition. Build and run it from
the repository root:

```bash
cargo build -p firmware-nucleo-h533re-smoke --release --target thumbv8m.main-none-eabi
cargo run -q -p labwired-cli -- test \
  --script examples/nucleo-h533re/io-smoke.yaml \
  --output-dir out/nucleo-h533re/io-smoke --no-uart-stdout
```

The smoke requires the USART2 marker and at least eight PA5 edges. The profile
includes the H5 RCC, GPIOA/GPIOC, USART2, SysTick, LD2, B1, and the default
USART2 console selection. The engine captures USART2 output from its register
model; it does not simulate the STLINK-V3EC bridge or validate the PA2/PA3 pad
waveform. USB device behavior, crypto, timers, and other board peripherals
remain outside the profile.

The committed evidence is under [evidence/](evidence/). The smoke checks both
USART2 output and PA5 transitions. Its fidelity report says the board power net
is not routed, so VDD is assumed ideal and power-on/brown-out behavior is not
simulated. This profile uses no known zero-answering peripheral stubs and has
no physical-board capture.
