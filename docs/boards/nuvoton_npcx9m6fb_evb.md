# Nuvoton NPCX9M6F EVB validation status

## Current measurement (2026-11-16)

The imported NPCX9M6F EVB system uses the same chip surface as
[`nuvoton_npcx9`](nuvoton_npcx9.md) (UART1..4, GPIO0..GPIOF,
ITIM32_1..6, ITIM64, MTC, TWD, HFCG, LFCG, SPIP, and the code RAM, data RAM view,
battery-backed RAM, image_type and PM1..4 memory windows). It booted
`firmware-nuvoton-npcx9-smoke`
(SHA-256 `31a428efb777bf6e8140594cdef81c6514316afcc82a3bd0f221bad4eb4d8314`),
rebooted once through the TWD watchdog, ran to **100,000,000 cycles**
(567,218 instructions) and passed **16/16 smoke assertions**, with the same
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
mtc ttc=1 pt=1 irq=3 steps=1 ms1=1000 ms2=1000 ok
twd regs=1 t0=1 irq=3 ms1=1000 ms2=1000 tc=1 lock=1 unlock=1 run=1 touch=1 dec=1 stop=1 ok
cdcg load=1 div=1 hlock=1 lf=1 llock=1 ok
mem code=1 data=1 image=1 pm=4 bb=1 ok
spip reset=1 off=1 x8=1 x16=1 cfg=1 quiet=1 rirq=1 rclr=1 wirq=1 wclr=1 ok
twd expire .........|twd reboot wdrst=1 stopped=1 clr=1 t0=1 ok
bbram kept=1 ok
```

Evidence:
[`docs/coverage/renode-onboarding/metrics/nuvoton_npcx9m6fb_evb/`](../coverage/renode-onboarding/metrics/nuvoton_npcx9m6fb_evb/).
Checks, claimed peripherals, sources, model limits and negative controls are
in [`nuvoton_npcx9.md`](nuvoton_npcx9.md).

The NPCX9 boot-ROM handoff and the board LED wiring are not modelled. The
profile's `verified` field remains false. This result is limited to the
local model behaviour; it is not datasheet validation or hardware
confirmation.
