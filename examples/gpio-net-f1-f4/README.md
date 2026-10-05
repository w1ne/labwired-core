# STM32F1 and F4 EXTI on a net, and internal pull-ups

An STM32F103, an STM32F401 and an ATmega328P in one world, joined by
`gpio_net` interconnects (see [GPIO nets](../../docs/howto/gpio-nets.md)):

| Net | Wire | Pull | What it shows |
|-----|------|------|---------------|
| `irq` | AVR PD2 -> F103 PB0, F401 PC1 | down | Both STM32s count the AVR's 10 pulses with EXTI interrupts. The F103 picks port B for line 0 in `AFIO_EXTICR1`; the F401 picks port C for line 1 in `SYSCFG_EXTICR1`. |
| `alert` | AVR PD4 <-> F401 PA8 | none | The F401's internal pull-up (`PUPDR = 01`) is the only pull. The AVR pulls the wire low 3 times and the F401 counts them on EXTI8. |
| `wake` | F103 PA0 <-> AVR PD5 | none | The AVR's internal pull-up (`DDRD5 = 0`, `PORTD5 = 1`) is the only pull. The F103 pulls the wire low 4 times through an open-drain output, and the AVR polls them. |

Results: the F103 stores `10 10 1` at `0x20000100`, the F401 stores
`10 10 3 3`, and the AVR prints `AVR wake f=4 r=4`.

Test: `cargo test -p labwired-core --test world_multichip gpio_net_f1_f4`.

Rebuild the firmware with `./build.sh` (clang + ld.lld, avr-gcc). The ELFs
are committed.
