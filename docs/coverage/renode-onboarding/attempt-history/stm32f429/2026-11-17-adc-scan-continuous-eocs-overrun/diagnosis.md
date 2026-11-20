# STM32F429 continuous ADC scan attempt — overrun

Command:

```bash
CARGO_BUILD_JOBS=1 CARGO_INCREMENTAL=0 python3 scripts/run_renode_onboarding_inventory.py --only stm32f429
```

The one-shot two-rank scan still passed. The first continuous-mode smoke used
`EOCS=1` and attempted to drain every conversion in firmware. It observed the
expected channel sequence (`IN1, IN2, IN1, IN2, IN1`), but the ADC's 15-cycle
conversion cadence outpaced those MMIO reads and set OVR. The stop check also
raced a subsequent conversion because the ADC was disabled after clearing the
NVIC pending bit.

Captured UART:

```text
adc1 scan_cont=000003FF,000007FF,000003FF,000007FF,000003FF wrap=1 irq=1 ovr=1 stop=0 FAIL
```

This attempt remains a failure record. The follow-up checks `EOCS=0`, observes
the end-of-group interrupt three times while reading the final rank result,
then disables the ADC before clearing NVIC pending. ST RM0090 §13.3.8 specifies
that `EOCS=0` reports EOC at the end of each regular group and `CONT` restarts
the sequence at the first selected channel.

Scoreboard aggregation was attempted, but the runner's Python environment did
not have PyYAML (`generate_onboarding_scoreboard.py` exited 2); the existing
aggregate was therefore left untouched.
