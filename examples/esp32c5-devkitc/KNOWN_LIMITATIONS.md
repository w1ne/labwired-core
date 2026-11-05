# ESP32-C5-DevKitC-1 v1.2 profile limits

This is a focused UART0 and GPIO27 smoke profile. The local runner checks the
UART0 TX marker, GPIO27 edge count, and decoded red RGB LED frame. It is not a
ROM boot, ESP-IDF boot, hosted `model_verified`, or physical-board result.

- Modeled: HP RISC-V core, flash/SRAM/ROM address windows, UART0 register head,
  TX FIFO, UART0 host capture, UART0 PCR source/enable registers, and GPIO27's
  direct output, GPIO_FUNC27_OUT_SEL_CFG, and IO_MUX function select. A generic
  NeoPixel component decodes the firmware's GPIO27 WS2812 frame. The firmware
  selects PLL_F80M and programs CLKDIV=694 for approximately 115,200 baud.
- Not modeled: mask-ROM reset/boot sequence, dynamic clock tree, PLL
  generation, UART0 APB-gate enforcement, LP core, PSRAM, GPIO pads other than
  GPIO27, RMT, USB/UART bridge electrical propagation, Wi-Fi, Bluetooth,
  Thread, Zigbee, and other peripherals.
- Board scope: ESP32-C5-DevKitC-1 v1.2 with the N8R8 module. The older v1.1
  board revision is outside this profile.

All register and timing behavior is SIM-DERIVED; no silicon capture is claimed.
The external LabWired verifier does not list ESP32-C5 as a supported target.
