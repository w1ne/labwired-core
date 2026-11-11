# Validation record

Demand rank: **ESP32 #1 (38%)**. This is a board-profile addition for existing
ESP32-S3 silicon.

The local onboarding smoke builds Seeed's `seeed_xiao_esp32s3` Arduino target
with an 8 MiB flash layout, then fast-starts the application ELF and checks the
USB-serial marker plus five or more GPIO21 edges on the attached active-low user
LED. The exact command output and metrics are committed under `evidence/`.

The profile represents the board's 8 MiB flash. The physical 8 MiB OPI PSRAM is
intentionally excluded; firmware is built QSPI-only and does not access it.
Wi-Fi, BLE, USB electrical signaling/enumeration, battery charging, charge LED,
camera, microphone, and SD are outside this acceptance test. A passing local
smoke is simulator evidence, not a physical-board test.

A separate `--rom-boot` attempt loaded the vendored S3 mask ROM and entered the
second-stage bootloader, then looped at `0x40041a7c` without reaching the app;
the UART and GPIO oracle clauses failed at 120 million steps. That run is not
accepted as evidence. Full mask-ROM boot remains outside this board profile;
the passing result below is specifically the application ELF fast-start path.

The hosted `labwired_verify` catalog does not contain the XIAO ESP32-S3 board
ID. Its separate hosted check returned `status: model_verified` for the same
USB-serial marker and GPIO21-low behavior on the supported ESP32-S3-DevKitC-1
profile. That checks shared S3 firmware behavior, not this XIAO-specific board
descriptor or pin wiring. The hosted result has one non-blocking approximation:
the circuit has no routed supply net, so VDD is idealized and brown-out behavior
is not simulated. The raw oracle result is in
[`evidence/hosted-verification.json`](evidence/hosted-verification.json).
