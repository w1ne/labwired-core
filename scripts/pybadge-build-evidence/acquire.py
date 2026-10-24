"""Acquire only immutable public worker/target inputs on the hosted runner."""
import argparse
import hashlib
import io
import json
from pathlib import Path
import tarfile
import time
import urllib.request

PINS = (
    ("pxt-arcade", "4.2.1", "d403926da1c96dd71a696f0182dbdd82b97babb9d0de9281b8e2a6b67c57cf32",
     {"package/built/target.json": "target.json", "package/LICENSE": "LICENSE-pxt-arcade.txt"}),
    ("pxt-core", "13.2.1", "e2c4c2a3f353f02abff1eff98ad066eab28e643311d0688961fb8b0388578033",
     {"package/built/web/pxtworker.js": "pxtworker.js", "package/LICENSE": "LICENSE-pxt-core.txt"}),
)
MAX_BYTES = 64 * 1024 * 1024


def select_members(raw, expected_sha, members):
    if hashlib.sha256(raw).hexdigest() != expected_sha:
        raise ValueError("package SHA256 mismatch; refusing changed inputs")
    result = {}
    with tarfile.open(fileobj=io.BytesIO(raw), mode="r:gz") as archive:
        for member in archive:
            if member.name not in members:
                continue
            if not member.isfile() or member.name in result or member.size > MAX_BYTES:
                raise ValueError("ambiguous, linked or oversized selected member")
            data = archive.extractfile(member).read(MAX_BYTES + 1)
            if len(data) != member.size or len(data) > MAX_BYTES:
                raise ValueError("selected member size mismatch")
            result[member.name] = data
    if set(result) != set(members):
        raise ValueError("missing required package member")
    return {members[name]: data for name, data in result.items()}


def fetch(url):
    for attempt in range(1, 4):
        try:
            with urllib.request.urlopen(url, timeout=30) as response:
                data = response.read(MAX_BYTES + 1)
            if len(data) > MAX_BYTES:
                raise ValueError("package exceeds acquisition bound")
            return data
        except (OSError, TimeoutError) as error:
            print(f"Public input acquisition attempt {attempt}/3: {url}: {error}", flush=True)
            if attempt == 3:
                raise
            time.sleep(attempt)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--lite", type=Path, required=True)
    parser.add_argument("--out", type=Path, required=True)
    args = parser.parse_args()
    args.out.mkdir(parents=True, exist_ok=True)
    dest = args.lite / "packages/scratch-gui/static/makecode/arcade"
    dest.mkdir(parents=True, exist_ok=True)
    records = []
    for name, version, sha, members in PINS:
        url = f"https://registry.npmjs.org/{name}/-/{name}-{version}.tgz"
        raw = fetch(url)
        selected = select_members(raw, sha, members)
        for filename, data in selected.items():
            with (dest / filename).open("xb") as output:
                output.write(data)
        records.append({"url": url, "sha256": sha, "bytes": len(raw), "members": {
            filename: {"bytes": len(data), "sha256": hashlib.sha256(data).hexdigest()}
            for filename, data in selected.items()
        }})
        # Retain successful staged acquisition when a subsequent input fails.
        (args.out / "acquisition.json").write_text(json.dumps(records, indent=2) + "\n")
    print("Two SHA-bound public packages acquired; no firmware bases downloaded")


if __name__ == "__main__":
    main()
