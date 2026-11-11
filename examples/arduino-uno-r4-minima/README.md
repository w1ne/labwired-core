# Arduino UNO R4 Minima Onboarding Example

Run all commands from the repository root.

Demand context: Arduino kits rank third among kit categories in the measured
survey at 37%; RA4M1 has no separate MCU demand rank in that survey.

## Purpose

This example provides deterministic bring-up for Arduino UNO R4 Minima (R7FA4M1AB) using the supported subset:
1. `ra_sysc` (HOCO / OSCSF)
2. `port1` / `port3` (`ra_port`)
3. `sci2` UART (header Serial on P301/P302)
4. `iic1` RIIC master (BME280 on A4/SDA=P101 and A5/SCL=P100)
5. `systick`

USB CDC Serial is intentionally out of scope (USBFS is a stub window only).
IIC1 is a behavioral controller model attached directly to external devices;
the simulator does not enforce PFS electrical routing to the header pads.

**SIM-DERIVED.** Not silicon-verified. The local firmware reads BME280 chip
ID `0x60` over the modeled IIC1 bus and reports it on SCI2. Use a BME280
breakout rated for the board's 5 V host, with suitable level shifting and
pull-ups, when wiring real hardware.

## Quick Run

```bash
cargo build -p firmware-ra4m1-bme280-smoke --release --target thumbv7em-none-eabi
cargo run -q -p labwired-cli -- test --script examples/arduino-uno-r4-minima/bme280-smoke.yaml --output-dir out/arduino-uno-r4-minima/bme280-smoke --no-uart-stdout
```

Expected result:
1. smoke test passes
2. IIC1 trace includes the BME280 ID register `0xD0` and ID `0x60`
3. UART contains `UNO_R4_BME280_OK id=0x60`

## Files

1. `system.yaml`: local board mapping for simulation runs.
2. `bme280-smoke.yaml`: deterministic IIC1 + BME280 ID smoke assertion.
3. `REQUIRED_DOCS.md`: source-grounding references (R01UH0887, pinout; FSP PAC as address cross-check only).
4. `EXTERNAL_COMPONENTS.md`: external component declaration.
5. `VALIDATION.md`: reproducible validation/audit commands.
6. `vendor-status.md`: live store check and real-board voltage note.
