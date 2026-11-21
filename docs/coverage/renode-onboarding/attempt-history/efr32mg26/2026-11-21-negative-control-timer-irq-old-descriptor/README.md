# Negative control: EFR32MG26 TIMER0..9 IRQ lines, old descriptor (2026-11-21)

Firmware: `firmware-mg26-demo` with the TIMER0..9 NVIC-line check.
Script: `examples/brd2709a/timer-irq-smoke.yaml`.
Descriptor: `configs/chips/efr32mg26.yaml` from `origin/main` a1b1bf6b0
before this change. `timer1..8` have no `irq`; `timer9` has `irq: 5`.

Result: `MG26-TIMER-IRQ 0010 0000 0000 0000 0000 0000 0000 0000 0000 0020 FAIL`;
the smoke fails 2/3. TIMER1 pends no line; TIMER9 pends line 5 (TIMER1's
vector). With `irq: 5..13` on `timer1..9` the same firmware reads
`0010 0020 0040 0080 0100 0200 0400 0800 1000 2000 ok`.
