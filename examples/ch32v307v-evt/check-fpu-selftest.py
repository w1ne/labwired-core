#!/usr/bin/env python3
"""Compare the CH32V307 FPU self-test's debug-printf terminal with the host
oracle, line by line.

  check-fpu-selftest.py <evidence-dir> [expected.txt]

<evidence-dir> is the --output-dir of a `labwired test` run of
ch32fun-fpu-selftest-smoke.yaml; its peripheral-logs/debug_printf.terminal.log
holds what the firmware printed. expected.txt (default: fpu-selftest/
expected.txt next to this script) is the same fpu_selftest.c run on the build
host's x86-64 SSE/FMA unit (build-ch32fun-examples.sh). Every line must match
exactly, in order: one hash per (vector, rounding mode), one per vector for
the mode-independent instructions, then the direct checks.
"""
import pathlib
import sys


def main() -> int:
    if len(sys.argv) not in (2, 3):
        print(__doc__.strip(), file=sys.stderr)
        return 2
    evidence = pathlib.Path(sys.argv[1])
    expected_path = (
        pathlib.Path(sys.argv[2])
        if len(sys.argv) == 3
        else pathlib.Path(__file__).resolve().parent / "fpu-selftest" / "expected.txt"
    )
    got = (evidence / "peripheral-logs" / "debug_printf.terminal.log").read_text().splitlines()
    want = expected_path.read_text().splitlines()
    bad = 0
    for i, line in enumerate(want):
        have = got[i] if i < len(got) else "<missing>"
        if have != line:
            bad += 1
            print(f"line {i + 1}: expected {line!r}, got {have!r}")
    if len(got) > len(want):
        bad += 1
        print(f"{len(got) - len(want)} extra line(s) after the expected output")
    if bad:
        print(f"FAIL: {bad} of {len(want)} lines differ")
        return 1
    print(f"PASS: all {len(want)} lines match the host FPU oracle")
    return 0


if __name__ == "__main__":
    sys.exit(main())
