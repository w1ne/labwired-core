# Negative control: FE310 GPIO without its PLIC source, same CLI, firmware and script

- CLI: `labwired` built 2026-11-16 03:38 UTC by `run_renode_onboarding_inventory.py
  --only sifive-fe310` from the GPIO-interrupt change.
- Chip: `chip-gpio-unwired.yaml.txt`, a copy of
  `configs/chips/onboarding/sifive-fe310.yaml` without the GPIO `irq: 8`. The
  GPIO model keeps its interrupt registers but drives no PLIC source.
- Firmware: the 2026-11-16 FE310 GPIO-interrupt smoke, SHA-256
  `46ad0fc090ddda2a6d333f275fda0df0c5b47a0ce4fcd7cbdd215489af21504b`.
- Result: FAIL 2/4 checks at 400,000 steps. `gpio irq rise ... rise_ip=00000004
  ... pending=0 disabled_low=1 meip=0 high_relatch=1 ... FAIL`: the latch sets,
  but PLIC source 10 never pends and MEIP stays low. The trap probe then waits
  for an interrupt that does not come, so the later lines are absent. The
  lines before it still pass.
