#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/../../.."

# The RV32IMAC template ELF is committed because the RISC-V cross toolchain is
# not part of the standard board runner. Check it against the documented
# build digest before the CLI smoke consumes it.
sha256sum --check examples/weact-bluepill-plus-ch32v203/fixtures.sha256
