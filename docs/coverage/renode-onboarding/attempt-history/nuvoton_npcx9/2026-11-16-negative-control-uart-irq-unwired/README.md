# Negative control: NPCX9 UART IRQ lines not wired

- Date: 2026-11-16
- CLI: lane debug build with the NPCX FIFO-mode UART registers and level IRQ.
- Firmware: `firmware-nuvoton-npcx9-smoke`, SHA-256
  `317a070b8d7d79453a51a977169e142e282efe8191ee2152dff11b77a1d1af39`
  (the same ELF as the passing run).
- Descriptor: `configs/chips/onboarding/nuvoton_npcx9.yaml` at 50912a434
  (cr_uart1..4 without `irq`).
- Smoke script: the current one (injections "bB", "cC", "dD").

Result: FAIL 8/9. `uart irq fts=4 txp=0 txrel=4 frs=3 rx=0 hits=0 drained=0
FAIL`. The UFTSTS / UFRSTS values are correct; no UART line pends and the RX
handler never runs, so the second byte stays queued.
