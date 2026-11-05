# Validation contract

## Mandatory digital regressions

`g0_tiol112` covers exact SRAM/IRQ mapping, PLL64 derivation, rejection of a
16 MHz reset state under a 64 MHz timing label, APB-divider rejection, TI truth
table, EN suppression, enable/disable delays, local echo, propagation,
WAKE-to-EXTI and NFAULT. A no-enable-wait negative control must fail to produce
a complete CQ character; a later guarded character must pass. A second control
rejects UART traffic until the firmware's live RCC clock matches the timebase.
Its controlled-latency test is a measurement-oracle regression: it deliberately
inserts 12 µs software delay and proves that first CQ start is measured rather
than TC. It is not application execution evidence.

`modern_uart_timing` covers timed STM32 USART TC, RDR, PE/FE/ORE and interrupt
behavior. `iolink_master_config` integration tests cover the functional master's selected
M-sequence, explicit OD widths, current PDout and rejection of ignored config.

## Real firmware session

`g0_customer_timing` is explicitly ignored unless the operator provides an
ELF and runs `--ignored`. It loads that ELF through the normal loader and
executes its startup, EXTI, USART IRQ and polling code. No stack state is
patched by the harness. Startup and OPERATE happen through master frames.
The mandatory payload is a prepared 32-byte block chosen by current PDout;
PD byte 0 equals the selector, bytes 1..31 are independently checked against
the firmware's documented offset formula.

The report contains every request/reply, stop/start timestamps, tA, complete
reply duration, per-byte gaps, CPU frequency, PHY delays and the network event
trace. It separates STARTUP/PREOPERATE latency, 100 READ and 100 WRITE cases,
and eight application-rate cases with absolute 29.412 ms start intervals.
Invalid CKT must produce no response during its 5 ms observation window.
A tA
violation writes its artifact before failing. It may therefore be a useful
failure witness; a successful process exit is required for a passing claim.

For COM3, tA is measured as:

```
tA = first_device_CQ_start - final_master_CQ_stop_end
```

The bound is 1..10 bit times, approximately 4.340..43.403 µs at 230400 baud. RDR
RXNE occurs at the middle of the stop bit; using that instant would start the
timer too early by half a bit. The last reply's TC includes all 34 characters
(about 1.62 ms at COM3) and is also a different quantity.

Each device reply's stop-end to next-start byte gap is also checked against
0..3 bit times. Both limits come from the v1.1.4 specification's A.3.4/A.3.5.
The JSON is saved before the tA and maximum byte-gap gates are evaluated.

## Preserved measured evidence

The checked-in `evidence/` summaries pin firmware source, compiler, ELF hash,
runtime-model commit and the exact harness source hash. Large network traces
are represented by a hash of the raw report.

| Run | Coverage | Result |
|---|---|---|
| `type2v-com3-zig-disabled.json` | 100 READ + 100 WRITE + 8 application-rate replies | Timed C/Q model gate passed; 208 OPERATE replies, tA 26.473..39.769 µs, zero reply byte gaps |
| `type2v-com2-minimum-delay-failure.json` | 4 READ + 4 WRITE + 8 application-rate replies | Preserved failure: response starts too early for COM2; OPERATE tA 15.789..25.914 µs |
| `reference16-gcc-startup.json` | Actual 16 MHz GCC reference startup, DPP and DeviceOperate | Timed UART/EXTI/PHY prerequisite passed; this is not the 64 MHz PD32 profile |

The first two runs use the explicitly labeled Zig/Clang diagnostic runtime.
Their internal DLL timing enforcement is disabled; the external CQ tA and gap
gates are enabled. The COM2 failure is retained as a witness for the separate
minimum-response-delay fix. These results do not establish silicon timing.

## Explicit limits

Firmware DLL timing enforcement and the external oracle are separate. The
baseline profile inherits `IOLINK_TIMING_ENFORCE_DEFAULT=0`; setting the legacy
20 ms completion threshold does not enable its checks. The report records the
ELF-resolved `rig_timing_enforced` flag when exported, or an explicit
source-pinned `IOLINKI_G0_DLL_TIMING_ENFORCED=0|1` declaration for an older image.
The external CQ tA and byte-gap assertions remain enabled in both cases.
An enabled firmware variant is a separate artifact and must pass its own
receive-timestamp/timeout tests.

This proves a reproducible digital model result for a particular ELF. It is
not an IAR 9.70 build, a calibrated Cortex-M0+ worst-case execution model,
or a Beckhoff EP6228-0022 interoperability test. Interrupt-entry cycle costs,
flash wait states, bus contention and instruction timings need silicon
correlation before promising a hardware maximum. Dynamic clock switching is
outside the fixed operating-point model. TI's 4 µs enable/disable delays are
configurable typical scenarios, not guaranteed bounds. Cable/voltage/current,
protection, wake-current detection, EMC and conformance testing remain outside
this digital run. DMA/DMAMUX are unsupported and absent from the descriptor.

The full ISDU application service suite is separately exercised against the
real stack in the companion protocol fixture; cyclic timing tests alone do
not establish every ISDU segmentation/busy/retry behavior.

The ISDU acknowledgment timer is 5000 ms from receipt of the completed
request until the beginning of the device response. It is not a limit on the
whole segmented response transfer. See IO-Link Interface and System
Specification v1.1.4, June 2024,
[Table 102 and Table 53 transitions T4/T6](https://io-link.com/fileadmin/user_upload/Downloads/Package_2024/IOL-Interface-Spec_10002_V114_Jun24.pdf).
