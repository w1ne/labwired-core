# nRF52 SAADC held-input functional contract

This original MIT slice makes configured SAADC conversions respond to held
analog input levels. It does not qualify micro:bit microphone audio capture.

The register facts and conversion equation follow Nordic's
[SAADC product specification](https://docs.nordicsemi.com/r/bundle/ps_nrf52832/page/saadc.html)
for the shared nRF52 SAADC IP. Buffer/scan semantics additionally follow
[nRF52833 Product Specification v1.7](https://docs.nordicsemi.com/bundle/nRF52833_PS_v1.7/resource/nRF52833_PS_v1.7.pdf)
([manufacturer PDF mirrored by ANU](https://comp.anu.edu.au/courses/comp2300/assets/manuals/nRF52833_PS_v1.7.pdf)),
§§6.21.3–6.21.4, pp577–579. The nRF52833 instance is SAADC at `0x40007000`,
IRQ7. The [micro:bit Foundation pinmap](https://tech.microbit.org/hardware/schematic/)
assigns `MIC_IN` to P0.05/AIN3 and `RUN_MIC` to P0.20. A microphone fixture must
drive AIN3 through SAADC, rather than the PDM peripheral.

## Input and code mapping

`Peripheral::set_adc_channel_input(ain, millivolts)` holds one physical AIN0..7.
`adc_channel_count()` returns 8. `clear_adc_channel_input(ain)` releases it.
Invalid physical input indices return false without altering an input. The
WASM `set_adc_channel_millivolts("saadc", 3, millivolts)` and
`clear_adc_channel("saadc", 3)` use these generic hooks. The existing STM32 ADC
downcast remains the compatibility fallback.

CH[n] is a converter configuration slot, not an AIN or GPIO index. PSELP/PSELN
values 1..8 select AIN0..7, so any CH[n].PSELP=4 selects the microphone pad.
PSELP=0 disables a slot; invalid selectors are skipped. PSELN is ignored in
single-ended mode. In differential mode NC is modeled as ground. Selector9
uses the explicit modeled 3300mV supply, also used by VDD/4 reference.

The configured gain choices are 1/6, 1/5, 1/4, 1/3, 1/2, 1, 2 and 4.
Reference is 600mV internal or 825mV modeled VDD/4. Integer rational arithmetic
computes `(VP-VN) * gain/reference * 2^(resolution-m)`, where m=0 for
single-ended mode and m=1 for differential mode. Division truncates toward
zero. Single-ended results saturate to 0..2^N-1; differential results saturate
to -2^(N-1)..2^(N-1)-1. DMA stores signed little-endian 16-bit samples.

Un-driven AINs are explicitly modeled at ground, including after releasing the
last held input. There is no synthetic 3000mV fallback and no conversion for
disconnected slots. Nordic Tier-1 fixtures explicitly select VDD (PSELP=9)
and expect 3754 at 12-bit / 938 at 10-bit with the modeled 3300mV supply.

## Functional START/scan contract and remaining work

START requires ENABLE and latches PTR/MAXCNT, resets AMOUNT, arms acquisition
and generates STARTED. Edits to PTR/MAXCNT affect the next START, not the active
buffer. Each accepted SAMPLE visits enabled CH[n] slots once in ascending order
and appends their results. A partial final scan stops at MAXCNT without writing
past it; END fires only when the buffer fills, then disarms acquisition. SAMPLE
before START, after END or while another scan is pending is ignored. MAXCNT is
bounded to 32767 samples; zero capacity and no valid enabled slots generate no
samples or conversion events. Existing event latches require software clear.

A delay-zero scheduler event or the bare-bus tick drains each scan, not an
entire buffer. Only one scheduler entry is announced per trigger; generation
tokens prevent canceled entries from completing newer triggers. STOP/disable
cancel pending scans and disarm, preserving partial AMOUNT. A new START resets
AMOUNT and cancels any previous pending scan. Input/configuration is evaluated
when the scan drains; this zero-delay functional model is not acquisition-time
sampling. Invalid/non-RAM PTR and DMA bus-write errors remain unsupported;
successful RAM writes are proven only with valid RAM buffers.

DONE and RESULTDONE latch on nonempty scans; END latches only at buffer-full.
Their enabled bits raise
the level IRQ. Nordic event bits use `(event offset-0x100)/4`, so STARTED=0,
END=1, DONE=2, RESULTDONE=3, CALIBRATEDONE=4 and STOPPED=5. Event clear and
INTENCLR deassert the line. Acquisition time, oversampling/BURST, local timed
sampling, calibration, limit events and resistor networks are not modeled.

START/scan unit gates cover bare bus and scheduled completion, sparse channels,
partial buffers/sentinels, pointer edits, amount/events/IRQ and canceled-event
restart. Hosted native source proof passed at `18a8cf70`: 26 SAADC unit tests,
the new ARM scan guest, four tick512 EasyDMA tests and three native WASM routing
tests ([run36713727900](https://github.com/CrispStrobe/labwired-core/actions/runs/36713727900)).
The [receipt](../receipts/2026-09-30-microbit-saadc-scan-hosted-proof.json) records
the exact tested merge ref separately from the PR head and the missing ADC ELF
artifact limitation. Main landing and the separate whole-chip CorePerf baseline
gate remain pending; this is not browser ADC or microphone/audio qualification. The earlier
local exact-source stub harness is not engine proof. Rebuild
migrated fixture blobs with `scripts/tier1/build_nordic_rp2040.sh --nordic-only`.

A second native run at `ee80c19b` preserved the actual hosted ADC ELF, verified
SHA256 `3875d7473117d105a34abf561c37e1a087915ea775fa99c7eb687d32d3cbc5b0`.
The same 297 selected functional test executions passed, but the old CPU-base
motion benchmark failed at median 0.865205x / min 0.862687x. Its
[failed-baseline receipt](../receipts/2026-09-30-microbit-saadc-scan-ee80-proof.json)
and original motion/GPIO JSONs are retained separately from the earlier green
host result. The stacked CPU/GPIO/SAADC candidate is not qualified by either
historical run; it needs fresh combined hosted proof.

The stacked candidate at `e734e675` now passed native combined qualification
([run36720954929](https://github.com/CrispStrobe/labwired-core/actions/runs/36720954929)):
311 core plus three native WASM selected test executions, including all 26 SAADC
unit tests, the actual ARM scan guest and four tick512 EasyDMA tests. On the
recorded AMD EPYC 9V45 runner, active median was 6.326197x and motion median/min
1.996539x/1.988887x. The [combined receipt](../receipts/2026-09-30-microbit-saadc-scan-combined-proof.json)
distinguishes the PR head from tested merge `cfd20eb6`, retains original JSONs,
complete selected functional log and runner context, and records the downloaded
ADC ELF SHA256 `f07e2f81dadc606e1d958c198807db291c57367804f82542c455cd3c4803ae5e`.
This qualifies that native candidate, not browser ADC, microphone capture or
the separate whole-chip baseline gate. Host variation prevents interpreting
cross-run score changes as a measured SAADC optimization.

Before qualifying microphone capture, implement and prove sample
timing/PPI/oversampling, RUN_MIC gating and an explicitly bounded waveform
source. Microphone bias, sound-pressure calibration and analog noise require
separate board fixtures. No vendor firmware or analog hardware model is added
by this slice.
