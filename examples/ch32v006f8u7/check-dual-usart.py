#!/usr/bin/env python3
"""Decode the USART TX pads of ch32fun's examples_x00x/dual_usart.

Reads the `logic_edges` block of a `labwired test` result.json for
ch32fun-dual-usart-smoke.yaml (its `gpio_edges` assertions arm the capture
on PD5 and PD2) and decodes each pad as 115200 8N1 at 48 MHz (416.67
cycles per bit, idle high). It checks that the bytes on each pad are that
port's own text and nothing else:

- PD5 (USART1_TX, AFIO_PCFR1.USART1_RM = 0) carries the banner, "USART1
  ready" and "tick N via USART1" lines, and no USART2 text;
- PD2 (USART2_TX, USART2_RM = 3) carries the banner, "USART2 ready" and
  "tick N via USART2", and no USART1 text;
- every frame has a low start bit and a high stop bit.

The input may be gzipped. Prints the decoded text of each pad and exits
non-zero on the first mismatch.
"""
import gzip
import json
import sys

BIT = 48_000_000 / 115_200


def edges_of(chan):
    return [(e["cycle"], e["value"]) for e in chan["transitions"]]


def level_at(edges, initial, cycle):
    level = initial
    for c, v in edges:
        if c > cycle:
            break
        level = v
    return level


def decode(chan):
    edges = edges_of(chan)
    initial = chan["initial"]
    out = bytearray()
    i = 0
    while i < len(edges):
        c, v = edges[i]
        # A start bit is a falling edge from the idle-high line.
        if v != 0 or level_at(edges, initial, c - 1) != 1:
            i += 1
            continue
        byte = 0
        for bit in range(8):
            byte |= level_at(edges, initial, c + BIT * (bit + 1.5)) << bit
        if level_at(edges, initial, c + BIT * 0.5) != 0:
            sys.exit(f"pin {chan['pin']}: start bit at {c} not low at mid-bit")
        if level_at(edges, initial, c + BIT * 9.5) != 1:
            sys.exit(f"pin {chan['pin']}: framing error, stop bit of frame at {c}")
        out.append(byte)
        end = c + BIT * 9.5
        while i < len(edges) and edges[i][0] <= end:
            i += 1
    return out.decode("ascii", "replace")


def main(path):
    opener = gzip.open if path.endswith(".gz") else open
    with opener(path, "rt") as f:
        result = json.load(f)
    pads = {(ch["peripheral"], ch["pin"]): ch for ch in result["logic_edges"]["channels"]}
    expect = {("gpiod", 5): "USART1", ("gpiod", 2): "USART2"}
    for (port, pin), port_name in expect.items():
        chan = pads.get((port, pin))
        if chan is None:
            sys.exit(f"{port}:{pin}: not captured")
        text = decode(chan)
        print(f"--- {port}:{pin} ({port_name}_TX) ---")
        print(text, end="")
        other = "USART2" if port_name == "USART1" else "USART1"
        want = [
            "\r\nCH32V00x dual_usart example\r\n",
            f"{port_name} ready\r\n",
            f"tick 0 via {port_name}\r\n",
            f"tick 1 via {port_name}\r\n",
            f"tick 2 via {port_name}\r\n",
        ]
        pos = 0
        for w in want:
            at = text.find(w, pos)
            if at < 0:
                sys.exit(f"{port}:{pin}: missing {w!r} after offset {pos}")
            pos = at + len(w)
        if other in text:
            sys.exit(f"{port}:{pin}: carries {other} text")
    print("OK: PD5 carries only USART1, PD2 only USART2, all frames 8N1")


if __name__ == "__main__":
    main(sys.argv[1] if len(sys.argv) > 1 else "result.json")
