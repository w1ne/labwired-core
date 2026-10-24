#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Execute exact pinned PXT method fragments, not the full ARM runtime."""
import argparse
import hashlib
import json
from pathlib import Path
import subprocess
import tempfile
import urllib.request

PIN = "31abf23d118f35010fb75122e60a1eb2b6dffe9f"
INPUTS = {
    "libs/screen---st7735/screen.cpp":
        "dea9ea175d65d885275eb0715d56353674feae88d64d29a5eb2809f899a0d958",
    "LICENSE": "dea9265341829002e2c23a7372393eb2ed6e26085fb623f38a4ba0af833f30a6",
}
FRAGMENTS = {
    "sendIndexedImage444": "95429c6b140c357228bd9a2f56a65cafca75d83b7b2f56fa338bba25f3bdce05",
    "boardIdPredicate": "89f00c6fbca7dc68e7fdee9a2a459f61a5beb0e8b5337df7adaefa70523c8a71",
}


def fragment(source, start, end):
    if source.count(start) != 1:
        raise RuntimeError("ambiguous pinned fragment start")
    begin = source.index(start)
    finish = source.index(end, begin)
    return source[begin:finish]


def download_inputs():
    downloaded = {}
    for name, expected in INPUTS.items():
        url = f"https://raw.githubusercontent.com/microsoft/pxt-common-packages/{PIN}/{name}"
        with urllib.request.urlopen(url, timeout=30) as response:
            data = response.read(65537)
        if len(data) > 65536 or hashlib.sha256(data).hexdigest() != expected:
            raise RuntimeError(f"pinned input mismatch: {name}")
        downloaded[name] = data
    return downloaded


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--work-dir", required=True)
    parser.add_argument("--output", required=True)
    parser.add_argument("--notice-output", required=True)
    args = parser.parse_args()
    root = Path(__file__).resolve().parent
    downloaded = download_inputs()
    source = downloaded["libs/screen---st7735/screen.cpp"]
    # Preserve the exact bytes, including comments/indentation. Only the host
    # class/transport boundary is authored. No source substitutions or fixes.
    method = fragment(source, b"    void sendIndexedImage444(", b"\n#endif\n};")
    predicate = fragment(
        source,
        b"        uint32_t boardId = (uint32_t)getConfig(CFG_BOOTLOADER_BOARD_ID, 0);",
        b"\n#endif",
    )
    hashes = {
        "sendIndexedImage444": hashlib.sha256(method).hexdigest(),
        "boardIdPredicate": hashlib.sha256(predicate).hexdigest(),
    }
    if hashes != FRAGMENTS:
        raise RuntimeError("pinned fragment mismatch")
    with tempfile.TemporaryDirectory(prefix="pxt-direct-", dir=args.work_dir) as folder:
        work = Path(folder)
        (work / "method.inc").write_bytes(method)
        (work / "predicate.inc").write_bytes(predicate)
        (work / "LICENSE-pxt-common-packages").write_bytes(downloaded["LICENSE"])
        compiler = subprocess.check_output(["g++", "--version"], text=True).splitlines()[0]
        binary = work / "trace"
        subprocess.run([
            "g++", "-m32", "-std=c++17", "-O0", "-Wall", "-Wextra", "-pedantic",
            "-I", str(work), str(root / "trace.cc"), "-o", str(binary),
        ], check=True, timeout=60)
        captured = json.loads(subprocess.check_output([str(binary)], text=True, timeout=15))
        expected_cases = ["small-even", "odd-width-padding", "full-width-batch", "odd-width-batch"]
        if [case["name"] for case in captured["cases"]] != expected_cases:
            raise RuntimeError("missing direct renderer cases")
        negative = subprocess.run(
            [str(binary), "--corrupt-data-capture"],
            text=True, capture_output=True, timeout=15,
        )
        if negative.returncode == 0 or "RAMWR mismatch" not in negative.stderr:
            raise RuntimeError("named corrupt-data mutant did not fail")
    result = {
        "schema": "labwired.st7735.pxt-direct-fragment-trace.v1",
        "sourcePin": PIN,
        "inputSha256": INPUTS,
        "fragmentSha256": hashes,
        "compiler": compiler,
        "architecture": "host x86 32-bit; not ARM",
        "capture": captured,
        "negativeControl": "corrupt first RAMWR data byte rejected with RAMWR mismatch",
        "limits": [
            "exact original method and board-ID predicate fragments, not full WDisplay translation unit",
            "authored palette fields, source buffers, selection invocation and synchronous SPI/pin boundary",
            "does not prove actual PXT Image storage, build macro, CF2 loading or production branch dispatch",
            "odd-width padding and original transfer batches preserved, not corrected or normalized",
            "no panel colour profile, ARM/SAM/DMA/IRQ/fiber, browser, timing or RTx qualification",
        ],
    }
    text = json.dumps(result, indent=2) + "\n"
    Path(args.output).write_text(text, encoding="utf-8")
    Path(args.notice_output).write_bytes(downloaded["LICENSE"])
    print(text, end="")


if __name__ == "__main__":
    main()
