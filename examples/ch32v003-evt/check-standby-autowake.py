#!/usr/bin/env python3
"""Bracket the CH32V003 auto-wakeup of ch32fun's standby_autowake.

The firmware spends Delay_Ms(5000) (240,000,000 cycles at 48 MHz, plus its
boot and first printf), enables the AWU at LSI/4096 with AWUWR 63, then
loops on __WFE() and prints "awake, N". The model wakes it every
63 x 4096 / 128 kHz = 2.016 s = 96,768,000 cycles after AWUEN, so the third
wake ("awake, 2") is due at AWUEN + 290,304,000 >= 530,304,000 cycles.

Two runs of the unmodified firmware stop either side of that instant:

  530,200,000 cycles: "awake, 1" printed, "awake, 2" not yet. A core that
                      fell through WFE, or woke early, prints far more.
  532,000,000 cycles: "awake, 2" printed. That needs AWUEN within 1.6 M
                      cycles of the 5 s delay and a period of at most
                      (532.0 M - 240 M) / 3 = 97.33 M cycles, which rules
                      out a 64-count (AWUWR + 1) window, 98.30 M cycles.

Usage: check-standby-autowake.py <labwired-cli> <out-dir>
"""
import json
import pathlib
import subprocess
import sys

HERE = pathlib.Path(__file__).resolve().parent
SCRIPT = HERE / "ch32fun-standby-autowake-smoke.yaml"

BRACKETS = [
    # (max_cycles, must contain, must not contain)
    (530_200_000, ["awake, 1"], ["awake, 2"]),
    (532_000_000, ["awake, 2"], ["awake, 3"]),
]


def run(cli: str, out: pathlib.Path, max_cycles: int, texts: list[str]) -> dict:
    out.mkdir(parents=True, exist_ok=True)
    asserts = "\n".join(
        f'  - peripheral_log: {{ peripheral: debug_printf, log: terminal, contains: "{t}" }}'
        for t in texts
    )
    script = out / "bracket.yaml"
    script.write_text(
        'schema_version: "1.0"\n'
        "inputs:\n"
        f'  firmware: "{HERE / "ch32fun/standby_autowake.elf"}"\n'
        f'  system: "{(HERE / "../../configs/systems/ch32v003evt-ch32fun-standby-autowake-smoke.yaml").resolve()}"\n'
        "limits:\n"
        f"  max_cycles: {max_cycles}\n"
        f"  max_steps: {max_cycles + 10_000_000}\n"
        "  wall_time_ms: 900000\n"
        f"assertions:\n{asserts}\n"
    )
    subprocess.run(
        [cli, "test", "--script", str(script), "--output-dir", str(out)],
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
        check=False,
    )
    result = json.loads((out / "result.json").read_text())
    return {
        a["assertion"]["peripheral_log"]["contains"]: a["passed"]
        for a in result["assertions"]
    } | {"_cycles": result["stop_reason_details"]["observed"]["value"]}


def main() -> int:
    cli, out_dir = sys.argv[1], pathlib.Path(sys.argv[2])
    ok = True
    for max_cycles, present, absent in BRACKETS:
        seen = run(cli, out_dir / str(max_cycles), max_cycles, present + absent)
        for text in present:
            good = seen[text]
            ok &= good
            print(f"{seen['_cycles']:>11,} cycles: {text!r:12} printed  {'ok' if good else 'FAIL'}")
        for text in absent:
            good = not seen[text]
            ok &= good
            print(f"{seen['_cycles']:>11,} cycles: {text!r:12} absent   {'ok' if good else 'FAIL'}")
    print("PASS" if ok else "FAIL")
    return 0 if ok else 1


if __name__ == "__main__":
    sys.exit(main())
