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
`abs(x)` / `min(a, b)` / `max(a, b)` / `pow(a, b)` / `exp(x)`. Nothing else:
rounding to a register count is `encode`'s job, and a value that depends on what
the part is currently doing is a rule, not an expression. Channels evaluate in
declaration order, so a later one may read an earlier one and a cycle cannot be
written. A name that is neither a declared input nor an earlier derived channel
is a **load error**.

`pow` and `exp` arrived with the analog plants, each named by the datasheet that
forced it: the CdS photoresistor's `R = R₁₀ · (lux/10)^-γ` and the NTC's beta
equation `R = R₀ · e^(B(1/T − 1/T₀))`. They are functions, not a general power
operator, for the same reason the other three are: a `^` token invites a grammar
this language is not going to grow.

### `derived[].when` — a threshold on a BOOLEAN channel

```yaml
behavior:
  derived:
    - { name: charge_bump_mv, when: "usb_present >= 0.5", expr: "150" }
```

One comparison (`>`, `>=`, `<`, `<=`, `==`, `!=`) between two expressions in the
same grammar. **When it does not hold the channel is `0`, not `expr`.** No
`otherwise:` key, because a guarded channel is a TERM IN A SUM and 0 is that
sum's identity — a part that needs a different alternative writes
`base + gated`, which says out loud which half is the baseline. One comparison
and no `and`/`or`, because a compound condition is a second derived channel,
named, where a reader can see it.

A comparison is refused inside a plain `expr:` — it is a guard, never
arithmetic — so a descriptor cannot grow a conditional by accident.

Proved by **`lipo_charger.yaml`**: `usb_present` is a boolean carried as 0/1 on
a float stimulus channel, and "the charger is connected" is the half-way test
`>= 0.5` that the deleted Rust model used. Writing it here is what stopped that
threshold from being a line of engine code no descriptor could see.

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

## Register encoding keys

`encode:` is the register's measurement encoding. Beyond `scale` / `offset` /
`clamp_min` / `clamp_max` / `wrap` it carries four keys that exist because a
real part needed them, and each is documented here with the part that found it.

### `encode: { bcd: true }` — binary-coded decimal, both directions

Two decimal digits per byte, tens in the high nibble. **Symmetric**: a read
encodes, a write decodes. It is the LAST step of the read encode (after scale,
offset, the clamp window and `wrap`) and the FIRST step of the write decode, so
the word the model STORES is always decimal — `reg()`, `field()` and
`scale_from` all read a number, never a pair of nibbles.

```yaml
# DS3231 0x00: the seconds of a settable clock. `write_mask` on a BCD register
# is a plain AND on the byte the master wrote (the wire domain), because the
# flag packed alongside is not part of the number.
- { name: SECONDS, addr: 0x00, width: 1, endian: be, access: rw,
    source: unix_time, calendar: second, write_mask: 0x7F,
    encode: { bcd: true } }

# A plain BCD storage register — an alarm byte, clamped to the range it holds.
- { name: ALARM1_SECONDS, addr: 0x07, width: 1, endian: be, access: rw,
    write_mask: 0x7F, encode: { bcd: true, clamp_min: 0.0, clamp_max: 59.0 } }
```

A nibble above 9 is not a decimal digit; on the write side it is decoded the way
a counter chain reads it (`0x1A` is 20), and on the read side a count with more
digits than the register holds saturates at all-nines.

### `calendar:` — one civil field of a settable clock

On a register whose `source:` carries Unix seconds. A **read** reports that
civil field of the instant; a **write** RECOMPOSES — it replaces that field and
leaves the other six. Fields: `second` `minute` `hour` `weekday` `day` `month`
`year` (`year` is the two digits an RTC holds, `weekday` is 1..7 Sunday-first).

```yaml
- { name: HOURS, addr: 0x02, width: 1, endian: be, access: rw,
    source: unix_time, calendar: hour, write_mask: 0x3F,
    encode: { bcd: true } }
```

Without the write half a `source`d register is read-only and `RTClib::adjust()`
— the first call almost every RTC sketch makes — does nothing at all. Without
the read half the seven registers are seven independent bytes that can disagree
with each other about what day it is. The arithmetic is Hinnant's
civil-from-days / days-from-civil pair, UTC, no leap seconds.

⚠️ `input(KEY)` **skips** a `calendar:` register when it looks for the encoding
to report a channel through: such a register reports a FIELD, not the value, so
a rule asking for `input(unix_time)` gets the truncated engineering value.

### `encode: { clamp_from: [...] }` — a field-driven saturation window

The mirror of `scale_from`. The window is read from another register's
bit-field instead of being a constant, because on many parts the saturation
point is something firmware chose.

```yaml
# ADXL345 DATAX0. FULL_RES (bit 3) and the range bits (1:0) are not contiguous,
# so one mask picks all three and the map is keyed by the combination.
- name: DATAX0
  addr: 0x32
  width: 2
  endian: le
  access: r
  signed: true
  source: x
  scale_from: { register: DATA_FORMAT, mask: 0x0B,
                map: { 0x00: 256.0, 0x03: 32.0, 0x0B: 256.0 } }
  encode:
    clamp_from:
      - register: DATA_FORMAT
        mask: 0x0B
        map:
          0x00: { min: -512.0,  max: 512.0 }
          0x03: { min: -512.0,  max: 512.0 }
          0x0B: { min: -4096.0, max: 4096.0 }
```

A field value absent from `map` leaves the constant window (or none) in force —
the same "unmapped ⇒ neutral" rule `scale_from` has. Several entries INTERSECT,
each narrowing the window. A constant `clamp_max` here would be right for
exactly one of eight settings and would silently stop the part saturating at
the other seven.

### `encode: { round: floor | ceil | trunc | nearest }`

How the encoded value becomes an integer count. `nearest` (`f64::round`) is the
default and is what every descriptor written before the key existed means.

## Stimulus-channel keys

### `noise_sigma_key` — one `config:` value over a channel SET

A channel's `noise_sigma` is a property of the part; `noise_sigma_key` names the
`config:` key a PLACEMENT can set to override it. Spelled once per channel, so
one key reaches a whole set:

```yaml
metadata:
  config_keys:
    - { name: noise_sigma, ty: float,
        doc: "Gaussian noise sigma in channel units (g accel, °/s gyro)." }
  inputs:
    - { key: ax, label: "Accel X", unit: g, min: -16, max: 16,
        noise_sigma_key: noise_sigma }
    - { key: ay, label: "Accel Y", unit: g, min: -16, max: 16,
        noise_sigma_key: noise_sigma }
    # …and the other four motion axes. `temp` deliberately does NOT carry it:
    # the documented sigma is in g and °/s.
```

Per channel rather than as a group so a part whose axes have genuinely different
figures can still say so, and so reading one channel's entry tells you
everything that moves it. The key must also appear in `metadata.config_keys` to
be advertised in the peripheral manifest.

### `expr_scale` — counts per engineering unit, for a rule expression

The rule language is integers. On a register device `input(KEY)` is already the
value the register reports, so a rule comparing it against `reg(DATA)` compares
like with like. A pins-only part has no register to borrow an encoding from, so
it states the same thing directly:

```yaml
# HX711: the channel is grams and the frame is 24 bits at 100 counts per gram.
- { key: weight, label: "Weight", unit: g, min: -50000, max: 50000,
    default: 0, expr_scale: 100.0 }
```

Without it `input(weight)` truncates to whole grams and a load cell loses
exactly the digits it exists to measure — silently, because 10 g and 10.5 g
would shift out the same word.

### `crc8.covers: { bytes: N }` — a checksum over the first N ANSWER bytes

Three scopes, and a part is exactly one of them:

| `covers:` | the checksum framing |
|---|---|
| `response` (default) | one byte after EVERY 16-bit word, over that word alone — the Sensirion shape |
| `transaction` | one byte at the END, over `[addr·W, cmd, addr·R, data…]` — the SMBus PEC |
| `{ bytes: N }` | one byte at the END, over the first **N answer bytes** |

```yaml
    # AHT20 rev 1.1 §5.4: seven bytes out, the last a CRC-8 of the six before it.
    crc8: { poly: 0x31, init: 0xFF, covers: { bytes: 6 } }
```

`N` counts ANSWER bytes because that is how a datasheet states it. A **write-only**
command answers nothing and gets no checksum; every command that DOES answer must
answer at least `N` bytes or it is a LOAD error — a checksum over bytes the part
never sent is a literal wearing a checksum's name.

### `response[].fields` — a packed word that straddles byte boundaries

A command device's response word was one `source` (or one `const`) capped at a
`u32`. `fields:` gives it the same composite shape a register has, and widens it
to eight bytes:

```yaml
        response:
          - { const: 0x08, width: 1 }         # status
          - width: 5                          # 40 bits, big-endian
            endian: be
            fields:
              - { source: humidity,    shift: 20, width_bits: 20, encode: { scale: 10485.76 } }
              - { source: temperature, shift: 0,  width_bits: 20,
                  encode: { scale: 5242.88, offset: 262144.0 } }
```

Byte 3 of that word carries humidity[3:0] in its HIGH nibble and
temperature[19:16] in its LOW nibble — neither a word boundary nor a `u32`.
Without the key the only port is a CONSTANT payload, which freezes the part at
one reading and makes its checksum a literal.

`shift + width_bits` must fit inside `8 * width`, and `width` is 1..=8; both are
load errors rather than silent truncation. Each field is rounded and saturated
at its OWN bit width before `shift` places it — the same call
`register_read_bytes` makes, so a field means the same thing in a register and
in a response.

### `i2c.not_ready_byte` — what a command device says while it is busy

A command with `delay_us:` holds its response until the simulated clock reaches
the deadline. Until then the part answers `0xFF` (open bus) unless the datasheet
gives that byte a meaning:

```yaml
    not_ready_byte: 0x88     # AHT20: BUSY (bit 7) | CAL (bit 3)
```

⚠️ `0xFF` happens to carry BUSY and CAL too, which is exactly why this needs
declaring rather than leaving to luck: a part whose ready flag is active-LOW
would read READY the whole time it was busy.

### `bits:` — one declared channel standing for eight, seeded by ONE integer

A part whose channels are switch positions takes them as a BITMASK, not as eight
floats. `bits:` says so:

```yaml
metadata:
  config_keys:
    - { name: inputs, ty: int,
        doc: "Initial 8-bit input state (0..0xFF). Bit i seeds channel i." }
  inputs:
    - key: ch          # → ch0 … ch7
      label: "D"       # → D0  … D7
      unit: level
      min: 0.0
      max: 1.0
      bits: { count: 8, config_key: inputs }
```

The group expands ONCE, in `DeviceDescriptor::from_yaml`, so the kit metadata,
`peripherals-manifest.json`, `SimInput`, and a register field's `source:` all
see eight ordinary channels and none of them knows the group existed. `count`
channels are named `{key_prefix}{i}` / `{label_prefix}{i}`, defaulting to the
entry's own `key` and `label`.

`config_key` is what makes it a schema key rather than eight lines of copy-paste:
bit *i* of the integer under that key seeds channel *i* — set ⇒ `max`, clear ⇒
`min`. A channel's OWN key still wins when the placement sets it, so
`inputs: 0xA5` with `ch1: 1` means what it reads like.

⚠️ The 74HC165 was blocked on exactly this (PR #1186): four shipped manifests
set `inputs: 165`, per-channel seeding could not express it, and a port without
the key would have shipped a `config:` value that parses and changes nothing.
It also brought SPI config seeding into existence at all — until that port
`GenericSpiDevice` had no `seed_from_config`, so ANY starting value in an SPI
part's `config:` was silently ignored.

## Edge-driven `gpio_device` parts

A `gpio_device` is serviced on the peripheral tick. That is right for a part
sampled on a schedule and **wrong for a part clocked by firmware**: a
`digitalWrite(SCK, HIGH); digitalWrite(SCK, LOW)` pair is two MMIO stores inside
one tick interval, so a tick-only pass samples the pad after both and sees no
change. A 24-bit shift-out clocked by 48 stores would deliver one edge, or none.

A descriptor whose `rules:` listen for a pin EDGE is therefore serviced
synchronously inside the MMIO write path, and nothing extra is declared to get
it — the engine reads the rules:

```yaml
behavior:
  primitive: gpio_device
  pins:    { SCK: sck_pin }      # observed: pads the MCU drives
  outputs: [DOUT]                # driven: pads the MCU samples
  output_pins: { DOUT: dt_pin }
  rules:
    - on: { pin: SCK, edge: rising }   # ⇐ this makes the part edge-driven
      when: "var(shifting) && var(bit_index) < 24"
      do:
        - { var: { name: dout_level, value: "(var(raw) >> (23 - var(bit_index))) & 1" } }
        - { var: { name: bit_index, value: "var(bit_index) + 1" } }
        - { pin: DOUT, level: "var(dout_level)" }
```

⚠️ **Rule order is load-bearing.** Rules fire in declaration order and each
`do:` runs to completion, so a later rule sees what an earlier one assigned. In
`hx711.yaml` the rule that CLOSES the frame is declared before the rule that
shifts a bit: the other way round, the 24th edge would set `bit_index` to 24 and
the close rule would fire in the SAME event, dropping DOUT before the master
sampled the last bit. The frame reads one bit short, in the low bit only, every
time.

The pads such a part drives still go out through the narrowed `DevicePins` port,
exactly as on the tick pass — this changes WHEN `service` runs, not what it may
touch.

### The same hook is what a bus-resident Rust model uses

`edge_service_addrs` is a `BusResidentDevice` method, not a descriptor feature.
Any model on `SystemBus::gpio_devices` — Rust or YAML — names the OUTPUT
registers whose writes must service it, and the bus does the rest.

That matters because it is the only reason `SystemBus` no longer carries a
typed field per bit-banged part. It used to carry three:

| gone | what it was | what replaced it |
|---|---|---|
| `hx711: Vec<Hx711>` + `maybe_clock_hx711` | a 24-bit shift-out clocked by SCK | `hx711.yaml`, a `gpio_device` |
| `tm1637: Vec<Tm1637>` + `maybe_clock_tm1637` | an I²C-like 2-wire display, framed by CLK/DIO edges | `tm1637_7seg.yaml`, a `gpio_device` |
| `seven_segment: Vec<SevenSegment>` + `maybe_sample_seven_segment` | nine pads, combinational | `seven_segment.yaml`, a `gpio_device` |

Each hook was one part's private copy of
`maybe_service_edge_driven_gpio_devices`, complete with its own cache of
resolved GPIO peripheral indices, called from three places in
`bus/accessors.rs`. **The bus now has no typed display field at all**, and
`no_typed_display_field_on_the_bus` in
`crates/core/tests/bus_resident_device_port.rs` fails if one comes back.

Two facts a bus-resident display states, and both are load-bearing:

* `edge_service_addrs()` — the ODR addresses, sorted and **deduped**. The bus
  consults this on every MMIO write, so a duplicate is a cost paid per store.
  Nine 7-segment pads on one port are ONE address.
* `needs_per_cycle_service()` → `false`. A device whose pads move only when
  firmware stores to a GPIO output register, that owns no timer and drives no
  pad, is already serviced inside the write path; a tick pass would resample
  bits that cannot have moved. Saying so is what keeps such a board on the
  walk-free fast path and off `max_safe_tick_interval() == 1`. ⚠️ A
  `gpio_device` descriptor with `timers:` must NOT say this — its clock only
  advances on the tick.

A bus-resident display also reports through `BusResidentDevice::evidence()`,
which is what lets it publish artifacts without a typed bus field and an arm of
its own in `for_each_bus_resident_device`. A DESCRIPTOR fills that seam through
[`artifact:`](#artifact--what-a-part-shows) below.

### `on: { pins: [...] }` — one store, one event, every pad's new level

`on: { pin: X, edge: … }` is raised once per pad that moved. That is exactly
right for a part clocked on ONE line, and it cannot express a part framed by
two.

⚠️ **A single MMIO store can move several pads at once.** `BSRR = (1<<8) |
(1<<(9+16))` sets CLK and clears DIO in one instruction. Decomposed into two
sequential edge events, whichever fires first is decided against a STALE level
for the other line — and a TM1637 START is *"DIO fell **while CLK was high**"*,
a condition over both pads. A store that moved both would either synthesise a
START the firmware never sent or miss one it did.

The simultaneous-pad event is the fix. The engine resamples **every** observed
pad, installs the whole snapshot, and only then raises:

```yaml
behavior:
  primitive: gpio_device
  pins: { CLK: clk_pin, DIO: dio_pin }
  vars: { prev_clk: 1, prev_dio: 1, in_txn: 0 }
  rules:
    # START — DIO fell while CLK stayed high ACROSS THIS STORE.
    - on: { pins: [CLK, DIO] }
      when: "var(prev_clk) && pin(CLK) && var(prev_dio) && !pin(DIO)"
      do: [{ var: { name: in_txn, value: 1 } }]

    # ⚠️ LAST: latch the levels this store delivered.
    - on: { pins: [CLK, DIO] }
      do:
        - { var: { name: prev_clk, value: "pin(CLK)" } }
        - { var: { name: prev_dio, value: "pin(DIO)" } }
```

* **`pin(NAME)`** is a new name in the expression vocabulary: the CURRENT level
  of a pad the part observes or drives, 0 or 1. Inside any rule it is the
  post-store level of **every** pad, not just the one whose event was raised.
* **Matching is by INTERSECTION.** `on: { pins: [CLK, DIO] }` fires whether one
  of them moved or both. Requiring the exact set would make the rule fire only
  on the rarest store.
* **It fires BEFORE the per-pad `pin:` events** of the same store, so a part can
  use both.
* **`pins:` is a LEVEL event; `pin:` is an EDGE event.** The difference shows on
  the FIRST service pass: an edge needs a previous level and the first pass has
  none, so `pin:` raises nothing — while `pins:` fires, because the first store
  is as much a statement of levels as any later one. A combinational part that
  waited for a second store would stay blank forever under firmware that lights
  a digit once and leaves it alone.
* **A rule listening for `pins:` makes the part edge-serviced**, exactly as a
  `pin:` rule does.
* A `pin()` naming a pad the descriptor does not declare is a **load error**, in
  `when:` and in every action expression — not a guard that silently reads low.

⚠️ **Rule order is load-bearing here too, and more sharply.** Every guard above
compares `var(prev_*)` against `pin(*)`, so the rule that latches `prev_*` must
be declared **last**. Anywhere else it turns every condition into a comparison
of a level with itself: no START, no STOP, no sampled bit, and a panel that
stays blank while the firmware appears to work.

## `artifact:` — what a part SHOWS

A part could be simulated perfectly by a rule list and **inspect as nothing**:
`evidence()` / `artifacts()` had to be implemented, and only a concrete type can
implement a trait. So a ported TM1637 decoded every frame correctly and
published no text, no panel and no evidence — which made every display-oracle
clause about it unresolvable and painted an empty panel in the browser.

`artifact:` closes that. **The rules fill a RAM; the engine renders it.**

```yaml
behavior:
  primitive: gpio_device        # or spi_device — the SAME key
  vars: { g0: 0, g1: 0, g2: 0, g3: 0, g4: 0, g5: 0, display_on: 0, bright: 0 }
  rules: [ … the rules that fill those vars … ]

  artifact:
    kind: text_display          # text_display | framebuffer
    format: tm1637_grid         # meta.format — how the bytes are packed
    ram: { vars: [g0, g1, g2, g3] }
    decode: { font: seven_segment, digits: 4 }
    meta:
      - { key: lit_segments, source: lit_bits }
      - { key: display_on,   value: "var(display_on)", type: bool }
      - { key: brightness,   value: "var(bright)" }
      - { key: colon,        value: "var(g1) & 0x80", type: bool }
```

| key | meaning |
|---|---|
| `kind` | `text_display` (decoded characters) or `framebuffer` (packed pixels). These are the two kinds a display surface paints; a third spelling is a load error. |
| `id` | optional artifact-id **suffix**, for a part publishing more than one. ⚠️ Not an absolute id — the artifact is addressed by the DEVICE's manifest id, or two placements of one part would collide. |
| `format` | the `meta.format` string, matching a `crate::inspect::artifact_format` constant. A reader matches on it instead of downcasting to a Rust type. |
| `ram.vars` | variables, in order, each contributing its LOW BYTE. The list is the artifact's whole extent. |
| `ram.fifo` | a FIFO instead, oldest entry first. Exactly one of the two. |
| `decode.font` | `seven_segment` (the shared `0b0gfedcba` table, dp on bit 7) or `none`. |
| `decode.digits` | how many leading RAM bytes become `meta.text`. |
| `meta[].value` | an EXPRESSION over the part's own state — the whole rule vocabulary. |
| `meta[].source` | an engine-derived quantity over the rendered RAM: `lit_bits`, `ink_bytes`, `bytes`. |
| `meta[].type` | `int` (default) or `bool`. ⚠️ Not cosmetic: `display_on` is read with `as_bool()`, and an integer there is a different artifact. |
| `bytes` | publish the RAM as the artifact payload, gated behind `include_bytes`. Default false. |
| `fill_when` | expression: while true, every rendered byte reads `0xFF`. Checked FIRST. |
| `blank_when` | expression: while true, every rendered byte reads `0x00`. |

`format` and `generation` are stamped by the engine and cannot be redeclared.
`generation` is the cheap content hash a poller diffs, taken over the RAM the
artifact **publishes** — so a part that keeps more RAM than it shows (the
TM1637 has six GRIDs and a four-digit module wires four) does not report a
change nobody can see.

⚠️ **Why `lit_bits` is a `source:` and not a `popcount()` operator.** The
expression language has no bit-counting, and adding one would mean an operator
that exists for a single `meta` field — in a grammar whose whole argument is
that every name in it is something a datasheet says.

⚠️ **The key lives on the DESCRIPTOR, not on a primitive.** A part's artifact is
a property of the part: a segment display publishes the same `text_display`
whether the bytes arrived on nine pads or over SPI. So `gpio_device` and
`spi_device` read the same block and hand it to the same renderer. A second,
transport-flavoured spelling is how two renderings of one part drift apart.

### `blank_when` / `fill_when` — a panel that is off, without a second RAM

What a panel SHOWS is not always what its RAM holds. A MAX7219 in shutdown is
dark and a MAX7219 in display test is fully lit, and the datasheet is explicit
that **neither disturbs digit RAM** — the stored pattern reappears untouched
when the mode is cleared.

```yaml
  artifact:
    kind: framebuffer
    format: max7219_rows
    ram: { vars: [d0, d1, d2, d3, d4, d5, d6, d7] }
    fill_when:  "var(display_test)"     # 0xFF everywhere — checked FIRST
    blank_when: "var(shutdown)"         # 0x00 everywhere
    bytes: true
```

Both are evaluated at RENDER time over the one RAM, and everything derived from
the RAM follows: `meta.text`, `lit_bits`, `ink_bytes`, the `bytes` payload and
`generation` are all computed from what the panel shows, so a blanked panel
reports a blanked panel rather than the picture nobody can see.

⚠️ **`fill_when` wins, because the datasheet says so** — "display-test mode
overrides shutdown mode" (MAX7219/MAX7221, Table 10). Checked the other way
round, a display-test write on a shut-down panel would be invisible.

⚠️ **Written as rules instead, this is a SHADOW COPY of the RAM**, recomputed by
eight `var:` actions on every one of thirteen register writes — a part with two
RAMs that can disagree, and no way to report what firmware actually stored. Both
readings come out of one store here, which is what the deleted Rust model's
`framebuffer()` / `digit_ram()` pair was.

⚠️ **Neither can override the absence of a rail.** `powered:` refuses the bus
itself (below), so an unpowered part never leaves its power-on state at all.

## Parallel (8080) panels: the `I80Panel` seam

`Esp32s3LcdCam` drives an i80 panel with exactly one operation — a bus word and
a D/C level, after `LCD_USER`'s byte- and bit-order bits have been applied. It
used to hold `Vec<Arc<Ili9341Parallel>>`: a chip peripheral naming a part, and
the reason the parallel ILI9341 could not become a descriptor — port it and the
engine has no type to hold.

It now holds `Vec<Arc<dyn I80Panel>>`:

```rust
pub trait I80Panel: std::fmt::Debug + Send + Sync {
    fn i80_write_word(&self, dc_high: bool, word: u16);
}
```

`&self`, because a panel is shared between the GPIO observer watching its pads
and the peripheral strobing its bus. One method, because a second one is how a
controller learns about a part again — `i80_panel_seam_stays_narrow` in
`crates/core/tests/esp32s3_lcd_i80_pixels.rs` reads the trait's body and fails
on it, the same way `resident_device_port_stays_narrow` guards `DevicePins`.

⚠️ **Read a panel by FORMAT, never by concrete type.**
`bus.observed_of::<Ili9341Parallel>()` answers an empty iterator for a panel of
any other type — including the same panel the day it becomes a descriptor — so
a test written that way turns green by measuring nothing. Use
`SystemBus::display_artifacts_of_format(&[artifact_format::RGB565_BE], &opts)`
or `SystemBus::display_artifact(id, &opts)`: `meta.w`/`meta.h` are the logical
extents, `meta.painted_bytes` the non-zero BYTE count (not lit pixels), and the
payload is the oriented framebuffer.

## `logic_gate` — 74-series logic, as a truth table

A gate has **no bus**. No address, no register file, nothing to write and
nothing to read back — so neither `i2c_device`, `spi_device` nor `uart_device`
can describe one, and none of the 139 74-series symbols the 39-project KiCad
corpus drops could be a register part. What a gate has is input pads, output
pads, a boolean function between them, enables that take outputs off the wire,
and a propagation delay. That is the whole of `logic:`.

```yaml
# configs/devices/74hc125.yaml — quad 3-state buffer, one enable PER GATE
type: 74hc125

behavior:
  primitive: logic_gate
  logic:
    inputs: [A1, A2, A3, A4]
    outputs: [Y1, Y2, Y3, Y4]
    enables:
      - { pin: OE1, active: low, outputs: [Y1] }
      - { pin: OE2, active: low, outputs: [Y2] }
      - { pin: OE3, active: low, outputs: [Y3] }
      - { pin: OE4, active: low, outputs: [Y4] }
    table:
      Y1: "A1"
      Y2: "A2"
      Y3: "A3"
      Y4: "A4"
    tprop_ns: 9
    drive:
      hiz_when_disabled: true
```

`enables` is a LIST of `{ pin, outputs }` and not one part-wide enable pin
because the '125 genuinely has four. A model with a single OE switches all four
buffers together: it passes a one-gate test and is wrong on every board that
uses the '125 the way the corpus does — as four independently gated drivers onto
a shared net.

### The table is the shared expression grammar

A `table:` entry is compiled by the same [Phase C expression
parser](#reportedreg--the-word-a-register-would-put-on-the-wire) every Tier-2
rule guard uses — `! & | ^ ~`, parentheses, C precedence. Each bare pin name is
rewritten to the `var(NAME)` call the grammar already has, so a gate cannot grow
its own dialect of `&` and a malformed entry is refused at load with the same
error machinery.

The consequence a part author sees: **a pin role must be an identifier**
(`[A-Za-z_][A-Za-z0-9_]*`). The datasheets spell the '125's gates `1A` / `1Y` /
`1OE`, which start with a digit; the in-tree descriptors use `A1` / `Y1` / `OE1`
and the loader rejects anything else by name rather than letting it fail as a
parse error nobody can read. Use `!` and not `~`: `!` is the logical not a pad
wants, `~A` on a LOW pad is `-1`.

Each role binds to a pad through the `config:` key `<role lowercased>_pin` —
`A1` → `a1_pin` — so an eight-bit transceiver does not need a twenty-line
`pins:` block. `behavior.pins` / `behavior.output_pins` override it for a part
whose key is spelled differently.

### Three shapes, and exactly one per part

| block | part | what it says |
| --- | --- | --- |
| `table:` | combinational gate | `Y1: "!(A1 & B1)"` — output role → boolean function |
| `direction:` | bidirectional transceiver | `{ pin: DIR, a_to_b_when: 1, a: [A1…], b: [B1…] }`, paired by index |
| `select:` | 1-of-2 bus switch | `{ pin: S, low: [A1…], high: [B1…] }`, paired by index with `outputs:` |

Declaring none is a load error (the part would attach, drive nothing and look
like it worked); declaring two is a load error as well, because they are three
different parts and not three spellings of one.

A transceiver role appears in **both** `inputs:` and `outputs:`, and that is the
point: the same pad is read in one direction and driven in the other. Attach
binds both ends of it — the pin's output register (what the MCU drives) and its
input register (what the MCU samples) — so a DIR flip costs no pad resolution.

### `tprop_ns`, and the one-cycle floor

`tprop_ns` is converted to simulated CYCLES at attach, rounded up, with a floor
of **one cycle**. A zero-delay gate would let firmware store an input and read
the answer back inside the same instruction, which no real part does — and it
would make the answer depend on whether the bus happened to service the device
inside that store. One cycle is 12.5 ns at 80 MHz, the right order for the
5–15 ns this family specifies. A placement may override the clock the delay is
derived from with `config: { cpu_hz: … }`, which is how
`tests/logic_gate_74series.rs` walks a 9 ns delay one cycle at a time.

A `logic_gate` is serviced from BOTH the MMIO write hook (through
`edge_service_addrs`, so an input that moves and moves back inside one tick
interval is still seen) and the peripheral tick (so a deadline can EXPIRE with
nothing writing anything). It needs both; either alone is a part that answers
sometimes.

### ⚠️ What Hi-Z means on this twin

`drive: { hiz_when_disabled: true }` — the default, and the truth for every part
in this family — means a disabled output is **released**, not forced: the part
stops writing the pad.

It does NOT mean the pad floats to a pull-up. This engine has exactly two level
sources per pad — what the MCU drives out of a pin configured as an output, and
the external level a bus-resident device last applied — and `GpioPort` computes
the input word as "ODR for the bits driven push-pull, the latched external level
for the rest". There is no resistor network and no bus arbitration. So a
released output leaves the pad holding whatever level it last held: correct for
the case the '125 and the '245 exist for (the driver is off the wire and
something else owns the net), and an approximation when nothing else drives the
net at all, where silicon would drift to the MCU's internal pull and this model
stays put.

The observable in a test is therefore not "the pad went to X" but "the pad
stopped ANSWERING": move the input while the output is disabled and assert the
pad does not follow. Set `hiz_when_disabled: false` for the rare part whose
disabled output is actively pulled to 0; the engine then drives a LOW.

### ⚠️ What a `logic_gate` does not model

* **Contention.** Two parts driving one net is an electrical question; this
  engine has one level source per pad and the last writer wins.
* **Voltage domains.** A `74lvc1t45` level translator's whole purpose is VCCA ≠
  VCCB, and a level here is a bit. The direction and the isolation are modelled;
  the translation is a no-op, so a board a missing translator would break still
  runs.
* **A FET switch is not a buffer.** The `74cbtlv3257` is four pass transistors
  and is modelled as a one-way mux, because two pads cannot become one net here.
* Input thresholds, slew, drive strength, and the supply rail.

## `uart_device` — a part whose whole interface is a byte stream

Two shapes a register map cannot reach because the part has no registers: an
**AT command shell** (HC-05, SIM800L, every cellular modem) and an
**unsolicited stream** (a GPS emitting NMEA once a second). Both were
hand-written Rust, and the three models this primitive replaced were the same
file three times — the same line buffer, the same 128-byte cap, the same `poll`
that pops one byte. Only the command LADDER differed, and a ladder is a table.

```yaml
behavior:
  primitive: uart_device
  uart:
    baud: 38400                         # the datasheet's rate; diagnostic today
    frames:
      terminator: "\r\n"                # every byte of this ends a frame
      max_bytes: 128                    # a longer line is truncated, not grown
      ignore_case: true                 # the AT default
    responses:                          # tried IN ORDER; first match wins
      - { match: { prefix: "AT+VERSION" }, respond: "+VERSION:x\r\nOK\r\n" }
      - { match: "AT+NAME?",              respond: "+NAME:HC-05\r\nOK\r\n" }
      - { match: "AT",                    respond: "OK\r\n" }
      - { match: { prefix: "AT+" },       respond: "OK\r\n", delay_us: 90000 }
      - { match: any,                     respond: "ERROR\r\n" }
```

`match:` is `any`, a bare literal (which means EXACT), or `{ exact: … }` /
`{ prefix: … }`. There is no regex: a datasheet's command table is literals and
prefixes, and a regex in a part document is a second language with its own
failure modes. A response may also carry `do: [ … ]` — the same [`Action`]
vocabulary a rule uses — so a command that switches the part's mode answers
from the new mode.

⚠️ **A frame that is empty after trimming produces nothing.** `AT\r\n` is two
terminator bytes, so the `\r` completes the frame and the `\n` completes an
empty one; without that rule every command is answered twice.

### Unsolicited output, and templates

```yaml
  timers:
    - { name: sentence, period_us: 500000, start: on_reset }
  uart:
    unsolicited:
      - timer: sentence
        when: "var(idx) % 2 == 0"
        wrap: nmea
        template: "GPGGA,120000.00,{abs(input(lat)) / 10000000 * 1000000 + (abs(input(lat)) % 10000000 * 6 + 50) / 100:09.4},{input(lat) >= 0:char(N,S)},…"
  rules:
    - on: { timer: sentence }
      do: [ { var: idx, value: "var(idx) + 1" } ]
```

⚠️ **Every `unsolicited:` guard is evaluated BEFORE the timer's rules run**, so
two entries guarded `% 2 == 0` and `% 2 == 1` are mutually exclusive. If the
rule that increments `idx` ran first, the second guard would see the
incremented value and every tick would emit both sentences.

A **template** is literal text with `{EXPR}` or `{EXPR:FORMAT}` placeholders
over the ordinary integer expression language. The formats are:

| spelling | meaning |
|---|---|
| none, or `d` | plain decimal |
| `W.P` / `0W.P` / `.P` | fixed point: the integer is a count of `10^-P`, printed with `P` decimals, zero-padded to `W` characters total |
| `WX` | uppercase hex, zero-padded to `W` |
| `char(A,B)` | one character: `A` when the value is non-zero, else `B` |

**No float ever crosses the boundary**, which is what makes a rendered sentence
bit-identical on native and wasm. An NMEA position is `DDMM.mmmm`: the channel
declares `expr_scale: 10000000` so `input(lat)` is degrees × 1e7, integer
arithmetic converts to 1e-4 minutes, and `{…:09.4}` prints it. `abs()` takes
the magnitude and `char(N,S)` carries the hemisphere, because that is how the
sentence is shaped — a number and a sign in two different fields.

`wrap: nmea` is the one framing the engine knows: `$`, the payload, `*`, the
two uppercase hex digits of the XOR over the payload, CRLF. It is a key rather
than something a template could contain because the checksum is over the
template's own OUTPUT.

## FIFO streams

The shape a register map cannot fake: a queue the part fills on its own clock
and firmware drains. The depth and the overflow policy are the whole point — a
model that always hands back the newest sample passes firmware that never
drains fast enough, which is precisely the CPU-starvation bug worth simulating.

```yaml
  fifos:
    - name: samples
      depth: 32
      overflow: drop_newest            # "collects up to 32 values and then stops"
      fill:
        timer: sample                  # the SAME timer a rule may listen for
        when: "field(FIFO_CTL.FIFO_MODE) != 0"
        pack:                          # one entry, MSB-first, 63 bits max
          - { expr: "reported(DATAX0)", width_bits: 16 }
          - { expr: "reported(DATAY0)", width_bits: 16 }
          - { expr: "reported(DATAZ0)", width_bits: 16 }
      count: { register: FIFO_STATUS, field: ENTRIES }
      watermark:
        entries_from: { register: FIFO_CTL, field: SAMPLES }
        set: INT_SOURCE.WATERMARK

    # …and the registers that drain it:
      - { name: DATAX0, addr: 0x32, …, fifo: { name: samples, slot: 0 } }
      - { name: DATAY0, addr: 0x34, …, fifo: { name: samples, slot: 1 } }
      - { name: DATAZ0, addr: 0x36, …, fifo: { name: samples, slot: 2, pop: true } }
```

⚠️ **Bypass mode needs no second switch.** A `fifo:` register serves the
queue's oldest entry while the queue is NON-EMPTY and falls through to its live
`source:` when it is empty. In bypass the fill guard is false, so nothing is
ever queued, so the register reports the live conversion — byte for byte what
the part did before the FIFO existed. Get the fill guard right and the read
path is right for free.

⚠️ **`pop: true` goes on the LAST register of the burst.** A driver that
abandons the burst earlier gets the same sample again, which is what silicon
does with a read that never completed — and the trap a pop-on-first-byte model
would hide.

⚠️ **The watermark FOLLOWS the depth** unless the part declares `latch: true`.
That is what makes a driver's "drain until the watermark drops" loop terminate.

### `reported(REG)` — the word a register would put on the wire

`reg(NAME)` is the register's STORED word. For a measurement register that is
its reset value forever: nothing writes it, because the value is computed at
read time from the stimulus channel. `input(KEY)` is not the same thing either
— it borrows only the register's `encode:`, so a part whose counts-per-unit
comes from `scale_from` (the ADXL345's range bits) gets the raw engineering
value instead of the count.

`reported(NAME)` is the word the register would put on the wire right now,
through the same function the read path uses. A FIFO that packs what the data
registers report, and an alarm that compares against the clock the time
registers report, both need this and nothing else will do.

### The stream parts that did NOT become data, and why

`adxl345.yaml` is the FIFO primitive's proof part. Four more parts were looked
at for this round and none of them is a FIFO port; each is named here with the
reason, because "not yet ported" and "there is nothing there to port" are very
different facts.

- **BMI270** — **has no FIFO at all.** The shipped Rust model answered
  `CMD_FIFO_FLUSH` with a comment that says `no FIFO modelled`, and there is no
  queue, no watermark and no `FIFO_LENGTH` behind it. Porting it was a Tier-1 +
  Tier-2 register job (the config-load handshake gate, the paged FEATURES
  window, `scale_from` over `ACC_RANGE`/`GYR_RANGE`), not a stream job — and
  that job is now **done**: `bmi270.yaml`, byte-identical, the `stream:` key
  below. It still provides no FIFO coverage, which is why it stays named here.
- **MAX30102** — a real 32-deep FIFO, and still not portable as data, for two
  independent reasons. Its samples are **synthesised in Rust**: the model runs
  a seeded LCG to shape a photoplethysmogram with a systolic upstroke, a
  dicrotic notch and a diastolic decay. `fills[].pack` packs EXPRESSIONS over
  stimulus channels; it cannot generate a waveform, and a port that dropped the
  waveform would be a different part wearing the same `device_type`. Second,
  its sample clock is a stated **thunk** — the model's own header calls
  advancing one sample period per completed I²C transaction "a deliberate
  stand-in for the missing clock hook, not silicon behaviour". A declarative
  port has a real timer and would therefore not be a parity port. A waveform
  primitive is the honest unblock, and it is a primitive, not a key.
- **SX1278 / RA-02** and **nRF24L01+** — SPI register **shells with no air
  link**, no FIFO, and no IRQ pin (`no RF air link`, `no air link`, in their own
  first lines). 138 and 191 lines each, nearly all of it a register array behind
  an address/data phase machine.

  ⚠️ This entry used to say porting them "would move a stub, not a model", and
  that was the wrong call. A stub in Rust is engine code: it ships in every
  binary, only a Rust programmer can change it, and the declarative engine has
  to stay bug-compatible with it forever. A stub in YAML is three lines of
  `register_file:` that a customer can fork. They are **ported** —
  `lora_sx1278.yaml`, `nrf24l01.yaml`, `rc522.yaml` — and what is still missing
  is stated in each descriptor's own header rather than in this list. The rule
  that survives is the one about the FAKE: nothing invents a packet, a tag or an
  RSSI, and `nrf24l01.yaml` says in its first paragraph that a `write()` waiting
  on TX_DS waits forever.
- **MCP2515** — the SPI and register half is expressible, but the part's reason
  to exist is `attach_can_bus`: a `Sender`/`Receiver` pair of `CanFrame`s the
  engine hands it, plus `poll_external_bus`. That is an engine SEAM, not a
  descriptor key. A pack could declare `bus: can` and have the engine wire it,
  which is the shape to build — and it is a primitive-level change with its own
  attach contract, so it is named here rather than half-done.

## Register-shell keys

A **register shell** is a part whose datasheet map is a few meaningful registers
in a large space of storage the driver configures and reads back — and whose
interesting behaviour (a radio, an RF field) is not modelled at all. Five
shipped models were exactly that: the SX1278's 128 bytes, the MFRC522's 64, the
nRF24L01+'s 24, plus the two shells' framing quirks. The keys below are what it
took to make all of them data, and each is named with the datasheet sentence
that forced it.

### `spi.register_file` — flat RAM behind the declared map

Every command-byte address that no `registers:` entry covers is one byte of this
array: a read serves it, a write stores it. Absent ⇒ an undeclared address reads
`0xFF` (open bus) and swallows writes, which is what every descriptor written
before this key meant.

```yaml
spi:
  registers:
    - { name: RegVersion, addr: 0x42, width: 1, endian: be, access: r, reset: 0x12 }
  register_file:
    size: 0x80          # cells; an address at or above it is not backed
    fill: 0x00          # optional: what every cell powers up holding
    reset: { 0x01: 0x09 }   # sparse power-on values, stamped over `fill`
```

Declared registers still **win** at their own addresses, so a part may mix the
two: the nRF24L01+'s `STATUS` is a `write_one_to_clear` register and the other
twenty-three addresses are storage.

Why a file rather than one `RegisterSpec` per address: an address left
undeclared is not a blank. It reads `0xFF` and drops the driver's write, which is
a different part — so the alternative is inventing a name for 128 addresses,
127 of which the datasheet calls reserved.

It is deliberately **not** the I²C `register_file:`. That one owns a write
POINTER (`pointer_mask`, `first_write_after_start_sets_pointer`,
`auto_increment`) because an I²C register-file part selects its address with a
bus write. A SPI part's address comes out of the command byte and its walk is
`framing.auto_increment`, so those three keys would be dead fields a descriptor
could set and have ignored.

Proved by **`lora_sx1278.yaml`**, **`rc522.yaml`** and **`nrf24l01.yaml`**.

### `spi.framing.op_mask` / `op_read` / `op_write` — an OPCODE command byte

`mosi & op_mask` selects the operation and is compared against `op_read` and
`op_write`. A command byte matching NEITHER selects no register at all: its data
phase serves `command_response` (or `0xFF`) and **drops writes**.

```yaml
framing:
  op_mask: 0xE0
  op_read: 0x00       # R_REGISTER is 000A AAAA
  op_write: 0x20      # W_REGISTER is 001A AAAA
  addr_mask: 0x1F
```

It WINS over `rw_bit`. `rw_bit` carries a non-`None` default, so there is no way
to tell a defaulted one from a declared one and "declaring both is an error"
would reject every descriptor that sets `op_mask`; the op field is the more
specific statement of the same datasheet sentence, so it decides.

Proved by **`nrf24l01.yaml`** (§8.3.1, Table 19). Decoded by bit 5 alone — the
only direction vocabulary the engine had — `W_TX_PAYLOAD` (1010 0000) is a WRITE
to address 0x00, so a 32-byte payload burst walks its payload over CONFIG,
EN_AA, EN_RXADDR, SETUP_AW, SETUP_RETR, RF_CH and RF_SETUP. The register file is
silently destroyed by the command that sends a packet.

### `spi.framing.command_response` — the word clocked out during the command byte

Names the register whose word rides out on MISO while the master clocks the
command word in. Absent ⇒ `0x00`, the byte every descriptor returned there
before.

nRF24L01+ §8.3.1: "the STATUS register is serially shifted out on the MISO pin
simultaneously with the command word on MOSI". Every RF24-style driver reads its
interrupt flags that way — `write_register()` returns the byte the command phase
produced — so a part answering `0x00` there reports that no interrupt has ever
fired.

### `registers[].stream` — a port that holds the auto-increment pointer

The byte-wise auto-increment pointer does not advance past this register. The
register IS the port; what moves is an internal address counter the master
cannot address. It holds the pointer in **both** directions, because that is
what a port is.

```yaml
- { name: INIT_DATA, addr: 0x5E, width: 1, endian: le, access: rw, stream: true }
```

Proved by **`bmi270.yaml`**. Bosch's initialisation sequence streams the ~8 KB
feature-engine image into `INIT_DATA` in one burst, with `INIT_ADDR` advancing
inside the part. ⚠️ A pointer that stepped per byte would walk that one
transaction over the whole map thirty-two times — over `ACC_CONF`, over
`PWR_CTRL`, and over `CMD` (0x7E), where **one byte in every 256 of a firmware
image is `0xB6`, which is SOFTRESET**. The upload would reset the part it is
initialising, repeatedly, and the handshake it exists to satisfy could never
complete. A part's FIFO data register (the BMI270's own `FIFO_DATA`, 0x24) has
the same shape.

## Framed parts: a message instead of a register

Some parts have no register map at all. A MAX7219 is **written and never read**;
a 74HC595 has no addressable anything. Their unit of work is a *message*, so a
`spi_device` descriptor may declare `frames:` plus `rules:` and omit `registers:`
and `register_file:` entirely. `framing:` is not consulted for such a part, and
it presents `0x00` on MISO.

Inventing a register map for one of these is worse than having none: the command
phase would eat the first byte of every frame.

### `frame_byte(N)` — a rule reads the frame's own bytes

`on: frame` hands a rule `written`, which is ONE byte: the one that CLOSED the
frame. A MAX7219 transaction is `[address, data]`, so its address byte — and
with it all thirteen registers — was unreachable from a rule. `frame_byte(N)` is
byte N of the frame being handled, MOSI order.

```yaml
behavior:
  primitive: spi_device
  spi: {}                       # no register map: this part is written, never read
  frames:
    length: 2
    opcode_byte: true           # byte 0 is ALSO recorded in var(opcode)
    discard_partial: true       # a short frame is dropped, not decoded
  vars: { opcode: 0, d0: 0, intensity: 0 }
  rules:
    - on: frame
      when: "(var(opcode) & 0x0F) == 0x01"
      do: [{ var: { name: d0, value: "frame_byte(1)" } }]
    - on: frame
      when: "(var(opcode) & 0x0F) == 0x0A"
      do: [{ var: { name: intensity, value: "frame_byte(1)" } }]
```

| key | meaning |
|---|---|
| `frames.length` | fixed frame length in bytes. Absent ⇒ the frame ends at the transaction boundary only. |
| `frames.opcode_byte` | byte 0 is also recorded in `var(opcode)`. The part MUST declare `opcode` in `vars:`, or it is a load error. |
| `frames.discard_partial` | a transaction boundary that finds fewer than `length` bytes clears them and raises NOTHING; CS↓ clears them too. Default false. |

⚠️ **`frame_byte(N)` and `var(opcode)` are not redundant.** `frame_byte(0)` is
live only while the frame that carried it is being handled; `var(opcode)`
PERSISTS, so a part whose command byte decides what the NEXT frame means can
still answer. `frames.opcode_byte` has documented exactly that since it was
declared — it simply had no implementation until now.

⚠️ **The index is a LITERAL, checked at load** against the declared
`frames.length`. `frame_byte(2)` of a two-byte frame is a load error, and so is
`frame_byte()` in a part that declares no `frames:` at all — the same strictness
`pin(NAME)` gets, for the same reason: an unchecked index reads 0 forever, which
is a guard that is quietly always-false and looks exactly like a part the
firmware never clocked.

⚠️ **Why `discard_partial` is not the default.** A truncated command shell must
be SEEN and rejected, which is what the transaction-boundary frame exists for. A
fixed-width shift register is the opposite: eight clocked bits of a sixteen-bit
MAX7219 write are not half a write, they are a frame that never happened.
Delivered as a frame, a stray odd byte decodes its low nibble as a register
address and writes a zero data byte into a digit register — a row going dark
because of a byte the part never latched.

### `outputs:` on a `spi_device` — a bus part that drives PADS

A 74HC595 is an SPI part whose whole output is eight pins. `outputs:` and
`output_pins:` are the same keys a `gpio_device` uses: each role binds to a
`config:` key at attach, a `{ pin: QA, level: … }` action queues a transition,
and the per-tick pass drains the queue through the narrowed `DevicePins` port —
both seams, exactly as an I²C part's INT line goes out.

```yaml
  outputs: [QA, QB, QC, QD, QE, QF, QG, QH]
  output_pins: { QA: qa_pin, QB: qb_pin, QC: qc_pin, QD: qd_pin,
                 QE: qe_pin, QF: qf_pin, QG: qg_pin, QH: qh_pin }
  rules:
    - on: frame
      do: [{ var: { name: shift_reg, value: "frame_byte(0)" } }]
    - on: cs_release                      # RCLK↑ latches
      do:
        - { pin: QA, level: "var(shift_reg) & 0x01" }
        - { pin: QB, level: "var(shift_reg) & 0x02" }
```

⚠️ **A role whose `config:` key the placement does not set is SKIPPED, not an
error.** A board that leaves QD unconnected is an ordinary board; the rule still
runs and that line goes nowhere. It is also what keeps every placement written
before the descriptor existed working unchanged.

⚠️ **The pads move on the TICK**, not inside the transfer — the drain is one
pass per peripheral tick, the same one an I²C interrupt line rides.

### `powered:` on a `spi_device` / `gpio_device`

The `powered` config key (ABSENT MEANS POWERED — see `components::supply`) is
now honoured by both primitives. An explicit `powered: false` refuses the bus at
`transfer` / `service`, so the part stays at its power-on values **by
construction** rather than being blanked at readback: digit RAM never
accumulates, a timer never ages, and no pad is driven. The artifact is still
published, stamped `"powered": false`, because "dark" and "no evidence" are
different findings.

⚠️ An unpowered SPI part clocks out `0xFF`, not `0x00`. A chip with no rail
drives nothing, so the master samples the idle bus — the same all-ones this
engine reports everywhere else that means "nothing is answering".

### The register and command shells that did NOT port, and why

Five more were looked at this round. Each is named with the missing primitive,
because "not yet ported" and "there is nothing there to port" are different
facts — and so is "the model fakes it, and the fake is what you would be
porting".

- **SPS30** — a Sensirion command shell, and two things in it are not data.
  Its measured values are **IEEE-754 `f32`** on the wire (`value.to_be_bytes()`,
  datasheet §5.3.2), and a `response[]` word is an integer encoding. Worse, the
  response SHAPE is chosen by a parameter word the driver sent earlier:
  `start_measurement(0x0300)` makes each value two words plus two CRCs and
  `0x0500` makes it one word plus one, so the same opcode answers 60 bytes or 30
  depending on stored state. The primitives are a float response word and a
  response set selected by a `state:`; both are real, neither exists.
- **PN532** — an I²C command shell whose command is not at a fixed offset: the
  model scans the whole write stream for the byte pair `D4 02` anywhere in it
  and answers with a 19-byte literal ACK + firmware frame. `commands:` matches a
  fixed-width opcode at the head of the transaction and answers in 16-bit words.
  The missing primitive is the `uart_device` `responses:` table — a pattern match
  and a literal byte string — on an I²C transport. The RF field is not modelled
  either way: `PICC_IsNewCardPresent()` finds nothing because there is nothing
  in the field to find.
- **MLX90640** — 16-bit addressing (`pointer_width: 2`) is already a key, and
  the rest is not. Its 832-word EEPROM is a **self-consistent linearised
  calibration set computed in Rust**, chosen so the unmodified Melexis driver's
  `ExtractParameters` + `CalculateTo` collapses to an invertible `count ↔ °C`
  relation; its 768-word RAM is the thermal scene pushed back through that
  inversion, per pixel. `register_file.fill` fills an array with a constant and
  `reset:` stamps single cells — neither computes 1600 words from a scene. The
  primitive is a 2-D stimulus **grid** with a per-cell computed source, and it is
  a primitive, not a key.
- **DRV2605L** — its time base is a stated **thunk**, the same disqualification
  MAX30102 has. The model implements no `advance_time_us`: playback moves only
  when a caller invokes `advance_us` by hand, and the header says so ("Nothing
  advances on its own: a haptic effect started and never stepped stays
  asserted"). A declarative port has a real timer, so it would not be a parity
  port — it would be a different part that happens to answer the same probe. The
  effect library is a second, smaller problem: TI does not publish the ROM
  waveforms' durations or amplitudes, so the model's table is a stated
  approximation rather than data anyone can check.
- **lipo_charger** — ✅ **PORTED.** It was listed here because its pin voltage
  is computed from **two** channels rather than looked up on a curve over one:
  `3300 + 9 × soc_pct`, plus a 150 mV charge bump **iff** `usb_present ≥ 0.5`,
  clamped to 4200 mV, then integer-divided by the ÷2 divider. The three gaps
  named here now exist as general keys — `analog.formula` over a `derived:`
  channel, `derived[].when` (a threshold on a boolean channel), and
  `analog.encode: trunc` — and each is used by more than this part, which is
  what "inventing all three for one part" was the objection to. See
  `lipo_charger.yaml` and `analog_plant_migration_parity`.

Three more were looked at in the register-shell round that ported `vl53l1x`,
`bno055` and `bmp280`, and each is blocked on something specific:

- **AHT20** — ✅ **PORTED.** All three gaps are now general keys:
  `crc8.covers: { bytes: 6 }` (one checksum over the first N ANSWER bytes),
  `response[].fields` (a packed word up to 8 bytes whose fields straddle byte
  boundaries — the shared nibble of byte 3 carries humidity[3:0] AND
  temperature[19:16]), and `i2c.not_ready_byte` (the byte a command device
  answers while a delayed response is cooking; `0x88` = BUSY | CAL here, where
  the undeclared default is open bus).

  ⚠️ The BUSY **thunk is gone, and it was load-bearing.** The deleted model's
  own header said "we don't actually model elapsed time" and cleared BUSY after
  two status READS. The descriptor uses `delay_us: 80000` — AHT20 rev 1.1 §5.4's
  measurement time — on the same simulated microsecond clock every other delayed
  part uses. That **broke `examples/nucleo-f407-i2c`**, which polled sixteen
  times back-to-back (about 0.2 ms) and passed only because the model counted
  reads; that firmware would have failed on the bench. It now waits the 80 ms,
  and `aht20_migration_parity.rs` asserts both halves — BUSY at 79 999 µs, clear
  at 80 000 µs, and NOT clear after a thousand reads that spend no time.

  The measurement is no longer a constant either: `temperature` and `humidity`
  are ordinary stimulus channels and the checksum is computed over what the twin
  actually answered, which is what `response[].fields` bought. `aht20.rs` is
  DELETED rather than kept as an oracle — two of the things it did are the two
  things this port deliberately changes, so an oracle in `components/` would be
  asserting them.
- **BME280** — **STOPPED, and the blocker moved.** The plan was a general
  `derived[].invert: { of: <forward expr>, over: [lo, hi], tol }` — a
  deterministic bisection so a descriptor states the FORWARD datasheet formula
  and the engine inverts it, which is exactly what `invert_t` / `invert_p` /
  `invert_h` in `components/bme280.rs` do by hand against the exact
  `BME280_compensate_*_int32` reference code.

  Inversion is the *easy* half and `invert:` would be general. What stops the
  port is the arithmetic the forward expression itself is written in.
  `derived:` evaluates in **f64**, and Bosch's `_P_int64` does not fit:

  - `((1i64 << 47) + var1) * dig_P1` — with the shipped `dig_P1 = 38221` that
    product is `38221 × 2^47 ≈ 5.4e18`, about `2^62.2`. An f64 mantissa is 53
    bits (`2^53 ≈ 9.0e15`), so values that size are representable only to
    within about 1024, and the very next step is an arithmetic `>> 33` whose
    FLOOR flips at the boundary.
  - `(((p << 31) - var2) * 3125) / var1` — `p` reaches `2^20`, so the numerator
    reaches `≈ 2^62.6`. Same problem, same place.

  The temperature and humidity halves DO fit (their reference code is `i32`,
  peaking at 419 430 400) and would need only `floor()` added to the expression
  grammar to spell the arithmetic shifts. So a `derived[].invert` that landed
  today would port two of the part's three channels and leave the third
  hand-written — which is not a parity port, it is a part that is half a
  descriptor.

  ⚠️ The honest next step is an **integer/fixed-point evaluator** for `derived:`
  (i64 with explicit shift and floor-division), and `invert:` on top of that.
  Written down rather than half-built.

  ⚠️ Separately: `bme280.rs` would NOT be deletable even after a port.
  `crates/core/src/peripherals/nrf52/serial_instance.rs` attaches
  `Bme280::new(0x76)` as a generic slave in `twim_path_reads_bme280_chip_id`, so
  it would take the `bmp280.rs` route and move to the coverage ratchet's
  EXCLUDED list as a byte-parity oracle. Checked, and said, because the question
  changes what a port is worth.

  `bmp280.yaml` ported anyway, because that model answers constants and inverts
  nothing; its header says so.
- **SN74HC165** — ✅ **PORTED.** It was listed here because of the placement
  key: the kit takes `inputs: 165`, ONE integer that seeds all eight channels at
  once, and `examples/iolink-dido` plus three `iolink-station` manifests set it,
  while descriptor seeding was one `config:` key per channel carrying a float —
  so `inputs:` would have parsed and silently done nothing. That gap is now the
  general key `metadata.inputs[].bits:` (below), and the wire shape is the
  `spi_device` the entry predicted: `framing: { command_bytes: 0 }` plus one
  byte-wide register whose eight one-bit `fields:` read the eight channels, the
  exact shape `max31855.yaml` has. `crates/wasm/src/inputs.rs`
  (`get_sn74hc165_inputs`) now reads the byte back through
  `GenericSpiDevice::input_value` instead of downcasting to the struct — the
  same move `sim_input.rs` made for the ported I²C parts, and the one that stops
  the accessor answering "no shifter wired" the day a part becomes a descriptor.
  See `sn74hc165.yaml` and `sn74hc165_migration_parity.rs`.

### The pin-driven parts that did NOT port, and why

- **Push button / contact** — **STAYS RUST, and here is exactly why.** Checked
  against the resident-device path, which is the thing that would have made it
  portable: `config:`/placement cannot supply either of the two per-placement
  properties, because a button has no `external_devices` entry to carry them.

  1. **It is materialised from `board_io:`, not from `external_devices:`.**
     `SystemBus::attach_board_io_buttons` (`bus/from_config.rs`) walks
     `manifest.board_io` for `kind: button, signal: input`, resolves the named
     PERIPHERAL, and builds a `Button` addressed by `(peripheral base, pin
     index)`. A `gpio_device` descriptor resolves its pads from a `config:` key
     holding a PAD LABEL (`"PC13"`), and a `board_io` binding has no `config:`
     block and no pad label — it has a peripheral name and an integer. Two
     different addressing schemes, and the descriptor path speaks only one.
  2. **The stimulus channel KEY is per placement.** The binding picks one of six
     words (`pressed`, `obstacle`, `field`, `vibration`, `motion`, `touch`) so a
     PIR is the same contact under a word an agent can script blind.
     `metadata.inputs` is static per descriptor, so one descriptor is one
     channel name. `bits:` (above) fans one entry into MANY channels, but they
     are all present at once — it does not let a placement CHOOSE which one
     exists, and it should not: a part whose channel list depends on its wiring
     is not a part, it is six parts.
  3. **`active_high` is derived from the diagram**, not from the part. That one
     alone would be an ordinary `config:` key.

  So the port is a change to the `board_io` attach path first — pads addressed
  by peripheral + index, and a placement-chosen channel name — and a descriptor
  second. Neither belongs inside a parity port, and doing (3) alone would ship a
  descriptor that cannot replace the model.
- **4×4 keypad** — **STOPPED, and the entry needs a correction.** List-valued
  `pins:` roles are NOT the missing thing: `SystemBus::pin_list_config`
  (`bus/declarative_device.rs`) already reads a role whose `config:` value is a
  list, which is how `keypad.yaml`'s `rows: row_pins` / `cols: col_pins` resolve
  today. What it is not is GENERAL — it hardcodes `const EXPECTED: usize = 4`
  and its error strings say "keypad", so it is a keypad-shaped special case
  living inside `attach_matrix`, reachable by no other primitive.

  The two real blockers, measured:

  1. **`gpio_device` binds ONE pad per role.** `behavior.pins` and
     `behavior.output_pins` are `BTreeMap<String, String>` — role → one
     `config:` key holding one pad LABEL. A list-valued role would have to fan
     out the way `metadata.inputs[].bits:` (above) fans out channels: one
     declared role becoming `row0..row3`, each resolved from index *i* of the
     list under one key. That is the same trick and would be general; it is not
     written.
  2. **A rule cannot address a pad by INDEX.** `Event::Pin { name, edge }`,
     `Event::Pins` and `Action::Pin { name, level }` all carry a bare `String`,
     and the load-time name validation checks it against the declared role set.
     `pin(row[i])` does not parse, so even with (1) the sixteen-key scan would
     have to be written out as sixteen rules over eight flat role names.

  Doing it as eight flat roles and eight `config:` keys — which is possible
  today — changes the emitted `external_devices` block from two LIST keys to
  eight scalars, on both engines and in every shipped placement. That is a
  migration, not a parity port, and it makes the descriptor WORSE at describing
  the part: a keypad's rows are a set, and a schema that cannot say so is the
  thing to fix.
- **Rotary encoder** — the two observable questions are **SETTLED and PINNED**
  (`crates/core/tests/rotary_encoder_semantics.rs`); the port is still open on a
  third thing, named below.

  1. **Where the cadence anchors.** Settled: on the first SERVICED tick after a
     retarget, because the invariant that matters is that *no inter-edge gap is
     ever shorter than one interval, the first one included*. An EC11's phase
     figures are all MINIMUM durations, so a short phase is not a faster knob —
     it is a phase a debouncing decoder may legitimately drop. The test measures
     that invariant at four different sub-interval stimulus offsets, which is
     precisely where a free-running `timers:` grid gets it wrong.
  2. **`set_input` rounds where `input()` truncates.** Settled: ROUND, on both
     sides. A detent is a discrete mechanical stop — there is no shaft position
     2.6 detents from the origin — and truncation additionally biases the knob
     toward zero, so half a detent clockwise counts and half a detent
     anticlockwise does not. The test asserts the symmetry as well as the
     rounding. ⚠️ That makes `input()`'s truncation the thing a port must
     change, which is worth having written down before someone "fixes" it by
     making `set_input` truncate to match the engine.

  **Still open:** a descriptor's `timers:` has no way to RE-ANCHOR on a stimulus.
  `start: on_reset` is a free-running grid and `start_on_write:` is keyed to a
  REGISTER write, which a part with no registers never sees. Answer 1 above says
  the anchor must move when the target does, so the port needs a timer that a
  `set_input` can restart — a general key (`timers[].restart_on_input:`, say)
  that does not exist yet. Named rather than approximated by a grid.
- **DHT22 / AM2302** — **STOPPED.** The diagnosis stands (the model precomputes
  83 absolute edge times and answers `sensor_high_at(cycle)` by binary search;
  the information is in 27 µs vs 70 µs HIGH pulses after a 50 µs LOW slot, which
  a tick-driven `timers:` cannot resolve). The plan was to generalise the
  HC-SR04's existing edge-deadline path rather than invent a second scheduler.
  Measured, that path is further from an edge SCHEDULE than its name suggests:

  - **`HcSr04::take_edge_schedule` returns exactly two cycles**, `(rise, fall)`
    — one pulse window, not a list — and `next_edge_deadline_cycle` likewise
    hardcodes `[rise, fall]`. A DHT22 frame is 83 edges, so this is not a
    generalisation of a list; it is the introduction of one.
  - **It is not a trait.** `SystemBus::apply_hcsr04_event(sensor: usize)` indexes
    a concrete `Vec<HcSr04>` on the bus (`bus/device_hooks.rs`). Every
    bus-resident device that wants a deadline would first have to reach it
    through `BusResidentDevice` instead.
  - **It is `#[cfg(feature = "event-scheduler")]`.** With the flag off the path
    does not exist, so a DHT22 built on it alone would be a part that works on
    one build configuration — and the per-tick fallback is exactly what cannot
    resolve 27 µs from 70 µs.

  So a declared `schedule: [{ level, us }…]` emitted by an `emit_schedule` rule
  action needs all three of those first: an N-edge list, on the resident-device
  trait, with a tick-driven fallback that is honest about its resolution.
  ⚠️ And porting `hc_sr04.yaml` onto it is then part of the same change, not a
  follow-up — two schedulers for one concept is the thing to avoid.

## `timers[].period_from` — a field-driven timer period

A sample rate is a REGISTER on nearly every part that has one, and a constant
`period_us` is right for exactly one setting of it.

```yaml
  timers:
    - name: sample
      period_us: 10000               # what the source register's RESET value gives
      start: on_reset
      period_from:
        register: BW_RATE
        field: RATE                  # or `mask:` + `shift:`
        table:                       # field value → period in µs
          0x9: 20000                 #   50 Hz
          0xA: 10000                 #  100 Hz — the reset value
          0xD: 1250                  #  800 Hz
```

A **table** rather than a formula because that is the shape of the datasheet:
these are enumerations with footnotes. A part whose rate genuinely is a formula
over a wide field (the MPU6050's 8-bit `SMPLRT_DIV`) does not fit and is named
as still-blocked rather than approximated by a 256-row table.

An **unmapped** field value is NEUTRAL — `period_us` stays in force, the same
rule `scale_from` and `clamp_from` have — so a reserved encoding cannot
silently stop the part's clock. A RUNNING timer whose period changed is
re-anchored to `now + the new period`: firmware that rewrote the rate register
restarted the divider.

## `set_input:` — a rule that assigns a stimulus channel

Every other action changes something firmware can see through the wire. This
one changes what the part MEASURES, which is the only way a part can hold a
quantity that moves on its own clock.

```yaml
  timers:
    - { name: tick, period_us: 1000000, start: on_reset }
  rules:
    - on: { timer: tick }
      when: "field(CONTROL.EOSC) == 0"
      do: [ { set_input: unix_time, value: "input(unix_time) + 1" } ]
```

`value:` is in the same domain `input()` reads back, and `set_input` is its
exact inverse — a rule that writes back what it read changes nothing.

⚠️ **It does NOT raise `on: { input: KEY }`.** A rule that fed its own trigger
would be a loop, and the machine's recursion guard would drop the re-entry
silently rather than run it. A HOST driving the channel still raises the event,
because that is an outside event.

## What a pack cannot do

A pack is data interpreted by a **primitive** — `i2c_device`, `spi_device`,
`analog_source`, `display`, `led_strip`, `gpio_device`, `uart_device`,
`quadrature`, `matrix`, `one_wire`, `pulse_echo`.
Those primitives are the irreducible timing algorithms, and they live in Rust in
this repository.

`analog_source` is the primitive for parts whose whole interface is one
analogue voltage (a Sharp IR ranger's `Vo`, an MQ-x module's `AOUT`): the
descriptor carries the datasheet's output rule and the engine owns the rest
(SimInput plumbing, mV→ADC count, attach).

The output rule is **a curve or a formula, never both** — a part described by
two would have two answers for the same pin, and a load error says so rather
than a precedence rule picking one.

* **`curve:`** is the shape when the datasheet publishes a GRAPH: `(input, mV)`
  points, piecewise-linear between neighbours, plus stated out-of-band rules
  (`below_first: clamp`, `above_last.floor_mv`). The proof part is
  `gp2y0a21.yaml`, whose typical-output graph is exactly a table. A straight
  line is the degenerate case and its two endpoints are the whole curve —
  `potentiometer.yaml`, `mq6.yaml` and `soil_moisture.yaml` are each two rows.
* **`formula:`** is the shape when it publishes an EQUATION. Same grammar as
  `behavior.derived`, evaluated over the stimulus channels and any `derived:`
  names, producing millivolts:

  ```yaml
  behavior:
    primitive: analog_source
    derived:
      - name: r_ntc_ohm
        expr: "10000 * exp(3950 * (1 / (temperature + 273.15) - 1 / 298.15))"
    analog:
      formula: "min(max(3300 * 10000 / (r_ntc_ohm + 10000), 0), 3300)"
      encode: trunc
  ```

  Why not sample the equation into a table: it is lossy exactly where the
  equation is steep. The CdS power law behind `ldr.yaml` needs points ~0.0004 lx
  apart near darkness to stay inside one ADC LSB (0.806 mV), and the NTC beta
  equation needs ~3 °C spacing across its whole span — a table nobody can check
  against the datasheet, standing in for three constants anyone can. **A curve
  when the datasheet drew one, a formula when it wrote one.**

  `below_first` / `above_last` are CURVE rules and are refused alongside a
  formula: an expression is defined everywhere its channel range reaches, so the
  bound belongs in the expression (`min`/`max`) where it can be read.

Two functions exist in the expression language only because these parts needed
them, and both are named in the datasheets that forced them: **`pow(a, b)`** (the
CdS cell's `R = R₁₀ · (lux/10)^-γ`) and **`exp(x)`** (the NTC's beta equation).

`analog.encode: trunc | round` states how the real millivolt value becomes the
integer count the pin reports. `trunc` is the default because it is what every
analog model in this tree did (`v as u16`); a default that rounded would have
moved a shipped part's reading by 1 mV over half its range with nobody asking.

**More than one stimulus channel.** A part may declare several — `lipo_charger`
reads state-of-charge AND whether the charger is plugged in. With more than one
the descriptor must say which value reaches the pin: a `formula:` names its
channels itself, and a `curve:` needs `source:` to say which channel the table
is indexed by. Inferring one would silently ignore the rest.

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

**E-paper is where a framebuffer stops being one**, and the four keys the two
tri-colour panels forced are the four ways it differs. They are stated, never
inferred, and `ssd1680_tricolor_290.yaml` / `uc8151d_tricolor_290.yaml` are the
proof parts.

| key | the fact it states | what a house default would have done |
|---|---|---|
| `ram.planes: [black, red]` | Two independent 1-bpp RAMs selected by the command that opens the stream (SSD1680 0x24/0x26, UC8151D DTM1/DTM2). `ram_write` names one. The artifact payload is the planes concatenated, with `plane_bytes` giving the split. | One frame memory composes the two into a picture the model never produced — and a stream defaulting to the first plane paints the red image in black. |
| `ram.blank: 0xFF` | The ERASED byte, and therefore what an ink count treats as no ink. A set bit on e-paper is no ink; an OLED's GDDRAM is the other way round. It is also what `clear_ram` fills with. | A default of 0 reports a blank panel as fully inked and a cleared one as blank — backwards on both counts. |
| `ram.units: { col: bytes, row: pixels }` | What ONE STEP of each address counter covers. SSD1680 0x44 takes the X window as start/8 and 0x45 takes Y as raw pixel rows. | Reading both in pixels streams a plausible byte count into the wrong rows, with no error anywhere. |
| `refresh` + `refresh_generation` | FRAME MEMORY IS NOT THE SCREEN. SSD1680 0x20 and UC8151D 0x12 are what move the ink; until one arrives the glass still shows the previous image. The action latches RAM into a separate screen and bumps the counter `labwired_verify`'s `min_refresh_generation` clause resolves against. `{ plane: black, of: screen }` publishes the ink on the GLASS, which disagrees with `{ plane: black }` exactly when firmware wrote a frame it never activated. | Reporting frame memory as the picture passes firmware that wrote a perfect frame and never refreshed. |

Two more keys the same two panels forced, both about framing rather than pixels:

* **`ram.stream: window_counted`** — the window BOUNDS the stream: the controller
  accepts exactly `(col_end - col_start + 1) × (row_end - row_start + 1)` write
  units and then the stream is shut. That is the SSD1680's own behaviour, not a
  tidier spelling of `command`; running past it wraps the counters back to the
  window origin and overwrites the rows just written. The UC8151D has no RAM
  window command at all and stays on `command`.
* **`dc.unwired`** — what a panel does when NO D/C pad is resolved at attach,
  which is a real board (the ESP32 e-paper lab wires CS and nothing else). It is
  per panel because the two deleted models answered differently: the SSD1680
  INFERRED (a byte with no stream open is a command — which only terminates
  because the stream is window-counted), and the UC8151D could not infer at all
  and treated every byte as DATA. Both are declared cheats, both carry a `real:`
  clause, and a house default would have silently changed one panel's picture.

A `when: { arg, mask, equals }` guard on a `do:` entry runs it only for a given
PARAMETER value: the SSD1680's 0x22 is a sequence selector, where 0xF8 powers
the booster on and 0x83 powers it off. And `busy: { config_key, idle_level }`
drives the BUSY line the host polls to its idle level at attach — stated per
panel because SSD1680 is busy-HIGH and UC8151D is busy-LOW, and the wrong level
hangs GxEPD2's `_waitWhileBusy` for a timeout that never arrives at simulated
speed.

`led_strip` is the primitive for addressable LED strips, and it is separate from
`display` on purpose: a strip is a per-LED COLOUR ARRAY clocked by a wire
protocol, with no address counter, no command table, no window and no frame
memory a later command re-reads — the four things `display` exists to interpret.
Expressing a strip as a display would mean inventing all four and then declaring
in every descriptor that none of them moves.

The descriptor says which wire clocks the LEDs in, and the engine owns the two
decoders:

```yaml
# configs/devices/apa102.yaml — the clocked wire
behavior:
  primitive: led_strip
  led_strip:
    wire: spi_frames
    artifact_format: APA102_RGB
    default_pixels: 8
    supply_gated: true
    spi_frames:
      start_frame: [0x00, 0x00, 0x00, 0x00]
      frame_bytes: 4
      header_mask: 0xE0      # an LED frame's first byte has its top 3 bits set
      header_value: 0xE0     # a byte that fails this STOPS the decode
      brightness_mask: 0x1F  # must not overlap header_mask — the engine refuses it
      colour_bytes: [3, 2, 1]  # the wire carries B,G,R; the artifact publishes R,G,B
    artifact_meta: [brightness, powered, cs_pin]
```

```yaml
# configs/devices/ws2812.yaml — the single wire
behavior:
  primitive: led_strip
  led_strip:
    wire: nrz_gpio
    artifact_format: ws2812_grb
    default_pixels: 1
    timing:
      high_threshold_ns: 500    # a HIGH longer than this is a 1 bit
      reset_threshold_ns: 40000 # a LOW longer than this latches the frame
      bits_per_pixel: 24
    artifact_meta: [pixels_decoded, lit_pixels, data_pin]
```

`wire: spi_frames` attaches as an SPI device and latches on CS RELEASE, which is
what silicon does: a transaction shorter than a start frame plus one LED frame
leaves the previous colours untouched, so a glitchy transfer cannot blank a
strip. `wire: nrz_gpio` attaches as a GPIO observer on ONE pad and decodes real
edge times — every bit is a HIGH pulse whose DURATION is the bit value, and a
long LOW gap latches the frame. Nothing about the byte stream is inferred.

The artifact is the LED colour array in wire order plus the `meta` keys the
descriptor lists, so what the browser's strip overlay reads is the descriptor's
contract rather than a house style. The proof parts are `apa102.yaml` (clocked)
and `ws2812.yaml` (single-wire).

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
