# LabWired AI: Toolset for Agents (AIPi)

This directory provides the **Programmable Toolset** used by LabWired operators and agents to generate, verify, and emulate hardware peripherals from unstructured data.

## Structure

*   `docs/` - Algorithm specifications and [Agent Interface Guide](file:///home/andrii/Projects/labwired/docs/AGENT_INTERFACE.md).
*   `labwired_ai/` - Core Python modules (LLM, Schematic Parsing, IR Conversion, Orchestration).
*   `tests/` - E2E verification pipelines for agent-driven workflows.

## Key Commands

### `auto-ingest` — Zero-Touch Pipeline

End-to-end datasheet-to-simulation orchestrator with automatic retries:

```bash
python -m labwired_ai auto-ingest \
  --pdf datasheet.pdf --pages 6-12 \
  --name MY_CHIP --output-dir out/my-chip \
  --max-retries 3 --auto-approve-threshold 0.9
```

Chains: PDF ingestion → register extraction → behavioral synthesis → IR conversion → verification. On failure, collects errors, re-prompts the LLM, and retries (up to 3x). Confidence scoring auto-approves when pass rate >= threshold.

This pipeline should be treated as an internal catalog-onboarding tool, not a public self-serve product promise.

### Telemetry Export

When `LABWIRED_FOUNDRY_URL` and `LABWIRED_API_KEY` environment variables are set, usage telemetry (simulation minutes, operation types) is exported to the legacy Foundry Go backend.

> The Foundry backend has been deprecated as a product framing — paid CI metering is now handled by the Cloudflare Worker in `packages/api`. The telemetry export still works against the legacy backend if it's running, but it is not part of the live billing path and will be re-wired when this SDK is updated.

## Strategic Goal: The Agentic Moat

LabWired solves the **Peripheral Modeling Bottleneck** by providing a high-fidelity API that agents use to:
1.  **Extract**: Turn PDF datasheets into grounded Register Maps.
2.  **Synthesize**: Generate Rust drivers and simulation behaviors.
3.  **Verify**: Prove driver/firmware correctness in a bit-accurate ARM-native environment.

Public product positioning is narrower: customers run firmware against the curated board catalog via the CLI or paid CI tier; this AI toolchain remains the internal engine for expanding that catalog.

See [AGENT_INTERFACE.md](file:///home/andrii/Projects/labwired/docs/AGENT_INTERFACE.md) for external agent integration patterns.
