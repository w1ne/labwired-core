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

## What a pack cannot do

A pack is data interpreted by a **primitive** — `i2c_device`, `spi_device`,
`analog_source`, `quadrature`, `matrix`, `one_wire`, `pulse_echo`. Those
primitives are the irreducible timing algorithms, and they live in Rust in this
repository.

`analog_source` is the primitive for parts whose whole interface is one
analogue voltage (a Sharp IR ranger's `Vo`, an MQ-x module's `AOUT`): the
descriptor carries the datasheet's output curve as `(input, mV)` points plus
stated out-of-band rules (`below_first: clamp`, `above_last.floor_mv`), and the
engine owns the rest (SimInput plumbing, mV→ADC count, attach). The proof part
is `gp2y0a21.yaml`.

So: a part whose datasheet behaviour is a register map, a command/response
protocol, or one of the pin-timing shapes above is pure data and needs nothing
from us. A part with a genuinely new wire protocol needs a new primitive, which
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
