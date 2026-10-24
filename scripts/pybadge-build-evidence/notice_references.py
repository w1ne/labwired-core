"""Bounded installed notice-reference discovery, not licence-chain closure."""
import argparse
import gzip
import hashlib
import io
import json
from pathlib import Path
import re
import subprocess


def sha(data):
    return hashlib.sha256(data).hexdigest()


def reference_tokens(data):
    return sorted({value.decode() for value in re.findall(rb"\b(?:COPYING\.RUNTIME|COPYING3(?:\.LIB)?)\b", data)})


def selected_notice(path):
    name = path.name.lower().removesuffix(".gz")
    return name in {"copyright", "copyright-gcc", "copying", "copying3", "copying3.lib", "copying.runtime", "license", "licence"}


def decode_notice(raw, compressed, limit=1024 * 1024):
    if len(raw) > limit:
        raise ValueError("raw notice exceeds bound")
    if not compressed:
        return raw
    with gzip.GzipFile(fileobj=io.BytesIO(raw)) as stream:
        decoded = stream.read(limit + 1)
    if len(decoded) > limit:
        raise ValueError("decoded notice exceeds bound")
    return decoded


def missing_references(tokens, listed_names):
    names = {name.lower().removesuffix(".gz") for name in listed_names}
    return [token for token in tokens if token.lower() not in names]


def verify_version(package, expected, raw):
    name, separator, version = raw.decode().strip().partition("\t")
    if (not separator or name.split(":")[0] != package.split(":")[0]
            or version != expected):
        raise ValueError("installed notice package identity/version changed")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--out", type=Path, required=True)
    args = parser.parse_args()
    out = args.out.resolve()
    raw_input = (out / "dependency-notices.json").read_bytes()
    notices = json.loads(raw_input)
    captures, inventories, references = {}, {}, []
    total = 0
    for package, context in notices["packages"].items():
        if not re.fullmatch(r"[a-z0-9][a-z0-9+.-]*(?::[a-z0-9][a-z0-9-]*)?", package):
            raise ValueError("unsupported package context")
        version_result = subprocess.run(
            ["dpkg-query", "-W", "-f=${binary:Package}\t${Version}\n", package],
            check=True, stdout=subprocess.PIPE, stderr=subprocess.PIPE, timeout=30)
        verify_version(package, context["version"], version_result.stdout)
        result = subprocess.run(["dpkg-query", "-L", package], check=True, stdout=subprocess.PIPE,
                                stderr=subprocess.PIPE, timeout=30)
        if len(result.stdout) > 8 * 1024 * 1024:
            raise ValueError("package file inventory exceeds bound")
        lines = result.stdout.decode().splitlines()
        if len(lines) > 20000:
            raise ValueError("package file count exceeds bound")
        selected = sorted({Path(line) for line in lines if selected_notice(Path(line))})
        records = []
        for path in selected:
            allowed = (Path("/usr/share/doc"), Path("/usr/share/common-licenses"))
            if not path.is_absolute() or not any(path.is_relative_to(base) and path.resolve().is_relative_to(base) for base in allowed):
                raise ValueError("notice candidate outside package documentation roots")
            if not path.is_file() or path.stat().st_size > 1024 * 1024:
                raise ValueError("notice candidate missing or oversized")
            raw = path.read_bytes()
            decoded = decode_notice(raw, path.suffix == ".gz")
            total += len(raw) + len(decoded)
            if total > 8 * 1024 * 1024:
                raise ValueError("notice-reference captures exceed bound")
            relative_path = path.relative_to(Path("/usr/share")).as_posix()
            capture = "notice-reference-files/" + package.replace(":", "_") + "/" + relative_path
            destination = out / capture
            destination.parent.mkdir(parents=True, exist_ok=True)
            with destination.open("xb") as output:
                output.write(raw)
            record = {"shareRelativePath": relative_path,
                      "resolvedShareRelativePath": path.resolve().relative_to(Path("/usr/share")).as_posix(),
                      "capture": capture, "bytes": len(raw), "sha256": sha(raw),
                      "decodedBytes": len(decoded), "decodedSha256": sha(decoded)}
            if path.suffix == ".gz":
                record["decodedCapture"] = capture + ".decoded"
                with (out / record["decodedCapture"]).open("xb") as output:
                    output.write(decoded)
            captures[package + "/" + relative_path] = record
            records.append(relative_path)
        inventories[package] = {"version": context["version"], "dpkgListSha256": sha(result.stdout),
                                "selectedDocumentation": records}
    for key, candidate in notices["candidates"].items():
        if "package" not in candidate or "capture" not in candidate:
            continue
        raw = (out / candidate["capture"]).read_bytes()
        if sha(raw) != candidate["sha256"]:
            raise ValueError("notice candidate binding mismatch")
        tokens = reference_tokens(raw)
        if not tokens:
            continue
        package = candidate["package"]
        names = [Path(path).name for path in inventories[package]["selectedDocumentation"]]
        references.append({"candidate": key, "candidateSha256": sha(raw), "package": package,
                           "tokens": tokens, "missingSamePackageNames": missing_references(tokens, names),
                           "status": "unreviewed-reference-candidates"})
    result = {"schema": 1, "inputNoticesSha256": sha(raw_input), "packageInventories": inventories,
              "captures": captures, "references": references, "completeNoticeChain": False,
              "boundary": "Only documentation listed by the exact installed header-owner packages; filename matches are candidates, not resolved grants. Missing same-package names remain explicit; other GCC package references/common licence/exception correspondence and obligations require review"}
    with (out / "notice-references.json").open("x") as output:
        json.dump(result, output, indent=2)
        output.write("\n")
    print(f"Notice-reference discovery: {len(captures)} package documentation files, {len(references)} candidate chains; no clearance inferred")


if __name__ == "__main__":
    main()
