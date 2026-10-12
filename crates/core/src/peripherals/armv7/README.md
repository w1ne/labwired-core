# Initial RV1103 functional platform

`SystemBus::configure_armv7_rv1103()` installs one GIC distributor at
`ff1f1000/1000`, its CPU interface at `ff1f2000/2000`, and UART2 at
`ff4c0000/100`. Set a nonzero `cpu_hz` and supply memory before executing.
Construction adds peripherals to the existing bus; it does not remove any
previously installed devices or install a Linux image. Default buses remain
inert. All native build dependencies remain behind `qemu-arm-system`.

The fabric owns GIC levels. These entries deliberately carry no legacy NVIC
IRQ number. Physical timer CNTP drives INTID29; virtual CNTV drives INTID27,
never INTID30. The board's reduced DT advertises physical PPI13/14; independent
TrustZone timer banks, hypervisor interfaces and multiple CPUs are unsupported.
GIC bitmap banks are canonical u64 words and IRQ selection walks only eligible
set bits; no duplicated source-state cache exists. GIC EOImode0 is implemented; EOImode1 and software active-priority injection
are explicit errors. IRQ selection respects group enables, PMR, BPR, active
priority, target CPU0 and persistent level sources across EOI.
SGIs 0–15 are permanently enabled; enable-bank writes to those bits are
ignored, as specified for the [GIC-400 register profile, Table 3-2](https://documentation-service.arm.com/static/5e8f15e27100066a414f7424?token=).
SGI pending-source registers expose only CPU0; nonexistent CPU source bits
read zero and ignore writes.

The fixed 24 MHz architectural timer derives an absolute u128 count from the
bus's modeled cycle and CPU Hz. It has no host clock and no independent tick
owner. Signed TVAL and 64-bit CVAL use an extended compare epoch so deadline
conversion cannot wrap CPU ticks; deadlines beyond u64 CPU time are absent.
The virtual counter keeps a fixed extended epoch through CNTVOFF underflow and
low-64-bit wrap; offset writes shift its counter without rebasing a stored compare.
One retired native instruction contributes one modeled tick, which does not
claim physical CPU timing. CNTKCTL retains EL0 access, direction and event
index controls; EVNTEN is explicitly refused before mutation. The reduced
Linux fixture must use the supported early parameter
`clocksource.arm_arch_timer.evtstrm=0`; its pinned config otherwise enables
CONFIG_ARM_ARCH_TIMER_EVTSTREAM. WFE event-stream delivery is unsupported.

Upstream Cortex-A7 ACTLR reads zero, and its c10 wildcard reads PRRR/NMRR as
zero. The platform explicitly accepts Linux's ACTLR=0x40, PRRR=0xff0a81a8 and
NMRR=0x40e040e0 startup writes (also zero), as reductions with no cache timing
or memory-type effects. It does not fabricate readback storage. Other external
CP15 accesses remain errors. Native MMU ownership, short descriptors and
LPAE refusal remain unchanged; vendor sources are untouched.

UART registers have stride4 and exact aligned 1/2/4-byte transactions. TX is
immediate; RX capacity is one byte without FIFO enable and 16 with it. Host
injection is reconciled and truncated to capacity before CPU IRQ sampling or
register reads, recording overrun. The existing host queue is an input staging
queue and callers must bound their enqueues; the emulated FIFO is bounded.
Console capture retains the latest 64 KiB. DLAB divisor writes are never TX.
Logical MCR loopback implements modem status and internal RX. IIR implements
line status, RX threshold, latched THRE and modem interrupts. DMA, physical
baud/busy/modem timing and RX timeout interrupts are unsupported. UCV/CPR/DLF
report zero. Linux's exact DLF all-ones probe and RE_EN 1/0 probe writes are
ignored and read back zero; real fractional-divisor/RS485 requests are refused.

Console/RX attachment uses the existing UART capabilities, and stream peers
attach through `attach_uart_stream_by_id`. Attaching a protocol peer detaches
console capture. Peer polling uses the machine's existing central device-time
drive, including its initial time anchor, with a maximum 16 polls per peer per
slice. The seam supports later UART4/ESP-AT work; timed peer wake scheduling,
power behavior and a real ESP-AT peer are separate tasks.

Focused tests cover actual native Machine retirement and timer IRQ/WFI edges,
masked wake, no-progress waits, programming boundaries, code-identity swaps,
and single/batch equivalence. They do not establish Linux boot or hardware
acceptance. Driver evidence is pinned Linux 6.18.56 irq-gic.c, arm_arch_timer.c,
proc-v7.S and 8250 sources; register mapping follows the pinned RV1106-family
vendor dtsi, with actual RV1103 compatibility retained for the board fixture.
