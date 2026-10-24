# PyBadge native qualification

This is an incremental native-board implementation, not a claim that native
MakeCode Arcade already runs or reaches real time. BrickwrightLite's usable
PXT Arcade experience is a separate backend, not a CPU RTx measurement.

## Implemented and qualified

Core PR151 added the eight GPIO-clocked SN74HC165 buttons and was merged at
`f6d0cf8223de56d28dad09d57594f6febc6ab320` after all enabled exact-head checks
passed (Rust Core CI run 37240999321). Qualification includes all 256 button
combinations, active-low/MSB-first scan, transparent latch, frozen snapshot,
rising-only clock, serial zero fill, guest read-only input, and finite input
validation. No physical capture or hardware waiver changed.

## Blocking SPI slice (verification pending)

The PyBadge chip descriptor adds `sam_sercom_spi` at SERCOM4 `0x43000000`.
Live clock gates are MCLK APBDMASK bit 0 and GCLK channel 34. PB13/PAD1 SCK and
PB15/PAD3 MOSI require PORT mux C and DOPO=2. Byte-level SPI devices attach
through the standard trace, CS, D/C, inspection and input interfaces.

The authored MMIO tests exercise the actual board descriptor, a transport-only
slave observer, blocking byte/halfword/word transfers, FIFO consumption,
W1C flags, reset cancellation, wrong mode/mux, clock-off writes and a paused
in-flight transfer. An independently authored Thumb guest also configures MMIO,
polls TXC and writes an SRAM completion marker; with the wrong mux it must
remain blocked. Neither test proves display pixels or production native firmware.

Limits: no edge-sampling slave support (attachment is rejected), no split-vector
IRQ delivery, no DMA, no display attached, and no dynamically derived kernel
clock rate. BAUD delays use nominal core-clock cycles; they must not support an
RTx or wire-timing claim until CPU/kernel clock scaling is implemented and
qualified. The SERCOM4 interrupt vectors must not be collapsed onto one IRQ.

## Remaining order

1. Pass full exact-head controller verification; add authored guest-firmware
   polling proof and independent negative cases.
2. Add the ST7735 panel descriptor. Physical CS is PB7, D/C PB5, reset PA0,
   backlight PA1. Verify actual glass/RAM dimensions, rotation, address window,
   pixel format, reset and power/backlight behavior through guest MMIO. Do not
   relabel the existing ST7789 model as a qualified ST7735.
3. Add SAM DMAC descriptor/trigger/writeback/completion and interrupt behavior.
   Qualify disabled clocks, invalid descriptors/triggers, abort/reset and
   masked interrupts without bypassing the guest driver.
4. Run the pinned native firmware, qualify debugger/UI output and input, then
   measure actual frame rate/input latency and native RTx. Keep USB, audio and
   QSPI gaps explicit; do not mark full-board checkpoints complete early.

## Pinned runtime dependency

The hosted `samd51adafruit` build manifest from Lite run 36567239929 records
request `19efcdc73769fdfdeb51aa215c528bebad59782cbc538f72f4a194326f1f42b1`
and HEX SHA256 `9c2310bd5a65f0543c69a076e51a4067c803202a39228f3a7de41451be0da9ca`.
Its CODAL SAMD pin is `5bd6b93c219c7e784e885ba2d6812809fb6289a8`.
At that pin, even a one-byte SPI write uses DMA, waits for TXC and wakes the
fiber/event path. Thus a working blocking controller alone cannot qualify the
production firmware. DMAC RX/TX triggers for SERCOM4 are 12/13. DMAC channels
0..3 use IRQs 31..34; channels 4..31 share IRQ 35.

Primary sources:

- [Pinned CODAL SPI driver](https://github.com/lancaster-university/codal-samd/blob/5bd6b93c219c7e784e885ba2d6812809fb6289a8/src/ZSPI.cpp)
- [Pinned DMA descriptors](https://github.com/lancaster-university/codal-samd/blob/5bd6b93c219c7e784e885ba2d6812809fb6289a8/src/DmaInstance.cpp)
- [Pinned DMA initialization and IRQ dispatch](https://github.com/lancaster-university/codal-samd/blob/5bd6b93c219c7e784e885ba2d6812809fb6289a8/src/DmaFactory.cpp)
- [Pinned PAD/DOPO mapping](https://github.com/lancaster-university/samd-peripherals/blob/96563308fc7b97646cbe429953e79cb3405846f0/samd/samd51/sercom.c)
- [SERCOM4 register, GCLK and DMA map](https://github.com/arduino/ArduinoModule-CMSIS-Atmel/blob/46ab1021146152a64caf1ddbb837d8181b8faa35/CMSIS-Atmel/CMSIS/Device/ATMEL/samd51/include/instance/sercom4.h)

All engine/build/browser qualification runs on hosted CI. Evidence goes to
`/mnt/storage`, source work to `/mnt/volume1`; no large artifacts go to `/tmp`.
