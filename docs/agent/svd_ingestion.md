# SVD Ingestion Pipeline

LabWired supports ingesting CMSIS-SVD files to automatically generate `PeripheralDescriptor` YAMLs. This automated pipeline ensures high fidelity and saves manual effort when onboarding new chips.

## Overview

The `svd-ingestor` tool parses standard `.svd` files and converts them into the LabWired YAML format.

## Usage

Run the ingestor using cargo from the `core` directory:

```bash
cargo run -p svd-ingestor -- \
  --input <PATH_TO_SVD> \
  --output-dir <OUTPUT_DIRECTORY> \
  --filter <PERIPHERAL_NAMES>
```

### Arguments

*   `--input`: Path to the source `.svd` file.
*   `--output-dir`: Directory where generated `.yaml` files will be saved.
*   `--filter`: (Optional) Comma-separated list of peripheral names to process (e.g., `USART1,RCC`). If omitted, all peripherals are processed.

## Integration with LabWired AI

Agents can use this tool to "ground" their knowledge. When an agent identifies a chip, it should:
1.  Locate the SVD file (or ask the user to provide it).
2.  Run the ingestor to generate canonical peripheral definitions.
3.  Use these definitions in `system.yaml`.

## Example

Generating descriptors for an STM32F401's RCC and USART2:

```bash
cargo run -p svd-ingestor -- \
  --input core/tests/fixtures/real_world/stm32f401.svd \
  --output-dir core/examples/my_board/peripherals \
  --filter RCC,USART2
```
