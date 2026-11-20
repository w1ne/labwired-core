#!/usr/bin/env python3
"""Record a golden register trace of an MSP430 ELF on the GDB simulator.

Usage: trace.py <msp430-elf-gdb> <elf> <out.trace.z>

The simulator maps no memory at 0x0200-0x04FF (the RAM of an MSP430x1xx
part), so that region is added before the load.

The output is zlib-compressed text. Line 1 holds the sixteen registers before
the first instruction; every later line holds the registers that changed
across one instruction, as `<index>=<hex>` pairs (PC is index 0). Recording
stops when an instruction leaves every register unchanged (a jump to itself)
or after LIMIT instructions.
"""
import subprocess
import sys
import tempfile
import zlib

LIMIT = 400000


def main() -> None:
    gdb, elf, out = sys.argv[1:4]
    regs = "$pc,$sp,$sr,$cg,$r4,$r5,$r6,$r7,$r8,$r9,$r10,$r11,$r12,$r13,$r14,$r15"
    fmt = " ".join(["%x"] * 16)
    script = f"""
set pagination off
set confirm off
target sim
sim memory-region 0x200,0x300
load
starti
set $n = 0
set $last = -1
printf "{fmt}\\n", {regs}
while $n < {LIMIT} && $last != $pc
  set $last = $pc
  stepi
  printf "{fmt}\\n", {regs}
  set $n = $n + 1
end
quit
"""
    with tempfile.NamedTemporaryFile("w", suffix=".gdb", delete=False) as f:
        f.write(script)
    raw = subprocess.run(
        [gdb, "-q", "-batch", "-x", f.name, elf],
        capture_output=True,
        text=True,
        check=True,
    ).stdout
    rows = []
    for line in raw.splitlines():
        parts = line.split()
        if len(parts) != 16:
            continue
        try:
            rows.append([int(p, 16) for p in parts])
        except ValueError:
            continue
    if len(rows) < 2:
        sys.exit(f"{elf}: the simulator produced no trace:\n{raw[-2000:]}")
    lines = [" ".join(f"{v:x}" for v in rows[0])]
    for prev, cur in zip(rows, rows[1:]):
        lines.append(" ".join(f"{i:x}={v:x}" for i, (a, v) in enumerate(zip(prev, cur)) if a != v))
    # The final row repeats the self-jump; drop it.
    if lines[-1] == "":
        lines.pop()
    data = ("\n".join(lines) + "\n").encode()
    with open(out, "wb") as f:
        f.write(zlib.compress(data, 9))
    print(f"{elf}: {len(lines) - 1} instructions, {len(data)} bytes raw")


if __name__ == "__main__":
    main()
