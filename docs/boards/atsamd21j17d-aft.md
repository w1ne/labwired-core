# ATSAMD21J17D-AFT execution profile

The imported descriptor now has the J17D memory map: 128 KiB flash at
`0x00000000` and 16 KiB SRAM at `0x20000000`. Its execution profile contains
the SAM D21 PM, SYSCTRL, GCLK, and NVMCTRL register banks; modeled PORTA and
SERCOM0; and the Cortex-M0+ NVIC and SysTick. The imported zero-answering RTC,
timer, GPIO, and unrelated SERCOM windows are omitted.

## Smoke evidence

On 2026-11-03, `firmware-samd21-j17d-smoke` ran against
[`configs/systems/onboarding/atsamd21j17d-aft.yaml`](../../configs/systems/onboarding/atsamd21j17d-aft.yaml)
for 100,000 instructions. It emitted `samd21 sercom0 up` and passed both
configured assertions: UART output and the `max_steps` stop reason. The JUnit
report contains 3 cases (run plus two assertions), all passing.

Evidence files:

- [Onboarding metrics](../coverage/renode-onboarding/metrics/atsamd21j17d-aft/onboarding-metrics.json)
- [Simulator result](../coverage/renode-onboarding/metrics/atsamd21j17d-aft/simulation-output/result.json)
- [UART output](../coverage/renode-onboarding/metrics/atsamd21j17d-aft/simulation-output/uart.log)
- [JUnit report](../coverage/renode-onboarding/metrics/atsamd21j17d-aft/simulation-output/junit.xml)

This establishes a local LabWired CLI boot and the listed smoke assertions. It
does not establish complete peripheral coverage, full Renode parity, or
hardware behavior.
