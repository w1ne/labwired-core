# UNO R4 Minima SCI2 survival fixture refresh

**Classification:** maintenance on a demand-ranked board; this refresh adds no
board or MCU. Arduino kits ranked 3rd at 37% in the measured demand brief. The
smoke itself remains the UNO R4 Minima SCI2 UART path.

## Failure identity

- PR head: `03fa1ffa9b9a937e56f7dcd5cd6f0a9a5a769fc0`
- Board/system: Arduino UNO R4 Minima / `arduino-uno-r4-minima`
- Firmware source: `crates/firmware-ra4m1-demo/src/main.rs`
- CI case: `firmware_survival::test_ra4m1_uno_r4_smoke_survival`
- Oracle: UART output contains `OK`
- CI failure: [run 44031361462](https://github.com/w1ne/labwired-core/actions/runs/44031361462), feature-off survival case received an empty UART transcript.

The committed fixture predated the new SCI2 active-low MSTP gate. Its digest was
`6141e6b7a115bc2c6e13467ec7ff77b88fe9650ebd51569969233186d7c1e62e`; the
source firmware already clears MSTPCRB bit 29 before writing SCI2. Disassembly
confirmed the old ELF had no PRCR write at `0x4001E3FE` or MSTPCRB access at
`0x40047000`; the refreshed ELF performs both before the SCI2 writes, so the
current model correctly kept the old image's SCI2 clock-stopped.

## Ranked cause

**C1 — stale firmware fixture -> SCI2 remains gated -> UART oracle is empty.**
For: the CI/local failure is specific to the old committed ELF; the new source
clears MSTPCRB bit 29; replacing only the ELF with a fresh build makes the same
case pass. Against: no physical board capture is part of this evidence. The weak
link was the old ELF's MMIO sequence; disassembly and the single-artifact repair
now confirm it directly.

**C2 — survival harness/profile mismatch -> SCI2 output is not captured.**
For: this is another path than the standalone CLI smoke. Against: the exact same
survival harness and board profile pass after only the ELF changes, so the
harness mismatch candidate is rejected.

Cheapest discriminating test: rebuild the source fixture and rerun the focused
feature-off survival case; C1 predicts `OK` after refresh, while C2 predicts an
empty transcript until the harness/profile changes.

## Repair and evidence

Rebuilt the fixture from the checked-in firmware source with
`examples/arduino-uno-r4-minima/ci/refresh-survival-fixture.sh`. The updated
fixture digest is
`dd358f86cd6f00d9a61656332da78c5589c4e7160ad3cace11bd34d3222a0f07`.

The exact feature-off survival case passed after refresh. Its committed output
is in [`logs/fixture-survival.log`](logs/fixture-survival.log). The default
feature graph also passed as a regression control; its output is in
[`logs/fixture-survival-default.log`](logs/fixture-survival-default.log). The
build log is [`logs/fixture-refresh-build.log`](logs/fixture-refresh-build.log).
The ordinary UNO R4 BME280 smoke evidence remains in this directory and still
exercises the IIC1 sensor-ID read independently of this SCI2 regression check.

No physical board test was run; this closes a simulator fixture mismatch only.
