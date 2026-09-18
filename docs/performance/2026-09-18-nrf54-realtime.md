# nRF54L15 event sleep and real-time browser pacing

The imported Feather demo uses Embassy's Cortex-M thread executor. Its idle
instruction is WFE, which the engine previously decoded as NOP. Enabling idle
acceleration alone could not help: the CPU never entered architectural sleep.

WFE and SEV now have an event latch. A sleeping WFE resumes for an eligible
exception or an event; SEVONPEND records a new pending transition even for an
NVIC-disabled IRQ. PRIMASK, BASEPRI, FAULTMASK, active priority, cleared pending
bits and disabled external lines are respected. Exception return records an
event. Narrow and wide encodings behave identically. Compiled backends interpret
the wake boundary before executing code after WFE.

Reference execution also parks at WFE, advancing devices one cycle at a time.
Acceleration coalesces only these idle cycles, bounded by the scheduler's next
event and the existing capture, breakpoint and peripheral safety gates. CPU and
GRTC frequencies remain 128 MHz and 1 MHz. This preserves the existing device
model; it is not a claim of new silicon-level instruction timing accuracy.

Normal ARM CLI runs now use the batched Machine lifecycle, with the event
scheduler included in the default build and idle acceleration enabled. Host
pacing remains `max-speed`. `--time-mode realtime` explicitly requests pacing.
Instrumented runs retain their single-step path. `LABWIRED_ARM_SINGLE_STEP=1`
selects that reference path; `LABWIRED_IDLE_FAST_FORWARD=0` disables idle skips.
`LABWIRED_RUN_STATS=1` prints fuel and skipped-cycle counts. Each coalesced idle
cycle consumes one unit of `--max-steps`, matching Machine's fuel contract.

The companion app enables nRF54L15 idle acceleration in the actual worker init
payload. Browser worker configurations inherit the board-clock governor instead
of overriding it with turbo pacing. No timer rate is changed to fake real time.

The nRF54 hosted builder also needs `time-driver-grtc` and `tick-hz-1_000_000`.
Its older RTC1 / 32.768 kHz configuration made `after_millis(250)` request 8,192
GRTC ticks. The shipped demo worked around that with raw 250,000-tick waits;
the corrected configuration lets it use milliseconds again. Both the original
misconfigured ELF and a corrected GRTC ELF are retained as regression inputs.

## Validation commands

```sh
cargo test --release -p labwired-core --features event-scheduler \
  --test cortex_m_event_sleep --test nrf54l15_embassy_realtime \
  --test nrf54l15_grtc_walk_differential --test nrf54l15_idle_ff_speedup
cargo test --release -p labwired-cli --test arm_batched_path --test tier1_matrix_ratchet
cargo clippy -p labwired-core -p labwired-cli -p labwired-wasm --all-targets \
  --features labwired-core/event-scheduler -- -D warnings
```

The Embassy differential runs acceleration on/off to the same 65 million cycle
boundary and compares every captured GPIO edge, CPU snapshot, SRAM and
GRTC/GPIO/NVIC/SCB snapshots. The corrected image must have three edges and
250 ms intervals plus normal interrupt/executor instruction latency.

The app's `scripts/benchmark-nrf54-realtime.mjs` runs the release WASM and actual
RealtimeGovernor in Chromium for three seconds. It requires 0.95–1.05 simulated
seconds per wall second, at least ten LED edges, and the correct simulated edge
intervals. This harness measures the engine and governor, not signed-in UI or
physical hardware.

Architecture reference: [Arm Cortex-M4 Devices Generic User Guide, WFE and SEV](https://documentation-service.arm.com/static/5f2ac4ab60a93e65927bbdbf).

## Measured native equivalence (2026-09-18)

At 65,000,000 CPU cycles (507.8125 ms at 128 MHz), the original ELF took
62.631 s without acceleration and 41.490 ms with it; 64,959,242 cycles were
coalesced. The corrected GRTC ELF took 28.112 s and 2.042 ms respectively;
64,997,354 cycles were coalesced. These are execution-loop timings, excluding
ELF/config loading, on a host with concurrent builds.

The corrected ELF's P2.06 edges were exactly `(1152, high)`,
`(32001475, low)`, `(64001731, high)` in both runs. Full assertions for both
ELFs passed, including CPU, SRAM and peripheral state.

## Chromium measurement

The release WASM plus production RealtimeGovernor advanced 381,004,800 cycles
(2,976.6 ms simulated) in 3,002.1 ms wall time: **0.991506x real time**.
It captured 12 P2.06 edges with the same exact timestamps as native execution,
then 32,000,256 cycles between subsequent edges (250.002 ms).
380,996,793 idle cycles were coalesced. The benchmark's speed and interval
assertions passed. This first browser run used release WASM before wasm-opt.

Repeating with the deployment optimizer flags (`wasm-opt -O3 --strip-debug`
plus the project's enabled WASM features) passed too: 380,812,799 cycles
(2,975.100 ms simulated) in 3,000.2 ms wall, **0.991634x real time**, with
the same 12 exact GPIO edge cycles.

## Actual worker and default CLI

The production WorkerSimClient → SimHost → WASM path, with normal inspector and
snapshot settings, advanced 382,988,800 cycles in 3,001.4 ms: **0.996901x real
time**. All 12 GPIO edges matched the native reference timestamps and values;
no worker errors occurred. This additionally caught and fixed a pacing-clock
fallback: chip-only `cpu_hz` must supply the governor clock when the system YAML
has no override. An unknown pacing clock had selected turbo despite the correct
engine clock. The app's `scripts/benchmark-nrf54-worker.mjs` reproduces this check
against the built UI worker and optimized public WASM.

The default release CLI command (no `--batched` or pacing flags), with
`LABWIRED_RUN_STATS=1`, simulated 1,280,000,000 cycles (10 seconds) in **0.26 s
wall time**, including process startup/config/ELF loading: about **38x real
time**. It executed 24,680 instructions and coalesced 1,279,975,320 idle cycles;
fuel was exactly 1,280,000,000. The seven ARM batching/default-policy tests and
the tier-1 chip regression matrix passed.

These are local engine/browser-worker results, not a signed-in production
acceptance or a silicon measurement. CPU-bound firmware has no corresponding
real-time guarantee; only architecturally idle intervals are skipped.
