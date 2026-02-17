---
description: How to iteratively refine a hardware peripheral model using AI and simulation feedback
---

This workflow defines the "Generate -> Verify -> Fix" loop that an AI agent should follow to create high-fidelity LabWired peripheral models with minimal human intervention.

### Prerequisites
- High-fidelity PDF datasheet for the target peripheral.
- `labwired` Python bindings installed (`maturin develop` in `core/crates/python`).
- Target peripheral ID (e.g., `ADXL345`).

### Step 1: Initial Hypothesis (Extraction)
Use the `ingest-datasheet` tool to create the first draft of the peripheral model.
```bash
python3 -m labwired_ai.main ingest-datasheet \
  --pdf path/to/datasheet.pdf \
  --pages "6-12" \
  --name ADXL345 \
  --output adxl345_draft.yaml
```

### Step 2: Verification (The Execution Loop)
// turbo
1. Load the peripheral into the LabWired Python sandbox.
2. Run automated "rules" checks.
```bash
python3 -m labwired_ai.rules --model adxl345_draft.yaml --device adxl345
```

### Step 3: Iterative Correction
If Step 2 reports failures (e.g., "Reset value of REG_X expected 0x00, got 0xFF"), the agent must:
1. Re-read the specific section of the datasheet identified in the failure evidence.
2. Update the YAML/JSON model to correct the discrepancy.
3. Repeat Step 2 until all rules pass.

### Step 4: Behavioral Prototyping
Once the register map is static, verify timing hooks and side effects.
// turbo
```bash
python3 -m labwired_ai.executor --model adxl345_final.yaml --stimulus "write 0x2D 0x08; wait 100; read 0x2D"
```

### Step 5: Final Submission
Once the simulation matches the hardware behavior described in the datasheet, promote the model to the `configs/` directory.
