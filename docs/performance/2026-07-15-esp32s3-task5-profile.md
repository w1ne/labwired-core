# ESP32-S3 OLED Task 5 profile

## Decision

No runtime optimization was committed. The native profile does not justify an
Xtensa interpreter change: the dominant sampled cost is the per-cycle bus and
peripheral tick orchestration, not Xtensa instruction decode or execution.

This report is intentionally scoped to Task 5. It does not change ISA
semantics, ROM behavior, profile counters, C3 behavior, or the S3 workload.

## Workload and baseline

- Source revision: `cad3febf` (`test: pin complete S3 OLED legacy mapping`)
- Test: `crates/core/tests/esp32s3_oled_profile.rs`
- Release build command:

  ```text
  cargo test -p labwired-core --features event-scheduler \
    --test esp32s3_oled_profile --release --no-run
  ```

- Baseline command:

  ```text
  target/release/deps/esp32s3_oled_profile-47a2a070db637d37 \
    esp32s3_oled_native_baseline --ignored --nocapture
  ```

- Three native release runs: 1.319 s, 1.314 s, 1.318 s
- Throughput: 1.621, 1.627, 1.623 MIPS
- Retired instructions: `2,139,600`
- Exact first paint: cycle `1,139,600`, FNV-1a `4732199435356771915`
- Completion cycle: `2,139,600`
- Legacy entries: `79,165,200` (`37` active entries per cycle)
- Serial digest: `af2df535cf6fd7e4`
- Framebuffer digest: `c4eb9ef771b3ded8`

All three runs passed with identical counters and digests.

## Native sample evidence

The test binary was run with the same S3 setup and a bounded diagnostic budget
(`LABWIRED_ESP32S3_OLED_MAX_CYCLES=100000000`) plus an intentionally absent
completion marker. Precisely, the profiling command set
`LABWIRED_ESP32S3_OLED_SERIAL_MARKER=__never_emitted__`. The curated ELF emits
the browser S3 completion marker `S3 OLED painted`, not `__never_emitted__`, so
the test continued until the configured budget rather than stopping at normal
completion. This changes only the diagnostic run's stopping condition; it does
not change the firmware, simulator, or production/browser configuration.

The exact profiling command was:

```text
LABWIRED_ESP32S3_OLED_MAX_CYCLES=100000000 \
LABWIRED_ESP32S3_OLED_SERIAL_MARKER=__never_emitted__ \
target/release/deps/esp32s3_oled_profile-47a2a070db637d37 \
  esp32s3_oled_native_baseline --ignored --nocapture
```

While that process was running, macOS `sample` was invoked for 10 seconds:

```text
sample 42889 10 1 -file /tmp/labwired-s3-sample.txt
```

PID `42889` was the run-specific test process. After sampling, it was
terminated with `kill 42889`; a reproducible run should substitute the PID
reported by `ps aux` for the matching test binary. This was a profiling run
only; it was not added to the production workload and did not introduce a
simulator timing primitive.

The sample contained 7,595 main execution-thread samples:

- `Machine::run`: 5,324 samples as the simulation driver.
- `SystemBus::tick_peripherals_fully`: 4,509 samples as the dominant child
  path (~59% of samples).
- `SystemBus::tick_peripherals_phase1`: 1,438 samples (~19%), including
  legacy-walk orchestration.
- `Esp32s3Uart::int_raw` and SipHash-backed peripheral lookup frames were
  recurring sub-costs under the tick path.
- No Xtensa `step`, decode, or `execute` frame was a dominant leaf in the
  sample.

The evidence points to bus/peripheral scheduling and lookup work as the next
optimization target. That is outside Task 5's Xtensa-interpreter gate and
would require a separate semantics review, especially for GPIO/I2C0 timing
and IRQ behavior. No speculative change was made.

## Verification

- Native S3 OLED release baseline: passed three times.
- Xtensa LX7 focused unit tests: 8 passed.
- ESP32-C3 focused unit tests: 122 passed.
- No hardware parity claim: the connected board was detected previously, but
  OpenOCD capture is unavailable in this environment.
