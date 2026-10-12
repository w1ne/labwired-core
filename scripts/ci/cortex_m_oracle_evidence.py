"""Verify positive firmware/oracle receipts; successful libtest alone is insufficient."""

import hashlib
import json
from pathlib import Path
import re
import struct
import sys


def read_file(path):
    path = Path(path)
    if not path.is_file():
        raise ValueError(f"missing evidence: {path}")
    return path.read_bytes()


def verify_fixture(path):
    data = read_file(path)
    if len(data) <= 52 or data[:7] != b"\x7fELF\x01\x01\x01":
        raise ValueError(f"not a nonempty ELF32 little-endian fixture: {path}")
    if struct.unpack_from("<HHI", data, 16) != (2, 40, 1):
        raise ValueError(f"not an ARM executable ELF: {path}")
    if struct.unpack_from("<H", data, 40)[0] != 52 or not any(data[52:]):
        raise ValueError(f"invalid ELF header or empty payload: {path}")
    return {"fixture": str(path), "bytes": len(data), "sha256": hashlib.sha256(data).hexdigest()}


def verify_log(path, oracle, exit_code):
    contracts = {
        "nrf52_conformance": ("conformance_sim", "nRF52840 sim conformance digest (16 words):", 16, 0x52840D0E),
        "nrf52_cpu_conformance": ("cpu_conformance_sim", "nRF52840 sim CPU-conformance digest:", 9, 0x52840D0E),
        "f103_conformance": ("conformance_sim", "sim conformance digest:", 11, 0xC0DEF103),
    }
    if oracle not in contracts or exit_code != 0:
        raise ValueError(f"unknown oracle or unsuccessful exit: {oracle}, {exit_code}")
    text = re.sub(r"\x1b\[[0-9;]*m", "", read_file(path).decode("utf-8"))
    test, marker, count, done = contracts[oracle]
    if re.search(r"^\s*skip[:\s]", text, re.I | re.M):
        raise ValueError(f"oracle skipped: {path}")
    success = re.findall(r"test result: ok\. (\d+) passed; (\d+) failed; (\d+) ignored; (\d+) measured; (\d+) filtered out", text)
    if success != [("1", "0", "0", "0", "0")] or not re.search(rf"^test {test} \.\.\. ok\s*$", text, re.M):
        raise ValueError(f"expected exactly one named, unfiltered passing oracle: {path}")
    if text.count(marker) != 1:
        raise ValueError(f"missing or duplicated concrete firmware digest: {path}")
    digest = text.split(marker, 1)[1]
    if oracle == "f103_conformance":
        match = re.match(r"\s*\[([0-9A-Fa-f, ]+)\]", digest)
        words = [] if not match else [int(word.strip(), 16) for word in match[1].split(",")]
    else:
        rows = re.findall(r"^\s*\[(\d{2})\]\s+(\w+)\s+=\s+0x([0-9A-Fa-f]{8})\s*$", digest, re.M)
        if [int(index) for index, _, _ in rows] != list(range(count)) or not rows or rows[0][1] != "DONE":
            raise ValueError(f"incomplete indexed firmware digest: {path}")
        words = [int(word, 16) for _, _, word in rows]
    if len(words) != count or words[0] != done or any(word > 0xFFFFFFFF for word in words):
        raise ValueError(f"missing DONE or malformed firmware digest: {path}")
    return {"oracle": oracle, "log": str(path), "exit_code": exit_code, "digest_words": words, "sha256": hashlib.sha256(read_file(path)).hexdigest()}


def main(argv):
    try:
        if len(argv) == 3 and argv[1] == "fixture":
            evidence = verify_fixture(argv[2])
        elif len(argv) == 5 and argv[1] == "log":
            evidence = verify_log(argv[3], argv[2], int(argv[4]))
        else:
            raise ValueError("usage: fixture PATH | log ORACLE PATH EXIT_CODE")
        print(json.dumps(evidence))
        return 0
    except (ValueError, OSError, UnicodeError) as error:
        print(f"FAIL-ORACLE-EVIDENCE {error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    sys.exit(main(sys.argv))
