"""Bind dependency bytes to original request or pinned Git blobs; no licence decisions."""
import argparse
import hashlib
import json
from pathlib import Path, PurePosixPath
import subprocess

COMPONENTS = {
    ".": ("codal", "7cf09a3a37c8aca6e100b7265c81e6cdf6129dd2"),
    "libraries/codal-core": ("codal-core", "312ae57e0b31f5b9df07a81e9d846945828e3c5a"),
    "libraries/codal-itsybitsy-m4": ("codal-itsybitsy-m4", "6ecd80ccf1abcc126d06653f178ca03a4d1c6421"),
    "libraries/codal-samd": ("codal-samd", "5bd6b93c219c7e784e885ba2d6812809fb6289a8"),
    "libraries/codal-samd/asf4": ("asf4", "6664673f70d9170b4374a726c5322e9be6b3f237"),
    "libraries/codal-samd/samd-peripherals": ("samd-peripherals", "96563308fc7b97646cbe429953e79cb3405846f0"),
}


def sha(data):
    return hashlib.sha256(data).hexdigest()


def require_bytes(data, record):
    if len(data) != record["bytes"] or sha(data) != record["sha256"]:
        raise ValueError("dependency bytes do not match recorded identity")


def owner(name):
    path = PurePosixPath(name)
    if path.is_absolute() or ".." in path.parts or path.as_posix() != name or name == ".":
        raise ValueError("unsafe or noncanonical dependency path")
    for prefix in sorted(COMPONENTS, key=len, reverse=True):
        if prefix != "." and name.startswith(prefix + "/"):
            return prefix, name[len(prefix) + 1:]
    return ".", name


def request_bytes(name, request):
    value = request["replaceFiles"].get("/" + name)
    if not isinstance(value, str):
        raise ValueError("dependency absent from original request")
    return value.encode()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", type=Path, required=True)
    parser.add_argument("--out", type=Path, required=True)
    args = parser.parse_args()
    root, out = args.root.resolve(), args.out.resolve()
    raw_report = (out / "diagnostic-dependencies.json").read_bytes()
    report = json.loads(raw_report)
    raw_request = (out / "request.json").read_bytes()
    request = json.loads(raw_request)
    raw_manifest = (out / "builder-manifest.json").read_bytes()
    commits = json.loads(raw_manifest)["samd51adafruit"]["commits"]
    if set(commits) != set(COMPONENTS) or len(report["files"]) > 8192:
        raise ValueError("unexpected component or dependency census")
    git = lambda repo, *arguments: subprocess.run(
        ["git", "-C", str(repo), *arguments], check=True, stdout=subprocess.PIPE,
        stderr=subprocess.PIPE, timeout=30).stdout
    for prefix, (repository, commit) in COMPONENTS.items():
        expected_url = "https://github.com/lancaster-university/" + repository
        if commits[prefix] != {"url": expected_url, "commit": commit}:
            raise ValueError("component manifest differs from immutable reviewed pin")
        if git(root / prefix, "rev-parse", "HEAD").decode().strip() != commit:
            raise ValueError("actual component HEAD differs from reviewed pin")
    records = {}
    total = 0
    for key, record in report["files"].items():
        name = record["path"]
        owner(name)  # Validate toolchain and generated names too.
        if key != record["origin"] + "/" + name:
            raise ValueError("dependency key/path mismatch")
        if record["origin"] == "toolchain":
            records[key] = {"kind": "toolchain-unreviewed", "sha256": record["sha256"]}
            continue
        if record["origin"] != "build-source":
            raise ValueError("unknown dependency origin")
        if name.startswith("pxtapp/"):
            data = request_bytes(name, request)
            require_bytes(data, record)
            records[key] = {"kind": "original-request", "requestKey": "/" + name, "sha256": sha(data)}
        elif name == "build/codal_extra_definitions.h":
            data = (out / "generated" / name).read_bytes()
            require_bytes(data, record)
            records[key] = {"kind": "captured-generated-header", "sha256": sha(data),
                            "boundary": "Captured-byte binding, not full generator provenance or licensing clearance"}
        else:
            prefix, relative = owner(name)
            repository, commit = COMPONENTS[prefix]
            row = git(root / prefix, "ls-tree", "-z", commit, "--", relative)
            entries = row.split(b"\0")
            if len(entries) != 2 or entries[-1] or b"\t" not in entries[0]:
                raise ValueError("missing or ambiguous pinned Git file")
            metadata, actual_name = entries[0].split(b"\t", 1)
            mode, kind, blob = metadata.decode().split()
            if mode not in ("100644", "100755") or kind != "blob" or actual_name.decode() != relative:
                raise ValueError("pinned dependency is not an exact regular Git blob")
            size = int(git(root / prefix, "cat-file", "-s", blob))
            total += size
            if size > 4 * 1024 * 1024 or total > 64 * 1024 * 1024:
                raise ValueError("pinned source verification exceeds bound")
            data = git(root / prefix, "cat-file", "blob", blob)
            require_bytes(data, record)
            records[key] = {"kind": "pinned-git-blob", "component": prefix, "commit": commit,
                            "blob": blob, "path": relative, "sha256": sha(data),
                            "source": "https://github.com/lancaster-university/" + repository + "/blob/" + commit + "/" + relative}
    result = {"schema": 1, "records": records, "diagnosticReportSha256": sha(raw_report),
              "requestSha256": sha(raw_request), "builderManifestSha256": sha(raw_manifest),
              "boundary": "Hosted byte comparison against exact pinned Git blobs/request/captured header; toolchain and notices unreviewed; no licence classification, retained-inline proof, guest or isolated-implementation claim"}
    with (out / "dependency-origins.json").open("x") as output:
        json.dump(result, output, indent=2)
        output.write("\n")
    print(f"Dependency origin binding captured: {len(records)} records; no licence decisions")


if __name__ == "__main__":
    main()
