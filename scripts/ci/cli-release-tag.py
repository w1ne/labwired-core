#!/usr/bin/env python3
"""Select a stable CLI semver release from GitHub's release list (no network)."""
import json
import re
import sys

def select_cli_release(releases):
    if not isinstance(releases, list):
        raise ValueError('expected GitHub release array')
    candidates = []
    for release in releases:
        if not isinstance(release, dict) or release.get('draft') or release.get('prerelease'):
            continue
        tag = release.get('tag_name')
        match = re.fullmatch(r'v(\d+)\.(\d+)\.(\d+)', tag) if isinstance(tag, str) else None
        if match:
            candidates.append((tuple(map(int, match.groups())), tag))
    if not candidates:
        raise ValueError('no stable CLI semver release in returned release list')
    return max(candidates)[1]

if __name__ == '__main__':
    try:
        print(select_cli_release(json.load(sys.stdin)))
    except (ValueError, TypeError) as error:
        sys.exit('CLI release selection failed: ' + str(error))
