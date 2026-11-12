# Generic quadcopter runtime follow-up — 2026-11-12

The generic candidate keeps driver commands separate from live pad observations, publishes declared MCU/device rail batches atomically, and removes rare reset bodies from the per-instruction hot path. The original follow-ups are `12fec73c211ddb3b4e0cec4d1241f42d8b4ecb54` and `65efd6fd683e166169f23ccb7b47a153f681f929`; their original evidence and authorship are retained.

## Correctness

The final cold-reset source passed 15 regulator-session, 6 machine-power and 5 reset-input integration tests. The existing peripheral reset request test also passed when replayed against the current compiled library. That test verifies vector-table PC/MSP reload and the watchdog reset cause. The small harness avoids the local monolithic unit-test memory failure; the ordinary repository hooks and full CI remain required.

Earlier component runs passed 89 integration tests, 41 routing tests and 5 atomic rail unit tests. Those earlier receipts precede the cold-reset extraction and are not presented as a fresh rerun of the final source. No rail or driver-command implementation changed afterward.

## Performance

The first supply-boundary experiment failed the fixed ARM budgets and was reverted. The final source keeps both regulator preconditions in their original positions and moves only the existing SCB/peripheral reset bodies to cold helpers. Request checks, reset order, error propagation and cause publication remain intact.

| Board | Step Ir/step | Step baseline | Delta | Batch Ir/step | Instructions/batch |
|---|---:|---:|---:|---:|---:|
| r7fa6m5bh | 911.4 | 892.0 | +2.2% | 1.6 | 1023.9 |
| r7fa8m1ah | 909.5 | 892.0 | +2.0% | 1.5 | 1023.9 |

The unchanged official gate passed both modes. Batch differences remain below its existing 0.5 Ir/step absolute floor. Full supported-board CI, including ESP32, remains required. No baseline, tolerance or run limit was changed.

`receipt.json` records exact source/binary hashes and build settings. Compressed logs and the official machine-readable performance results are alongside it. This packet establishes no native firmware startup, five-MCU closed loop, hardware equivalence or flight readiness.
