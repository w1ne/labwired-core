#!/usr/bin/env python3
# LabWired - Firmware Simulation Platform
# Copyright (C) 2026 Andrii Shylenko
#
# This software is released under the MIT License.
# See the LICENSE file in the project root for full license information.
"""Generate `examples/common/labwired_simctl.h` from the `simctl` model.

The register offsets exist in exactly one place — the `pub const`s in
`crates/core/src/peripherals/simctl.rs` — and this script copies them into the
firmware header. Nothing hand-edits the header, so it cannot drift from the
model; `simctl_header_is_generated.rs` fails if it has been.

    python3 tools/gen_simctl_header.py           # rewrite the header
    python3 tools/gen_simctl_header.py --check   # exit 1 if it is stale
"""

from __future__ import annotations

import argparse
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
MODEL = ROOT / "crates/core/src/peripherals/simctl.rs"
HEADER = ROOT / "examples/common/labwired_simctl.h"

# The board that ships the device — the default base the header bakes in.
BOARD = ROOT / "configs/systems/pico-selftest.yaml"


def parse_model() -> tuple[dict[str, int], int]:
    """Pull the register offsets and window size out of the Rust model."""
    source = MODEL.read_text()

    def const(name: str) -> int:
        match = re.search(rf"^pub const {name}: u64 = (0x[0-9A-Fa-f]+);", source, re.M)
        if not match:
            sys.exit(f"error: `pub const {name}` not found in {MODEL}")
        return int(match.group(1), 16)

    # REGISTERS is the model's own list, so a register added there appears here
    # without editing this script.
    block = re.search(r"pub const REGISTERS: &\[\(&str, u64\)\] = &\[(.*?)\];", source, re.S)
    if not block:
        sys.exit(f"error: `REGISTERS` table not found in {MODEL}")
    names = re.findall(r'\("([A-Z]+)",', block.group(1))
    if not names:
        sys.exit(f"error: `REGISTERS` in {MODEL} is empty")

    return {name: const(name) for name in names}, const("WINDOW")


def parse_board_base() -> int:
    """The base address the shipped board declares for the device."""
    text = BOARD.read_text()
    match = re.search(r"base_address:\s*(0x[0-9A-Fa-f]+)", text)
    if not match:
        sys.exit(f"error: no simctl base_address in {BOARD}")
    return int(match.group(1), 16)


def render(regs: dict[str, int], window: int, base: int) -> str:
    defines = "\n".join(
        f"#define LABWIRED_SIMCTL_{name} LABWIRED_SIMCTL_REG(0x{off:02x})"
        for name, off in regs.items()
    )
    return f"""/* LabWired - Firmware Simulation Platform
 * Copyright (C) 2026 Andrii Shylenko
 *
 * This software is released under the MIT License.
 * See the LICENSE file in the project root for full license information.
 *
 * GENERATED FILE — DO NOT EDIT.
 * Regenerate with:  python3 tools/gen_simctl_header.py
 * Source of truth:  crates/core/src/peripherals/simctl.rs
 *
 * labwired_simctl.h — let firmware end its own simulation run with a verdict.
 *
 * Without this, a test asserts on what appeared on a serial line: it proves the
 * expected characters were printed, not that the firmware passed. Writing to
 * this device ends the run carrying a structured exit code the harness reads.
 *
 *   #define LABWIRED_SIM
 *   #include "labwired_simctl.h"
 *
 *   int main(void) {{
 *       if (!self_test()) LABWIRED_FAIL();
 *       LABWIRED_PASS();
 *   }}
 *
 * THE DEVICE DOES NOT EXIST ON SILICON. Its window is mapped outside the chip's
 * real peripheral space, so a write reaches nothing (and on many parts faults)
 * on a physical board. Every macro below compiles to nothing unless
 * LABWIRED_SIM is defined, so the same source builds for hardware untouched.
 *
 * Requires a board that declares the device — see
 * `configs/systems/pico-selftest.yaml`. Override LABWIRED_SIMCTL_BASE to match
 * the base address your board declares.
 */

#ifndef LABWIRED_SIMCTL_H
#define LABWIRED_SIMCTL_H

#include <stdint.h>

#ifdef LABWIRED_SIM

/* Base address of the simctl window. Must match the board manifest. */
#ifndef LABWIRED_SIMCTL_BASE
#define LABWIRED_SIMCTL_BASE 0x{base:08x}u
#endif

/* Size of the register window, in bytes. */
#define LABWIRED_SIMCTL_WINDOW 0x{window:02x}u

#define LABWIRED_SIMCTL_REG(off) \\
    (*(volatile uint32_t *)((uintptr_t)LABWIRED_SIMCTL_BASE + (off)))

{defines}

/* End the run with `code`. 0 is a pass, by the same convention a process exit
 * status uses. Never returns. */
#define LABWIRED_EXIT(code)                            \\
    do {{                                               \\
        LABWIRED_SIMCTL_EXIT = (uint32_t)(code);       \\
        for (;;) {{ /* the run ends at the store */ }}   \\
    }} while (0)

/* The two cases worth naming. */
#define LABWIRED_PASS() LABWIRED_EXIT(0)
#define LABWIRED_FAIL() LABWIRED_EXIT(1)

/* Assert from inside firmware: fail the run with `code` if `cond` is false. */
#define LABWIRED_ASSERT(cond, code)  \\
    do {{                             \\
        if (!(cond)) {{               \\
            LABWIRED_EXIT(code);     \\
        }}                            \\
    }} while (0)

/* Host stdout / stderr, distinct from any UART the board has. */
#define LABWIRED_PUTC(c) (LABWIRED_SIMCTL_SOUT = (uint32_t)(uint8_t)(c))
#define LABWIRED_PUTC_ERR(c) (LABWIRED_SIMCTL_SERR = (uint32_t)(uint8_t)(c))

static inline void labwired_puts(const char *s)
{{
    while (*s) {{
        LABWIRED_PUTC(*s++);
    }}
}}

/* SCLK is a 64-bit register read as two 32-bit words. A read of the low word is
 * itself an MMIO access that advances simulated time, so the halves can
 * straddle a carry: read high, low, high again, and retry if it moved. */
static inline uint64_t labwired_simctl_read64(volatile uint32_t *lo)
{{
    uint32_t hi, low;
    do {{
        hi = lo[1];
        low = lo[0];
    }} while (hi != lo[1]);
    return ((uint64_t)hi << 32) | low;
}}

/* Simulated time, in CPU cycles.
 *
 * The 32-bit form wraps — at 125 MHz, every ~34 s of simulated time — so use it
 * only for short intervals and prefer the 64-bit form across a run. */
static inline uint32_t labwired_sim_cycles(void)
{{
    return LABWIRED_SIMCTL_SCLK;
}}

static inline uint64_t labwired_sim_cycles64(void)
{{
    return labwired_simctl_read64(&LABWIRED_SIMCTL_SCLK);
}}

#else /* !LABWIRED_SIM — building for real hardware. */

#define LABWIRED_EXIT(code) ((void)0)
#define LABWIRED_PASS() ((void)0)
#define LABWIRED_FAIL() ((void)0)
#define LABWIRED_ASSERT(cond, code) ((void)0)
#define LABWIRED_PUTC(c) ((void)0)
#define LABWIRED_PUTC_ERR(c) ((void)0)

static inline void labwired_puts(const char *s) {{ (void)s; }}
static inline uint32_t labwired_sim_cycles(void) {{ return 0; }}
static inline uint64_t labwired_sim_cycles64(void) {{ return 0; }}

#endif /* LABWIRED_SIM */

#endif /* LABWIRED_SIMCTL_H */
"""


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--check",
        action="store_true",
        help="exit non-zero if the header on disk is not what this script generates",
    )
    args = parser.parse_args()

    regs, window = parse_model()
    rendered = render(regs, window, parse_board_base())

    if args.check:
        current = HEADER.read_text() if HEADER.exists() else ""
        if current != rendered:
            print(
                f"error: {HEADER.relative_to(ROOT)} is stale.\n"
                "       Run: python3 tools/gen_simctl_header.py",
                file=sys.stderr,
            )
            return 1
        return 0

    HEADER.write_text(rendered)
    print(f"wrote {HEADER.relative_to(ROOT)} ({len(regs)} registers)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
