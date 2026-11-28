# WeAct BluePill Plus (CH32V203C8T6)

**Demand position:** the measured MCU survey does not rank WCH. The CH32V203
is part of the standing brief's Tier C vendor bet (WCH RISC-V). This round
adds the first catalog board system for the existing CH32V203C8T6 model; it
is board coverage, not a new silicon model.

The WeAct Studio BluePill Plus is a 52.81 mm x 20.78 mm core board that WeAct
builds with several MCUs. This profile is the CH32V203C8T6 build
(WeActStudio/WeActStudio.BluePill-Plus-CH32, MIT):

| Board item | Pad | Profile |
|---|---|---|
| Blue user LED, active high | PB2 | `board_io` `led_pb2` |
| User KEY to 3.3 V (firmware pull-down) | PA0 | `board_io` `key_pa0` |
| USART1 TX header pin | PA9 | `debug_uart: usart1` (capture route only) |
| 8 MHz HSE, 32.768 kHz LSE | PD0/PD1, PC14/PC15 | not modelled |
| USB-C | PA11/PA12 | not modelled |
| SPI flash U3 (unplaced) | PA4..PA7 | not declared |

System profile: [`configs/systems/weact-bluepill-plus-ch32v203.yaml`](../../configs/systems/weact-bluepill-plus-ch32v203.yaml).
Buyability: [`vendor-status.md`](vendor-status.md).

## What the smoke proves

[`weact_template.c`](weact_template.c) is a clean-room bare-metal rewrite of
WeAct's own CH32V203 template (`Examples/CH32V203/01-template`): the same
USART1 banner, PB2 toggled every 250 passes of a 1 ms SysTick delay, and a
KEY press that changes the blink to every 100 ms after the release. No WeAct
or WCH SDK source is copied.

[`template-smoke.yaml`](template-smoke.yaml) holds KEY from 7.0 M to 7.8 M
cycles. It passes 4/4 (banner order, at least 10 PB2 edges, PA0 and PB2
pad modes). [`check-blink-rate.py`](check-blink-rate.py) then measures the
PB2 edge spacing from `result.json`:

```
ok: before press: 2 half-periods [251.06, 251.06] ms, want 250 ms +-1 %, need >= 2
ok: after release: 7 half-periods [100.43, 100.43, ...] ms, want 100 ms +-1 %, need >= 5
```

The extra 0.4 % is the loop body (about 34 cycles per 1 ms pass). The
negative control ([`evidence/template-smoke/negative-control`](evidence/template-smoke/negative-control))
removes the KEY stimuli: PB2 stays at 250 ms, `gpio_edges` fails (6 < 10)
and the checker fails on the 100 ms side.

## Copy-paste bare-metal C example

```c
#include <stdint.h>

#define RCC_APB2PCENR (*(volatile uint32_t *)0x40021018u)
#define GPIOA_CFGLR   (*(volatile uint32_t *)0x40010800u)
#define GPIOA_INDR    (*(volatile uint32_t *)0x40010808u)
#define GPIOA_OUTDR   (*(volatile uint32_t *)0x4001080Cu)
#define GPIOB_CFGLR   (*(volatile uint32_t *)0x40010C00u)
#define GPIOB_OUTDR   (*(volatile uint32_t *)0x40010C0Cu)

void main(void) {
    RCC_APB2PCENR |= (1u << 2) | (1u << 3);            /* GPIOA, GPIOB */
    GPIOA_CFGLR = (GPIOA_CFGLR & ~0xFu) | 0x8u;         /* PA0 input, pull */
    GPIOA_OUTDR &= ~1u;                                 /* pull-down */
    GPIOB_CFGLR = (GPIOB_CFGLR & ~0xF00u) | 0x300u;     /* PB2 push-pull */
    for (;;) {
        /* LED follows KEY: pressed -> PB2 high -> blue LED on. */
        if (GPIOA_INDR & 1u) {
            GPIOB_OUTDR |= 1u << 2;
        } else {
            GPIOB_OUTDR &= ~(1u << 2);
        }
    }
}
```

## Build and run

```sh
./examples/weact-bluepill-plus-ch32v203/build.sh   # needs RV32IMAC GCC
./examples/weact-bluepill-plus-ch32v203/ci/build.sh # digest check
./examples/weact-bluepill-plus-ch32v203/ci/test.sh  # smoke + rate checker
```

The committed `tests/fixtures/weact-bluepill-plus-ch32v203-template.elf` was
built with `riscv64-unknown-elf-gcc` 13.2.0 (Ubuntu 13.2.0-11ubuntu1+12);
sha256 in [`fixtures.sha256`](fixtures.sha256).

## Known limits, stated plainly

- The board's own firmware runs the 8 MHz HSE through the PLL to 144 MHz.
  The engine's F1 RCC model decodes PLLMUL with the STM32F1 table, so the
  V20x x18 setting gives 128 MHz. This firmware stays on the 8 MHz HSI.
- USB on PA11/PA12, the crystals, and the unplaced SPI flash are not
  modelled. The chip-level limits are in
  [`configs/chips/ch32v203c8t6.yaml`](../../configs/chips/ch32v203c8t6.yaml).
- No physical-board capture exists; this is model evidence only.
