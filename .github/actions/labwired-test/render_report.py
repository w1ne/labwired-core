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
RESULT_JSON_LIMIT_BYTES = 1024 * 1024
ASSERTION_RENDER_LIMIT = 200
DISPLAY_VALUE_LIMIT = 4096
SUMMARY_LIMIT_BYTES = 64 * 1024
SCRIPT_HASH_CHUNK_BYTES = 64 * 1024
METRIC_KEYS = ("steps_executed", "cycles", "instructions")
VALUE_TRUNCATION_MARKER = "… [truncated]"


def bounded_display(value: object, limit: int = DISPLAY_VALUE_LIMIT) -> tuple[str, bool]:
    """Return bounded printable report data and whether it was truncated."""

    if value is None:
        text = "unknown"
    else:
        try:
            text = str(value)
        except Exception:  # JSON values are plain, but keep diagnostics fail-safe.
            text = "unavailable"

    if len(text) <= limit:
        return text, False
    return f"{text[: limit - len(VALUE_TRUNCATION_MARKER)]}{VALUE_TRUNCATION_MARKER}", True


def display(value: object) -> str:
    """Return untrusted report data as a bounded printable string."""

    return bounded_display(value)[0]


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


def load_result(path: Path) -> tuple[dict, list[str]]:
    try:
        with path.open("rb") as result_file:
            contents = result_file.read(RESULT_JSON_LIMIT_BYTES + 1)
    except OSError:
        return {}, ["result.json is unavailable; report values are unavailable"]

    if len(contents) > RESULT_JSON_LIMIT_BYTES:
        return {}, [
            "result.json exceeded the 1048576-byte rendering limit; "
            "report values are unavailable"
        ]

    try:
        data = json.loads(contents.decode("utf-8"))
    except (UnicodeDecodeError, json.JSONDecodeError, RecursionError):
        return {}, ["result.json is malformed or unreadable; report values are unavailable"]
    if not isinstance(data, dict):
        return {}, ["result.json is not an object; report values are unavailable"]
    return data, []


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
    digest = hashlib.sha256()
    try:
        with Path(path_text).open("rb") as script_file:
            while chunk := script_file.read(SCRIPT_HASH_CHUNK_BYTES):
                digest.update(chunk)
    except OSError:
        return "unavailable"
    return digest.hexdigest()


def assertion_description(assertion: dict) -> tuple[str, bool]:
    value = assertion.get("assertion")
    if value is None:
        value = assertion.get("kind", assertion)
    try:
        description = json.dumps(value, ensure_ascii=False, sort_keys=True)
    except (TypeError, ValueError, RecursionError):
        description = display(value)
    return bounded_display(description)


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


def cap_markdown_summary(summary: str) -> str:
    """Keep the job-summary append below GitHub's per-step size limits."""

    encoded_summary = summary.encode("utf-8")
    if len(encoded_summary) <= SUMMARY_LIMIT_BYTES:
        return summary

    notice = (
        "\n\n> Report summary truncated after 65536 bytes. "
        "See the HTML report artifact for the bounded report.\n"
    )
    prefix_limit = SUMMARY_LIMIT_BYTES - len(notice.encode("utf-8"))
    prefix = encoded_summary[:prefix_limit].decode("utf-8", errors="ignore").rstrip()
    return f"{prefix}{notice}"


def render_summary(
    status: str,
    stop_reason: str,
    metrics: dict[str, str],
    passed: int,
    failed: int,
    message: str | None,
    diagnostics: list[str],
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
    diagnostic_lines: list[str] = []
    if message is not None:
        diagnostic_lines.append(f"- Message: {markdown_code(message)}")
    diagnostic_lines.extend(f"- {diagnostic}" for diagnostic in diagnostics)
    diagnostics_section = ""
    if diagnostic_lines:
        diagnostics_section = f"### Diagnostics\n\n{'\n'.join(diagnostic_lines)}\n\n"

    summary = f"""## LabWired test — {markdown_code(status)}

Verdict: {markdown_code(status)}

Stop reason: {markdown_code(stop_reason)}

{diagnostics_section}### Metrics

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
    return cap_markdown_summary(summary)


def render_html(
    status: str,
    stop_reason: str,
    metrics: dict[str, str],
    assertions: list[tuple[dict, str]],
    passed: int,
    failed: int,
    message: str | None,
    diagnostics: list[str],
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
        f"<td><code>{escaped(description)}</code></td>"
        "</tr>"
        for index, (assertion, description) in enumerate(assertions, start=1)
    )
    if not assertion_rows:
        assertion_rows = "<tr><td colspan=\"3\">No assertions were recorded.</td></tr>"

    message_html = ""
    if message is not None:
        message_html = f"<dl><dt>Message</dt><dd><code>{escaped(message)}</code></dd></dl>"
    notice_html = ""
    if diagnostics:
        notice_html = "<ul>" + "".join(
            f"<li>{escaped(diagnostic)}</li>" for diagnostic in diagnostics
        ) + "</ul>"
    diagnostics_section = ""
    if message_html or notice_html:
        diagnostics_section = f"""  <section aria-labelledby=\"diagnostics\">
    <h2 id=\"diagnostics\">Diagnostics</h2>
    {message_html}{notice_html}
  </section>
"""

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
{diagnostics_section}  <section aria-labelledby=\"metrics\">
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
    <pre>{html.escape(uart, quote=True)}</pre>
  </section>
</body>
</html>
"""


def assertion_diagnostic(total: int) -> str | None:
    if total <= ASSERTION_RENDER_LIMIT:
        return None
    omitted = total - ASSERTION_RENDER_LIMIT
    noun = "assertion" if omitted == 1 else "assertions"
    verb = "was" if omitted == 1 else "were"
    return (
        f"Only the first {ASSERTION_RENDER_LIMIT} assertions are shown; "
        f"{omitted} additional {noun} {verb} omitted."
    )


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

    data, diagnostics = load_result(result_json)
    status, status_truncated = bounded_display(data.get("status", "unknown"))
    stop_reason, stop_reason_truncated = bounded_display(data.get("stop_reason", "unknown"))
    raw_assertions = data.get("assertions", [])
    assertion_entries = (
        raw_assertions[:ASSERTION_RENDER_LIMIT] if isinstance(raw_assertions, list) else []
    )
    assertions: list[tuple[dict, str]] = []
    assertion_description_truncated = False
    for assertion in assertion_entries:
        if not isinstance(assertion, dict):
            continue
        description, description_truncated = assertion_description(assertion)
        assertions.append((assertion, description))
        assertion_description_truncated = assertion_description_truncated or description_truncated
    passed = 0
    failed = 0
    if isinstance(raw_assertions, list):
        for assertion in raw_assertions:
            if not isinstance(assertion, dict):
                continue
            if assertion.get("passed") is True:
                passed += 1
            elif assertion.get("passed") is False:
                failed += 1
    assertion_count = len(raw_assertions) if isinstance(raw_assertions, list) else 0
    if assertion_notice := assertion_diagnostic(assertion_count):
        diagnostics.append(assertion_notice)

    metric_values: dict[str, str] = {}
    metric_truncated = False
    for key in METRIC_KEYS:
        metric_values[key], truncated = bounded_display(data.get(key, "unknown"))
        metric_truncated = metric_truncated or truncated

    message: str | None = None
    message_truncated = False
    if "message" in data:
        message, message_truncated = bounded_display(data["message"])

    raw_run_url = os.environ.get("LABWIRED_RUN_URL") or "unavailable"
    raw_source_revision = os.environ.get("LABWIRED_SOURCE_REVISION") or "unavailable"
    raw_release_version = os.environ.get("LABWIRED_RELEASE_VERSION") or "unavailable"
    raw_script = os.environ.get("LABWIRED_SCRIPT", "")
    run_url, run_url_truncated = bounded_display(raw_run_url)
    source_revision, source_revision_truncated = bounded_display(raw_source_revision)
    release_version, release_version_truncated = bounded_display(raw_release_version)
    script, script_truncated = bounded_display(raw_script)
    if any(
        (
            status_truncated,
            stop_reason_truncated,
            metric_truncated,
            message_truncated,
            run_url_truncated,
            source_revision_truncated,
            release_version_truncated,
            script_truncated,
            assertion_description_truncated,
        )
    ):
        diagnostics.append(f"Report values are capped at {DISPLAY_VALUE_LIMIT} characters.")
    digest = script_sha256(raw_script)

    summary_md.parent.mkdir(parents=True, exist_ok=True)
    report_html.parent.mkdir(parents=True, exist_ok=True)
    summary_md.write_text(
        render_summary(
            status,
            stop_reason,
            metric_values,
            passed,
            failed,
            message,
            diagnostics,
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
            metric_values,
            assertions,
            passed,
            failed,
            message,
            diagnostics,
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
