#!/usr/bin/env python3
"""Parse openocd raw step-trace output into one JSON object per simulator step."""
import json
import re
import sys

REG_LINE = re.compile(r"\(\s*\d+\)\s+(\w+)\s+\(\/\d+\):\s+(0x[0-9a-fA-F]+)")
PERI_KEY = re.compile(r"^(\w+):0x[0-9a-fA-F]+$")
PERI_VAL = re.compile(r"^(0x[0-9a-fA-F]+):\s+([0-9a-fA-F]+)")
STEP_KEY = re.compile(r"^step:(\d+)$")


def main(path: str) -> None:
    current: dict | None = None
    pending_key: str | None = None

    with open(path, "r", errors="replace") as f:
        for line in f:
            s = line.strip()

            m = STEP_KEY.match(s)
            if m:
                if current is not None:
                    print(json.dumps(current))
                current = {"step": int(m.group(1))}
                pending_key = None
                continue

            if current is None:
                continue

            m = PERI_KEY.match(s)
            if m:
                pending_key = m.group(1)
                continue

            m = REG_LINE.match(s)
            if m:
                current[m.group(1).lower()] = m.group(2).lower()
                pending_key = None
                continue

            m = PERI_VAL.match(s)
            if m and pending_key is not None:
                hex_val = m.group(2).lstrip("0") or "0"
                current[pending_key] = "0x" + hex_val.lower()
                pending_key = None

    if current is not None:
        print(json.dumps(current))


if __name__ == "__main__":
    if len(sys.argv) < 2:
        print("usage: parse-trace.py <raw-trace.txt>", file=sys.stderr)
        sys.exit(2)
    main(sys.argv[1])
