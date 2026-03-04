[← Back to Hub](../README.md)

# LabWired Foundry: System-Level Synthesis Architecture

## Overview
The LabWired Foundry Verification-as-a-Service (VaaS) API is primarily designed for the iterative development and formal verification of individual hardware peripherals (e.g., an isolated BME280 sensor). However, the underlying orchestration engine natively supports **System-Level Synthesis**—simulating complex environments containing a microcontroller unit (MCU), varied communication buses (I2C, SPI), and multiple interacting peripherals (sensors, radios, actuators).

This document outlines how autonomous agents utilize the Foundry to scale from single-component verification to full-board system synthesis.

## The "Lego Block" Philosophy

Agents construct complex systems using verified components from the **Asset Catalog**. The Foundry ensures that if an agent compiles a system from "Solid Proven" blocks, the simulation guarantees high fidelity and deterministic behavior without hallucinating unknown peripheral logic.

### 1. Component Verification (The Building Blocks)
Before a complex system can be synthesized, individual components must be verified.
- The agent utilizes the iterative API loop (`GET /tasks/next` -> `POST /verify`) to implement isolated components.
- Example: The agent implements a BME280 temperature sensor. Once `labwired test` yields a "Solid Proof", the peripheral is placed in the Asset Catalog.

### 2. System Inventory (Catalog Retrieval)
When tasked with building a complex MCU board, the agent first consults the Asset Catalog:
- `GET /v1/catalog`
- The agent selects required, pre-verified entities: an STM32 MCU profile, the BME280 sensor, an nRF24 radio, etc.

### 3. System Wiring (The Netlist)
The agent synthesizes a master **`system.yaml`** (or `machine.yaml`). This file acts as the hardware netlist, defining the topological connections between the blocks:
- Instantiating the MCU core.
- Mapping the BME280's SDA/SCL pins to the MCU's `I2C1` interface.
- Mapping the nRF24's MISO/MOSI/SCK/CS pins to the MCU's `SPI2` interface.
- Defining common clock domains and power rails.

### 4. System Verification & Integration Testing
The agent posts the integrated `system.yaml` to the engine.

> **Proposed API Endpoint**: `POST /v1/systems/verify`

The Foundry executes the system-level simulation synchronously. The resulting output is significantly more complex than a single peripheral test:
- **Multi-Bus VCD Traces**: The returned Value Change Dump (`vcd_url`) contains synchronized traces of the I2C bus, SPI bus, and internal MCU state simultaneously.
- **Integration Compiler Logs**: The engine detects mismatched baud rates, address collisions on the I2C bus, or incorrectly mapped IRQ lines.

### 5. Iterative Debugging at the System Level
If the system verification fails, the agent leverages the returned traces and logs. For instance, if the backend reports an I2C address collision because two sensors default to `0x76`, the agent dynamically rewires one sensor's address pin in the `system.yaml` and resubmits to `POST /v1/systems/verify`.

## Summary
By exposing the backend's inherent system-level simulation capabilities through a dedicated Agent API, the LabWired Foundry evolves from a simple model generator into an autonomous hardware integration test bench capable of verifying complex Cyber-Physical Systems.
