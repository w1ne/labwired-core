# GPIO nets between machines

A `gpio_net` interconnect joins GPIO pads of two or more world nodes into one
electrical net: an interrupt line, a ready line, a chip-select handshake, a wake
pin, or a shared open-drain alert line with a pull-up. Each member keeps
running its own firmware; the net decides what level the wire carries and when
each member sees it.

Runnable example: [`examples/gpio-net-two-boards`](../../examples/gpio-net-two-boards)
(an STM32G0B1 and an ATmega328P).

## Manifest

One `gpio_net` per wire. `nodes` lists every node that owns a member, once each.

```yaml
interconnects:
  - type: gpio_net
    nodes: [avr, stm]
    config:
      name: alert              # optional label shown in reports
      pull: up                 # none (default) | up | down
      latency_ns: 100          # default 100; zero is refused
      members:
        - { node: stm, peripheral: gpiob, pin: 4 }   # open-drain output
        - { node: avr, peripheral: portd, pin: 4 }   # DDR drives low
```

| Key | Meaning |
|-----|---------|
| `members` | At least two pads on at least two nodes. `peripheral` is the node's GPIO peripheral id (`gpioa`, `portd`, ...). A pad can be on one net only; merge nets that share a pad. |
| `pull` | The board's resistor to a rail. A weak level, like a chip's internal pull (see below). |
| `latency_ns` | Wire delay from a pad edge to every member seeing it. Default 100 ns. Zero is refused, and so is anything below one cycle of the slowest member. |

## What the net does

For the drives present at one instant, each member being driving-0 or
driving-1 (strong), held by its own internal pull-up or pull-down (weak), or
released with no pull:

1. any member driving 0 gives 0, else any driving 1 gives 1;
2. else the weak sources, which are the members' internal pulls and the net's
   `pull`: all of them up gives 1, all down gives 0;
3. weak pulls to both rails with nothing driving are a resistor divider. The
   net reports `GPIO_NET_PULL_CONFLICT` (begin and end times, every member's
   drive) and reads the net's own `pull` (the board resistor is normally much
   stronger than a chip's 30-50 kOhm internal one). With no net `pull` it
   reads 0, as contention does;
4. else the net floats: it reads 0 and is flagged `GPIO_NET_FLOATING`.

A chip's internal pull is therefore part of the net. An input with
`PUPDR = 01` on an STM32, or an ATmega pad with `DDRx = 0` and `PORTx = 1`,
holds a wire high with no `pull:` on the net
(`examples/gpio-net-f1-f4`). Reports show it as the member's drive:
`pull_up` / `pull_down` next to `z`, `low` and `high`. On the pad's own
four-state trace a pulled net pad reads `h` / `l` (the IEEE 1164 weak levels).
Pads that are not on a net keep reporting `z` for an undriven input, pulled or
not.

Members driving 0 and 1 together are in **contention**. The wire resolves to 0
(a low-side driver usually wins) and the net reports `GPIO_NET_CONTENTION` with
the time it began, the time it ended, and every member's drive and own cycle.
Nothing is dropped silently.

Open-drain falls out of this: pads that only pull low or release, plus
`pull: up`, make a wired-AND. If one member holds the wire low and a second
joins and then the first lets go, the wire stays low until the second releases:
one low pulse, not two (`open_drain_with_pull_up_is_a_wired_and` in
`crates/core/src/network/gpio_net.rs`).

A member's drive is what *its own* output stage does. What the net feeds back
into the pad (the level firmware reads from `IDR` or `PIND`) is never counted
as the pad's own drive, so a pad cannot hold a wire up just because it once
saw it high.

## Timing

Edges are delivered at `t_edge + latency_ns`, to every member including the
driver, at an exact cycle of each member (the first instruction boundary at or
after that time), through the same `set_gpio_input` path a board button uses:
EXTI interrupts and timer captures see a real edge.

The world runs in conservative rounds, the way a timed UART network does. A
round is never longer than the shortest net latency, so an edge produced in a
round can never be due at a peer before the round ends. Results do not depend
on the order nodes run in or on the round length: `world_multichip.rs` (`gpio_net_world`) runs the
example with both node orders and with rounds from 10 ns to 100 ns and compares
every counter, every UART transcript and every applied delivery.

`latency_ns` is also the speed knob. A 100 ns round is a few cycles per node,
so a world with only 100 ns nets runs slower than a lone machine; raise
`latency_ns` on wires that do not need to be that fast and rounds get longer.

Machines on no net run unchanged; the per-step cost of a machine is the same
with or without nets elsewhere in the world.

## Which pads can be on a net

A net member's GPIO model has to report what it drives and accept an external
level. Supported today:

| GPIO model | Capture | Notes |
|------------|---------|-------|
| `GpioPort`, every register family (STM32 `v2` and `f1`, nRF52/54, Kinetis, EFR32 series 2, SAM, RA, i.MX RT) | push: the port reports its own edges, idle fast-forward stays on | exercised end to end on STM32 `v2` (G0B1, F401) and `f1` (F103). The EXTI raises edge interrupts for net edges on G0 and U5 (port from `EXTI_EXTICRx`), F1 (port from `AFIO_EXTICRx`) and F4 (port from `SYSCFG_EXTICRx`). Internal pulls reported: STM32 `v2` `PUPDR`, STM32 `f1` input-with-pull (`CNF = 10`, `ODR` picks the rail), nRF52 `PIN_CNF.PULL`, EFR32 `INPUTPULL`, SAM `PINCFG.PULLEN`. Kinetis, RA and i.MX RT keep their pull outside the GPIO block, so their pulls are not on the net yet. |
| `avr_gpio` (ATmega328P `portb`/`portc`/`portd`) | **poll**: sampled at every cycle boundary | exact, but the machine runs one instruction at a time and does not fast-forward idle time while a pad is on a net. An input with its `PORTx` bit set is a pull-up on the net. |

Any other GPIO model (ESP32 family, RP2040 SIO) is refused when the world
is built, naming the pad.

Limits worth knowing:

- The ATmega328P port model does not see `MCUCR.PUD` (it is in the CPU's IO
  space), so a pad with `PORTx = 1` counts as pulled up even when firmware
  has set `PUD`.
- A pad that firmware drives itself does not raise its own EXTI edge for its own
  transition (the STM32 EXTI model reacts to edges from outside). It does see
  the release when the wire rises after a peer let go.
- The ATmega328P model has no external or pin-change interrupt yet: count its
  edges by polling `PINx`.
- A pad routed to a peripheral signal the model does not publish has no known
  drive and is refused.

## Reports

`World::gpio_net_reports()` (and `result.json` `gpio_nets` from
`labwired test`, and `WasmWorld.gpio_net_report()` in the browser) lists each
net: level, edge count, contention and floating counters, each member's drive,
and the diagnostics with times in picoseconds.
