#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Hosted-only exact-source driver trace; not SAM/DMA/guest qualification."""
import argparse
import hashlib
import json
from pathlib import Path
import subprocess
import tempfile
import urllib.request

PIN = "312ae57e0b31f5b9df07a81e9d846945828e3c5a"
INPUTS = {
    "source/drivers/ST7735.cpp": "d8aafbdd338c9501c314f33983eb1ceb1229bc0acfcd10c28a4d2527d756c805",
    "inc/drivers/ST7735.h": "41263645d09fe352ac21ddf72d3aa762c30465f90388ccad18a5775a899f5f23",
}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--work-dir", required=True)
    parser.add_argument("--output", required=True)
    parser.add_argument("--fixture")
    args = parser.parse_args()
    root = Path(__file__).resolve().parent
    with tempfile.TemporaryDirectory(prefix="st7735-codal-", dir=args.work_dir) as folder:
        work = Path(folder)
        for name, expected in INPUTS.items():
            url = f"https://raw.githubusercontent.com/lancaster-university/codal-core/{PIN}/{name}"
            with urllib.request.urlopen(url, timeout=30) as response:
                data = response.read(65537)
            if len(data) > 65536 or hashlib.sha256(data).hexdigest() != expected:
                raise RuntimeError(f"pinned input mismatch: {name}")
            (work / Path(name).name).write_bytes(data)
        compiler = subprocess.check_output(["g++", "--version"], text=True).splitlines()[0]
        binary = work / "trace"
        # Match the original ARM driver's 32-bit pointer/unsigned assumptions;
        # do not hide a pointer-truncation error with -fpermissive on a 64-bit host.
        subprocess.run([
            "g++", "-m32", "-std=c++17", "-O0", "-Wall", "-Wextra", "-pedantic",
            "-I", str(root / "stubs"), "-I", str(work),
            str(work / "ST7735.cpp"), str(root / "trace.cc"), "-o", str(binary),
        ], check=True, timeout=60)
        cases = json.loads(subprocess.check_output([str(binary)], text=True, timeout=15))
        if [case["width"] for case in cases] != [3, 4, 5]:
            raise RuntimeError("missing driver cases")
        fixture_sha = None
        if args.fixture:
            fixture_bytes = Path(args.fixture).read_bytes()
            fixture_sha = hashlib.sha256(fixture_bytes).hexdigest()
            if fixture_sha != "2e4d70928807baa1b3110a2a09ef6c32d9482ab6fa32ac0ce0cd8efe76b2d062":
                raise RuntimeError("original captured fixture hash mismatch")
            fixture = json.loads(fixture_bytes)
            if (fixture["sourcePin"] != PIN or fixture["inputSha256"] != INPUTS
                    or fixture["cases"] != cases):
                raise RuntimeError("live original driver does not match guest fixture")
        negative = subprocess.run(
            [str(binary), "--corrupt-window-capture"],
            text=True, capture_output=True, timeout=15,
        )
        if negative.returncode == 0 or "CASET mismatch" not in negative.stderr:
            raise RuntimeError("named corrupt-window capture mutant did not fail")
        result = {
            "schema": "labwired.st7735.codal-host-trace.v1",
            "sourcePin": PIN,
            "inputSha256": INPUTS,
            "compiler": compiler,
            "architecture": "host x86 32-bit; not ARM",
            "cases": cases,
            "negativeControl": "corrupt CASET capture rejected with CASET mismatch",
            "guestFixtureSha256": fixture_sha,
            "limits": [
                "actual original driver/header with artificial host transport/events",
                "no SAM, DMA, IRQ, deployed CF2/module or physical timing qualification",
                "not an authored Cortex-M guest or performance measurement",
            ],
        }
        text = json.dumps(result, indent=2) + "\n"
        Path(args.output).write_text(text, encoding="utf-8")
        print(text, end="")


if __name__ == "__main__":
    main()
