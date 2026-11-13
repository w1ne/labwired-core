#!/usr/bin/env python3
"""Split an old single-file board registry into one file per board.

Used once to migrate `validation/manifest.yaml` and
`validation/arduino-matrix/boards.yaml`, and kept so a pull request branched
before that migration can move its own entry across without hand-editing:

    git show <old-base>:validation/manifest.yaml > /tmp/old-manifest.yaml   # or your branch's copy
    python3 scripts/registry_split.py --registry manifest --from /tmp/old-manifest.yaml --id my-board

writes validation/boards/my-board.yaml. Use your branch's version of the old
file (`git show HEAD:validation/manifest.yaml`) to carry your edits across.
`--all` writes every entry plus `_index.yaml` (used by the migration).

Text-level split, so comments inside an entry travel with it. Comment lines
between entries (section banners) are kept in `_index.yaml` with `--all`
(noting the id they preceded); with `--id` the banner right above that entry
is copied onto it.
"""

from __future__ import annotations

import argparse
import re
import sys
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent

REGISTRIES = {
    "manifest": ("validation/boards", "boards"),
    "arduino-matrix": ("validation/arduino-matrix/boards", "boards"),
}

ENTRY_RE = re.compile(r"^  - id:\s*(\S+)\s*(?:#.*)?$")
TOP_KEY_RE = re.compile(r"^[A-Za-z_][\w-]*:")
GAP_RE = re.compile(r"^ {0,2}#|^\s*$")  # blank, or a comment at indent <= 2


def split_text(text: str, list_key: str = "boards"):
    """Return (header_lines, {id: entry_lines}, gaps, tail_lines).

    entry_lines are de-indented to a top-level mapping. `gaps` is a list of
    (preceding_id_or_None, comment_lines) for banner comments between entries.
    """
    lines = text.split("\n")
    start = next((i for i, ln in enumerate(lines) if ln.rstrip() == f"{list_key}:"), None)
    if start is None:
        raise SystemExit(f"error: no top-level `{list_key}:` line found")
    header = lines[:start]

    # Block boundaries: each `  - id:` line, and the first top-level key after.
    idx: list[tuple[int, str]] = []
    tail_at = len(lines)
    for i in range(start + 1, len(lines)):
        m = ENTRY_RE.match(lines[i])
        if m:
            idx.append((i, m.group(1).strip("\"'")))
        elif TOP_KEY_RE.match(lines[i]):
            tail_at = i
            break

    entries: dict[str, list[str]] = {}
    gaps: list[tuple[str | None, list[str]]] = []
    first_start = idx[0][0] if idx else tail_at
    lead = [ln for ln in lines[start + 1 : first_start] if ln.strip()]
    if lead:
        gaps.append((idx[0][1] if idx else None, lead))

    for n, (i, eid) in enumerate(idx):
        end = idx[n + 1][0] if n + 1 < len(idx) else tail_at
        block = lines[i:end]
        # Trailing blank / indent<=2 comment lines are banners for what follows.
        cut = len(block)
        while cut > 1 and GAP_RE.match(block[cut - 1]):
            cut -= 1
        banner = [ln for ln in block[cut:] if ln.strip()]
        block = block[:cut]
        if banner:
            nxt = idx[n + 1][1] if n + 1 < len(idx) else None
            gaps.append((nxt, banner))
        out = ["id: " + block[0].split("id:", 1)[1].strip()]
        for ln in block[1:]:
            out.append(ln[4:] if ln.startswith("    ") else ln.lstrip() if ln.strip() == "" else ln)
        if eid in entries:
            raise SystemExit(f"error: duplicate id {eid!r} in the old file")
        entries[eid] = out
    tail = lines[tail_at:]
    return header, entries, gaps, tail


def render_index(header, gaps, tail) -> str:
    out = list(header)
    while out and not out[-1].strip():
        out.pop()
    if gaps:
        out += ["", "# Banner comments that sat between entries in the old single file.",
                "# Each notes the entry that followed it; the entries now live one per file."]
        for nxt, banner in gaps:
            out += ["#", f"# --- before: {nxt if nxt else '(end of list)'} ---"]
            out += [ln if ln.startswith("#") else "# " + ln.strip()[1:].lstrip() if ln.strip().startswith("#") else ln for ln in banner]
    if tail:
        out += [""] + [ln for ln in tail]
    while out and not out[-1].strip():
        out.pop()
    return "\n".join(out) + "\n"


def main(argv=None) -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--registry", choices=sorted(REGISTRIES), required=True)
    ap.add_argument("--from", dest="src", required=True, help="old-format file ('-' for stdin)")
    ap.add_argument("--id", action="append", default=[], help="entry to extract (repeatable)")
    ap.add_argument("--all", action="store_true", help="write every entry and _index.yaml")
    ap.add_argument("--out", type=Path, help="output directory (default: the registry's directory)")
    args = ap.parse_args(argv)
    if bool(args.id) == args.all:
        ap.error("give --id ID (repeatable) or --all")

    rel, key = REGISTRIES[args.registry]
    out_dir = args.out or REPO_ROOT / rel
    text = sys.stdin.read() if args.src == "-" else Path(args.src).read_text(encoding="utf-8")
    header, entries, gaps, tail = split_text(text, key)

    wanted = list(entries) if args.all else args.id
    missing = [i for i in wanted if i not in entries]
    if missing:
        print(f"error: not in the old file: {', '.join(missing)}", file=sys.stderr)
        return 1
    out_dir.mkdir(parents=True, exist_ok=True)
    banners = {nxt: b for nxt, b in gaps if nxt}
    for eid in wanted:
        body = entries[eid]
        if not args.all and eid in banners:
            # A single-entry extraction keeps the banner that sat right above it.
            body = [ln.strip() if ln.strip().startswith("#") else ln for ln in banners[eid]] + body
        (out_dir / f"{eid}.yaml").write_text("\n".join(body) + "\n", encoding="utf-8")
        print(f"wrote {out_dir / (eid + '.yaml')}")
    if args.all:
        (out_dir / "_index.yaml").write_text(render_index(header, gaps, tail), encoding="utf-8")
        print(f"wrote {out_dir / '_index.yaml'}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
