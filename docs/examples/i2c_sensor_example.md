# Example: I²C temperature sensor (TMP102)

Run firmware that talks to a **TMP102** over I²C on the twin. No special LabWired HAL in the firmware — normal bus reads.

---

## What you get

| Piece | Location |
|-------|----------|
| Device model | [`configs/devices/tmp102.yaml`](../../configs/devices/tmp102.yaml) |
| Type id | `tmp102` |
| Default I²C address | `0x48` |
| Demo + e2e | [`examples/esp32s3-i2c-tmp102/`](../../examples/esp32s3-i2c-tmp102/) |

The model’s temperature register can **self-drift** (+0.5 °C per full read, wrap 35 °C → 20 °C) so demos exercise thresholds without external stimulus. For a sensor driven by a **temperature** input channel (noise + lag), see [`mcp9808.yaml`](../../configs/devices/mcp9808.yaml).

---

## 1. Attach the device

Use a real system from the repo and add (or keep) the TMP102 on the I²C bus your firmware uses. Pattern:

```yaml
external_devices:
  - id: "tmp102"
    type: "tmp102"
    connection: "i2c0"    # match the bus id in that system / chip
    config:
      i2c_address: 0x48
```

Copy field names from a working system under `configs/systems/` or `examples/*/system.yaml` — schemas differ slightly by board family.

---

## 2. Firmware

Firmware should:

1. Init I²C on the pins wired in the system  
2. Write pointer `0x00`, read 2 bytes (12-bit left-justified temp, big-endian)  
3. Print `T = …` on UART (or USB-serial)  

The ESP32-S3 demo does this once per second and toggles a GPIO above 30 °C. See [`examples/esp32s3-i2c-tmp102/`](../../examples/esp32s3-i2c-tmp102/).

---

## 3. Run / test

From the core repo (with ESP32-S3 fixtures when using the e2e test):

```bash
# End-to-end test (builds + asserts UART and GPIO)
cargo test -p labwired-core --features esp32s3-fixtures \
  --release --test e2e_i2c_tmp102
```

Or run your own ELF:

```bash
labwired run --firmware path/to/firmware.elf --system path/to/system.yaml
labwired test --script path/to/smoke.yaml
```

**Expected (demo):** UART lines like `T = 25.00 C`, then rising temperatures as the model drifts.

---

## 4. Agent path

```text
labwired_describe id=tmp102
# wire diagram on a board that has I2C
labwired_run → labwired_verify (serial contains "T =" or register checks)
```

---

## Next

| | |
|--|--|
| Add your own sensor | [Onboard a part](../howto/onboard-part.md) |
| MCP9808 (SimInput temperature) | [`configs/devices/mcp9808.yaml`](../../configs/devices/mcp9808.yaml) |
| SPI example direction | [ADXL345 SPI device](../../configs/devices/adxl345_spi.yaml) · [Onboard a part](../howto/onboard-part.md) |
