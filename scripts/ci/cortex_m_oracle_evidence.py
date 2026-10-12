"""Verify positive firmware/oracle receipts; successful libtest alone is insufficient."""

import hashlib
import json
import os
from pathlib import Path
import re
import selectors
import signal
import struct
import subprocess
import sys
import time


def collect_log(command, path, max_bytes=1000000, timeout_seconds=300):
    """Bound output as it arrives; own and terminate the producer's process group."""
    if not command or max_bytes < 1 or timeout_seconds <= 0:
        raise ValueError("invalid bounded producer configuration")
    overflow = timed_out = False
    retained = 0
    with Path(path).open("wb") as log:
        process = subprocess.Popen(command, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, start_new_session=True)
        selector = selectors.DefaultSelector()
        selector.register(process.stdout, selectors.EVENT_READ)
        deadline = time.monotonic() + timeout_seconds
        try:
            while selector.get_map() or process.poll() is None:
                remaining = deadline - time.monotonic()
                if remaining <= 0:
                    timed_out = True
                    break
                for key, _ in selector.select(min(remaining, 0.1)):
                    chunk = os.read(key.fd, 16384)
                    if not chunk:
                        selector.unregister(key.fileobj)
                        continue
                    available = max_bytes - retained
                    log.write(chunk[:available])
                    retained += min(len(chunk), available)
                    if len(chunk) > available:
                        overflow = True
                        break
                if overflow:
                    break
        finally:
            # This session was created by Popen above. Kill no pre-existing
            # processes; descendants must also stop if the producer exits early.
            for sig in (signal.SIGTERM, signal.SIGKILL):
                try:
                    os.killpg(process.pid, sig)
                except ProcessLookupError:
                    pass
                if sig == signal.SIGTERM:
                    try:
                        process.wait(timeout=0.2)
                    except subprocess.TimeoutExpired:
                        pass
            process.wait()
            process.stdout.close()
            selector.close()
    code = 125 if overflow else 124 if timed_out else process.returncode
    if code < 0:
        code = 128 - code
    return {"exit_code": code, "producer_exit_code": process.returncode,
            "overflow": overflow, "timed_out": timed_out, "bytes": retained}


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
        if len(argv) >= 5 and argv[1] == "collect" and argv[3] == "--":
            evidence = collect_log(argv[4:], argv[2])
            print(json.dumps(evidence))
            return evidence["exit_code"]
        elif len(argv) == 3 and argv[1] == "fixture":
            evidence = verify_fixture(argv[2])
        elif len(argv) == 5 and argv[1] == "log":
            evidence = verify_log(argv[3], argv[2], int(argv[4]))
        else:
            raise ValueError("usage: fixture PATH | log ORACLE PATH EXIT_CODE | collect PATH -- COMMAND")
        print(json.dumps(evidence))
        return 0
    except (ValueError, OSError, UnicodeError) as error:
        print(f"FAIL-ORACLE-EVIDENCE {error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    sys.exit(main(sys.argv))
