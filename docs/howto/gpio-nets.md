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
EXTI interrupts and timer captures see a real edge.

Each node keeps its own clock (conservative parallel discrete-event
simulation). A node may run as far as its *safe horizon*: for each net it is
on, the time the slowest member of that net has reached, plus the net's
latency. An edge nobody has reported yet happens after its driver's current
time, so its delivery is due after that horizon: every delivery a node needs is
known before the node gets there, whatever order the nodes run in and however
far apart their clocks are. After a node runs, its drive changes are merged
into its nets up to the time all their members have reached.

Results do not depend on node order, round length or how the world is driven:
`world_multichip.rs` (`gpio_net_world`) runs the example with both node orders,
with rounds from 10 ns to 100 ns, round by round, in one `run_until_ps` call and
on the old lockstep round driver (`set_gpio_lockstep`, still the driver for a
world that also has a timed UART network), and compares every counter, every
UART transcript, every applied delivery and every node's cycle count, part way
through and at the end.

`step_all` still advances the world by one round (the shortest latency) per
call, so a `max_steps` limit, the browser's step batches and Python's
`run_for` keep their meaning. `World::run_until_ps(t)` runs to `t` in one call,
and `World::step_rounds(n)` does `n` rounds in one call when that gives the
same result (the browser's `step_batch` uses it).

### Speed

At the default 100 ns latency a net no longer costs a multiple of the
machines' own time. `examples/gpio-net-two-boards`, 30 ms of simulated time,
release build, against the same two machines each run alone for 30 ms (median
of repeated runs on a shared machine, so treat the figures as rough):

| How it is driven | Before | Now |
|------------------|--------|-----|
| one `run_until_ps` / `step_rounds` call | n/a | 1.16x the two machines alone |
| `step_all` per round | 2.05x | 1.55x |
| `labwired test --script .../test.yaml` (40 ms, per round, wall) | 0.54 s | 0.36 s |

(`cargo test --release -p labwired-core --test world_multichip -- --ignored
--nocapture gpio_net_speed` prints the first two.)

What is left:

- A net node still runs in pieces no longer than the shortest latency of its
  nets, because its own edges come back to it after one latency. The world
  nodes built today (Cortex-M, AVR) tick their peripherals every cycle anyway,
  so the pieces cost almost nothing extra. A node that would otherwise run
  wide batches or fast-forward idle time (an ESP32-C3 ROM-boot node) loses
  that while it is on a net, so it runs slower than alone at 100 ns.
- A sleeping node does not stretch the horizon yet: a node idle in `WFI` still
  advances one latency at a time while its peers do.
- `step_all` pays a fixed cost per call (the results map), about half the
  machines' own time at 100 ns rounds. Drive long runs with `run_until_ps` or
  `step_rounds`; `latency_ns` still makes rounds longer where a wire does not
  need to be fast.

Machines on no net run unchanged and are not held to any net's latency.

## Which pads can be on a net

A net member's GPIO model has to report what it drives and accept an external
level. Supported today:

| GPIO model | Capture | Notes |
|------------|---------|-------|
| `GpioPort`, every register family (STM32 `v2` and `f1`, nRF52/54, Kinetis, EFR32 series 2, SAM, RA, i.MX RT) | push: the port reports its own edges, idle fast-forward stays on | exercised end to end on STM32 `v2` (G0B1). EXTI sees external GPIO edges on the G0 and U5 EXTI only; the F1/F4 EXTI model does not, so count edges there by polling. |
| `avr_gpio` (ATmega328P `portb`/`portc`/`portd`) | **poll**: sampled at every cycle boundary | exact, but the machine runs one instruction at a time and does not fast-forward idle time while a pad is on a net |

Any other GPIO model (ESP32 family, RP2040 SIO) is refused when the world
is built, naming the pad.

Limits worth knowing:

- A chip's **internal** pull-ups and pull-downs are not part of the net; put the
  pull on the net (`pull:`), as the external resistor it is.
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
