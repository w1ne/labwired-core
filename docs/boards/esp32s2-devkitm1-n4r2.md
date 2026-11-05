# ESP32-S2-DevKitM-1-N4R2

**Demand rank:** ESP32 family, rank 1 in the 2025 JetBrains State of Embedded
survey (38% usage).

The board is listed by Espressif with the recommended ESP32-S2-MINI-2 module,
4 MiB flash and 2 MiB PSRAM. The live board guide marks older MINI-1 variants
EOL; this profile targets only the N4R2 board. See
[`examples/esp32s2-devkitm1-n4r2/evidence/vendor-status.md`](../../examples/esp32s2-devkitm1-n4r2/evidence/vendor-status.md).

## Working slice

The current model boots a freestanding single-core Xtensa LX7 ELF, captures a
UART0 marker, and drives the onboard WS2812 RGB LED on GPIO18. The committed
smoke checks serial output, GPIO edges, and a decoded red pixel.

ROM boot, flash MMU behavior, Wi-Fi, USB OTG, PSRAM, buttons, analog behavior,
the external USB-to-UART bridge, and other peripherals are outside this slice.
The profile's local smoke is not hosted LabWired verification or a physical
hardware result.

Run it from the repository root after installing Espressif's
`xtensa-esp32s2-elf-gcc`:

```bash
scripts/build-esp32s2-devkitm1-smoke.sh
cargo run -q -p labwired-cli -- test \
  --script examples/esp32s2-devkitm1-n4r2/onboarding-smoke.yaml \
  --output-dir out/esp32s2-devkitm1-n4r2 \
  --no-uart-stdout
```
