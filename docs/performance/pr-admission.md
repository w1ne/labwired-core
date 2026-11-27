# Performance admission for pull requests

Runtime, build, configuration and performance-input changes require a measured
performance result before merge. The `perf` label is not an exemption switch.

## Required checks

`core-ci.yml` calls `core-perf-pr.yml` through its parallel `pr-performance` job.
The existing protected `pr-core-feature-off` aggregate requires that job and
both library feature jobs to succeed. No new branch-protection context is
needed. A failed, cancelled or skipped required measurement makes this
aggregate fail; the aggregate uses `always()` so a failed dependency cannot
turn the required check into a skip.

The reusable workflow has three jobs:

1. `perf-scope` verifies the actual tested merge, its two parents and the event
   base/head identities. It diffs the tested merge against its actual first
   parent, including deletions and renames. A Git error or invalid source
   identity fails admission and writes no successful scope output.
2. `pr-throughput` measures selected changes. Documentation and independently
   compiled test-only changes skip this job. Unclassified source/build inputs
   require measurement, including new crates. WASM source and build inputs,
   chip/system descriptors, performance fixtures, baselines and scripts are
   in scope. Data files do not become documentation solely because they live
   below `docs/`.
3. `pr-perf-verdict` always runs. It accepts only exact `true`/`false` scope
   output, a successful source decision and either a successful selected
   measurement or an explicitly unselected, skipped job. Missing output,
   failure, cancellation and unexpected job execution are errors.

Draft PRs run the source policy but omit measurements. Their verdict and
protected aggregate use distinct draft names, so a draft result cannot satisfy
normal required checks. `ready_for_review` starts the complete admission path.
Manual core-CI dispatch also measures the short set.

## Bounded native measurement

The required PR sample uses three standard-toolchain representatives:
`esp32c3`, `rp2040` and `atmega328p`. It uses the same native CLI, firmware
fixtures, execution modes, baselines and thresholds as the full matrix:

```sh
cargo build --release -p labwired-cli --features event-scheduler
python3 scripts/perf/board_rtx.py \
  --boards esp32c3,rp2040,atmega328p \
  --repeats 3 --seconds 1 --min-rtx 1.0 --no-workloads
python3 scripts/perf/board_perf.py \
  --boards esp32c3,rp2040,atmega328p --require-all
```

The RTx command requires median native spin throughput of at least one CPU
second per wall second over three samples. The instruction-cost check uses
all existing modes for each selected fixture, the existing three-repetition
Callgrind slopes, the existing 3% regression rule and its 0.5 host-instruction
absolute noise floor for batch mode. No baseline is raised or rewritten.
Missing fixtures, unavailable selected modes and failed measurements fail.
The job has a 30-minute limit including builds and cache work; it reuses the
existing native performance cache and does not install an Xtensa toolchain.
Both source admission and measurement pin Python 3.12 before installing Python
dependencies, matching the full matrix environment.

This is representative native regression coverage. It does not establish
performance on every board, the WASM boundary or the browser host. The app's
packaged-WASM and host cold-boot/resume gate must independently check those
paths. Native CLI spin speed cannot establish OLED frame progress or correct
warm snapshot restoration.

## Full matrix

`core-perf.yml` retains the full unchanged matrix nightly, on manual dispatch
and when a PR carries the `perf` label. The optional result is named
`full-perf-verdict` so its unrequested result is distinct from mandatory PR
admission. The label adds full measurement; it cannot remove the short one.
Full selection uses a separate `full_required` output and the same strict
verdict logic. Full fleet thresholds, workload gates and regression issue
reporting remain unchanged. The full matrix does not run on every push to
main.

## Regression controls

`test_pr_hotpath_gate.py` checks source selection and Git-history failures.
`test_pr_perf_admission.py` executes the real workflow selector and protected
aggregate shell. It covers an absent label, WASM-only changes, explicit
full-matrix requests, invalid source history, missing scope outputs, skipped,
failed and cancelled required work, draft naming, all measurement modes and
the explicit documentation-only path. These controls run in both admission
and the existing PR gate.
