# Validation record

## Demand rank and target

MSPM0G3507 has no separate measured MCU rank in the demand survey. TI LaunchPad
kit use is measured at 7%, so this is strategic vendor coverage rather than a
top-demand MCU pick. ESP32 remains rank 1 at 38% measured use.

## Model and smoke scope

The board system selects the MSPM0G3507 M0+ core, 128 KiB flash and 32 KiB SRAM,
then maps UART0's TXDATA register, a CTL0 EN/TXE capture gate, and a static TXFE
status bit. The firmware image is linked at flash address 0, uses the MSPM0
reset clock assumption for its 115200 8N1 setup, enables UART0 TX, and writes
the smoke marker. The run is limited to 300,000 steps and asserts both the
marker and the configured max-step stop.

The captured marker proves Cortex-M0+ execution and UART0 register-side TX
traffic. It does not verify GPRCM power state, IOMUX/pad routing, UART timing or
FIFO behavior, RX, GPIO, timers, ADC, onboard sensors, LEDs, or an electrical
XDS110 path. Those blocks are absent from this focused system profile rather
than described as working models. The board pin map is in [`board-map.md`](board-map.md).

The local deterministic smoke is not a physical-board result or the hosted
LabWired model-oracle verification. Current vendor lifecycle and buyability
evidence is in [`vendor-status.md`](vendor-status.md).

## Reproduce

```sh
examples/lp-mspm0g3507-launchpad/ci/test.sh
```

The latest committed build/run logs, CLI result, UART capture, and generated
`onboarding-metrics.json` are under [`evidence/`](evidence/).
