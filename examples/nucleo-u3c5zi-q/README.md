# NUCLEO-U3C5ZI-Q quick start

## Demand rank

This is a demand-ranked addition for "other STM32": rank #4 at 13% in the
JetBrains State of Embedded survey. Nucleo/Discovery boards are 23% of the
reported kits. The exact board is Active and currently in stock; see
[vendor-status.md](vendor-status.md).

## Copy-paste smoke

The no-std Rust firmware sends `U3C5_USART1_OK` through the board's USART1 VCP
and toggles green LD1 on PA5. Run from the repository root:

```bash
cargo build -p firmware-nucleo-u3c5zi-q-smoke --release --target thumbv8m.main-none-eabi
cargo run -q -p labwired-cli -- test \
  --script examples/nucleo-u3c5zi-q/io-smoke.yaml \
  --output-dir out/nucleo-u3c5zi-q/io-smoke --no-uart-stdout
```

The smoke requires the USART1 marker and at least eight PA5 transitions. The
descriptor includes only the Cortex-M33 boot/memory path, RCC gates, GPIOA,
USART1, LD1 and the host console route. UART text is captured from the USART
register model; the physical ST-LINK bridge and PA9/PA10 wire waveform are not
simulated. Other MCU and board peripherals, TrustZone policy, clock changes,
and electrical power behavior remain outside this profile.

Build and run logs are committed under [evidence/logs](evidence/logs/).
The committed result is from the repository's local onboarding harness. The
hosted `labwired_verify` catalog does not yet expose this board, so this result
is not reported as a hosted `model_verified` verdict; see
[VALIDATION.md](VALIDATION.md).
