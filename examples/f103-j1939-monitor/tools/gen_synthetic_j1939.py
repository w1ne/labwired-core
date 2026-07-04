#!/usr/bin/env python3
"""Synthesize a deterministic J1939 candump `.log` for f103-j1939-monitor.

Generates purely synthetic SAE J1939 bus traffic -- engine-speed "noise"
frames, DM1 active-diagnostic-trouble-code broadcasts from nine source
addresses, and three concurrent pairs of interleaved BAM (multi-packet
transport protocol) sessions -- and writes it as a SocketCAN candump log
(`(SECONDS.MICROS) can0 IDHEX#DATAHEX`, extended 29-bit identifiers)
parseable by `crates/core/src/network/candump.rs`.

No third-party capture data is read, embedded, or derived here: every
byte and timestamp in the output is computed by this script from the
constants below. This replaces a previous example log that replayed a
real third-party capture, which this repository has no permission to
redistribute.

Usage:
    python3 gen_synthetic_j1939.py OUTPUT.log [--end SECONDS]

Prints `frames=<N>` to stderr on exit, where N is the number of frames
written (after filtering to timestamps < --end).
"""
import argparse
import sys

# Absolute timeline base; the player only uses relative (t - T0) timing,
# so the exact absolute value is arbitrary but fixed for determinism.
T0 = 100.0

# All timestamps in this spec are exact multiples of 5 ms (the GCD of the
# 5 ms / 10 ms / 20 ms / 35 ms / ... / 100 ms / 1000 ms intervals used
# below). We generate every timestamp as an integer count of 5 ms "ticks"
# and convert to seconds with a single multiplication at the very end, so
# two logically-equal timestamps (e.g. a noise frame and a BAM frame that
# both land on t=100.565) produce bit-identical floats. Building the same
# value via different chains of float addition (e.g. `113 * 0.005` vs.
# `0.500 + 0.065`) can differ in the last bit, which would silently break
# the "stable tie-break by emission order" rule the candump sort relies on.
TICK_S = 0.005


def tick_to_seconds(tick):
    return tick * TICK_S


def fmt_data(data_bytes):
    """Format a payload as uppercase hex pairs, no separators."""
    return "".join(f"{b:02X}" for b in data_bytes)


def gen_noise():
    """Section 1: PGN 0xF004 (5 ms) and PGN 0xFEF1 (100 ms) noise frames.

    Emission order: all PGN 0xF004 frames, then all PGN 0xFEF1 frames --
    this is the tie-break order used when either collides in timestamp
    with anything emitted later (DM1, BAM).
    """
    frames = []
    f004_payload = bytes.fromhex("FFFF001900FFFFFF")
    for i in range(1600):
        t = tick_to_seconds(i)  # every 1 tick = 5 ms
        frames.append((t, "0CF00400", f004_payload))
    fef1_payload = bytes.fromhex("FFFFFFFFFFFFFFFF")
    for i in range(80):
        t = tick_to_seconds(20 * i)  # every 20 ticks = 100 ms
        frames.append((t, "18FEF100", fef1_payload))
    return frames


def gen_dm1():
    """Section 2: DM1 active-DTC broadcasts from 9 source addresses."""
    frames = []
    source_addrs = [0x00, 0x03, 0x0B, 0x0F, 0x10, 0x17, 0x19, 0x21, 0x31]
    payload_sa00 = bytes.fromhex("03FF0100007FFFFF")
    payload_other = bytes.fromhex("00FF0000007FFFFF")
    for idx, sa in enumerate(source_addrs):
        start_tick = 40 + 2 * idx  # 0.200s = 40 ticks, 0.010s = 2 ticks
        payload = payload_sa00 if sa == 0x00 else payload_other
        can_id = f"18FECA{sa:02X}"
        for round_ in range(8):
            t = tick_to_seconds(start_tick + 200 * round_)  # 1.0s = 200 ticks
            frames.append((t, can_id, payload))
    return frames


def _tp_dt_frames(can_id, payload, packet_count, dt_times):
    """Build TP.DT frames: seq byte + up to 7 payload bytes, 0xFF padded."""
    frames = []
    for k, t in zip(range(1, packet_count + 1), dt_times):
        chunk = payload[7 * (k - 1) : 7 * k]
        data = bytes([k]) + bytes(chunk)
        if len(data) < 8:
            data = data + bytes([0xFF] * (8 - len(data)))
        frames.append((t, can_id, data))
    return frames


def gen_bam():
    """Section 3: 3 concurrent BAM session pairs (engine + retarder config).

    Engine Configuration (SA 0x00, PGN 0xFEE3, 39 bytes, 6 TP.DT packets)
    and Retarder Configuration (SA 0x0F, PGN 0xFEE1, 19 bytes, 3 TP.DT
    packets) open ~10 ms apart and their TP.DT streams interleave on the
    wire before either session closes.
    """
    frames = []

    engine_payload = bytearray(39)
    engine_payload[0] = 0xC0
    engine_payload[1] = 0x12
    for i in range(2, 39):
        engine_payload[i] = 0xA0 + i

    retarder_payload = bytearray(19)
    for i in range(19):
        retarder_payload[i] = 0x50 + i

    engine_cm = bytes.fromhex("20270006FFE3FE00")
    retarder_cm = bytes.fromhex("20130003FFE1FE00")

    # Base ticks for T in {0.500, 3.000, 5.500} seconds (5 ms/tick).
    for base_tick in (100, 600, 1100):
        frames.append((tick_to_seconds(base_tick), "1CECFF00", engine_cm))
        # Engine TP.DT offsets: 0.020/0.050/0.080/0.110/0.140/0.170s = 4/10/16/22/28/34 ticks
        engine_dt_times = [
            tick_to_seconds(base_tick + off) for off in (4, 10, 16, 22, 28, 34)
        ]
        frames.extend(_tp_dt_frames("1CEBFF00", engine_payload, 6, engine_dt_times))

        # Retarder CM offset: 0.010s = 2 ticks
        frames.append((tick_to_seconds(base_tick + 2), "1CECFF0F", retarder_cm))
        # Retarder TP.DT offsets: 0.035/0.065/0.095s = 7/13/19 ticks
        retarder_dt_times = [tick_to_seconds(base_tick + off) for off in (7, 13, 19)]
        frames.extend(_tp_dt_frames("1CEBFF0F", retarder_payload, 3, retarder_dt_times))

    return frames


def generate(end_seconds):
    """Build the full frame list, filter by end time, and stable-sort by t.

    Frames are appended in spec order (noise, then DM1, then BAM); Python's
    sort is stable, so any timestamp tie preserves that emission order.
    """
    all_frames = gen_noise() + gen_dm1() + gen_bam()
    filtered = [f for f in all_frames if f[0] < end_seconds]
    filtered.sort(key=lambda f: f[0])
    return filtered


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("output", help="path to write the candump .log file")
    parser.add_argument(
        "--end", type=float, default=8.0, help="emit only frames with t < end (default 8.0)"
    )
    args = parser.parse_args()

    frames = generate(args.end)

    with open(args.output, "w", newline="\n") as f:
        for t, can_id, data in frames:
            f.write(f"({T0 + t:.6f}) can0 {can_id}#{fmt_data(data)}\n")

    print(f"frames={len(frames)}", file=sys.stderr)


if __name__ == "__main__":
    main()
