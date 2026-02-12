# Demo Dry Run (Release Gate)

Use this before every external demo to validate the critical path:

1. Digital twin model generation path
2. IR conversion + codegen
3. Project wiring (`asset init` + `asset add-peripheral`)
4. Local simulator run
5. DAP backend smoke test (VS Code debug backend)
6. Docker runtime + `labwired-dap` availability

## Fallback Mode (No Live AI Required)

Uses pre-generated models from `ai/tests/*_gen.yaml` and is resilient to API/key/network issues.

```bash
python3 ai/tests/demo_dry_run.py --mode fallback --device LM75B --docker
```

## Live Mode (Optional)

Runs full datasheet ingestion via LLM.

```bash
python3 ai/tests/demo_dry_run.py \
  --mode live \
  --device LM75B \
  --datasheet ai/tests/fixtures/lm75b.pdf \
  --pages 1-8 \
  --docker
```

## Notes

- Default chip for dry-run is `ci-fixture-cortex-m3-uart1` because it is stable with fixture firmware.
- The script writes outputs to `ai/tests/demo_dry_run_output/`.
- For VS Code-specific UI checks (breakpoints/panels), run a quick manual pass after this script.
