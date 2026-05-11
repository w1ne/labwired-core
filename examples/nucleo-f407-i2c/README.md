# Nucleo-F407 I²C Onboarding

Bare-metal Rust firmware that **actually drives the STM32 I²C state
machine** — START → SB → ADDR → DR transfers with explicit SR1 / SR2
polling — against two real-silicon I²C devices attached via the
`external_devices` yaml mechanism:

- **AHT20** @ `0x38` — command-stream protocol with BUSY-poll
- **BMP280** @ `0x76` — register-bank chip-ID read (expect `0x58`)

This is the hardware-oracle anchor for the LabWired STM32 I²C lane.
F407 silicon is on the bench; F401 will follow as a yaml delta once
the lane is silicon-verified.

## What's different from `examples/demo-blinky`

`demo-blinky` was the historical "I²C example" for STM32 but it cheated
the I²C state machine — it read `I2C1_DR` directly without driving
START/SB/ADDR. That worked for a stub but proved nothing about the
peripheral model. This example does the full transaction so the
simulator and silicon execute the same code path.

## Build

```bash
cargo build --release -p nucleo-f407-i2c
```

Produces `target/thumbv7em-none-eabi/release/nucleo-f407-i2c`. ELF
entry is at `0x08000401` (flash base + thumb bit).

## Run in the simulator

```bash
labwired \
    --firmware target/thumbv7em-none-eabi/release/nucleo-f407-i2c \
    --system examples/nucleo-f407-i2c/system.yaml
```

You should see at boot:

```
i2c attach: 'aht20' (type=aht20) -> 'i2c1'
i2c attach: 'bmp280' (type=bmp280) -> 'i2c1'
```

The LED on PA5 (modeled via `board_io` in `system.yaml`) lights when
both devices respond correctly.

## What this proves vs. doesn't prove

**Proves (simulator side):**
- F407 chip yaml loads cleanly; I²C1 wires through the legacy F1/F2/F4
  register layout.
- Firmware compiles to a valid Cortex-M4F ELF and exercises the I²C
  peripheral end-to-end.
- AHT20 + BMP280 device models attach via `external_devices` and respond
  to the firmware's drive sequence.

**Does not yet prove (waiting on hardware):**
- Byte-for-byte SR1/SR2/DR/CR1/CR2 parity with real F407 silicon.
- Whether the simulator's I²C state machine timing matches silicon.

The remaining proof lands when AHT20 + BMP280 hardware arrives and the
oracle capture is recorded. See
[`docs/boards/stm32f407.md`](../../docs/boards/stm32f407.md) for the
onboarding roadmap.

## Files

| Path                | Purpose                                                                     |
|---------------------|-----------------------------------------------------------------------------|
| `src/main.rs`       | Bare-metal Rust firmware with explicit I²C state-machine drive code         |
| `Cargo.toml`        | `cortex-m` + `cortex-m-rt` + `panic-halt` (no HAL crate — raw register writes) |
| `memory.x`          | Linker memory map: 1 MB flash @ 0x08000000, 128 KB SRAM @ 0x20000000        |
| `build.rs`          | Copies `memory.x` into the link search path                                 |
| `.cargo/config.toml`| Pins target to `thumbv7em-none-eabi` (Cortex-M4, soft-float)                |
| `system.yaml`       | LabWired system manifest — chip ref + AHT20 + BMP280 + LED                  |
