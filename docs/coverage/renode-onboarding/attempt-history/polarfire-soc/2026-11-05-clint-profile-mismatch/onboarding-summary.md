# Onboarding Smoke: polarfire-soc

- status: `fail`
- run_attempted: `True`
- booted: `True`
- smoke_passed: `False`
- elapsed_seconds: `10`
- elapsed_ms: `9765`
- threshold_seconds: `3600`
- threshold_met: `True`
- failure_stage: `run_smoke`
- first_error_signature: `2026-11-05T22:57:04.603993Z ERROR labwired_cli::execute: Assertion failed: UartOrdered(UartOrderedAssertion { uart_ordered: ["*** Booting Zephyr OS build v4.1.0 ***", "Hello World! mpfs_icicle/polarfire/u54"] }) (captured len=0)`
- failure_hint: `Smoke assertion mismatch. Compare expected stop/assertions with current deterministic output.`
