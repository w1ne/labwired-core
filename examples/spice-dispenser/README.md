# SpiceDispenser on LabWired (ESP32-S3, faithful rom-boot)

Runs the **unmodified** firmware of the [SpiceDispenser](https://github.com/w1ne/SpiceDispenser)
(a voice/BLE-driven smart spice dispenser: an ESP32-S3 rotates a revolver to a
compartment and pulses a shutter servo through a PCA9685 I²C PWM board) on
LabWired's faithful ESP32-S3 chip model — booting the **real Espressif boot
ROM** from the reset vector, exactly like silicon.

This example is also a **hardware-validated** case study: the same firmware was
run on a physical ESP32-S3 (rev v0.2) over USB-Serial-JTAG, and the model's
boot is **bit-identical** to silicon through the 2nd-stage bootloader hand-off
(see [VALIDATION.md](./VALIDATION.md)).

## What runs, faithfully

`labwired run --rom-boot` boots the genuine ESP32-S3 ROM from `0x40000400`, the
real 2nd-stage bootloader loads the app from the modeled SPI-flash controller,
and execution enters the unmodified application image:

```
ESP-ROM:esp32s3-20210327
rst:0xc (RTC_SW_CPU_RST),boot:0x8 (SPI_FAST_FLASHE_BOOT)
load:0x3fce2820,len:0x10cc
load:0x403c8700,len:0xc2c
load:0x403cb700,len:0x30c0
entry 0x403c88b8
```

No fast-boot, no FreeRTOS shortcuts, no firmware-symbol hooks — the chip's own
ROM and bootloader run the real flash image.

## Files

- [`system.yaml`](./system.yaml) — the board (esp32s3-zero chip + dispenser wiring notes).
- [`RUNBOOK.md`](./RUNBOOK.md) — exact reproduction: building the real ROM/DROM
  images, the `--rom-boot` invocation, and flashing/monitoring the physical board.
- [`VALIDATION.md`](./VALIDATION.md) — model-vs-silicon comparison and the current
  boot frontier.

## Inputs you must supply (not vendored)

The Espressif boot ROM is copyright Espressif and is **not** committed. Generate
the flat ROM/DROM images from the copy shipped with the ESP toolchain, and point
LabWired at them plus the firmware flash image — see RUNBOOK.md.
