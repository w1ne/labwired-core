---
description: "A board on LabWired is two YAML documents of data — a chip descriptor for the silicon and a system manifest for the board — plus the memory-region cpu_writes policy that turns a rogue CPU store into a named BusFault."
---

# Add a board (chip + system manifest)

"What does it take to get my own board running?" On LabWired the answer is
**data: two YAML documents and a test script**. There is no platform class, no
board file in C#, nothing to compile into the simulator:

- the **chip descriptor** says what the *silicon* is — core, clock, flash and
  RAM regions, the peripheral windows the bus maps;
- the **system manifest** says what the *board* is — which chip it carries,
  which external devices sit on which buses, and `board_io`: which pin a user
  LED or button is soldered to.

[Add a peripheral block (YAML)][add-peripheral] adds a register block *inside* a
chip and [Add an I²C sensor (data + Python)][python-device] wires a part to a
bus. This page goes one level up: a whole minimal Cortex-M4 board, built and
proved with `labwired test` — and then the feature that makes `flash:` and
`ram:` first-class memory regions: **`cpu_writes`**, the per-region CPU-write
policy whose `fault` mode turns a store into read-only flash into a precise
BusFault that names the region.

[add-peripheral]: ./add-a-peripheral.md
[python-device]: ./python-device.md

Everything below ran from a source checkout; the board lives in
`/tmp/add-a-board/`. Nothing on the page is invented output.

## 1. The chip descriptor — what the silicon is

`chip.yaml`, minimal but complete, every key annotated. Structure follows the
in-tree fixture [configs/chips/ci-fixture-stm32l476-readonly.yaml][l476-fixture]
(and the C8 in [configs/chips/stm32f103.yaml][f103]) — cut down to one GPIO
port and one UART:

[l476-fixture]: ../../../configs/chips/ci-fixture-stm32l476-readonly.yaml
[f103]: ../../../configs/chips/stm32f103.yaml

```yaml
# /tmp/add-a-board/chip.yaml — the SILICON as data. No board facts here.
name: "my-board"        # free-form label for the chip
arch: "arm"             # instruction-set family: arm | riscv | xtensa | avr
core: "cortex-m4"       # the exact core; gates core-specific bus behavior
                        # (e.g. bit-band aliasing exists only on M3/M4)
cpu_hz: 80_000_000      # the clock THIS chip runs at, Hz — declared, not
                        # derived from a clock tree (see section 5)

flash:
  base: 0x08000000      # start of the flash window; the boot alias mirrors it
                        # at address 0x0 (which is where reset fetches vectors)
  size: "512KB"         # 512 x 1024 — KB is binary; MB is 10^6, so spell
                        # binary-megabyte sizes in KB ("1024KB" = 1 MiB)
ram:
  base: 0x20000000
  size: "128KB"

peripherals:
  - id: "gpioa"                 # the name board_io and firmware configs cite
    type: "gpio"                # a model the engine already has, by name
    base_address: 0x48000000    # window base the bus maps
    size: "1KB"                 # window length
    config:
      profile: "stm32v2"        # a bare `gpio` MUST name its register layout;
                                # the engine never guesses one
  - id: "uart1"
    type: "uart"
    base_address: 0x4000C000
    size: "1KB"
    # No `irq:` — a polled console needs no vector. A real board would wire
    # the UART's interrupt line; `irq:` is an optional key on the peripheral.
```

Notes that save a round trip:

- `arch:` and `core:` are two fields because they answer different questions:
  `arch` collapses a family (`arm` covers the whole Cortex-M range), while
  `core` names the exact part and gates behavior that is per-core rather than
  per-family (bit-band aliasing on M3/M4, `core: "cortex-m33"` elsewhere in
  the tree).
- The `peripherals:` entries are *windows*, not models: `type:` selects a
  register map the engine implements, `base_address`/`size` say where in the
  address space it answers. `config:` carries per-instance data — for a bare
  `type: "gpio"`, the mandatory `profile`.
- A chip file has no `board_io:`. Which pin an LED is soldered to is a board
  fact, and board facts live in the manifest.
- `size:` strings take a bare byte count, `0x…`, or a unit: `KB` and `KiB` are
  1024 bytes, `MiB` is 2^20, but **`MB` is 10^6** — it is the long-standing
  reason 1 MiB of L476 flash is written `"1024KB"` rather than `"1MB"`.

## 2. The system manifest — what the board is

`system.yaml` instantiates the chip and records the board wiring. The LED
binding is the same schema as the NUCLEO-L476RG's LD2 in
[configs/systems/nucleo-l476rg.yaml][l476-system]:

[l476-system]: ../../../configs/systems/nucleo-l476rg.yaml

```yaml
# /tmp/add-a-board/system.yaml — the BOARD as data.
name: "my-board"
chip: "chip.yaml"       # path resolved relative to THIS file (below)
external_devices: []    # parts on I²C/SPI/UART buses; none on this board
board_io:               # board-level pin bindings
  - id: "led_ld2"       # a user LED ...
    kind: "led"
    peripheral: "gpioa" # ... on a peripheral id from the chip file ...
    pin: 5              # ... at PA5
    signal: "output"
    active_high: true   # lit when the pin reads high
```

How the `chip:` path resolves — the rule, not a guess:

- **A bare word** (`stm32f103`) is a *built-in chip name*, resolved from the
  engine's embedded descriptor registry (or a loaded plugin), never a file.
  A spec counts as a path if it contains `/` or `\`, or ends in
  `.yaml`/`.yml`/`.json`.
- **A path** like `chip.yaml` or `../chips/stm32l476.yaml` is joined to
  **the directory of the manifest file**, not your shell's cwd
  (`ResolvedSystem::from_manifest_file` → `ChipDescriptor::resolve_with`).
  So a manifest is relocatable together with everything it names.

`board_io:` is what turns a bare SoC into a board: the browser's board view
reads the bound GPIO pin's output level through `led_ld2` to light the LED; a
`button` binding with `signal: input` is additionally materialised as a
drivable stimulus. A plain `labwired test` run does not need the bindings —
but the LED line is the difference between "a chip" and "a board".

## 3. Run it

The script pins the board to a firmware image and states what must be true at
the end. Paths inside a script (`inputs.firmware`, `inputs.system`) resolve
**relative to the script file**, so the test directory is self-contained:

```yaml
# /tmp/add-a-board/script.yaml
schema_version: "1.0"
inputs:
  firmware: "firmware.elf"   # relative to this script
  system: "system.yaml"
limits:
  max_steps: 100000
assertions:
  - uart_contains: "OK"
  - expected_stop_reason: max_steps
```

For the firmware, use a committed fixture for this core — the same
[`tests/fixtures/uart-ok-thumbv7m.elf`][fixture] the CI scripts use
([examples/ci/dummy-max-uart-bytes.yaml][ci]). It is a Thumb v7-M (Cortex-M3)
image, but the v7-M instruction subset executes on the M4 core, and the loader
maps its segments through the chip's boot alias (vectors at `0x0` mirror into
flash at `0x08000000`), so it loads and runs unchanged (run from the checkout
root):

[fixture]: ../../../tests/fixtures/uart-ok-thumbv7m.elf
[ci]: ../../../examples/ci/dummy-max-uart-bytes.yaml

```bash
cp tests/fixtures/uart-ok-thumbv7m.elf /tmp/add-a-board/firmware.elf
cargo run -p labwired-cli -- test --script /tmp/add-a-board/script.yaml \
  --output-dir /tmp/add-a-board/out
```

Real output:

```text
2026-10-18T15:14:34.326220Z  INFO labwired_core::system::builder: Loading chip descriptor: chip.yaml
2026-10-18T15:14:34.327492Z  INFO labwired_loader: ELF Entry Point: 0x401
labwired-cli test: idle_ff=on (event_scheduler=on)
labwired-cli test: hosted fast path (exact windows up to 1024 cycles)
OK
PASS  2/2 checks · script · 100000 steps · 0.00s
```

The `OK` line is the firmware itself: it stores `'O'`, `'K'`, `'\n'` to the
UART's data register at `0x4000C000`, and the test's `uart_contains` assertion
reads the captured console. The board boots, prints, and passes — no firmware
build in this walkthrough, just the committed fixture. The test-script schema
(limits, assertions, `max_uart_bytes` and friends) is in the
[CI Test Runner][ci-runner].

[ci-runner]: ../../ci_test_runner.md

## 4. Memory-region policy: `cpu_writes`

A firmware bug that scribbles over its own flash is one of the nastiest things
to debug on hardware: on many parts the store silently commits and the crash
lands somewhere else entirely. LabWired makes the region's policy explicit
data. Both `flash:` and `ram:` accept:

```yaml
flash:
  base: 0x08000000
  size: "512KB"
  cpu_writes: allow   # allow (default) | warn | fault
ram:
  base: 0x20000000
  size: "128KB"
  cpu_writes: allow
```

| Mode | A CPU store into the region… | Recorded in `result.json` |
|---|---|---|
| `allow` (default) | commits silently | this region records nothing; the whole block is absent only when both regions are `allow` |
| `warn` | commits; one deduplicated log line | every violation counted, first kept |
| `fault` | does not commit; the core takes a data fault | every violation counted, first kept |

The gate applies to CPU stores only (never loader writes, snapshot restores,
or the generic DMA path). `fault` refuses the store and, on ARMv7-M with
BusFault enabled, surfaces as a precise data BusFault with `BFAR` valid; `warn`
commits but logs one deduplicated WARN. Every non-`allow` region records each
violation and the first `{addr, cycle, pc}`, and `result.json` carries the
block only when at least one region is non-`allow` — the board built above is
all-default and has no such key. The authoritative rule set, including the
boot-alias corner and the image-linked-at-zero caveat, is
[Configuration → `cpu_writes`](../../configuration_reference.md#cpu_writes-the-memory-region-store-policy).

### The receipt: a store into `cpu_writes: fault` flash

The in-tree example [examples/readonly-flash/][readonly-flash] runs a tiny
firmware that stores to `0x0800_1000`, inside flash, against the L476 fixture
chip whose flash declares `cpu_writes: fault`. The command, run in this
checkout, and its real output:

[readonly-flash]: ../../../examples/readonly-flash/README.md

```bash
cargo run -p labwired-cli -- test --system examples/readonly-flash/system.yaml \
  --script examples/readonly-flash/script.yaml \
  --output-dir /tmp/labwired-readonly-flash
```

```text
2026-10-18T15:14:34.606073Z  INFO labwired_core::system::builder: Loading chip descriptor: ../../configs/chips/ci-fixture-stm32l476-readonly.yaml
2026-10-18T15:14:34.646398Z  INFO labwired_loader: ELF Entry Point: 0x8000040
labwired-cli test: idle_ff=on (event_scheduler=on)
labwired-cli test: hosted fast path (exact windows up to 1024 cycles)
PASS  1/1 checks · script · 5000 steps · 0.00s
FAULT  Precise BusFault: data access at 0x0800_1000 (BFAR valid; store to read-only flash), at PC 0x0800_006C in `sensor_read` (fault_fixture.c:61), called from `main`.
       HFSR=0x00000000 CFSR=0x00008200 [PRECISERR] BFAR=0x08001000
```

The verdict names the region, the address, and the source line — the store
never committed. `/tmp/labwired-readonly-flash/result.json` carries the block
(a verbatim excerpt):

```json
  "memory_policy": {
    "flash": {
      "mode": "fault",
      "violations": 1,
      "first": {
        "addr": 134221824,
        "cycle": 7,
        "pc": 134217870
      }
    },
    "ram": {
      "mode": "allow",
      "violations": 0
    }
  }
```

`violations: 1` is the firmware's own store at cycle 7 — the *load* of the same
image wrote flash directly and was not counted, which is the "CPU stores only"
rule in one number.

Flip `cpu_writes` to `warn` on the walkthrough board and re-run the same
store: make `chip-warn.yaml` = `chip.yaml` with `cpu_writes: warn` on its
`flash:`, and `system-warn.yaml` = `system.yaml` with
`chip: "chip-warn.yaml"`. The new script — identical to `script.yaml` except
for the firmware and manifest — is:

```yaml
# /tmp/add-a-board/script-warn.yaml
schema_version: "1.0"
inputs:
  firmware: "fault-write.elf"   # relative to this script
  system: "system-warn.yaml"
limits:
  max_steps: 5000
assertions:
  - expected_stop_reason: max_steps
```

The store-to-flash ELF is the receipt's own firmware,
`tests/fixtures/fault-verdict/fault-readonly-flash.elf` (linked at
`0x08000000`), copied next to the script:

```bash
cp tests/fixtures/fault-verdict/fault-readonly-flash.elf \
  /tmp/add-a-board/fault-write.elf
cargo run -p labwired-cli -- test --script /tmp/add-a-board/script-warn.yaml \
  --output-dir /tmp/add-a-board/out-warn
```

Real output (the WARN line's timestamp is RFC 3339 wall-clock; wall-seconds in
the PASS line vary between runs):

```text
2026-10-18T18:41:56.727911Z  WARN labwired_core::bus::memory_policy: CPU store into flash with cpu_writes: warn — store committed addr=0x08001000 pc=0x0800008E
PASS  1/1 checks · script-warn · 5000 steps · 0.00s
```

and the recorded first violation is unchanged (`"mode": "warn", "violations":
1`, `addr` `0x08001000`, cycle 7) — committing is not forgetting.

## 5. What this board does not include

- **Device models.** `type: "gpio"` and `type: "uart"` are models the engine
  ships; a part like a sensor is a *device* you declare in `external_devices:`
  and model as data — see [Add an I²C sensor (data + Python)][python-device].
- **Peripheral register blocks.** A register block the engine does not know is
  its own YAML attached to the chip with `type: "declarative"` — see
  [Add a peripheral block (YAML)][add-peripheral].
- **Clock trees.** `cpu_hz:` is *declared*, not derived: nothing walks RCC/PLL
  registers to compute a clock. The chip file states the rate (and a board
  that runs the part slower overrides it with `cpu_hz:` in the manifest — the
  NUCLEO-L476RG declares 4 MHz because its firmware never leaves the MSI reset
  clock). Self-timed device models scale by it, so the number must agree with
  what the firmware was built against.
- **Firmware builds.** This walkthrough reuses a committed ELF; for a real
  board bring-up — smoke firmware, validation trail, board docs — follow the
  [board onboarding playbook](../../board_onboarding_playbook.md).

## Where to go for more

| Topic | Link |
|---|---|
| Full schema reference (chip, manifest, peripherals) | [Configuration (YAML)](../../configuration_reference.md) |
| End-to-end onboarding process for a real board | [Board onboarding playbook](../../board_onboarding_playbook.md) |
| The gold-reference board, page by page | [NUCLEO-L476RG](../../boards/nucleo-l476rg.md) |
| The read-only-flash example and its README | [examples/readonly-flash][readonly-flash] |
| Test scripts | [CI Test Runner][ci-runner] |
