# Validation — NUCLEO-F103RB

**Status: simulator smoke passed 2026-10-27.** This validates the modeled
STM32F103RBT6 system and firmware path. It is not a physical-board test.

## Build and smoke results

Tool: `labwired-cli 0.25.0`

Firmware target: `thumbv7m-none-eabi`

Firmware SHA-256: `8d3407f85e744978a678eddce3fb427b057c244ca91d9a4d2e16b57270e33630`

```sh
cargo build -p firmware-nucleo-f103rb-smoke --release --target thumbv7m-none-eabi
cargo run -q -p labwired-cli -- test \
  --script examples/nucleo-f103rb/io-smoke.yaml \
  --output-dir out/nucleo-f103rb/io-smoke --no-uart-stdout
cargo run -q -p labwired-cli -- test \
  --script examples/nucleo-f103rb/uart-smoke.yaml \
  --output-dir out/nucleo-f103rb/uart-smoke --no-uart-stdout
```

- Firmware build completed successfully.
- GPIO + USART2 smoke: **PASS, 3/3 assertions**, 200,000 steps. UART captured
  `OK`; PA5 captured **10 edges**; stop reason was `MaxSteps`.
- UART-only smoke: **PASS, 2/2 assertions**, 200,000 steps. UART captured
  `OK`; stop reason was `MaxSteps`.

## Unsupported-instruction audit

```sh
./scripts/unsupported_instruction_audit.sh \
  --firmware target/thumbv7m-none-eabi/release/firmware-nucleo-f103rb-smoke \
  --system configs/systems/nucleo-f103rb.yaml \
  --max-steps 200000 \
  --out-dir out/unsupported-audit/nucleo-f103rb \
  --fail-on-unsupported
```

Result: simulator exit 0; **200,000 instructions executed, 0 unsupported**
(100% instruction support for this run). The generated report was
`out/unsupported-audit/nucleo-f103rb/report.md`.

Physical NUCLEO-F103RB UART/GPIO comparison remains unverified. Peripheral
coverage beyond the documented GPIO and USART2 smoke path remains subject to
the limits in `KNOWN_LIMITATIONS.md`.
