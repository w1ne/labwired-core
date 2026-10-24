"""Pinned hosted GCC notice streaming; source-byte access, not isolated implementation."""
import argparse
import bz2
import hashlib
import json
from pathlib import Path, PurePosixPath
import tarfile
import urllib.request
import zipfile

RUN = 37821668715
HEAD = "0d7645361f89be082a510e06337b330e9a946a7d"
ARTIFACT = 11569199112
INPUT_SHA = "9f5059261b2040a3c41d9da2f31eaadab9d83f9b8a66431f1fad62e71bf9bbd9"
SOURCE = "gcc-arm-none-eabi@15:13.2.rel1-2"
FILENAME = "gcc-arm-none-eabi_13.2.rel1.orig.tar.bz2"
SOURCE_BYTES = 104971782
SOURCE_SHA = "2db06abe865ce68eced25e449aecb45cdb6c151c3551c739620cfc3730214c91"
URL = "https://archive.ubuntu.com/ubuntu/pool/universe/g/gcc-arm-none-eabi/" + FILENAME
NOTICE_NAMES = {"COPYING", "COPYING3", "COPYING3.LIB", "COPYING.RUNTIME"}


def sha(raw):
    return hashlib.sha256(raw).hexdigest()


class BoundedReader:
    def __init__(self, stream, limit):
        self.stream, self.limit, self.count = stream, limit, 0

    def read(self, size):
        if size < 0:
            raise ValueError("unbounded expanded read rejected")
        raw = self.stream.read(min(size, self.limit - self.count + 1))
        self.count += len(raw)
        if self.count > self.limit:
            raise ValueError("expanded stream exceeds bound")
        return raw


def selected_notice(name):
    return PurePosixPath(name).name in NOTICE_NAMES


def scan(stream, capture, expanded_limit=2 * 1024 * 1024 * 1024,
         member_limit=150000, notice_limit=1024 * 1024, total_notice_limit=4 * 1024 * 1024):
    bounded = BoundedReader(stream, expanded_limit)
    headers, seen, records, unsupported = hashlib.sha256(), set(), [], []
    total_payload, total_notices = 0, 0
    with tarfile.open(fileobj=bounded, mode="r|", bufsize=64 * 1024) as archive:
        for member in archive:
            path = PurePosixPath(member.name)
            name = path.as_posix()
            if path.is_absolute() or ".." in path.parts or name in seen or len(name) > 4096:
                raise ValueError("unsafe/duplicate/oversized source member path")
            seen.add(name)
            if len(seen) > member_limit or member.size < 0 or member.size > 512 * 1024 * 1024:
                raise ValueError("source member count/size bound")
            total_payload += member.size
            if total_payload > expanded_limit:
                raise ValueError("source member payload total exceeds bound")
            header = {"path": name, "bytes": member.size, "typeHex": member.type.hex()}
            headers.update(json.dumps(header, sort_keys=True, separators=(",", ":")).encode() + b"\n")
            if selected_notice(name):
                if not member.isfile():
                    unsupported.append({**header, "linkTarget": member.linkname,
                                        "status": "unresolved-nonregular-notice-not-followed"})
                    if len(unsupported) > 128:
                        raise ValueError("nonregular notice count exceeds bound")
                else:
                    if len(records) >= 128 or member.size > notice_limit:
                        raise ValueError("selected notice count/size bound")
                    total_notices += member.size
                    if total_notices > total_notice_limit:
                        raise ValueError("selected notice aggregate bound")
                    with archive.extractfile(member) as body:
                        raw = body.read(notice_limit + 1)
                    if len(raw) != member.size:
                        raise ValueError("selected notice length mismatch")
                    records.append({"member": name, "tarDataOffset": member.offset_data,
                                    **capture(name, raw), "status": "notice-candidate-unreviewed"})
            # Streaming iteration does not need TarFile's accumulated member cache.
            archive.members.clear()
    observed = {PurePosixPath(r["member"]).name for r in records}
    return {"memberCount": len(seen), "memberPayloadBytes": total_payload,
            "expandedBytesReadAtTarStop": bounded.count, "headerInventorySha256": headers.hexdigest(),
            "selectedNotices": records, "unresolvedNonregularNotices": unsupported,
            "missingRegularNoticeNames": sorted(NOTICE_NAMES - observed)}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--prior", type=Path, required=True)
    parser.add_argument("--run-record", type=Path, required=True)
    parser.add_argument("--artifact-record", type=Path, required=True)
    parser.add_argument("--work", type=Path, required=True)
    parser.add_argument("--out", type=Path, required=True)
    args = parser.parse_args()
    if args.out.resolve() == args.work.resolve() or args.work.resolve().is_relative_to(args.out.resolve()):
        raise ValueError("source cache must stay outside uploaded evidence")
    run_raw, artifact_raw = args.run_record.read_bytes(), args.artifact_record.read_bytes()
    run, artifacts = json.loads(run_raw), json.loads(artifact_raw)
    if (run["id"] != RUN or run["head_sha"] != HEAD or run["run_attempt"] != 1
            or run["conclusion"] != "success" or run["event"] != "pull_request"
            or run["path"] != ".github/workflows/pybadge-toolchain-source-evidence.yml"):
        raise ValueError("unqualified source metadata run")
    selected = [a for a in artifacts["artifacts"] if a["id"] == ARTIFACT]
    if (len(selected) != 1 or selected[0]["digest"] != "sha256:" + INPUT_SHA
            or selected[0]["workflow_run"]["id"] != RUN or selected[0]["workflow_run"]["head_sha"] != HEAD):
        raise ValueError("source metadata artifact mismatch")
    if args.prior.stat().st_size > 1024 * 1024:
        raise ValueError("source metadata ZIP exceeds bound")
    with args.prior.open("rb") as stream:
        if hashlib.file_digest(stream, "sha256").hexdigest() != INPUT_SHA:
            raise ValueError("source metadata ZIP hash mismatch")
    with zipfile.ZipFile(args.prior) as archive:
        if len(archive.namelist()) != len(set(archive.namelist())) or archive.getinfo("source-packages.json").file_size > 1024 * 1024:
            raise ValueError("ambiguous/oversized metadata member")
        source_raw = archive.read("source-packages.json")
    source = json.loads(source_raw)["sources"][SOURCE]
    if source["archiveMembersNotFetched"][FILENAME] != {"bytes": SOURCE_BYTES, "sha256": SOURCE_SHA}:
        raise ValueError("original GCC archive pin mismatch")
    if source["url"].rsplit("/", 1)[0] + "/" + FILENAME != URL:
        raise ValueError("original GCC archive origin mismatch")
    args.work.mkdir(parents=True, exist_ok=True)
    args.out.mkdir(parents=True, exist_ok=True)

    def capture(name, raw):
        path = args.out / name
        path.parent.mkdir(parents=True, exist_ok=True)
        with path.open("xb") as stream:
            stream.write(raw)
        return {"capture": name, "bytes": len(raw), "sha256": sha(raw)}

    result = {"schema": 1, "inputRun": RUN, "inputHead": HEAD, "inputArtifact": ARTIFACT,
              "inputArchiveSha256": INPUT_SHA, "runRecord": capture("input-run.json", run_raw),
              "artifactRecord": capture("input-artifacts.json", artifact_raw),
              "sourceReport": capture("source-packages.json", source_raw),
              "sourceArchive": {"url": URL, "filename": FILENAME, "bytes": SOURCE_BYTES, "sha256": SOURCE_SHA},
              "completeNoticeChain": False}
    downloaded, actual = 0, hashlib.sha256()
    archive_path = args.work / FILENAME
    with urllib.request.urlopen(URL, timeout=120) as response, archive_path.open("xb") as destination:
        if response.geturl() != URL:
            raise ValueError("unexpected GCC source redirect")
        while chunk := response.read(1024 * 1024):
            downloaded += len(chunk)
            if downloaded > SOURCE_BYTES:
                raise ValueError("GCC source download exceeds pinned size")
            actual.update(chunk)
            destination.write(chunk)
    if downloaded != SOURCE_BYTES or actual.hexdigest() != SOURCE_SHA:
        raise ValueError("GCC source archive byte binding mismatch")
    with bz2.BZ2File(archive_path, "rb") as expanded:
        result["scan"] = scan(expanded, lambda name, raw: capture("notices/" + name, raw))
    result["boundary"] = ("Whole pinned compressed GCC source archive acquired/hash-checked before streaming. "
                          "Implementation bytes pass through decompression/skipping; not isolated/clean-room or avoidance of byte access. "
                          "Only regular notice candidates are extracted/captured; nonregular named notices are not followed. "
                          "No implementation inspection/execution, compiler/build/guest or source-to-binary/legal/admission clearance. "
                          "Raw source archive and full member inventory are not uploaded; their hashes/extraction remain hosted observations. "
                          "Named file presence does not resolve exact header/exception correspondence or obligations.")
    with (args.out / "gcc-source-notices.json").open("x") as stream:
        json.dump(result, stream, indent=2)
        stream.write("\n")
    print(f"Captured {len(result['scan']['selectedNotices'])} GCC notice candidates; missing regular names: {result['scan']['missingRegularNoticeNames']}; no clearance")


if __name__ == "__main__":
    main()
