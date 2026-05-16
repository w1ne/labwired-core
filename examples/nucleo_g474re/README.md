# nucleo_g474re

This example was synthesized by Foundry as a board onboarding starter.

## Scope

- App-core boot path
- RCC/GPIO/UART baseline
- Board LED and user button mapping
- Vendor example selection reference only

## Recommended Vendor Example

Reference package: STM32CubeG4

- STM32CubeG4 UART_Printf: Caller-requested workload reference for the onboarding proof.

## Required Source Confirmation

- Confirm MCU part number and package
- Confirm VCP UART instance on ST-LINK
- Confirm LED and button GPIO mappings from schematic
- Confirm deferred subsystem scope matches the requested onboarding contract

## Auto-Resolved Source Docs

- `datasheet` required: /tmp/g474_docs/stm32g474re-datasheet.txt
- `supporting_doc` required: /tmp/g474_docs/nucleo-g474re-board.txt
- `supporting_doc` required: /tmp/g474_docs/stm32g474-reference.txt
- `supporting_doc` required: /tmp/g474_docs/stm32cubeg4-example.txt
