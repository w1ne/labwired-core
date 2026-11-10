# Validation record

## Scope

Demand rank: **ESP32 #1 (38%)**; Arduino kits: **37%**. This is new system
coverage for the Arduino Nano ESP32 on the already-modelled ESP32-S3. The
acceptance behavior is a BME280 ID and raw-temperature read over I2C0.

## Reproduce

```sh
examples/arduino-nano-esp32-bme280/firmware/build-firmware.sh
cargo run -q -p labwired-cli -- test \
  --script examples/arduino-nano-esp32-bme280/bme280-smoke.yaml \
  --output-dir out/arduino-nano-esp32-bme280 \
  --no-uart-stdout
```

## Result

The committed evidence is under `evidence/`. The smoke requires BME280 ID
`0x60`, raw temperature `0x082DC0`, and the firmware's idle-loop stop. This is
digital-twin evidence. It does not confirm the physical board, the electrical
I2C path, or silicon parity.

The hosted LabWired oracle gate also returned `status: model_verified` for the
same serial marker; the result is saved in
[`evidence/model-verification.json`](evidence/model-verification.json). The
hosted Nano target resolves to the generic `esp32-s3-devkitc-1` MCU component,
with GPIO11/12 explicitly wired to the modeled BME280. This verifies the S3
firmware-to-sensor behavior. It does not independently verify the Nano header
pad mapping. Its model also assumes an ideal MCU VDD rail, so power-on and
brown-out behavior are not simulated.

The selected route is A4/GPIO11 for SDA and A5/GPIO12 for SCL; the firmware
sets both pins explicitly. The local smoke builds with the Nano board's flash
and clock definition but the generic ESP32-S3 Arduino variant. This avoids the
Nano variant's pre-`setup()` recovery/RGB initializer, which currently aborts
in the local model. The simulation proves the BME280 transaction on I2C0; it
does not prove the Nano variant initializer, OPI PSRAM, USB DFU, native USB
electrical behavior, board-level pad routing, or silicon parity.
