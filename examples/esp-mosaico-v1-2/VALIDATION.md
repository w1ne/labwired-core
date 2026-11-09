# ESP-Mosaico V1.2 validation

Run from the repository root:

```bash
rustup target add riscv32imac-unknown-none-elf
CARGO_BUILD_RUSTC_WRAPPER= scripts/onboarding_smoke.sh \
  --target-id esp-mosaico-v1-2 \
  --crate firmware-esp-mosaico-v1-2-smoke \
  --target riscv32imac-unknown-none-elf \
  --script examples/esp-mosaico-v1-2/onboarding-smoke.yaml \
  --system configs/systems/esp-mosaico-v1-2.yaml \
  --out-dir examples/esp-mosaico-v1-2/evidence
```

Pass criteria: the simulator reports `pass`, GPIO8 has at least two observed
edges as GPIO7 is pressed and released, GPIO7 reads high after release, and the
GPIO8 output latch is low. The image runs from HP SRAM at `0x2F000000`; no ROM,
flash controller, ESP-IDF startup or physical board is exercised.
