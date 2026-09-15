# Teensy 4.1 Onboarding Example

Run all commands from `core/`.

## Purpose

This example provides deterministic bring-up for Teensy 4.1 using the minimal supported subset:
1. `ccm` (`imx_ccm` — CCGR ungating)
2. `iomuxc` (`imx_iomuxc` sticky stub)
3. `gpio2` (`imxrt` — DR / GDIR / DR_TOGGLE)
4. `lpuart6` (`nxp_lpuart` — Teensy Serial1)
5. `systick`
6. `flexspi` stub only (no XIP)

Teensy 4.1 silicon is **MIMXRT1062**; the chip yaml is the **RT1064-class** cousin (GPIO/LPUART/CCM class). Do not claim 4MB SiP flash or FlexSPI XIP — SRAM/DTCM linked only.

**SIM-DERIVED.** Not silicon-verified.

## Quick Run

```bash
cargo build -p firmware-imxrt1064-demo --release --target thumbv7em-none-eabi
cargo run -q -p labwired-cli -- test --script examples/teensy-41/uart-smoke.yaml --output-dir out/teensy-41/uart-smoke --no-uart-stdout
```

Expected result:
1. smoke test passes
2. UART contains `OK`

## Files

1. `system.yaml`: local board mapping for simulation runs.
2. `uart-smoke.yaml`: deterministic UART smoke assertion.
3. `REQUIRED_DOCS.md`: source-grounding references (IMXRT1060RM, PJRC pinout).
4. `EXTERNAL_COMPONENTS.md`: external component declaration.
5. `VALIDATION.md`: reproducible validation/audit commands.
