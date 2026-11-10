# Hosted PSRAM verification

On 2026-11-10, hosted `labwired_verify` ran the Arduino example on board
`esp32-s3-devkitc-1-n16r8` and returned:

- `status`: `model_verified`
- `run_status`: `pass`
- Oracle: serial contains `ESP32_S3_N16R8_PSRAM_PASS bytes=4096`
- `stop_reason`: `assertions_passed`

The sketch found PSRAM, allocated 4 KiB with
`heap_caps_malloc(..., MALLOC_CAP_SPIRAM | MALLOC_CAP_8BIT)`, wrote and read
back 1,024 words, and emitted the acceptance marker. The verifier reported one
approximation: the MCU had no routed power net, so VDD was assumed ideal and
power-on/brown-out reset was not simulated. This is a model result, not a
physical-board test.

The local repository smoke is separate: it directly programs the S3 MMU table,
probes the first, middle, and final words of the 8 MiB window, and records its
build and simulator logs in this directory. It does not claim to test the OPI
initialization sequence or memory timings.
