# LabWired PC Trace Audit Protocol

The `labwired-audit` tool compares the retired program-counter sequence from a
hardware capture with the sequence from a LabWired simulation. It produces an
inspectable PC-parity report for the declared comparison scope.

This is one evidence channel, not proof of complete hardware equivalence. The
tool does **not** currently compare register values or UART bytes, hash the
firmware binary, or cryptographically sign its report.

## CLI

```bash
./scripts/labwired-audit.py \
  --hw-trace out/golden-reference/hw_trace.json \
  --sim-trace out/golden-reference/sim_trace.json \
  --target "NUCLEO-H563ZI" \
  --firmware "firmware-h563-demo" \
  --output out/golden-reference/determinism_report.json
```

`--firmware` is a descriptive label. It is not a path consumed or hashed by the
audit. Firmware identity must be established separately when the report is used
as evidence.

## Inputs

1. **Hardware trace**: a JSON array whose entries contain `pc`, normally
   captured from a physical board through a debug probe.
2. **Simulator trace**: a JSON array whose entries contain `pc`, captured from
   LabWired while running the intended firmware and system model.

The audit aligns the traces at the first common PC inside the configured search
window, then compares PCs in order.

## Comparison scopes

The scope is explicit and recorded in `comparison_scope`:

- **Exact (default):** both aligned traces must have the same length. A length
  mismatch fails closed before any PASS can be issued.
- **Bounded (`--max-steps N`):** both aligned traces must supply at least `N`
  entries. PASS covers exactly those `N` PC steps.
- **Prefix (`--allow-prefix`):** compares the complete shorter aligned trace.
  PASS is labeled prefix-only and makes no claim about the unobserved remainder.

`--max-steps` and `--allow-prefix` are mutually exclusive. A comparison with no
steps cannot pass. A FAIL report makes the CLI exit non-zero.

## Output

The JSON report includes:

- `status` (`PASS` or `FAIL`);
- `verification_kind: pc_sequence`;
- alignment offsets, available lengths, and declared scope;
- per-step hardware and simulator PCs;
- the first mismatch, when one is observed;
- explicit `performed: false` fields for firmware-integrity and checksum checks
  that this tool does not perform.

A PC-sequence PASS means only that the compared PCs matched over the recorded
scope. Register parity, UART parity, firmware identity, peripheral behavior, and
cryptographic attestation require separate evidence.

## Workflow

1. Capture a hardware trace from the intended board and firmware.
2. Capture a simulator trace from the intended model and firmware.
3. Choose an exact, bounded, or prefix scope deliberately.
4. Run `labwired-audit` and retain both input traces with the report.
5. State any additional firmware-integrity, register, UART, or hardware evidence
   separately rather than attributing it to this report.

The current golden-reference helper uses `--allow-prefix` because a fixed
hardware capture can be shorter than the simulator run. Its PASS therefore
covers the captured common prefix, not the simulator trace beyond it.
