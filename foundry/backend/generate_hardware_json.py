#!/usr/bin/env python3
import json
import os
from pathlib import Path

import yaml


REPO_ROOT = Path(__file__).resolve().parents[2]
ONBOARDING_DIR = REPO_ROOT / "core" / "configs" / "onboarding"
OUTPUT_FILE = REPO_ROOT / "foundry" / "backend" / "configs" / "hardware.json"


def load_board_entry(path: Path):
    data = yaml.safe_load(path.read_text(encoding="utf-8")) or {}
    name = data.get("name") or path.stem
    pass_rate = int(data.get("pass_rate", 0) or 0)
    verified = bool(data.get("verified", False))
    # Tier-1 means fully validated and verified in the unified catalog source.
    tier = 1 if (verified and pass_rate >= 100) else 2

    return {
        "id": f"board-{name}",
        "name": name,
        "type": "board",
        # Keep contract shape; onboarding manifests are now the SoT.
        "repl_path": f"core/configs/onboarding/{path.name}",
        "tier": tier,
    }


def main():
    if not ONBOARDING_DIR.exists():
        raise SystemExit(f"onboarding directory not found: {ONBOARDING_DIR}")

    hardware = []
    for path in sorted(ONBOARDING_DIR.glob("*.yaml")):
        hardware.append(load_board_entry(path))

    hardware.sort(key=lambda x: (x["tier"], x["type"], x["name"]))
    OUTPUT_FILE.parent.mkdir(parents=True, exist_ok=True)
    OUTPUT_FILE.write_text(json.dumps(hardware, indent=2) + "\n", encoding="utf-8")
    print(f"Generated {len(hardware)} entries in {OUTPUT_FILE}")


if __name__ == "__main__":
    main()
