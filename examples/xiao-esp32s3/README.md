# Seeed Studio XIAO ESP32S3 — ESP32-S3 on the existing S3 model

The XIAO ESP32S3 carries an ESP32-S3R8 (dual-core Xtensa LX7, 240 MHz, 8 MB
in-package octal PSRAM) with 8 MB flash. It uses the same die as the
ESP32-S3-DevKitC-1 row. The board differs from the DevKitC in ways that stock
Arduino code sees:

- `LED_BUILTIN` is GPIO21, a plain active-low LED. The DevKitC drives an RGB
  LED through RMT.
- The board JSON builds with `ARDUINO_USB_CDC_ON_BOOT=1`, so `Serial` is the
  USB-Serial-JTAG (HWCDC). The matrix sketches also mirror every line to
  `Serial0` (UART0).
- Wire is on D4/D5 = GPIO5/GPIO6. SPI is on D8/D9/D10 = GPIO7/8/9, with
  `SS` = GPIO44.
- `A0` is GPIO1 = ADC1_CH0. The variant declares it as `static const uint8_t`,
  so `defined(A0)` is false in a sketch.

`configs/systems/seeed-xiao-esp32s3.yaml` claims the user LED and the BOOT
button only. PSRAM, Wi-Fi/BLE, the camera/SD expansion board, the battery
charger and power are not claimed. No physical hardware comparison has been
made.

## Reproduce

From the labwired-core repository root, with PlatformIO `espressif32`:

```sh
python3 validation/arduino-matrix/run_matrix.py --boards xiao_esp32s3
```

Expected: 9 pass / 0 skip / 0 fail. See `evidence/arduino-matrix/` and
`evidence/vendor-status.md`.
