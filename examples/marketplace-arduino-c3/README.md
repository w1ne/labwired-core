# Marketplace Arduino (ESP32-C3 Super Mini)

Arduino `Wire` sketch that polls marketplace I²C kits on **SDA=GPIO4 / SCL=GPIO5**.

## Build

```bash
cd platformio/marketplace-arduino-c3
pio run
```

## Flash image for `--rom-boot`

```bash
BUILD=.pio/build/marketplace
python3 - <<'PY'
from pathlib import Path
build = Path(".pio/build/marketplace")
flash = bytearray(b"\xff" * (4 * 1024 * 1024))
for path, off in [("bootloader.bin", 0), ("partitions.bin", 0x8000), ("firmware.bin", 0x10000)]:
    data = (build / path).read_bytes()
    flash[off : off + len(data)] = data
Path("/tmp/marketplace-c3-flash.bin").write_bytes(flash)
print("wrote", len(flash))
PY
```

## Run (needs C3 ROM bins)

```bash
export LABWIRED_ESP32C3_ROM=/path/to/c3_rom.bin
export LABWIRED_ESP32C3_ROM_DATA=/path/to/c3_rom_data.bin
export LABWIRED_ESP32C3_FLASH=/tmp/marketplace-c3-flash.bin

cd ../..  # core/
cargo run -q -p labwired-cli -- test \
  --script examples/marketplace-arduino-c3/stimuli-smoke.yaml \
  --rom-boot \
  --capture-app-entry /tmp/mkt-c3-app.lwrs \
  --output-dir /tmp/mkt-c3
```

Use a high `max_steps` (e.g. 200_000_000+) so the Arduino app starts after ROM + second-stage boot.

### Serial note (Super Mini)

Board flag `ARDUINO_USB_CDC_ON_BOOT=1` maps `Serial` to USB-CDC. LabWired captures **UART0**, so the sketch logs on `Serial0` (and still `begin`s USB Serial with a zero TX timeout so USB does not block in sim).

### Resume snapshot (skip ~ROM replay)

After a cold run writes `/tmp/mkt-c3-app.lwrs`, later runs can:

```bash
cargo run -q -p labwired-cli -- test \
  --script examples/marketplace-arduino-c3/stimuli-smoke.yaml \
  --rom-boot \
  --resume-snapshot /tmp/mkt-c3-app.lwrs \
  --output-dir /tmp/mkt-c3-resume
```

Self-key mismatch (new flash/ELF) errors out — fall back to cold boot + re-capture.

### Proven UART / stimuli (2026-07-23)

Cold rom-boot 200M steps (`result.json` status **pass**):

| Phase | INA219 | ADS1115 | AS5600 | BNO055 | VL53L0X |
|-------|--------|---------|--------|--------|---------|
| Default | 3300 mV / 0 mA | A0_raw=0 | 0° | heading=0 | 200 mm |
| After stimuli @ 170M cycles | 12000 mV / 1500 mA | 16384 | 90° | 45 | 777 mm |

Also emits `MARKETPLACE ARDUINO C3`, `SENSORS READY`, `DS3231 TIME=12:00:00`.
