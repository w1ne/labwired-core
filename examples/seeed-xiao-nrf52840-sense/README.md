# Seeed XIAO nRF52840 Sense

This example onboards the Seeed XIAO nRF52840 Sense as a LabWired board target.
It reuses the generic `nrf52840` chip descriptor and adds board-level wiring for
the XIAO RGB LED and SPI bus.

## Modeled Board I/O

| Signal | nRF pin | LabWired binding | Notes |
|--------|---------|------------------|-------|
| Red LED | P0.26 | `led_red` | Active-low |
| Green LED | P0.30 | `led_green` | Active-low |
| Blue LED | P0.06 | `led_blue` | Active-low |
| SPI SCK | P1.13 | `spi0` register config | XIAO D8 |
| SPI MISO | P1.14 | `spi0` register config | XIAO D9 |
| SPI MOSI | P1.15 | `spi0` register config | XIAO D10 |

## Build

```bash
cargo build -p firmware-nrf52840-demo --release --target thumbv7em-none-eabi
```

## Run Smoke Test

```bash
cargo run -q -p labwired-cli -- test \
  --script examples/seeed-xiao-nrf52840-sense/uart-gpio-spi-smoke.yaml \
  --output-dir out/seeed-xiao-nrf52840-sense/uart-gpio-spi-smoke \
  --no-uart-stdout
```

The smoke firmware enables UART0, configures the RGB LED GPIO lines, configures
SPIM0 pin select / frequency / TXD registers, starts an SPIM transfer, and emits
`NRF52840_SMOKE_OK` over UART.

## Fidelity Scope

This target is verified for LabWired register-level smoke coverage of UART0,
GPIO OUT/OUTSET/OUTCLR/DIR/DIRSET/DIRCLR, and a minimal SPIM0 task/event path.
The SPIM model records register state and completes `TASKS_START`, but it does
not yet fetch EasyDMA buffers from system memory or route bytes to attached SPI
devices. Full hardware parity for SPI waveforms requires a SWD probe or logic
analyzer capture.
