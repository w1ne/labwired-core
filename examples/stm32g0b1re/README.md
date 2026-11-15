# STM32G0B1RE peripheral examples

Run commands from the repository root. The image is Cortex-M0+
(`thumbv6m-none-eabi`). Source:
[`crates/firmware-stm32g0b1-demo`](../../crates/firmware-stm32g0b1-demo/).

This is the in-repo illustration of the G0B1 slice in
[`configs/chips/stm32g0b1re.yaml`](../../configs/chips/stm32g0b1re.yaml):
RCC at the G0 offsets, USART1 on PA9 AF1, TIM2, and the G0 GPIO EXTI bank.
It does not boot the customer IO-Link ELF and it does not claim an
electrical C/Q link. That witness stays in
[`docs/iolinki-g0-prerequisites.md`](../../docs/iolinki-g0-prerequisites.md).

Register numbers are from RM0444. The chip yaml is the address map the
simulator loads.

## Build and run

```bash
rustup target add thumbv6m-none-eabi
cargo build -p firmware-stm32g0b1-demo --release --target thumbv6m-none-eabi
cp target/thumbv6m-none-eabi/release/firmware-stm32g0b1-demo \
   tests/fixtures/stm32g0b1re-smoke.elf

cargo run -q -p labwired-cli -- test \
  --script examples/stm32g0b1re/io-smoke.yaml
```

UART from a passing run:

```
OK
usart1 pa9 af1
tim2 psc=15
exti0 port=B falling imr=1
```

## Example 1 — USART1 on PA9, AF1

G0 USART1 is `0x40013800`. The TX pad is PA9, and the alternate function
is 1 (not the L0 AF4 map). Clock the port and the UART first: GPIOA is
`RCC_IOPENR` bit 0 (`0x40021034`), USART1 is `RCC_APBENR2` bit 14
(`0x40021040`).

```rust
// PA9 mode = alternate function, AFRH nibble = 1.
moder |= 0b10 << (9 * 2);
afrh |= 1 << 4;
// 16 MHz HSI / 115200.
usart1_brr = 16_000_000 / 115_200;
usart1_cr1 = UE | TE;
```

Bytes written to TDR (`0x28`) while TXE (`ISR` bit 7) is set are the
console bytes. The smoke expects `OK` and `usart1 pa9 af1`.

## Example 2 — TIM2 prescaler

TIM2 is `0x40000000`. Its clock gate is `RCC_APBENR1` bit 0
(`0x4002103C`). `PSC` is at offset `0x28`. With the 16 MHz reset clock,
`PSC = 15` divides to 1 MHz (`16 MHz / (15 + 1)`).

```rust
tim2_psc = 15;
```

The smoke reads `PSC` back from both the firmware and
`memory_value` at `0x40000028`.

## Example 3 — EXTI line 0 on PB0, falling edge

The G0 EXTI block is `0x40021800`. It is not the F1 or L4 register map.
This slice programs the GPIO lines only.

| Register | Offset | Example write | Meaning |
|----------|--------|---------------|---------|
| `FTSR1` | `0x04` | `1` | Line 0 falls |
| `EXTICR1` | `0x60` | `1` | Line 0 source is port B (low 3 bits) |
| `IMR1` | `0x80` | `1` | Line 0 interrupt unmasked |
| `FPR1` | `0x10` | write-1-to-clear | Falling pending flag |

```rust
exti_exticr1 = 1; // PB0
exti_ftsr1 = 1;
exti_imr1 = 1;
```

Port codes in `EXTICR` are `0` = PA, `1` = PB, `2` = PC. A falling edge
on the selected pad sets `FPR1` and raises EXTI IRQ 5 while `IMR1` has
that line set. Clearing is write-1-to-clear on `FPR1`. Rising and
falling flags are separate (`RPR1` at `0x0C`, `RTSR1` at `0x00`).

The smoke checks the three configuration words. It does not inject the
pad edge; the customer ELF test does that, and
`crates/core/tests/stm32/g0_exti.rs` checks the edge without firmware.
