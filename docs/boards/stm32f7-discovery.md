# STM32F7 Discovery (STMicroelectronics STM32F746NG) — UART/LED smoke

A Cortex-M7 board (32F746GDISCOVERY) with 1 MiB flash at `0x0800_0000` and
DTCM at `0x2000_0000`. LD1 (PI1) is the user LED; the ST-LINK VCP is USART1.

## Status at a glance

| Aspect             | Status                                                              |
|--------------------|---------------------------------------------------------------------|
| Chip yaml          | [`configs/chips/stm32f746.yaml`](../../configs/chips/stm32f746.yaml) |
| System yaml        | [`configs/systems/stm32f7-discovery.yaml`](../../configs/systems/stm32f7-discovery.yaml) |
| Committed ELF      | `tests/fixtures/stm32f746-discovery-smoke.elf`                       |
| Validation         | `firmware_survival::test_stm32f746_discovery_smoke_survival`         |
| Tier               | **smoke-manual** — boots firmware and prints, **no silicon diff**    |

## What is proven

A soft-float bare-metal firmware un-gates RCC AHB1/APB2, writes `OK\n` on
USART1 (the shared STM32 v2 peripheral IP), and toggles PI1 through BSRR. The
F7 descriptor builds through `SystemBus::from_config` with the shared F7/F4
peripheral set; the F7-specific RCC register layout is modelled.

## What is not proven

No silicon diff and no executing-fidelity differential. LTDC, Ethernet,
DMA2D, USB and QuadSPI are stub windows; the cache/MPU and F7-specific clock
gaps documented in the chip yaml are unmodelled.

## How to run

```bash
labwired test --system configs/systems/stm32f7-discovery.yaml \
  --firmware tests/fixtures/stm32f746-discovery-smoke.elf
```

The onboarding smoke lives in
[`examples/stm32f7-discovery/`](../../examples/stm32f7-discovery/).
