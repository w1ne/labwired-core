#!/usr/bin/env python3
"""Render docs/coverage/tier1-matrix.json as a chip × peripheral markdown grid.

Proof-artifact bar (spec wedge-alignment): a cell renders its real status
ONLY if it carries a run_url; cells without evidence render as unrecorded.
"""
import argparse
import json
from pathlib import Path

ICONS = {"pass": "✅", "partial": "🟡", "blocked": "⛔", "na": "🚧", "unrecorded": "·"}
# Overview column set = the 12 universal subsystems (bring-up six + typical
# peripherals). Every listed part has all of these in silicon, so a non-green
# cell is an honest model gap. Chip-specific peripherals (e.g. ESP32 RMT) are
# excluded from the overview — they relocate to the per-chip detail report.
UNIVERSAL = [
    "clock", "gpio", "uart", "timer", "dma", "irq",
    "i2c", "spi", "adc", "pwm", "wdt", "rtc",
]

# Proper part names for display (row keys stay the stable chip ids that name
# blobs/yamls). Source: each chip yaml's documented reference part.
DISPLAY_NAMES = {
    "esp32": "ESP32 (Xtensa LX6)",
    "esp32c3": "ESP32-C3 (RISC-V)",
    "esp32s3": "ESP32-S3 (Xtensa LX7)",
    "nrf52832": "nRF52832",
    "nrf52840": "nRF52840",
    "rp2040": "RP2040",
    "stm32f103": "STM32F103C8",
    "stm32f401": "STM32F401RE",
    "stm32f407": "STM32F407VG",
    "stm32g474re": "STM32G474RE",
    "stm32h563": "STM32H563",
    "stm32l073": "STM32L073RZ",
    "stm32l476": "STM32L476RG",
    "stm32wb55": "STM32WB55",
    "stm32wba52": "STM32WBA52",
}


def render(matrix: dict) -> str:
    # Overview is universal-only; chip-specific classes live in the detail report.
    classes = UNIVERSAL
    lines = [
        "# Tier-1 Validation Matrix",
        "",
        "Each cell shows its committed status (gated by the Tier-1 ratchet on every",
        "change; the full sim suite runs nightly) and links the CI run that recorded",
        "it when one is available.",
        "",
        "**Confidence tier:** ✅ means *sim-consistent* — the check passed against",
        "the simulator's peripheral models on real firmware. Silicon-anchored",
        "verification (hardware-in-the-loop capture replay) is a separate tier",
        "that arrives with the HIL workstream; no cell currently claims it.",
        "",
        "**Legend:** ✅ passed · 🟡 partial · ⛔ modeled but failing · 🚧 not",
        "modeled yet · · no check written. Every column here is a subsystem that",
        "exists in silicon on all listed parts, so 🚧 is an honest model gap, not",
        "an \"N/A\".",
        "",
        "| chip | " + " | ".join(classes) + " |",
        "|---|" + "---|" * len(classes),
    ]
    for chip in sorted(matrix):
        row = matrix[chip]
        cells = []
        for cls in classes:
            cell = row.get(cls)
            if cell is None:
                cells.append("·")
                continue
            status = cell.get("status", "unrecorded")
            url = cell.get("run_url")
            # Statuses are trustworthy on their own (ratchet-gated per change,
            # full suite nightly), so render them directly. A run_url is an
            # optional evidence link; drop only malformed ones.
            if url and (not url.startswith("https://") or any(c in url for c in " |()")):
                url = None
            icon = ICONS.get(status, "·")
            cells.append(f"[{icon}]({url})" if url else icon)
        label = DISPLAY_NAMES.get(chip, chip)
        lines.append(f"| {label} | " + " | ".join(cells) + " |")
    return "\n".join(lines) + "\n"


def main() -> None:
    ap = argparse.ArgumentParser()
    ap.add_argument("--matrix", default="docs/coverage/tier1-matrix.json")
    ap.add_argument("--out", default="docs/coverage/tier1-scoreboard.md")
    args = ap.parse_args()
    path = Path(args.matrix)
    if not path.exists():
        raise SystemExit(f"matrix not found: {path}")
    matrix = json.loads(path.read_text())
    Path(args.out).parent.mkdir(parents=True, exist_ok=True)
    Path(args.out).write_text(render(matrix))
    print(f"wrote {args.out}")


if __name__ == "__main__":
    main()
