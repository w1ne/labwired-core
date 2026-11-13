# ESP32-H2-DevKitM-1-N4S

Espressif's ESP32-H2 single-core RISC-V module board, with 4 MiB flash. The
current N4S variant is active and purchasable; the older N4 order code is
marked NRND. ESP32 family demand rank: **#1** in the 2025 MCU-usage survey.

## Status at a glance

| Aspect | Status |
|---|---|
| Chip descriptor | [`configs/chips/esp32h2.yaml`](../../configs/chips/esp32h2.yaml) |
| Board system | [`configs/systems/esp32h2-devkitm-1-n4s.yaml`](../../configs/systems/esp32h2-devkitm-1-n4s.yaml) |
| Firmware example | [`examples/esp32h2-devkitm-1-n4s/`](../../examples/esp32h2-devkitm-1-n4s/) |
| Support level | L1 UART smoke, simulator-derived |
| Verified behavior | UART0 FIFO output reaches the LabWired host-console capture |
| Vendor availability | [Dated N4S evidence](../../examples/esp32h2-devkitm-1-n4s/evidence/vendor-status.md) |

## Pins

| Board signal | GPIO | Profile status |
|---|---:|---|
| UART0 TX | GPIO24 | Chip pin map recorded; electrical IO_MUX route is not simulated |
| UART0 RX | GPIO23 | Chip pin map recorded; receive path is not exercised |
| Addressable RGB LED | GPIO8 | Pin recorded; WS2812/RMT protocol is not modeled |
| BOOT button | GPIO9 | Pin recorded; button and strap behavior are not modeled |

## Supported slice and limits

The smoke loads a bare ELF into the 4 MiB flash address window and checks the
UART0 FIFO/console behavior. It is a fast-boot simulation; it does not execute
the mask ROM, second-stage bootloader, or physical USB-to-UART bridge. PCR is
limited to the UART0 clock-control registers. The GPIO mapping exposes only the
common data/enable register head, with no matrix routing, GPIO interrupt, or pad
electrical claim.

Bluetooth LE, IEEE 802.15.4, Thread, Zigbee, USB, the RGB LED protocol, and the
rest of the H2 peripheral map are outside this profile. No live-silicon
differential has been run.

## Run

From the repository root:

```sh
rustup target add riscv32imc-unknown-none-elf
examples/esp32h2-devkitm-1-n4s/ci/build.sh
examples/esp32h2-devkitm-1-n4s/ci/test.sh
```

See the [validation runbook](../../examples/esp32h2-devkitm-1-n4s/VALIDATION.md)
for the exact smoke assertion and evidence path.
