#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Convert executable-test measurements into a bounded, provenance-bearing receipt."""
import argparse
import hashlib
import json
import math
from pathlib import Path
import re
import statistics


def report(log, commit, guest_source):
    payloads = re.findall(r'^MICROBIT_GUEST_SHA256=([0-9a-f]{64})$', log, re.MULTILINE)
    samples = [json.loads(line.removeprefix('MICROBIT_ACTIVE_SAMPLE '))
               for line in log.splitlines() if line.startswith('MICROBIT_ACTIVE_SAMPLE ')]
    if len(payloads) != 1 or len(samples) != 5 or not re.fullmatch(r'[0-9a-f]{40}', commit):
        raise ValueError('missing exact payload/commit provenance or five samples')
    for index, sample in enumerate(samples):
        if sample['pass'] != index + 1 or sample['cycles'] <= 0 or sample['wallSeconds'] <= 0:
            raise ValueError('invalid timing sample')
        calculated = sample['cycles'] / 64_000_000 / sample['wallSeconds']
        if not math.isfinite(calculated) or not math.isclose(calculated, sample['rtxAt64MHz'], rel_tol=1e-9):
            raise ValueError('RTx does not match simulated cycles and wall time')
        pixels = sample['pixels']
        if len(pixels) != 25 or any(type(pixel) is not int or not 0 <= pixel <= 255 for pixel in pixels):
            raise ValueError('invalid bounded framebuffer')
        if any((pixel > 0) != (offset // 5 == offset % 5) for offset, pixel in enumerate(pixels)):
            raise ValueError('guest did not drive the complete diagonal')
        if sample['guestButtonMask'] != (index % 2):
            raise ValueError('guest did not process button input')
    median = statistics.median(sample['rtxAt64MHz'] for sample in samples)
    return {'schema': 'labwired.microbit.active-board.v1', 'commit': commit,
            'firmwareSha256': payloads[0], 'guestSourceSha256': hashlib.sha256(guest_source).hexdigest(),
            'engine': 'native Rust event-scheduler', 'cpuHz': 64_000_000,
            'workload': 'guest five-row GPIO scan including silicon P1.05 and active-low button reads',
            'samples': samples, 'medianRtx': median, 'realtimeTargetMet': median >= 1,
            'limitations': ['no browser-WASM measurement', 'no sensor/audio qualification',
                            'functional cycle model, not silicon-cycle-accurate timing']}


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--log', required=True)
    parser.add_argument('--commit', required=True)
    parser.add_argument('--guest-source', default='examples/microbit-v2/board-io.S')
    parser.add_argument('--output', required=True)
    args = parser.parse_args()
    value = report(Path(args.log).read_text(), args.commit, Path(args.guest_source).read_bytes())
    Path(args.output).write_text(json.dumps(value, indent=2, sort_keys=True) + '\n')
    print('active-board median %.3fx; >=1x target met: %s' % (value['medianRtx'], value['realtimeTargetMet']))


if __name__ == '__main__':
    main()
