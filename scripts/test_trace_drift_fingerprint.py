#!/usr/bin/env python3
import unittest

from trace_drift_fingerprint import fingerprint_from_objects


class TraceDriftFingerprintTest(unittest.TestCase):
    def test_fingerprint_tracks_peripheral_state(self):
        result = {
            "status": "ok",
            "stop_reason": "uart_match",
            "steps_executed": 42,
            "assertions": [{"kind": "uart_contains", "passed": True}],
        }
        snapshot_a = {
            "cpu": {"pc": 4, "registers": {"r0": 1}},
            "peripherals": {"uart1": {"tx": "OK\n"}},
        }
        snapshot_b = {
            "cpu": {"pc": 4, "registers": {"r0": 1}},
            "peripherals": {"uart1": {"tx": "NO\n"}},
        }

        self.assertNotEqual(
            fingerprint_from_objects(result, snapshot_a, "OK\n"),
            fingerprint_from_objects(result, snapshot_b, "OK\n"),
        )

    def test_fingerprint_ignores_cpu_and_cycle_accounting(self):
        result_a = {
            "status": "ok",
            "stop_reason": "uart_match",
            "steps_executed": 42,
            "cycles": 100,
            "instructions": 20,
            "limits": {"max_steps": 1000},
            "assertions": [{"kind": "uart_contains", "passed": True}],
            "stop_reason_details": {"pc": 4},
        }
        result_b = {
            **result_a,
            "cycles": 250,
            "instructions": 45,
            "limits": {"max_steps": 2000},
            "stop_reason_details": {"pc": 8},
        }
        snapshot_a = {
            "cpu": {"pc": 4, "registers": {"r0": 1}},
            "peripherals": {"uart1": {"tx": "OK\n"}},
        }
        snapshot_b = {
            "cpu": {"pc": 8, "registers": {"r0": 9}},
            "peripherals": {"uart1": {"tx": "OK\n"}},
        }

        self.assertEqual(
            fingerprint_from_objects(result_a, snapshot_a, "OK\n"),
            fingerprint_from_objects(result_b, snapshot_b, "OK\n"),
        )


if __name__ == "__main__":
    unittest.main()
