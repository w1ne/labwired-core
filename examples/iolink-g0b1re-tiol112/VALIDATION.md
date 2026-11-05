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

The same suite loads the exact NUCLEO-G0B1RE board manifest and uses a real
`Machine` to exercise UART transmit, local echo, EN release and idle receive
in both legacy and event-scheduler builds. Register-only component checks use
the explicit peripheral-only service boundary. The Machine test uses the same
`timed_uart_sync` handoff as World's timed UART rounds after external peers
enqueue wire activity; this verifies scheduler dispatch rather than treating
a forced peripheral walk as a scheduler test.

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
| `type2v-com3-zig-enforced.json` | 100 READ + 100 WRITE + 8 application-rate replies, internal monitor enabled | Timed C/Q model gate passed; 208 OPERATE replies, tA 28.004..41.301 µs, zero reply byte gaps and zero internal timing errors |
| `type2v-com2-zig-enforced.json` | 100 READ + 100 WRITE + 8 application-rate replies, internal monitor enabled | Timed C/Q model gate passed; 208 OPERATE replies, tA 29.132..31.679 µs, zero reply byte gaps and zero internal timing errors |
| `type2v-stream-event-scheduler.json` | Actual ELF with the interactive one-action UART master and event scheduler | Functional gate passed: current selectors 7/9, READ34/WRITE33; this stream test does not measure CQ tA |
| `type2v-com3-zig-disabled.json` | 100 READ + 100 WRITE + 8 application-rate replies | Timed C/Q model gate passed; 208 OPERATE replies, tA 26.473..39.769 µs, zero reply byte gaps |
| `type2v-com2-minimum-delay-failure.json` | 4 READ + 4 WRITE + 8 application-rate replies | Preserved failure: response starts too early for COM2; OPERATE tA 15.789..25.914 µs |
| `reference16-gcc-startup.json` | Actual 16 MHz GCC reference startup, DPP and DeviceOperate | Timed UART/EXTI/PHY prerequisite passed; this is not the 64 MHz PD32 profile |

The enabled COM3 and COM2 runs use companion source
[`10e8196e2630c54474fbec16abbb301f515c7736`](https://github.com/w1ne/iolinki/commit/10e8196e2630c54474fbec16abbb301f515c7736)
and distinct, hash-pinned Zig/Clang diagnostic ELFs. This source adds a board
response floor anchored to the last receive IRQ, retains the separate 6 µs
PHY enable guard, and applies the internal cycle monitor to valid OPERATE
frame starts. The earlier disabled-monitor COM3 pass and COM2 minimum-delay
failure remain separate, unchanged witnesses.

The enabled runs verify the exported flag and the DLL's actual state through
an ELF-hash-checked DWARF layout. Both finish with zero byte, cycle, completion,
aggregate timing and timeout counters. The deliberate invalid CKT produces
one CRC and one framing diagnostic with no reply. Each run also checks
STARTUP/PREOPERATE tA, all 32 current-selector payload bytes, checksums, UART
parity/framing, complete driver release, and eight exact 29.412 ms application
start intervals. These results establish the stated digital model scenarios,
not silicon timing or a GCC/newlib/IAR acceptance run.

The local required gates at this checkpoint are 29 focused default-feature
tests, 15 event-scheduler tests, and the explicitly invoked real-ELF stream
test. The component suite includes a genuine Machine scheduler path. The
remaining broad PR checks run in CI; this is still a scoped draft extension.

## Fresh verification of the observable lab

The `*-recheck.json` reports are new executions against core source checkpoint
[`06dfd2395224a66d7093aca674800b2850b751fe`](https://github.com/w1ne/labwired-core/commit/06dfd2395224a66d7093aca674800b2850b751fe).
They use the same hash-verified final firmware images above and leave the
earlier evidence unchanged. Both full COM3/COM2 runs again pass with the same
208-reply tA ranges, zero reply byte gaps and zero internal timing violations.

Each fresh run records 6,977 received reply characters with zero UART parity
or framing errors, 211 validated reply checksums including startup, and 6,656
verified process-data bytes. Every byte of all 208 PD32 replies is checked,
including the eight application-cadence cases. The single intentional bad CKT
still produces no reply; its DLL CRC/framing diagnostics are distinct from
the zero UART decoding errors.

The oracle also reads and checks the actual firmware setup after wake:
USART1 on PA9/PA10 AF1, PA8 output, PB0/PB1 inputs with pull-ups, 8E1 framing,
the selected COM BRR, TIM2 PSC63 and PLL64. The browser diagram lowering uses
the same UART/pads, explicit 64 MHz operating profile, and TYPE2V32/1/OD1
master configuration. The generated chip, board and kit metadata caches were
verified byte-identical to the core files.

Fresh local checks pass: 29 default-feature tests, 25 event-scheduler tests,
and the explicitly invoked real-ELF one-action fixture. That fixture now also
asserts the UI-facing phase is OPERATE and process data is valid. The additive
browser state serializer passes `cargo check -p labwired-wasm` on the available
host target. A wasm32 browser build and deployed execution are separate CI
and release gates; no local wasm32 toolchain was available. The final-runtime
GCC artifact was unavailable during this verification, so diagnostic Clang
results are not substituted for GCC acceptance.

## Explicit limits

Firmware DLL timing enforcement and the external oracle are separate. The
baseline profile inherits `IOLINK_TIMING_ENFORCE_DEFAULT=0`; setting the legacy
20 ms completion threshold does not enable its checks. The report records the
ELF-resolved `rig_timing_enforced` flag when exported, or an explicit
source-pinned `IOLINKI_G0_DLL_TIMING_ENFORCED=0|1` declaration for an older image.
The external CQ tA and byte-gap assertions remain enabled in both cases.
An optional ELF-hash-checked DWARF layout allows read-only snapshots of the
actual DLL state and counters, including field widths that differ between
compilers. Enabled images with this probe must finish with zero timing errors.
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

The interactive master profile currently drives startup and cyclic process
data; it does not issue ISDU services. The hosted generic `uart_frames` and
`uart_frame_response` oracles decode 8N1 pad edges, so they do not validate
this 8E1 C/Q link. Shared labs must label live byte-functional traces,
recorded native C/Q measurements and the separate ISDU protocol proof by
their actual execution path and source/artifact identity.

The ISDU acknowledgment timer is 5000 ms from receipt of the completed
request until the beginning of the device response. It is not a limit on the
whole segmented response transfer. See IO-Link Interface and System
Specification v1.1.4, June 2024,
[Table 102 and Table 53 transitions T4/T6](https://io-link.com/fileadmin/user_upload/Downloads/Package_2024/IOL-Interface-Spec_10002_V114_Jun24.pdf).
