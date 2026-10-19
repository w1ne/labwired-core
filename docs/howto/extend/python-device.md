---
description: "An I2C sensor is data, not Python code: model an LM74-class temperature part as a YAML device pack, attach it to a system, prove it with a firmware, then drive and assert it from the Python SDK."
---

# Add an I²C sensor (data + Python)

Renode issue [#689][renode-689] asks whether an external device on SPI or I²C —
"e.g. temperature or ambient light sensor" — can be modelled in Python and
connected to the CPU, ideally as a Python twin of the C# `TI_LM74.cs` sensor.
On LabWired the answer is a split, and the split is the point:

- **A device is data.** A part that speaks a bus the engine already implements
  (I²C, SPI, UART, GPIO timing) is one YAML document: a register map plus
  `behavior:` primitives. There is no Python-defined-device mechanism in this
  repository — nothing interprets Python as a device model — and this page adds
  an **LM74-class I²C temperature sensor** entirely that way.
- **Python drives and asserts.** The `labwired` Python SDK (`Sim`) opens a
  machine, drives the stimulus inputs a device declares, moves simulated time,
  and asserts on what came back: UART text, memory words, transcripts. It does
  not define devices.

[renode-689]: https://github.com/renode/renode/issues/689

This page is the bus-part companion to [Add a peripheral block
(YAML)][add-peripheral]: that one adds a register block *inside* a chip, this
one wires a part to an I²C controller and proves it end to end with
`labwired test`, then reads the same result from Python.

[add-peripheral]: ./add-a-peripheral.md

## 1. Read the datasheet into a register map

The LM74 is a TI/National temperature sensor. Its datasheet [SNIS107] states the
temperature word format precisely: a **13-bit two's-complement** value with
`LSB = 0.0625 °C`, framed MSB-first in 16 bits, with the low bits reserved:

| Bits | Meaning |
|---|---|
| `D15:D3` | 13-bit two's-complement temperature (`D15` = sign); 1 LSB = 0.0625 °C |
| `D2` | always set high (the datasheet's "high bit") |
| `D1:D0` | tri-state on the SPI part; read as 1 in this model |

So the fixed reading this walkthrough models, +25.0 °C, is
`400 counts = (400 << 3) | 0x4 | 0x3 =` **`0x0C87`** — the same word the
datasheet's own Temperature Data Format table prints for +25 °C (`0C 87h`).

The modelled part puts that word behind the ordinary I²C **register-pointer**
protocol (write one byte to select a register, then read or write its bytes MSB
first):

| Register | Pointer | Width | Access | Reset | Contents |
|---|---|---|---|---|---|
| `TEMP` | `0x00` | 2 bytes, big-endian | read-only | `0x0C87` | +25.0 °C, fixed |
| `CONFIG` | `0x01` | 1 byte | read/write | `0x00` | plain storage; bit 0 is named `SHUTDOWN` |

Two honesty notes before any YAML:

- **This is a model, not silicon.** The LM74 itself is an SPI/MICROWIRE part —
  there is no I²C LM74 to copy. The *temperature word* follows the datasheet's
  format; the pointer map and the bus are this model's own. There is no
  conversion time, no self-heating, no alert output, and `CONFIG.SHUTDOWN`
  stores and reads back but nothing acts on it.
- **The temperature register is a constant.** No `source:` channel drives it
  (see the descriptor below), which is exactly why the Python section finds
  nothing to drive for this part.

[SNIS107]: https://www.ti.com/lit/ds/symlink/lm74.pdf

## 2. Write the descriptor

The descriptor is one document. Its `behavior:` body is the same schema every
built-in `configs/devices/*.yaml` part uses; the `schema:`/`type:` envelope is
what makes it a **part pack** a system manifest can carry ([Part
packs][part-packs]). This is the complete file, `/tmp/lm74/lm74.yaml` in the
run below:

[part-packs]: ../../part-packs.md

```yaml
# LM74-class I2C temperature sensor — a MODEL, not silicon.
#
# The LM74 itself (TI/National SNIS107) is an SPI/MICROWIRE part; there is no
# I2C LM74. This file models an LM74-CLASS part on LabWired's I2C
# register-pointer primitive. The temperature WORD follows the LM74
# datasheet's 13-bit two's-complement format:
#
#   D15..D3  13-bit two's-complement temperature, LSB = 0.0625 °C
#   D2       always set high ("D2: Always set high", Temperature Data Format)
#   D1:D0    tri-state on the SPI part; read as 1 in this model
#
# +25.0 °C = 400 counts = (400 << 3) | 0x4 | 0x3 = 0x0C87 — the datasheet's
# own table entry for +25 °C.
#
# Wire protocol: ordinary register pointer. The master writes one byte
# selecting the register, then reads or writes its bytes (MSB first).
# `auto_increment` is deliberately NOT set: this family's map OVERLAPS — the
# configuration register lives at 0x01, the same byte address the 2-byte
# temperature word spans — and a byte-wise auto-increment walk could only
# serve one register at 0x01. A pointed read latches exactly one register's
# word, so a TEMP read returns both temperature bytes and a CONFIG access
# reaches the configuration byte (the TMP102/LM75 shape).
schema: labwired.part/v1
type: howto:lm74
source: howto-extend
behavior:
  primitive: i2c_device
  i2c:
    default_address: 0x48
    registers:
      # Fixed +25.0 °C. No `source:` — nothing drives it; the register reads
      # back its reset value, which is the honest shape of a constant model.
      - name: TEMP
        addr: 0x00
        width: 2
        endian: be
        access: r
        reset: 0x0C87
      # Configuration: plain storage a driver writes and reads back. Bit 0 is
      # named SHUTDOWN; nothing acts on it, because a fixed model has no
      # conversion to shut down (stated in the page, not hidden).
      - name: CONFIG
        addr: 0x01
        width: 1
        endian: be
        access: rw
        reset: 0x00
        bits:
          - { name: SHUTDOWN, shift: 0 }
metadata:
  label: "LM74-class temperature (I2C model)"
  summary: "Register-pointer I2C temperature part, fixed +25.0 °C, config register readable/writable."
  category: i2c
  config_keys:
    - name: i2c_address
      ty: int
      doc: "7-bit slave address. Defaults to the modelled 0x48."
```

Notes that save a round trip:

- `type:` is namespaced (`howto:lm74`) on purpose. A pack whose bare name
  collides with a built-in part is a hard load error; a vendor prefix is the
  contract's recommended spelling ([Part packs][part-packs]).
- **No `auto_increment`**, unlike the ADXL345 descriptor, and that is a
  datasheet property of this family, not an omission: the 2-byte `TEMP` word
  and `CONFIG` share the byte address `0x01`, and a byte-wise pointer walk
  could serve only one of them there. A pointed read latches exactly one
  register's full word — the same shape as the shipped
  [TMP102 descriptor][tmp102]. The ADXL345 can auto-increment because its data
  registers do not overlap.
- `access: r` with a `reset:` is how a constant part reads back: no `update:`,
  no rule, no timer. `CONFIG` is a plain `rw` store — a driver that writes it
  and reads it back sees its own byte.
- There are no `metadata.inputs`, so the part exposes no stimulus channels. The
  Python section below is explicit about what that means.

[tmp102]: ../../../configs/devices/tmp102.yaml

## 3. Attach it to a system

A system manifest names the chip, carries the pack under `parts:`, and places
the part on a bus under `external_devices:` — the same block the shipped
[adxl345-sensor-lab][adxl-lab] system uses. `/tmp/lm74/system.yaml`:

[adxl-lab]: ../../../configs/systems/adxl345-sensor-lab.yaml

```yaml
name: "lm74-sensor-lab"
chip: "chips/stm32f103.yaml"
parts:
  - path: "./lm74.yaml"
external_devices:
  - id: "lm74"
    type: "howto:lm74"
    connection: "i2c1"
    config:
      i2c_address: 0x48
board_io: []
```

- `parts: - path:` is a CLI convenience: `SystemManifest::from_file` reads the
  file and **inlines the pack**, so what the simulation core sees is one
  self-contained manifest that also works in the browser and hosted runners
  (there is no filesystem there).
- An `external_devices[].type` resolves against this system's `parts:` first,
  then the built-in registries — so `howto:lm74` needs no engine change at all.
- `connection: "i2c1"` names the **chip's** controller (STM32F103 I²C1), and
  `config.i2c_address: 0x48` overrides the pack's default with the same value;
  omit it to take the pack's `default_address`.
- Two homes for one descriptor: a part meant to ship *built in* lives in
  `configs/devices/` and is `include_str!`-embedded when the engine is compiled,
  so adding one needs an engine rebuild. A part you own rides in the manifest
  as a pack and needs nothing. The `behavior:` schema is identical.

The `chips/` and `peripherals/` trees next to the manifest are a copy of
`configs/chips/stm32f103.yaml` and its `configs/peripherals/stm32f103/`
schemas, so the proof below is self-contained; in a checkout you can point
`chip:` at `configs/chips/stm32f103.yaml` directly.

## 4. Prove it

Everything below ran from `/tmp/lm74/` holding `chips/`, `peripherals/`,
`lm74.yaml`, `system.yaml`, `firmware.elf` and `script.yaml`.

The firmware drives I²C1 with the legacy F1 polling sequence — START, address
with the ADDR clear (read SR1 then SR2), the pointer byte, a **repeated START**,
then a two-byte read (byte 1 ACKed, byte 2 NACKed with STOP) — and publishes
the word it read plus a success marker to RAM. Pure MMIO, no LabWired-specific
API:

```asm
/* LM74-class walkthrough firmware: drive the modelled I2C part over the
 * STM32F103 I2C1 controller with the legacy F1 polling sequence
 * (START / ADDR / DR / SR1 / SR2) and publish the temperature word it read.
 * Pure MMIO — no LabWired-specific API. Built with arm-none-eabi-as/ld,
 * linked at the STM32F103 flash base 0x08000000. */
    .syntax unified
    .cpu cortex-m3
    .thumb

    .section .vectors, "a"
    .word 0x20005000            /* initial SP */
    .word reset + 1
    .rept 14
    .word default_handler + 1
    .endr

    .text
    .global reset
    .thumb_func
reset:
    /* Clock I2C1: RCC_APB1ENR.I2C1EN (0x4002101C, bit 21). */
    ldr  r0, =0x4002101C
    ldr  r1, =0x00200000
    str  r1, [r0]

    /* I2C1 from reset: SWRST, then FREQ=16 MHz, CCR=80 (100 kHz), TRISE=17. */
    ldr  r6, =0x40005400
    ldr  r1, =0x8000
    str  r1, [r6, #0x00]        /* CR1.SWRST */
    movs r1, #0
    str  r1, [r6, #0x00]        /* release reset */
    movs r1, #16
    str  r1, [r6, #0x04]        /* CR2.FREQ = 16 MHz */
    movs r1, #80
    str  r1, [r6, #0x1C]        /* CCR: PCLK1 / (2 * 100 kHz) */
    movs r1, #17
    str  r1, [r6, #0x20]        /* TRISE: PCLK1_MHz + 1 */
    movs r1, #1
    str  r1, [r6, #0x00]        /* CR1.PE */

    /* ── Transaction 1: write the register pointer 0x00 (TEMP). ────────── */
    ldr  r0, [r6, #0x00]
    ldr  r1, =0x0100
    orrs r0, r0, r1
    str  r0, [r6, #0x00]        /* CR1 |= START */
    ldr  r2, =0x0001
    bl   wait_sr1               /* SB */
    ldr  r1, =0x90              /* 0x48 << 1 | W */
    str  r1, [r6, #0x10]        /* DR */
    ldr  r2, =0x0002
    bl   wait_sr1               /* ADDR */
    ldr  r1, [r6, #0x18]        /* SR2 read: completes the ADDR clear */
    movs r1, #0
    str  r1, [r6, #0x10]        /* DR: register pointer 0x00 */
    ldr  r2, =0x0080
    bl   wait_sr1               /* TXE */
    ldr  r2, =0x0004
    bl   wait_sr1               /* BTF: pointer byte on the wire */

    /* ── Transaction 2: repeated START, read the 2-byte word. ─────────── */
    ldr  r0, [r6, #0x00]
    ldr  r1, =0x0100
    orrs r0, r0, r1
    str  r0, [r6, #0x00]        /* CR1 |= START */
    ldr  r2, =0x0001
    bl   wait_sr1               /* SB */
    ldr  r1, =0x91              /* 0x48 << 1 | R */
    str  r1, [r6, #0x10]        /* DR */
    ldr  r2, =0x0002
    bl   wait_sr1               /* ADDR */
    ldr  r1, [r6, #0x18]        /* SR2 read: completes the ADDR clear */
    ldr  r0, [r6, #0x00]
    ldr  r1, =0x0400
    orrs r0, r0, r1
    str  r0, [r6, #0x00]        /* CR1 |= ACK: byte 1 is ACKed */
    ldr  r2, =0x0040
    bl   wait_sr1               /* RXNE */
    ldr  r4, [r6, #0x10]        /* MSB */
    ldr  r0, [r6, #0x00]
    bic  r0, r0, r1             /* r1 is still 0x0400: ACK off */
    ldr  r1, =0x0200
    orrs r0, r0, r1
    str  r0, [r6, #0x00]        /* CR1 = ... | STOP; byte 2 is NACKed */
    ldr  r2, =0x0040
    bl   wait_sr1               /* RXNE */
    ldr  r1, [r6, #0x10]        /* LSB */
    lsls r4, r4, #8
    orrs r4, r4, r1             /* word = MSB << 8 | LSB */

    /* ── Publish: temperature word + success marker. ──────────────────── */
    ldr  r3, =0x20000000
    str  r4, [r3]               /* 0x0C87 expected: +25.0 °C, 13-bit format */
    ldr  r1, =0xB007
    str  r1, [r3, #0x04]
park:
    b    park

fail:
    ldr  r3, =0x20000000
    ldr  r1, =0xDEAD
    str  r1, [r3]
    str  r1, [r3, #0x04]
    b    park

    .thumb_func
wait_sr1:
    ldr  r5, =200000
1:  ldr  r0, [r6, #0x14]        /* SR1 */
    tst  r0, r2
    bne  2f
    subs r5, r5, #1
    bne  1b
    b    fail
2:  bx   lr

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

`script.yaml` asserts the two words the firmware published (the test script
schema is in [CI Test Runner][ci-runner]):

[ci-runner]: ../../ci_test_runner.md

```yaml
schema_version: "1.0"
inputs:
  firmware: "/tmp/lm74/firmware.elf"
  system: "/tmp/lm74/system.yaml"
limits:
  max_steps: 100000
assertions:
  # The word the firmware read out of TEMP: +25.0 °C in the LM74 datasheet's
  # 13-bit big-endian format, 0x0C87.
  - memory_value:
      address: 0x20000000
      expected_value: 0x0C87
      mask: 0xFFFF
      size: 32
  # Success marker: the firmware only writes 0xB007 after the whole
  # START/ADDR/DR/repeated-START/read sequence completed.
  - memory_value:
      address: 0x20000004
      expected_value: 0xB007
      mask: 0xFFFF
      size: 32
  - expected_stop_reason: max_steps
```

Run it (with an installed CLI; from a source checkout,
`cargo run -p labwired-cli -- test …`):

```bash
labwired test --script /tmp/lm74/script.yaml \
  --no-uart-stdout --output-dir /tmp/lm74/out
```

Real output (wall-seconds vary between runs):

```text
2026-10-18T14:55:56.690691Z  INFO labwired_core::system::builder: Loading chip descriptor: chips/stm32f103.yaml
2026-10-18T14:55:56.784522Z  INFO labwired_loader: ELF Entry Point: 0x8000041
labwired-cli test: idle_ff=on (event_scheduler=on)
labwired-cli test: hosted fast path (exact windows up to 1024 cycles)
PASS  3/3 checks · script · 100000 steps · 0.00s
```

Two checks worth doing once, because a device that answered constants would
also "pass":

- Change the pack's reset from `0x0C87` to `0x0C86` and rerun. The same script
  now fails — `Memory assertion failed at 0x20000000 (size 32): expected 0xc87,
  got 0xc86 (mask 0xffff)` — so the word really comes out of the descriptor.
- `out/result.json` ends with the pack attached and inspected under
  `inspect.devices`:

  ```json
  {"id": "lm74", "device_type": "howto:lm74", "declared": true,
   "attachment": {"transport": "i2c", "bus": "i2c1", "address": 72},
   "artifacts": [{"kind": "i2c_registers", "id": "lm74",
     "meta": {"registers": {"CONFIG": 0, "TEMP": 3207}, "inputs": {}}}]}
  ```

  (Trimmed to the fields that matter; `3207` is `0x0C87`, address `72` is
  `0x48`.)

## 5. Drive and assert from Python

The Python side is the SDK documented in [Python SDK][python-sdk]. Install it
exactly as the quickstart there does — the wheel is built from source with
maturin, and **this checkout commits no built wheel**, so this page does not
paste a session transcript; what it shows is the complete program to run. For
the same `/tmp/lm74` system:

[python-sdk]: ../../python-sdk.md

```sh
python - <<'PY'
from labwired import Sim

with Sim("/tmp/lm74/firmware.elf", system="/tmp/lm74/system.yaml") as sim:
    sim.run_for("10ms")
    word = sim.read_u32(0x20000000)
    print(hex(word), (word >> 3) * 0.0625, "°C")
    print(sim.list_inputs())
PY
```

The word that comes back is the same `0x0C87` the test script asserts
(25.0 °C after the `>> 3` the datasheet format requires). Two honest notes:

- **`list_inputs()` reports an empty list for this part** — and that is the
  correct answer, not a bug. A fixed model declares no `metadata.inputs`
  stimulus channels, so there is nothing to drive: `set_input(...)` for a
  channel a part never declared is an error, per the SDK reference. The
  SDK drives what a descriptor exposes; this sensor has no knobs.
- **There is no UART here.** The firmware publishes to RAM, so the SDK's
  `expect()` / `uart_transcript()` would find nothing; `read_u32(0x20000000)`
  is the same bus read the script's `memory_value` assertion performs. For a
  part with UART output, `expect(regex, timeout=…)` is the natural assertion
  (see the quickstart's `OK` example).

A part that *does* declare inputs (an accelerometer's x/y/z, a temperature
slider, a button) shows up in `list_inputs()` and is driven with
`sim.set_input("key", value)` or the atomic `sim.set_inputs({...})` — the same
channels a `source:` register reads. That is the full extent of Python's
"modelling" role in this repo: stimulus in, observations out.

## What is not supported

- **No Python-defined-device path.** There is no `PythonPeripheral` analogue,
  no per-register Python hook, no way to attach a `.py` file as a device. A
  part whose wire protocol the primitives cannot express is a new Rust
  primitive — the boundary named in [Part packs][part-packs] — not a script.
- **External-process co-simulation is signal-level only.** The
  [co-simulation linkage][cosim] exchanges scalar `inputs`/`outputs` on a
  fixed time step. It cannot implement an I²C transaction device: a
  register-pointer slave must parse START/address/pointer frames, hold state
  across bytes inside one transaction, and drive the data line before the
  master's next poll. Neither is it wired to the I²C controllers as a device
  seam.
- **This pack models, and does not model.** The value is fixed at 25.0 °C —
  nothing drives `TEMP`. `CONFIG.SHUTDOWN` is storage, with no conversion to
  shut down. No conversion time, no alert pin, no address-select aliases.
- **Python does not extend fidelity.** `Sim` exposes drives and observations
  over the same models the CLI runs — it adds no device behavior of its own,
  and the SDK docs are explicit that simulation checks firmware against
  modelled peripherals, not electrical behaviour or physical parity.

[cosim]: ../../cosim-external-process.md

## Where to go for more

| Topic | Link |
|---|---|
| The descriptor schema in full (built-in parts are the same object) | [`adxl345.yaml`][adxl], [`tmp102.yaml`][tmp102] in `configs/devices/` |
| The part-pack contract (`labwired.part/v1`, `parts:`, resolution order) | [Part packs][part-packs] |
| Adding a register block inside a chip (the sibling walkthrough) | [Add a peripheral block (YAML)][add-peripheral] |
| Register maps, fields, side effects for chip blocks | [Declarative Registers][declarative] |
| Test scripts (`memory_value`, limits, stop reasons) | [CI Test Runner][ci-runner] |
| Driving and asserting from Python | [Python SDK][python-sdk] |
| Signal-level external-process coupling | [Co-simulation][cosim] |
| System and chip YAML in context | [Configuration (YAML)](../../configuration_reference.md) |

[adxl]: ../../../configs/devices/adxl345.yaml
[declarative]: ../../declarative_registers.md
