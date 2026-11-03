# NUCLEO-F446RE

This first board profile is intentionally limited to the paths exercised by
`io-smoke.yaml`: reset-clock bring-up, USART2 output through the ST-LINK VCP,
and LD2 GPIO transitions. The USER button is mapped as board I/O but is not
read by this smoke. Other F446 peripherals are omitted from the chip profile;
accesses to them fault as unsupported.

Build and run the committed smoke from the repository root:

```bash
scripts/onboarding_smoke.sh \
  --target-id nucleo-f446re \
  --crate firmware-f446-demo \
  --target thumbv7em-none-eabi \
  --script examples/nucleo-f446re/io-smoke.yaml \
  --system configs/systems/nucleo-f446re.yaml \
  --out-dir examples/nucleo-f446re/evidence
```

The archived run logs and UART/GPIO evidence are under `evidence/`.
