#!/usr/bin/env python3
"""Best-effort secretless OIDC upload of public CI results to api.labwired.com.

Never fails the job: if id-token permission is absent (e.g. fork PR) or the
endpoint is unreachable, prints an informational notice and exits 0.
"""
from __future__ import annotations

import io
import json
import os
import sys
import urllib.error
import urllib.parse
import urllib.request
import zipfile
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


def _zip_out_dir(out_dir: Path) -> bytes:
    buf = io.BytesIO()
    with zipfile.ZipFile(buf, "w", compression=zipfile.ZIP_DEFLATED) as zf:
        if out_dir.is_dir():
            total = 0
            for path in sorted(out_dir.rglob("*")):
                if not path.is_file():
                    continue
                size = path.stat().st_size
                if size > 2 * 1024 * 1024 or total + size > MAX_BUNDLE_BYTES:
                    continue
                zf.write(path, path.relative_to(out_dir).as_posix())
                total += size
    return buf.getvalue()


def main() -> int:
    api_url = os.environ.get("API_URL", "https://api.labwired.com").rstrip("/")
    out_dir = Path(os.environ.get("OUTDIR", "out/labwired"))
    verdict = os.environ.get("VERDICT", "error").strip()
    summary_line = os.environ.get("SUMMARY_LINE", "").strip()

    token = _mint_oidc_token(api_url)
    if not token:
        print("LabWired upload skipped (no GitHub OIDC id-token permission; add `permissions: { id-token: write }` for live replay links).")
        _set_output("report_url", "")
        return 0

    bundle = _zip_out_dir(out_dir)
    upload_url = f"{api_url}/v1/ci/runs"
    req = urllib.request.Request(
        upload_url,
        data=bundle,
        headers={
            "Authorization": f"Bearer {token}",
            "Content-Type": "application/zip",
            "X-LabWired-Verdict": verdict,
            "X-LabWired-Summary": summary_line[:256],
            "User-Agent": "w1ne-labwired-core-action/v1",
        },
        method="POST",
    )
    try:
        with urllib.request.urlopen(req, timeout=20) as resp:
            body = json.loads(resp.read().decode("utf-8"))
            report_url = str(body.get("report_url") or body.get("url") or "")
            _set_output("report_url", report_url)
            if report_url:
                print(f"Uploaded LabWired CI run: {report_url}")
    except urllib.error.HTTPError as exc:
        print(f"LabWired upload returned HTTP {exc.code} (non-fatal)")
        _set_output("report_url", "")
    except Exception as exc:
        print(f"LabWired upload skipped ({exc})")
        _set_output("report_url", "")

    return 0


if __name__ == "__main__":
    sys.exit(main())
