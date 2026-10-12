# Ordered interrupt and debugger metadata

Two public metadata fields now use `BTreeMap`, with keys iterating in lexical
`String` order:

| Public field | Previous concrete type | Current concrete type |
| --- | --- | --- |
| `labwired_config::PeripheralDescriptor::interrupts` | `Option<HashMap<String, u32>>` | `Option<BTreeMap<String, u32>>` |
| `labwired_core::bus::SystemBus::debug_schemas` | `HashMap<String, Vec<RegisterSchema>>` | `BTreeMap<String, Vec<RegisterSchema>>` |

This is an intentional public Rust source API change. Downstream code that
assigns, returns or borrows these fields as concrete `HashMap` types must migrate.
Source and binary compatibility are not promised. Release version selection
remains the release owner's responsibility; this change does not bump a version.

For an existing map, collect its entries into the new type:

```rust
use std::collections::BTreeMap;

let ordered = old.into_iter().collect::<BTreeMap<_, _>>();
```

For an optional interrupt map, preserve absence while converting present values:

```rust
let ordered_interrupts = old_interrupts
    .map(|map| map.into_iter().collect::<BTreeMap<_, _>>());
```

New explicit empty constructors use `BTreeMap::new()`. Reference annotations use
`&Option<BTreeMap<String, u32>>` or `&BTreeMap<String, Vec<RegisterSchema>>`.
Code using ordinary lookup, insertion or map iteration keeps the same methods;
iteration now observes the ordered contract.

Interrupt numbers and debugger register values retain their existing shapes.
Serializing either metadata map emits keys in lexical order; consumers must
account for this intentional text-order change. Missing or null interrupt metadata stays
`None`, an explicit YAML empty map stays `Some(empty)`, and empty IR/SVD interrupt
lists keep their existing `None` conversion. Duplicate names retain the last
inserted value. SVD inheritance still inserts base interrupts before derived
ones, so a derived duplicate overrides the base number. Malformed numeric or
map input keeps the existing typed-parser failure contract.

Debugger schemas remain eagerly loaded from the embedded registry first and
filesystem second. Missing or malformed descriptors are skipped with the
existing warnings. The last valid duplicate peripheral ID wins; a later skipped
invalid descriptor does not replace it. Attaching schemas replaces the whole
metadata map, including replacement by an empty map. It preserves actual bus
peripheral order and register/field vector order. A model's own nonempty
`describe_registers()` retains precedence over debugger metadata. Schemas name
registers and confer no additional simulated hardware fidelity.

The regression targets cover public types, interrupt input-order/serialization
invariance, duplicate/optional/error/IR/SVD behavior, and the fixed STM32L476
inventory of 58 descriptors and 949 registers. The frozen golden fixture remains
unchanged. Existing IRQ/timing, inspection and MMIO checks remain required under
CoreCI, including the default-feature and scheduler-enabled metadata targets.

Whole-process performance repeatability remains unqualified. Establishing an
ordered metadata API supplies a common foundation for a separately qualified
cache-borrow comparison; it does not establish the cause of previous raw
instruction-count variation or a performance improvement. CLI arguments,
firmware, measurement boundaries, logging, strict repeatability gates, baselines
and acknowledgments retain their existing contracts.
