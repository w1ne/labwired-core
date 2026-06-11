# ESP32-C3 WiFi/BT radio register-map RE harness

Reverse-engineers the undocumented C3 radio register surface by tracing the real
IDF WiFi driver on live silicon. See `docs/esp32c3_radio_reverse_engineering.md`.

- `main/wifi_probe.c` — minimal `esp_wifi_init`→`set_mode`→`esp_wifi_start` probe
  with breakpoint anchors bracketing each bring-up phase.
- `trace_radio.sh <elf> <out_dir>` — flashes, sets HW breakpoints on the anchors
  over USB-JTAG (openocd-esp32), dumps the candidate radio windows per phase.

Build: `idf.py set-target esp32c3 build` (ESP-IDF v5.3.1).
