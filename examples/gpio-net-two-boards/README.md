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

## Other chips on the same wires

The STM32 firmware does not care what is on the other end:

| Environment | Peer | Peer's pads | How the peer counts |
|-------------|------|-------------|---------------------|
| `env-rp2040.yaml` | RP2040 (`rp2040-pico`) | `sio` GP2 irq, GP3 ready, GP4 alert (open drain by `GPIO_OE`) | IO_BANK0 GPIO interrupt (`IO_IRQ_BANK0`) |
| `env-esp32c6.yaml` | ESP32-C6 (`esp32c6-devkitc`) | `gpio` GPIO4 irq, GPIO5 ready, GPIO6 alert (open drain by `PAD_DRIVER`) | GPIO interrupt through the interrupt matrix (source 30, CPU line 9) |

Each peer leaves `[ready rising, alert falling, alert rising, interrupts
taken, done]` in SRAM (`0x20000100` on the RP2040, `0x40800100` on the C6):
`7, 5, 5, ≥17, 1`. Sources: `src/rp2040.c`, `src/esp32c6.c` (bare metal, built
with clang). Tests: `an_rp2040_counts_the_stm32_edges_with_gpio_interrupts`
and `an_esp32c6_counts_the_stm32_edges_with_gpio_interrupts` in
`crates/core/tests/world_multichip.rs`.

## Contention

`env-contention.yaml` puts both boards on one push-pull wire: the AVR holds it
low, the STM32 drives it high for a few tens of microseconds. The net reports
`GPIO_NET_CONTENTION` with the time and both members, and the wire stays low.

## Rebuild the firmware

```bash
examples/gpio-net-two-boards/build.sh   # arm-none-eabi-gcc, avr-gcc, clang + lld
```

The ELFs are committed so the tests run without a toolchain.

`stm-demo.elf` / `avr-demo.elf` are the same sources with every delay 20x longer
(`-DTIME_SCALE=20`; `env-demo.yaml`). The playground runs those, so the pulses
last hundreds of microseconds and a person can watch them in the logic analyzer.
The counts are the same (`the_demo_timing_counts_the_same_as_the_fast_one`).

## Limits

See the how-to for the full list. `examples/gpio-net-f1-f4` shows F1/F4 EXTI on a net
and a wire held up by a chip's internal pull-up.
