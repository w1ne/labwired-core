#!/usr/bin/env python3
"""Best-effort secretless OIDC upload of public CI results to api.labwired.com.

Matches POST /v1/ci/runs contract (JSON metadata + optional base64 tar.gz bundle).
Never fails the job: private repos, missing id-token permission, or transient API
errors log a notice and exit 0.
"""
from __future__ import annotations

import base64
import io
import json
import os
import sys
import tarfile
import urllib.error
import urllib.parse
import urllib.request
from pathlib import Path

MAX_BUNDLE_BYTES = 8 * 1024 * 1024


def _set_output(name: str, value: str) -> None:
    gh_out = os.environ.get("GITHUB_OUTPUT", "")
    if gh_out:
        with open(gh_out, "a", encoding="utf-8") as fh:
            fh.write(f"{name}={value}\n")


def _mint_oidc_token(audience: str) -> str | None:
    req_url = os.environ.get("ACTIONS_ID_TOKEN_REQUEST_URL", "").strip()
    req_token = os.environ.get("ACTIONS_ID_TOKEN_REQUEST_TOKEN", "").strip()
    if not req_url or not req_token:
        return None
    sep = "&" if "?" in req_url else "?"
    url = f"{req_url}{sep}audience={urllib.parse.quote(audience, safe='')}"
    req = urllib.request.Request(
        url,
        headers={
            "Authorization": f"Bearer {req_token}",
            "Accept": "application/json",
            "User-Agent": "w1ne-labwired-core-action/v1",
        },
        method="GET",
    )
    try:
        with urllib.request.urlopen(req, timeout=15) as resp:
            payload = json.loads(resp.read().decode("utf-8"))
            tok = payload.get("value")
            return str(tok) if tok else None
    except Exception as exc:
        print(f"LabWired OIDC token request skipped: {exc}")
        return None


def _build_tar_gz(out_dir: Path) -> bytes:
    if not out_dir.is_dir():
        return b""
    buf = io.BytesIO()
    try:
        with tarfile.open(fileobj=buf, mode="w:gz") as tar:
            tar.add(str(out_dir), arcname="artifacts")
        return buf.getvalue()
    except (OSError, tarfile.TarError) as exc:
        print(f"LabWired bundle packing skipped ({exc}); uploading metadata only.")
        return b""


def main() -> int:
    if os.environ.get("REPO_PRIVATE", "false").strip().lower() == "true":
        print("LabWired: repository is private — skipping public upload.")
        _set_output("report_url", "")
        return 0

    api_url = os.environ.get("API_URL", "https://api.labwired.com").rstrip("/")
    out_dir = Path(os.environ.get("OUTDIR", "out/labwired"))
    verdict = os.environ.get("VERDICT", "error").strip().lower()
    if verdict not in {"pass", "fail", "unproven", "error"}:
        verdict = "error"

    try:
        passed = int(os.environ.get("ASSERTIONS_PASSED", "0") or 0)
        total = int(os.environ.get("ASSERTIONS_TOTAL", "0") or 0)
    except ValueError:
        passed, total = 0, 0
    failed = max(0, total - passed)

    token = _mint_oidc_token(api_url)
    if not token:
        print("LabWired upload skipped (no GitHub OIDC id-token permission; add `permissions: { id-token: write }` for live replay links).")
        _set_output("report_url", "")
        return 0

    payload: dict[str, object] = {
        "status": verdict,
        "tests_passed": passed,
        "tests_failed": failed,
        "cli_version": os.environ.get("CLI_VERSION", "").strip() or None,
        "gallery": True,
    }

    bundle = _build_tar_gz(out_dir)
    if bundle and len(bundle) <= MAX_BUNDLE_BYTES:
        payload["bundle_base64"] = base64.b64encode(bundle).decode("ascii")

    req = urllib.request.Request(
        f"{api_url}/v1/ci/runs",
        data=json.dumps(payload).encode("utf-8"),
        headers={
            "Authorization": f"Bearer {token}",
            "Content-Type": "application/json",
            "User-Agent": "w1ne-labwired-core-action/v1",
        },
        method="POST",
    )
    try:
        with urllib.request.urlopen(req, timeout=25) as resp:
            body = json.loads(resp.read().decode("utf-8"))
            report_url = str(body.get("report_url") or body.get("url") or "")
            _set_output("report_url", report_url)
            if report_url:
                print(f"Uploaded LabWired CI run: {report_url}")
    except urllib.error.HTTPError as exc:
        err_body = ""
        try:
            err_body = exc.read().decode("utf-8", errors="replace")[:300]
        except Exception:
            pass
        print(f"LabWired upload returned HTTP {exc.code} (non-fatal): {err_body}")
        _set_output("report_url", "")
    except Exception as exc:
        print(f"LabWired upload skipped ({exc})")
        _set_output("report_url", "")

    return 0


if __name__ == "__main__":
    sys.exit(main())
