# Hardware Onboarding Workflows

This document details the two primary methods for onboarding new hardware (MCUs and Peripherals) into LabWired:
1.  **SVD Ingestion (Automated)**: Best for standard MCUs with CMSIS-SVD files.
2.  **AI/Manual Generation**: Best for external peripherals (Sensors, Drivers) or MCUs without SVDs.

---

## Path A: SVD Ingestion (Recommended)

The `svd-ingestor` tool parses standard `.svd` files and converts them into the LabWired `declarative` YAML format. This automated pipeline ensures high fidelity and saves manual effort.

### 1. Prerequisite
- Locate the vendor's `.svd` file for the target MCU.
- Ensure `svd-ingestor` is available in the `core` workspace.

### 2. Usage
Run the ingestor using cargo from the `core` directory:

```bash
cargo run -p svd-ingestor -- \
  --input <PATH_TO_SVD> \
  --output-dir <OUTPUT_DIRECTORY> \
  --filter <PERIPHERAL_NAMES>
```

**Arguments**:
*   `--input`: Path to the source `.svd` file.
*   `--output-dir`: Directory where generated `.yaml` files will be saved.
*   `--filter`: (Optional) Comma-separated list of peripheral names to process (e.g., `CLOCK,UART`).

### 3. Example: nRF52832
```bash
# Generate CLOCK, GPIO, and UART models
cargo run -p svd-ingestor -- \
  --input core/tests/fixtures/real_world/nrf52832.svd \
  --output-dir core/examples/nrf52-demo/peripherals \
  --filter CLOCK,P0,UARTE0
```
*Result*: Generates `clock.yaml`, `p0.yaml`, and `uarte0.yaml` which can be referenced in `chip.yaml`.

---

## Path B: AI-Assisted Generation (External Peripherals)

For external components (e.g., Accelerometers, Drivers) where no SVD exists, we use a Vision-Language Model (VLM) pipeline to extract models from datasheets.

### 1. The Algorithm
The process follows a "Generate -> Verify -> Fix" loop:
1.  **Schematic Analysis (VLM)**: Identify component connections (e.g., "U3 is LM75B on I2C1").
2.  **Datasheet Ingestion**: Extract register maps from PDF tables.
3.  **Logic Synthesis**: Derive behavioral logic (side effects, timing) from text descriptions.

### 2. Workflow: Iterative Refinement
This workflow defines how an Agent should create a model using the `labwired_ai` toolset.

#### Step 1: Initial Hypothesis
Use the `ingest-datasheet` tool to create the first draft.
```bash
python3 -m labwired_ai.main ingest-datasheet \
  --pdf path/to/datasheet.pdf \
  --pages "6-12" \
  --name ADXL345 \
  --output adxl345_draft.yaml
```

#### Step 2: Verification Loop
Load the model into the Python sandbox and run automated rule checks.
```bash
python3 -m labwired_ai.rules --model adxl345_draft.yaml --device adxl345
```
*   **If Fail**: Agent reads the specific datasheet section cited in the error, corrects the YAML, and re-runs Step 2.
*   **If Pass**: Proceed to Step 3.

#### Step 3: Behavioral Prototyping
Verify timing hooks and side effects.
```bash
python3 -m labwired_ai.executor --model adxl345_final.yaml --stimulus "write 0x2D 0x08; wait 100; read 0x2D"
```

### 3. Risks & Mitigations
*   **Hallucination**: LLMs may invent registers. **Mitigation**: Grounding (cite page numbers) and cross-reference with SVD if available.
*   **Vision Errors**: Misreading schematics. **Mitigation**: User confirmation of pinout.

---

## Path C: External Device Integration (I2C/SPI)

Once a bus controller (I2C/SPI) is onboarded via Path A or B, external sensors can be attached to it via the system manifest.

### 1. Attachment Strategy
- **I2C**: Define the `address` and `connection` (bus controller ID).
- **SPI**: Define the `connection`. Note that CS is typically handled via GPIO in the firmware, so the `Spi` core broadcasts transfers to all attached devices, and devices should only respond if they were "selected" by a preceding GPIO operation that the simulator understands (or by being the only device).

### 2. Manifest Configuration
Add the device to `system.yaml`:
```yaml
external_devices:
  - id: "temp_sensor"
    type: "lm75"
    connection: "TWI0"
    config:
      address: 0x48
```

---

## Validation
Regardless of the generation method, the final step is **Simulation Verification**:
1.  Create a minimal firmware that interacts with the peripheral.
2.  Search for success/failure loops in the disassembly to identify target PCs.
3.  Run `labwired --audit-checks` to ensure no Bus Faults or access violations occur.
4.  Verify register transitions using `--trace` or a test script.
