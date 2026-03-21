[← Back to Hub](../README.md)

# LabWired Compatibility Matrix

This document defines the current support levels for MCU architectures, families, and peripherals from the user perspective.

Recommended starting point:
- Start with the bundled Cortex-M examples and CI fixtures first.
- Treat the recommended examples as the reference path for evaluation and demos.

## Support Tiers

| Tier | Definition |
| :--- | :--- |
| **Tier 1 (Recommended)** | Best current user experience. Reliable for demos, docs, and validation. |
| **Tier 2 (Partial)** | Useful, but may require manual setup, narrower peripheral assumptions, or careful reading of example docs. |
| **Tier 3 (Experimental)** | Research-grade or incomplete. Do not treat as launch-grade support. |

## Recommended Evaluation Targets

| Target | Why start here |
| :--- | :--- |
| **STM32F103 / Cortex-M3 examples** | Best low-friction starting point for local simulation and simple firmware runs. |
| **CI UART fixtures** | Fastest way to confirm deterministic output and artifact generation. |
| **NUCLEO-H563ZI showcase** | Stronger feature demonstration once the basic workflow is already understood. |

## MCU Architectures

| Architecture | Tier | Notes |
| :--- | :--- | :--- |
| **ARM Cortex-M3 (STM32F103)** | Tier 1 | Best general-purpose starting point. |
| **ARM Cortex-M33 (STM32H563)** | Tier 1 | Strong showcase path, but not the lowest-friction first run. |
| **Generic ARMv6-M** | Tier 2 | Good for constrained Cortex-M evaluation. |
| **RISC-V (RV32I)** | Tier 3 | Available, but not the primary launch path. |

## Peripheral Ecosystem (STM32)

| Peripheral | Tier | Notes |
| :--- | :--- | :--- |
| **RCC / FLASH / SYSTICK** | Tier 1 | Core launch path depends on these being stable. |
| **GPIO / NVIC / SCB** | Tier 1 | Good fit for debug and board bring-up workflows. |
| **USART / UART** | Tier 1 | Primary user-visible signal path for examples and CI. |
| **I2C / SPI** | Tier 2 | Useful, but often tied to mock or device-specific assumptions. |
| **ADC / DMA** | Tier 2 | Present, but not the safest first evaluation target. |
| **USB / CAN / Ethernet** | Tier 3 | Not launch-grade support. |

## External Components (Mocks)

| Component | Tier | Notes |
| :--- | :--- | :--- |
| **TMP102 (I2C Sensor)** | Tier 1 | Core reference for I2C stubbing. |
| **LM75B (I2C Sensor)** | Tier 1 | Reliable fallback for demo dry runs. |
| **Generic LED / Button** | Tier 1 | Integrated with VS Code Command Center. |

## Auto-Generated Matrix

A machine-readable compatibility matrix is auto-generated on every CI build by `core/scripts/generate_compat_matrix.py`. It enumerates chip configs, peripheral types, and smoke test coverage, and is uploaded as a CI artifact (`compatibility-matrix`). Use it for programmatic queries; this document provides the human-readable interpretation.

## User Guidance

- If you are evaluating LabWired for the first time, do not start with experimental targets.
- If your use case depends on Tier 2 or Tier 3 features, treat the workflow as assisted evaluation rather than drop-in production replacement.
- Release readiness depends on this document staying aligned with actual examples and validation evidence.
