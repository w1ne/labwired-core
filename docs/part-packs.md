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
- **lipo_charger** — an `analog_source` in shape, but its pin voltage is
  computed from **two** channels rather than looked up on a curve over one:
  `3300 + 9 × soc_pct`, plus a 150 mV charge bump **iff** `usb_present ≥ 0.5`,
  clamped to 4200 mV, then integer-divided by the ÷2 divider. Three gaps for one
  small part — `analog.source` naming a `derived:` channel, a threshold on a
  boolean channel, and the model's two integer truncations — and inventing all
  three for one part is how a vocabulary stops being a vocabulary.

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
