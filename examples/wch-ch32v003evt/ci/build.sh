#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/../../.."

# These compact RV32E/EC ELFs are committed because the exact cross toolchain
# is not part of the standard board runner. Check both source-built fixtures
# against the documented build digests before the CLI smoke consumes them.
sha256sum --check examples/wch-ch32v003evt/fixtures.sha256
