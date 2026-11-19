#!/bin/sh
set -eu

repo_root=$(CDPATH='' cd -- "$(dirname -- "$0")/../../.." && pwd)
cd "$repo_root"
examples/adafruit-feather-esp32-v2/verify-fixture.sh
examples/adafruit-feather-esp32-v2/verify-bme280-fixture.sh
