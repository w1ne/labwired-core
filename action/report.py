#!/usr/bin/env python3
"""Render GitHub Step Summary and optional sticky PR comment for w1ne/labwired-core."""
from __future__ import annotations

import json
import os
import re
import sys
import urllib.error
import urllib.request
from pathlib import Path

MAX_UART_CHARS = 4000


def _comment_marker() -> str:
    tag = re.sub(r"[^A-Za-z0-9_.-]+", "-", os.environ.get("COMMENT_TAG", "").strip()).strip("-")
    return f"<!-- labwired-verify:v1:{tag} -->" if tag else "<!-- labwired-verify:v1 -->"


def _read_json(path: Path) -> dict | None:
    if not path.is_file():
        return None
    try:
        data = json.loads(path.read_text(encoding="utf-8"))
        return data if isinstance(data, dict) else None
    except (OSError, json.JSONDecodeError):
        return None


def _safe_fence(text: str) -> str:
    longest = max((len(m.group(0)) for m in re.finditer(r"`+", text)), default=0)
    fence = "`" * max(4, longest + 1)
    return f"{fence}text\n{text.rstrip()}\n{fence}"


def _set_output(name: str, value: str) -> None:
    gh_out = os.environ.get("GITHUB_OUTPUT", "")
    if gh_out:
        with open(gh_out, "a", encoding="utf-8") as fh:
            fh.write(f"{name}={value}\n")


def _pr_number() -> int | None:
    event_path = os.environ.get("GITHUB_EVENT_PATH", "")
    if not event_path or not os.path.isfile(event_path):
        return None
    try:
        ev = json.loads(Path(event_path).read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError):
        return None
    pr = ev.get("pull_request")
    if isinstance(pr, dict) and isinstance(pr.get("number"), int):
        return pr["number"]
    return None


def _gh_api(method: str, url: str, token: str, payload: dict | None = None) -> tuple[int, dict | list | None]:
    data = json.dumps(payload).encode("utf-8") if payload is not None else None
    req = urllib.request.Request(
        url,
        data=data,
        headers={
            "Authorization": f"Bearer {token}",
            "Accept": "application/vnd.github+json",
            "X-GitHub-Api-Version": "2022-11-28",
            "Content-Type": "application/json",
            "User-Agent": "w1ne-labwired-core-action/v1",
        },
        method=method,
    )
    try:
        with urllib.request.urlopen(req, timeout=15) as resp:
            raw = resp.read().decode("utf-8", errors="replace")
            return resp.status, (json.loads(raw) if raw else None)
    except urllib.error.HTTPError as exc:
        return exc.code, None
    except Exception:
        return 0, None


def _upsert_pr_comment(body: str, marker: str) -> None:
    if os.environ.get("PR_COMMENT", "true").lower() != "true":
        return
    token = os.environ.get("GITHUB_TOKEN", "").strip()
    repo = os.environ.get("GITHUB_REPOSITORY", "").strip()
    pr_num = _pr_number()
    if not token or not repo or pr_num is None:
        return

    api_base = os.environ.get("GITHUB_API_URL", "https://api.github.com").rstrip("/")
    list_url = f"{api_base}/repos/{repo}/issues/{pr_num}/comments?per_page=100"
    status, comments = _gh_api("GET", list_url, token)
    existing_id = None
    if status == 200 and isinstance(comments, list):
        for c in comments:
            if isinstance(c, dict) and marker in str(c.get("body", "")):
                existing_id = c.get("id")
                break

    if existing_id is not None:
        code, _ = _gh_api("PATCH", f"{api_base}/repos/{repo}/issues/comments/{existing_id}", token, {"body": body})
        print(f"Updated LabWired PR comment #{existing_id} (HTTP {code})")
    else:
        code, _ = _gh_api("POST", f"{api_base}/repos/{repo}/issues/{pr_num}/comments", token, {"body": body})
        print(f"Posted LabWired PR comment on #{pr_num} (HTTP {code})")


def _format_assertion(item: dict) -> str:
    if isinstance(item.get("assertion"), dict) and item["assertion"]:
        parts = [f"`{k}` = `{v}`" for k, v in item["assertion"].items()]
        base = ", ".join(parts)
    else:
        base = str(item.get("name") or item.get("type") or item.get("kind") or "assertion")
    detail = str(item.get("message") or item.get("detail") or "").strip()
    label = f"{base} — {detail}" if detail else base
    return label.replace("|", "\\|")


def main() -> int:
    out_dir = Path(os.environ.get("OUTDIR", "out/labwired"))
    verdict = os.environ.get("VERDICT", "error").strip().lower()
    summary_line = os.environ.get("SUMMARY_LINE", "").strip()
    report_url = os.environ.get("REPORT_URL", "").strip()
    api_url = os.environ.get("API_URL", "https://api.labwired.com").rstrip("/")
    repo = os.environ.get("GITHUB_REPOSITORY", "").strip()
    sha = os.environ.get("GITHUB_SHA", "").strip()
    tag = os.environ.get("COMMENT_TAG", "").strip()
    marker = _comment_marker()

    badge_url = f"{api_url}/v1/ci/badge/{repo}.svg" if repo else ""
    repo_ci_url = f"https://app.labwired.com/ci/repo/{repo}" if repo else "https://app.labwired.com/ci"
    playground_url = report_url or (f"https://app.labwired.com/ci/run/{repo}/{sha}" if repo and sha else "https://app.labwired.com")

    _set_output("badge_url", badge_url)
    _set_output("playground_url", playground_url)

    circuit = _read_json(out_dir / "circuit-report.json") or {}
    sim = _read_json(out_dir / "result.json") or {}

    icon = {"pass": "✅", "fail": "❌", "unproven": "⚠️", "error": "💥"}.get(verdict, "💥")
    verdict_label = {
        "pass": "Verified by LabWired",
        "fail": "Verification Failed",
        "unproven": "Unproven",
        "error": "Verification Error",
    }.get(verdict, verdict)
    heading = f"## {icon} {verdict_label}" + (f" · `{tag}`" if tag else "")

    lines: list[str] = [
        marker,
        heading,
        "",
        f"**`{summary_line}`**",
        "",
    ]

    if badge_url:
        lines.append(f"[![Verified by LabWired]({badge_url})]({repo_ci_url})")
        lines.append("")

    links: list[str] = []
    if report_url:
        links.append(f"[Open live LabWired replay]({report_url})")
    elif repo:
        links.append(f"[Open LabWired CI page]({repo_ci_url})")
    links.append("[Open LabWired Playground](https://app.labwired.com)")
    links.append("[`uses: w1ne/labwired-core@v1`](https://github.com/w1ne/labwired-core)")
    lines.append(" · ".join(links))
    lines.append("")

    if circuit.get("checked"):
        src_fmt = circuit.get("source_format") or circuit.get("source_kind") or "circuit"
        board = circuit.get("board") or "—"
        mapped = circuit.get("mapped_parts", 0)
        dropped = circuit.get("dropped_parts") or []
        erc = circuit.get("erc") or {}
        drc = circuit.get("drc") or {}
        erc_str = f"{erc.get('errors', 0)} errors, {erc.get('warnings', 0)} warnings" if erc.get("checked") else "not run"
        drc_str = f"{drc.get('errors', 0)} errors, {drc.get('warnings', 0)} warnings" if drc.get("checked") else "not applicable"

        lines.extend([
            "### Hardware & Import Check",
            "",
            "| Check | Result |",
            "| --- | --- |",
            f"| Source format | `{src_fmt}` ({', '.join(f'`{f}`' for f in (circuit.get('source_files') or []))}) |",
            f"| Target twin board | `{board}` ({mapped} parts mapped, {len(dropped)} dropped) |",
            f"| ERC | {erc_str} |",
            f"| PCB DRC | {drc_str} |",
            "",
        ])

        erc_diags = erc.get("diagnostics") or []
        if erc_diags:
            lines.append("#### ERC Diagnostics")
            for d in erc_diags[:15]:
                lines.append(f"- **{d.get('severity', 'info')}** (`{d.get('code', '')}`): {d.get('message', '')}")
            lines.append("")

        drc_viols = drc.get("violations") or []
        if drc_viols:
            lines.append("#### PCB DRC Violations")
            for v in drc_viols[:15]:
                nets = ", ".join(v.get("nets") or [])
                suffix = f" (nets: {nets})" if nets else ""
                lines.append(f"- **{v.get('kind', 'drc')}**: {v.get('detail', '')}{suffix}")
            lines.append("")

        questions = circuit.get("questions") or []
        if questions:
            lines.append("#### Questions on Unmapped Footprints / Parts")
            for q in questions[:15]:
                lines.append(f"- {q}")
            lines.append("")

    assertions = sim.get("assertions")
    if isinstance(assertions, list) and assertions:
        lines.extend([
            "### Firmware Assertions",
            "",
            "| Status | Assertion |",
            "| --- | --- |",
        ])
        for item in assertions:
            if not isinstance(item, dict):
                continue
            ok = item.get("passed") is True
            lines.append(f"| {'✅ Pass' if ok else '❌ Fail'} | {_format_assertion(item)} |")
        lines.append("")

    uart_path = out_dir / "uart.log"
    if uart_path.is_file():
        uart_text = uart_path.read_text(encoding="utf-8", errors="replace").strip()
        if uart_text:
            tail = uart_text[-MAX_UART_CHARS:]
            lines.extend([
                "<details><summary>UART output</summary>",
                "",
                _safe_fence(tail),
                "",
                "</details>",
                "",
            ])

    if badge_url:
        snippet = f"[![Verified by LabWired]({badge_url})]({repo_ci_url})"
        lines.extend([
            "<details><summary>Add the Verified by LabWired badge to your README</summary>",
            "",
            "```markdown",
            snippet,
            "```",
            "",
            "</details>",
            "",
        ])

    markdown = "\n".join(lines).rstrip() + "\n"

    summary_path = os.environ.get("GITHUB_STEP_SUMMARY", "")
    if summary_path:
        with open(summary_path, "a", encoding="utf-8") as fh:
            fh.write(markdown)

    _upsert_pr_comment(markdown, marker)
    return 0


if __name__ == "__main__":
    sys.exit(main())
