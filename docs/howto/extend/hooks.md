---
description: "There is no generic hooks: key. LabWired's hook surfaces are device behaviour as YAML, host-side observation (bus trace, --stop-on, logic capture, Python frames) and the Rust SimulationObserver / Peripheral traits — each with a receipt from this checkout."
---

# Hook surfaces (device, host, Rust)

"Where do I hang my hook?" is three different questions on LabWired, and there
is deliberately **no generic `hooks:` key** in chip, manifest or part YAML.
Neither the schema parser (`crates/config/src`) nor the shipped descriptors
(`configs/`) accept one; the only hit for `hooks` there is a comment in
`configs/chips/esp32c6.yaml` that names `bus/device_hooks.rs`. What exists
instead:

- **Device data hooks** — a part YAML's `behavior:` block: pin-edge `rules:` and
  finite `schedules:` that *are* the simulated behaviour. Use this when the
  "hook" is part of the device being modelled.
- **Host observation** — artifacts and streams produced by a run:
  `--bus-trace-out` (VCD/JSON), `--stop-on` (ESP32-S3 console), logic capture
  (`--gpio-trace` / `--watch-gpio`), and the Python SDK's `Sim::frames()`. Use
  this when firmware, not the model, owns the behaviour and you only need to
  watch it.
- **Rust extension points** — `SimulationObserver` callbacks and the
  `Peripheral` trait's tick/clock attach points. Use this when you are changing
  the simulator itself.

That list is the surface, not a sampler: there is no fourth family and no
generic `hooks:` key. Mechanisms live *inside* the families — `behavior.rules`
and `behavior.schedules` are family 1's two mechanisms; family 2's are the
bus-trace ring, `--stop-on`, logic capture and the Python frame drain; family 3
is the observer callbacks plus the `Peripheral` attach/tick seams.

| You want to… | Family | Entry point |
|---|---|---|
| make a sensor answer a protocol | 1. device data | `behavior.rules` / `behavior.schedules` in device YAML |
| capture what went over the wires | 2. host | `labwired run --bus-trace-out trace.vcd` |
| stop right after a firmware log line | 2. host | `labwired run --stop-on <text>` (ESP32-S3) |
| capture GPIO edges as JSON lines | 2. host | `labwired run --gpio-trace trace.jsonl` (ESP32-S3) |
| capture pad edges into the verdict | 2. host | `labwired test --watch-gpio gpioc:2` → `result.json` `logic_edges` |
| assert on trace events from a script | 2. host | Python SDK `Sim.frames()` |
| add tracing/coverage instrumentation | 3. Rust | `impl SimulationObserver` |
| give a model the clock or a tick seam | 3. Rust | `impl Peripheral` (`attach_cycle_clock`, `tick`) |

Everything quoted below was run from this checkout on 2026-10-18. Nothing is
invented output. Build the CLI first (`cargo build -p labwired-cli`; the
commands use `./target/debug/labwired`) and run from the labwired-core
checkout root — wall-seconds in receipt tails vary between runs.

## 1. Device data hooks: `behavior:` in a part pack

### What fires

A `gpio_device` descriptor is serviced in two places, and the descriptor
chooses neither explicitly — the engine reads its shape:

- **Pin edges** (`rules:` with `on: {pin: …, edge: …}`) run synchronously in
  the bus's MMIO write path through
  `maybe_service_edge_driven_gpio_devices` (`crates/core/src/bus/device_hooks.rs:335`),
  so a store followed by another store in the same tick is still two events.
  The rule model is documented under
  [Edge-driven `gpio_device` parts](../../part-packs.md#edge-driven-gpio_device-parts).
- **Scheduled waveforms** (`schedules:`) expose the next deadline through
  `next_resident_edge_deadline_cycle` (`crates/core/src/bus/device_hooks.rs:11`)
  and are serviced at instruction boundaries by
  `service_resident_scheduled_edges` (`crates/core/src/bus/device_hooks.rs:27`).
  With `timing: exact` they fire even when peripheral ticks are coarser than
  the pulses; see [GPIO finite edge schedules](../../part-packs.md#gpio-finite-edge-schedules).

### Where it is configured and implemented

`type: dht22` resolves to the committed part pack
[configs/devices/dht22.yaml](../../../configs/devices/dht22.yaml) via the
embedded registry (`crates/config/src/embedded.rs:95`). The YAML parses into
`DeviceBehavior` — `pin_sampling` at `crates/config/src/peripherals.rs:2426`,
`schedules` at `crates/config/src/peripherals.rs:2429`. At runtime the
declarative GPIO interpreter keeps the schedule table
(`crates/core/src/peripherals/components/declarative_gpio.rs:68`) and drains
`Emit`/`Cancel` requests at `crates/core/src/peripherals/components/declarative_gpio.rs:333`.
The hand-written Rust DHT model still exists, but only as a waveform parity
oracle — production attachment uses the YAML.

### Worked example: a DHT22 response latched on a qualified release

The whole device is data — there is no Rust callback anywhere in the response
path. The relevant parts of `configs/devices/dht22.yaml`:

```yaml
behavior:                                # line 3
  primitive: gpio_device
  pin_sampling: {DATA: open_drain_release}   # line 11
  schedules:                             # line 15
    response:
      output: DATA
      idle: true
      final: true
      timing: exact
      segments:
        - hold: {level: true, us: 30}    # host has released, part not yet low
        - hold: {level: false, us: 80}   # response low
        - hold: {level: true, us: 80}    # response high
        - bits:                          # 40 data bits from var(frame)
            value: "var(frame)"
            count: 40
            order: msb_first
            zero: [{level: false, us: 50}, {level: true, us: 27}]
            one:  [{level: false, us: 50}, {level: true, us: 70}]
        - hold: {level: false, us: 50}   # trailing low
  rules:                                 # line 32
    - on: {pin: DATA, edge: falling}
      do: [{cancel_schedule: response}]
    - on: {pin: DATA, edge: rising}
      min_hold_us: 1000                  # only a real host release qualifies
      do:
        - {var: h, value: "input(humidity)"}
        - {var: t, value: "abs(input(temperature)) | (input_negative(temperature) << 15)"}
        - {var: frame, value: "(var(h) << 24) | (var(t) << 8) | (((var(h) >> 8) + (var(h) & 255) + (var(t) >> 8) + (var(t) & 255)) & 255)"}
        - {emit_schedule: response}
```

Reading it as a hook: the MCU pulls DATA low for >1 ms and releases it
(`open_drain_release` sampling makes the release a rising edge). The qualified
rising rule latches both stimulus channels and the DHT checksum into
`var(frame)`, then arms the finite schedule. Every segment and bit is emitted
by the engine; a falling edge at any point cancels the rest. Nothing in the
device is a callback into Rust.

### Receipt 1: drive it from the CLI with real firmware

`crates/core/tests/fixtures/gpio-schedules/dht-frame.elf` is a busy-poll
firmware: it releases PC2, then measures all 83 transitions of the response and
stores the count at `0x20000100`. Two small files attach the pack to an
STM32L476 and assert the frame completed:

```yaml
# dht22-system.yaml — scratch file at the checkout root
name: "dht22-cli-receipt"
chip: "configs/chips/stm32l476.yaml"
external_devices:
  - id: sensor
    type: dht22
    connection: gpio
    config:
      data_pin: PC2
      cpu_hz: 80000000
```

```yaml
# dht22-test.yaml — scratch file at the checkout root
schema_version: "1.0"
inputs:
  firmware: "crates/core/tests/fixtures/gpio-schedules/dht-frame.elf"
  system: "dht22-system.yaml"
limits:
  max_steps: 1000000
assertions:
  - memory_value:
      address: 0x20000100
      expected_value: 83
      size: 32
```

Both YAMLs are scratch: create them at the checkout root — there the
manifest's `chip:` resolves (manifest-relative) and the script's `inputs`
resolve (script-relative), so no absolute path appears — then delete them
afterwards.

```bash
./target/debug/labwired test -c dht22-test.yaml \
  --output-dir /tmp/hook-surfaces/dht-out
```

Real output (tail):

```text
labwired-cli test: idle_ff=on (event_scheduler=on)
PASS  1/1 checks · dht22-test · 1000000 steps · 2.57s
```

`/tmp/hook-surfaces/dht-out/result.json` (verbatim excerpts):

```json
  "status": "pass",
  "stop_reason": "max_steps",
```

```json
  "assertions": [
    {
      "assertion": {
        "memory_value": {
          "address": 536871168,
          "expected_value": 83,
          "mask": null,
          "size": 32
        }
      },
      "passed": true
    }
  ],
```

`83` means the firmware saw the complete frame — 83 pulse transitions on the
wire, not an inferred register value. The test-script assertion surface is
documented in the [CI Test Runner](../../ci_test_runner.md); its `memory_value`
example is the environment (node-qualified) shape, while a single-machine
script uses the unqualified form above, as this run shows.

### Receipt 2: the committed schedule-parity test

The in-tree test pins the decoded bytes, not just the transition count — four
temperature/humidity seeds, exact and batched stepping, 5 ms peripheral ticks:

```bash
cargo test -p labwired-core --test gpio_schedule_firmware busy_polled_dht
```

```text
running 1 test
test busy_polled_dht_frames_preserve_every_interval_with_coarse_ticks ... ok

test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 1 filtered out; finished in 2.34s
```

Its expected bytes for 22.0 °C / 50 % are `[1, 244, 0, 220, 209]`
(`crates/core/tests/gpio_schedule_firmware.rs:46`), with the raw pulse-width
array decoded at `crates/core/tests/gpio_schedule_firmware.rs:65-72`.

## 2. Host observation: bus trace, console stop, logic capture, Python frames

### `--bus-trace-out` — the wire record, VCD or JSON

**What fires.** Each transacted byte is pushed as a `BusTraceEvent`
(`crates/core/src/bus/bus_trace.rs:154`) into a shared ring of 4096 events
(`BUS_TRACE_LIMIT`, `crates/core/src/bus/bus_trace.rs:37`; ring at
`crates/core/src/bus/bus_trace.rs:166`). The payload is I²C address/data with
ack/nack, SPI `mosi`/`miso`, or a UART `tx`/`rx` octet — the `Display` impl is
the human form (`crates/core/src/bus/bus_trace.rs:101-151`). CAN frames are
kept in the JSON (lossless) but deliberately omitted from the VCD: a CAN frame
is not an 8-bit wire sample, and a never-written `wire 8` channel would read as
an idle bus (`crates/cli/tests/bus_trace_export.rs:69-119`).

**Where configured.** `--bus-trace-out <path>` on `labwired run`
(`crates/cli/src/lib.rs:381`). Export happens at end of run in
`export_bus_trace_if_requested` (`crates/cli/src/commands/run.rs:130`); a
`.json` extension writes the raw event list, anything else writes VCD
(`crates/cli/src/commands/run.rs:145-154`). The VCD stamps each event at its
**sequence number** with a 1 µs timescale
(`crates/cli/src/bus_vcd.rs:90-96`), while the JSON keeps the real engine
`cycle`.

**Receipt — the C3 blinky, UART only.** 57 octets, VCD and JSON in one run
each:

```bash
./target/debug/labwired run --chip configs/chips/esp32c3.yaml \
  --firmware examples/esp32c3-blinky/firmware/esp32c3_blinky.elf \
  --max-steps 800000 --bus-trace-out /tmp/hook-surfaces/blinky.vcd
```

```text
LED ON
LED OFF
LED ON
LED Olabwired-cli run: bus trace (57 events) -> "/tmp/hook-surfaces/blinky.vcd"
```

The emitted VCD (first events) declares the UART bus and encodes its octets:

```text
$timescale 1 us $end
$scope module bus_trace $end
$var wire 8 ! uart0 $end
$upscope $end
$enddefinitions $end
#1
b01000011 !
#2
b00110011 !
```

`b01000011` is `0x43`, the `C` that starts the firmware's banner. The same run
with `--bus-trace-out /tmp/hook-surfaces/blinky.json` writes the lossless form:

```json
[
  {
    "seq": 1,
    "cycle": 13312,
    "bus": "uart0",
    "payload": {
      "protocol": "uart",
      "direction": "tx",
      "byte": 67
    }
  },
```

Note that the flag's help calls this the "universal I²C/SPI bus trace"; UART
octets are recorded too, as the receipt shows.

### `--stop-on <text>` — stop on the firmware's own console

**What fires.** The run mirrors the firmware console into a buffer and scans it
every 100,000 steps; the first match logs the step and breaks the stepping loop
(`crates/cli/src/commands/run.rs:1392-1402`). It needs the USB-Serial-JTAG
console, so it is **ESP32-S3 only** (`crates/cli/src/lib.rs:342-347`).

**Receipt — the committed S3 source-debug fixture.** This Arduino sketch prints
a growing number over USB CDC; `81` is the fifth `loop()` print:

```bash
LABWIRED_ESP32S3_FLASH=tests/fixtures/source-debug/esp32s3-arduino-flash.bin \
  ./target/debug/labwired run --chip configs/chips/esp32s3.yaml \
  --firmware tests/fixtures/source-debug/esp32s3-arduino.elf \
  --rom-boot --max-steps 200000000 --stop-on 81
```

Real output (tail; the `81` is the firmware line):

```text
81labwired-cli run: --stop-on "81" matched at step 101700000
```

The run ended at the matched step and printed no `reached --max-steps`
epilogue: that line is emitted only when the stepping loop actually exhausts
the budget (`crates/cli/src/commands/run.rs:1488`). Pick a marker that does
not occur in boot output: `--stop-on 27` matches inside the ROM banner
`ESP-ROM:esp32s3-20210327` at step 100000.

### Logic capture — `--watch-gpio` (test) and `--gpio-trace` (run)

**What fires.** Both drain the same in-engine GPIO tap the browser logic
analyzer reads. On `labwired test`, `--watch-gpio <peripheral:pin>`
(repeatable — `CH0`, `CH1`, … in argument order) records each watched pad's
edge series into `result.json`'s `logic_edges` block: per channel,
`peripheral`, `pin`, `initial`, the `transitions` (`cycle`, `value`), and
`gaps`/`dropped` bookkeeping. On `labwired run`, `--gpio-trace <path>` writes
one JSON line per transition (`{"sim_cycle":N,"pin":P,"from":B,"to":B}` with
the default `--gpio-trace-format bool`; `4state` adds `z`/`x` states and
drive-change lines). The `run` flag's JSON observer is installed on the
ESP32-S3 path; a chip-only fast-boot run with no board I/O wiring (the C3
blinky above, say) writes no trace file.

**Receipt — the same DHT22 script, with the sensor's data pad watched.**
Adding `--watch-gpio gpioc:2` to the receipt above:

```bash
./target/debug/labwired test -c dht22-test.yaml --no-uart-stdout \
  --watch-gpio gpioc:2 --output-dir /tmp/hook-surfaces/dht-watch-out
```

```text
labwired-cli test: idle_ff=on (event_scheduler=on)
PASS  1/1 checks · dht22-test · 1000000 steps · 1.68s
```

`result.json` (verbatim excerpt — 87 transitions in total, first two shown):

```json
  "logic_edges": {
    "dropped": 0,
    "now_cycle": 1000000,
    "channels": [
      {
        "ch": 0,
        "channel": "CH0",
        "peripheral": "gpioc",
        "pin": 2,
        "initial": 0,
        "transitions": [
          {
            "cycle": 7,
            "value": 1
          },
          {
            "cycle": 13,
            "value": 0
          },
```

**Receipt — `--gpio-trace` on the S3 fixture.** The `--stop-on` receipt's
command plus `--gpio-trace`:

```bash
LABWIRED_ESP32S3_FLASH=tests/fixtures/source-debug/esp32s3-arduino-flash.bin \
  ./target/debug/labwired run --chip configs/chips/esp32s3.yaml \
  --firmware tests/fixtures/source-debug/esp32s3-arduino.elf \
  --rom-boot --max-steps 200000000 --stop-on 81 \
  --gpio-trace /tmp/hook-surfaces/s3-stop.jsonl
```

The run wrote 10 transitions. First two and last lines of
`/tmp/hook-surfaces/s3-stop.jsonl`:

```json
{"sim_cycle":5539549,"pin":2,"from":false,"to":true}
{"sim_cycle":5543815,"pin":2,"from":true,"to":false}
...
{"sim_cycle":101624699,"pin":2,"from":true,"to":false}
```

The last line's cycle matches the `GPIO2: 1->0 (cycle=101624699)` log line the
same run prints just before the `--stop-on` match — the trace, the log and the
console stop agree on when the pad dropped.

### `Sim::frames()` — protocol events from Python

The Python SDK exposes the same trace ring as a drained list of dictionaries
(`seq`, `cycle`, `at`, `bus`, `summary`, the native `payload`), so a script can
assert on events without parsing a VCD. Implementation:
`crates/python/python/labwired/__init__.py:224`; usage and field list:
[Python SDK](../../python-sdk.md) (`frames()` at line 127).

This page has no Python receipt because the `labwired` Python package is not
installed in this checkout; the C and Rust surfaces above are the executed
proof. The callback-style alternative for Python-driven co-simulation models is
[Co-Simulation Plugins](../../cosimulation_plugins.md).

## 3. Rust extension points

### `SimulationObserver` — observe, don't modify

The trait is `crates/core/src/lib.rs:252`. All callbacks have default no-op
bodies: `on_simulation_start`/`on_simulation_stop`, `on_trace_event`,
`on_step_start`/`on_step_end`, `on_memory_write`, `on_peripheral_tick`, and
`on_core_switch` (`crates/core/src/lib.rs:253-264`). Register through
`Machine::add_observer`, which mirrors the observer onto the bus
(`crates/core/src/lib.rs:3374`), or on the bus alone through
`SystemBus::add_observer` (`crates/core/src/bus/accessors.rs:21`).

In-tree consumers to copy from:

- `VcdObserver` — the `--vcd` writer (`crates/cli/src/vcd_trace.rs:64`);
- `TraceObserver` — the instruction trace ring (`crates/core/src/trace.rs:72`);
- `PcCoverageObserver` — firmware coverage (`crates/core/src/pc_coverage.rs:227`).

**Receipt — delivery to a bus observer and to a machine-registered observer:**

```bash
cargo test -p labwired-core --test observer_store_visibility
```

```text
running 7 tests
test machine_registered_observer_sees_bus_stores ... ok
test machine_registration_does_not_evict_a_bus_only_observer ... ok
test u16_peripheral_store_publishes_two_le_bytes_and_no_phantom_bytes ... ok
test u32_peripheral_store_publishes_four_le_bytes ... ok
test rejected_store_publishes_nothing ... ok
test observer_in_both_lists_sees_each_store_once ... ok
test wide_and_narrow_stores_agree_byte_for_byte ... ok

test result: ok. 7 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s
```

The test also pins the two registration traps that used to silently produce an
empty trace: wide MMIO stores must publish one event per byte (no phantom
bytes), and registering on the `Machine` must not evict a bus-only observer
(`crates/core/tests/observer_store_visibility.rs:59-184`).

### `Peripheral` — attach points and the tick seam

The model trait is `crates/core/src/lib.rs:980`. The seams a hook typically
implements:

| Method | Location | Contract |
|---|---|---|
| `attach_cycle_clock` | `crates/core/src/lib.rs:2247` | default no-op; called once by `SystemBus::add_peripheral` (`crates/core/src/bus/construct.rs:285`) so `&self` reads can lazily sync counters to "now" |
| `attach_irq_line` | `crates/core/src/lib.rs:2264` | default no-op; same attach choke point, tells the model its NVIC line |
| `tick` | `crates/core/src/lib.rs:1079` | periodic peripheral tick |
| `tick_elapsed` | `crates/core/src/lib.rs:1086` | tick aware of elapsed cycles |
| `tick_with_bus` | `crates/core/src/lib.rs:1604` | tick with bus access |
| `uses_scheduler` | `crates/core/src/lib.rs:2134` | opt into the event scheduler instead of legacy polling |

The default implementations are load-bearing: a hand-built bus that bypasses
`add_peripheral` must keep legacy semantics, which is why the clock attach is
opt-in and why dropping it is a ratchet failure
(`crates/core/tests/tick_interval_inventory.rs:676`). The full attach order in
`add_peripheral` is clock, IRQ line, `cpu_hz`, then bus trace
(`crates/core/src/bus/construct.rs:285-289`).

## Where each is documented

| Surface | Reference |
|---|---|
| `behavior.rules`, `behavior.schedules`, DHT/HC pulse semantics | [Part packs — edge-driven parts](../../part-packs.md#edge-driven-gpio_device-parts) and [GPIO finite edge schedules](../../part-packs.md#gpio-finite-edge-schedules) |
| Register maps for declarative peripherals | [Declarative Registers](../../declarative_registers.md) |
| `Sim.frames()` and the rest of the Python API | [Python SDK](../../python-sdk.md) |
| Co-simulation models and plugin seams | [Co-Simulation Plugins](../../cosimulation_plugins.md) |
| CLI flags (`--bus-trace-out`, `--stop-on`, `--gpio-trace`, `--watch-gpio`; `labwired run --help` is authoritative for new flags) | [CLI Reference](../../cli_reference.md) |
| Test scripts (`memory_value`, `uart_contains`, limits) | [CI Test Runner](../../ci_test_runner.md) |
