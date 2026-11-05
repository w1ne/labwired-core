# Validation Report: polarfire-soc

**Architecture:** RISC-V RV64IMAC, one U54 application hart in M-mode.

## Renode Onboarding Smoke

**Status:** Booted and smoke-passing in the local LabWired CLI.

The imported descriptor was narrowed from a mislabeled ARM profile with broad
stub windows to the sourced PolarFire MSS memory map, one RV64IMAC U54, the
model-backed MMUART0/1, and explicit zero-answering startup stubs. The stubs do
not model PLIC, GPIO, I2C, or SPI behavior. The E51, other U54 harts, F/D,
supervisor mode, DMA, storage, and other MSS peripherals are outside this
profile.

The committed stock Zephyr v4.1.0 `mpfs_icicle` hello fixture booted on the
profile, printed its boot banner and `Hello World! mpfs_icicle/polarfire/u54`,
and stopped at the 60,000,000-cycle limit after executing 43,938 instructions.
Both smoke assertions passed. This is a local LabWired CLI simulation against
the bounded profile; it is not execution in Renode or on physical hardware.

| Evidence | Path |
|---|---|
| Current aggregate scoreboard | [Renode onboarding scoreboard](../coverage/renode-onboarding-scoreboard.md) |
| Per-target metrics | [onboarding-metrics.json](../coverage/renode-onboarding/metrics/polarfire-soc/onboarding-metrics.json) |
| Simulator result | [result.json](../coverage/renode-onboarding/metrics/polarfire-soc/simulation-output/result.json) |
| UART capture | [uart.log](../coverage/renode-onboarding/metrics/polarfire-soc/simulation-output/uart.log) |
| JUnit assertions | [junit.xml](../coverage/renode-onboarding/metrics/polarfire-soc/simulation-output/junit.xml) |

The initial profile attempt is retained under
`docs/coverage/renode-onboarding/attempt-history/polarfire-soc/`; it booted but
failed because the imported chip name did not select the existing MPFS U54
CLINT parameters.

## Bounded Profile

| Block | Declared profile |
|---|---|
| CPU | RV64IMAC U54 application hart, M-mode; CLINT at `0x02000000` |
| eNVM window | `0x20220000`, 128 KiB |
| DDR window | `0x80000000`, 8 MiB |
| L2 LIM | `0x08000000`, 512 KiB |
| MMUART0/1 | Model-backed DesignWare APB UARTs at `0x20000000` / `0x20100000` |
| PLIC, MSS GPIO0–2, I2C0/1, SPI1 | Explicit zero-answering stubs used during startup; behavior is not claimed |

The register map follows the [Microchip PolarFire SoC MSS Technical Reference
Manual](https://www.microchip.com/content/dam/mchp/documents/FPGA/ProductDocuments/ReferenceManuals/PolarFire_SoC_FPGA_MSS_Technical_Reference_Manual_VC.pdf)
and Zephyr v4.1.0's `dts/riscv/microchip/mpfs.dtsi` and `mpfs_icicle` board
description.
