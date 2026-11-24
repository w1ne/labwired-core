# nRF54L15 missing TIMER instances — negative control (2026-11-24)

## Defect

`configs/chips/nrf54l15.yaml` declared four of the seven TIMER instances the
part has. The shared Zephyr devicetree `dts/vendor/nordic/nrf54l_05_10_15.dtsi`
and the vendored Nordic SVD (`tests/fixtures/real_world/nrf54l15.svd`) both
define:

| instance | secure base | IRQ | cc-num | base clock |
|----------|-------------|-----|--------|------------|
| TIMER00  | 0x5005_5000 | 85  | 6      | 128 MHz HF PLL (core clock) |
| TIMER10  | 0x5008_5000 | 133 | 8      | 32 MHz HFXO |
| TIMER20  | 0x500C_A000 | 202 | 6      | 16 MHz PCLK16M |
| TIMER21  | 0x500C_B000 | 203 | 6      | 16 MHz PCLK16M |
| TIMER22  | 0x500C_C000 | 204 | 6      | 16 MHz PCLK16M |
| TIMER23  | 0x500C_D000 | 205 | 6      | 16 MHz PCLK16M |
| TIMER24  | 0x500C_E000 | 206 | 6      | 16 MHz PCLK16M |

The profile omitted TIMER00, TIMER23 and TIMER24. Any firmware whose devicetree
aliases one of them faults on its first register access.

`svd_conformance` did not catch this: it checks the base address and IRQ of the
peripherals a descriptor already declares, never whether an SVD instance is
missing from the descriptor.

## Negative control

Descriptor reverted to the pre-fix revision; everything else (models, tests)
unchanged. Command:

```bash
cargo test -p labwired-core --test nrf nrf_timer_base_clock
```

Result: 2 passed, 2 failed.

```
test nrf54l_profiles_declare_every_timer_the_devicetree_defines ... FAILED
  timer00: not declared; firmware using it bus-faults
  timer23: not declared; firmware using it bus-faults
  timer24: not declared; firmware using it bus-faults

test nrf_timers_count_their_base_clock_not_the_core_clock ... FAILED
called `Result::unwrap()` on an `Err` value: MemoryViolation(1342526728)
```

1342526728 = `0x5005_5000`, the TIMER00 base: the behavioural row cannot even
write `BITMODE` before the fix.

## After the fix

Same command, same binary: 4 passed, 0 failed. `nrf_timers_count_their_base_clock_not_the_core_clock`
now also measures TIMER00 (640 counts in 640 CPU cycles, 1:1 with the core) and
TIMER23 (80 counts in 640 cycles, 1:8) on `nrf54l15.yaml`.

## Gate source: vendor SVD, not a local table

The first revision of the gate carried its own `(id, base, irq, cc-num)` table.
That made the check circular: the descriptor and the expectation could both be
wrong in the same way. The gate now parses the secure `<peripheral>` blocks of
`tests/fixtures/real_world/nrf54l15.svd` and `nrf54lm20a.svd` for instance
names, base addresses and interrupt values, so the completeness claim is
checked against the vendor file. Only `cc-num` stays a table entry: every SVD
instance derives from `GLOBAL_TIMER00_NS` with `<dim>8</dim>`, so the SVD
cannot express the per-instance channel count.

Re-run negative control with the SVD-sourced gate (remove only the `timer24`
declaration, same binary): `svd-sourced-gate.log`.

```
nrf54l15.yaml timer estate disagrees with nrf54l15.svd:
  timer24: not declared; SVD has it at 0x500CE000
```
