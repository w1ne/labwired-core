---
description: How to import SVD files into LabWired
---

# Importing SVD Files

The `labwired-cli` tool includes experimental support for importing CMSIS-SVD files and converting them into the LabWired Intermediate Representation (IR).

## Command

```bash
# Convert an STM32F103 SVD to LabWired YAMLs
cargo run -p svd-ingestor -- --input stm32f103.svd --output-dir ./peripherals
```

## Output Format

The output is a JSON file conforming to the `labwired-ir` schema, containing:
- Device metadata (name, description)
- Peripherals (base address, registers, fields)
- Interrupt mapping
