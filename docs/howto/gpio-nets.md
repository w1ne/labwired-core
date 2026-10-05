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
| `pull` | The resistor to a rail. Used when no member drives the wire. |
| `latency_ns` | Wire delay from a pad edge to every member seeing it. Default 100 ns. Zero is refused, and so is anything below one cycle of the slowest member. |

## What the net does

For the drives present at one instant, each member being driving-0, driving-1
or released:

1. any member driving 0 gives 0, else any driving 1 gives 1;
2. else the net's `pull`;
3. else the net floats: it reads 0 and is flagged `GPIO_NET_FLOATING`.

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
EXTI interrupts, AVR INT0/INT1 and pin-change interrupts, and timer captures
see a real edge.

The world runs in conservative rounds, the way a timed UART network does. A
round is never longer than the shortest net latency, so an edge produced in a
round can never be due at a peer before the round ends. Results do not depend
on the order nodes run in or on the round length: `world_multichip.rs` (`gpio_net_world`) runs the
example with both node orders and with rounds from 10 ns to 100 ns and compares
every counter, every UART transcript and every applied delivery.

`latency_ns` is also the speed knob. A 100 ns round is a few cycles per node,
so a world with only 100 ns nets runs slower than a lone machine; raise
`latency_ns` on wires that do not need to be that fast and rounds get longer.

A node skips idle time (a Cortex-M in `WFI`, an ATmega328P in `SLEEP`) only
when idle fast-forward is on for it (`set_idle_fast_forward(true)` on the
world's machine); the results are the same either way
(`idle_fast_forward_skips_the_avr_sleep_and_changes_nothing`).

Machines on no net run unchanged; the per-step cost of a machine is the same
with or without nets elsewhere in the world.

## Which pads can be on a net

A net member's GPIO model has to report what it drives and accept an external
level. Supported today:

| GPIO model | Capture | Notes |
|------------|---------|-------|
| `GpioPort`, every register family (STM32 `v2` and `f1`, nRF52/54, Kinetis, EFR32 series 2, SAM, RA, i.MX RT) | push: the port reports its own edges, idle fast-forward stays on | exercised end to end on STM32 `v2` (G0B1). EXTI sees external GPIO edges on the G0 and U5 EXTI only; the F1/F4 EXTI model does not, so count edges there by polling. |
| `avr_gpio` (ATmega328P `portb`/`portc`/`portd`) | push: the port reports its own edges, idle fast-forward stays on | INT0/INT1 and PCINT0..2 see external edges; a core parked in `SLEEP` is skipped until the next edge or Timer0 overflow. Exercised end to end on the Uno (`gpio-net-two-boards`). |

Any other GPIO model (ESP32 family, RP2040 SIO) is refused when the world
is built, naming the pad.

Limits worth knowing:

- A chip's **internal** pull-ups and pull-downs are not part of the net; put the
  pull on the net (`pull:`), as the external resistor it is.
- A pad that firmware drives itself does not raise its own EXTI edge for its own
  transition (the STM32 EXTI model reacts to edges from outside). It does see
  the release when the wire rises after a peer let go.
- The ATmega328P samples INT0/INT1 and PCINT pads at instruction boundaries,
  so an edge is seen at the first boundary after it arrives. Waking from
  power-down, power-save or standby takes four cycles, as from idle: the
  oscillator start-up time is not modelled.
- A pad routed to a peripheral signal the model does not publish has no known
  drive and is refused.

## Reports

`World::gpio_net_reports()` (and `result.json` `gpio_nets` from
`labwired test`, and `WasmWorld.gpio_net_report()` in the browser) lists each
net: level, edge count, contention and floating counters, each member's drive,
and the diagnostics with times in picoseconds.
