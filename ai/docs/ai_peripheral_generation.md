# Algorithm for AI-Generated Peripherals from PCB Docs

## Goal
To automate the creation of `labwired` declarative peripheral models (`.yaml`) from PCB schematics and component datasheets. This addresses the "core value" of rapidly simulating new hardware.

## Input Data
1.  **PCB Schematics**: PDF or Image files showing component interconnections.
2.  **Datasheets**: PDF documents for specific components (sensors, memories, drivers).
3.  **BOM (Bill of Materials)**: Optional, for resolving part numbers.

## Proposed Algorithm

### Phase 1: Schematic Analysis (Context Extraction)
**Goal**: Identify *what* to simulate and *how* it connects to the MCU.

1.  **Image Preprocessing**: Convert Schematic PDF to high-res images.
2.  **Component Detection (VLM)**:
    *   Use a Vision-Language Model (e.g., GPT-4o, Claude 3.5 Sonnet, Gemini 1.5 Pro) to analyze the schematic.
    *   **Prompt**: "Identify all Integrated Circuits (ICs) connected to the main microcontroller. List their designators (e.g., U1), part numbers, and the communication bus used (I2C, SPI, UART, GPIO)."
3.  **Netlist Extraction**:
    *   Trace lines from the identified component to the MCU pins to confirm the bus.
    *   **Output**: List of `TargetComponent` objects:
        ```json
        {
          "designator": "U3",
          "part_number": "AT24C256",
          "bus": "I2C1",
          "address": "0x50" // for I2C
        }
        ```

### Phase 2: Datasheet Ingestion (Register Map Extraction)
**Goal**: Extract the memory map and register definitions.

1.  **Text & Table Extraction**:
    *   Use a tool like `marker` or `unstructured` to convert PDF datasheet to Markdown/JSON.
    *   Focus on sections titled "Register Map", "Register Description", "Memory Map".
2.  **Register Parsing (LLM)**:
    *   **Prompt**: "Extract the register table. For each register, provide: Name, Offset, Reset Value, Access (R/W), and a list of Bit Fields (Name, Range, Description)."
    *   **Normalization**: Map access types (e.g., "r/w", "rw", "read/write") to `labwired` types (`ReadWrite`, `ReadOnly`, `WriteOnly`).
3.  **Output**: Structured `RegisterDefinition` list.

### Phase 3: Behavioral Logic Synthesis
**Goal**: Derive `timing` hooks and `side_effects` (Hardware Simulation Logic).

*Note: Standard SVD files commonly lack this behavioral data (timing, triggers). This is the core value-add of the AI approach.*

1.  **Logic Extraction (LLM)**:
    *   **Input**: Datasheet text descriptions of control bits.
    *   **Prompt**: "Identify causal relationships. Example: 'Setting bit TXEN starts transmission, resulting in TXC interrupt after 10 cycles'."
2.  **Intermediate Representation (IR)**:
    *   Construct a `BehaviorModel` JSON:
        ```json
        {
          "triggers": [
            { "event": "write", "register": "CR1", "mask": "0x2000", "action": "start_tx" }
          ],
          "actions": {
            "start_tx": { "delay": 100, "set_flag": "SR.TXE", "interrupt": "USART1" }
          }
        }
        ```
3.  **Mapping to Declarative Format**:
    *   Convert `write_action: one_to_clear` from text "writing 1 clears this bit".
    *   Convert `read_action: clear` from "read data register to clear RXNE".

### Phase 4: Code/YAML Generation
**Goal**: Produce the final `.yaml` file.

1.  **Hybrid Generation**:
    *   **Registers**: Can utilize SVD structure if available, or generate `RegisterDescriptor` directly.
    *   **Logic**: Inject the `timing` and `side_effects` into the YAML.
2.  **Validation**:
    *   Ensure register offsets do not overlap.
    *   Verify referred registers in timing hooks exist.

## Feasibility & Evaluation

| Component | Maturity | Risk | Value |
|-----------|----------|------|-------|
| Schematic Analysis (VLM) | Medium | High (Complex layouts, handwriting) | High (Auto-configuration) |
| Register Map Extraction | High | Low (Structured tables in PDF) | Medium (Saves typing) |
| Logic Synthesis | Low | High (Ambiguity in text) | **Verify High** (Enables simulation) |

## Double Pass: Concrete Walkthrough (Hypothetical LM75 Temperature Sensor)

To validate the "Input -> Simulation" pipeline, let's trace a simple I2C sensor.

### 1. Input Data
*   **Schematic Snippet**: An image showing an IC labeled `U3` connected to pins `PB6` (SCL) and `PB7` (SDA) of the MCU. Part number `LM75B` is visible.
*   **Datasheet Snippet**: "The LM75B is a temperature sensor... Address pointer register (0x00) accesses: Temperature (0x00, Read Only), Configuration (0x01, R/W)..."

### 2. Phase 1: Schematic Analysis (VLM)
*   **VLM Prompt**: "List all ICs connected to the MCU. Identify the bus."
*   **Output**:
    ```json
    {
      "component": "U3",
      "part_number": "LM75B",
      "bus_type": "I2C",
      "mcu_connections": {"SCL": "PB6", "SDA": "PB7"}
    }
    ```
*   **System Action**: System identifies `PB6/PB7` corresponds to `I2C1` peripheral on the STM32F103 (target MCU).

### 3. Phase 2: Register Map Extraction (LLM)
*   **LLM Prompt**: "Extract register map from datasheet text."
*   **Datasheet Text**: "Register 0x01 is Configuration. Bit 0: Shutdown. When set to 1, device enters low power mode."
*   **Output (IR)**:
    ```yaml
    registers:
      - name: TEMP
        offset: 0x00
        access: ReadOnly
      - name: CONF
        offset: 0x01
        fields:
          - name: SHUTDOWN
            bit_range: [0, 0]
            description: "1 = Low Power Mode"
    ```

### 4. Phase 3: Behavioral Logic (The "AI" Value)
*   **LLM Prompt**: "Identify side effects. What happens when SHUTDOWN is written?"
*   **Output (IR)**:
    ```yaml
    behaviors:
      - trigger: "write CONF.SHUTDOWN = 1"
        action: "power_state = OFF"
    ```
    *(Note: For an I2C sensor, the "simulation" logic is often just responding to reads. Complex logic like "power down" might just log a message in the initial version.)*

### 5. Final Output: LabWired Peripheral YAML
This YAML is loaded by the I2C Controller simulation to mock the device at address `0x48` (default LM75).

```yaml
peripheral: LM75B
type: I2C_Device
registers:
  - id: TEMP
    address_offset: 0x00
    access: ReadOnly
    reset_value: 0x0000
    description: "Temperature Data"
  - id: CONF
    address_offset: 0x01
    fields:
      - name: SD
        bit_range: [0, 0]
timing: [] # Passive device
```

## Conclusion of Double Pass
The algorithm is feasible but relies heavily on the **Schematic -> Netlist** accuracy.
*   **Risk**: VLM misinterpreting complex bus wiring.
*   **Mitigation**: User confirmation step. "I found U3 (LM75B) on I2C1. Is this correct?"

## Third Pass: Risks & Mitigations

We must anticipate where this probabilistic pipeline will fail.

### 1. Hallucination Risks (High Severity)
*   **Problem**: LLM invents comfortable lies.
    *   *Example*: "Register 0x05 is the 'Turbo Mode' register" (does not exist).
    *   *Example*: "Writing 1 to bit 7 generates a nuclear explosion interrupt" (logic exaggeration).
*   **Mitigation**:
    *   **Grounding**: Force the model to cite page numbers from the datasheet.
    *   **Verification**: If an SVD is available, hard-fail if the LLM output contradicts the SVD's register map.
    *   **Confidence Scores**: Ask the LLM "On a scale of 1-10, how sure are you this bit is self-clearing?". Flag low confidence items for user review.

### 2. OCR & Vision Errors (Medium Severity)
*   **Problem**: Misreading schematics.
    *   *Example*: Confusing `PB6` (I2C) with `PA6` (SPI).
    *   *Example*: Failing to read a blurry part number.
*   **Mitigation**:
    *   **Netlist Cross-Check**: If the VLM says "I2C on PA6", check `labwired`'s own MCU definition. If PA6 doesn't support I2C, flag error.
    *   **User Confirmation**: "I see a chip on PA6. Please confirm the pinout."

### 3. Ambiguity in "Human" Datasheets
*   **Problem**: Text is vague. "Write 1 to start." (Start what? When does it finish?).
*   **Mitigation**:
    *   **Sensible Defaults**: Be pessimistic. If duration is unknown, assume immediate completion (0 cycles).
    *   **Comments**: Generate YAML comments: `# AI Note: Exact timing unspecified, assuming 0 cycles.`

### 4. Version Mismatches
*   **Problem**: Datasheet is for `Rev B`, Schematic uses `Rev C`.
*   **Mitigation**:
    *   Prompt the model to extract the "Revision" string from both documents and warn on mismatch.

## Strategic Conclusion
The "AI Generation" feature should be positioned as a **"Drafting Tool"**, not a "Compiler". It gets the user 80% of the way there—creating the file, typing the registers, scaffolding the logic—leaving the user to fix the final 20% (complex timing, specific edge cases). This creates immense value by removing the "Blank Page Problem".
