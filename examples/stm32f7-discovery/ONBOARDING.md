# STM32F746G-DISCO onboarding

Checked against ST's live product and store pages on 2026-11-03. The board is
listed as **Active** and orderable; see [`evidence/vendor-status.md`](evidence/vendor-status.md).

## Modeled board paths

- Board system: [`configs/systems/stm32f7-discovery.yaml`](../../configs/systems/stm32f7-discovery.yaml)
- MCU profile: [`configs/chips/stm32f746.yaml`](../../configs/chips/stm32f746.yaml)
- Board mapping: LD1 is PI1 and the ST-LINK Virtual COM Port is USART1. The
  source references are UM1974 and RM0385, listed in [`REQUIRED_DOCS.md`](REQUIRED_DOCS.md).
- Firmware smoke: `firmware-stm32f746-demo`, built for
  `thumbv7em-none-eabi`. It ungates GPIOI and USART1, emits `OK` over USART1,
  and sets PI1. The smoke asserts both the UART marker and at least one PI1
  GPIO edge.

Reproduce the committed build and simulator output with:

```bash
scripts/onboarding_smoke.sh \
  --target-id stm32f746-discovery \
  --crate firmware-stm32f746-demo \
  --target thumbv7em-none-eabi \
  --script examples/stm32f7-discovery/io-smoke.yaml \
  --system configs/systems/stm32f7-discovery.yaml \
  --out-dir examples/stm32f7-discovery/evidence
```

The run result, UART bytes, GPIO edge assertion, build logs, and onboarding
metrics are committed under [`evidence/`](evidence/).

The hosted LabWired catalog currently lists `stm32f7-discovery` as
`compilable: false`, so there is no hosted `labwired_verify` verdict for this
board. The evidence here is the repository-local LabWired CLI smoke; it is
simulator evidence, not a hosted verification or hardware confirmation.

## Model boundary

This is a narrow simulator profile. It demonstrates firmware startup, USART1
output, and LD1's PI1 GPIO path. LTDC, Ethernet, DMA2D, USB OTG FS, and QuadSPI
are explicitly zero-answering windows in the chip descriptor and are not
covered by this smoke. FMC/SDRAM, SAI, SDMMC, DCMI, RNG, and GPIOJ/K are not
declared. The LCD, touch controller, external memories, and other board
peripherals are therefore outside the supported claim. This evidence is
simulator-only; the STM32F746 model has no silicon comparison.
