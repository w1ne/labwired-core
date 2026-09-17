# Part packs — `labwired.part/v1`

A **part pack** is one file describing one part, completely. It is the only
thing you need to connect a part LabWired has never seen — from a private
catalog, a customer's internal library, a vendor's own repo, or a directory on
your laptop — without a line of code in this repository and without publishing
anything.

The contract exists because the alternative had already grown four places to
edit per part: a descriptor in `configs/devices/`, a `KITS` entry, a catalog
record in the app, and a hand-mirrored emitter. All four are derivable from one
document, so this is that document.

## Why one file and not four

A part is one physical thing. Splitting its description across repos means the
halves drift, and drift in this domain is silent: a catalog record claiming 6
pins against a model with 4 does not fail a build, it fails a customer's
firmware at 3am with a wiring error that reads like their bug. One file, one
part, one source of truth — and the loader refuses a second definition of the
same `type` rather than picking a winner.

## The document

```yaml
schema: labwired.part/v1       # required — the contract version this file obeys
type: acme:tmp999              # required — globally unique id, `vendor:part`
source: acme-private           # optional — provenance; who shipped this pack
overrides: tmp102              # optional — the ONLY way to shadow a built-in

behavior:                      # required — how it behaves on the wire (the sim)
  primitive: i2c_device
  i2c:
    default_address: 0x4A
    registers: [ ... ]

emit:                          # optional — canvas wiring → system-manifest entry
  connection: i2c
  config: [ ... ]

metadata:                      # optional — label, summary, stimulus channels
  label: "ACME TMP999"
  inputs: [ ... ]

catalog:                       # optional — the app-layer record (pins, class)
  deviceClass: i2c_device
  refPrefix: U
  pins: [ ... ]
```

`behavior`, `emit` and `metadata` are the existing `configs/devices/*.yaml`
schema, unchanged — every shipped descriptor is already a valid pack body, which
is deliberate: the built-in parts and your private parts are the same kind of
object, so the private path is the one we dogfood daily rather than a bolted-on
side door.

`catalog` is ignored by this crate. It is carried through for the app layer
(`@labwired/board-config`), so a pack stays one file end to end. Its fields are
the `CatalogPart` fields, spelled exactly as TypeScript spells them
(`deviceClass`, `refPrefix`, `defaultI2cAddress`, `pins`) — the block is handed
over verbatim, and a rename step here would only be one more thing to get wrong.

### `type` must be namespaced

Use `vendor:part`. A bare `tmp999` is accepted but risks colliding with a
built-in we add later; a collision is a hard error at load, so an un-namespaced
private pack is a future build break you have chosen to schedule.

### `overrides` is the only way to shadow a built-in

If a pack's `type` names a part the engine already ships, loading fails:

```
part pack 'tmp102' (source: acme-private) shadows a built-in part.
Set `overrides: tmp102` to replace it deliberately, or rename the pack.
```

Setting `overrides:` to the same string makes the replacement explicit and
attributable in a bug report. Silence is never the answer to "which model ran?".

## Connecting a pack

Packs travel in the system manifest, so every transport that already carries a
manifest carries packs too — the CLI, the browser wasm build, and the hosted
builder's `/run`. There is no new endpoint and no new file the runtime has to
find on disk.

```yaml
# system.yaml
chip: "esp32c3"
parts:
  - path: "./private/acme-tmp999.yaml"    # CLI-only convenience, inlined on load
  - schema: labwired.part/v1              # or the pack inline, verbatim
    type: acme:hum1
    behavior: { ... }

external_devices:
  - id: t1
    type: acme:tmp999                     # resolves against `parts:` above
    connection: i2c0
    route: { sda: "GPIO4", scl: "GPIO5" }
```

`path:` is a `labwired` CLI convenience: `SystemManifest::from_file` reads the
file and replaces the entry with its contents, exactly as it already does for a
`can-player`'s `path:`. The simulation core never sees a `path:` — it has no
filesystem in wasm, and a contract that only works on one of our three runtimes
is not a contract.

## Resolution order

For an `external_devices[].type`, in order:

1. `parts:` in this manifest
2. the built-in `PeripheralKit` registry (`peripherals::kit::registry`)
3. the embedded declarative descriptors (`configs/devices/*.yaml`)
4. the legacy hand-written attach arms

A pack at step 1 that also exists at step 2 or 3 is the collision error above,
not a silent win.

## Connecting a source to the app

The engine reads packs out of a manifest. The app is what puts them there:

```ts
import { registerPartSource } from '@labwired/board-config';

registerPartSource({
  id: 'acme-private',
  packs: await fetchEntitledCatalog(orgId),   // any origin: HTTP, file, bundle
});
```

From that call on, the part behaves like any other:

- `getCatalogPart('acme:tmp999')` resolves it, so ERC, wiring, netlist export
  and the compiler treat it as a first-class part.
- `listCatalogParts()` includes it, so the palette offers it. (Enumerate with
  that, never `Object.values(CATALOG)` — the latter sees only parts we ship,
  which is how a connected part ends up simulating correctly while being
  invisible in the palette meant to offer it.)
- `compile()` inlines the packs the diagram actually uses into the manifest's
  `parts:`, so the lab runs on an engine that has never heard of your catalog —
  and keeps running after the source that supplied the part is gone.
- The canvas draws it from its declared `pins` via the generic renderer. Shipping
  hand-drawn artwork is an improvement, never a prerequisite.

Registration enforces the same rules the engine does, and one more: the app's
built-in set is not the engine's. A part can be catalogued in the app and
modelled in the engine, or modelled in the engine and absent from the app
catalog (`tmp102` is). Each side therefore checks its own set, and a pack has to
clear both.

## The Tier-1 primitives a register map can reach for

`behavior` is data interpreted by a primitive, and the vocabulary below is what
a register-map or command descriptor may say without any Rust. Each key exists
because a real datasheet sentence could not be written without it, and each is
named with the part that proved it — a key with no shipped descriptor behind it
is a key nobody has checked.

Every one of these defaults to the behaviour descriptors had before it existed,
so adding the key to the schema moved no shipped part's bytes.

### `behavior.derived` — a value the part COMPUTES

A named value computed from the stimulus channels by a small arithmetic
expression, evaluated fresh on every read. A `source:` on a register, a field or
a response word names it exactly as it names a stimulus channel.

```yaml
behavior:
  derived:
    - { name: power_w, expr: "bus_voltage * abs(current)" }
```

The grammar is names, decimal literals, `+ - * /`, unary `-`, parentheses, and
`abs(x)` / `min(a, b)` / `max(a, b)`. Nothing else: rounding to a register count
is `encode`'s job, and a value that depends on what the part is currently doing
is a rule, not an expression. Channels evaluate in declaration order, so a later
one may read an earlier one and a cycle cannot be written. A name that is
neither a declared input nor an earlier derived channel is a **load error**.

Proved by **`ina219.yaml`**, whose POWER register (§8.5.4) is
`bus_mV × |I_mA| / 1000` — a product of two stimulus channels. Also by
`mlx90614.yaml`, where the °C → K conversion the datasheet itself performs is a
derived channel rather than an `encode.offset`, because folding it into the
encode would multiply before adding where §8.4.4 adds before multiplying.

### `source_from` — which channel a register reports

A register whose measurement channel is selected by a bit-field of *another*
register. Same `register` / `mask` / `shift` extraction as `scale_from`, applied
to the question instead of to the answer's scale.

```yaml
- name: CONVERSION
  addr: 0x00
  source_from:
    register: CONFIG
    shift: 12
    mask: 0x07
    table: { 0: a0_a1_diff, 4: a0, 5: a1, 6: a2, 7: a3 }
```

A field value with no `table` entry falls back to the register's own `source`
and reads 0 otherwise. A `table` entry naming neither an input channel nor a
derived channel is a **load error** — a converter that silently reports ground
on one of its inputs is exactly the quiet wrong answer a twin exists to refuse.

Proved by **`ads1115.yaml`** (§9.3.3 Table 8): CONVERSION follows the MUX bits
of CONFIG, while `scale_from` on the same register follows the PGA bits.

### `fields[].scale_from` — a left-justified output register

`scale_from` inside a composite field, same shape and same engine helper as the
register-level key. This is what makes a left-justified output expressible: the
field's `shift` / `width_bits` round the value to N bits **before** the shift,
so the low bits are zero the way silicon leaves them, and the per-field
`scale_from` supplies the full-scale select that placement alone could not.

```yaml
- name: OUT_X
  addr: 0x01
  width: 2
  fields:
    - source: x
      shift: 2          # left-justified: 14 bits at bit 2
      width_bits: 14
      signed: true
      scale_from: { register: XYZ_DATA_CFG, mask: 0x03,
                    map: { 0: 4096.0, 1: 2048.0, 2: 1024.0, 3: 1024.0 } }
```

There is no `justify:` key: the shift is already `shift`, and a second spelling
of one thing would need a rule for which wins.

Proved by **`mma8451q.yaml`** (§6.2, Table 5) — a 14-bit count at bit 2 whose
counts-per-g `XYZ_DATA_CFG.FS` selects. Before this a descriptor could have the
justification or the range switch, never both.

### `zero_unless` — the inverted power gate

`zero_when`'s mirror: the register reads an all-zero word **unless** a masked
bit of the named register is set. Same struct; a register declares at most one
of the two, and declaring both is a load error.

```yaml
- name: OUT_X
  addr: 0x01
  zero_unless: { register: CTRL_REG1, mask: 0x01 }
```

The polarity is in the key name, where the line reads as the datasheet sentence
("reads zero unless ACTIVE is set"), rather than in a `negate:` boolean whose
absence would silently mean one of the two — a mistyped boolean flips a power
gate with no error, a misspelled key is an unknown field.

Proved by **`mma8451q.yaml`** §6.1: `CTRL_REG1.ACTIVE` takes the part *out* of
standby, the opposite polarity to the VEML7700 shutdown bit `zero_when` was
written for.

### `i2c.auto_increment_map` — the hybrid auto-increment jump

Addresses the auto-increment pointer **jumps from** instead of stepping through.
Applies only on the auto-increment walk; an explicit pointer write is never
remapped. Empty ⇒ the pointer always steps by one.

```yaml
i2c:
  auto_increment: true
  auto_increment_map:
    - { from: 0x06, to: 0x33 }
```

The remap is unconditional: "this map applies only while that enable bit is set"
is a state machine, not a map, so a descriptor that declares the jump models the
part in hybrid mode and says so.

Proved by **`fxos8700.yaml`** §14.2 — with `M_CTRL_REG2.hyb_autoinc_mode` set, a
read walking off the end of the accelerometer block continues at the
magnetometer block, so a 6-axis driver pulls twelve bytes in one transaction.
Without the jump it reads reserved space as its magnetometer data.

### `crc8.covers` — SMBus PEC vs Sensirion word CRC

* `response` (default) — one checksum byte after every 16-bit word, computed
  over that word alone. The Sensirion framing every command descriptor had.
* `transaction` — one checksum byte after the whole response, computed over the
  **addressed SMBus frame**: `[addr << 1, command, (addr << 1) | 1, data…]`.

```yaml
i2c:
  code_width: 1
  crc8: { poly: 0x07, init: 0x00, covers: transaction }
```

This is the SMBus Packet Error Code (SMBus 3.1 §6.4.1) — it covers the address
and command bytes the master drove, so it cannot be computed from the response
in isolation. The address used is the one the device is *attached* at, so a part
moved by `i2c_address:` still answers a PEC its driver accepts.

Proved by **`mlx90614.yaml`** §8.4.3, whose read-word frame is exactly
`[addr·W, cmd, addr·R, LSB, MSB, PEC]`. A driver that validates the PEC — which
the good MLX drivers do — rejected every reading from a word-scoped checksum.

### `response[].endian` — a little-endian response word

Byte order of one command-response word. Absent ⇒ `be`, the 16-bit big-endian
Sensirion word.

```yaml
response:
  - { source: ambient_k, width: 2, endian: le, encode: { scale: 50.0 } }
```

SMBus is little-endian by definition (SMBus 3.1 §6.5.5, "data is sent low byte
first"), so every SMBus read-word part answers LSB then MSB. Byte-swapping the
value into the encode instead would produce a word whose two halves are a
different measurement. Proved by **`mlx90614.yaml`**.

### `metadata.inputs[].config_key` — a seed key that differs from the channel

The `external_devices` `config:` key that seeds a channel's starting value, when
it is spelled differently from the runtime channel `key`. Absent ⇒ the channel
key itself.

```yaml
inputs:
  - { key: surface_temp, label: "Surface temperature", unit: "°C",
      min: -70.0, max: 380.0, default: 18.0, config_key: surface_temp_c }
```

Needed where a hand-written kit named the two differently and shipped
`system.yaml` files already set the seed. Without it the seed would parse and
silently do nothing, and the part would boot at the descriptor default.

## What a pack cannot do

A pack is data interpreted by a **primitive** — `i2c_device`, `spi_device`,
`analog_source`, `display`, `quadrature`, `matrix`, `one_wire`, `pulse_echo`.
Those primitives are the irreducible timing algorithms, and they live in Rust in
this repository.

`analog_source` is the primitive for parts whose whole interface is one
analogue voltage (a Sharp IR ranger's `Vo`, an MQ-x module's `AOUT`): the
descriptor carries the datasheet's output curve as `(input, mV)` points plus
stated out-of-band rules (`below_first: clamp`, `above_last.floor_mv`), and the
engine owns the rest (SimInput plumbing, mV→ADC count, attach). The proof part
is `gp2y0a21.yaml`.

`display` is the primitive for framebuffer panels. The descriptor carries the
frame memory's geometry and pixel format, how a command byte is told apart from
a data byte (a D/C pad on 4-wire SPI, a control byte on I²C), the command table
as `{ opcode, args, do }`, and how the address counters wrap per addressing
mode. The engine owns the counter arithmetic, the window wrap, the orientation
map and the paint artifact — one implementation for every panel. Pixel VALUES
are never transformed: contrast, gamma and inversion are reported as flags, so
what the artifact holds is what firmware wrote and a photograph of the glass can
be compared against it. The proof parts are `ssd1306.yaml` (I²C, page-major
1 bpp) and `st7789.yaml` (SPI, row-major RGB565 with MADCTL orientation).

So: a part whose datasheet behaviour is a register map, a command/response
protocol, a framebuffer command table, or one of the pin-timing shapes above is
pure data and needs nothing from us. A part with a genuinely new wire protocol needs a new primitive, which
is a change to this crate. That boundary is honest and worth stating to a
customer up front: we can onboard your sensor catalogue without seeing it, but a
novel protocol is engineering, not configuration.

The same split applies to silicon. A private MCU is a chip descriptor —
`chip: "./acme-soc.yaml"` on the CLI, or the `chipYaml` field on the hosted
builder's `/run` — and needs no code here as long as its peripheral blocks are
ones the engine models. A novel peripheral block does not.

## Regenerating the cross-boundary fixture

`crates/core/tests/fixtures/emitted-part-pack-manifest.yaml` is `compile()`
output captured verbatim, so the engine test proves it can run the manifest the
app actually writes. Regenerate it from `packages/board-config`:

```sh
npx tsx -e "
import { compile } from './src/compile';
import { registerPartSource } from './src/part-sources';
registerPartSource({ id: 'acme-private', packs: [/* the pack in the test */] });
console.log(compile(/* the diagram in the test */).systemYaml);
" > ../../core/crates/core/tests/fixtures/emitted-part-pack-manifest.yaml
```

A diff there is a real cross-boundary change and wants reading, not blessing.

## Not yet covered

Named so nobody discovers them by surprise:

- **Private boards.** A private *chip* runs today through the paths above, but
  the app's `BOARDS` and `CHIP_YAMLS` are still build-time constants, so a
  private board cannot be offered in the picker the way a private part can be
  offered in the palette. Extending this contract to boards is the natural next
  increment — it carries more than a part does (pin map, renderer, compile
  toolchains, PlatformIO profile), which is why it is not folded in here.
- **Reverse mapping.** `system-to-diagram.ts` turns a manifest back into a
  canvas from `CATALOG` alone, so a shared lab containing a pack part will not
  round-trip into a diagram until that reads through `getCatalogPart` too.
- **Legacy compat maps.** `component-meta.ts` derives `COMPONENT_META` from
  `CATALOG` at module-init, so a pack declaring `boardIoKind` is not seen by the
  wire-derived board_io path. Same for `partSeedIntent.ts` and the ERC
  "did you mean" suggestions.
- **Entitlement.** Nothing here decides WHO may load which catalog. A pack is
  data; deciding that an org may fetch it is the API's job, not this contract's.
