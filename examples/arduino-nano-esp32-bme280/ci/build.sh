#!/bin/sh
set -eu

repo_root=$(CDPATH='' cd -- "$(dirname -- "$0")/../../.." && pwd)
cd "$repo_root"
examples/arduino-nano-esp32-bme280/ci/build-bme280-firmware.sh
