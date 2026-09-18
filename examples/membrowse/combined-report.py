#!/usr/bin/env python3
"""
Merge a MemBrowse memory report and a LabWired result.json into one verdict.

MemBrowse reads the ELF and the linker script: it knows every byte the linker
placed and which symbol and source file it came from. It cannot know how much
stack the firmware burns, because nothing has run.

LabWired runs the same ELF on the modeled chip: it measures main-stack and heap
high-water by paint, and it knows the device totals from the chip catalog rather
than from the linker script. It has no symbol-level attribution.

Neither tool alone can answer "does this firmware fit". This script adds the two
halves together, gates the sum, and cross-checks the linker script's idea of the
memory map against the silicon model's.

Usage:
    combined-report.py --target nrf54l15-dk \\
        --membrowse out/membrowse.json \\
        --labwired out/labwired/result.json \\
        --budgets budgets.yaml \\
        --markdown out/combined.md

Exit codes:
    0  every gate passed
    1  a budget was exceeded, or the LabWired run did not pass
    2  bad usage or unreadable input
"""

from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path
from typing import Any

# Sections that land in RAM at runtime, and the ones that identify a code region.
RAM_MARKER_SECTIONS = {".bss"}
CODE_MARKER_SECTIONS = {".text"}
RAM_SECTION_NAMES = {".data", ".bss"}

KNOWN_BUDGET_KEYS = {
    "flash_used_bytes",
    "ram_static_bytes",
    "main_stack_high_water_bytes",
    "ram_combined_bytes",
}

BAR_WIDTH = 24


def die(message: str) -> None:
    print(f"error: {message}", file=sys.stderr)
    raise SystemExit(2)


def load_json(path: Path) -> dict[str, Any]:
    try:
        return json.loads(path.read_text())
    except OSError as exc:
        die(f"cannot read {path}: {exc}")
    except json.JSONDecodeError as exc:
        die(f"{path} is not valid JSON: {exc}")
    raise AssertionError("unreachable")


def load_budgets(path: Path | None, target: str) -> dict[str, int]:
    if path is None:
        return {}
    try:
        import yaml
    except ImportError:
        die("--budgets needs PyYAML (pip install pyyaml)")
    try:
        doc = yaml.safe_load(path.read_text()) or {}
    except OSError as exc:
        die(f"cannot read {path}: {exc}")
    targets = doc.get("targets") or {}
    if target not in targets:
        print(
            f"note: no budgets for target '{target}' in {path}; reporting only",
            file=sys.stderr,
        )
        return {}
    return targets[target] or {}


def section_names(region: dict[str, Any]) -> set[str]:
    return {s.get("name", "") for s in region.get("sections") or []}


def classify_regions(layout: dict[str, Any]) -> tuple[dict | None, dict | None]:
    """Pick the RAM region and the code region out of a MemBrowse memory_layout.

    A region holding .bss is RAM. A region holding .text is where code lives.
    .data appears in both (load address in flash, run address in RAM), so it
    cannot be the discriminator on its own.
    """
    ram = code = None
    for name, region in layout.items():
        names = section_names(region)
        entry = dict(region, name=name)
        if names & RAM_MARKER_SECTIONS and ram is None:
            ram = entry
        elif names & CODE_MARKER_SECTIONS and code is None:
            code = entry
    return ram, code


def ram_symbols(membrowse: dict[str, Any], limit: int) -> list[dict[str, Any]]:
    syms = [
        s
        for s in membrowse.get("symbols") or []
        if s.get("section") in RAM_SECTION_NAMES and (s.get("size") or 0) > 0
    ]
    syms.sort(key=lambda s: s.get("size", 0), reverse=True)
    return syms[:limit]


def fmt(n: int | None) -> str:
    return "—" if n is None else f"{n:,}"


def bar(used: int, total: int) -> str:
    if total <= 0:
        return ""
    filled = min(BAR_WIDTH, round(BAR_WIDTH * used / total))
    return "█" * filled + "░" * (BAR_WIDTH - filled)


def pct(used: int, total: int) -> float:
    return 0.0 if total <= 0 else 100.0 * used / total


class Report:
    """Collects the merged numbers and every gate verdict."""

    def __init__(self, target: str, membrowse: dict, labwired: dict, budgets: dict):
        self.target = target
        self.budgets = budgets
        self.failures: list[str] = []
        self.warnings: list[str] = []

        self.status = labwired.get("status", "unknown")
        self.assertions = labwired.get("assertions") or []
        self.firmware_hash = labwired.get("firmware_hash")
        self.architecture = membrowse.get("architecture", "unknown")
        self.toolchain = membrowse.get("toolchain")

        footprint = labwired.get("footprint") or {}
        memory = labwired.get("memory") or {}
        self.ram_total = footprint.get("ram_total_bytes")
        self.flash_total = footprint.get("flash_total_bytes")
        self.lw_ram_static = footprint.get("ram_static_bytes")
        self.lw_flash_used = footprint.get("flash_used_bytes")

        self.stack_method = memory.get("main_stack_method", "unsupported")
        self.stack_peak = (
            memory.get("main_stack_high_water_bytes")
            if self.stack_method == "paint"
            else None
        )
        self.stack_reason = memory.get("main_stack_unsupported_reason")
        self.heap_method = memory.get("heap_method", "unsupported")
        self.heap_peak = (
            memory.get("heap_high_water_bytes") if self.heap_method == "paint" else None
        )

        layout = membrowse.get("memory_layout") or {}
        self.ram_region, self.code_region = classify_regions(layout)
        self.ram_static = (self.ram_region or {}).get("used_size")
        self.flash_used = (self.code_region or {}).get("used_size")

        self.ram_sections = {
            s["name"]: s["size"] for s in (self.ram_region or {}).get("sections") or []
        }
        self.top_ram_symbols = ram_symbols(membrowse, 8)

        # Combined RAM is only honest when every contributor was actually measured.
        parts = [self.ram_static, self.stack_peak, self.heap_peak]
        self.ram_combined = (
            sum(p for p in parts if p is not None)
            if self.ram_static is not None and self.stack_peak is not None
            else None
        )

        self._check_map_agreement()
        self._check_behaviour()
        self._check_budgets()

    def _check_map_agreement(self) -> None:
        """The two tools read the memory map from different sources. Compare them."""
        pairs = [
            ("RAM size", (self.ram_region or {}).get("limit_size"), self.ram_total),
            (
                "code region size",
                (self.code_region or {}).get("limit_size"),
                self.flash_total,
            ),
            ("static RAM", self.ram_static, self.lw_ram_static),
            ("code bytes", self.flash_used, self.lw_flash_used),
        ]
        self.map_checks = []
        for label, from_ld, from_model in pairs:
            if from_ld is None or from_model is None:
                self.map_checks.append((label, from_ld, from_model, "unknown"))
                continue
            agrees = from_ld == from_model
            self.map_checks.append(
                (label, from_ld, from_model, "match" if agrees else "MISMATCH")
            )
            if not agrees:
                self.warnings.append(
                    f"{label}: linker script says {fmt(from_ld)} B, silicon model says "
                    f"{fmt(from_model)} B — the linker script and the chip do not agree"
                )

    def _check_behaviour(self) -> None:
        if self.status != "pass":
            failed = [
                json.dumps(a.get("assertion"))
                for a in self.assertions
                if not a.get("passed")
            ]
            detail = f" ({len(failed)} assertion(s) failed)" if failed else ""
            self.failures.append(f"LabWired run status is '{self.status}'{detail}")

    def _gate(self, key: str, value: int | None, label: str) -> tuple | None:
        budget = self.budgets.get(key)
        if budget is None:
            return None
        if value is None:
            self.warnings.append(
                f"budget '{key}' is set but {label} was not measured on this target"
            )
            return (label, None, budget, "not measured")
        ok = value <= budget
        if not ok:
            self.failures.append(
                f"{label} {fmt(value)} B exceeds budget {fmt(budget)} B "
                f"(over by {fmt(value - budget)} B)"
            )
        return (label, value, budget, "pass" if ok else "FAIL")

    def _check_budgets(self) -> None:
        self.gates = [
            g
            for g in (
                self._gate("flash_used_bytes", self.flash_used, "flash used"),
                self._gate("ram_static_bytes", self.ram_static, "static RAM"),
                self._gate(
                    "main_stack_high_water_bytes", self.stack_peak, "peak main stack"
                ),
                self._gate(
                    "ram_combined_bytes",
                    self.ram_combined,
                    "combined RAM (static+peak)",
                ),
            )
            if g is not None
        ]
        for key in sorted(set(self.budgets) - KNOWN_BUDGET_KEYS):
            self.warnings.append(f"unknown budget key '{key}' ignored")

    @property
    def ok(self) -> bool:
        return not self.failures

    # ---------------------------------------------------------------- rendering

    def markdown(self) -> str:
        verdict = "✅ pass" if self.ok else "❌ fail"
        out: list[str] = []
        out.append(f"### Memory & behaviour — `{self.target}` · {verdict}")
        out.append("")
        meta = [f"arch `{self.architecture}`"]
        if self.toolchain and self.toolchain != "N/A":
            meta.append(f"toolchain `{self.toolchain}`")
        if self.firmware_hash:
            meta.append(f"ELF `{self.firmware_hash[:12]}`")
        out.append(" · ".join(meta))
        out.append("")

        out.append("#### RAM: what the linker placed + what the run actually used")
        out.append("")
        out.append("| Contributor | Bytes | Source |")
        out.append("| --- | ---: | --- |")
        for name in (".data", ".bss"):
            if name in self.ram_sections:
                out.append(
                    f"| `{name}` | {fmt(self.ram_sections[name])} | MemBrowse (static) |"
                )
        if self.stack_peak is not None:
            out.append(
                f"| peak main stack | {fmt(self.stack_peak)} | LabWired (measured) |"
            )
        else:
            reason = self.stack_reason or self.stack_method
            out.append(f"| peak main stack | not measured | LabWired ({reason}) |")
        if self.heap_peak is not None:
            out.append(f"| peak heap | {fmt(self.heap_peak)} | LabWired (measured) |")
        if self.ram_combined is not None and self.ram_total:
            out.append(
                f"| **worst case** | **{fmt(self.ram_combined)}** | "
                f"**{pct(self.ram_combined, self.ram_total):.2f}% of "
                f"{fmt(self.ram_total)} B** |"
            )
            out.append("")
            out.append(
                f"`{bar(self.ram_combined, self.ram_total)}` "
                f"{fmt(self.ram_total - self.ram_combined)} B headroom"
            )
        else:
            out.append("")
            out.append(
                "> Combined worst case is unavailable: the runtime half was not "
                "measured on this target, so static RAM is a floor, not the answer."
            )
        out.append("")

        if self.flash_used is not None and self.flash_total:
            out.append("#### Code")
            out.append("")
            out.append(
                f"`{bar(self.flash_used, self.flash_total)}` {fmt(self.flash_used)} / "
                f"{fmt(self.flash_total)} B ({pct(self.flash_used, self.flash_total):.2f}%)"
            )
            out.append("")

        if self.top_ram_symbols:
            out.append("<details><summary>Largest RAM symbols (MemBrowse)</summary>")
            out.append("")
            out.append("| Symbol | Bytes | Section | Source |")
            out.append("| --- | ---: | --- | --- |")
            for s in self.top_ram_symbols:
                src = s.get("source_file") or "—"
                out.append(
                    f"| `{s['name']}` | {fmt(s['size'])} | `{s.get('section','')}` | `{src}` |"
                )
            out.append("")
            out.append("</details>")
            out.append("")

        out.append("<details><summary>Memory map cross-check</summary>")
        out.append("")
        out.append("Linker script (MemBrowse) vs chip catalog (LabWired):")
        out.append("")
        out.append("| Quantity | Linker script | Silicon model | |")
        out.append("| --- | ---: | ---: | --- |")
        for label, a, b, state in self.map_checks:
            mark = {"match": "✅", "MISMATCH": "⚠️", "unknown": "–"}[state]
            out.append(f"| {label} | {fmt(a)} | {fmt(b)} | {mark} |")
        out.append("")
        out.append("</details>")
        out.append("")

        passed = sum(1 for a in self.assertions if a.get("passed"))
        out.append(
            f"#### Behaviour — {passed}/{len(self.assertions)} LabWired assertions passed"
        )
        out.append("")
        for a in self.assertions:
            mark = "✅" if a.get("passed") else "❌"
            out.append(f"- {mark} `{json.dumps(a.get('assertion'))}`")
        out.append("")

        if self.gates:
            out.append("#### Budgets")
            out.append("")
            out.append("| Gate | Actual | Budget | |")
            out.append("| --- | ---: | ---: | --- |")
            for label, value, budget, state in self.gates:
                mark = {"pass": "✅", "FAIL": "❌", "not measured": "–"}[state]
                out.append(f"| {label} | {fmt(value)} | {fmt(budget)} | {mark} |")
            out.append("")

        for w in self.warnings:
            out.append(f"> ⚠️ {w}")
        for f in self.failures:
            out.append(f"> ❌ {f}")
        if self.warnings or self.failures:
            out.append("")

        out.append(
            "<sub>Static analysis by [MemBrowse](https://membrowse.com) · "
            "runtime measurement by [LabWired](https://labwired.com)</sub>"
        )
        return "\n".join(out)

    def as_dict(self) -> dict[str, Any]:
        return {
            "target": self.target,
            "ok": self.ok,
            "architecture": self.architecture,
            "firmware_hash": self.firmware_hash,
            "static": {
                "flash_used_bytes": self.flash_used,
                "ram_static_bytes": self.ram_static,
                "ram_sections": self.ram_sections,
            },
            "runtime": {
                "main_stack_method": self.stack_method,
                "main_stack_high_water_bytes": self.stack_peak,
                "heap_method": self.heap_method,
                "heap_high_water_bytes": self.heap_peak,
                "status": self.status,
                "assertions_passed": sum(1 for a in self.assertions if a.get("passed")),
                "assertions_total": len(self.assertions),
            },
            "combined": {
                "ram_combined_bytes": self.ram_combined,
                "ram_total_bytes": self.ram_total,
                "ram_headroom_bytes": (
                    None
                    if self.ram_combined is None or self.ram_total is None
                    else self.ram_total - self.ram_combined
                ),
                "flash_total_bytes": self.flash_total,
            },
            "map_checks": [
                {
                    "quantity": label,
                    "linker_script": a,
                    "silicon_model": b,
                    "state": state,
                }
                for label, a, b, state in self.map_checks
            ],
            "gates": [
                {"gate": label, "actual": value, "budget": budget, "state": state}
                for label, value, budget, state in self.gates
            ],
            "warnings": self.warnings,
            "failures": self.failures,
        }


def main() -> int:
    ap = argparse.ArgumentParser(
        description="Merge a MemBrowse report and a LabWired result.json into one verdict."
    )
    ap.add_argument(
        "--target", required=True, help="Target name, keyed into --budgets."
    )
    ap.add_argument(
        "--membrowse",
        required=True,
        type=Path,
        help="JSON from `membrowse report <elf> <ld> --json`.",
    )
    ap.add_argument(
        "--labwired",
        required=True,
        type=Path,
        help="result.json from `labwired test`.",
    )
    ap.add_argument("--budgets", type=Path, help="Combined budgets YAML.")
    ap.add_argument("--markdown", type=Path, help="Write the markdown report here.")
    ap.add_argument("--json", type=Path, help="Write the merged numbers here.")
    ap.add_argument(
        "--no-gate",
        action="store_true",
        help="Always exit 0; report without failing the build.",
    )
    args = ap.parse_args()

    report = Report(
        args.target,
        load_json(args.membrowse),
        load_json(args.labwired),
        load_budgets(args.budgets, args.target),
    )

    markdown = report.markdown()
    if args.markdown:
        args.markdown.parent.mkdir(parents=True, exist_ok=True)
        args.markdown.write_text(markdown + "\n")
    if args.json:
        args.json.parent.mkdir(parents=True, exist_ok=True)
        args.json.write_text(json.dumps(report.as_dict(), indent=2) + "\n")
    print(markdown)

    return 0 if (report.ok or args.no_gate) else 1


if __name__ == "__main__":
    raise SystemExit(main())
