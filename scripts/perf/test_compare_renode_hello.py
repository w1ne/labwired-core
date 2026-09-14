# LabWired - Firmware Simulation Platform
# Copyright (C) 2026 Andrii Shylenko
# SPDX-License-Identifier: MIT
"""Unit tests for the Renode hello wall-time gate (no engines required)."""
from __future__ import annotations

import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

import compare_renode_hello as c  # noqa: E402


def test_verdict_labwired_faster_passes():
    assert c.verdict(0.20, 3.00, slack=0.20) is True


def test_verdict_equal_passes():
    assert c.verdict(1.00, 1.00, slack=0.20) is True


def test_verdict_within_slack_passes():
    assert c.verdict(1.19, 1.00, slack=0.20) is True


def test_verdict_slower_than_slack_fails():
    assert c.verdict(1.21, 1.00, slack=0.20) is False


def test_verdict_rejects_non_positive_renode():
    assert c.verdict(0.2, 0.0, slack=0.20) is False


def test_pinned_elf_sha_constant_is_64_hex():
    assert len(c.ELF_SHA256) == 64
    int(c.ELF_SHA256, 16)


def test_committed_elf_matches_pin():
    assert c.ELF.is_file(), f"missing {c.ELF}"
    assert c.sha256_file(c.ELF) == c.ELF_SHA256
