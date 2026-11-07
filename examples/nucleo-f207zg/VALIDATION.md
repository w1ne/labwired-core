# NUCLEO-F207ZG validation

The reproducible local smoke builds `firmware-nucleo-f207zg-smoke`, boots it
against `configs/systems/nucleo-f207zg-smoke.yaml`, and requires at least 16
GPIO transitions on PB0 before the instruction limit. The evidence directory
records the CLI build, firmware build, simulator output, assertion result, and
onboarding metrics.

This validates only the Cortex-M3 reset path, RCC AHB1 GPIOB gate, and GPIOB
output path attached to LD1. No peripheral from the known-stub allowlist is
used. ST-LINK/VCP, USB, Ethernet, buttons, and other STM32F207 behavior remain
unclaimed. The hosted LabWired catalog currently returns NOT_FOUND for
`nucleo-f207zg`, so there is no hosted `labwired_verify` verdict for this
target; do not describe the result as hosted model verification.
