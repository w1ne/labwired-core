---
description: "Add a missing memory-mapped peripheral register block to a chip model in YAML — no C# or Rust required."
---

# Add a peripheral block (YAML)

"My firmware talks to a register block the model does not have — how do I add
it?" Renode issue [#303][renode-303] asks exactly that about an EFM32GG
flash controller. On LabWired, a block whose behaviour is a register map plus
timed bit changes is **data**: one YAML descriptor listing registers, offsets,
access modes and fields, plus `timing:` entries for state that moves on its
own. No peripheral class, no C# project, no Rust. This page adds a missing
MSC-class flash controller to a chip, end to end, and proves it with
`labwired test`.

[renode-303]: https://github.com/renode/renode/issues/303

The mechanism is already load-bearing in this repo: the ESP32-C3 register wall
is 26 `type: declarative` peripherals in [configs/chips/esp32c3.yaml][c3], and
the field-level reference is [Declarative Registers][declarative-registers].

[c3]: ../../../configs/chips/esp32c3.yaml

## 1. Read the datasheet into a register table

The worked example is a reduced EFM32/EFR32-family **MSC** (Memory System
Controller), the flash-controller block from the issue above. Six registers,
byte offsets from the block base (`0x40030000` on EFR32MG26):

| Register | Offset | Access | Bits used here |
|---|---|---|---|
| `MSC_CMD` | `0x000` | write-only | `START[0]` — writing 1 starts the addressed operation |
| `MSC_CTRL` | `0x004` | R/W | — (storage) |
| `MSC_STATUS` | `0x008` | read-only | `BUSY[0]` running; `DONE[1]` finished |
| `MSC_ADDR` | `0x00C` | R/W | — (storage) |
| `MSC_WDATA` | `0x010` | write-only | — (storage) |
| `MSC_RDATA` | `0x014` | read-only | — (storage) |

The blocked piece of firmware is the wait loop: it writes `MSC_CMD = START`,
then polls `MSC_STATUS.BUSY`/`DONE`. That transition is modelled with two
`timing:` entries — no code:

| Trigger | Delay | Action |
|---|---|---|
| write `MSC_CMD` = 1 | 0 peripheral ticks | `set_bits` `MSC_STATUS.BUSY` |
| write `MSC_CMD` = 1 | 64 peripheral ticks | `set_bits` `MSC_STATUS.DONE`, `clear_bits` `MSC_STATUS.BUSY` |

`START` is visible on the next peripheral tick and clears 64 ticks later. With
the default `peripheral_tick_interval: 1` that is 64 CPU cycles — a
tick-quantised delay, not a datasheet figure; changing `delay_cycles` is the
one knob.

## 2. Write the block file

`msc.yaml` in full — copy-paste ready:

```yaml
# EFR32MG26 MSC (Memory System Controller), reduced to the six registers this
# walkthrough needs. Sources: EFR32xG26 Reference Manual rev 1.0 section 6.7
# (register map) and section 6.8 (fields). Offsets are byte offsets from the
# block base, 0x40030000 on this part.
peripheral: "MSC"
version: "1.0"
registers:
  - id: "MSC_CMD"
    address_offset: 0x000
    size: 32
    access: "WRITE_ONLY"
    reset_value: 0x00000000
    fields:
      - name: "START"
        bit_range: [0, 0]
        description: "Write 1 to start the addressed operation."

  - id: "MSC_CTRL"
    address_offset: 0x004
    size: 32
    access: "READ_WRITE"
    reset_value: 0x00000000

  - id: "MSC_STATUS"
    address_offset: 0x008
    size: 32
    access: "READ_ONLY"
    reset_value: 0x00000000
    fields:
      - name: "BUSY"
        bit_range: [0, 0]
        description: "Set while a started command is running."
      - name: "DONE"
        bit_range: [1, 1]
        description: "Set when the command has finished."

  - id: "MSC_ADDR"
    address_offset: 0x00C
    size: 32
    access: "READ_WRITE"
    reset_value: 0x00000000

  - id: "MSC_WDATA"
    address_offset: 0x010
    size: 32
    access: "WRITE_ONLY"
    reset_value: 0x00000000

  - id: "MSC_RDATA"
    address_offset: 0x014
    size: 32
    access: "READ_ONLY"
    reset_value: 0x00000000

# The state machine, in data. A START write raises BUSY on the next peripheral
# tick; 64 ticks later BUSY drops and DONE rises.
timing:
  - id: "busy"
    trigger: !write { register: "MSC_CMD", value: 1 }
    delay_cycles: 0
    action:
      set_bits: { register: "MSC_STATUS", bits: 0x1 }

  - id: "done"
    trigger: !write { register: "MSC_CMD", value: 1 }
    delay_cycles: 64
    action:
      set_bits: { register: "MSC_STATUS", bits: 0x2 }

  - id: "clear_busy"
    trigger: !write { register: "MSC_CMD", value: 1 }
    delay_cycles: 64
    action:
      clear_bits: { register: "MSC_STATUS", bits: 0x1 }
```

Notes that save a round trip:

- `access:` spellings are `READ_WRITE` / `READ_ONLY` / `WRITE_ONLY` (aliases
  `R/W`, `rw`, `RO`, `r`, `WO`, `w`). Access is enforced: a write to a
  read-only register is dropped, a read of a write-only register returns 0.
- `fields:` uses `bit_range: [msb, lsb]` and is what the inspector decodes;
  it does not mask the stored word.
- The trigger is the **tagged** form `!write { ... }`; serde_yaml 0.9 rejects
  the plain-map spelling of an externally-tagged enum.
- One `timing:` entry applies one bit action. A transition that sets one bit
  and clears another is two entries at the same delay, applied in declaration
  order.

## 3. Attach it to the chip

In the copy of the chip descriptor (here: a copy of
[configs/chips/efr32mg26.yaml][chip] with its native `msc` entry removed, so
the block is genuinely missing again), add to `peripherals:`:

[chip]: ../../../configs/chips/efr32mg26.yaml

```yaml
  - id: "msc"
    type: "declarative"
    clock: { reg: "clken1", bit: 16 }
    base_address: 0x40030000
    size: "4KB"
    config:
      path: "./msc.yaml"
```

- `type: "declarative"` with `config.path` is the attach arm; `path` resolves
  relative to the chip file, so the descriptor travels with it.
- `clock:` is the chip's clock gate, not part of the block: on this part the
  MSC does not answer the bus until `CMU_CLKEN1` bit 16 is set, exactly like
  silicon. Leave the key out if the block is not gated.

## 4. Prove it

Everything below ran from a source checkout with `/tmp/apc/` holding `chip.yaml`,
`msc.yaml`, `firmware.elf`, `system.yaml` and `script.yaml`.

`system.yaml`:

```yaml
name: "apc-msc"
chip: "/tmp/apc/chip.yaml"
external_devices: []
```

A tiny firmware drives the block through its normal MMIO registers — clock the
MSC, start a command, accumulate every `MSC_STATUS` word it reads, publish the
final one. No LabWired-specific API:

```asm
/* Tiny Cortex-M firmware for the add-a-peripheral walkthrough: clock the MSC
 * block, start a command, and publish what MSC_STATUS says. Pure MMIO — no
 * LabWired-specific API. Built with arm-none-eabi-as/ld, linked at the EFR32
 * flash base 0x08000000. */
    .syntax unified
    .cpu cortex-m33
    .thumb

    .section .vectors, "a"
    .word 0x20010000            /* initial SP */
    .word reset + 1
    .rept 14
    .word default_handler + 1
    .endr

    .text
    .global reset
    .thumb_func
reset:
    /* 1. Clock the MSC block: CMU_CLKEN1 (0x40008068), bit 16. Until this
     *    store, every MSC access is dropped by the clock gate. */
    ldr  r0, =0x40008068
    ldr  r1, =0x00010000
    str  r1, [r0]

    /* 2. MSC_CMD = START. */
    ldr  r0, =0x40030000
    movs r1, #1
    str  r1, [r0, #0x00]

    /* 3. Poll MSC_STATUS, accumulating every word seen. */
    ldr  r3, =0x20000000
    movs r5, #0
    ldr  r4, =4000
poll:
    ldr  r2, [r0, #0x08]
    orr  r5, r5, r2
    subs r4, r4, #1
    bne  poll

    /* 4. Publish the accumulator and the final status word. */
    str  r5, [r3, #0x08]        /* OR of every MSC_STATUS read */
    ldr  r2, [r0, #0x08]
    str  r2, [r3, #0x04]        /* MSC_STATUS at the end */
park:
    b    park

    .thumb_func
default_handler:
    b    default_handler

    .ltorg
```

```bash
arm-none-eabi-as -o firmware.o firmware.S
arm-none-eabi-ld -nostdlib -e reset --build-id=none \
  -Ttext=0x08000040 --section-start=.vectors=0x08000000 \
  -o firmware.elf firmware.o
arm-none-eabi-strip firmware.elf
```

`script.yaml` asserts the two RAM words the firmware published. A
single-machine `memory_value` assertion reads through the bus, so the same
assertion can also read a register (or any memory-mapped word) directly:

```yaml
schema_version: "1.0"
inputs:
  firmware: "/tmp/apc/firmware.elf"
  system: "/tmp/apc/system.yaml"
limits:
  max_steps: 100000
assertions:
  # OR of every MSC_STATUS word the firmware read while polling: both BUSY
  # (bit 0) and DONE (bit 1) must have been observed.
  - memory_value:
      address: 0x20000008
      expected_value: 0x3
      mask: 0x3
      size: 32
  # The last MSC_STATUS read: BUSY clear, DONE set.
  - memory_value:
      address: 0x20000004
      expected_value: 0x2
      mask: 0x3
      size: 32
  - expected_stop_reason: max_steps
```

Run it (with an installed CLI, `labwired test --script /tmp/apc/script.yaml
--no-uart-stdout`):

```bash
cargo run -p labwired-cli -- test --script /tmp/apc/script.yaml \
  --no-uart-stdout --output-dir /tmp/apc/out
```

Real output (wall-seconds vary between runs):

```text
2026-10-18T14:27:00.743955Z  INFO labwired_core::system::builder: Loading chip descriptor: /tmp/apc/chip.yaml
2026-10-18T14:27:00.749375Z  INFO labwired_loader: ELF Entry Point: 0x8000041
labwired-cli test: idle_ff=on (event_scheduler=on)
PASS  3/3 checks · script · 100000 steps · 0.03s
```

Two sanity checks worth doing once, because a declarative block that answers
zeros would also "load":

- Remove the CMU enable store from the firmware and rebuild. The same script
  fails: `Memory assertion failed at 0x20000008 (size 32): expected 0x3, got
  0x0 (mask 0x3)` — the clock gate drops the accesses, as documented.
- `out/result.json` ends with the block in `inspect.peripherals` (`kind:
  "declarative"`) and decodes the final word: `MSC_STATUS.value: 2`, `DONE =
  1`, `BUSY = 0`.

The test script schema (limits, assertions, `memory_value` details) is in
[CI Test Runner][ci-runner].

[ci-runner]: ../../ci_test_runner.md

## Where to go for more

| Topic | Link |
|---|---|
| Field-level reference, side effects, timing entries | [Declarative Registers][declarative-registers] |
| Register layout in context of a whole chip | [Configuration (YAML)](../../configuration_reference.md) |
| Parts you wire on a bus (I²C/SPI sensors, actuators) — a different object from a chip block | [Onboard a part](../onboard-part.md) |
| Deep reference for device descriptors and packs | [Part packs](../../part-packs.md) |
| Test scripts | [CI Test Runner][ci-runner] |

[declarative-registers]: ../../declarative_registers.md

## What this does not do

- **It does not write flash.** `MSC_WDATA` and `MSC_ADDR` are storage; no page
  erase or program happens, and no `MSC_IF`/`MSC_IEN` interrupt is raised. Add
  a hook (Rust) or a richer primitive only when firmware must observe the
  flash array itself.
- **It does not route interrupts.** A descriptor can declare an `interrupts:`
  map and a `timing` entry can name `interrupt:`; connecting that to the
  vector table is chip-level work, see [Declarative Registers][declarative-registers].
- **It does not connect DMA.** Nothing in the descriptor reaches the DMA/LDMA
  fabric.
- **Fields do not mask.** `fields:` is decode metadata for inspection; write
  masking is per register via `side_effects.write_action` (e.g.
  `write_one_to_clear`), not per field.
- **Delays are tick-quantised.** `delay_cycles` counts peripheral ticks, not
  nanoseconds; the default tick interval makes one tick one CPU cycle.
