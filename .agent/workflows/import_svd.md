---
description: How to import SVD files into LabWired
---

# Importing SVD Files

The `labwired-cli` tool includes experimental support for importing CMSIS-SVD files and converting them into the LabWired Intermediate Representation (IR).

## Command

```bash
cargo run -p labwired-cli -- experimental import-svd --input <path/to/device.svd> --output <path/to/output.json>
```

## Example

```bash
# Convert an STM32F103 SVD to LabWired JSON
cargo run -p labwired-cli -- experimental import-svd --input stm32f103.svd --output stm32f103.json
```

## Output Format

The output is a JSON file conforming to the `labwired-ir` schema, containing:
- Device metadata (name, description)
- Peripherals (base address, registers, fields)
- Interrupt mapping
