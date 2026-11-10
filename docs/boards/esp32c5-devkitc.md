# ESP32-C5-DevKitC-1 v1.2

The ESP32-C5-DevKitC-1 v1.2 is a current Espressif development board built
around the ESP32-C5-WROOM-1 module. The N8R8 variant carries 8 MB flash and
8 MB PSRAM. ESP32 is demand rank **#1** in the 2025 JetBrains MCU-use survey
(38%). This is a rank-1 ESP32-family coverage extension on an already-covered
board, not a new MCU or board demand pick. It adds the modeled RMT TX RGB LED
path.

**Live availability rechecked 2026-11-10:** [Espressif's v1.2 board guide](https://docs.espressif.com/projects/esp-dev-kits/en/latest/esp32c5/esp32-c5-devkitc-1/user_guide.html)
lists the N8R8 variant and retail/wholesale order paths. DigiKey's [exact
ESP32-C5-DEVKITC-1-N8R8 listing](https://www.digikey.com/en/products/detail/espressif-systems/ESP32-C5-DEVKITC-1-N8R8/28718162)
reported **Part Status: Active**, **312 in stock**, and **US$15.00** for one
unit. Inventory changes over time. The older v1.1 guide revision
is outside this profile. Adafruit offered the [BME280 breakout PID 2652](https://www.adafruit.com/product/2652)
for purchase; DigiKey listed its [matching PID 2652 listing](https://www.digikey.com/en/products/detail/adafruit-industries-llc/2652/5604372)
as **Active** with **1,990 in stock**.

## Profile and smoke

The profile models the HP RV32IMAC core, flash/SRAM/ROM address windows, a
narrow PCR slice, UART0's register/FIFO behavior, I2C0, and the C5 GPIO matrix
and IO_MUX paths for GPIO6, GPIO10 and GPIO27, plus RMT TX0 timed output. The
committed bare-metal firmware selects PLL_F80M for UART0 and RMT, sends
`ESP32_C5_UART0_OK` through UART0 (GPIO11/U0TXD on J3 pin 2), reads the BME280
ID and temperature ADC over I2C0, and sends one red GRB WS2812 frame through
RMT TX0 to the onboard RGB LED on GPIO27. The smoke and evidence are in
[`examples/esp32c5-devkitc/`](../../examples/esp32c5-devkitc/).

| Aspect | Profile |
|---|---|
| Chip descriptor | [`configs/chips/esp32c5.yaml`](../../configs/chips/esp32c5.yaml) |
| Board system | [`configs/systems/esp32c5-devkitc.yaml`](../../configs/systems/esp32c5-devkitc.yaml) |
| Smoke firmware | [`crates/firmware-esp32c5-devkitc-smoke/`](../../crates/firmware-esp32c5-devkitc-smoke/) |
| Board pins used | UART0 TX GPIO11 / J3-2; UART0 RX GPIO12 / J3-3; BME280 SDA GPIO6 / J1-7 and SCL GPIO10 / J1-11; RGB LED GPIO27 |
| Evidence | [`examples/esp32c5-devkitc/evidence/`](../../examples/esp32c5-devkitc/evidence/) |
| Modeling level | Local deterministic UART/I2C/RMT/WS2812 smoke; no silicon capture or ROM boot claim |

The BME280 smoke requires the configured C5 I2C0 matrix route and checks
register values from the sensor model. It does not model I2C analog behavior
or sensor measurements from physical silicon. GPIO behavior outside the listed
pads is not included. RMT receive, carrier, DMA and fractional clock division,
the USB-UART bridge's electrical pad routing, PSRAM, radios, native USB, LP
core and dynamic clock tree are also outside the profile. See
[known limitations](../../examples/esp32c5-devkitc/KNOWN_LIMITATIONS.md).

The checked-in fixture checks UART output, BME280 registers, RMT completion,
GPIO27 edges and the red LED frame. The external `labwired_verify` service does
not currently list ESP32-C5 as a supported target, so this is not a hosted
`model_verified` result or a physical-board test.

## Run the example

```bash
rustup target add riscv32imc-unknown-none-elf
cargo build -p firmware-esp32c5-devkitc-smoke --release --target riscv32imc-unknown-none-elf
cargo run -q -p labwired-cli -- test \
  --script examples/esp32c5-devkitc/onboarding-smoke.yaml \
  --output-dir out/esp32c5-devkitc \
  --no-uart-stdout
```
