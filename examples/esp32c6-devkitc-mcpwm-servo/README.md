# ESP32-C6-DevKitC-1 MCPWM servo

**Demand rank:** ESP32 is rank 1 in the measured MCU survey; ESP32 kits are
rank 2 among project kits.

This board profile drives an external SG90 hobby servo from MCPWM0 PWM0A on
header J1 GPIO4. Firmware configures a 50 Hz up-count waveform with a 1.5 ms
high pulse. The smoke requires the UART marker, the programmed compare value,
and GPIO4 edges from the routed MCPWM output.

The system also describes the DevKitC-1's onboard GPIO8 RGB LED and GPIO9 BOOT
button. This smoke does not exercise either device. It does not claim that the
board's Wi-Fi, Bluetooth, Thread/Zigbee radios, USB, or electrical power path
are simulated.

Run from the repository root:

```sh
scripts/onboarding_smoke.sh \
  --target-id esp32c6-devkitc-mcpwm-servo \
  --crate firmware-esp32c6-demo \
  --target riscv32imc-unknown-none-elf \
  --script examples/esp32c6-devkitc-mcpwm-servo/onboarding-smoke.yaml \
  --system configs/systems/esp32c6-devkitc-mcpwm-servo.yaml \
  --out-dir examples/esp32c6-devkitc-mcpwm-servo/evidence
```

The build and simulation logs are committed under `evidence/logs/` with the
structured result under `evidence/simulation-output/`.
