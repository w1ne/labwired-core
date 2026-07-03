#!/usr/bin/env bash
# Run the multi-chip station integration tests against the freshly built ELFs.
set -euo pipefail
ROOT="$(git rev-parse --show-toplevel)"
cd "$ROOT"
export LABWIRED_REQUIRE_IOLINK_ELFS=1
cargo test -p labwired-core --release --test world_multichip -- --nocapture
# Single-board iolink-dido lab: the NATIVE master must reach OPERATE with
# checksum-valid (green CK) cyclic frames against the real device firmware.
cargo test -p labwired-core --release --test e2e_iolink_dido -- --nocapture
# Full master-stack service coverage on the wire: ISDU read (vendor name),
# cyclic PD-output echo, event trigger/read, and data-storage round-trip.
cargo test -p labwired-core --release --test world_station_services -- --nocapture
