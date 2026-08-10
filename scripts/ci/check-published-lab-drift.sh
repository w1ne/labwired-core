#!/usr/bin/env bash
# Fail if the owner's LIVE published lab no longer matches the sketch this repo
# tests against.
#
# WHY THIS EXISTS
# ---------------
# `crates/core/tests/world_esp32c3_ble_pong.rs` boots two ESP32-C3 instances on
# a COMMITTED flash image and claims that proves "a real user's real sketch
# works". CI has no way to compile from the live project, so the sketch is
# frozen in-tree (crates/core/tests/fixtures/esp32c3-ble-pong.ino) and pinned by
# hash (crates/core/tests/fixture_ble_pong_provenance.rs).
#
# A pinned hash keeps the two committed files honest with EACH OTHER. It cannot
# notice the third party: the owner editing the published project. That is
# exactly what happened — the fixture stayed at the labwired-core #828 build
# while the published sketch moved on twice (silicon-validated strap/TAG
# identity, `setWindow(30)` + `PUBLISH_MS 700`, then a dual-host election fix).
# Three gates stayed green while both OLEDs went black in the browser, because
# not one of them ever looked at the live project. This script is the lane that
# looks.
#
# WHAT IT ASSERTS
#   1. The live project still resolves and is still readable.
#   2. `source_code` and `firmware[0].source_code` are byte-identical — the
#      lab's whole premise is ONE image flashed to both chips, so if those two
#      ever diverge the two-node test is modelling something the project no
#      longer is.
#   3. `source_code` is byte-identical to the committed .ino.
#
# IT FAILS CLOSED, ON PURPOSE. A drift watcher that treats "could not reach the
# API" as "no drift" reports fresh forever and is worse than absent. Network
# failure and real drift both exit non-zero; the messages say which.
#
# Usage: scripts/ci/check-published-lab-drift.sh [sketch-path] [project-id]
# Env:   LABWIRED_API_BASE   override the API (default: production)
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
repo="$(cd "$here/../.." && pwd)"

sketch="${1:-$repo/crates/core/tests/fixtures/esp32c3-ble-pong.ino}"
project_id="${2:-c477f82961e86f601e7b908ae7e12311}"
api="${LABWIRED_API_BASE:-https://api.labwired.com}"
url="$api/v1/projects/$project_id"

[ -f "$sketch" ] || { echo "missing committed sketch: $sketch" >&2; exit 1; }

command -v python3 >/dev/null 2>&1 || {
  echo "python3 not on PATH — cannot parse the project JSON, refusing to run" >&2
  exit 1
}

# Portable across BSD (macOS) and GNU mktemp: BSD -t takes a PREFIX, GNU -t
# wants a TEMPLATE with at least three X's and rejects one without them
# ("too few X's in template"), which is exactly how this failed in CI while
# passing locally. Give an explicit path template and skip -t entirely.
tmp="$(mktemp "${TMPDIR:-/tmp}/published-lab-drift.XXXXXX")"
trap 'rm -f "$tmp"' EXIT

# The project is PUBLIC, so this needs no credentials. Keep it that way: a
# drift check that depends on a token starts failing for token reasons and gets
# muted, which is how watchers die.
#
# The explicit User-Agent is not cosmetic. Cloudflare's WAF 403s clients whose
# UA looks automated before the Worker ever sees the request — the superproject's
# packages/api/scripts/verify-live-api.mjs carries the same workaround for the
# same reason. Without it this check fails in CI while passing on a laptop.
code="$(curl -sS -L --retry 3 --retry-connrefused -m 60 \
  -A 'labwired-deploy-verifier/1' \
  -o "$tmp" -w '%{http_code}' "$url" || echo 000)"
if [ "$code" != "200" ]; then
  echo "FETCH FAILED  $url  (HTTP $code)" >&2
  echo "  This is NOT a pass. Either the API is unreachable, or the project was" >&2
  echo "  unpublished / made private / deleted. If the lab is intentionally gone," >&2
  echo "  delete this check and the fixture it guards in the same change — do not" >&2
  echo "  leave a world test claiming to cover a lab that no longer exists." >&2
  exit 1
fi

SKETCH_PATH="$sketch" PROJECT_ID="$project_id" URL="$url" python3 - "$tmp" <<'PY'
import hashlib, json, os, sys

raw = open(sys.argv[1], "rb").read()
try:
    project = json.loads(raw)["project"]
except Exception as exc:  # noqa: BLE001 - any shape change must be loud
    sys.exit(f"UNPARSEABLE PROJECT JSON from {os.environ['URL']}: {exc}")

live = project.get("source_code")
if not isinstance(live, str) or not live:
    sys.exit(f"project {os.environ['PROJECT_ID']} has no source_code — the API shape changed.")
live_bytes = live.encode("utf-8")

# The lab flashes ONE image to both chips. If the per-chip firmware entry ever
# carries different text, the two-node test's premise is dead and no hash
# comparison downstream means anything.
firmware = project.get("firmware") or []
if not firmware:
    sys.exit(
        "project has no firmware[] entry — the second MCU lost its sketch.\n"
        "The two-node test boots ONE image on both chips; that is no longer what the project is."
    )
peer = firmware[0].get("source_code")
peer_bytes = (peer or "").encode("utf-8")
if peer_bytes != live_bytes:
    sys.exit(
        "LIVE PROJECT IS NO LONGER ONE IMAGE ON BOTH CHIPS.\n"
        f"  source_code          sha256 {hashlib.sha256(live_bytes).hexdigest()}  ({len(live_bytes)} bytes)\n"
        f"  firmware[0].source_code sha256 {hashlib.sha256(peer_bytes).hexdigest()}  ({len(peer_bytes)} bytes)\n"
        "world_esp32c3_ble_pong builds BOTH nodes from a single flash image. Either the\n"
        "project regressed, or the test needs to become a two-image world — decide which,\n"
        "do not repin."
    )

committed_bytes = open(os.environ["SKETCH_PATH"], "rb").read()
live_sha = hashlib.sha256(live_bytes).hexdigest()
committed_sha = hashlib.sha256(committed_bytes).hexdigest()

if live_sha != committed_sha:
    rel = "crates/core/tests/fixtures/esp32c3-ble-pong.ino"
    sys.exit(
        "PUBLISHED LAB HAS DRIFTED FROM THE COMMITTED FIXTURE.\n"
        f"  live      sha256 {live_sha}  ({len(live_bytes)} bytes)  {os.environ['URL']}\n"
        f"  committed sha256 {committed_sha}  ({len(committed_bytes)} bytes)  {rel}\n"
        "\n"
        "world_esp32c3_ble_pong is now testing firmware nobody runs. Refresh the pair:\n"
        f"  1. Write the live source_code to {rel}\n"
        "  2. Recompile it (labwired_compile, board `esp32-c3-supermini`, language\n"
        "     `arduino`, lib_deps `adafruit/Adafruit SSD1306` + `adafruit/Adafruit GFX\n"
        "     Library`) and concatenate the flash images at their offsets, 0xFF padding,\n"
        "     into crates/core/tests/fixtures/esp32c3-ble-pong-flash.bin\n"
        "  3. Update SKETCH_SHA256 and FLASH_SHA256 in\n"
        "     crates/core/tests/fixture_ble_pong_provenance.rs\n"
        "  4. Re-run the world test and FIX IT IF IT FAILS. A red world test here means\n"
        "     the published lab is broken for real users — that is the finding, not a\n"
        "     chore blocking the refresh."
    )

print(f"ok  published lab matches the committed sketch  sha256 {live_sha}  ({len(live_bytes)} bytes)")
print("ok  source_code and firmware[0].source_code are byte-identical (one image, both chips)")
PY
