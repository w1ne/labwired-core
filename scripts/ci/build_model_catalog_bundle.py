#!/usr/bin/env python3
"""Build the public configs archive and its catalog manifest."""

import argparse
import hashlib
import json
import tarfile
from pathlib import Path
from typing import Sequence


def build_bundle(
    configs_dir: Path,
    chips: list[str],
    core_commit: str,
    output_dir: Path,
) -> None:
    """Write configs.tar.gz and catalog.json into output_dir."""
    output_dir.mkdir(parents=True, exist_ok=True)
    archive_path = output_dir / "configs.tar.gz"
    with tarfile.open(archive_path, "w:gz") as archive:
        archive.add(configs_dir, arcname="configs")

    manifest = {
        "schema_version": 1,
        "core_commit": core_commit,
        "archive": archive_path.name,
        "sha256": hashlib.sha256(archive_path.read_bytes()).hexdigest(),
        "chips": sorted(chips),
    }
    (output_dir / "catalog.json").write_text(
        json.dumps(manifest, indent=2) + "\n", encoding="utf-8"
    )


def main(argv: Sequence[str] | None = None) -> int:
    repo_root = Path(__file__).resolve().parents[2]
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--configs-dir", type=Path, default=repo_root / "configs")
    parser.add_argument("--chips-json", required=True, help="JSON array of chip names")
    parser.add_argument("--core-commit", required=True)
    parser.add_argument("--output-dir", required=True, type=Path)
    args = parser.parse_args(argv)

    chips = json.loads(args.chips_json)
    if not isinstance(chips, list) or not all(isinstance(chip, str) for chip in chips):
        parser.error("--chips-json must be a JSON array of strings")
    build_bundle(args.configs_dir, chips, args.core_commit, args.output_dir)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
