# Validation: ESP32-C61-DevKitC-1-N8R2 UART0 + GPIO4

The committed onboarding smoke builds the `firmware-esp32c61-smoke` bare-metal
RV32IMC image and runs it on the repository engine. It asserts the UART0
success marker, at least eight transitions on modeled `gpio:4`, and the
bounded `max_steps` stop condition.

GPIO4 is exposed at J1 pin 3 on the board guide's v2.0 pin map. The smoke writes
`GPIO_FUNC4_OUT_SEL_CFG` with C61 `SIG_GPIO_OUT_IDX=256`, enables output through
GPIO+`0x38`, then toggles GPIO4 through `GPIO_OUT_W1TS/W1TC` at +`0x08`/+`0x0C`.
The register model translates C61's shifted GPIO offsets into the shared direct
GPIO behavior.

Evidence is committed in [`evidence/`](evidence/). This is local simulator
evidence, not a hardware run or silicon-correlation result. The output edge
assertion proves the modeled GPIO latch and enable path; IO_MUX pad selection
and voltage at J1 pin 3 are not modeled. A separate [BME280 smoke](../esp32c61-devkitc-1-n8r2-bme280/VALIDATION.md)
exercises only the C61 I2C0 matrix route. The descriptor does not claim GPIO
input, GPIO interrupts, other matrix signals, GPIO26..29, Wi-Fi, Bluetooth,
native USB, PSRAM, or the RGB LED. Other GPIO registers outside the direct
output and I2C0 matrix subsets have no behavioral model.
