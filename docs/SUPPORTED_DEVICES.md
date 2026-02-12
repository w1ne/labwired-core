# LabWired Tier 1 Device Support Strategy

To demonstrate the power of LabWired, we are committing to **full, deep support** for the three most popular microcontroller families in the industry/hobbyist space.

These devices will serve as the "Gold Standard" for our simulation fidelity, driving the development of the core engine.

## The Tier 1 Trio

| Device | Core | Why Selected? | Key Features to Support |
| :--- | :--- | :--- | :--- |
| **STM32F401** | ARM Cortex-M4F | Industry standard for embedded dev. Massive ecosystem. | RCC (complex clock tree), DMA, GPIO Matrix. |
| **RP2040** | Dual Cortex-M0+ | Raspberry Pi Pico. Huge hobbyist following. Unique PIO. | **Dual Core** simulation, PIO (Programmable I/O), SIO. |
| **nRF52832** | ARM Cortex-M4F | Dominant in IoT/BLE. | Radio peripheral modeling, PPI (Programmable Peripheral Interconnect), EasyDMA. |

## Support Definition ("Fully Supported")

For a Tier 1 device, "support" means more than just loading the ELF. It includes:

1.  **Strict IR Model**: Full SVD ingestion (Done).
2.  **Memory Map**: Accurate RAM/ROM regions defined in `labwired-core`.
3.  **Interrupt Controller (NVIC)**: Correct vector table sizing and priority grouping.
4.  **Key Peripherals**:
    *   **GPIO**: Pin state visualization.
    *   **UART**: Interactive console I/O.
    *   **Timers**: Accurate counting and interrupt generation.
5.  **Board Integration**: Specific board presets (e.g., "Nucleo-F401RE", "Pico").

## Roadmap

### Phase 1: Ingestion (Current)
-   Verify SVD ingestion for all three (STM32, RP2040, nRF52).
-   Generate `models/<device>.json` ground truth.

### Phase 2: Core Integration
-   Add `DeviceConfig` presets in `crates/config`.
-   Implement specific peripheral hooks (e.g. hooking RP2040 SIO to core logic).

### Phase 3: Validation
-   Run standard Blinky/UART firmware binaries from unmodified SDKs (STM32Cube, Pico SDK, Zephyr) on the simulator.
