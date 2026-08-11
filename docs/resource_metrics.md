# Resource metrics (P0)

Scenario budgets for firmware **footprint** and **main-stack high-water** during
`labwired test`. These are CI gates for “does this build still fit?”, not
silicon performance counters (no MHz, no CPI, no peripheral latency).

Worked examples: [examples/metrics/](../examples/metrics/README.md).

---

## What is measured

| Signal | Source | `result.json` path |
|--------|--------|--------------------|
| Flash used | ELF section sum (text + data) | `footprint.flash_used_bytes` |
| Static RAM | ELF section sum (data + bss) | `footprint.ram_static_bytes` |
| Main stack high-water | Stack paint after load/reset | `memory.main_stack_high_water_bytes` |

Also present when available:

- `footprint.text_bytes` / `data_bytes` / `bss_bytes` — Berkeley-style split
- `footprint.flash_total_bytes` / `ram_total_bytes` — from the **chip catalog**
  (YAML `flash.size` / `ram.size`), not from the linker script
- `footprint.flash_used_pct` / `ram_static_pct` — percent of catalog totals
- `memory.main_stack_method` — `paint` | `disabled` | `unsupported`
- `memory.main_stack_free_min_bytes`, `main_stack_overflow_suspected`, …

Method string for footprint is always `elf_section_totals_v1`. Notes typically
include:

- `section_sum_not_bin_image` — totals are section sums, not `objcopy -O binary` size
- `totals_from_chip_catalog` — device totals came from the chip descriptor

---

## Scope and limits (P0)

- **Scenario budgets, not silicon perf.** Limits you write in the test script
  are product/CI ceilings for that scenario, not datasheet capacity tests.
- **Main stack only.** Paint tracks the primary ARM stack (reset SP → heap
  floor). No FreeRTOS task stacks, no MSP/PSP split, no ISR depth in P0.
- **Section-sum flash.** Flash used = sum of alloc `text` + `data` sections.
  Compare to `arm-none-eabi-size`, not to the flashed `.bin` length.
- **Catalog totals.** Percent-full uses chip YAML sizes when the system
  references a known chip; unknown totals omit the pct fields.
- **`labwired test` only.** Footprint and paint are wired into the headless
  test runner. Interactive `run` / playground do not emit these blocks today.

---

## Script surface

```yaml
schema_version: "1.0"
inputs:
  system: "./system.yaml"
  firmware: "./firmware.elf"
limits:
  max_steps: 200000
  max_cycles: 200000
stack_paint: true          # default true; set false to skip paint
assertions:
  # Exactly one limit key per resource_budget assertion.
  - resource_budget:
      max_flash_bytes: 20000
  - resource_budget:
      max_ram_static_bytes: 8000
  - resource_budget:
      max_main_stack_bytes: 4096
  - expected_stop_reason: max_cycles
```

### `stack_paint`

| Value | Effect |
|-------|--------|
| `true` (default) | Fill free main-stack RAM with a paint pattern before the run; scan high-water after |
| `false` | Skip paint; `memory.main_stack_method` is `disabled`; do not assert `max_main_stack_bytes` |

### Kill switch

Environment variable **`LABWIRED_STACK_PAINT`**:

- `0`, `false`, or `off` (case-insensitive) → force paint **off** even if the
  script sets `stack_paint: true`
- unset or any other value → honor the script flag

### `resource_budget`

Each assertion must set **exactly one** of:

| Key | Compared to |
|-----|-------------|
| `max_flash_bytes` | `footprint.flash_used_bytes` |
| `max_ram_static_bytes` | `footprint.ram_static_bytes` |
| `max_main_stack_bytes` | `memory.main_stack_high_water_bytes` (paint) |

Pass when `measured <= limit`. If the metric is unavailable (`measured` null —
e.g. paint `unsupported` / `disabled`), the assertion **fails**.

### Fail-only evidence

On failure, `assertions[].evidence` is:

```json
{
  "type": "resource_budget",
  "name": "max_main_stack_bytes",
  "measured": 172,
  "limit": 1,
  "method": "paint"
}
```

On pass, `evidence` is omitted. Methods you may see: `elf_section_totals_v1`,
`paint`, `disabled`, `unsupported`, `footprint_unavailable`.

---

## jq examples

```bash
# Footprint + memory block from a green run
jq '{footprint, memory}' /tmp/m-pass/result.json

# text_bytes path (section-sum text)
jq '.footprint.text_bytes' /tmp/m-pass/result.json

# Only failed budget asserts (evidence present)
jq '.assertions[] | select(.passed == false)' /tmp/m-fail/result.json

# High-water vs limit when evidence is attached
jq '.assertions[]
    | select(.evidence.type == "resource_budget")
    | {name: .evidence.name, measured: .evidence.measured, limit: .evidence.limit}' \
  /tmp/m-fail/result.json
```

---

## Paint behaviour (ARM)

1. After load/reset, resolve SP (vector table / CPU) and a heap floor
   (`_end`, `__bss_end__`, or `__heap_start` when present; otherwise the end of
   RAM-resident PT_LOAD image including BSS).
2. Fill `[heap_floor, SP)` with paint word `0xA5A5A5A5` (skipping pure
   file-backed image; GNU `._user_heap_stack` NOBITS reserves are paintable).
3. After the run, scan from low addresses upward while words still equal paint;
   high-water = window size − remaining paint.

If the range is too small, outside RAM, or otherwise unsafe, paint is
`unsupported` with a stable `main_stack_unsupported_reason` string — and any
`max_main_stack_bytes` assert fails closed.

Non-ARM arches report `unsupported` / `arch_not_implemented` in P0.

---

## Exit codes

| Code | Meaning |
|------|---------|
| 0 | All assertions passed |
| 1 | One or more assertions failed (including resource budgets) |
| 2 | Config / script error |
| 3 | Runtime error |

---

## See also

- [Run limits](limits.md) — `max_steps`, `max_cycles`, wall time, …
- [CLI reference](cli_reference.md) — `labwired test` flags
- [Configuration reference](configuration_reference.md) — system / chip YAML
- [examples/metrics](../examples/metrics/README.md) — STM32F103 blinky budgets
