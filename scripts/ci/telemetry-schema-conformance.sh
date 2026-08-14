#!/usr/bin/env bash
# The two failure-beacon producers in this repository must speak the vocabulary
# the endpoint accepts.
#
# `install.sh` and the CLI's panic hook each hand-write a JSON body, and the
# endpoint that receives it lives in another repository. A field renamed there,
# or a value invented here, breaks reporting in the quietest way available: the
# endpoint answers 400 to a caller that ignores the response, so the failure of
# the failure reporting is itself invisible. Nothing else in either repo can
# see that.
#
# So the vocabulary is fetched from the endpoint — `GET
# /v1/telemetry/failure/schema`, which is served from the same constant the
# validator is built from — and the producers are checked against it. Not a
# copy of the list kept here: a copy is the problem this is meant to catch.
#
# Usage: scripts/ci/telemetry-schema-conformance.sh [schema-url]
set -uo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$repo_root" || exit 1

SCHEMA_URL="${1:-${LABWIRED_TELEMETRY_SCHEMA_URL:-https://api.labwired.com/v1/telemetry/failure/schema}}"
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
failures=0

if ! curl -fsSL --retry 3 -m 20 "$SCHEMA_URL" -o "$work/schema.json"; then
  # A gate that cannot reach its oracle must not pass. The endpoint being down
  # is itself worth knowing: it is where every producer sends its failures.
  echo "::error::could not fetch the failure schema from ${SCHEMA_URL}"
  exit 1
fi

python3 - "$work/schema.json" scripts/install.sh crates/cli/src/crash_report.rs <<'PY'
import json, re, sys

schema = json.load(open(sys.argv[1]))
installer_src = open(sys.argv[2]).read()
cli_src = open(sys.argv[3]).read()
problems = []


def check(producer, field, value):
    """`value` must be a member of the schema set that `field` names."""
    allowed = schema[{
        'surface': 'surfaces',
        'event': 'events',
        'stage': 'stages',
        'error_class': 'error_classes',
        'channel': 'channels',
    }[field]]
    if value not in allowed:
        problems.append(f'{producer}: {field}="{value}" is not in the schema')


def check_keys(producer, keys):
    for key in keys:
        if key not in schema['body_keys']:
            problems.append(f'{producer}: sends "{key}", which the endpoint rejects the whole body for')


# ── install.sh ────────────────────────────────────────────────────────────────
# The beacon body is one quoted JSON string; read the keys out of it, and the
# literal values it fixes.
beacon = installer_src[installer_src.index('beacon() {'):installer_src.index('# `die <error_class>')]
check_keys('install.sh', re.findall(r'\\"([a-z_]+)\\":', beacon))
for field, value in re.findall(r'\\"(surface|event|channel)\\":\\"([a-z0-9._]+)\\"', beacon):
    check(f'install.sh', field, value)

# Every class the installer can pass to `die` has to be one the endpoint keeps
# rather than folding into `other`.
for value in re.findall(r'\n\s*die ([a-z_]+) ', installer_src):
    check('install.sh die', 'error_class', value)

# ── crates/cli/src/crash_report.rs ────────────────────────────────────────────
body = cli_src[cli_src.index('let body = serde_json::json!('):cli_src.index('let _ = ureq::post')]
check_keys('crash_report.rs', re.findall(r'"([a-z_]+)":', body))
for field, value in re.findall(r'"(surface|event|stage|channel|error_class)":\s*"([a-z0-9._]+)"', body):
    check('crash_report.rs', field, value)

if problems:
    print('\n'.join(f'::error::{p}' for p in problems))
    print(f'\n{len(problems)} producer field(s) the endpoint does not accept.')
    print('The schema is served from packages/api/src/telemetry-failure.ts in the labwired repo.')
    sys.exit(1)

print('install.sh and crash_report.rs send only what the endpoint accepts.')
PY
status=$?
[ "$status" -eq 0 ] || failures=1

exit "$failures"
