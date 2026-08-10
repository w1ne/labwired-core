#!/usr/bin/env bash
#
# LabWired - Firmware Simulation Platform
# Copyright (C) 2026 Andrii Shylenko
#
# This software is released under the MIT License.
# See the LICENSE file in the project root for full license information.
#
# ─────────────────────────────────────────────────────────────────────────────
# A `cargo test` wrapper that refuses to report green for a test target which
# contains ZERO tests.
#
# THE FAILURE THIS EXISTS FOR. A libtest binary with no tests prints
#
#     running 0 tests
#     test result: ok. 0 passed; 0 failed; 0 ignored; ...
#
# and exits 0. To CI, and to a human reading a green tick, that is
# indistinguishable from a suite that ran and passed. `crates/core/tests/` is
# full of files whose whole body sits behind `#![cfg(feature = "...")]`; built
# without the feature each one compiles to an empty binary and reports success.
# `board_batch_width` (3 tests) and `census_probe` (3 tests) were both caught
# doing this by accident rather than by a gate.
#
# HOW IT CHECKS. It does not parse `cargo test` stdout — the `Running <path>`
# banner goes to stderr and the `0 passed` line to stdout, so pairing them is
# an ordering guess. Instead it asks cargo, in JSON, which test executables the
# invocation builds, then asks each executable how many tests it holds
# (`--list`). A binary that lists nothing is the bug, named by path, before a
# single test runs.
#
# Doctests are deliberately out of scope: `cargo test --no-run` does not build
# them, and a crate with no doc examples legitimately reports `0 passed`. That
# trailing zero is also exactly what makes this class easy to MISREAD by eye —
# `cargo test -p labwired-config` really runs 107 tests and then prints a
# fourth `test result: ok. 0 passed` for its (empty) doctest target.
#
# USAGE — drop it in front of an existing CI step, arguments unchanged:
#
#     scripts/ci/cargo-test-nonvacuous.sh -p labwired-core \
#       --features event-scheduler --test board_batch_width -- --nocapture
#
# It compiles once (`--no-run`), verifies, then runs the real `cargo test` with
# the identical arguments and exits with ITS status. The second cargo call is a
# cache hit, so the wrapper costs the `--list` calls and nothing else.
#
# WHAT IT DOES NOT COVER. A target whose every test is `#[ignore]`d also
# executes nothing in a normal lane, but libtest's `--list` cannot distinguish
# those (it lists ignored tests too), and this repo has lanes that legitimately
# run such files with `-- --ignored`. That class is reasoned about by name in
# `NIGHTLY_ONLY` in crates/core/src/tests/scheduler_lane_coverage.rs.
# ─────────────────────────────────────────────────────────────────────────────
set -euo pipefail

if [ $# -eq 0 ]; then
    echo "usage: $0 <cargo test arguments...>" >&2
    exit 2
fi

# `--no-run` has to go BEFORE the harness separator, or cargo hands it to
# libtest and the suite runs anyway. Split argv at the first bare `--`.
cargo_args=()
harness_args=()
seen_sep=0
for arg in "$@"; do
    if [ "$seen_sep" -eq 0 ] && [ "$arg" = "--" ]; then
        seen_sep=1
        continue
    fi
    if [ "$seen_sep" -eq 0 ]; then
        cargo_args+=("$arg")
    else
        harness_args+=("$arg")
    fi
done

artifacts="$(mktemp)"
trap 'rm -f "$artifacts"' EXIT

echo "==> cargo test ${cargo_args[*]} --no-run (enumerating test targets)"
cargo test "${cargo_args[@]}" --no-run --message-format=json >"$artifacts"

# Pull the test executables out of cargo's JSON. `"test": true` in the profile
# is what marks a compiler-artifact as a libtest binary; it excludes build
# scripts, dependency rlibs and any plain bin target rebuilt along the way.
# `mapfile` is bash 4+, and macOS ships bash 3.2 — read the list the portable
# way so this script behaves the same locally and on the runner.
executables=()
while IFS= read -r line; do
    [ -n "$line" ] && executables+=("$line")
done < <(
    python3 - "$artifacts" <<'PY'
import json, sys

seen = []
with open(sys.argv[1], encoding="utf-8", errors="replace") as fh:
    for line in fh:
        line = line.strip()
        if not line.startswith("{"):
            continue
        try:
            msg = json.loads(line)
        except json.JSONDecodeError:
            continue
        if msg.get("reason") != "compiler-artifact":
            continue
        if not msg.get("profile", {}).get("test"):
            continue
        exe = msg.get("executable")
        if exe and exe not in seen:
            seen.append(exe)
for exe in seen:
    print(exe)
PY
)

# ── ANTI-VACUITY ────────────────────────────────────────────────────────────
# This script's whole job is to catch a suite that checked nothing; a wrapper
# that itself checked nothing would be the same bug one level up. Every
# `cargo test` invocation in this repo builds at least one test binary, so an
# empty list means the JSON shape changed, the arguments selected no target, or
# the compile produced nothing — never "all clear".
if [ ${#executables[@]} -eq 0 ]; then
    {
        echo "FAIL: cargo built NO test executables for: cargo test ${cargo_args[*]}"
        echo "      This wrapper cannot have verified anything. Either the argument"
        echo "      selection matches no target, or the --message-format=json shape"
        echo "      changed. Do not read this as a pass."
    } >&2
    exit 1
fi

vacuous=()
total_tests=0
for exe in "${executables[@]}"; do
    n="$("$exe" --list --format terse 2>/dev/null | grep -c ': test$' || true)"
    total_tests=$((total_tests + n))
    if [ "$n" -eq 0 ]; then
        vacuous+=("$exe")
    fi
done

echo "==> ${#executables[@]} test target(s), $total_tests test(s) total"

if [ ${#vacuous[@]} -gt 0 ]; then
    {
        echo
        echo "FAIL: ${#vacuous[@]} test target(s) contain ZERO tests:"
        printf '        %s\n' "${vacuous[@]}"
        echo
        echo "  A libtest binary with no tests prints \`test result: ok. 0 passed\`"
        echo "  and exits 0, so this step would have reported GREEN while checking"
        echo "  nothing. Almost always the file is behind \`#![cfg(feature = ...)]\`"
        echo "  and this invocation did not pass the feature."
        echo
        echo "  Fix it in the manifest, not here: give the target a"
        echo "    [[test]]"
        echo "    name = \"<target>\""
        echo "    required-features = [\"<feature>\"]"
        echo "  block, and cargo will REFUSE to build it without the feature"
        echo "  instead of reporting it green. That contract is enforced by"
        echo "  crates/core/src/tests/no_vacuous_test_targets.rs."
    } >&2
    exit 1
fi

echo "==> cargo test ${cargo_args[*]}${harness_args[*]:+ -- ${harness_args[*]}}"
if [ ${#harness_args[@]} -gt 0 ]; then
    exec cargo test "${cargo_args[@]}" -- "${harness_args[@]}"
else
    exec cargo test "${cargo_args[@]}"
fi
