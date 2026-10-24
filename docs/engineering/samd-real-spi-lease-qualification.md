# Prospective actual blocking SPI terminal-result ownership

## Landed actual checkpoint — 2026-10-15

Runtime32 merged `f9a6525e104e28e42c529e1da8718dc900744b4d`; Core237 merged
`068c2c1a370f266ac97399aff8592303ea423d6e`. Both runtime and all23 enabled
engine checks passed, including all workspace shards and final aggregates;
only seven declared optional engine jobs skipped. Reviewed/tested/landed trees,
exact parents and default refs were verified.

Final engine source `a6352784c056eb4b6f37caa795263357054acdba` at tree
`12a2297840c8e80bc38d1dea333bd0b0a1f28fd9` passed
[all four actual cases38678769414/job116238283467](https://github.com/CrispStrobe/labwired-core/actions/runs/38678769414/job/116238283467)
and a frozen independent source/image-bound original reader. Both lease profiles
finished at158067 guest steps,31 prefix+28 recovery observations and10 swaps:
two accepted DMA bytes, no wire delivery before CS release, remaining30/32,
actual NotifyOne matching enrollment, competing BUSY before the original owner
consumed DEVICE_SPI_ERROR, live fiber-bound helper returns and actual Event/main
resume. The direct settled boundary at91831 has zero DMA/SPI IRQs; recovery then
has two entries/returns for each. Both positive controls finish at126844 steps,
28 observations and six swaps. These counts are not RTx.

[Full exact images, original hashes, initial failures and limits](https://github.com/CrispStrobe/codal-samd/blob/c82a5dd53edcfe95fa6aadd3454593db33efa340/docs/REAL-SPI-BLOCKING-LEASE-RESULTS.md)
preserve the initial symbol-borrow compilation failure, incorrect optional-wire
mutation expectation and later Clippy-only failure. Fixes did not weaken actual
assertions, bounds or floors. Driver/model and earlier callers remain unchanged;
no consumer pin moved. Natural stale IRQs/timeouts, object lifetime and complete
native Arcade/image/licence/installed/performance admission remain separate.
The [current continuation contract](samd-runtime-replacement-plan.md#current-continuation-after-actual-spi-callers)
scopes the next actual lifetime and board/image tasks.

## Retained prospective acceptance contract

Mode14 from immutable runtime `0f952e5399c0723f8312b9f9a7d767272856d099`
adds a caller only; driver/model and prior callers remain unchanged. Preserve
the [qualified software-pended idle IRQ checkpoint](samd-real-spi-stale-qualification.md).
This new slice is not qualified by the earlier asynchronous cancellation result.

Require a real blocking owner and distinct cancelling fiber. Observe the actual
blocking wrapper entry, enrollment through real `fiber_wake_on_event` and a
partial32-byte RAM transfer. Cancellation must disable the channel and release
CS, deliver the real NotifyOne event for that exact enrolled value, and return
success to the cancelling worker without invoking an asynchronous failure.

The worker immediately attempts a valid competing start before yielding. Observe
its actual call/LR return with DEVICE_BUSY before the original owner consumes
the terminal result. Observe the original blocking wrapper's return register
DEVICE_SPI_ERROR, original fiber identity and actual owner/main Event path;
only then admit the unchanged28-row positive async/blocking recovery. Read exact
constants from immutable selected headers, never invent error/notification codes.

Fiber entry functions share a trampoline return address. Bind every helper
return to the live upstream `currentFiber`, not just a PC or guest counter.
Keep raw helper entries/returns and identities, enrollment, actual NotifyOne
constructor/fire return, channel-disable bytes, every partial DATA/wire byte,
cleanup snapshots and remaining writeback count. Masking alone does not stop
DMA; snapshot remaining count only after channel disable. Preserve partial wire
progress without assuming zero bytes; no new DATA/wire/CS effect is allowed
after cancellation settles before recovery.

Build mode14 and positive mode8 with separate clean immutable dependency trees,
execute both scheduler profiles and retain all four outcomes even on failure.
Require nonvacuous deletion, wrong identity/result/event, extra-effect and ordering
refusals. Freeze a source/image-bound independent reader before acquiring the
sole actual original. Preserve full original IRQ counts and every initial
failure; do not replay guests to inspect evidence. Keep startup/vector/data/BSS,
compiler/linker/resource gates and the unchanged four-million-step bound. Every
enabled final-head check and affected actual caller must pass before matched
merges and reviewed/tested/landed tree/parent/ref verification. A shared driver
or model repair needs fresh affected earlier caller qualification too.

Keep driver, Timer, MessageBus and buffers alive throughout. Destructor lifetime,
natural stale IRQs/timeouts, complete board/image/native Arcade, installed app,
RTx, physical behavior and licence/distribution remain separate. No consumer
pin, threshold, acknowledgement or physical capture changes.
