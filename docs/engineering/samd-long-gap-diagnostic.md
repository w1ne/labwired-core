# Actual Timer long-gap diagnostic and repair qualification

The real service/clock slice landed through
[PR222](https://github.com/CrispStrobe/labwired-core/pull/222), merge
`708da30a2492f2d0c0503de5e7ab4b6a208def2a`, after22 enabled checks passed.
This next diagnostic is not whole native Arcade or a repaired time contract.

The preserved first diagnostic pins own runtime source
[`07de1e4bbc0910a5f6b192df13c5df3a6fe467af`](https://github.com/CrispStrobe/codal-samd/commit/07de1e4bbc0910a5f6b192df13c5df3a6fe467af),
whose [five-image build38465804291](https://github.com/CrispStrobe/codal-samd/actions/runs/38465804291)
passed. Actual core remains `2ed19d64ec3359143409a4b0a9758b4aeef3ee7d`;
Timer and clock implementations are unchanged for this first diagnostic.

The guest arms a real20 ms Event, masks interrupts, waits80 ms through actual
DWT-based low-level captures, and reads Timer only before/after the gap.
The pinned Timer narrows elapsed to16 bits. Its `sigma` and millisecond remainder
are also16-bit; changing just the elapsed cast is not a sufficient repair.
The [actual diagnostic run38472304691](https://github.com/CrispStrobe/labwired-core/actions/runs/38472304691)
reproduced the exact failure in both profiles at9660466 steps: low-level
elapsed80002 us versus Timer14474 us, no IRQ/Event/switch under the mask.
The frozen original-log audit passed as expected bug reproduced after delayed
official step metadata became available. Its initial metadata refusal and the
sole original remain preserved; no audit rule was relaxed.

The existing eight cases retain their four-million-step bound. Only the new
long case receives a separately named twelve-million-step bound. Both profiles
must bind exact source/image/map/startup and actual architectural pending
exception15 under PRIMASK, with no IRQ entry, Event or context switch. Guest
COUNTFLAG is recorded separately from the architectural pending observation.

The first diagnostic accepts only guest assertion `0xe0000c35`, with actual
low-level elapsed80–81 ms and Timer elapsed lower by approximately65536 us
within the source-defined overhead bound. Raw ordered stage/pending/time
witnesses must support that exact outcome. Timeout, a different panic,
architectural fault or arbitrary manufactured marker cannot qualify it.
A passing diagnostic means **expected bug reproduced**, not correct time.

Preserve the actual first original before any Timer repair. Then review counter
width/modular arithmetic, `sigma` and remainder together; qualify microseconds
and milliseconds, overdue Event delivery and all original cases against the
fixed source. No host clock advances, pending-bit mutation, injected Event or
success stub. Default physical builds, historical image rejection, app pins,
captures, acknowledgements and performance thresholds remain unchanged.

## Separately named positive qualification — pending

The next fixture pins own runtime
[`0c95081222a840af0701b25417a8f73bf6842225`](https://github.com/CrispStrobe/codal-samd/commit/0c95081222a840af0701b25417a8f73bf6842225),
whose [five-image clean build38478663383](https://github.com/CrispStrobe/codal-samd/actions/runs/38478663383)
passed. It records41 explicit inputs, including the attributed Timer-width
patch and control/build/provenance scripts. Each image passed480 source-bound
integer controls with six source-refusal controls. These cover declared
counter widths, wrap and millisecond remainder arithmetic; they are not
hardware/clock/compiler/guest qualification.

The repaired caller additionally records actual `getTime()` milliseconds.
The positive host harness requires stage4 and `0x600d`, restored80ms us/ms
intervals, exactly one overdue Event from SysTick, one architectural IRQ
entry/return pair, no assembly switch, and no final pending exception. Raw
stage2 must still show pending SysTick under PRIMASK before any Event or entry.
It uses separately named FIXED boundary/result records, preserving the old
diagnostic source and original. Both profiles and all eight previous cases
must pass exact-source guest qualification; the old bounds remain unchanged.

This fixture pin is not consumer/app adoption. No repaired guest result,
whole-runtime admission, native Arcade frame/input/debugger or RTx outcome
is established by the clean build or prospective assertions.
