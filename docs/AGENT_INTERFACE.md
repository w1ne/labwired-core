# LabWired AIPi: Technical Specification for Agents

This document defines the **Agentic Interface for Peripheral Ingestion (AIPi)**. It is designed for external agents (LLMs, automated verifiers) to operate the LabWired toolset as a programmable hardware back-end.

## 1. The AIPi Protocol (State Machine)

Agents must transition through three distinct states to achieve bit-accurate emulation:

| State | Action | Tool | Output Artifact |
| :--- | :--- | :--- | :--- |
| **Ingested** | Extract logic from PDF | `labwired_ai.main ingest-datasheet` | `peripheral.yaml` |
| **Verified** | Cross-check vs physical constraints | `ai/tests/e2e_test.py` | `verification_report.md` |
| **Emulated** | Run firmware in loop | `labwired sim run` | `trace.vcd`, `telemetry.json` |

## 2. Ingestion & Grounding (Machine-Readable Evidence)

The ingestion tool produces YAML with internal **grounding metadata**. Agents should parse these fields to determine the "reliability" of the model.

```yaml
# Schema snippet for Agentic Parsing
registers:
  - name: DEVID
    evidence: "Quoted snippet from PDF" # USE THIS for hallucination checking
    reasoning: "LLM Step-by-step logic"  # USE THIS for causal traceback
    confidence: 0.95                   # Signals if human review is needed
```

## 3. The Verification API (Automatic Truth-Checking)

The verification tool returns structured exit codes for agentic decision making:

*   **Exit 0**: Pass. Proceed to Emulation.
*   **Exit 1**: Schema Error. (Agent should retry ingestion with better prompt).
*   **Exit 2**: Physical Contradiction. (e.g., Register offset exceeds peripheral boundary).
*   **Exit 3**: Logic Hallucination. (Evidence does not support the action).

## 4. Emulation Hooks (The "Remote Hands" for Agents)

Once in the **Emulated** state, the toolset provides hooks for agents to inspect the "Live Silicon":

*   `--inspect-reg <NAME>`: Read the value of a synthesized register at runtime.
*   `--inject-fault <BITFIELD>`: Manually toggle a bit to see if the agent's driver handles the error.
*   `--trace-bus <BUS_ID>`: Export a logic analyzer trace for the agent to verify protocol compliance.

## 5. Monetization Control

Agents must track **Simulation Minutes** (`SIM_MIN`).
*   **Budget Guardrail**: Use `--max-sim-min 10` to prevent infinite loops from draining the agent's account.
*   **Qualification Kits**: Enterprises can request the `TQK_VERIFY` tool to generate ISO 26262 safety evidence automatically.

---
> [!IMPORTANT]
> LabWired is a **deterministic back-end**. If the agent provides an incorrect `system.yaml`, the simulation will fail with a `BusPanic`. Use the `verification` tool early and often.
