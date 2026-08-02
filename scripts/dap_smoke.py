#!/usr/bin/env python3
"""Drive labwired-dap over stdio and assert the debugger surface actually works.

This is the regression test for what the VS Code extension depends on. It speaks
raw DAP (Content-Length framed JSON) to the binary, exactly as VS Code does, so
it catches breakage that a Rust unit test cannot: wrong framing, a request that
silently returns an empty body, or a launch that fails only when a system
manifest is attached.

Two things are asserted, and they map to two things a user sees:

  * CPU registers   -> the "Registers" scope in the debug pane
  * peripheral regs -> the "Peripherals" tree

Peripheral register decoding was 0 across every native-peripheral chip before the
debug-register-schema work; `--require-peripheral-registers` is what keeps it
from silently regressing to 0 again.

Usage:
    scripts/dap_smoke.py --dap target/release/labwired-dap \\
        --firmware tests/fixtures/nrf52840-demo.elf \\
        --system examples/nrf52840-proximity-lab/system.yaml \\
        --require-peripheral-registers
"""

from __future__ import annotations

import argparse
import json
import subprocess
import sys
import time
from pathlib import Path


class DapClient:
    """Minimal DAP client: Content-Length framed JSON over the adapter's stdio."""

    def __init__(self, dap_path: str, cwd: str, timeout: float):
        self.proc = subprocess.Popen(
            [dap_path],
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            stderr=subprocess.DEVNULL,
            cwd=cwd,
        )
        self.seq = 0
        self.deadline = time.time() + timeout

    def send(self, command: str, arguments: dict | None = None) -> int:
        self.seq += 1
        message: dict = {"seq": self.seq, "type": "request", "command": command}
        if arguments is not None:
            message["arguments"] = arguments
        payload = json.dumps(message).encode()
        assert self.proc.stdin is not None
        self.proc.stdin.write(b"Content-Length: %d\r\n\r\n" % len(payload) + payload)
        self.proc.stdin.flush()
        return self.seq

    def read(self) -> dict | None:
        """Read one DAP message, or None on EOF/timeout."""
        assert self.proc.stdout is not None
        header = b""
        while b"\r\n\r\n" not in header:
            if time.time() > self.deadline:
                return None
            byte = self.proc.stdout.read(1)
            if not byte:
                return None
            header += byte
        length = int(header.decode().split("Content-Length:")[1].split("\r\n")[0].strip())
        return json.loads(self.proc.stdout.read(length))

    def close(self) -> None:
        self.proc.kill()


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--dap", default="target/release/labwired-dap")
    parser.add_argument("--firmware", required=True)
    parser.add_argument("--system", default=None)
    parser.add_argument("--cwd", default=".")
    parser.add_argument("--timeout", type=float, default=60.0)
    parser.add_argument(
        "--require-peripheral-registers",
        action="store_true",
        help="Fail unless at least one peripheral decodes named registers.",
    )
    parser.add_argument(
        "--min-peripherals",
        type=int,
        default=1,
        help="Fail if fewer than this many peripherals are enumerated.",
    )
    args = parser.parse_args()

    if not Path(args.cwd, args.dap).exists() and not Path(args.dap).exists():
        print(f"FAIL: debug adapter not found at {args.dap}", file=sys.stderr)
        return 2

    client = DapClient(args.dap, args.cwd, args.timeout)
    launch: dict = {"program": args.firmware, "stopOnEntry": True}
    if args.system:
        launch["systemConfig"] = args.system

    client.send("initialize", {"adapterID": "labwired"})
    client.send("launch", launch)
    client.send("configurationDone")

    cpu_registers: list[tuple[str, str]] = []
    peripherals: list[dict] = []
    failures: list[str] = []
    saw_peripheral_response = False

    while True:
        message = client.read()
        if message is None:
            failures.append("timed out or adapter exited before the surface was verified")
            break
        if message.get("type") != "response":
            continue

        command = message.get("command")

        if not message.get("success", True):
            failures.append(f"{command} failed: {message.get('message')}")
            break

        if command == "configurationDone":
            client.send("threads")
            client.send("stackTrace", {"threadId": 1})
            client.send("scopes", {"frameId": 0})
        elif command == "scopes":
            for scope in message["body"]["scopes"]:
                if scope["name"] == "Registers":
                    client.send("variables", {"variablesReference": scope["variablesReference"]})
        elif command == "variables":
            cpu_registers = [(v["name"], v["value"]) for v in message["body"]["variables"]]
            client.send("readPeripherals")
        elif command == "readPeripherals":
            saw_peripheral_response = True
            peripherals = message["body"].get("peripherals", [])
            break

    client.close()

    # ── Assertions ────────────────────────────────────────────────────────────
    # Register naming is architecture-specific: Cortex-M exposes R0..R12/SP/LR/PC,
    # RISC-V exposes x0..x31/pc (x2 being the stack pointer). Assert the two
    # registers a debugger is useless without, by any of their spellings.
    register_names = {name.lower() for name, _ in cpu_registers}
    required_any = {
        "program counter": {"pc"},
        "stack pointer": {"sp", "x2"},
    }
    for label, spellings in required_any.items():
        if not (register_names & spellings):
            failures.append(
                f"{label} missing from the Registers scope "
                f"(looked for {'/'.join(sorted(spellings))}; got {len(cpu_registers)} registers)"
            )

    if not saw_peripheral_response:
        failures.append("no readPeripherals response")
    elif len(peripherals) < args.min_peripherals:
        failures.append(
            f"expected >= {args.min_peripherals} peripherals, got {len(peripherals)}"
        )

    decoding = [p for p in peripherals if p.get("registers")]
    if args.require_peripheral_registers and not decoding:
        failures.append(
            f"0 of {len(peripherals)} peripherals decode named registers "
            "(the Peripherals tree would show 'No register descriptors available' for every entry)"
        )

    # ── Report ────────────────────────────────────────────────────────────────
    print(f"CPU registers:  {len(cpu_registers)} " f"({', '.join(n for n, _ in cpu_registers[:8])}...)")
    print(f"Peripherals:    {len(peripherals)}")
    print(f"  decoding regs: {len(decoding)}")
    for peripheral in decoding[:3]:
        sample = ", ".join(
            f"{r['name']}=0x{r['value']:08x}" if isinstance(r.get("value"), int) else f"{r['name']}=?"
            for r in peripheral["registers"][:4]
        )
        print(f"    {peripheral['name']}: {sample}")

    if failures:
        print("\nFAIL:", file=sys.stderr)
        for failure in failures:
            print(f"  - {failure}", file=sys.stderr)
        return 1

    print("\nPASS")
    return 0


if __name__ == "__main__":
    sys.exit(main())
