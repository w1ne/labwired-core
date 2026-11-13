# Raspberry Pi Pico 2 (RP2350) board smoke

Run all commands from `core/`.

## Purpose

Demand rank: **RP2350 family, rank 5** in the measured MCU list (12%); the
RP2040/RP2350 family is the #2 demand-ordered target group. This is board-level
integration and smoke coverage for the existing RP2350 model, not a new silicon
model.

The canonical system is [`configs/systems/pico2.yaml`](../../configs/systems/pico2.yaml).
It maps the onboard LED to GP25/SIO. The smoke toggles that LED through the SIO
output latch and checks captured UART0 output at the RP2350 PL011 base
(`0x40070000`). The official pinout maps GP0/UART0 TX and GP1/UART0 RX to
header pins 1 and 2. On real hardware, they need an external UART adapter;
this smoke does not claim USB CDC or USB mass-storage support.

| Pico 2 signal | Board connection | Twin mapping |
| --- | --- | --- |
| GP25 | onboard user LED | `led_gp25` via `sio` |
| GP0 / UART0 TX | header pin 1 | UART0 output capture |
| GP1 / UART0 RX | header pin 2 | UART0 input pin |

See the [official Pico 2 pinout](https://pip-assets.raspberrypi.com/categories/1005-raspberry-pi-pico-2/documents/RP-008301-DS-1-Pico-2-Pinout.pdf)
for the complete 40-pin header map.

Pico 2 has an RP2350 with dual Cortex-M33 cores, 4 MiB QSPI flash at
`0x10000000`, and 520 KB SRAM at `0x20000000`. This smoke runs core 0 only and
reuses RP2040 behavioural models with the RP2350 clock/reset profile and moved
APB map.

## Quick Run

```bash
rustup target add thumbv8m.main-none-eabi   # once
RUSTFLAGS="-C link-arg=-Tlink.x" cargo build -p firmware-rp2350-demo \
  --release --target thumbv8m.main-none-eabi
cargo run -q -p labwired-cli -- test \
  --script examples/pico2/io-smoke.yaml \
  --output-dir out/pico2/io-smoke \
  --no-uart-stdout
```

Expected result:
1. smoke test passes (exit 0)
2. UART contains `PICO2_GPIO25_UART_OK`
3. GP25 has at least two observed edges

## Files

1. [`configs/systems/pico2.yaml`](../../configs/systems/pico2.yaml): board mapping.
2. `io-smoke.yaml`: UART0 and onboard LED smoke assertions.
3. `REQUIRED_DOCS.md`: source-grounding references.
4. `EXTERNAL_COMPONENTS.md`: external component declaration.

The live [Raspberry Pi Pico 2 product page](https://www.raspberrypi.com/products/raspberry-pi-pico-2/)
listed it as available now from $5 and says the Pico 2 series will remain in
production until at least January 2040 when checked on 2026-11-11; see
[`evidence/vendor-status.md`](evidence/vendor-status.md).
