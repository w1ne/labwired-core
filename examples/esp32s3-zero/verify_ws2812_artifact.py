#!/usr/bin/env python3
"""Require the modeled ESP32-S3-Zero WS2812 to decode the transmitted frame."""

import json
import sys
from pathlib import Path


def main() -> int:
    if len(sys.argv) != 2:
        print(f"usage: {sys.argv[0]} RESULT_JSON", file=sys.stderr)
        return 2

    result_path = Path(sys.argv[1])
    result = json.loads(result_path.read_text(encoding="utf-8"))
    if result.get("status") != "pass":
        raise SystemExit(f"smoke result is {result.get('status')!r}, expected 'pass'")
    if any(not item.get("passed", False) for item in result.get("assertions", [])):
        raise SystemExit("one or more smoke assertions failed")

    device = next(
        (
            item
            for item in result.get("inspect", {}).get("devices", [])
            if item.get("id") == "status_neopixel"
        ),
        None,
    )
    if device is None or device.get("device_type") != "neopixel":
        raise SystemExit("status_neopixel device artifact is missing")

    artifact = next(
        (
            item
            for item in device.get("artifacts", [])
            if item.get("kind") == "framebuffer"
            and item.get("meta", {}).get("format") == "ws2812_grb"
        ),
        None,
    )
    if artifact is None:
        raise SystemExit("WS2812 framebuffer artifact is missing")

    meta = artifact["meta"]
    valid_frame = any(
        frame.get("bytes") == [64, 0, 0] and frame.get("lit_pixels") == 1
        for frame in meta.get("frames_seen", [])
    )
    if meta.get("data_pin") != 21:
        raise SystemExit(f"WS2812 data pin is {meta.get('data_pin')!r}, expected 21")
    if meta.get("pixels_decoded", 0) < 1 or meta.get("lit_pixels", 0) < 1 or not valid_frame:
        raise SystemExit(f"expected one lit green pixel [64, 0, 0], got {meta!r}")

    print("WS2812 model artifact: GPIO21 decoded one lit green pixel [64, 0, 0]")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
