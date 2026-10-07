# SPDX-License-Identifier: MIT
import copy
import json
import unittest
from microbit_active_report import report


class ReceiptTests(unittest.TestCase):
    def samples(self):
        return [{'pass': index + 1, 'cycles': 64_000_000, 'wallSeconds': 0.5,
                 'rtxAt64MHz': 2, 'guestButtonMask': index % 2,
                 'pixels': [255 if x == y else 0 for y in range(5) for x in range(5)]}
                for index in range(5)]

    def log(self, samples):
        return 'MICROBIT_GUEST_SHA256=' + 'a' * 64 + '\n' + '\n'.join(
            'MICROBIT_ACTIVE_SAMPLE ' + json.dumps(sample) for sample in samples)

    def test_valid_receipt(self):
        result = report(self.log(self.samples()), 'b' * 40, b'original source')
        self.assertEqual(result['medianRtx'], 2)
        self.assertTrue(result['realtimeTargetMet'])

    def test_slow_result_is_recorded_not_claimed_real_time(self):
        samples = self.samples()
        for sample in samples:
            sample.update(wallSeconds=2, rtxAt64MHz=0.5)
        self.assertFalse(report(self.log(samples), 'b' * 40, b'source')['realtimeTargetMet'])

    def test_bad_samples_rejected(self):
        for field, value in [('pass', 2), ('cycles', 0), ('wallSeconds', -1),
                             ('rtxAt64MHz', 9), ('guestButtonMask', 1), ('pixels', [0] * 25)]:
            samples = copy.deepcopy(self.samples())
            samples[0][field] = value
            with self.subTest(field=field), self.assertRaises(ValueError):
                report(self.log(samples), 'b' * 40, b'source')

    def test_provenance_and_sample_count_required(self):
        with self.assertRaises(ValueError):
            report(self.log(self.samples()[:4]), 'b' * 40, b'source')
        with self.assertRaises(ValueError):
            report(self.log(self.samples()), 'missing', b'source')


if __name__ == '__main__':
    unittest.main()
