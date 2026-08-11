# Resource metrics examples (P0)

Scenario budgets for flash, static RAM, and **main stack** high-water — not
silicon performance counters. Metrics and assertions are collected by
`labwired test` only (not `labwired run` / playground).

## Layout

```
examples/metrics/
  README.md                          # this file
  stm32f103-blinky/
    system.yaml                      # chip + optional LED on PA5
    test-pass.yaml                   # green path: flash + RAM + stack budgets
    test-fail-stack.yaml             # max_main_stack_bytes: 1 → assert fail
    test-paint-off.yaml              # stack_paint: false, no stack budget
```

Firmware: `tests/fixtures/stm32f103-blinky.elf` (Arduino-style forever blink).

Full reference: [docs/resource_metrics.md](../../docs/resource_metrics.md).

## Run

From the repo root (use `--script`; the binary is `labwired`):

```bash
cargo run -p labwired-cli -- test \
  --script examples/metrics/stm32f103-blinky/test-pass.yaml \
  --output-dir /tmp/m-pass

cargo run -p labwired-cli -- test \
  --script examples/metrics/stm32f103-blinky/test-fail-stack.yaml \
  --output-dir /tmp/m-fail
# exit code 1, resource_budget evidence on the stack assert

cargo run -p labwired-cli -- test \
  --script examples/metrics/stm32f103-blinky/test-paint-off.yaml \
  --output-dir /tmp/m-paint-off
```

## What each script shows

| Script | `stack_paint` | Stack budget | Expected |
|--------|---------------|--------------|----------|
| `test-pass.yaml` | `true` (default) | `max_main_stack_bytes: 4096` | pass (exit 0) |
| `test-fail-stack.yaml` | `true` | `max_main_stack_bytes: 1` | fail (exit 1) + evidence |
| `test-paint-off.yaml` | `false` | none | pass; `memory.main_stack_method: disabled` |

Flash / RAM budgets are scenario ceilings (generous for this blinky), not
datasheet fill targets. Each `resource_budget` assertion sets **exactly one**
of `max_flash_bytes`, `max_ram_static_bytes`, `max_main_stack_bytes`.

## jq samples

Pass result — footprint and main-stack high-water:

```bash
jq '{footprint, memory}' /tmp/m-pass/result.json
# {
#   "footprint": {
#     "method": "elf_section_totals_v1",
#     "text_bytes": 12760,
#     "data_bytes": 124,
#     "bss_bytes": 2548,
#     "flash_used_bytes": 12884,
#     "ram_static_bytes": 2672,
#     "flash_total_bytes": 1000000,
#     "ram_total_bytes": 20480,
#     "flash_used_pct": 1.29,
#     "ram_static_pct": 13.05,
#     "notes": ["section_sum_not_bin_image", "totals_from_chip_catalog"]
#   },
#   "memory": {
#     "main_stack_method": "paint",
#     "main_stack_high_water_bytes": 172,
#     ...
#   }
# }
```

Fail-only evidence (stack budget breach):

```bash
jq '.assertions[] | select(.passed == false)' /tmp/m-fail/result.json
# {
#   "assertion": { "resource_budget": { "max_main_stack_bytes": 1 } },
#   "passed": false,
#   "evidence": {
#     "type": "resource_budget",
#     "name": "max_main_stack_bytes",
#     "measured": 172,
#     "limit": 1,
#     "method": "paint"
#   }
# }
```

## Kill switch

`LABWIRED_STACK_PAINT=0` (or `false` / `off`) forces paint off even when the
script has `stack_paint: true`. Useful when debugging paint interaction.

## Notes

- **Main stack only** — no RTOS task stacks, no ISR nesting water-marks in P0.
- **Section-sum flash** — Berkeley text/data/bss from ELF sections, not the
  raw `.bin` image size (`notes` include `section_sum_not_bin_image`).
- **Catalog totals** — chip YAML flash/RAM sizes fill `*_total_bytes` /
  `*_pct` when known (`totals_from_chip_catalog`).
- Behavioral stop for this fixture with `max_steps` = `max_cycles` = 200000 is
  `max_cycles` (cycles advance faster than steps on multi-cycle ops).
