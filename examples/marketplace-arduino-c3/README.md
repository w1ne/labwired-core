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
  --rom-boot --output-dir /tmp/mkt-c3
```

Use a high `max_steps` (e.g. 200_000_000+) so the Arduino app starts after ROM + second-stage boot.
