# Validation — Seeed Studio XIAO ESP32-C3 + BME280

On 2026-11-06 the onboarding smoke built the shared ESP32-C3 register-level
firmware with the `xiao` feature, which selects GPIO6/GPIO7 for the board's
standard D4/D5 I2C route. The local simulator passed all 3 assertions after
1,000,000 instructions: BME280 ID `0x60`, raw temperature bytes `0x082DC0`, and
the expected max-step stop.

Latest committed results are in `evidence/`: build and run logs, structured
smoke output, onboarding metrics, and the generated target scoreboard. This is
simulator evidence only; it is not a physical-board test or a hosted
`labwired_verify` result.
