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
line status, RX threshold, functional RX timeout, latched THRE and modem
interrupts. DMA and physical baud/busy/modem timing remain unsupported.
The RX timeout uses the fixed functional interval described below. UCV/CPR/DLF
report zero. Linux's exact DLF all-ones probe and RE_EN 1/0 probe writes are
ignored and read back zero; real fractional-divisor/RS485 requests are refused.

Console/RX attachment uses the existing UART capabilities, and stream peers
attach through `attach_uart_stream_by_id`. Attaching a protocol peer detaches
console capture. Peer polling uses the machine's existing central device-time
drive, including its initial time anchor, with a maximum 16 polls per peer per
slice. The optional UART4 slice, behavioral ESP-AT peer and external supply gates
are described below. Timed peer wake scheduling, ESP firmware and RF remain
unsupported.

Focused tests cover actual native Machine retirement and timer IRQ/WFI edges,
masked wake, no-progress waits, programming boundaries, code-identity swaps,
and single/batch equivalence. They do not establish Linux boot or hardware
acceptance. Driver evidence is pinned Linux 6.18.56 irq-gic.c, arm_arch_timer.c,
proc-v7.S and 8250 sources; register mapping follows the pinned RV1106-family
vendor dtsi, with actual RV1103 compatibility retained for the board fixture.

`configure_armv7_rv1103_i2c3()` separately opts in to RK3x v1 at
`ff460000/1000`, named `i2c3`, driving GIC INTID53 (SPI21). It requires the
existing ARM fabric and refuses duplicate names or overlapping peripheral or
CPU memory mappings. It changes no foundation DT, manifest, or artifact pin.
After controller installation, `attach_manifest_external_devices()` uses the
existing manifest part-pack → PeripheralKit → declarative resolver, recording
placements before kits resolve them. Duplicate IDs and repeated attachment
are errors. A failed attachment can leave earlier parts attached: discard the
incomplete bus. The normal `attach_i2c_slave_with_route` funnel trace-wraps the
canonical ADXL345 kit; no RV-specific sensor register or axis state exists.
SDA/SCL routes are accepted as fixed-controller placement metadata; this slice
does not simulate pinmux or physical pads.

The RK3x model accepts only aligned little-endian 32-bit MMIO. CON, CLKDIV,
MRXADDR, MRXRADDR, MTXCNT, MRXCNT, IEN and W1C IPD follow the bounded upstream
sequence; mode3, reserved CON bits and MRXADDR lanes other than valid lane0
are errors. The eight-word TX/RX buffers hold at most 32 bytes. FCNT reports
the last successful chunk's completed byte count (including the first TX
address); its synthetic reset-to-zero is not a hardware reset-value claim.
START raises pending START only. Count writes execute chunks, strip the first
TX address, and continue the same selected slave without restarting. Combined
reads send zero to three valid register-prefix bytes once, then start the read
phase on the same instance. Plain RX continuations preserve the read cursor.
LASTACK is a terminal master NACK and does not set NAKRCV. Successful chunks
set only MBTF/MBRF; byte-level BTF/BRF are omitted.

Pending interrupts survive masking, and level `(IPD & IEN & 0x7f) != 0`
is reconciled with the GIC on writes and events. START/STOP conditions are
processed once per rising request bit. STOP releases a live selected slave
once and raises STOP even without a target or with IEN=0. CON.EN=0 abandons
internal selection for the driver's restart sequence without fabricating an
inner STOP. Full controller reset clears its registers, buffers, IRQ and
selection while preserving the sensor's state. Address claims are checked
before select/start/write/read/stop and on continuations. Absent/static-off
parts generate only NAKRCV with no target calls or completion; a failed
transaction remains failed until STOP/restart. ACTACK=0 deliberately retains
this bounded stop-on-NACK behavior: Linux I2C_M_IGNORE_NAK is unsupported,
with no synthesized target success. The latest 256 controller diagnostic
entries include START/STOP/NAK and IRQ transitions; successful component
traffic remains in the existing canonical bus trace.

Slaves forward nested stimulus discovery, inspect artifacts, named logs and
pin queues through the established helpers. Each slave receives central
device time exactly once per elapsed microsecond; MMIO and peek do not
advance it. CLKDIV/tuning only store functional clock configuration. DMA,
CRU/reset/pinmux programming, physical clock/SCL/baud timing, arbitration and
clock stretching are unsupported. Host protocol tests exercise the unchanged
Linux driver sequence, canonical ADXL345 DEVID/register/axis traffic and two
runtime stimuli. They are not a Linux guest module-load or hardware proof.

The unchanged driver reference is Linux 6.18.56
[`i2c-rk3x.c`](https://raw.githubusercontent.com/gregkh/linux/v6.18.56/drivers/i2c/busses/i2c-rk3x.c),
SHA256 `cee9b45dfee3fb4a7168585603c88df5e3422493425ac1bd35046d509d910423`.
The SDK's genuine `rockchip,rv1106-i2c`/`rockchip,rk3399-i2c` fallback selects
v1 timing. Any later basket DT must preserve those identities and disclose
its synthetic fixed functional clocks; the accepted foundation stays separate.

`configure_armv7_rv1103_uart4()` separately installs UART4 at
`ff4e0000/100`, named `uart4`, with SPI29 / GIC INTID61. It applies the same
atomic fabric/name/window/memory checks as the I2C3 opt-in. CPU IRQ sampling
reconciles only the two registered ARM UART indices, including UART4 epochs
before reading GIC pins, without scanning peripherals. UART2 and the base
Linux fixture remain independent. DW UART RX interrupt priority is line
status, FIFO threshold, receive timeout, THRE, modem status. Below-threshold
nonempty RX interrupts after a fixed **functional 4000us** idle interval;
this is not physical divisor/baud timing. Real accepted arrivals, RBR reads,
and RX clear reset the interval; DLL reads and peeks do not. Shared legacy RX
queue changes are detected by a bounded content signature at actual service.

The `esp32c3_esp_at` kit attaches through the ordinary UART stream-host
capability. It is an explicitly **behavioral mock**, with no ESP firmware,
Wi-Fi networking, RF, or inferred radio power load. Strict CRLF frames accept
`AT`, `AT+GMR`, `AT+LABMOCKSTATUS?`, `AT+LABMOCKCONNECT`, and
`AT+LABMOCKDISCONNECT`. GMR names the LabWired mock, and connection/status are
mock state only. Malformed/unknown frames return `ERROR\r\n`. Payloads hold at
most 256 bytes; overflow drains to the next CRLF and yields one ERROR. The
single reply queue holds at most 1024 bytes. A saturated queue emits ERROR
when it fits or drops a complete frame, with a bounded log; rejected frames
cannot change mock connection state. At most one reply byte is released per
poll with >=1000us accumulated elapsed time. Excess credit is discarded.
Service intervals and guest FIFO drains must prevent 16-byte RX overrun;
this is functional pacing rather than baud accuracy. Common input, logs,
inspection and shared BusTrace report the actual peer/host state.

Before attaching routed parts, register one external functional domain:
`bus.register_supply_domain("external3v3", 2.7)?`. Set both ADXL345 and ESP
placement config to `supply_domain: external3v3, supply_min_v: 2.7`. The
threshold is a disclosed functional gate assumption, not silicon behavior.
Domains start off and accept `bus.update_supply_domain(name, voltage_v)?`
only for known names and finite nonnegative voltages. Incompatible domain
registration/placement thresholds are errors. Task4 must feed the existing
plant output before each guest window; these gates contain no battery model
and do not replace the host execution SupplySupervisor.

Unrouted legacy parts remain on; `powered: false` stays off permanently.
I2C input/identity/log/inspection forwarding survives cutoff, while all inner
bus, clock and pin operations are suppressed. Sensor registers are retained
across cutoff as an explicit functional assumption. Off/on domain transitions
change a shared generation, while healthy voltage updates do not. RK3x
checks the selected device generation on every transaction phase/chunk,
including a cut+restore with no intervening controller service.

A routed ESP requires the optional **exclusive generation-aware UART host**
contract. DW accepts it only on an empty UART/RX FIFO, detaches console
capture, refuses additional kit peers and console capture, and exposes no
shared RX injection handle while exclusive. Normal fallible bus and kit
attachment paths reject exclusive-host conflicts and generation-aware peers
requiring explicit exclusive attachment before changing console capture or
peer state. Direct low-level callers must
use the exclusive attach method and cannot mix host injections with this
protocol FIFO. This restriction makes it safe to invalidate already delivered
host RX on any domain generation change at actual RX/IRQ/poll service.
Peeks observe invalidation without mutating queues, timers or IRQ state.
Cutoff discards ESP partial frames, queued replies and mock connection state.

The focused component/controller tests establish these shared functional
contracts. **Basket Linux guest exchanges remain NOT_RUN in Task3**; real
module insertion/helper execution and plant/full-host-power acceptance are
Task4 gates. Physical board, rail, footprint, thermal and RF gates remain
pending while hardware is unavailable. Local dogfood requires no npm
publication or manual deployment.
