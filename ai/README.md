# LabWired AI: Toolset for Agents (AIPi)

This directory provides the **Programmable Toolset** used by agents to generate, verify, and emulate hardware peripherals from unstructured data.

## Structure

*   `docs/` - Algorithm specifications and [Agent Interface Guide](file:///home/andrii/Projects/labwired/docs/AGENT_INTERFACE.md).
*   `labwired_ai/` - Core Python modules (LLM, Schematic Parsing, IR Conversion).
*   `tests/` - E2E verification pipelines for agent-driven workflows.

## Strategic Goal: The Agentic Moat

LabWired solves the **Peripheral Modeling Bottleneck** by providing a high-fidelity API that agents use to:
1.  **Extract**: Turn PDF datasheets into grounded Register Maps.
2.  **Synthesize**: Generate Rust drivers and simulation behaviors.
3.  **Verify**: Prove driver/firmware correctness in a bit-accurate ARM-native environment.

See [AGENT_INTERFACE.md](file:///home/andrii/Projects/labwired/docs/AGENT_INTERFACE.md) for external agent integration patterns.
