# SPDX-License-Identifier: MIT
import copy
import json
import unittest
from microbit_motion_report import COMPILE_FLAGS, POSES, SOURCE_FILES, expected_raw, framed_bundle, half_away, report


class MotionReceiptTests(unittest.TestCase):
    def samples(self):
        samples = []
        for index in range(5):
            accel, mag = POSES[index % 2]
            accel_raw, mag_raw = expected_raw(accel, mag)
            samples.append({'pass': index + 1, 'cycles': 64_000_000, 'wallSeconds': .5,
                            'rtxAt64MHz': 2, 'guestButtonMask': index % 2,
                            'pixels': [255 if x == y else 0 for y in range(5) for x in range(5)],
                            'accelInput': accel.copy(), 'magInput': mag.copy(),
                            'accelRaw': accel_raw, 'magRaw': mag_raw,
                            'accelSamples': index + 1, 'magSamples': index + 1, 'scans': index + 1,
                            'error': 0, 'accelDma': 0x60001, 'magDma': 0x60001})
        return samples

    def log(self, samples):
        return 'MICROBIT_MOTION_GUEST_SHA256=' + 'a' * 64 + '\n' + '\n'.join(
            'MICROBIT_MOTION_SAMPLE ' + json.dumps(sample) for sample in samples)

    def test_valid_receipt(self):
        receipt = report(self.log(self.samples()), 'b' * 40, b'framed source')
        self.assertTrue(receipt['realtimeTargetMet'])
        self.assertEqual(receipt['medianRtx'], 2)
        self.assertEqual(receipt['minimumRtx'], 2)
        self.assertEqual(receipt['guestSourceFiles'], list(SOURCE_FILES))
        self.assertEqual(receipt['compileFlags'], list(COMPILE_FLAGS))

    def test_slow_result_is_not_claimed_realtime(self):
        samples = self.samples()
        for sample in samples:
            sample.update(wallSeconds=2, rtxAt64MHz=.5)
        self.assertFalse(report(self.log(samples), 'b' * 40, b'source')['realtimeTargetMet'])

    def test_timing_frame_pose_transport_and_counts_are_enforced(self):
        mutations = [('pass', 2), ('cycles', 0), ('wallSeconds', -1), ('wallSeconds', float('nan')),
                     ('rtxAt64MHz', 9), ('rtxAt64MHz', float('inf')), ('guestButtonMask', 1),
                     ('pixels', [0] * 25), ('pixels', [True] * 25),
                     ('accelInput', [0, 0, 0]), ('magInput', [30, -15, 8]),
                     ('accelRaw', [0, 0, 0]), ('magRaw', [200, -100, 51]),
                     ('accelSamples', 0), ('magSamples', 0), ('scans', 0),
                     ('error', 1), ('accelDma', 0x50001), ('magDma', 0x60000),
                     ('error', False), ('cycles', True), ('accelRaw', [True, 0, 0])]
        for key, value in mutations:
            samples = copy.deepcopy(self.samples())
            samples[0][key] = value
            with self.subTest(key=key, value=value), self.assertRaises(ValueError):
                report(self.log(samples), 'b' * 40, b'source')

    def test_counts_must_increase_between_samples(self):
        for key in ('accelSamples', 'magSamples', 'scans'):
            samples = self.samples()
            samples[3][key] = samples[2][key]
            with self.subTest(key=key), self.assertRaises(ValueError):
                report(self.log(samples), 'b' * 40, b'source')

    def test_missing_fields_or_malformed_samples_rejected(self):
        for value in (None, [], 'bad', {}):
            samples = self.samples()
            samples[0] = value
            with self.subTest(value=value), self.assertRaises(ValueError):
                report(self.log(samples), 'b' * 40, b'source')
        samples = self.samples()
        del samples[0]['error']
        with self.assertRaises(ValueError):
            report(self.log(samples), 'b' * 40, b'source')

    def test_exact_provenance_and_sample_count_required(self):
        log = self.log(self.samples())
        for candidate, commit in ((self.log(self.samples()[:4]), 'b' * 40),
                                  (log + '\nMICROBIT_MOTION_GUEST_SHA256=' + 'a' * 64, 'b' * 40),
                                  (log + '\nMICROBIT_MOTION_GUEST_SHA256=bad', 'b' * 40),
                                  (log.replace('a' * 64, 'missing'), 'b' * 40),
                                  (log, 'missing'), (log + '\nMICROBIT_MOTION_SAMPLE {', 'b' * 40)):
            with self.subTest(commit=commit), self.assertRaises(ValueError):
                report(candidate, commit, b'source')

    def test_raw_rounding_is_half_away_and_clamped(self):
        self.assertEqual([half_away(v) for v in (1.5, -1.5, .5, -.5)], [2, -2, 1, -1])
        accel, mag = expected_raw([100, -100, 0], [1e9, -1e9, 0])
        self.assertEqual(accel, [511 * 64, -512 * 64, 0])
        self.assertEqual(mag, [32767, -32768, 0])

    def test_source_bundle_frames_every_source_and_flags(self):
        sources = dict(zip(SOURCE_FILES, (b'ab', b'c', b'd')))
        original = framed_bundle(sources)
        for key in SOURCE_FILES:
            changed = sources.copy()
            changed[key] += b'x'
            self.assertNotEqual(original, framed_bundle(changed))
        same_concatenation = dict(zip(SOURCE_FILES, (b'a', b'bc', b'd')))
        self.assertNotEqual(original, framed_bundle(same_concatenation))
        self.assertNotEqual(original, framed_bundle(sources, COMPILE_FLAGS + ('-O2',)))


if __name__ == '__main__':
    unittest.main()
