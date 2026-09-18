# Fidelity-preserving simulator throughput

Goal: real-time browser execution of the reported nRF54L15 Embassy firmware; maximum-throughput CLI defaults without changing clocks or losing events.

1. Reproduce with the deployed WASM and exact firmware; measure cycles, PCs, idle skips, and GPIO edges.
2. Correct WFE/SEV event sleep only after CPU regression tests fail. Preserve interrupt masks, exception return events, and scheduler deadlines. Compare acceleration enabled and disabled on the corrected CPU at identical simulated cycles.
3. Enable safe idle acceleration in the browser and CLI run path; retain explicit opt-out and existing observer gates. Keep browser real-time governor and CLI max-speed pacing.
4. Run CPU, scheduler and nRF regression tests. Benchmark real Chromium at real-time pace and native CLI unrestricted; verify GPIO cycle timestamps and state, then document measured limits.
