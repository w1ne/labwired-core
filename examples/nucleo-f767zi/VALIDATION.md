# NUCLEO-F767ZI Validation Runbook

Run from `core/`. The fixture is built for the actual NUCLEO-F767ZI with
PlatformIO `ststm32@19.7.1` and STM32duino, rather than borrowing an F401 ELF.
Source, exact toolchain and rebuild instructions are in
`tests/fixtures/stm32f767-stock-arduino/README.md`.

```bash
cargo run -q -p labwired-cli -- test \
  --script examples/nucleo-f767zi/io-smoke.yaml \
  --output-dir out/nucleo-f767zi/io-smoke --no-uart-stdout
```

The simulation should capture `F767_OK` on the ST-LINK USART3 path and at
least four transitions on the green LED at PB0. `uart-smoke.yaml` is the
serial-only variant. The model uses instantaneous F4-style overdrive-ready
transitions and a shared STM32 CRC model; unmodeled peripheral behavior and
physical-board acceptance are outside this fixture's evidence.
