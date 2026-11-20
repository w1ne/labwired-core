# I2C1 runner feature omitted

The target-only inventory run booted the imported STM32F429 system but failed the new I2C1 UART assertion. The firmware feature was enabled in CI but missing from `SMOKE_MAPPINGS` in `scripts/run_renode_onboarding_inventory.py`, so `smoke_i2c1` was not compiled into the binary. This is a runner configuration failure, not an observed I2C hardware-model failure. The run output and metrics are retained here; the mapping was then corrected before the target was rerun.
