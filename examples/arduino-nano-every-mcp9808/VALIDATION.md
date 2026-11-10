# Validation record

Run from the repository root:

```sh
CARGO_BUILD_RUSTC_WRAPPER= scripts/onboarding_smoke.sh \
  --target-id arduino-nano-every-mcp9808 \
  --firmware-build-script examples/arduino-nano-every-mcp9808/firmware/build.sh \
  --script examples/arduino-nano-every-mcp9808/onboarding-smoke.yaml \
  --system configs/systems/arduino-nano-every-mcp9808.yaml \
  --out-dir examples/arduino-nano-every-mcp9808/evidence
```

The committed local smoke passes the `NANO_EVERY_MCP9808_OK` assertion after
reading the MCP9808 manufacturer ID, device ID, and TA register through
ATmega4809 TWI0. The result is in
[`evidence/simulation-output/result.json`](evidence/simulation-output/result.json);
the UART marker is in
[`evidence/simulation-output/uart.log`](evidence/simulation-output/uart.log).

The hosted verification catalog does not list the Nano Every / ATmega4809, so
no hosted `labwired_verify` verdict is claimed. This is local digital-twin
evidence, not hardware confirmation.
