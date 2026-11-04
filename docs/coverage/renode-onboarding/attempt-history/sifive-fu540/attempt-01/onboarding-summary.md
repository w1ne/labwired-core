# Onboarding Smoke: sifive-fu540

- status: `fail`
- run_attempted: `True`
- booted: `False`
- smoke_passed: `False`
- elapsed_seconds: `16`
- elapsed_ms: `15677`
- threshold_seconds: `3600`
- threshold_met: `True`
- failure_stage: `run_smoke`
- first_error_signature: `2026-11-04T15:41:00.872501Z ERROR labwired_cli::execute: Simulation error at step 0: not implemented: eight-byte physical access at 0x20000010 requires RAM; MMIO64 is unsupported`
- failure_hint: `Model gap. Check unsupported access logs and implement/fix the first missing peripheral behavior.`
