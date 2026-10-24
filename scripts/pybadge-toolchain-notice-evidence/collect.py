"""Bounded exact-source packaging notices; never licence/admission clearance."""
import argparse
import hashlib
import io
import json
import lzma
from pathlib import Path, PurePosixPath
import re
import tarfile
import urllib.parse
import urllib.request
import zipfile

RUN = 37821668715
HEAD = "0d7645361f89be082a510e06337b330e9a946a7d"
ARTIFACT = 11569199112
DIGEST = "9f5059261b2040a3c41d9da2f31eaadab9d83f9b8a66431f1fad62e71bf9bbd9"
ARCHIVES = {
    "gcc-arm-none-eabi@15:13.2.rel1-2": (
        "gcc-arm-none-eabi_13.2.rel1-2.debian.tar.xz", 19812,
        "ba61e1e9268a753890d206c49601d694683b3f306676409a5fdec05163de1025"),
    "libstdc++-arm-none-eabi@26": (
        "libstdc++-arm-none-eabi_26.tar.xz", 4864,
        "bd4bdbfcabe2a2f2d8b1b768e5e8f1f250ee98bba801efcf99c24f74e82f6daf"),
    "newlib@4.4.0.20231231-2": (
        "newlib_4.4.0.20231231-2.debian.tar.xz", 13736,
        "7487340dd0f9a4cb17fed22b16adcf86235b3f81c22b8cf16634bc2cbec19986"),
}


def sha(raw):
    return hashlib.sha256(raw).hexdigest()


def notice_name(path):
    return PurePosixPath(path).name.lower() in {
        "copyright", "copyright-gcc", "copying", "copying3", "copying3.lib",
        "copying.runtime", "license", "licence"}


def source_url(descriptor_url, filename):
    parsed = urllib.parse.urlsplit(descriptor_url)
    if (parsed.scheme != "https" or parsed.netloc != "archive.ubuntu.com" or parsed.query
            or parsed.fragment or not parsed.path.startswith("/ubuntu/pool/")
            or ".." in PurePosixPath(parsed.path).parts
            or not re.fullmatch(r"[A-Za-z0-9][A-Za-z0-9.+_-]*", filename)):
        raise ValueError("unsafe source archive URL")
    return urllib.parse.urlunsplit((parsed.scheme, parsed.netloc,
                                   str(PurePosixPath(parsed.path).parent / filename), "", ""))


def read_notices(raw):
    if len(raw) > 1024 * 1024:
        raise ValueError("packaging archive exceeds compressed bound")
    try:
        decoder = lzma.LZMADecompressor(format=lzma.FORMAT_XZ, memlimit=64 * 1024 * 1024)
        expanded = decoder.decompress(raw, max_length=4 * 1024 * 1024 + 1)
    except lzma.LZMAError as error:
        raise ValueError("malformed or over-memory XZ archive") from error
    if len(expanded) > 4 * 1024 * 1024 or not decoder.eof or decoder.unused_data:
        raise ValueError("expanded XZ bound, truncation or trailing stream")
    inventory, notices, seen, total = [], {}, set(), 0
    with tarfile.open(fileobj=io.BytesIO(expanded), mode="r:") as archive:
        for member in archive:
            path = PurePosixPath(member.name)
            name = path.as_posix()
            if path.is_absolute() or ".." in path.parts or name in seen:
                raise ValueError("unsafe or duplicate archive member")
            seen.add(name)
            if len(seen) > 256 or member.size < 0:
                raise ValueError("packaging member count/size bound")
            total += member.size
            if total > 4 * 1024 * 1024:
                raise ValueError("packaging expanded-member bound")
            if not member.isfile() and not member.isdir():
                raise ValueError("unsupported nonregular packaging member")
            inventory.append({"path": name, "bytes": member.size,
                              "type": "file" if member.isfile() else "directory"})
            if notice_name(name) and member.isfile():
                if member.size > 1024 * 1024:
                    raise ValueError("notice exceeds bound")
                with archive.extractfile(member) as stream:
                    content = stream.read(1024 * 1024 + 1)
                if len(content) != member.size:
                    raise ValueError("notice member length mismatch")
                notices[name] = content
    return inventory, notices


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--prior", type=Path, required=True)
    parser.add_argument("--run-record", type=Path, required=True)
    parser.add_argument("--artifact-record", type=Path, required=True)
    parser.add_argument("--out", type=Path, required=True)
    args = parser.parse_args()
    run_raw, artifact_raw = args.run_record.read_bytes(), args.artifact_record.read_bytes()
    run, artifacts = json.loads(run_raw), json.loads(artifact_raw)
    if (run["id"] != RUN or run["head_sha"] != HEAD or run["run_attempt"] != 1
            or run["conclusion"] != "success" or run["event"] != "pull_request"
            or run["path"] != ".github/workflows/pybadge-toolchain-source-evidence.yml"):
        raise ValueError("unqualified metadata run")
    selected = [a for a in artifacts["artifacts"] if a["id"] == ARTIFACT]
    if (len(selected) != 1 or selected[0]["digest"] != "sha256:" + DIGEST
            or selected[0]["workflow_run"]["id"] != RUN
            or selected[0]["workflow_run"]["head_sha"] != HEAD):
        raise ValueError("metadata artifact identity mismatch")
    if args.prior.stat().st_size > 1024 * 1024:
        raise ValueError("metadata ZIP exceeds bound")
    with args.prior.open("rb") as stream:
        if hashlib.file_digest(stream, "sha256").hexdigest() != DIGEST:
            raise ValueError("metadata ZIP hash mismatch")
    with zipfile.ZipFile(args.prior) as archive:
        if len(archive.namelist()) != len(set(archive.namelist())):
            raise ValueError("duplicate metadata ZIP members")
        if archive.getinfo("source-packages.json").file_size > 1024 * 1024:
            raise ValueError("source report exceeds bound")
        source_raw = archive.read("source-packages.json")
        sources = json.loads(source_raw)
        record = sources["priorNotices"]
        if archive.getinfo(record["capture"]).file_size > 1024 * 1024:
            raise ValueError("prior notices exceed bound")
        notices_raw = archive.read(record["capture"])
        if len(notices_raw) != record["bytes"] or sha(notices_raw) != record["sha256"]:
            raise ValueError("prior notice binding mismatch")
    if set(sources["sources"]) != set(ARCHIVES) or sources["completeNoticeChain"] is not False:
        raise ValueError("unexpected source set/clearance status")
    prior_notices = json.loads(notices_raw)
    out = args.out
    out.mkdir(parents=True, exist_ok=True)

    def capture(name, content):
        path = out / name
        path.parent.mkdir(parents=True, exist_ok=True)
        with path.open("xb") as stream:
            stream.write(content)
        return {"capture": name, "bytes": len(content), "sha256": sha(content)}

    result = {"schema": 1, "inputRun": RUN, "inputHead": HEAD, "inputArtifact": ARTIFACT,
              "inputArchiveSha256": DIGEST, "runRecord": capture("input-run.json", run_raw),
              "artifactRecord": capture("input-artifacts.json", artifact_raw),
              "sourceReport": capture("source-packages.json", source_raw),
              "priorNotices": capture("prior-notices.json", notices_raw),
              "archives": {}, "completeNoticeChain": False}
    for key, (filename, size, digest) in ARCHIVES.items():
        source = sources["sources"][key]
        if source["archiveMembersNotFetched"][filename] != {"bytes": size, "sha256": digest}:
            raise ValueError("selected archive pin mismatch")
        url = source_url(source["url"], filename)
        with urllib.request.urlopen(url, timeout=60) as response:
            if response.geturl() != url:
                raise ValueError("unexpected archive redirect")
            raw = response.read(1024 * 1024 + 1)
        if len(raw) != size or sha(raw) != digest:
            raise ValueError("source archive byte binding mismatch")
        inventory, found = read_notices(raw)
        records = []
        package_names = [p for p, r in sources["packages"].items()
                         if r["source"] + "@" + r["sourceVersion"] == key]
        for name, content in found.items():
            records.append({"member": name, **capture("notices/" + filename + "/" + name, content),
                            "matchesCapturedInstalledCopyright": [p for p in package_names
                                if sha(content) == prior_notices["packages"][p]["sha256"]]})
        observed = {PurePosixPath(name).name.lower() for name in found}
        result["archives"][key] = {"filename": filename, "url": url, "bytes": len(raw), "sha256": sha(raw),
                                   "memberInventory": inventory, "selectedNotices": records,
                                   "missingNamedNotices": [n for n in ("COPYING3", "COPYING.RUNTIME", "copyright-gcc")
                                                           if n.lower() not in observed]}
    result["boundary"] = ("Whole compressed packaging archives acquired and hash-checked; only regular notice members read/extracted. "
                          "Implementation members are listed, not inspected/extracted/executed; raw archives are not uploaded. "
                          "No large GCC/newlib original archive, compiler/build/guest or grant inferred. "
                          "Missing references, exact exception correspondence, retained bytes and obligations remain open; not isolated implementation.")
    with (out / "packaging-notices.json").open("x") as stream:
        json.dump(result, stream, indent=2)
        stream.write("\n")
    print(f"Captured {sum(len(a['selectedNotices']) for a in result['archives'].values())} notice candidates from three exact small packaging archives; no clearance")


if __name__ == "__main__":
    main()
