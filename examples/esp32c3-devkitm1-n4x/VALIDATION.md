# Validation — ESP32-C3-DevKitM-1-N4X

Run the strict onboarding smoke from the repository root:

```sh
scripts/onboarding_smoke.sh \
  --target-id esp32c3-devkitm1-n4x \
  --crate firmware-esp32c3-devkitm1-smoke \
  --target riscv32imc-unknown-none-elf \
  --script examples/esp32c3-devkitm1-n4x/onboarding-smoke.yaml \
  --system configs/systems/esp32c3-devkitm1-n4x.yaml \
  --out-dir examples/esp32c3-devkitm1-n4x/evidence
```

The smoke passes only when the model observes at least 48 GPIO8 edges and
decodes a red pixel (`RGB565 0xF800`) from the 24-bit GRB one-wire stream. It
then stops at the configured step limit. The committed evidence is under
`evidence/`. The smoke firmware reserves a 4 KiB stack; this is not a general
application stack-size recommendation.

GPIO8's waveform and the attached LED model are checked by the repository's
local deterministic onboarding smoke. The hosted diagram validator accepted
an equivalent C3 Super Mini diagram, but the hosted verifier's current call
contract rejects the required Arduino `.ino` entry path; no hosted verdict was
returned. See [`evidence/labwired-verification.md`](evidence/labwired-verification.md).
