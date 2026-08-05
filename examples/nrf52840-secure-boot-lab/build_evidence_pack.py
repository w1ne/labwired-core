#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Build cra-evidence-pack/ from a labwired-cli test output directory."""

from __future__ import annotations

import argparse
import json
import shutil
from pathlib import Path


README = """# CRA-style evidence pack (LabWired virtual run)

This directory is a **CI evidence pack** from the nRF52840 secure-boot lab.

## What it is

- A **repeatable, cryptographic** demonstration of secure boot, signed OTA,
  anti-rollback, and SE attestation on a **simulated** nRF52840 + ATECC608A.
- Claim rows map the demo narrative to CRA Annex I–style themes.
- `run-manifest.json` carries a **signable SHA-256 digest** of inputs + results.

## What it is not

- Not a Notified Body certificate or full CRA technical documentation.
- Not silicon / HIL evidence (see `limitations.md`).
- The OEM **private** signing key for this run was **ephemeral** and is **not**
  included. Only `oem-verify-pubkey.hex` (public) is retained.

Re-run locally:

```bash
./run_evidence_ci.sh
```
"""

LIMITATIONS = """# Honest limitations

- On-chip and SE RNGs in the simulator are **deterministic PRNGs** (reproducible
  CI). They say nothing about entropy quality on real silicon.
- `APPROTECT` is **stored**, not enforced — debug-port lockout side effects are
  not modelled.
- Flash follows 1→0 / erase semantics but not real erase/program timing.
- The SE implements authentic ATECC608A command *shape* with real ECDSA (p256
  crate), not the full datasheet (no wake/idle timing, demo device key).
- This pack does not cover SBOM, vulnerability handling, support period, or
  other CRA process obligations outside the secure-update / RoT demo.
"""


def load_result(out_dir: Path) -> dict:
    p = out_dir / "result.json"
    if not p.is_file():
        return {}
    return json.loads(p.read_text())


def uart_text(out_dir: Path) -> str:
    p = out_dir / "uart.log"
    if p.is_file():
        return p.read_text(errors="replace")
    # Some runs embed uart in result.json
    result = load_result(out_dir)
    if isinstance(result.get("uart"), str):
        return result["uart"]
    if isinstance(result.get("uart_log"), str):
        return result["uart_log"]
    return ""


def overall_status(result: dict) -> str:
    st = (result.get("status") or result.get("result") or "").lower()
    if st in ("pass", "passed", "ok", "success"):
        return "pass"
    if st in ("fail", "failed", "error"):
        return "fail"
    # fallback: assertions list
    assertions = result.get("assertions") or []
    if assertions and all(a.get("passed", a.get("pass", False)) for a in assertions):
        return "pass"
    if assertions:
        return "fail"
    return "unknown"


def eval_evidence(item: dict, *, out_dir: Path, uart: str, result: dict) -> bool:
    kind = item.get("kind")
    if kind == "uart_contains":
        return item["value"] in uart
    if kind == "file_present":
        return (out_dir / item["path"]).is_file()
    if kind == "manifest_digest_nonempty" or "manifest_digest_nonempty" in item:
        man = out_dir / "run-manifest.json"
        if not man.is_file():
            return False
        data = json.loads(man.read_text())
        dig = data.get("digest") or ""
        return len(dig) >= 32
    if kind == "result_status_pass":
        return overall_status(result) == "pass"
    return False


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--out-dir", type=Path, required=True, help="labwired-cli --output-dir")
    ap.add_argument("--pack-dir", type=Path, required=True)
    ap.add_argument("--claims-map", type=Path, required=True)
    ap.add_argument("--pubkey-hex", type=Path, required=True)
    args = ap.parse_args()

    claims_doc = json.loads(args.claims_map.read_text())
    result = load_result(args.out_dir)
    uart = uart_text(args.out_dir)
    run_ok = overall_status(result) == "pass"

    pack = args.pack_dir
    pack.mkdir(parents=True, exist_ok=True)

    for name in ("result.json", "uart.log", "junit.xml", "run-manifest.json"):
        src = args.out_dir / name
        if src.is_file():
            shutil.copy2(src, pack / name)

    pubkey = args.pubkey_hex.read_text().strip()
    (pack / "oem-verify-pubkey.hex").write_text(pubkey + "\n")
    (pack / "README.md").write_text(README)
    (pack / "limitations.md").write_text(LIMITATIONS)

    evaluated = []
    any_fail = False
    for claim in claims_doc.get("claims", []):
        norm_ev = claim.get("evidence") or []
        flags = [eval_evidence(e, out_dir=args.out_dir, uart=uart, result=result) for e in norm_ev]
        ok = all(flags) if flags else False
        # If the whole run failed, force fail on claims that need uart success
        if not run_ok and claim["id"] != "ci_reproducible_manifest":
            # still evaluate files, but uart claims fail if run failed
            if any(e.get("kind") == "uart_contains" for e in norm_ev):
                ok = False
        status = "pass" if ok else "fail"
        if status == "fail":
            any_fail = True
        evaluated.append(
            {
                "id": claim["id"],
                "title": claim.get("title"),
                "cra_annex_i_ref": claim.get("cra_annex_i_ref"),
                "status": status,
                "notes": claim.get("notes"),
                "evidence": norm_ev,
            }
        )

    pack_status = "fail" if any_fail or not run_ok else "pass"
    claims_out = {
        "schema_version": claims_doc.get("schema_version", "1.0"),
        "pack_status": pack_status,
        "run_status": overall_status(result),
        "claims": evaluated,
    }
    (pack / "claims.json").write_text(json.dumps(claims_out, indent=2) + "\n")

    md = ["# Claims", "", f"**Pack status:** `{pack_status}`", "", "| id | status | title |", "|----|--------|-------|"]
    for c in evaluated:
        md.append(f"| `{c['id']}` | **{c['status']}** | {c['title']} |")
    md.append("")
    (pack / "claims.md").write_text("\n".join(md))

    print(f"wrote pack to {pack} status={pack_status}")
    return 0 if pack_status == "pass" else 1


if __name__ == "__main__":
    raise SystemExit(main())
