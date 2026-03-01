# LabWired Compatibility Matrix

This document defines the current support levels for various MCU architectures, families, and peripherals.

## Support Tiers

| Tier | Definition |
| :--- | :--- |
| **Tier 1 (Fully Supported)** | High reliability, deterministic validation, full peripheral coverage, and automated "WOW" demo scripts. |
| **Tier 2 (Partial)** | Core instruction set supported, basic peripherals (RCC/GPIO/UART) functional. Some manual intervention required. |
| **Tier 3 (Experimental)** | Beta-stage instruction set, limited peripheral support. Useful for research or simple firmware. |

## MCU Architectures

| Architecture | Tier | Notes |
| :--- | :--- | :--- |
| **ARM Cortex-M3 (STM32F103)** | Tier 1 | Our primary foundation and most tested target. |
| **ARM Cortex-M33 (STM32H563)** | Tier 1 | Used for high-end "Hardware Oracle" demos. |
| **Generic ARMv6-M** | Tier 2 | Baseline for minimal Cortex-M cores. |
| **RISC-V (RV32I)** | Tier 3 | Foundation present (Iteration 13.5). |

## Peripheral Ecosystem (STM32)

| Peripheral | Tier | Notes |
| :--- | :--- | :--- |
| **RCC / FLASH / SYSTICK** | Tier 1 | Deterministic boot and timing logic. |
| **GPIO / NVIC / SCB** | Tier 1 | Fully interrupt-capable and visible in VS Code. |
| **USART / UART** | Tier 1 | Terminal output / input as a first-class primitive. |
| **I2C / SPI** | Tier 2 | Protocol logic functional; requires device-specific mocks. |
| **ADC / DMA** | Tier 2 | Basic implementation in `v0.8.0`. |
| **USB / CAN / Ethernet** | Tier 3 | Planned for foundry-driven expansion. |

## External Components (Mocks)

| Component | Tier | Notes |
| :--- | :--- | :--- |
| **TMP102 (I2C Sensor)** | Tier 1 | Core reference for I2C stubbing. |
| **LM75B (I2C Sensor)** | Tier 1 | Reliable fallback for demo dry runs. |
| **Generic LED / Button** | Tier 1 | Integrated with VS Code Command Center. |
