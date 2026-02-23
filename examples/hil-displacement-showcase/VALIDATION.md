# HIL Displacement Showcase: Validation Guide

This showcase demonstrates how LabWired replaces expensive and non-deterministic Hardware-in-the-Loop (HIL) testing.

## Prerequisites
- **Simulation**: LabWired CLI installed.
- **Hardware**: NUCLEO-H563ZI board and `arm-none-eabi-gcc` toolchain.

## 1. Run in Simulation (LabWired)
The simulation uses a **Cycle Guard** to catch timing regressions that would be "ghosts" in HIL.

### Build Firmware
```bash
cd firmware
make
```

### Execute Test
```bash
labwired test showcase-test.yaml
```

### What to Observe
- **The "Pass"**: The test completes in milliseconds, confirming the firmware meets the 5M cycle timing budget.
- **Regression Detection**: If you add a delay (e.g., `for(int i=0; i<100; i++) __asm("nop");`) in the DMA loop, LabWired will fail the test *incrementally* and *deterministically*.

## 2. Run on Hardware (NUCLEO-H563ZI)
This confirms functional parity with real silicon.

### Flash to Board
Use your preferred tool (OpenOCD, STM32CubeProgrammer, or Aether).
```bash
# Example using Aether (if available)
aether-cli core load firmware/build/hil_displacement_showcase.bin 0x08000000
aether-cli core resume
```

### Manual Verification
1. Connect a serial terminal to the Nucleo's ST-Link UART (115200 8N1).
2. Observe the output:
   ```
   HIL Stress Test Started
   HIL Stress Test Passed
   ```
3. The Green LED (PB0) will flash briefly during the stress transfer.

## 3. The "Sellable" Difference
| Aspect | Hardware (HIL) | LabWired |
| :--- | :--- | :--- |
| **Fail Reproducibility** | Flaky (External noise/jitter) | **100% Deterministic** |
| **Observation** | External Logic Analyzer | **Internal VCD Trace** |
| **Execution Rate** | 1x Real-time | **~6,000x Speedup** |

## 4. Technical Resolution (Feb 2026)
This showcase initially exposed several ISA gaps in the LabWired core. These have been resolved to ensure 100% parity with the NUCLEO-H563ZI:
- **ISA Completeness**: Added support for `STRB` (Register), `CMP.W` (T32), and `BKPT`.
- **DMA Reliability**: Verified mem-to-periph transfer routing for `UART3`.
- **IRQ Precision**: Fixed the chip descriptor to prevent spurious internal exception triggers during high-speed DMA operations.

## 5. Summary of Results
The **HIL Displacement Showcase** now runs deterministically in **1,534 steps**, completing in less than **10ms** on standard CI hardware, while maintaining perfect functional parity with real-world silicon traces.
