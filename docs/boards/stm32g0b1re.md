# STM32G0B1RE — USART1, TIM2, and GPIO EXTI examples

A narrow **STM32G0B1RE** slice for the IO-Link reference firmware:
Cortex-M0+, 512 KB flash, 144 KB SRAM, 16 MHz reset HSI. The modelled
peripherals are the ones that slice uses. This page is the runnable
illustration of those peripherals. It is not a silicon capture and it
does not claim a full G0B1.

!!! tip "Live status"
    Authoritative automation:

    - [Chip conformance scoreboard](../coverage/chip-conformance.md)
    - [Target support rubric](../target_support_rubric.md)
    - Customer ELF witness: [G0 prerequisites](../iolinki-g0-prerequisites.md)

## Status at a glance

| Aspect | Status |
|--------|--------|
| Chip descriptor | [`configs/chips/stm32g0b1re.yaml`](../../configs/chips/stm32g0b1re.yaml) |
| Example | [`examples/stm32g0b1re/`](../../examples/stm32g0b1re/) |
| Firmware source | [`crates/firmware-stm32g0b1-demo/`](../../crates/firmware-stm32g0b1-demo/) |
| Committed ELF | `tests/fixtures/stm32g0b1re-smoke.elf` |
| Smoke | `examples/stm32g0b1re/io-smoke.yaml` |

## Flash / firmware artifact

```bash
cargo build -p firmware-stm32g0b1-demo --release --target thumbv6m-none-eabi
```

The committed ELF is that release binary. Copy it back to
`tests/fixtures/stm32g0b1re-smoke.elf` after a source change. The
customer IO-Link image is a different file, built by
`.github/workflows/iolinki-reference.yml`.

## Pins

| Signal | Pin | Alternate function |
|--------|-----|--------------------|
| USART1_TX | PA9 | AF1 |
| USART1_RX | PA10 | AF1 |
| USART2_TX / RX | PA2 / PA3 | AF1 |
| EXTI line 0 | PB0 | port code `1` in `EXTICR1` |

TIM2 has no pin in this example. The prescaler is programmed directly.

## Support matrix

| Peripheral | Example | Limit |
|------------|---------|-------|
| RCC, G0 offsets | ✅ clocks GPIOA, TIM2, USART1 | HSI ready only; no PLL |
| GPIOA `stm32v2` | ✅ PA9 mode and AFRH | Other ports are declared, not exercised here |
| USART1 | ✅ `OK` at 115200 8N1 | No RX, no COM2 byte exchange |
| TIM2 | ✅ `PSC = 15` (1 MHz from 16 MHz) | Counter is not started |
| EXTI, G0 GPIO bank | ✅ PB0 falling, `IMR1` unmasked | No pad edge in this smoke; see `g0_exti` and the customer ELF |
| SYSCFG | ❌ | Declared window, stub only |

## Exact IO-Link operating-point extension

The original smoke above remains the16MHz UART-console prerequisite.
[`examples/iolink-g0b1re-tiol112`](../../examples/iolink-g0b1re-tiol112/README.md)
adds USART2 at `0x40004400`, shared USART2/LPUART2 IRQ28, the complete144KiB
SRAM address range, and a separate TIOL112 transceiver model. PWR CR1/SR2 and
FLASH ACR are available for the64MHz clock-setup sequence; this does not
validate the other G0B1 flash/programming/power modes.

`peripherals::stm32g0_clock::validate_timed_uart_clock` derives HSI/PLL,
AHB/APB and TIM clocks from the actual RCC registers. It rejects a16MHz
firmware image measured against a64MHz timebase, divided APB, alternate UART
kernel clocks and non-reset USART PRESC. Simulation time remains a fixed
operating-point assumption; oscillator startup, changing clock frequency,
flash wait-state execution penalties and IAR instruction timing are unproven.
DMA/DMAMUX are not declared by this device slice. The timed UART reports
CR3.DMAR as unsupported, so an IRQ/polling run cannot establish DMA coverage.

## How to run

```bash
cargo run -q -p labwired-cli -- test \
  --script examples/stm32g0b1re/io-smoke.yaml
```

Worked register sequences for USART1, TIM2, and EXTI are in the
[example README](../../examples/stm32g0b1re/README.md).
