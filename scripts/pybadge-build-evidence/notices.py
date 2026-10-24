"""Notice candidates and package metadata, never automatic licence classification."""
import argparse
import hashlib
import json
from pathlib import Path
import re
import subprocess

from origins import COMPONENTS, owner, require_bytes


def sha(data):
    return hashlib.sha256(data).hexdigest()


def leading_comments(data, limit=16 * 1024):
    position = 3 if data.startswith(b"\xef\xbb\xbf") else 0
    end = 0
    while True:
        while position < len(data) and data[position:position + 1] in b" \t\r\n\f":
            position += 1
        if data[position:position + 2] == b"//":
            newline = data.find(b"\n", position)
            position = len(data) if newline == -1 else newline + 1
        elif data[position:position + 2] == b"/*":
            closing = data.find(b"*/", position + 2)
            if closing == -1:
                raise ValueError("unterminated leading comment")
            position = closing + 2
        else:
            return data[:end]
        if position > limit:
            raise ValueError("leading notice candidate exceeds bound")
        end = position


def package_owner(text, path):
    lines = text.splitlines()
    if len(lines) != 1 or ": " not in lines[0]:
        raise ValueError("missing or ambiguous package ownership")
    package, actual_path = lines[0].rsplit(": ", 1)
    if actual_path != str(path) or not re.fullmatch(r"[a-z0-9][a-z0-9+.-]*(?::[a-z0-9][a-z0-9-]*)?", package):
        raise ValueError("unsupported package owner/path")
    return package


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", type=Path, required=True)
    parser.add_argument("--out", type=Path, required=True)
    args = parser.parse_args()
    root, out = args.root.resolve(), args.out.resolve()
    raw_report = (out / "diagnostic-dependencies.json").read_bytes()
    raw_origins = (out / "dependency-origins.json").read_bytes()
    report, origins = json.loads(raw_report), json.loads(raw_origins)
    if origins["diagnosticReportSha256"] != sha(raw_report) or set(origins["records"]) != set(report["files"]):
        raise ValueError("notice input graph is unbound")
    run = lambda arguments: subprocess.run(arguments, check=True, stdout=subprocess.PIPE,
                                         stderr=subprocess.PIPE, timeout=30).stdout
    candidates, packages = {}, {}
    total = 0
    for key, record in report["files"].items():
        name = record["path"]
        owner(name)
        base = root if record["origin"] == "build-source" else Path("/usr")
        if record["origin"] not in ("build-source", "toolchain"):
            raise ValueError("unreviewed dependency origin")
        path = base / name
        if not path.resolve().is_relative_to(base) or path.stat().st_size > 4 * 1024 * 1024:
            raise ValueError("notice source escaped root or size bound")
        raw = path.read_bytes()
        require_bytes(raw, record)
        prefix = leading_comments(raw)
        total += len(prefix)
        if total > 4 * 1024 * 1024:
            raise ValueError("notice candidates exceed aggregate bound")
        candidate = {"sourceSha256": record["sha256"], "bytes": len(prefix),
                     "sha256": sha(prefix), "status": "candidate-unreviewed" if prefix else "no-leading-comment-unreviewed"}
        if prefix:
            relative = "notice-candidates/" + key + ".leading-comments"
            destination = out / relative
            destination.parent.mkdir(parents=True, exist_ok=True)
            with destination.open("xb") as output:
                output.write(prefix)
            candidate["capture"] = relative
        if record["origin"] == "toolchain":
            package = package_owner(run(["dpkg-query", "-S", str(path)]).decode(), path)
            candidate["package"] = package
            if package not in packages:
                version = run(["dpkg-query", "-W", "-f=${binary:Package}\t${Version}\n", package]).decode().strip()
                binary_name, separator, package_version = version.partition("\t")
                if not separator or not package_version or binary_name.split(":")[0] != package.split(":")[0]:
                    raise ValueError("ambiguous installed package version")
                copyright_file = Path("/usr/share/doc") / package.split(":")[0] / "copyright"
                if not copyright_file.resolve().is_relative_to(Path("/usr/share/doc")) or copyright_file.stat().st_size > 1024 * 1024:
                    raise ValueError("package copyright outside reviewed bound")
                copyright_raw = copyright_file.read_bytes()
                total += len(copyright_raw)
                if total > 4 * 1024 * 1024:
                    raise ValueError("notice/package captures exceed aggregate bound")
                relative = "toolchain-notices/" + package.replace(":", "_") + "/copyright"
                destination = out / relative
                destination.parent.mkdir(parents=True, exist_ok=True)
                with destination.open("xb") as output:
                    output.write(copyright_raw)
                packages[package] = {"version": package_version, "capture": relative,
                                     "bytes": len(copyright_raw), "sha256": sha(copyright_raw),
                                     "boundary": "Installed dpkg ownership/version and captured notice; not package archive or binary provenance proof"}
        candidates[key] = candidate
    build_evidence = json.loads((out / "build-evidence.json").read_bytes())
    component_notices = {prefix: [] for prefix in COMPONENTS}
    for record in build_evidence["files"]:
        name = record["path"]
        if not re.fullmatch(r"LICEN[CS]E(?:\.[^/]*)?", Path(name).name, re.IGNORECASE):
            continue
        prefix, relative = owner(name)
        repository, commit = COMPONENTS[prefix]
        raw = (out / "generated" / name).read_bytes()
        require_bytes(raw, record)
        original = run(["git", "-C", str(root / prefix), "show", commit + ":" + relative])
        if original != raw:
            raise ValueError("captured component notice differs from pinned Git content")
        component_notices[prefix].append({"capture": "generated/" + name, "sha256": sha(raw),
                                         "source": "https://github.com/lancaster-university/" + repository + "/blob/" + commit + "/" + relative})
    evidence = {"schema": 1, "candidates": candidates, "packages": packages,
                "componentNotices": component_notices, "diagnosticReportSha256": sha(raw_report),
                "originsSha256": sha(raw_origins),
                "boundary": "Leading comments are unclassified candidates; absent comments/root notices never grant permission. Package origins/notices, obligations and retained bytes require review; no legal/store/admission or isolated implementation claim"}
    with (out / "dependency-notices.json").open("x") as output:
        json.dump(evidence, output, indent=2)
        output.write("\n")
    print(f"Notice evidence captured: {len(candidates)} dependency records, {len(packages)} installed toolchain packages; all unreviewed")


if __name__ == "__main__":
    main()
