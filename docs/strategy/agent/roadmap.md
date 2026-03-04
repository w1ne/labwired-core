[← Back to Hub](../../README.md)

# LabWired AI: Detailed Roadmap

This document breaks down the "AI Peripheral Generation" track into small, testable deliverables.

## 🏁 Phase 1: The "Simple Start" (Datasheet Ingestion)
**Goal**: Automate the tedious part of peripheral creation (typing register definitions).

*   **Step 1.1: PDF Text Extraction**
    *   Input: PDF Datasheet (e.g., STM32F103 Reference Manual).
    *   Action: Extract text from a specific page range.
    *   Deliverable: `labwired-ai extract-text --pdf <file> --pages 100-105`
*   **Step 1.2: Register Table Parsing (LLM)**
    *   Input: Raw text of a register description.
    *   Action: Identify Register Name, Offset, Bit Fields.
    *   Output: JSON Intermediate Representation (IR).
    *   Deliverable: `labwired-ai parse-register "Address offset: 0x00, Reset value: 0x0000"`
*   **Step 1.3: YAML Generation**
    *   Action: Convert JSON IR to `labwired-config` YAML.
    *   Deliverable: A valid YAML file that `labwired-cli` can load.



## 🧠 Phase 2: Logic Synthesis (Behavior)
**Goal**: Make the generated peripherals *do* something.

*   **Step 2.1: Side Effect classification**
    *   Action: Classify bits as `read_to_clear`, `write_1_to_clear`, etc.
*   **Step 2.2: Interrupt Extraction**
    *   Action: Identify which events trigger interrupts (e.g., "RXNEIE bit enables interrupt when RXNE is set").

## 👁 Phase 3: Schematic Vision (Long Term)
**Goal**: Auto-configure the `system.yaml` from a schematic image.

*   **Step 3.1**: Netlist Component Extraction (VLM).
*   **Step 3.2**: Pin Mapping.

## 🏅 Quality Tiers (Iterative Delivery)
To ensure we ship useful tools early, we define three maturity levels:

*   **🥉 Bronze (Helper)**:
    *   Extracts Register Names and Offsets correctly.
    *   User manually enters bit fields and logic.
    *   *Value*: Saves ~50% of typing effort.
*   **🥈 Silver (Drafter)**:
    *   Extracts Fields and Reset Values.
    *   *Value*: Saves ~80% of typing effort.
*   **🥇 Gold (Automator)**:
    *   Extracts Access Types and basic Logic/Interrupts.
    *   *Value*: "Review only" workflow.

## Phase 4: Board Onboarding Automation (Q2 2026)
**Goal:** Reduce "Time to First Simulation" from minutes to seconds.

### 4.1 SVD "Chip Scaffolding"
*   **Current State**: User manually creates `chip.yaml` and links peripherals.
*   **Planned**: `svd-ingestor --scaffold` generates the full `chip.yaml` automatically.

### 4.2 Project Templates (`labwired new`)
*   **Current State**: Manual `mkdir`, copy-paste `Cargo.toml`.
*   **Planned**: `cargo labwired new --board nrf52` generates full project structure with dependencies and memory layout.
