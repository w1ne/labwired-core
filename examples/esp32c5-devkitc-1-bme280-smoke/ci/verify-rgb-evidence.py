#!/usr/bin/env python3
"""Require the simulator to decode the ESP32-C5 board's red RGB frame."""

import json
import sys
from pathlib import Path


def artifacts(result):
    inspection = result.get("inspect", {})
    for device in inspection.get("devices", []):
        for artifact in device.get("artifacts", []):
            if "device_type" not in artifact and device.get("device_type"):
                artifact = {**artifact, "device_type": device["device_type"]}
            yield artifact


def frame_bytes(artifact):
    meta = artifact.get("meta", {})
    if meta.get("bytes") is not None:
        yield meta["bytes"]
    for frame in meta.get("frames_seen", []):
        if frame.get("bytes") is not None:
            yield frame["bytes"]


if len(sys.argv) != 2:
    raise SystemExit(f"usage: {sys.argv[0]} RESULT.json")

result_path = Path(sys.argv[1])
result = json.loads(result_path.read_text(encoding="utf-8"))
for artifact in artifacts(result):
    meta = artifact.get("meta", {})
    if (
        artifact.get("kind") == "framebuffer"
        and artifact.get("device_type") == "neopixel"
        and meta.get("data_pin") == 27
        and meta.get("format") == "ws2812_grb"
        and meta.get("pixels_decoded") == 1
        and meta.get("lit_pixels") == 1
        and [0, 255, 0] in frame_bytes(artifact)
    ):
        print("decoded one red WS2812 pixel on C5 GPIO27 (wire bytes GRB=00 FF 00)")
        raise SystemExit(0)

raise SystemExit(f"no one-pixel red WS2812 artifact on GPIO27 in {result_path}")
