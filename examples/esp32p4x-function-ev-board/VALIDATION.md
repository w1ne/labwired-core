# ESP32-P4X Function EV validation runbook

Run from the repository root:

```bash
rustup target add riscv32imac-unknown-none-elf
CARGO_BUILD_RUSTC_WRAPPER= scripts/onboarding_smoke.sh \
  --target-id esp32p4x-function-ev-board \
  --crate firmware-esp32p4x-function-ev-board-smoke \
  --target riscv32imac-unknown-none-elf \
  --script examples/esp32p4x-function-ev-board/onboarding-smoke.yaml \
  --system configs/systems/esp32p4x-function-ev-board.yaml \
  --out-dir examples/esp32p4x-function-ev-board/evidence
```

Pass criteria: the local simulator result is `pass`, GPIO8 (`gpio:8`) produces
at least 32 observed edges, and execution stops at the configured step limit.
The image links into the P4 IROM window at `0x40000000` and HP RAM at
`0x4ff40000`, with the low 256 KiB reserved for ROM. The smoke directly loads
the RISC-V ELF entry point; this is not a ROM boot or flash-controller test.

The evidence is a local repository run. The hosted LabWired verifier does not
list ESP32-P4/P4X as a supported target, so no hosted `model_verified` result
is claimed.
