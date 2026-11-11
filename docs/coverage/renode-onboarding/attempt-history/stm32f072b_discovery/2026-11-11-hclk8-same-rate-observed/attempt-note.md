# HCLK/8 model configuration gap

The target booted and exercised the new clock-source assertion. It reported identical CVR decrements for `CLKSOURCE=0` and `CLKSOURCE=1` (`0x85` in both modes), so the external divider was not configured in the profile actually loaded by the imported system. The first edit had updated the shipped `configs/chips/stm32f072.yaml`, while this target loads the separate imported `configs/chips/onboarding/stm32f072.yaml`. The next attempt adds the divider to the imported chip descriptor and reruns only this target.
