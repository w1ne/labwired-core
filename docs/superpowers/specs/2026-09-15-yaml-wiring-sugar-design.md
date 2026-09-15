# YAML wiring sugar

**Date:** 2026-09-15  
**Status:** approved for implementation (user: plan and implement with subagents)

## Problem

Chip YAML already names peripherals, addresses, and IRQs — the same job as a Renode `.repl`. The map is verbose in three places Renode makes cheap:

1. IRQ is a bare number (`irq: 2`) with the controller implied.
2. A family of chips copies a whole descriptor instead of including a common file.
3. Instance knobs live under `config:` even when they are one key (`easyDMA: true`).

This spec adds sugar only. Existing YAML keeps working. Peripheral *behavior* stays Rust.

## Non-goals

- Adopting `.repl` syntax.
- Runtime-compiled peripherals.
- Migrating the in-tree chip catalog in this change (tests + docs only).
- `include` inside built-in `include_str!` chips (no filesystem). Path-loaded chips only.

## Design

### 1. IRQ target: `irq: nvic@2`

`PeripheralConfig.irq` stays `Option<u32>` (the line number the engine already uses).

Deserializer accepts:

| YAML | Result |
|---|---|
| `irq: 2` | `Some(2)` |
| `irq: "2"` | `Some(2)` |
| `irq: nvic@2` | `Some(2)` |
| `irq: "nvic@2"` | `Some(2)` |
| omitted | `None` |

`controller@line` stores the line. The controller name is recorded on a new optional field `irq_controller: Option<String>` (`Some("nvic")`) so later wiring can check it. Unknown shapes (`irq: foo`, `irq: nvic@`, `irq: @2`) error at parse time.

Engine code that reads `.irq` does not change.

### 2. Flattened instance properties

Unknown keys on a peripheral mapping merge into `config`.

```yaml
- id: uart0
  type: nrf52840_uart
  base_address: 0x40002000
  irq: nvic@2
  easyDMA: true
```

is the same as today's `config: { easyDMA: true }`. If both appear, **`config:` wins** for that key.

Known fields (`id`, `type`, `base_address`, `size`, `irq`, `clock`, `config`, `irq_controller`) are not flattened into `config`.

### 3. `include` on path-loaded chips

```yaml
include: nrf52-common.yaml
name: nrf52840
# local peripherals override included ones by id
```

or `include: [a.yaml, b.yaml]`.

Paths are relative to the **including file**. Load order: includes first (left to right), then the local document. Merge:

- `peripherals`: union by `id`; local replaces the same id.
- `memory_regions`: union by `name`; local replaces.
- `pins`: local keys override.
- Scalars (`name`, `arch`, `core`, `cpu_hz`, `flash`, `ram`, …): local value wins when present; includes fill gaps. `name`/`arch`/`flash`/`ram` remain required on the **final** document (they may come entirely from an include).

Cycles error. Missing include file errors with the path.

`ChipDescriptor::from_file` and path `resolve` run include expansion. `serde_yaml::from_str` on a builtin string does **not** (no base directory).

## Testing

Config-crate unit tests with temp YAML files. No engine behavior change, so no core golden updates.

## Docs

Short note in chip YAML docs / config module rustdoc. One example in `crates/config/tests/fixtures/` only.
