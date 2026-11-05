# ESP32-C5-DevKitC-1 v1.2 profile limits

This is an L1 UART0 smoke profile. Its passing transcript proves that the
simulated HP RISC-V core executed the committed firmware and that the UART0 TX
FIFO emitted the expected marker. It is not a ROM boot, ESP-IDF boot, hosted
model-verification, or physical-board result.

- Modeled: HP RISC-V core, flash/SRAM/ROM address windows, UART0 register head,
  TX FIFO, UART0 host capture, and the UART0 PCR source/enable registers. The
  firmware selects PLL_F80M and programs CLKDIV=694 for approximately 115,200
  baud.
- Not modeled: mask-ROM reset/boot sequence, dynamic clock tree, PLL
  generation, UART0 APB-gate enforcement, LP core, PSRAM, GPIO/IO_MUX pads,
  USB/UART bridge electrical propagation, the
  onboard RGB LED and its C5-specific GPIO/RMT path, Wi-Fi, Bluetooth, Thread,
  Zigbee, and other peripherals.
- Board scope: ESP32-C5-DevKitC-1 v1.2 with the N8R8 module. The older v1.1
  board revision is outside this profile.

All register and timing behavior is SIM-DERIVED; no silicon capture is claimed.
