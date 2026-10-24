#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Reuse only an official successful guest execution with unchanged inputs."""
import argparse
import hashlib
import io
import json
import os
from pathlib import Path
import re
import subprocess
import zipfile

REPO = 'CrispStrobe/labwired-core'
FIXTURE = '592748e51e6b82238d9c1d6a69ffa4e74ccc4ce1'
WORKFLOW = '.github/workflows/samd-transfer-state-guest.yml'
ALLOWED_DOCS = {'docs/testing/IGNORED_TESTS.md', 'scripts/samd-transfer-state/README.md'}
POSITIVE = 'bd539fda7f60b37b82e9e47b96180dff2208718d9e1da8057c5a8a82408ab2d3'
NEGATIVE = 'f2191da7b4214aff866aad4c863b476483f720ff9b1550f1c54c5103a17a3949'


def command(*args):
    return subprocess.check_output(args)


def api(path):
    return json.loads(command('gh', 'api', f'repos/{REPO}/{path}'))


def admissible(record, run, commit, changes):
    return (
        record.get('schema') == 1 and record.get('fixture') == FIXTURE
        and record.get('run') == run['id'] and record.get('attempt') == 1
        and run.get('run_attempt') == 1 and run.get('conclusion') == 'success'
        and run.get('event') == 'pull_request' and run.get('path') == WORKFLOW
        and record.get('commit') == commit['sha']
        and record.get('tree') == commit['commit']['tree']['sha']
        and len(commit.get('parents', [])) == 2
        and commit['parents'][1]['sha'] == run['head_sha']
        and record.get('head') == run['head_sha']
        and all(path in ALLOWED_DOCS for path in changes)
    )


def evidence(raw):
    assert len(raw) <= 1024 * 1024
    with zipfile.ZipFile(io.BytesIO(raw)) as archive:
        names = archive.namelist()
        assert len(names) == len(set(names)) and len(names) <= 12
        assert sum(i.file_size for i in archive.infolist()) <= 4 * 1024 * 1024
        assert all(not n.startswith('/') and '..' not in Path(n).parts for n in names)
        records = [n for n in names if n.endswith('/qualification.json') or n == 'qualification.json']
        assert len(records) == 1
        for suffix in ['state-guest-default.log', 'state-guest-scheduler.log']:
            selected = [n for n in names if n.endswith(suffix)]
            assert len(selected) == 1
            log = archive.read(selected[0]).decode()
            results = re.findall(r'^SAMD_TRANSFER_STATE case=(\S+) elf_sha256=(\w+) status=(\S+) steps=(\d+) bss_clear=true entry=true$', log, re.M)
            assert results == [('positive', POSITIVE, '0x600d', '1425'),
                               ('premature-success', NEGATIVE, '0x4', '302')]
            assert '1 passed; 0 failed; 0 ignored' in log
        return json.loads(archive.read(records[0]))


def select():
    current_run = int(os.environ['GITHUB_RUN_ID'])
    runs = api('actions/workflows/samd-transfer-state-guest.yml/runs?status=success&per_page=10')['workflow_runs']
    for run in runs:
        if run['id'] == current_run or run['event'] != 'pull_request':
            continue
        artifacts = api(f"actions/runs/{run['id']}/artifacts")['artifacts']
        found = [a for a in artifacts if a['name'] == 'authored-state-guest-results' and not a['expired'] and a['size_in_bytes'] <= 1024 * 1024]
        if len(found) != 1:
            continue
        raw = command('gh', 'api', f"repos/{REPO}/actions/artifacts/{found[0]['id']}/zip")
        assert found[0]['digest'] == 'sha256:' + hashlib.sha256(raw).hexdigest()
        try:
            record = evidence(raw)
        except (AssertionError, ValueError, KeyError, zipfile.BadZipFile):
            continue  # Old or reused artifacts are not fresh qualification.
        assert all(re.fullmatch('[0-9a-f]{40}', record[key]) for key in ['commit', 'head', 'tree'])
        commit = api(f"commits/{record['commit']}")
        command('git', 'fetch', '--no-tags', 'origin', record['commit'])
        assert command('git', 'rev-parse', record['commit'] + '^{tree}').decode().strip() == record['tree']
        changes = command('git', 'diff', '--name-only', '-z', record['commit'], 'HEAD').decode().split('\0')
        changes = [path for path in changes if path]
        jobs = api(f"actions/runs/{run['id']}/jobs")['jobs']
        if not (len(jobs) == 1 and jobs[0]['name'] == 'guest' and jobs[0]['conclusion'] == 'success'
                and all(s['conclusion'] == 'success' for s in jobs[0]['steps'])):
            continue
        if admissible(record, run, commit, changes):
            return {'reuse': True, 'run': run['id'], 'artifact': found[0]['id'],
                    'qualifiedCommit': record['commit'], 'currentCommit': command('git', 'rev-parse', 'HEAD').decode().strip(),
                    'changedDocumentation': changes, 'artifactDigest': found[0]['digest']}
    return {'reuse': False, 'reason': 'No official fresh qualification with unchanged non-documentation inputs'}


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--record', action='store_true')
    args = parser.parse_args()
    if args.record:
        data = {'schema': 1, 'fixture': FIXTURE, 'run': int(os.environ['GITHUB_RUN_ID']),
                'attempt': int(os.environ['GITHUB_RUN_ATTEMPT']), 'head': os.environ['QUALIFICATION_HEAD'],
                'commit': command('git', 'rev-parse', 'HEAD').decode().strip(),
                'tree': command('git', 'rev-parse', 'HEAD^{tree}').decode().strip()}
        Path('qualification.json').write_text(json.dumps(data, sort_keys=True) + '\n')
    else:
        data = select()
        Path('qualification-reuse.json').write_text(json.dumps(data, sort_keys=True) + '\n')
        with open(os.environ['GITHUB_OUTPUT'], 'a') as output:
            output.write('reuse=' + str(data['reuse']).lower() + '\n')
        print(json.dumps(data, sort_keys=True))


if __name__ == '__main__':
    main()
