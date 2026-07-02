# f103-j1939-monitor — per-SA BAM reassembly against a real CANsub capture

An RX-only J1939 node on the STM32F103 bxCAN peripheral (real normal mode,
accept-all filter — not loopback), fed by the `can-player` external device
replaying an 8-second slice of a **real** CANsub J1939 sample capture
(see `captures/README.md` for provenance). The firmware reassembles
multi-packet BAM (Broadcast Announce Message) transport sessions, decodes
DM1 (active diagnostic trouble code) lamp status from every source address
on the bus, and prints everything as human-readable UART lines.

This is a regression test for a specific, real class of J1939-stack bug:
**keying BAM reassembly sessions by source address (SA), not globally.**

## The bug this catches

The capture slice contains two J1939 BAM broadcasts whose Data Transfer
(TP.DT) packet streams are genuinely interleaved on the wire — Engine
Configuration (PGN 0xFEE3) from SA 0x00 and Retarder Configuration
(PGN 0xFEE1) from SA 0x0F, opening about 11 ms apart, with their TP.DT
frames alternating before either session closes.

A **correct** stack keys each BAM reassembly session by the sender's source
address, so the two sessions reassemble independently even though their
frames interleave:

```
BAM sa=00 pgn=FEE3 len=39 data=0013B98033DA201B...   ENGINE idle_rpm=608
BAM sa=0F pgn=FEE1 len=19 data=030360224A804D27...
```

A **naive** stack that keeps only a single global reassembly buffer (common
in toy/demo J1939 code, since most test traffic never actually interleaves)
overwrites the engine session's buffer with the retarder's TP.DT bytes as
they arrive, because both sessions share one buffer regardless of SA. The
result still gets tagged `pgn=FEE3` (from the engine's last-seen Connection
Management frame) but the payload bytes are the retarder's:
`data=030360...`, decoding to a nonsense `idle_rpm=96`. This bug is
invisible on synthetic/non-interleaved test traffic and only shows up
against real, concurrently-broadcasting buses — which is why this example
replays a real capture instead of a hand-written one.

`examples/f103-j1939-monitor/firmware/main.c` implements the per-SA fix
(`sess_for()`, keyed on `sa`); `j1939-replay.yaml` asserts both correct BAM
lines verbatim, so a regression back to a single global session fails the
build.

The firmware also tabulates DM1 lamp-status frames and counts distinct
source addresses (`DM1 sources=9` — every SA that raises DM1 in the slice).

## Run

Build the firmware once:
```
cd examples/f103-j1939-monitor/firmware && make
```

Run the replay session (8s capture slice, real `can-player` device reading
`captures/j1939-slice-8s.log` from disk):
```
cd examples/f103-j1939-monitor
cargo run -q -p labwired-cli -- test --script j1939-replay.yaml \
  --output-dir out --no-uart-stdout
```
`out/uart.log` shows every decoded BAM reassembly and DM1 line, in capture
order. At `ticks_per_second: 1000000` (1 tick = 1 µs of recorded time), the
8-second slice runs out to `max_steps: 9000000` (`expected_stop_reason:
max_steps`), giving the firmware headroom to boot and drain the bus.

`system.yaml` is a playground variant of `replay-system.yaml`: identical
`can-player` device, but with the capture inlined as `config.data` (a 2s
slice) instead of `config.path`, since the wasm playground has no
filesystem to read `captures/*.log` from.

## Determinism

Two runs of `j1939-replay.yaml` produce byte-identical `uart.log` and
`result.json` (the simulator, the candump parser, and the `can-player`
device's tick-scheduled replay are all fully deterministic — no wall-clock
or OS-timing dependence). This is what makes the module usable as a
reliable CI regression gate for the per-SA reassembly fix above.
