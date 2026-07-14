#!/usr/bin/env python3
"""Render safe, self-contained LabWired GitHub Actions run reports."""

from __future__ import annotations

import argparse
import hashlib
import html
import json
import os
from pathlib import Path


UART_LIMIT_BYTES = 64 * 1024
METRIC_KEYS = ("steps_executed", "cycles", "instructions")


def display(value: object) -> str:
    """Return untrusted report data as a printable string."""

    if value is None:
        return "unknown"
    return str(value)


def markdown_code(value: object) -> str:
    """Keep untrusted values inside a Markdown code span they cannot close."""

    text = display(value).replace("\r", " ").replace("\n", " ")
    longest_run = 0
    current_run = 0
    for character in text:
        if character == "`":
            current_run += 1
            longest_run = max(longest_run, current_run)
        else:
            current_run = 0
    delimiter = "`" * (longest_run + 1)
    return f"{delimiter} {text} {delimiter}"


def escaped(value: object) -> str:
    return html.escape(display(value), quote=True)


def load_result(path: Path) -> dict:
    try:
        data = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, UnicodeDecodeError, json.JSONDecodeError):
        return {}
    return data if isinstance(data, dict) else {}


def read_uart(path: Path) -> str:
    try:
        with path.open("rb") as uart_log:
            contents = uart_log.read(UART_LIMIT_BYTES + 1)
    except OSError:
        return "[UART transcript unavailable]"

    truncated = len(contents) > UART_LIMIT_BYTES
    transcript = contents[:UART_LIMIT_BYTES].decode("utf-8", errors="replace")
    if truncated:
        transcript += "\n[UART transcript truncated after 65536 bytes]"
    return transcript


def script_sha256(path_text: str) -> str:
    try:
        contents = Path(path_text).read_bytes()
    except OSError:
        return "unavailable"
    return hashlib.sha256(contents).hexdigest()


def assertion_description(assertion: dict) -> str:
    value = assertion.get("assertion")
    if value is None:
        value = assertion.get("kind", assertion)
    try:
        return json.dumps(value, ensure_ascii=False, sort_keys=True)
    except (TypeError, ValueError):
        return display(value)


def assertion_outcome(assertion: dict) -> str:
    if assertion.get("passed") is True:
        return "passed"
    if assertion.get("passed") is False:
        return "failed"
    return "unknown"


def write_github_output(output, key: str, value: object) -> None:
    """Write one GitHub output without allowing a value to add another key."""

    text = display(value).replace("\r", "\n")
    if "\n" not in text:
        output.write(f"{key}={text}\n")
        return

    delimiter = "LABWIRED_OUTPUT_EOF"
    while delimiter in text:
        delimiter += "_"
    output.write(f"{key}<<{delimiter}\n{text}\n{delimiter}\n")


def render_summary(
    status: str,
    stop_reason: str,
    metrics: dict[str, str],
    passed: int,
    failed: int,
    run_url: str,
    source_revision: str,
    release_version: str,
    script: str,
    digest: str,
    result_json: Path,
    uart_log: Path,
    summary_md: Path,
    report_html: Path,
) -> str:
    metric_lines = "\n".join(
        f"- {name.replace('_', ' ').title()}: {markdown_code(value)}"
        for name, value in metrics.items()
    )
    artifact_lines = "\n".join(
        f"- {markdown_code(path.name)}" for path in (result_json, uart_log, summary_md, report_html)
    )
    return f"""## LabWired test — {markdown_code(status)}

Verdict: {markdown_code(status)}

Stop reason: {markdown_code(stop_reason)}

### Metrics

{metric_lines}

### Assertions

Assertions: `{passed}` passed, `{failed}` failed

### Provenance

- Run: {markdown_code(run_url)}
- Source revision: {markdown_code(source_revision)}
- LabWired release: {markdown_code(release_version)}
- Script: {markdown_code(script)}
- Script sha256: {markdown_code(digest)}

### Artifacts

{artifact_lines}
"""


def render_html(
    status: str,
    stop_reason: str,
    metrics: dict[str, str],
    assertions: list[dict],
    passed: int,
    failed: int,
    run_url: str,
    source_revision: str,
    release_version: str,
    script: str,
    digest: str,
    uart: str,
) -> str:
    badge_class = {"pass": "pass", "fail": "fail", "error": "error"}.get(
        status.lower(), "unknown"
    )
    metric_cards = "\n".join(
        f"<article class=\"metric\"><h3>{escaped(name.replace('_', ' ').title())}</h3>"
        f"<p>{escaped(value)}</p></article>"
        for name, value in metrics.items()
    )
    assertion_rows = "\n".join(
        "<tr>"
        f"<td>{index}</td>"
        f"<td class=\"{assertion_outcome(assertion)}\">{escaped(assertion_outcome(assertion))}</td>"
        f"<td><code>{escaped(assertion_description(assertion))}</code></td>"
        "</tr>"
        for index, assertion in enumerate(assertions, start=1)
    )
    if not assertion_rows:
        assertion_rows = "<tr><td colspan=\"3\">No assertions were recorded.</td></tr>"

    return f"""<!doctype html>
<html lang=\"en\">
<head>
  <meta charset=\"utf-8\">
  <meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">
  <title>LabWired test report</title>
  <style>
    :root {{ color-scheme: light dark; font-family: system-ui, sans-serif; }}
    body {{ margin: 2rem auto; max-width: 72rem; padding: 0 1rem; line-height: 1.5; }}
    .badge {{ border-radius: 999px; color: white; display: inline-block; font-weight: 700; padding: .25rem .7rem; text-transform: uppercase; }}
    .pass {{ background: #087f23; }} .fail {{ background: #b42318; }} .error {{ background: #8f1d78; }} .unknown {{ background: #5c5c5c; }}
    .metrics {{ display: grid; gap: 1rem; grid-template-columns: repeat(auto-fit, minmax(11rem, 1fr)); }}
    .metric {{ border: 1px solid #8886; border-radius: .5rem; padding: .8rem; }}
    .metric h3, .metric p {{ margin: 0; }} .metric p {{ font-size: 1.35rem; font-weight: 700; }}
    table {{ border-collapse: collapse; width: 100%; }} th, td {{ border: 1px solid #8886; padding: .55rem; text-align: left; vertical-align: top; }}
    td.passed {{ color: #087f23; font-weight: 700; }} td.failed {{ color: #b42318; font-weight: 700; }} td.unknown {{ font-weight: 700; }}
    dt {{ font-weight: 700; }} dd {{ margin: 0 0 .8rem; overflow-wrap: anywhere; }}
    pre {{ background: #111; border-radius: .5rem; color: #f5f5f5; overflow-x: auto; padding: 1rem; white-space: pre-wrap; }}
  </style>
</head>
<body>
  <header>
    <h1>LabWired test report</h1>
    <p><span class=\"badge {badge_class}\">{escaped(status)}</span></p>
    <p>Stop reason: <code>{escaped(stop_reason)}</code></p>
  </header>
  <section aria-labelledby=\"metrics\">
    <h2 id=\"metrics\">Metrics</h2>
    <div class=\"metrics\">{metric_cards}</div>
  </section>
  <section aria-labelledby=\"assertions\">
    <h2 id=\"assertions\">Assertions</h2>
    <p>{passed} passed, {failed} failed</p>
    <table>
      <thead><tr><th>#</th><th>Outcome</th><th>Assertion</th></tr></thead>
      <tbody>{assertion_rows}</tbody>
    </table>
  </section>
  <section aria-labelledby=\"provenance\">
    <h2 id=\"provenance\">Provenance</h2>
    <dl>
      <dt>Run URL</dt><dd><code>{escaped(run_url)}</code></dd>
      <dt>Source revision</dt><dd><code>{escaped(source_revision)}</code></dd>
      <dt>LabWired release</dt><dd><code>{escaped(release_version)}</code></dd>
      <dt>Script</dt><dd><code>{escaped(script)}</code></dd>
      <dt>Script sha256</dt><dd><code>{escaped(digest)}</code></dd>
    </dl>
  </section>
  <section aria-labelledby=\"uart\">
    <h2 id=\"uart\">UART transcript</h2>
    <pre>{escaped(uart)}</pre>
  </section>
</body>
</html>
"""


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("result_json")
    parser.add_argument("uart_log")
    parser.add_argument("summary_md")
    parser.add_argument("report_html")
    parser.add_argument("github_output")
    args = parser.parse_args()

    result_json = Path(args.result_json)
    uart_log = Path(args.uart_log)
    summary_md = Path(args.summary_md)
    report_html = Path(args.report_html)
    github_output = Path(args.github_output)

    data = load_result(result_json)
    status = display(data.get("status", "unknown"))
    stop_reason = display(data.get("stop_reason", "unknown"))
    raw_assertions = data.get("assertions", [])
    assertions = (
        [assertion for assertion in raw_assertions if isinstance(assertion, dict)]
        if isinstance(raw_assertions, list)
        else []
    )
    passed = sum(assertion.get("passed") is True for assertion in assertions)
    failed = sum(assertion.get("passed") is False for assertion in assertions)
    metrics = {key: display(data.get(key, "unknown")) for key in METRIC_KEYS}
    run_url = os.environ.get("LABWIRED_RUN_URL") or "unavailable"
    source_revision = os.environ.get("LABWIRED_SOURCE_REVISION") or "unavailable"
    release_version = os.environ.get("LABWIRED_RELEASE_VERSION") or "unavailable"
    script = os.environ.get("LABWIRED_SCRIPT", "")
    digest = script_sha256(script)

    summary_md.parent.mkdir(parents=True, exist_ok=True)
    report_html.parent.mkdir(parents=True, exist_ok=True)
    summary_md.write_text(
        render_summary(
            status,
            stop_reason,
            metrics,
            passed,
            failed,
            run_url,
            source_revision,
            release_version,
            script,
            digest,
            result_json,
            uart_log,
            summary_md,
            report_html,
        ),
        encoding="utf-8",
    )
    report_html.write_text(
        render_html(
            status,
            stop_reason,
            metrics,
            assertions,
            passed,
            failed,
            run_url,
            source_revision,
            release_version,
            script,
            digest,
            read_uart(uart_log),
        ),
        encoding="utf-8",
    )

    github_output.parent.mkdir(parents=True, exist_ok=True)
    with github_output.open("a", encoding="utf-8") as output:
        write_github_output(output, "status", status)
        write_github_output(output, "summary_md", summary_md)
        write_github_output(output, "report_html", report_html)


if __name__ == "__main__":
    main()
