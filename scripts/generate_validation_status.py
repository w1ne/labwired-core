#!/usr/bin/env python3
"""
Generate docs/boards/VALIDATION_STATUS.md from validation/manifest.yaml and
enforce the no-silent-decay rule.

The board docs drift; this generated table does not. It is the single
machine-checked view of what is actually validated, against which silicon, and
on what date — plus an automated DRIFT gate that catches the case where a
peripheral model changed AFTER the board's last silicon capture.

Modes
-----
  (default)      regenerate docs/boards/VALIDATION_STATUS.md in place
  --check        regenerate to memory and diff against the committed file;
                 exit 1 if they differ (run this in CI so the doc cannot go stale)
  --drift        exit 1 if any silicon-tier board has DRIFTED past its drift_ack
  (you normally run CI with BOTH:  --check --drift)

Either gate flag also runs the drift-watch COVERAGE audit (see below).

Drift
-----
For each board with `silicon.last_capture`, the newest git commit date across
`models` is compared to the capture date. If newer, the board has drifted. A
dated `drift_ack` (>= the newest model date) is an explicit human acknowledgement
that keeps it green; any later model change re-breaks the gate.

Drift-watch coverage
--------------------
The drift gate can only see what `models` lists, and an incomplete list fails
OPEN: the board reads "fresh" forever while the files its claim rests on change
underneath. esp32c3 shipped that way — its tier is a reset-state oracle asserted
against the declarative descriptors in `configs/peripherals/esp32c3/`, and all
29 of them were outside its watch list, as was the shared `esp_uart.rs` its real
UART0/UART1 register map moved into on 2026-07-28.

So we audit the watch list itself, mechanically:
  * every `path:` a board's chip yaml wires (resolved relative to the chip yaml)
    must be covered by an entry in that board's `models`; and
  * every listed `models` path must exist — a stale path is a silently disabled
    watch, not a warning.
Coded (non-declarative) peripheral impls cannot be derived from the yaml and are
still listed by hand; this audit closes the mechanical half of the hole.

Needs PyYAML (pip install pyyaml) and a full-history checkout (fetch-depth: 0)
so `git log -- <path>` resolves dates. That requirement is ENFORCED, not
documented — see require_full_history(); on a truncated history this script used
to emit a plausible document with wrong dates rather than fail.
"""

from __future__ import annotations

import argparse
import difflib
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

    Both are drift-gate holes that fail OPEN — the board keeps reading "fresh"
    while something its claim depends on is unwatched. See module docs.
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
                "covered by its `models` drift-watch list, so a change to them cannot "
                "fail the drift gate:\n  " + "\n  ".join(uncovered) + "\n"
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
    # No repository → every `git log` returns empty, every date renders "—", and
    # the drift gate silently degrades to "nothing has ever drifted".
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

    # Partial clones: only a filter that omits TREES is disqualifying, and that
    # distinction is deliberate rather than an oversight. This script never reads
    # file contents — it walks commits and diffs trees against a pathspec — so
    # `--filter=blob:none` (and blob:limit=*) leaves every object the walk touches
    # local and the dates exact. That is the cheap full-history clone worth
    # encouraging for a metadata-only job like this one, so it is allowed. A
    # tree-omitting filter (`tree:0`) is a different animal: git must refetch
    # trees from the promisor one at a time to evaluate the pathspec — unusably
    # slow when it works, and wrong when the promisor is unreachable — so treat
    # anything that is not blob-scoped as unusable rather than guessing.
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
    """Refuse to run at all on a history this script cannot read correctly.

    WHY THIS IS FATAL AND NOT A WARNING
        Every "Newest model" date in the rendered document comes from
        `git log -1 --format=%cI -- <path>`. On a truncated history that walk
        bottoms out at the graft boundary instead of the real last-touching
        commit, and git reports the boundary commit — for every path whose real
        last change predates it. Nothing errors. The document simply comes out
        wrong, and it comes out looking entirely plausible: on a 112-commit
        shallow clone of this repo `ci-fixture-riscv` rendered 2026-08-04 (the
        graft commit b730a43) where the truth on full history is 2026-03-09
        (9957cda8) — four months out, and eight boards wrong at once. The only
        reason it was caught is that CI, which checks out fetch-depth: 0,
        disagreed with a locally regenerated file.

        Those dates are not decoration; they are the left-hand side of the drift
        comparison. Truncation always skews them NEW (an unreachable parent reads
        as "created at the boundary"), which manufactures drift on boards that are
        fine — and the natural way to silence a red gate is to stamp a drift_ack
        at the date the tool just printed. That ack is dated from the graft, not
        from any model change anyone reviewed, so it then blankets every genuine
        model change up to that date: the false positive converts itself into a
        durable false negative. A silicon-validation gate that quietly reads the
        wrong input is worse than no gate, because it is believed.

    WHY IT GUARDS EVERY MODE, NOT ONLY --check/--drift
        Plain generate is the most dangerous mode, not the least. It is the one
        that exits 0 and writes the wrong dates into the committed file, which is
        how they get pushed. --check and --drift do at least fail, but they fail
        with the wrong story — a spurious "out of date" diff, or a phantom DRIFT
        list — and the remedy their own error text prints is the regenerate
        command that commits the damage. No invocation of this script has any use
        for a document built from dates it cannot trust, so all of them stop.
    """
    defect = history_defect()
    if defect is None:
        return
    print(
        f"ERROR: incomplete git history ({defect}).\n"
        "       Board dates here are derived from `git log -1 -- <model path>`, which on a\n"
        "       truncated history resolves to the graft boundary instead of the real commit.\n"
        "       Refusing to emit a document whose dates and drift verdict would be wrong.\n"
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

    WHY CONTENT AND NOT A DATE
      The drift gate asks "has the model changed since silicon was captured?".
      That is a question about CONTENT, but `newest_commit_date` answers it with
      a git timestamp — and a timestamp is metadata that history rewriting moves
      while the content stands still.

      Squash-merge is the case that bit us (#834). A PR acks drift on its branch
      with the date of its own model commit; GitHub squashes, stamping the merge
      time onto every file the PR touched; `newest` jumps past the ack and main
      goes red on content that was reviewed and acked. The author could not have
      written a covering date, because the date the model "changes" on main is
      the merge date and that commit does not exist yet at review time.

      A digest is the thing that IS knowable pre-merge and invariant across the
      merge: squash, rebase and cherry-pick all preserve content. So an ack
      carrying a digest keeps covering exactly the tree it was written for, and
      stops covering the moment a byte of model source actually moves.

    Where a board records one, this REPLACES the date comparison rather than
    supplementing it — see evaluate(). That is strictly stronger on both axes:
    it stops a same-content re-date from failing, and stops a same-day content
    edit from passing.
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


def evaluate(board: dict) -> dict:
    """Compute drift status for one board."""
    silicon = board.get("silicon")
    models = board.get("models", [])
    newest = newest_commit_date(models)
    capture = as_date(silicon["last_capture"]) if silicon else None
    ack = as_date(board.get("drift_ack"))

    drifted = bool(capture and newest and newest > capture)
    # How an ack covers a model.
    #
    #   With a `drift_ack_digest`: CONTENT decides, and only content. The ack
    #   covers exactly the tree it was written against.
    #   Without one: the legacy DATE rule, kept so an un-stamped ack still works.
    #
    # Digest-authoritative rather than date-OR-digest, because it is strictly
    # stronger than the date rule on BOTH axes:
    #
    #   * a squash merge that re-dates a model without changing a byte no longer
    #     reds main on a reviewed, acked tree (#834), and
    #   * a model edit made on or before the ack date no longer slips through.
    #     The date rule accepted any content as long as `ack >= newest`, so an
    #     edit landing the same day an ack was written was silently covered.
    #     That was a real hole and this closes it.
    #
    # The cost is that any genuine model change now re-fails until a human
    # re-acks and re-stamps, which is exactly the manifest's stated intent:
    # "any model change PAST the ack date re-fails the gate. No silent decay."
    recorded = board.get("drift_ack_digest")
    if ack and recorded:
        acked = recorded == model_digest(models)
    else:
        acked = bool(ack and newest and ack >= newest)
    # A board with no silicon capture cannot "drift" — it never claimed silicon.
    failing = drifted and not acked

    if not silicon:
        status = "no silicon capture"
    elif not drifted:
        status = "✅ fresh"
    elif acked:
        status = f"⚠ drift acked {ack:%Y-%m-%d} (re-capture pending)"
    else:
        status = f"✖ DRIFT — model {newest:%Y-%m-%d} > capture; RE-CAPTURE"

    return {
        "newest_model": newest,
        "digest": model_digest(models),
        "capture": capture,
        "drifted": drifted,
        "failing": failing,
        "status": status,
    }


def render(manifest: dict) -> str:
    boards = manifest["boards"]
    lines: list[str] = []
    lines.append("<!-- GENERATED by scripts/generate_validation_status.py — DO NOT EDIT BY HAND.")
    lines.append("     Source of truth: validation/manifest.yaml. Regenerated and gated on every CI run. -->")
    lines.append("")
    lines.append("# Board validation status")
    lines.append("")
    lines.append(
        "Machine-generated from `validation/manifest.yaml`. CI regenerates this on "
        "every run (`--check`) and fails if a peripheral model changed after a "
        "board's last silicon capture without a dated `drift_ack` (`--drift`). "
        "Tiers: 🟢 silicon · 🟡 manual-smoke · ⚪ structural."
    )
    lines.append("")
    lines.append(
        "The models column is a content digest over everything that board's "
        "`models` list watches, NOT a commit date. Rendering the newest "
        "committer date here meant every squash merge that touched a watched "
        "path re-dated the column and made this committed file stale, so "
        "`--check` demanded a regen commit that carried no information (#834, "
        "and #798 before it). A digest moves only when the models actually do."
    )
    lines.append("")
    lines.append("| Board | Tier | Last silicon capture | Models | Status |")
    lines.append("|-------|------|----------------------|--------|--------|")
    for b in boards:
        ev = evaluate(b)
        tier = TIER_BADGE.get(b["tier"], b["tier"])
        cap = f"{ev['capture']:%Y-%m-%d}" if ev["capture"] else "—"
        dg = f"`{ev['digest'][:16]}`" if ev["digest"] else "—"
        lines.append(f"| `{b['id']}` | {tier} | {cap} | {dg} | {ev['status']} |")
    lines.append("")

    # Per-board detail
    for b in boards:
        ev = evaluate(b)
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
        lines.append(f"- Drift status: **{ev['status']}**")
        lines.append("")
    return "\n".join(lines).rstrip() + "\n"


def write_ack_digests(manifest: dict) -> int:
    """Stamp `drift_ack_digest` next to every existing `drift_ack`.

    Deliberately only touches boards that ALREADY carry a human-written
    `drift_ack`. Writing a digest is not an acknowledgement — the ack is the
    human assertion, the digest only pins WHICH tree it was asserted about. A
    board with no ack must stay un-acked; this must never be a way to bulk-
    silence the gate.

    Edits the YAML as text rather than round-tripping through the loader: the
    manifest is a hand-maintained document whose comments carry the reasoning
    for every ack, and PyYAML would drop all of them.
    """
    text = MANIFEST.read_text()
    lines = text.split("\n")
    stamped = 0

    for board in manifest.get("boards", []):
        if not board.get("drift_ack"):
            continue
        digest = model_digest(board.get("models", []))
        if not digest:
            continue
        if board.get("drift_ack_digest") == digest:
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
    print(f"stamped {stamped} drift_ack_digest value(s)")
    return 0


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--check", action="store_true", help="fail if committed doc is stale")
    ap.add_argument("--drift", action="store_true", help="fail if any board drifted past its ack")
    ap.add_argument(
        "--write-ack-digests",
        action="store_true",
        help="stamp drift_ack_digest for every board that already carries a drift_ack",
    )
    args = ap.parse_args()

    # Before anything reads the manifest or touches the doc: this exits(2) — an
    # environment precondition failure, like the missing-PyYAML exit above, not a
    # gate verdict (1) — if the checkout cannot supply trustworthy dates.
    require_full_history()

    manifest = yaml.safe_load(MANIFEST.read_text())

    if args.write_ack_digests:
        return write_ack_digests(manifest)

    rendered = render(manifest)

    rc = 0

    # Coverage before content: a stale doc is visible, an unwatched model is not.
    if args.check or args.drift:
        rc |= audit_watch_lists(manifest)

    if args.check:
        existing = OUT_DOC.read_text() if OUT_DOC.exists() else ""
        if existing != rendered:
            # Print the actual difference. This doc is rendered partly from git
            # commit dates, so a checkout whose history differs from yours can
            # fail this gate while it passes locally — and "is out of date" on
            # its own gives a CI reader nothing to act on.
            diff = "".join(
                difflib.unified_diff(
                    existing.splitlines(keepends=True),
                    rendered.splitlines(keepends=True),
                    fromfile=f"{OUT_DOC.name} (committed)",
                    tofile=f"{OUT_DOC.name} (regenerated here)",
                )
            )
            print(
                f"ERROR: {OUT_DOC.relative_to(CORE_ROOT)} is out of date.\n"
                "       Run: python3 scripts/generate_validation_status.py\n"
                f"{diff}",
                file=sys.stderr,
            )
            rc = 1
    elif not args.drift:
        # Pure generate mode (no gate flags): rewrite the doc in place.
        OUT_DOC.write_text(rendered)
        print(f"wrote {OUT_DOC.relative_to(CORE_ROOT)}")

    if args.drift:
        failing = [b["id"] for b in manifest["boards"] if evaluate(b)["failing"]]
        if failing:
            print(
                "ERROR: silicon validation has DRIFTED (model changed after last capture, "
                "no covering drift_ack):\n  " + "\n  ".join(failing) + "\n"
                "       Re-run the live diff and bump silicon.last_capture, or set a dated "
                "drift_ack in validation/manifest.yaml.",
                file=sys.stderr,
            )
            rc = 1

    return rc


if __name__ == "__main__":
    sys.exit(main())
