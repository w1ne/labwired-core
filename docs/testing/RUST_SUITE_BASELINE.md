# Rust suite baseline

**277 test targets, 4984 tests, 72 ignored** (debug profile, recorded 2026-08-16).

Machine-readable: [`rust-suite-baseline.json`](rust-suite-baseline.json).
Recorded and compared by [`scripts/ci/rust-suite-baseline.py`](../../scripts/ci/rust-suite-baseline.py).

```
python3 scripts/ci/rust-suite-baseline.py            # report drift
python3 scripts/ci/rust-suite-baseline.py --write    # re-record
```

## The gap this closes

Two gates already guard the suite at the *target* level, and both stay green
while a suite quietly empties out:

| Existing gate | What it catches |
| --- | --- |
| `workspace_test_shard.py` `classify()` + `workspace-test-shards.json` | A test target that is not classified, or an exclusion entry naming a target that no longer exists. A binary cannot silently appear or vanish. |
| `crates/core/src/tests/no_vacuous_test_targets.rs` | An integration binary that compiles to **zero** tests. |

Neither counts the test **functions** inside a target that still exists and is
still non-empty. A suite can go from 40 tests to 1 — a `#[cfg(feature)]` that
stops matching, a `mod tests` that loses its `#[cfg(test)]`, a rename that
orphans half a file — and both gates pass.

This was verified, not assumed. Commenting out two `#[test]` attributes in
`crates/svd-ingestor/tests/parsing_test.rs` (leaving the functions compiling, so
the target survives and stays non-vacuous):

```
$ cargo test -p labwired-core --lib no_vacuous
test result: ok. 4 passed; 0 failed          <- existing gate: green

$ python3 scripts/ci/rust-suite-baseline.py
baseline: 277 targets, 4984 tests, 72 ignored
now:      277 targets, 4982 tests, 72 ignored

TARGETS THAT LOST TESTS — the case no existing gate catches:
  svd-ingestor/parsing_test[test]: 4 -> 2
```

Target count unchanged at 277, so the shard classifier saw nothing either.

## Why this is a report and not a gate

The counts move with the environment (below), so a hard threshold would fail for
reasons that are not regressions, and a gate that cries wolf gets silenced
within a week. The per-shard PR lane is what fails on red; this tells a human
what the shape should be. `--write` after an understood change is the whole
maintenance burden.

## Preconditions — a count taken without these is not comparable

- **Feature unification.** Enumeration comes from a single
  `cargo test --workspace --no-run`, never a per-package loop. Under the unified
  build `crates/wasm` forces `event-scheduler` on, which both adds targets whose
  `required-features` now match and changes runtime by **27×**
  (`labwired-cli::no_elf_c3_rom_boot`: 8.59 s under `-p labwired-cli`, 232 s
  under the workspace build). The script therefore calls
  `workspace_test_shard.build_workspace_tests()` rather than shelling out on its
  own — one enumerator, shared with the lane that actually runs the tests.
- **Profile: debug.** Files carrying `#![cfg(not(debug_assertions))]` compile to
  an empty binary in debug and are recorded as 0 with `release_only: true`. They
  are not missing. Three targets are in this state.
- **Ignored ≠ absent.** 72 tests are `#[ignore]`, tracked by reason in
  [`IGNORED_TESTS.md`](IGNORED_TESTS.md). The script reports a rising ignored
  count separately, because a suite whose tests all became `#[ignore]` holds the
  same total and executes nothing.
- **Python ≤ 3.12.** `crates/python` builds pyo3 0.20, which refuses any newer
  interpreter, so on a modern host the *workspace build* fails — a toolchain
  gap, not red. The script detects this and sets
  `PYO3_USE_ABI3_FORWARD_COMPATIBILITY=1` itself. CI's ubuntu images ship 3.12
  and never hit it.
- **Cross targets.** Suites that cross-build firmware at test time are excluded
  from PR shards by `workspace-test-shards.json`. That exclusion is about
  *running*; they still build, so their counts are present here.

## Shape

17 targets hold zero tests. All are `bin`/`lib` unittest binaries for crates
with no `#[cfg(test)]` module — cargo emits one per bin regardless — except the
three `release_only` entries above. `no_vacuous_test_targets.rs` covers the
`test` kind, where zero would be a real defect.

## Cross-check against CI

The PR shard aggregate on the same tree reports **258 targets / 4874 tests**;
this file records **277 / 4984**. The gap is not a discrepancy — it is the
cross-build-excluded suites plus the `lib` pseudo-targets the aggregate counts
separately. Those exclusions are about *running*, not building, so they are
present here and absent there. Reading either number as the other is the
mistake this section exists to prevent.

The +1 target / +3 tests over the first measurement of this file is
`labwired-config/chip_pins_ratchet`, landed in core#999 between the two runs —
which is the tool reporting exactly what it is for.

The distribution is heavily skewed: `labwired-core`'s lib unittest binary alone
holds 3223 of the 4984.
