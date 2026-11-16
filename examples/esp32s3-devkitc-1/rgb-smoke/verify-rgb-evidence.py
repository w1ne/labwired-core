#!/usr/bin/env python3
"""Require the simulator to decode the v1.1 RGB LED's one red pixel."""

import json
import sys
from pathlib import Path


def artifacts(result):
    inspection = result.get("inspect", {})
    for group in ("peripherals", "devices"):
        for item in inspection.get(group, []):
            for artifact in item.get("artifacts", []):
                if "device_type" not in artifact and item.get("device_type"):
                    artifact = {**artifact, "device_type": item["device_type"]}
                yield artifact


def decoded_frames(artifact):
    meta = artifact.get("meta", {})
    direct = artifact.get("bytes")
    if direct is not None:
        yield direct
    for frame in meta.get("frames_seen", []):
        yield frame.get("bytes")


if len(sys.argv) != 2:
    raise SystemExit(f"usage: {sys.argv[0]} RESULT.json")

result_path = Path(sys.argv[1])
result = json.loads(result_path.read_text(encoding="utf-8"))
for artifact in artifacts(result):
    meta = artifact.get("meta", {})
    if (
        artifact.get("kind") == "framebuffer"
        and artifact.get("device_type") == "neopixel"
        and meta.get("format") == "ws2812_grb"
        and meta.get("pixels_decoded") == 1
        and meta.get("lit_pixels") == 1
        and [0, 255, 0] in decoded_frames(artifact)
    ):
        print("decoded one red WS2812 pixel on GPIO38 (wire bytes GRB=00 FF 00)")
        raise SystemExit(0)

raise SystemExit(f"no one-pixel red WS2812 artifact in {result_path}")
