# ESP32-C5-DevKitC-1 v1.2 profile limits

This is a focused UART0, I2C0/BME280 and GPIO27/RMT TX smoke profile. The local
runner checks the UART0 TX marker, BME280 chip ID and raw temperature ADC
sample, RMT TX completion, GPIO27 edge count, and decoded red RGB LED frame. It
is not a ROM boot, ESP-IDF boot, hosted `model_verified`, or physical-board
result.

- Modeled: HP RISC-V core, flash/SRAM/ROM address windows, UART0 register head,
  TX FIFO, UART0 host capture, UART0 PCR source/enable registers,
  GPIO_FUNC27_OUT_SEL_CFG, and IO_MUX function select. RMT models TX0 timed
  output, channel configuration/status, interrupt state and shared symbol RAM
  through the GPIO matrix. The C5 profile fixes its source at PLL_F80M and
  does not model RX sampling, carrier modulation, DMA or fractional clock
  division. I2C0 uses the C5 controller register map and PCR gate with a
  C5-specific GPIO6/GPIO10 matrix route. Its attached BME280 responds to
  register reads. A generic NeoPixel component decodes the firmware's GPIO27
  WS2812 frame. The firmware selects PLL_F80M and programs CLKDIV=694 for
  approximately 115,200 baud.
- Not modeled: mask-ROM reset/boot sequence, dynamic clock tree, PLL
  generation, I2C analog/electrical behavior, general-purpose GPIO behavior on
  unlisted pads, UART0 APB-gate enforcement, LP core, PSRAM, RMT RX/carrier/DMA,
  USB/UART bridge electrical propagation, Wi-Fi, Bluetooth, Thread, Zigbee,
  and other peripherals.
- Board scope: ESP32-C5-DevKitC-1 v1.2 with the N8R8 module. The older v1.1
  board revision is outside this profile.

All register and timing behavior is SIM-DERIVED; no silicon capture is claimed.
The external LabWired verifier does not list ESP32-C5 as a supported target.
