# Validation: ESP32-C61-DevKitC-1-N8R2 UART0

The committed onboarding smoke builds the `firmware-esp32c61-smoke` bare-metal
RV32IMAC image and runs it on the repository engine. It asserts that UART0
captures `ESP32C61_DEVKITC1_UART0_OK` and that execution reaches the bounded
`max_steps` stop condition.

Evidence is in [`evidence/`](evidence/) after the onboarding smoke is run. The
descriptor contains only the UART0 register model and the PCR clock-gate model,
so unsupported peripheral addresses fault instead of returning fabricated
zero values.

This is a local deterministic engine smoke. It is not a hardware run or a
silicon-correlation result. The current smoke does not exercise pad routing,
the board USB-UART bridge electrically, PSRAM, Wi-Fi, Bluetooth, native USB,
or the RGB LED.
