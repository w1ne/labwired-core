# Validation — SAM4S Xplained

Claims, evidence, and what is explicitly **not** claimed yet.

## Tier status

| Capability | Tier | Evidence |
| :--- | :--- | :--- |
| Boot from ELF via the entry point (PC init) | smoke-tested | run below: `Initial PC: 0x00400000` (flash base) |
| Cortex-M4 executing the committed firmware | smoke-tested | `labwired test` 3/3 checks, 4,000,000 steps |
| Flash (0x00400000, 1 MiB) + SRAM (0x20000000, 128 KiB) load + execute | smoke-tested | firmware links into flash and runs with the stack in SRAM |
| PMC peripheral clock enables (`PCER0`) + master-clock-ready (`SR`) | smoke-tested | UART0/PIO accesses work after the firmware enables them |
| UART0 console (8N1, BRGR 65 at 120 MHz) | smoke-tested | banner + heartbeat on the TX sink |
| SAM4S PIO output (PER/OER/SODR/CODR/ODSR) | smoke-tested | `led=on odsr=00000400`, `led=off odsr=00000000` |
| SAM4S PIO input with pull-up (PIOA PA5) | smoke-tested | `btn=1` (released) |
| Generic analog-to-ADC route on EXT1 PA17/AD[0] and PA18/AD[1] | co-sim tested | `cosim_sam4s_analog::sam4s_xplained_ntc_and_diode_netlists_convert_through_pa17_pa18_adc`; the same netlists resolve through `analog_pins` and convert on the `sam4s_adc` model (scan, user sequence, 12/10-bit) |
| Generic analog-to-GPIO route on EXT1 PA17/AD[0] and PA18/AD[1] | co-sim tested | `cosim_sam4s_analog::sam4s_xplained_ntc_and_diode_netlists_reach_ext1_pio_inputs`; Beta NTC and Shockley diode voltages solve to distinct pads and are asserted through PDSR with test-only thresholds |
| Determinism (same inputs → same output) | smoke-tested | two runs, identical UART streams (`diff` clean) |
| Instruction support for this firmware | audited | `unsupported_instruction_audit.sh`: 200,000 instructions, 0 unsupported |
| TC0/TC1, SPI0, TWI0/TWI1, EEFC, RSTC/SUPC/RTT/WDT/RTC, CHIPID | **not modelled** | declared `type: stub` |
| Silicon capture / executing-fidelity differential | **none** | SIM-DERIVED |

## Commands and evidence (verified 2026-10-23)

```bash
# from core/
cargo build -p firmware-sam4s-xplained-smoke --release --target thumbv7em-none-eabi
#   Finished `release` profile — warning-free

./target/release/labwired test \
  --script examples/sam4s-xplained/io-smoke.yaml \
  --output-dir out/sam4s-xplained-smoke --no-uart-stdout
#   PASS  3/3 checks · io-smoke · 4000000 steps · 1.46s

./target/release/labwired \
  --firmware target/thumbv7em-none-eabi/release/firmware-sam4s-xplained-smoke \
  --system configs/systems/sam4s-xplained.yaml --max-steps 400000
#   Initial PC: 0x00400000
#   SAM4S XPLAINED SMOKE / cpu=cortex-m4 uart=uart0
#   led=on odsr=00000400 / led=off odsr=00000000 / btn=1 / hb=0 …

# Determinism: two runs, identical streams
… | grep -E '^(SAM4S|cpu=|led=|btn=|hb=)' > run1
… > run2 ; diff run1 run2   # clean

./scripts/unsupported_instruction_audit.sh \
  --firmware target/thumbv7em-none-eabi/release/firmware-sam4s-xplained-smoke \
  --system configs/systems/sam4s-xplained.yaml \
  --max-steps 200000 --out-dir out/unsupported-audit/sam4s-xplained
#   simulator exit 0 · Instructions executed: 200000 · Unsupported: 0

cargo test -p labwired-core --lib peripherals::sam4s::tests
#   5 passed; 0 failed

cargo test -p labwired-core --test firmware_survival test_sam4s_xplained_smoke_survival
```

## Notes

- The SAM4S PIO is the older parallel-I/O block; the model is `sam4s_pio`
  (a new engine peripheral), not the SAM D21 `PORT` block.
- The button's released level is the PIO's internal pull-up (enabled by the
  firmware); the board's external pull-up is not separately modelled.
- The PMC model reports MOSCXTS|MCKRDY from reset — the reset RC clock is
  running, so firmware does not need to reprogram the PLL. PLL bring-up is not
  exercised by this smoke.
- TC/SPI/TWI/EEFC/system-controller blocks are stubs.

## Analog pad routing (verified 2026-11-06)

```bash
cargo test -p labwired-core --test cosim_sam4s_analog -- --nocapture
# 2 passed: generic NTC and Shockley diode netlists route to PA17/PA18 PDSR,
# and (2026-11-08) through analog_pins into ADC AD0/AD1 conversions
```

The PDSR test uses test-only GPIO thresholds and claims no silicon threshold
values. The ADC test (`sam4s_xplained_ntc_and_diode_netlists_convert_through_pa17_pa18_adc`)
converts the solved voltages through the `sam4s_adc` model: an ascending
CHER scan with EMR.TAG, a SEQR1 user sequence at MR.LOWRES, EOC/DRDY
read-to-clear, and OVRE/GOVRE overruns. Codes are SIM-DERIVED against the
datasheet's register description; no SAM4S bench conversion was captured.
