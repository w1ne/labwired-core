# Final circuit runtime integration

This source combines the reviewed rail transaction and GPIO drive command fixes with main 5136a4dd. Declared receiver bounds admit only their explicitly bounded negative numerical zero; receiver state and traces retain the raw sample. MCU and device rail state is prepared completely before either receiver publishes. The drive command route requires a known driver and its paired enable; strict pad observation remains a separate input.

The internal regulator CPU admission function now inlines its empty-registry path. A nonempty registry still loads current sampled ownership with Acquire ordering. Its failure text and refusal are unchanged. The new behavioral test calls both single and bounded CPU advance before attachment, verifies unchanged cycles and observations, then attaches a complete sampled model to the same machine and verifies subsequent admission.

## Validation

The composed source passed 126 cases: config 4, routing 41, atomic publication 5, unchanged downcast ratchet 2, and adjacent integration cases 74. Targeted config Clippy passed. See [composition results](composition/results.json) and the adjacent logs with source hashes. Those receipts bind the source before the small admission optimization. The optimized source passed all 15 controlled-regulator session cases, including the new direct CPU seam regression. See [admission output](performance/admission.log.gz).

The paired performance measurement used Rust 1.95.0, the existing opt1 development profile with debug and incremental output disabled, event-scheduler, the unchanged board_perf.py fixture resolver, and identical firmware bytes. RA6 cost changed from 1244.5 to 1235.5 host instructions per simulated step. RA8 changed from 1242.6 to 1233.6. Each saves 9 instructions (about 0.72 percent). See the [paired comparison](performance/comparison.json). Release CI makes the acceptance decision. The [before](performance/before-status.json) and [after](performance/after-status.json) local reports correctly return failure against the release baseline of 892. ESP32 is unmeasured locally because its toolchain and exact fixture are absent; the completed prior CI artifact contains reports only. The final unchanged full CI remains authoritative.

Both frozen local CLI hashes, source hashes, exact commands, test output and original performance status are retained in this directory. Baseline hashes, ratchet limits, iteration limits and required checks remain unchanged.

## Scope

The generic source excludes the watched AT32/RP onboarding implementation and its human acknowledgment. The earlier native49 probes stopped on the rail and GPIO observation boundaries addressed here. Native50 execution, firmware startup, motor operation, flight behavior and hardware equivalence remain unverified by this bundle.

The later plugin-only main 974db2e merge is checked separately against identical Rust/config tree objects and the nine plugin contract cases before publication.
