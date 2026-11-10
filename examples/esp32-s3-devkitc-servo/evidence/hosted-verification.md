# Hosted MCPWM oracle

`labwired_verify` returned `status: model_verified` for the register-level
Arduino sketch in `../firmware/mcpwm_hosted_verify.ino` on
`esp32-s3-devkitc-1-n16r8`.

Oracle clauses passed:

- UART contains `ESP32_S3_DEVKITC1_V10_MCPWM_SERVO_OK`.
- GPIO4 toggled.

The hosted run used a 9,000,000-step budget and stopped at the limit after the
proof clauses held. The verifier reported one non-blocking approximation: it
assumed ideal VDD because the board had no explicit routed power net. This is
model verification, not physical-board evidence. The local bare-metal board
smoke and its complete logs are recorded alongside this file.
