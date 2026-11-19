# Onboarding Smoke: efr32mg26

- status: `fail`
- run_attempted: `True`
- booted: `True`
- smoke_passed: `False`
- elapsed_seconds: `335`
- elapsed_ms: `334801`
- threshold_seconds: `3600`
- threshold_met: `True`
- failure_stage: `run_smoke`
- first_error_signature: `2026-11-15T15:36:39.307876Z ERROR labwired_cli::execute: Assertion failed: UartContains(UartContainsAssertion { uart_contains: "timers w32=1 t1w32=1 run=1 cnt=1 stop=1 irq=3 direct_kept=1 alias_clr=1 t1of=1 t1masked=1 t1pend=1 t1rel=1 ok" }`
- failure_hint: `Smoke assertion mismatch. Compare expected stop/assertions with current deterministic output.`
