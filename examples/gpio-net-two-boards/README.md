# Two boards, three wires

An STM32G0B1 and an ATmega328P in one world, joined by `gpio_net`
interconnects (see [GPIO nets](../../docs/howto/gpio-nets.md)):

| Net | Wire | Pull | What it shows |
|-----|------|------|---------------|
| `irq` | AVR PD2 -> STM32 PB0 | down | The STM32 counts the AVR's pulses with an EXTI interrupt (rising and falling separately). |
| `ready` | STM32 PB1 -> AVR PD3 | down | The AVR counts the STM32's pulses with INT1 (rising edges), sleeping in between. |
| `alert` | STM32 PB4 <-> AVR PD4 | up | One shared open-drain line. The STM32 pulls it low 5 times (the AVR counts them with the PCINT2 pin-change interrupt), then the AVR pulls it 3 times and the STM32 counts them. |

Each board reports over its UART:

```
STM irq r=10 f=10 alert r=3 f=3
AVR ready=7 alert f=5 r=5
```

Every number comes from the firmware's loops: 10 irq pulses, 7 ready pulses,
5 alert pulses from the STM32 and 3 from the AVR.

## Run it

```bash
labwired test --script examples/gpio-net-two-boards/test.yaml --output-dir out
```

`out/result.json` has a `gpio_nets` block with each net's level, edge count
(20, 14 and 16) and diagnostics. The test is also
`cargo test -p labwired-core --test world_multichip gpio_net_world`, which additionally checks
that node order and round length change nothing.

## Contention

`env-contention.yaml` puts both boards on one push-pull wire: the AVR holds it
low, the STM32 drives it high for a few tens of microseconds. The net reports
`GPIO_NET_CONTENTION` with the time and both members, and the wire stays low.

## Rebuild the firmware

```bash
examples/gpio-net-two-boards/build.sh   # arm-none-eabi-gcc, avr-gcc
```

The ELFs are committed so the tests run without a toolchain.

`stm-demo.elf` / `avr-demo.elf` are the same sources with every delay 20x longer
(`-DTIME_SCALE=20`; `env-demo.yaml`). The playground runs those, so the pulses
last hundreds of microseconds and a person can watch them in the logic analyzer.
The counts are the same (`the_demo_timing_counts_the_same_as_the_fast_one`).

## Limits

A chip's internal pull-ups are not part of the net. See the how-to for the full
list.
