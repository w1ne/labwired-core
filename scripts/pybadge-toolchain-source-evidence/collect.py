"""Bind qualified header-owner versions to APT source records; no build or clearance."""
import argparse
import hashlib
import json
from pathlib import Path, PurePosixPath
import re
import subprocess
import urllib.request
import zipfile

PRIOR_RUN = 37817595613
PRIOR_HEAD = "486a54e42721c0cabd21642d789a071aeee2d8eb"
PRIOR_ARTIFACT = 11568138312
PRIOR_SHA = "e3ca50c8750a0832d1b5c012d9a42a336ea44953400f866f71b7c53e3b191506"
EXPECTED = {"gcc-arm-none-eabi": "15:13.2.rel1-2",
            "libstdc++-arm-none-eabi-dev": "15:13.2.rel1-2+26",
            "libnewlib-dev": "4.4.0.20231231-2"}


def sha(raw):
    return hashlib.sha256(raw).hexdigest()


def paragraphs(raw):
    text = raw.decode()
    if text.startswith("-----BEGIN PGP SIGNED MESSAGE-----\n"):
        _, separator, text = text.partition("\n\n")
        if not separator or "\n-----BEGIN PGP SIGNATURE-----" not in text:
            raise ValueError("malformed clear-signed descriptor")
        text = text.split("\n-----BEGIN PGP SIGNATURE-----", 1)[0]
        text = "\n".join(line[2:] if line.startswith("- ") else line for line in text.splitlines())
    result = []
    for chunk in re.split(r"\n\s*\n", text.strip()):
        fields, previous = {}, None
        for line in chunk.splitlines():
            if line.startswith((" ", "\t")):
                if previous is None:
                    raise ValueError("orphan continuation")
                fields[previous] += "\n" + line.strip()
            else:
                key, separator, value = line.partition(":")
                if not separator or key in fields or not re.fullmatch(r"[A-Za-z][A-Za-z0-9-]*", key):
                    raise ValueError("malformed or duplicate control field")
                fields[key], previous = value.strip(), key
        result.append(fields)
    return result


def exact_record(raw, package, version):
    records = [r for r in paragraphs(raw) if r.get("Package") == package and r.get("Version") == version]
    if not records or any(record != records[0] for record in records):
        raise ValueError("missing or ambiguous exact package record")
    return records[0]


def source_identity(record):
    source = record.get("Source", record["Package"])
    match = re.fullmatch(r"([a-z0-9][a-z0-9+.-]*)(?: \(([^\s()]+)\))?", source)
    if not match:
        raise ValueError("unsupported source identity")
    return match[1], match[2] or record["Version"]


def checksum_files(field):
    result = {}
    for line in field.splitlines():
        if not line.strip():
            continue
        parts = line.split()
        if len(parts) != 3:
            raise ValueError("malformed source checksum")
        digest, size, name = parts
        if (not re.fullmatch(r"[0-9a-f]{64}", digest) or not size.isdigit()
                or not re.fullmatch(r"[A-Za-z0-9][A-Za-z0-9.+_-]*", name)
                or name in result):
            raise ValueError("unsafe or duplicate source member")
        result[name] = {"bytes": int(size), "sha256": digest}
    if not result:
        raise ValueError("empty source checksum inventory")
    return result


def descriptor_url(directory, name):
    path = PurePosixPath(directory)
    if path.is_absolute() or ".." in path.parts or not directory.startswith("pool/"):
        raise ValueError("unsafe archive directory")
    if not re.fullmatch(r"[A-Za-z0-9][A-Za-z0-9.+_-]*\.dsc", name):
        raise ValueError("unsafe descriptor filename")
    return "https://archive.ubuntu.com/ubuntu/" + directory + "/" + name


def verify_descriptor(raw, package, version, listed, descriptor_name):
    if len(raw) != listed[descriptor_name]["bytes"] or sha(raw) != listed[descriptor_name]["sha256"]:
        raise ValueError("descriptor byte binding mismatch")
    records = paragraphs(raw)
    if len(records) != 1 or records[0].get("Source") != package or records[0].get("Version") != version:
        raise ValueError("descriptor source/version mismatch")
    members = checksum_files(records[0]["Checksums-Sha256"])
    if members != {name: record for name, record in listed.items() if name != descriptor_name}:
        raise ValueError("APT/descriptor source member mismatch")
    return members


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--prior", type=Path, required=True)
    parser.add_argument("--run-record", type=Path, required=True)
    parser.add_argument("--artifact-record", type=Path, required=True)
    parser.add_argument("--out", type=Path, required=True)
    args = parser.parse_args()
    run_raw, artifact_raw = args.run_record.read_bytes(), args.artifact_record.read_bytes()
    run, artifacts = json.loads(run_raw), json.loads(artifact_raw)
    if (run["id"] != PRIOR_RUN or run["head_sha"] != PRIOR_HEAD or run["run_attempt"] != 1
            or run["conclusion"] != "success" or run["event"] != "pull_request"
            or run["path"] != ".github/workflows/pybadge-build-evidence.yml"):
        raise ValueError("unqualified prior run")
    selected = [a for a in artifacts["artifacts"] if a["id"] == PRIOR_ARTIFACT]
    if (len(selected) != 1 or selected[0]["digest"] != "sha256:" + PRIOR_SHA
            or selected[0]["workflow_run"]["head_sha"] != PRIOR_HEAD
            or selected[0]["workflow_run"]["id"] != PRIOR_RUN):
        raise ValueError("prior artifact metadata mismatch")
    if args.prior.stat().st_size > 4 * 1024 * 1024:
        raise ValueError("prior archive exceeds bound")
    with args.prior.open("rb") as stream:
        if hashlib.file_digest(stream, "sha256").hexdigest() != PRIOR_SHA:
            raise ValueError("prior archive hash mismatch")
    with zipfile.ZipFile(args.prior) as archive:
        names = archive.namelist()
        if len(names) != len(set(names)) or archive.getinfo("dependency-notices.json").file_size > 1024 * 1024:
            raise ValueError("ambiguous or oversized prior notice inventory")
        notices_raw = archive.read("dependency-notices.json")
    notices = json.loads(notices_raw)
    if {p: r["version"] for p, r in notices["packages"].items()} != EXPECTED:
        raise ValueError("unexpected header-owner package versions")
    out = args.out
    out.mkdir(parents=True, exist_ok=True)

    def capture(name, raw):
        with (out / name).open("xb") as stream:
            stream.write(raw)
        return {"capture": name, "bytes": len(raw), "sha256": sha(raw)}

    def apt(arguments):
        raw = subprocess.run(["apt-cache", *arguments], check=True, stdout=subprocess.PIPE,
                             stderr=subprocess.PIPE, timeout=60).stdout
        if len(raw) > 8 * 1024 * 1024:
            raise ValueError("APT control output exceeds bound")
        return raw

    result = {"schema": 1, "priorRun": PRIOR_RUN, "priorHead": PRIOR_HEAD,
              "priorArtifact": PRIOR_ARTIFACT, "priorArchiveSha256": PRIOR_SHA,
              "priorRunRecord": capture("prior-run.json", run_raw),
              "priorArtifactRecord": capture("prior-artifacts.json", artifact_raw),
              "priorNotices": capture("prior-notices.json", notices_raw),
              "packages": {}, "sources": {}, "completeNoticeChain": False}
    for package, version in EXPECTED.items():
        binary_raw = apt(["show", package + "=" + version])
        binary = exact_record(binary_raw, package, version)
        source, source_version = source_identity(binary)
        result["packages"][package] = {"version": version, "source": source, "sourceVersion": source_version,
                                       "binaryControl": capture(package + ".binary-control", binary_raw)}
        key = source + "@" + source_version
        if key in result["sources"]:
            continue
        source_raw = apt(["showsrc", "--only-source", source])
        source_control = exact_record(source_raw, source, source_version)
        listed = checksum_files(source_control["Checksums-Sha256"])
        descriptors = [name for name in listed if name.endswith(".dsc")]
        if len(descriptors) != 1 or listed[descriptors[0]]["bytes"] > 1024 * 1024:
            raise ValueError("missing/ambiguous/oversized source descriptor")
        name = descriptors[0]
        url = descriptor_url(source_control["Directory"], name)
        with urllib.request.urlopen(url, timeout=60) as response:
            if not response.geturl().startswith("https://archive.ubuntu.com/ubuntu/pool/"):
                raise ValueError("unexpected descriptor redirect")
            raw = response.read(1024 * 1024 + 1)
        members = verify_descriptor(raw, source, source_version, listed, name)
        result["sources"][key] = {"source": source, "version": source_version, "url": url,
                                  "sourceControl": capture(source + ".source-control", source_raw),
                                  "descriptor": capture(name, raw), "archiveMembersNotFetched": members}
    result["boundary"] = ("Exact-version APT binary/source records and checksum-bound descriptors only. "
                          "APT trust/index provenance is a hosted observation; descriptor signatures are not independently verified. "
                          "No source archives, implementation bodies, compiler, firmware build or guest. "
                          "COPYING/exception correspondence, source-to-binary proof, obligations and admission remain unresolved.")
    with (out / "source-packages.json").open("x") as stream:
        json.dump(result, stream, indent=2)
        stream.write("\n")
    print(f"Bound {len(result['packages'])} header-owner packages to {len(result['sources'])} source descriptors; no notice closure")


if __name__ == "__main__":
    main()
