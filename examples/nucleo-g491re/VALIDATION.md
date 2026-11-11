# NUCLEO-G491RE validation

**Scope:** SIM-derived, board GPIO smoke. This is not a silicon capture or a
hardware-confirmed result. The model claims only the STM32G4 RCC/GPIO subset
used by this smoke and the board's LD2/B1 wiring. The shared STM32G491RE chip
descriptor also declares GPIOB and I2C1 for the separately scoped BME280 add-on
profile; this base-board smoke does not exercise that path.

## Run

```bash
scripts/onboarding_smoke.sh \
  --target-id nucleo-g491re \
  --crate firmware-nucleo-g491re-smoke \
  --target thumbv7em-none-eabi \
  --script examples/nucleo-g491re/io-smoke.yaml \
  --system configs/systems/nucleo-g491re.yaml \
  --out-dir examples/nucleo-g491re/evidence \
  --threshold-seconds 3600
```

The committed smoke result is **PASS 2/2** after 200,000 steps. It records 50
transitions on `gpioa:5` (LD2) and reaches the configured instruction budget.
Build/run logs and the structured result are under `evidence/`.

## Model boundary

The board has an onboard ST-LINK-V3E, USB connector, 24 MHz HSE and 32.768 kHz
LSE as documented by ST. Those blocks, the ST-LINK virtual COM path, analog
measurement circuitry, and other G491 peripherals are outside this descriptor.
The 16 KiB CCM SRAM at `0x10000000` (and its `0x20018000` alias) is also
intentionally not mapped; the descriptor covers the contiguous 96 KiB main
SRAM only. No zero-answering peripheral stub is used on the PA5 smoke path.
The hosted LabWired catalog currently has no G491 board entry; `labwired_list`
returned no G491 board, and the verifier connector rejected `nucleo-g491re` as
outside its board enum. This smoke therefore has a local CLI pass and no hosted
`labwired_verify` verdict.
