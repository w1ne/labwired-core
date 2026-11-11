# ESP32-H2-DevKitM-1-N4S UART smoke

ESP32 family demand rank: **#1 (38% MCU usage in the 2025 survey)**. This
target adds the H2 Thread/Zigbee-capable member to the existing ESP32 family
coverage. Espressif's current board order code is **ESP32-H2-DevKitM-1-N4S**;
the earlier N4 order code is marked NRND.

This profile boots a bare RISC-V ELF into the 4 MiB flash window and checks the
H2 UART0 FIFO/host-console path. UART0 is the only behavior exercised here.
The USB-to-UART bridge and IO_MUX pad route are not simulated, so the smoke
does not claim a physical USB serial connection.

## Build and run

```sh
rustup target add riscv32imc-unknown-none-elf
examples/esp32h2-devkitm-1-n4s/ci/build.sh
examples/esp32h2-devkitm-1-n4s/ci/test.sh
```

The test expects `H2_N4S_UART_OK` on the UART stream. The captured run log is
[`evidence/logs/run_smoke.log`](evidence/logs/run_smoke.log).

## Scope

- Modeled for this profile: RISC-V fast-boot ELF loading, UART0 FIFO/console,
  UART0 clock gating, and the common GPIO data/enable register head.
- Not modeled: mask-ROM/bootloader startup, USB-to-UART bridge, IO_MUX pad
  electrical routing, the WS2812 RGB LED protocol, button/strap behavior,
  GPIO interrupts, Bluetooth LE, IEEE 802.15.4, Thread, Zigbee, USB, and the
  remaining H2 peripherals.
- This is simulator evidence, not a live-silicon differential.

Vendor availability evidence: [`evidence/vendor-status.md`](evidence/vendor-status.md).
