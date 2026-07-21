#!/usr/bin/env python3
import hashlib
import json
import pathlib
import sys


def fingerprint_payload(result, snapshot, uart):
    # Deliberately exclude CPU registers, cycles, instructions, limits, and
    # stop_reason_details. Keep peripheral end state because it is part of the
    # simulator contract this drift gate protects.
    return {
        "result": {
            "status": result.get("status"),
            "stop_reason": result.get("stop_reason"),
            "steps_executed": result.get("steps_executed"),
            "assertions": result.get("assertions"),
        },
        "peripherals": snapshot.get("peripherals"),
        "uart": uart,
    }


def fingerprint_from_objects(result, snapshot, uart):
    blob = json.dumps(
        fingerprint_payload(result, snapshot, uart),
        sort_keys=True,
        separators=(",", ":"),
    ).encode("utf-8")
    return hashlib.sha256(blob).hexdigest()


def fingerprint_from_files(result_path, snapshot_path, uart_path):
    result = json.loads(pathlib.Path(result_path).read_text())
    snapshot = json.loads(pathlib.Path(snapshot_path).read_text())
    uart = pathlib.Path(uart_path).read_text()
    return fingerprint_from_objects(result, snapshot, uart)


def main(argv):
    if len(argv) != 4:
        print(
            "usage: trace_drift_fingerprint.py <result.json> <snapshot.json> <uart.log>",
            file=sys.stderr,
        )
        return 2
    print(fingerprint_from_files(argv[1], argv[2], argv[3]))
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv))
