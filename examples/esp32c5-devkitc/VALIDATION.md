# ESP32-C5-DevKitC-1 validation runbook

Run from the repository root.

```bash
rustup target add riscv32imc-unknown-none-elf
cargo build -p firmware-esp32c5-devkitc-smoke --release --target riscv32imc-unknown-none-elf
cargo run -q -p labwired-cli -- test \
  --script examples/esp32c5-devkitc/onboarding-smoke.yaml \
  --output-dir out/esp32c5-devkitc \
  --no-uart-stdout
```

Pass criteria: exit code 0, UART output contains `ESP32_C5_UART0_OK`, and the
run stops at the configured step limit. The smoke image links into the C5's
unified flash window at `0x4200_0000` and uses the 384 KiB HP SRAM window at
`0x4080_0000`.
