#!/usr/bin/env bash
# LabWired - Firmware Simulation Platform
# Copyright (C) 2026 Andrii Shylenko
#
# This software is released under the MIT License.
# See the LICENSE file in the project root for full license information.
#
# The firmware is committed (SDCC's pic14 port is not in distro packages);
# only the CLI is built here.
set -euo pipefail
ROOT="$(git rev-parse --show-toplevel)"
cd "$ROOT"
cargo build -q -p labwired-cli
