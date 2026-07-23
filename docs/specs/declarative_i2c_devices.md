# Declarative I²C devices (`behavior.primitive: i2c_device`)

Adding an I²C sensor to LabWired is **one YAML file** in `configs/devices/`,
plus a one-line kit registration. No per-part Rust. The generic engine
(`crates/core/src/peripherals/components/declarative_i2c.rs`) interprets the
descriptor: `GenericI2cDevice` implements `I2cDevice` + `SimInput`, and
`DeclarativeI2cKit` registers it in the normal `PeripheralKit` registry — so a
declarative device gets the peripherals manifest, `'kit'` sim backing, and
component-id stamping exactly like a hand-written kit.

Shipped examples: `sht31.yaml` (16-bit commands + Sensirion CRC-8),
`bh1750.yaml` (single-byte opcodes), `veml7700.yaml` (register-pointer device,
config-dependent scaling — migrated from Rust behind a byte-parity harness).

## The boundary rule

**The engine owns all mechanics; the YAML is only the part's datasheet
contract.** Bus framing, pointer bookkeeping, byte order, CRC computation,
data-ready timing, and input routing live in the engine, written once. Every
field in a descriptor must be derivable from the part's datasheet alone — if a
field requires knowing how the simulator works, it belongs on the engine side.

There is deliberately **no expression language**. The expressiveness ceiling
is: linear encode (`scale`/`offset`/clamp), `scale_from` factors keyed off
register bit-fields, and divide-mode `resolution` with a factor table. A part
that needs more stays a hand-written Rust kit (that is the escape hatch, per
the primitive-composition principle in the device-DSL design).

## Descriptor schema

```yaml
type: <part-id>                 # must match the catalog part type
behavior:
  primitive: i2c_device
  i2c:
    default_address: 0x44       # firmware can override via i2c_address config
    code_width: 2               # command shape only: 1 or 2 opcode bytes (default 2)
    crc8: { poly: 0x31, init: 0xFF }   # optional; frames each 16-bit response word

    registers:                  # register-pointer shape (VEML7700 class)
      - name: ALS_CONF
        addr: 0x00              # pointer byte
        width: 2                # bytes
        endian: le              # le | be
        access: rw              # r | rw (rw stores writes, readable back)
        reset: 0x0001
      - name: ALS
        addr: 0x04
        width: 2
        endian: le
        access: r
        source: lux             # input-channel key (metadata.inputs)
        resolution:             # divide mode: count = round(value / (base × Πfactors))
          base: 0.0576
          factors:              # each keyed off another register's bit-field
            - { register: ALS_CONF, mask: 0x1800, shift: 11, map: { 0: 1.0, 1: 0.5 } }
        # OR linear mode:  encode: { scale: 374.49, offset: 16851.86 }
        # source_scale: 1.15    # optional pre-multiply on the source value

    commands:                   # command shape (Sensirion / BH1750 class)
      - name: single_shot_high_no_stretch
        code: 0x2400            # big-endian on the wire, code_width bytes
        response:
          - { source: temperature, width: 2, encode: { scale: 374.4857, offset: 16851.857 } }
          - { const: 0x8010, width: 2 }      # constant word (status reads)
        # delay_us: 15000       # see the timing caveat below
        # params_words: 1       # parameter words accepted and ignored

metadata:
  label: "Sensirion SHT31"
  summary: "Temperature + humidity sensor over I²C."
  category: i2c
  inputs:                       # load-bearing: defines the SimInput channels
    - { key: temperature, label: "Temperature", unit: "°C", min: -40, max: 125, default: 22 }
```

A descriptor is exactly one shape: `registers` XOR `commands` (validated).

## Semantics the engine guarantees

- Register mode: first written byte latches the pointer; further bytes
  accumulate into `rw` registers at the declared width/endianness. Unknown
  pointer reads a zero word (matches the veml7700 reference behavior).
- Command mode: dispatch fires at `code_width` bytes, big-endian. Write-only
  or unknown commands queue no response; reads past the buffer return `0xFF`
  (matches the scd41 reference behavior). CRC-8 framing is byte-identical to
  `sensirion::encode_words`.
- Encodings: linear `encode` multiplies; `resolution` divides and reproduces
  a division-based oracle bit-for-bit **only when the factor table entries are
  exact in f64** (the VEML7700 IT/gain factors are powers of two). Do not
  substitute multiply-by-reciprocal for a datasheet division — it diverges by
  one LSB on x.5 rounding ties (proven by the VEML7700 parity sweep).
- ⚠️ `delay_us` gates data-ready on `I2cDevice::advance_time_us`, which most
  controllers do not drive yet. Until time is plumbed universally, **do not
  set `delay_us`** — model always-ready, the same choice the hand-written
  Sensirion models made.

## Adding a part (checklist)

Core (this repo):
1. `configs/devices/<part>.yaml` — the descriptor. Cross-check every encoding
   against the datasheet formulas (the BH1750 initially shipped with the
   lux conversion inverted; a datasheet review is not optional).
2. Register it: `embedded_device_yaml` list in `crates/config`, plus a
   `DeclarativeI2cKit` static appended to `KITS` in `kit/registry.rs`.
3. Regenerate the vendored manifest fixture; `peripheral_kit_gate` and
   `manifest_json_matches_registry` must pass.

Superproject (`w1ne/labwired`): catalog entry, `part-models.ts` +
`part-simulation.ts` (+ compatibility matrix) one-liners, a ~10-line
`i2cSensorCard` `.tsx`, then regenerate the ui peripherals manifest and
catalog facts.

## Migrating a hand-written kit

Use `veml7700_parity.rs` as the template: drive the old Rust model and the
new `GenericI2cDevice` through **identical** `I2cDevice` call scripts and
assert byte-equal streams — power-on defaults for every register, every
config-field combination the Rust model distinguishes, a dense "nice value"
sweep over the source channels (this is what catches rounding-tie divergence;
random sweeps do not), rejection/edge framing cases. Swap the registry entry
only when parity is green; demote the Rust model to `#[cfg(test)]` as the
oracle. Never bend either model to make parity pass — a mismatch means the
schema needs a (minimal, datasheet-language) extension or the part stays Rust.
