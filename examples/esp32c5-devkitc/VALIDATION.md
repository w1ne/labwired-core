# ESP32-C5-DevKitC-1 validation runbook

Run from the repository root.

```bash
rustup target add riscv32imc-unknown-none-elf
CARGO_BUILD_RUSTC_WRAPPER= scripts/onboarding_smoke.sh \
  --target-id esp32c5-devkitc \
  --crate firmware-esp32c5-devkitc-smoke \
  --target riscv32imc-unknown-none-elf \
  --script examples/esp32c5-devkitc/onboarding-smoke.yaml \
  --system configs/systems/esp32c5-devkitc.yaml \
  --out-dir examples/esp32c5-devkitc/evidence
```

Pass criteria: the structured result is `pass`, UART output contains
`ESP32_C5_UART0_OK`, `BME280_ID=0x60` and `BME280_TEMP_RAW=0x082DC0`, GPIO27
produces at least 48 edges, the RGB LED decodes a red pixel (`0xF800`), and the
run stops at the configured step limit. The onboarding script checks
`result.json` so failed assertions cannot leave the metrics marked as passing.
The smoke image links into the C5's unified flash window at `0x4200_0000` and
uses the 384 KiB HP SRAM window at `0x4080_0000`. This is local simulator
evidence; the hosted LabWired verifier currently has no ESP32-C5 target.
