# Modular Components Design Document

This document describes the architectural vision for making LabWired components (Cores, Peripherals, and Interconnects) truly modular and swappable.

## Architectural Boundaries

LabWired follows a "Wiring" model where components are connected via explicit signals rather than being tightly coupled in code.

### 1. Core (CPU)
The Core is responsible for instruction execution. It interacts with the rest of the system through:
- **Bus Interface**: A standard interface for memory transactions (Read/Write).
- **Interrupt Interface**: A set of input lines (IRQs) that can trigger exception handling.
- **Clock/Reset Interface**: Inputs to control execution state and timing.

### 2. Bus / Interconnect
The Bus serves as the system's central nervous system, routing transactions between the Core and Peripherals.
- **Static Mapping**: Memory-mapped regions (Flash, RAM, Peripherals).
- **Arbitration**: Handling multiple masters (DMA, Cores) accessing shared resources.

### 3. Peripheral Model
Peripherals are standalone components that:
- Map to one or more address ranges.
- Expose **Signals** (GPIO pins, IRQ lines).
- Can be "ticked" to simulate internal state transitions over time.

## Multi-Architecture Vision

To "emulate them all," LabWired must support a variety of popular architectures beyond ARM Cortex-M.

### Supported Architectures (Targets)
- **ARM Cortex-M (e.g., STM32, RP2040)**: Current focus. Strong emphasis on NVIC and Thumb ISA.
- **RISC-V (e.g., ESP32-C3, CH32V)**: Rapidly growing open-source ISA. Simplified interrupt models (CLINT/CLIC).
- **Xtensa (e.g., ESP32, ESP32-S3)**: Popular for high-performance IoT. Features complex multi-level interrupt controllers and windowed registers.
- **AVR (e.g., ATmega328P)**: Legacy but still widespread in industrial/hobbyist sectors. 8-bit architecture with direct vector table mapping.

## Multi-Core Support

Many modern MCUs (ESP32, RP2040) are multi-core. Our architecture must transition from a single `cpu` ownership model to a flexible execution model.

### 1. MultiCoreMachine
The `Machine` struct will be evolved to support multiple cores sharing a common `SystemBus`.
- **Inter-Processor Communication (IPC)**: Support for hardware semaphores and mailboxes.
- **Shared Peripherals**: Proper handling of concurrent access to atomic peripherals.

### 2. Heterogeneous Cores
Some systems combine different architectures (e.g., a Cortex-M4 and a Cortex-M0+, or a RISC-V co-processor). Modular boundaries allow these cores to "plug into" the same bus regardless of their internal ISA.

## Revised Interrupt Model for Portability

To support diverse architectures, the "Interrupt Controller" must move from being an ARM-specific NVIC to a generic component that translates `InterruptLine` signals into ISA-specific exceptions.

- **AVR**: Signals map directly to vector table offsets.
- **Xtensa**: Signals map to specific interrupt levels and priority groups.
- **RISC-V**: Signals map to Local or Global (PLIC) interrupt inputs.

## Migration Path: "Emulate Them All"

1. **Generic Cpu Trait refinement**: Ensure `Cpu` does not expose ARM-specific registers or concepts in its core interface.
2. **Pluggable Interrupt Controllers**: Move NVIC logic into a standalone peripheral that implements a generic `InterruptController` trait.
3. **Multi-core Scheduler**: Introduce a simulation tick that propagates cycles across all active cores in a `Machine`.
