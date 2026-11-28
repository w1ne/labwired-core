# STM32F429 continuous ADC scan attempt — stale group EOC

Command:

```bash
CARGO_BUILD_JOBS=1 CARGO_INCREMENTAL=0 PATH=/usr/bin:/home/andrii/.cargo/bin:/usr/local/bin:/bin /usr/bin/python3 scripts/run_renode_onboarding_inventory.py --only stm32f429
```

The imported platform booted 800,000 instructions. The one-shot scan passed,
and the continuous scan reached three EOC events without OVR. Its final-rank
data reads were `0x07FF, 0x03FF, 0x07FF`, however, and EOC remained visible
after disabling the ADC because the old EOC acknowledgement had been lost:

```text
adc1 scan_cont_groups=000007FF,000003FF,000007FF groups=0 irq=1 ovr=0 stop=0 FAIL
```

Cause: after ADC_DR was read at group end, the model masked EOC using a latch.
It reset that latch after the next non-final conversion, allowing the previous
group's EOC to reappear during the new group. The fix keeps the acknowledgement
latched until the next EOC event. Firmware now disables the ADC before testing
that EOC remains clear and before clearing NVIC pending.

The generated scoreboard correctly recorded this measured result as
**226 booted; 225 boot-and-smoke passed; 1 failed; 19 not run**. That failure is
preserved in this attempt directory. The refreshed aggregate will be produced
only after the corrected target run.
