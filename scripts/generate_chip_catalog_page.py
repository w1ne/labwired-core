#!/usr/bin/env python3
"""Render docs/coverage/chips.json as the published chip list page.

One page listing every chip LabWired models, public and private, with its
coverage status. The data comes from generate_chip_catalog.py, never from a
hand-written list, so the page cannot drift from the catalogues.

Private parts appear as a name + coverage level only. Their descriptor
contents (register maps, addresses, geometry) are never read into this page;
generate_chip_catalog.py counts them and writes nothing else out.
"""
from __future__ import annotations

import argparse
import html
import json
import sys
from pathlib import Path

# Human label per coverage value, and the tone class it renders with.
TONES = {
    "L2 verified": "ok",
    "L1 modelled": "ok",
    "L0 estate": "warn",
    "modelled": "warn",
    "verified (P0-P5)": "ok",
    "modelled (private)": "warn",
    "excluded": "muted",
}


def esc(value) -> str:
    return html.escape(str(value if value is not None else ""))


def tone(chip: dict) -> str:
    if chip.get("status") == "excluded":
        return "muted"
    return TONES.get(chip.get("coverage", ""), "warn")


def coverage_cell(chip: dict) -> str:
    cov = chip.get("coverage") or "modelled"
    if chip.get("status") == "excluded":
        return (
            '<span class="lw-badge lw-badge--muted" '
            f'title="{esc(chip.get("excluded_reason") or "retired")}">excluded</span>'
        )
    extra = ""
    if chip["provenance"] == "public":
        t1 = chip.get("tier1") or {}
        if t1.get("pass"):
            extra = f' <span class="lw-dim">T1 {t1["pass"]}✓</span>'
        elif t1.get("partial") or t1.get("blocked"):
            extra = (
                f' <span class="lw-dim">T1 {t1.get("pass", 0)}✓'
                f'/{t1.get("partial", 0)}~/{t1.get("blocked", 0)}✗</span>'
            )
        rm = chip.get("reg_match")
        if rm:
            extra += f' <span class="lw-dim">reg {rm}</span>'
    return f'<span class="lw-badge lw-badge--{tone(chip)}">{esc(cov)}</span>{extra}'


def render(data: dict) -> str:
    chips = data["chips"]
    s = data["summary"]
    families = sorted({c["family"] for c in chips})
    vendors = sorted({c["vendor"] for c in chips})
    coverages = sorted({c["coverage"] for c in chips})

    out: list[str] = []
    a = out.append

    a("# Chip list")
    a("")
    priv = s.get("private", 0)
    if priv:
        a(
            f"Every chip LabWired models: **{s['chips']} parts** — "
            f"**{s['public']} public** (open models in "
            f"[labwired-core](https://github.com/w1ne/labwired-core)) and "
            f"**{priv} private** (commercial models). "
            f"{s['excluded']} retired part(s) are kept in the table marked excluded."
        )
    else:
        a(
            f"The public chip catalogue: **{s['chips']} parts** with an open model "
            f"in [labwired-core](https://github.com/w1ne/labwired-core). "
            f"{s['excluded']} retired part(s) are kept in the table marked excluded. "
            "Commercial (private) models are listed in the "
            "LabWired app under your account."
        )
    a("")
    a(
        "The table is generated from the live catalogues by "
        "`scripts/generate_chip_catalog.py` and refreshed in CI, so it cannot "
        "drift from the models."
        + (
            " Private parts show a name and a coverage level only — their "
            "descriptor contents stay internal."
            if priv
            else ""
        )
    )
    a("")
    a("## How to read the coverage level")
    a("")
    a("| Level | Meaning |")
    a("|---|---|")
    a(
        "| **L2 verified** | Conformance level 2: register-level match to silicon "
        "*and* a behaviour gate passes on real firmware. |"
    )
    a(
        "| **L1 modelled** | The peripheral estate answers at its own addresses "
        "*or* a behaviour gate passes — one of the two. |"
    )
    a("| **L0 estate** | Modelled and boots, with no register or behaviour gate yet. |")
    a(
        "| **modelled** | The chip descriptor exists and drives firmware; no "
        "conformance level recorded yet. |"
    )
    a(
        "| **verified (P0-P5)** | Private model that passed all six evidence gates: "
        "descriptor, runnable example, smoke on a real image, instruction audit, "
        "and a committed report. |"
    )
    a(
        "| **modelled (private)** | Private model that runs firmware but has not "
        "cleared every gate. |"
    )
    a("| **excluded** | Retired on purpose — obsolete or not buyable. |")
    a("")
    a("> **T1** is the tier-1 happy/sad-path matrix (`T1 8✓` = 8 subsystems "
      "passing). **reg** is the count of cold-reset registers reproduced from a "
      "silicon capture. Both are public-side evidence.")
    a("")

    # Filter controls. Plain HTML so it works with mkdocs-material without a
    # plugin; the script reads the data rows and hides non-matching ones.
    a('<div class="lw-chips" markdown="0">')
    a('  <div class="lw-chips__controls">')
    a(
        '    <input id="lw-chip-q" type="search" placeholder="Filter by name…" '
        'aria-label="Filter chips by name" />'
    )
    a('    <label>Family <select id="lw-chip-family">')
    a('      <option value="">all families</option>')
    for fam in families:
        a(f'      <option value="{esc(fam)}">{esc(fam)}</option>')
    a("    </select></label>")
    a('    <label>Vendor <select id="lw-chip-vendor">')
    a('      <option value="">all vendors</option>')
    for v in vendors:
        a(f'      <option value="{esc(v)}">{esc(v)}</option>')
    a("    </select></label>")
    a('    <label>Coverage <select id="lw-chip-coverage">')
    a('      <option value="">all coverage</option>')
    for cov in coverages:
        a(f'      <option value="{esc(cov)}">{esc(cov)}</option>')
    a('      <option value="excluded">excluded only</option>')
    a("    </select></label>")
    a("  </div>")
    a(
        f'  <p class="lw-chips__count" id="lw-chip-count">Showing {s["chips"]} '
        f'of {s["chips"]} chips.</p>'
    )
    a('  <table class="lw-chips__table">')
    a("    <thead><tr>")
    a('      <th scope="col">Chip</th>')
    a('      <th scope="col">Vendor</th>')
    a('      <th scope="col">Family</th>')
    a('      <th scope="col">Package</th>')
    a('      <th scope="col">Coverage</th>')
    a('      <th scope="col">Model</th>')
    a("    </tr></thead>")
    a("    <tbody>")
    for c in chips:
        search = " ".join(
            [c["name"], c["vendor"], c["family"], c.get("package") or "", c["coverage"]]
        ).lower()
        a(
            f'      <tr class="lw-chip" data-name="{esc(c["name"])}" '
            f'data-family="{esc(c["family"])}" data-vendor="{esc(c["vendor"])}" '
            f'data-coverage="{esc(c["coverage"])}" '
            f'data-status="{esc(c["status"])}" '
            f'data-search="{esc(search)}">'
        )
        a(f'        <td><code>{esc(c["name"])}</code></td>')
        a(f'        <td>{esc(c["vendor"])}</td>')
        a(f'        <td>{esc(c["family"])}</td>')
        pkg = c.get("package") or "—"
        a(f'        <td class="lw-dim">{esc(pkg)}</td>')
        a(f'        <td>{coverage_cell(c)}</td>')
        a(f'        <td class="lw-dim">{esc(c["provenance"])}</td>')
        a("      </tr>")
    a("    </tbody>")
    a("  </table>")
    a('  <p class="lw-chips__empty" id="lw-chip-empty" hidden>No chip matches '
      "that filter.</p>")
    a("</div>")
    a("")
    a("## Counts by family and vendor")
    a("")
    a("| Vendor | Chips |")
    a("|---|---|")
    for vend, n in s["byVendor"].items():
        a(f"| {esc(vend)} | {n} |")
    a("")
    a("| Family | Chips |")
    a("|---|---|")
    for fam, n in s["byFamily"].items():
        a(f"| {esc(fam)} | {n} |")
    a("")
    a("## Related coverage pages")
    a("")
    a("- [Chip conformance](chip-conformance.md) — the L0/L1/L2 register scoreboard.")
    a("- [Tier-1 matrix](tier1-scoreboard.md) — per-subsystem pass matrix.")
    a("- [Arduino scoreboard](arduino-scoreboard.md) and "
      "[Zephyr scoreboard](zephyr-scoreboard.md) — board-level runs.")
    a("")

    return "\n".join(out) + "\n"


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--core-root", default=str(Path(__file__).resolve().parents[1]))
    ap.add_argument("--data", default="", help="chips.json (default: docs/coverage/chips.json)")
    ap.add_argument("--out", default="", help="markdown (default: docs/coverage/chips.md)")
    args = ap.parse_args()

    root = Path(args.core_root).resolve()
    data_path = Path(args.data) if args.data else root / "docs/coverage/chips.json"
    out_path = Path(args.out) if args.out else root / "docs/coverage/chips.md"

    if not data_path.exists():
        sys.stderr.write(
            f"error: {data_path} not found — run scripts/generate_chip_catalog.py first\n"
        )
        return 2

    data = json.loads(data_path.read_text())
    out_path.write_text(render(data))
    print(f"wrote {out_path} — {data['summary']['chips']} chips")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
