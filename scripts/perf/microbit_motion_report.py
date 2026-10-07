#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Validate selected-LSM303AGR guest measurements and emit a bounded receipt."""
import argparse
import hashlib
import json
import math
from pathlib import Path
import re
import statistics

SOURCE_FILES = ('board-io.S', 'motion-polled.inc', 'board-io.ld')
COMPILE_FLAGS = ('-mcpu=cortex-m4', '-mthumb', '-nostdlib', '-DMICROBIT_MOTION_IO', '-Wl,-T,board-io.ld')
POSES = (([1, -.5, .25], [30, -15, 7.5]),
         ([-.25, .125, -.75], [-30, 0, 60]))


def framed_bundle(sources, flags=COMPILE_FLAGS):
    """Length-frame names and payloads: concatenation boundaries are unambiguous."""
    result = bytearray(b'labwired.microbit.motion-source.v1\0')
    records = [(name, sources[name]) for name in SOURCE_FILES]
    records.append(('compileFlags', json.dumps(list(flags), separators=(',', ':')).encode()))
    for name, payload in records:
        encoded_name = name.encode()
        result.extend(len(encoded_name).to_bytes(8, 'big'))
        result.extend(encoded_name)
        result.extend(len(payload).to_bytes(8, 'big'))
        result.extend(payload)
    return bytes(result)


def source_bundle(repo_root):
    directory = Path(repo_root) / 'examples/microbit-v2'
    return framed_bundle({name: (directory / name).read_bytes() for name in SOURCE_FILES})


def finite_number(value):
    return type(value) in (int, float) and math.isfinite(value)


def half_away(value):
    return int(math.copysign(math.floor(abs(value) + .5), value))


def expected_raw(accel, mag):
    return ([max(-512, min(511, half_away(value / .0039))) * 64 for value in accel],
            [max(-32768, min(32767, half_away(value / .15))) for value in mag])


def report(log, commit, guest_source_bundle):
    hashes = re.findall(r'^MICROBIT_MOTION_GUEST_SHA256=([0-9a-f]{64})$', log, re.MULTILINE)
    lines = [line for line in log.splitlines() if line.startswith('MICROBIT_MOTION_SAMPLE ')]
    try:
        samples = [json.loads(line.removeprefix('MICROBIT_MOTION_SAMPLE ')) for line in lines]
    except json.JSONDecodeError as exc:
        raise ValueError('invalid measurement JSON') from exc
    hash_lines = [line for line in log.splitlines() if line.startswith('MICROBIT_MOTION_GUEST_SHA256=')]
    if len(hash_lines) != 1 or len(hashes) != 1 or len(samples) != 5 or not re.fullmatch(r'[0-9a-f]{40}', commit):
        raise ValueError('missing exact payload/commit provenance or five samples')
    previous_counts = {key: 0 for key in ('accelSamples', 'magSamples', 'scans')}
    for index, sample in enumerate(samples):
        try:
            if type(sample['pass']) is not int or sample['pass'] != index + 1:
                raise ValueError('invalid sample order')
            if (type(sample['cycles']) is not int or sample['cycles'] <= 0
                    or not finite_number(sample['wallSeconds']) or sample['wallSeconds'] <= 0
                    or not finite_number(sample['rtxAt64MHz'])):
                raise ValueError('invalid timing sample')
            calculated = sample['cycles'] / 64_000_000 / sample['wallSeconds']
            if not math.isfinite(calculated) or not math.isclose(calculated, sample['rtxAt64MHz'], rel_tol=1e-9):
                raise ValueError('RTx does not match simulated cycles and wall time')
            pixels = sample['pixels']
            if (not isinstance(pixels, list) or len(pixels) != 25
                    or any(type(p) is not int or not 0 <= p <= 255 for p in pixels)
                    or any((p > 0) != (offset // 5 == offset % 5) for offset, p in enumerate(pixels))):
                raise ValueError('guest did not drive a bounded complete diagonal')
            if type(sample['guestButtonMask']) is not int or sample['guestButtonMask'] != index % 2:
                raise ValueError('guest did not process alternating button input')
            accel, mag = POSES[index % 2]
            for key, expected in (('accelInput', accel), ('magInput', mag)):
                actual = sample[key]
                if (not isinstance(actual, list) or len(actual) != 3
                        or any(not finite_number(v) for v in actual) or actual != expected):
                    raise ValueError('unexpected physical input pose')
            for key, expected in zip(('accelRaw', 'magRaw'), expected_raw(accel, mag)):
                actual = sample[key]
                if (not isinstance(actual, list) or len(actual) != 3
                        or any(type(v) is not int for v in actual) or actual != expected):
                    raise ValueError('guest sensor conversion did not match the physical pose')
            for key in previous_counts:
                count = sample[key]
                if type(count) is not int or count <= previous_counts[key]:
                    raise ValueError('guest sampling or display stopped progressing')
                previous_counts[key] = count
            for key, expected in (('error', 0), ('accelDma', 0x60001), ('magDma', 0x60001)):
                if type(sample[key]) is not int or sample[key] != expected:
                    raise ValueError('guest sensor transport error or incorrect EasyDMA amount')
        except (KeyError, TypeError, OverflowError) as exc:
            raise ValueError('missing or malformed measurement fields') from exc
    speeds = [sample['rtxAt64MHz'] for sample in samples]
    median = statistics.median(speeds)
    return {'schema': 'labwired.microbit.motion-board.v1', 'commit': commit,
            'firmwareSha256': hashes[0],
            'guestSourceBundleSha256': hashlib.sha256(guest_source_bundle).hexdigest(),
            'guestSourceFiles': list(SOURCE_FILES), 'compileFlags': list(COMPILE_FLAGS),
            'engine': 'native Rust event-scheduler', 'cpuHz': 64_000_000,
            'workload': 'guest five-row GPIO scan, button reads, selected LSM303AGR polled TWIM0 EasyDMA samples',
            'samples': samples, 'medianRtx': median, 'minimumRtx': min(speeds),
            'realtimeTargetMet': median >= 1,
            'limitations': ['selected LSM303AGR variant only; held physical input poses',
                            'polled sensors; no shared sensor IRQ qualification',
                            'no ADC, microphone or speaker workload',
                            'no browser-WASM measurement',
                            'functional cycle model, not silicon-cycle-accurate timing; no silicon capture']}


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--log', required=True)
    parser.add_argument('--commit', required=True)
    parser.add_argument('--output', required=True)
    args = parser.parse_args()
    value = report(Path(args.log).read_text(), args.commit, source_bundle(Path(__file__).resolve().parents[2]))
    Path(args.output).write_text(json.dumps(value, indent=2, sort_keys=True) + '\n')
    print('motion-board median %.3fx; >=1x target met: %s' % (value['medianRtx'], value['realtimeTargetMet']))


if __name__ == '__main__':
    main()
