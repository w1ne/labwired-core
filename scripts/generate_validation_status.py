#!/usr/bin/env python3
"""
Generate docs/boards/VALIDATION_STATUS.md from validation/manifest.yaml.

The report is NOT committed (it is git-ignored). Its "Models" column hashes every
watched source file, so almost any model edit changed it and any two PRs touching
models conflicted in it. It is rendered where it is read: core-ci uploads it as a
workflow artifact and job summary, and deploy-docs builds it into the docs site
(https://docs.labwired.com/boards/VALIDATION_STATUS/).

The generated report records historical silicon evidence and whether the
current watched source matches the content recorded with that capture.
Source-only drift is informational: no human source-review acknowledgement,
renewal, or expiry is required to commit, push, or pass CI.

Modes
-----
  (default)      render docs/boards/VALIDATION_STATUS.md (git-ignored) in place
  --check        run every integrity gate (full history, manifest/capture
                 validity, duplicate keys, watch-list coverage, report renders,
                 report is not committed); does not compare against a file
  --output PATH  also write the rendered report to PATH (with --check/--drift)
  --drift        report source/capture differences without failing on drift
  --digests      print current, captured, and historical ACK content digests

Both --check and --drift enforce watch-list integrity. Every peripheral path
referenced by a board's chip YAML must be covered by its models list, and every
listed model path must exist. A missing watch can conceal a source change.

Capture provenance
------------------
A source match requires a recorded silicon.models_digest equal to the current
content digest. A missing captured digest leaves the match unverified. Commit
dates and legacy drift_ack fields never establish current hardware validation.
A changed or unverified source match requires a physical re-capture before
claiming current silicon validation, but does not block source integration.

Historical drift_ack fields remain readable metadata. The optional legacy
--write-ack-digests flags update only that metadata when explicitly requested;
they never change a capture or a validation verdict.

Needs PyYAML and a full-history checkout (fetch-depth: 0). The existing history,
watch coverage, duplicate-key, and capture-metadata checks remain enforced.
"""

from __future__ import annotations

import argparse
import posixpath
import re
import subprocess
import sys
from datetime import date, datetime
from pathlib import Path

try:
    import yaml
except ImportError:
    print("ERROR: PyYAML required — `pip install pyyaml`", file=sys.stderr)
    sys.exit(2)

CORE_ROOT = Path(__file__).resolve().parent.parent
MANIFEST = CORE_ROOT / "validation" / "manifest.yaml"
OUT_DOC = CORE_ROOT / "docs" / "boards" / "VALIDATION_STATUS.md"

TIER_BADGE = {
    "silicon-verified": "🟢 silicon-verified",
    "silicon-smoke": "🟢 silicon-smoke",
    "sim-validated": "🔵 sim-validated (deep model, no HW diff)",
    "smoke-manual": "🟡 smoke-manual",
    "structural": "⚪ structural",
}


def parse_iso(v: str) -> datetime:
    """datetime.fromisoformat, but accepting the trailing 'Z' git emits for UTC.

    `git log --format=%cI` renders UTC as '...T00:11:20Z'. Only Python 3.11+
    accepts that suffix, so on the macOS system interpreter (3.9) every local
    run of this script died in newest_commit_date() while CI's 3.12 passed —
    the regeneration command the error message itself tells you to run was
    impossible to run on a stock Mac. Normalise instead of requiring 3.11.
    """
    return datetime.fromisoformat(v[:-1] + "+00:00" if v.endswith("Z") else v)


# `path: "../peripherals/esp32c3/system.yaml"` inside a chip yaml.
CHIP_YAML_PATH_RE = re.compile(r"""^\s*path:\s*["']?([^"'\s#]+)["']?\s*(?:#.*)?$""", re.M)


def covers(model_entry: str, path: str) -> bool:
    """True if a `models` entry (file or directory) covers `path`."""
    return path == model_entry or path.startswith(model_entry.rstrip("/") + "/")


def watch_gaps(board: dict) -> tuple[list[str], list[str]]:
    """(uncovered chip-yaml paths, listed model paths that do not exist).

    Both are provenance holes: a missing watch can hide a source change while
    the report still claims a match. These remain validation failures.
    """
    models = board.get("models", [])
    missing = [m for m in models if not (CORE_ROOT / m).exists()]

    chip_rel = board.get("chip")
    uncovered: list[str] = []
    if chip_rel and (CORE_ROOT / chip_rel).exists():
        chip_dir = Path(chip_rel).parent
        for raw in CHIP_YAML_PATH_RE.findall((CORE_ROOT / chip_rel).read_text()):
            # Chip yamls reach configs/peripherals via `../`; normalise textually
            # (posixpath.normpath, not resolve()) so the result is repo-relative
            # and stable regardless of where the checkout lives or symlinks.
            wired = posixpath.normpath(posixpath.join(chip_dir.as_posix(), raw))
            if not any(covers(m, wired) for m in models):
                uncovered.append(wired)
    return sorted(set(uncovered)), missing


def audit_watch_lists(manifest: dict) -> int:
    """Fail the build on any drift-watch hole. Returns 0 or 1."""
    rc = 0
    for b in manifest["boards"]:
        uncovered, missing = watch_gaps(b)
        if missing:
            print(
                f"ERROR: {b['id']}: `models` lists path(s) that do not exist — a stale "
                f"entry watches nothing:\n  " + "\n  ".join(missing),
                file=sys.stderr,
            )
            rc = 1
        if uncovered:
            print(
                f"ERROR: {b['id']}: {len(uncovered)} path(s) wired by {b['chip']} are NOT "
                "covered by its `models` watch list, so a change to them cannot "
                "be reported:\n  " + "\n  ".join(uncovered) + "\n"
                "       Add them (a parent directory counts) to validation/manifest.yaml.",
                file=sys.stderr,
            )
            rc = 1
    return rc


def git_out(*args: str) -> str | None:
    """stdout of a read-only git command in CORE_ROOT, or None if git exited nonzero.

    None and "" are different answers here: `config --get-regexp` exits 1 with no
    output when nothing matches (fine), while `rev-parse` exits 1 when this is not
    a repository at all (not fine). Callers below depend on telling those apart.
    """
    p = subprocess.run(["git", *args], cwd=CORE_ROOT, capture_output=True, text=True)
    return p.stdout.strip() if p.returncode == 0 else None


def history_defect() -> str | None:
    """Why this checkout cannot answer "when did <path> last change?", or None.

    Each branch below is a way for `git log -1 -- <path>` to return an answer that
    is confidently wrong rather than absent, which is why they are checked at all.
    """
    # Without a repository, `git log` cannot supply source-history diagnostics.
    if git_out("rev-parse", "--is-inside-work-tree") != "true":
        return "not a git checkout — there is no history here to read dates from"

    if git_out("rev-parse", "--is-shallow-repository") == "true":
        return "shallow clone — history is truncated at the graft boundary"

    # `git replace` refs and the legacy .git/info/grafts file rewrite the parent
    # chain the log walk follows, so the walk can terminate at a synthetic
    # boundary exactly as a shallow clone does. Rare, but the resulting wrongness
    # is identical and the check is two cheap plumbing calls.
    #
    # --git-path answers relative to git's cwd, i.e. CORE_ROOT, not this process's;
    # joining is a no-op when it comes back absolute (linked worktrees, GIT_DIR).
    grafts = git_out("rev-parse", "--git-path", "info/grafts")
    if grafts and (CORE_ROOT / grafts).exists():
        return f"grafted history (`{grafts}` exists) — the parent chain is rewritten"
    if git_out("for-each-ref", "--format=%(refname)", "refs/replace/"):
        return "replace refs present (refs/replace/*) — the parent chain is rewritten"

    # The history walk reads commits and trees, so blob-only filters leave
    # its required objects local. model_digest separately reads watched files
    # from the working tree; watch-list checks require those paths to exist.
    # A tree-omitting filter can force a fetch per pathspec evaluation and leave
    # history diagnostics incomplete if the promisor is unreachable. Preserve
    # the existing guard against every filter that is not blob-scoped.
    configured = git_out("config", "--get-regexp", r"^remote\..*\.partialclonefilter") or ""
    bad = set()
    for line in configured.splitlines():
        # `remote.origin.partialclonefilter blob:none` — key, space, filter spec.
        parts = line.split(None, 1)
        if len(parts) == 2 and not parts[1].startswith("blob:"):
            bad.add(parts[1])
    if bad:
        return (
            f"partial clone with a tree-omitting filter ({', '.join(sorted(bad))}) — the tree "
            "objects `git log -- <path>` needs are not local"
        )
    return None


def require_full_history() -> None:
    """Retain the full-history precondition in every reporting mode.

    `newest_model` remains available as diagnostic metadata. Truncated,
    replaced, grafted, or tree-incomplete history can silently invent its date.
    The source-match verdict uses captured content digests, never those dates.
    """
    defect = history_defect()
    if defect is None:
        return
    print(
        f"ERROR: incomplete git history ({defect}).\n"
        "       Board dates here are derived from `git log -1 -- <model path>`, which on a\n"
        "       truncated history resolves to the graft boundary instead of the real commit.\n"
        "       Refusing to report from untrustworthy history.\n"
        "       Fix: git fetch --unshallow\n"
        "       In CI: actions/checkout with `fetch-depth: 0`.\n"
        "       A bounded `git fetch --depth=<n>` is NOT a fix — it only moves the boundary,\n"
        "       and the depth that would suffice is whatever reaches past the OLDEST last-touch\n"
        "       among all `models` paths (months of history), which you cannot know without\n"
        "       already having the history. Deepen until `git rev-parse\n"
        "       --is-shallow-repository` prints false.",
        file=sys.stderr,
    )
    sys.exit(2)


def newest_commit_date(paths: list[str]) -> date | None:
    """Newest committer date (YYYY-MM-DD) across the given repo paths, or None.

    Correctness rests on the whole history being present; require_full_history()
    is what makes that an assertion instead of an assumption.
    """
    newest: date | None = None
    for rel in paths:
        target = CORE_ROOT / rel
        if not target.exists():
            # A listed model path that no longer exists is itself a manifest bug;
            # audit_watch_lists() turns this into a hard failure under the gates.
            print(f"WARNING: manifest model path does not exist: {rel}", file=sys.stderr)
            continue
        out = subprocess.run(
            ["git", "log", "-1", "--format=%cI", "--", rel],
            cwd=CORE_ROOT,
            capture_output=True,
            text=True,
        )
        iso = out.stdout.strip()
        if not iso:
            continue
        d = parse_iso(iso).date()
        if newest is None or d > newest:
            newest = d
    return newest


def model_digest(paths: list[str]) -> str | None:
    """A content hash over every file under `paths`, or None if none exist.

    File paths and bytes determine this digest. Squash, rebase, and cherry-pick
    can change commit dates without changing the source. Conversely, an
    uncommitted or same-day edit changes the digest immediately. Only a digest
    recorded with a physical capture can establish that capture's source match.
    """
    import hashlib

    h = hashlib.sha256()
    found = False
    for rel in sorted(paths):
        target = CORE_ROOT / rel
        if not target.exists():
            continue
        files = sorted(target.rglob("*")) if target.is_dir() else [target]
        for f in files:
            if not f.is_file():
                continue
            found = True
            h.update(str(f.relative_to(CORE_ROOT)).encode())
            h.update(b"\0")
            h.update(f.read_bytes())
            h.update(b"\0")
    return h.hexdigest() if found else None


def as_date(v) -> date | None:
    if v is None:
        return None
    if isinstance(v, date):
        return v
    return parse_iso(str(v)).date()


def capture_provenance(board: dict) -> tuple[date | None, str | None]:
    """Read a capture without turning missing or malformed evidence into a match."""
    silicon = board.get("silicon")
    if silicon is None:
        return None, None
    label = f"{board.get('id', '<unnamed>')}: silicon"
    if not isinstance(silicon, dict):
        raise ValueError(f"{label} must be a mapping or null")
    try:
        capture = as_date(silicon.get("last_capture"))
    except (TypeError, ValueError) as exc:
        raise ValueError(f"{label}.last_capture must be a valid date") from exc
    if capture is None:
        raise ValueError(f"{label}.last_capture is required for a capture record")

    # Absence is allowed for older captures, but a present malformed digest is
    # invalid evidence, not informational source drift.
    recorded = silicon.get("models_digest")
    if "models_digest" in silicon:
        if not isinstance(recorded, str) or not re.fullmatch(r"[0-9a-f]{64}", recorded):
            raise ValueError(f"{label}.models_digest must be a lowercase SHA-256 digest")
    return capture, recorded


def evaluate(board: dict, today: date | None = None) -> dict:
    """Report capture/source provenance independently of human ACKs or the clock.

    `today` is retained for callers and calendar-stability regression tests.
    Source drift never fails a gate. Integrity errors still fail separately.
    """
    models = board.get("models", [])
    newest = newest_commit_date(models)
    digest = model_digest(models)
    capture, captured_digest = capture_provenance(board)
    source_match = digest == captured_digest if captured_digest is not None else None
    drifted = source_match is False

    if capture is None:
        status = "no silicon capture"
    elif source_match is None:
        status = "source match unverified — capture has no model digest"
    elif source_match:
        status = "capture source matches (historical silicon evidence)"
    else:
        status = "source changed since capture — re-capture needed for current silicon claim"

    return {
        "newest_model": newest,
        "digest": digest,
        "capture": capture,
        "capture_digest": captured_digest,
        "source_match": source_match,
        "drifted": drifted,
        "status": status,
    }


def render(manifest: dict, today: date | None = None) -> str:
    """Render historical evidence and source matches without calendar changes."""
    boards = manifest["boards"]
    lines: list[str] = []
    lines.append("<!-- GENERATED by scripts/generate_validation_status.py — DO NOT EDIT BY HAND.")
    lines.append("     Source of truth: validation/manifest.yaml. Not committed; rendered by CI and the docs deploy. -->")
    lines.append("")
    lines.append("# Board validation status")
    lines.append("")
    lines.append(
        "Machine-generated from `validation/manifest.yaml` and not committed. CI enforces "
        "(`--check`) a manifest that renders, complete source watch lists, and valid capture metadata. "
        "Source/capture differences (`--drift`) are informational and do not require "
        "a human source-review acknowledgement. "
        "Tiers: 🟢 silicon · 🟡 manual-smoke · ⚪ structural."
    )
    lines.append("")
    lines.append(
        "The tier and capture date describe historical evidence. A source match "
        "requires the current `models` content digest to equal `silicon.models_digest` "
        "recorded with that capture. Missing captured digests remain unverified; "
        "legacy `drift_ack` fields and commit dates cannot establish a match. "
        "A changed or unverified match needs a physical re-capture before claiming "
        "current silicon validation. Automated simulation results remain separate evidence."
    )
    lines.append("")
    lines.append(
        "The models column hashes watched paths and file contents. It changes "
        "with source edits, including uncommitted and same-day changes, and stays "
        "stable when identical content is rebased or squash-merged."
    )
    lines.append("")
    lines.append("| Board | Tier | Last silicon capture | Models | Status |")
    lines.append("|-------|------|----------------------|--------|--------|")
    for b in boards:
        ev = evaluate(b, today=today)
        tier = TIER_BADGE.get(b["tier"], b["tier"])
        cap = f"{ev['capture']:%Y-%m-%d}" if ev["capture"] else "—"
        dg = f"`{ev['digest'][:16]}`" if ev["digest"] else "—"
        lines.append(f"| `{b['id']}` | {tier} | {cap} | {dg} | {ev['status']} |")
    lines.append("")

    # Per-board detail
    for b in boards:
        ev = evaluate(b, today=today)
        lines.append(f"## `{b['id']}` — {TIER_BADGE.get(b['tier'], b['tier'])}")
        lines.append("")
        lines.append(f"- Doc: [`{b['doc']}`]({Path(b['doc']).name})  ·  Chip: `{b['chip']}`")
        if b.get("note"):
            lines.append(f"- Note: {b['note']}")
        sil = b.get("silicon")
        if sil:
            lines.append(
                f"- Silicon: **{ev['capture']:%Y-%m-%d}** on {sil.get('probe', '?')} — {sil.get('result', '')}"
            )
        else:
            lines.append("- Silicon: none — not validated against real hardware.")
        for t in b.get("offline_tests", []):
            lines.append(f"  - offline (CI): {t}")
        lines.append(f"- Source/capture status: **{ev['status']}**")
        lines.append("")
    return "\n".join(lines).rstrip() + "\n"


def print_digests(manifest: dict) -> int:
    """Print current and recorded digests without changing evidence or gating drift."""
    boards = manifest.get("boards", [])
    if not boards:
        print("manifest lists no boards")
        return 0
    width = max(len(board["id"]) for board in boards)

    for board in boards:
        verdict = evaluate(board)
        digest = verdict["digest"] or "(no model path resolves; nothing to hash)"
        print(f"{board['id']:<{width}}  {digest}  {verdict['status']}")

        # Keep capture evidence and optional historical source-review metadata
        # distinct. Neither is rewritten by this read-only report.
        captured = (board.get("silicon") or {}).get("models_digest")
        acked = board.get("drift_ack_digest")
        for label, recorded in (("capture", captured), ("historical ack", acked)):
            if recorded and recorded != verdict["digest"]:
                print(f"{'':<{width}}  {label} recorded {recorded}")

    # Source/capture differences are informational in all reporting modes.
    return 0


def write_ack_digests(manifest: dict, stamp_all: bool = False) -> int:
    """Update optional legacy ACK metadata only when explicitly requested.

    Kept for callers that maintain historical `drift_ack` fields. This does not
    create an ACK, change physical capture evidence, affect source-match status,
    or satisfy a gate. The default updates only known source/capture mismatches;
    `stamp_all` also updates other existing ACK records. Comments are preserved.
    """
    text = MANIFEST.read_text()
    lines = text.split("\n")
    stamped = 0
    skipped_not_drifted: list[str] = []

    for board in manifest.get("boards", []):
        if not board.get("drift_ack"):
            continue
        digest = model_digest(board.get("models", []))
        if not digest:
            continue
        if board.get("drift_ack_digest") == digest:
            continue
        # Preserve the legacy default's bounded scope; this is never run by CI.
        if not stamp_all and not evaluate(board)["drifted"]:
            skipped_not_drifted.append(board["id"])
            continue

        # Find this board's `drift_ack:` line: scan from its `- id:` header to
        # the next one, so a shared date can't match the wrong board.
        start = next(
            (i for i, ln in enumerate(lines) if ln.strip() == f"- id: {board['id']}"),
            None,
        )
        if start is None:
            print(f"WARNING: no `- id: {board['id']}` line found", file=sys.stderr)
            continue
        end = next(
            (i for i in range(start + 1, len(lines)) if lines[i].strip().startswith("- id: ")),
            len(lines),
        )
        ack_i = next(
            (i for i in range(start, end) if lines[i].strip().startswith("drift_ack:")),
            None,
        )
        if ack_i is None:
            continue

        indent = lines[ack_i][: len(lines[ack_i]) - len(lines[ack_i].lstrip())]
        digest_i = next(
            (i for i in range(start, end) if lines[i].strip().startswith("drift_ack_digest:")),
            None,
        )
        if digest_i is None:
            lines.insert(ack_i + 1, f"{indent}drift_ack_digest: {digest}")
        else:
            lines[digest_i] = f"{indent}drift_ack_digest: {digest}"
        stamped += 1

    MANIFEST.write_text("\n".join(lines))
    print(f"stamped {stamped} historical drift_ack_digest value(s); capture evidence unchanged")
    # Name what was left alone and why. The old sweep's cost stayed invisible
    # until someone diffed the manifest and found boards they never touched.
    if skipped_not_drifted:
        print(
            f"left {len(skipped_not_drifted)} historical digest(s) alone — no known "
            f"capture mismatch: {', '.join(sorted(skipped_not_drifted))}"
        )
        print("  (use --write-ack-digests-all to tidy those deliberately)")
    return 0


class DuplicateKeyError(ValueError):
    pass


class _UniqueKeyLoader(yaml.SafeLoader):
    """SafeLoader that refuses every duplicate mapping key.

    Silently taking the last value can conceal conflicting capture evidence or
    source paths. Historical ACK fields follow the same uniqueness rule.
    """

    def construct_mapping(self, node, deep=False):
        seen = {}
        for key_node, _ in node.value:
            key = self.construct_object(key_node, deep=deep)
            if key in seen:
                raise DuplicateKeyError(
                    f"duplicate key {key!r} at line {key_node.start_mark.line + 1} "
                    f"(first at line {seen[key] + 1})"
                )
            seen[key] = key_node.start_mark.line
        return super().construct_mapping(node, deep=deep)


def load_manifest(text: str):
    try:
        manifest = yaml.load(text, Loader=_UniqueKeyLoader)
        for board in manifest["boards"]:
            capture_provenance(board)
        return manifest
    except (DuplicateKeyError, ValueError) as exc:
        sys.exit(f"error: validation/manifest.yaml: {exc}")


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument(
        "--check",
        action="store_true",
        help="run all integrity gates (history, manifest, capture metadata, watch lists, not-committed)",
    )
    ap.add_argument(
        "--output",
        type=Path,
        help="also write the rendered report here (default location is written only in plain generate mode)",
    )
    ap.add_argument("--drift", action="store_true", help="report source/capture differences; drift is informational, integrity errors fail")
    ap.add_argument(
        "--digests",
        action="store_true",
        help="print each board's current model digest against what it recorded",
    )
    ap.add_argument(
        "--write-ack-digests-all",
        action="store_true",
        help=(
            "update all existing legacy ACK digests as optional historical metadata; "
            "never changes capture evidence or validation status"
        ),
    )
    ap.add_argument(
        "--write-ack-digests",
        action="store_true",
        help="update existing legacy ACK digests for known capture mismatches; optional metadata only",
    )
    args = ap.parse_args()

    # Before anything reads the manifest or touches the doc: this exits(2) — an
    # environment precondition failure, like the missing-PyYAML exit above, not a
    # gate verdict (1) — if the checkout cannot supply trustworthy dates.
    require_full_history()

    manifest = load_manifest(MANIFEST.read_text())

    if args.digests:
        return print_digests(manifest)

    if args.write_ack_digests or args.write_ack_digests_all:
        return write_ack_digests(manifest, stamp_all=args.write_ack_digests_all)

    rendered = render(manifest)

    rc = 0

    # Coverage before content: a stale doc is visible, an unwatched model is not.
    if args.check or args.drift:
        rc |= audit_watch_lists(manifest)

    if args.check:
        # The report is generated output: it must not be committed, or every
        # model edit rewrites it again and unrelated PRs conflict in it.
        rel = OUT_DOC.relative_to(CORE_ROOT).as_posix() if OUT_DOC.is_relative_to(CORE_ROOT) else None
        if rel and git_out("ls-files", "--error-unmatch", "--", rel) is not None:
            print(
                f"ERROR: {rel} is tracked by git. It is generated (git-ignored) and published "
                "by CI/deploy-docs; committing it reintroduces merge conflicts.\n"
                f"       Run: git rm --cached {rel}",
                file=sys.stderr,
            )
            rc = 1
    elif not args.drift and args.output is None:
        # Pure generate mode (no gate flags): rewrite the doc in place.
        OUT_DOC.write_text(rendered)
        print(f"wrote {OUT_DOC.relative_to(CORE_ROOT)}")

    if args.output is not None:
        args.output.parent.mkdir(parents=True, exist_ok=True)
        args.output.write_text(rendered)
        print(f"wrote {args.output}")

    if args.drift:
        verdicts = {
            b["id"]: evaluate(b) for b in manifest["boards"] if b.get("silicon") is not None
        }
        counts = {
            "matching": sum(v["source_match"] is True for v in verdicts.values()),
            "changed": sum(v["source_match"] is False for v in verdicts.values()),
            "unverified": sum(v["source_match"] is None for v in verdicts.values()),
        }
        print(
            "Source/capture report (informational; no source-review ACK required): "
            + ", ".join(f"{count} {label}" for label, count in counts.items())
        )
        for board_id, verdict in verdicts.items():
            if verdict["source_match"] is not True:
                print(f"  {board_id}: {verdict['status']}")

    return rc


if __name__ == "__main__":
    sys.exit(main())
