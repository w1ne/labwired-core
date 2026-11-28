# STM32F429 continuous ADC scan attempt — stop boundary

Command:

```bash
CARGO_BUILD_JOBS=1 CARGO_INCREMENTAL=0 PATH=/usr/bin:/home/andrii/.cargo/bin:/usr/local/bin:/bin /usr/bin/python3 scripts/run_renode_onboarding_inventory.py --only stm32f429
```

The imported platform booted 800,000 instructions. The one-shot scan passed;
the continuous scan observed three group-end results of `0x07FF`, an IRQ, and
no OVR. The stop assertion still found EOC set immediately after disabling the
ADC, consistent with a conversion completing at the stop boundary:

```text
adc1 scan_cont_groups=000007FF,000007FF,000007FF groups=3 irq=1 ovr=0 stop=0 FAIL
```

The follow-up disables the ADC, clears any completion flag that latched before
the disable took effect, confirms ADON reads clear, and then verifies EOC stays
clear for 64 cycles. This attempt remains preserved as a measured failure.
The generated scoreboard records **226 booted; 225 boot-and-smoke passed; 1
failed; 19 not run**.
