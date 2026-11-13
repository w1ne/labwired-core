# ATSAMD21J17D-AFT execution profile

The imported descriptor has the J17D memory map: 128 KiB flash at
`0x00000000` and 16 KiB SRAM at `0x20000000`. Its execution profile contains
the SAM D21 PM, SYSCTRL, GCLK and NVMCTRL register banks; modelled PORT groups
A and B; SERCOM0, SERCOM1 and SERCOM3 as USARTs; TC4 and TC6 (`samd21_tc`);
the RTC (`samd21_rtc`); and the Cortex-M0+ NVIC and SysTick. That is the set
of blocks Renode's `platforms/cpus/atsamd21j17d-aft.repl` models (Renode's
`usart2` at `0x42001400` is SERCOM3 on silicon and is named that here).

`samd21_tc` (`crates/core/src/peripherals/sam/tc21.rs`) models COUNT8/COUNT16,
the NFRQ/MFRQ tops, up/down counting, ONESHOT, the RETRIGGER/STOP commands,
STATUS.STOP and the OVF/MC0/MC1 interrupts, gated by the instance's
PM.APBCMASK bit. It counts CPU cycles / prescaler (GCLK routing is not
modelled). COUNT32, capture, events, output inversion and pad waveforms are
refused or not modelled.

`samd21_rtc` (`crates/core/src/peripherals/sam/rtc21.rs`) models all three
modes: MODE0 (32-bit COUNT, COMP0, MATCHCLR), MODE1 (16-bit COUNT, PER,
COMP0/COMP1) and MODE2 (clock/calendar in 24- or 12-hour form, leap years,
ALARM0 with MASK.SEL, MATCHCLR), with the datasheet's next-edge flag timing,
the IRQ 3 line and the PM.APBAMASK.RTC gate. Renode's `SAMD21_RTC` covers
MODE2 only. GCLK routing is not modelled: GCLK_RTC is a fixed 1.024 kHz
(OSC32K / 32, the RTCZero/ASF setup) unless the chip declares `gclk_hz`.
EVCTRL events and FREQCORR are refused.

## Smoke evidence

On 2026-11-08 (Round 379) the firmware gained an RTC check and the run grew to
800,000 instructions; all 5 assertions pass. The RTC lines in the ordered
transcript cover MODE2 (2024-02-28 23:59:58 rolls into the leap day, ALARM0
at 00:00:01 raises its flag, pends IRQ 3 and releases after W1C + ICPR),
MODE0 (COMP0 = 3 with MATCHCLR: CMP0 and OVF together, counter restarted) and
MODE1 (PER = 4: CMP0, CMP1, then OVF). The summary word is now `0x5D21003F`.
With the `rtc` window removed, the same firmware bus-faults at `0x40001400`
and passes 2/5
(`attempt-history/atsamd21j17d-aft/2026-11-08-negative-control-no-rtc/`).

Round 377 (TC model):

On 2026-11-08 (Round 377), `firmware-samd21-j17d-smoke` ran against
[`configs/systems/onboarding/atsamd21j17d-aft.yaml`](../../configs/systems/onboarding/atsamd21j17d-aft.yaml)
for 400,000 instructions and passed all 5 assertions: the console banner; an
ordered UART transcript covering PB10 DIR/OUT readback, TC4 (CC0 readback,
counter advance, CC0 match + OVF, IRQ 19 pending in the NVIC, W1C clear, CMD
STOP), TC6 (8-bit down one-shot: STOP at reset, OVF, PER reload, self-stop,
RETRIGGER, STOP) and SERCOM1/SERCOM3 (CTRLA readback, DRE, TXC, TXC W1C);
at least 4 edges on the PA17 LED; the summary word `0x5D21001F` at
`0x20002000`; and the `max_steps` stop reason. With TC4/TC6 removed from the
chip, the same firmware bus-faults at `0x42003000` and passes 2/5
(`attempt-history/atsamd21j17d-aft/2026-11-08-negative-control-no-tc/`).

Evidence files:

- [Onboarding metrics](../coverage/renode-onboarding/metrics/atsamd21j17d-aft/onboarding-metrics.json)
- [Simulator result](../coverage/renode-onboarding/metrics/atsamd21j17d-aft/simulation-output/result.json)
- [UART output](../coverage/renode-onboarding/metrics/atsamd21j17d-aft/simulation-output/uart.log)
- [JUnit report](../coverage/renode-onboarding/metrics/atsamd21j17d-aft/simulation-output/junit.xml)

This establishes a local LabWired CLI run and the listed smoke assertions. It
does not establish complete peripheral coverage, full Renode parity, or
hardware behavior.
