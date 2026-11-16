# Nuvoton NPCX9M6F EVB validation status

## Current measurement (2026-11-16)

The imported NPCX9M6F EVB system uses the same chip surface as
[`nuvoton_npcx9`](nuvoton_npcx9.md) (UART1..4, GPIO0..GPIOF,
ITIM32_1..6, ITIM64). It booted `firmware-nuvoton-npcx9-smoke` (SHA-256
`317a070b8d7d79453a51a977169e142e282efe8191ee2152dff11b77a1d1af39`) for
**400,000 instructions** and passed **9/9 smoke assertions**, with the same
UART capture (UART1..4 share one capture stream):

```text
NPCX9 UART1 smoke
itim regs=1 en=1 run=1 stop=1 irq=3 sts_cleared=1 sts=1 masked=1 pend=1 rel=1 each=6 w64=1 borrow=1 lfclk=1 ok
gpio ports=16 latch=1 drive=1 pull=1 od=1 lock=1 ok
cr_uart2 tx
cr_uart3 tx
cr_uart4 tx
uarts tbe=3 rx=3 isolated=1 ok
uart irq fts=4 txp=4 txrel=4 frs=3 rx=3 hits=3 drained=3 ok
```

Evidence:
[`docs/coverage/renode-onboarding/metrics/nuvoton_npcx9m6fb_evb/`](../coverage/renode-onboarding/metrics/nuvoton_npcx9m6fb_evb/).
Checks, claimed peripherals, sources, model limits and negative controls are
in [`nuvoton_npcx9.md`](nuvoton_npcx9.md).

The NPCX9 boot-ROM handoff and the board LED wiring are not modelled. The
profile's `verified` field remains false. This result is limited to the
local model behaviour; it is not datasheet validation or hardware
confirmation.
