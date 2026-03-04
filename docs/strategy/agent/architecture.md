[← Back to Hub](../../README.md)

# LabWired AI Architecture

The `ai/` directory hosts the tooling for **Automated Peripheral Modeling**. This component is designed to be loosely coupled with the main emulator, interacting primarily through data formats (YAML).

## Directory Structure

```text
ai/
├── docs/           # Specifications and Algorithm Descriptions
├── src/            # Python source code (VLM/LLM logic)
├── scripts/        # CLI wrappers
└── tests/          # Validation against known SVDs
```

## Integration Contract

The AI component produces **Declarative Peripheral Models** (YAML) that strictly adhere to the schema defined in `labwired-config`.

*   **Input**: Unstructured Data (PDF Datasheets, Schematic Images).
*   **Output**: Structured YAML (`.yaml`).
*   **Consumer**: The Emulator (`crates/config` parser).

## Workflow

1.  **Drafting**: User runs `labwired-ai gen <datasheet>` to create a draft model.
2.  **Refining**: User verifies the YAML against the datasheet.
3.  **Simulating**: `labwired-cli` loads the YAML to simulate the hardware.

## Separation of Concerns

*   **Emulator**: Deterministic, Rust-based, "Ground Truth".
*   **AI Tools**: Probabilistic, Python-based, "Drafting Assistant".

This separation ensures that complex AI dependencies (PyTorch, Transformers) do not bloat the core emulator build.
