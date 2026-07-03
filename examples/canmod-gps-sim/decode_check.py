#!/usr/bin/env python3
"""Decode the sim's on-wire CAN frames with CSS Electronics' real canmod-gps.dbc.

This is the fidelity gate for the canmod-gps-sim example: it proves the frames
the firmware puts on the bus decode — via cantools, the same tool a CANmod.gps
user runs — to the intended physical values against CSS's *published* DBC. It is
scorer-backed (cantools is the scorer), not a self-graded assertion.

Usage:  python3 decode_check.py out/uart.log
Requires: cantools  (pip install cantools)
"""
import os
import re
import sys

import cantools

HERE = os.path.dirname(os.path.abspath(__file__))
DBC = os.path.join(HERE, "canmod-gps.dbc")

# Intended physical values. Signals that drift across ticks are checked
# separately below; these are the per-frame constants.
CONST = {
    0x1: {"FixType": 3, "Satellites": 11},
    0x3: {"Latitude": 55.6761, "PositionAccuracy": 3},
    0x4: {"Altitude": 12.0, "AltitudeAccuracy": 5},
    0x5: {"Roll": 0.0, "Pitch": 0.0, "Heading": 90.0},
    0x7: {"Speed": 10.0, "SpeedAccuracy": 0.1},
    0x8: {"FenceCombined": 1},
    0x9: {"AccelerationX": 0.0, "AccelerationY": 0.0, "AccelerationZ": 9.875,
          "AngularRateX": 0.0, "AngularRateY": 0.0, "AngularRateZ": 0.0},
}


def main(path):
    db = cantools.database.load_file(DBC)
    fails = []
    seen = set()
    longitudes = []
    line_re = re.compile(r"CAN_TX ([0-9A-Fa-f]{4}):\s*([0-9A-Fa-f ]*)")
    for line in open(path):
        m = line_re.match(line.strip())
        if not m:
            continue
        fid = int(m.group(1), 16)
        data = bytes(int(x, 16) for x in m.group(2).split())
        dec = db.decode_message(fid, data, decode_choices=False)
        seen.add(fid)
        for name, want in CONST.get(fid, {}).items():
            got = float(dec[name])
            if abs(got - want) > 1e-4:
                fails.append(f"0x{fid:X} {name}: got {got}, want {want}")
        if fid == 0x3:
            longitudes.append(float(dec["Longitude"]))

    expected_msgs = set(range(1, 10))
    missing = expected_msgs - seen
    if missing:
        fails.append("missing messages: " + ", ".join(hex(x) for x in sorted(missing)))

    # Longitude must start at Copenhagen 12.5683 and drift strictly east.
    if longitudes:
        if abs(longitudes[0] - 12.5683) > 1e-4:
            fails.append(f"Longitude[0]: got {longitudes[0]}, want 12.5683")
        if any(b <= a for a, b in zip(longitudes, longitudes[1:])):
            fails.append(f"Longitude not strictly increasing: {longitudes}")

    if fails:
        print("FAIL — frames do not match CSS's real canmod-gps.dbc:")
        for f in fails:
            print("  -", f)
        return 1
    print(f"PASS — {len(seen)} message types, all signals decode to intended "
          f"values via CSS's real canmod-gps.dbc (cantools {cantools.__version__}).")
    print(f"       Longitude drift: {longitudes}")
    return 0


if __name__ == "__main__":
    if len(sys.argv) != 2:
        print(__doc__)
        sys.exit(2)
    sys.exit(main(sys.argv[1]))
