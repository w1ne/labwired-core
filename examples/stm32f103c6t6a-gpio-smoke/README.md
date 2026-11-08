# STM32F103C6T6A bare-part GPIO smoke

This is exact-part coverage for **STM32F103C6**, the rank 3 MCU family in the
JetBrains State of Embedded survey (17%). This round fills the C6 smoke gap in
the requested C8/C6/R8/R6/T4/T6 low-density and medium-density coverage; it is
an additional variant within the already-modelled STM32F103 family.

The fixture represents an STM32F103C6T6A in LQFP48 with an external LED from
3.3 V through a current-limiting resistor to PC13. PC13 is therefore active-low.
The firmware enables GPIOC, configures PC13 as push-pull output, then toggles
the LED. The linker and chip descriptors both constrain the part to 32 KiB of
flash and 10 KiB of SRAM.

The profile claims only the C6T6A memory geometry, RCC clock gate, GPIOC and
the PC13 LED connection. It does not claim USART, ADC, timer, SPI, I2C, USB or
CAN behavior. The broader `configs/chips/stm32f103c6.yaml` remains a catalog
profile that reuses the medium-density peripheral map.

## Run

```sh
scripts/onboarding_smoke.sh \
  --target-id stm32f103c6t6a-gpio-smoke \
  --crate firmware-stm32f103c6-gpio-smoke \
  --target thumbv7m-none-eabi \
  --script examples/stm32f103c6t6a-gpio-smoke/io-smoke.yaml \
  --system configs/systems/stm32f103c6t6a-gpio-smoke.yaml \
  --out-dir examples/stm32f103c6t6a-gpio-smoke/evidence
```

The committed `evidence/` directory holds the build and smoke logs, GPIO edge
assertion output and run metrics. The live part-status check is recorded in
[`evidence/vendor-status.md`](evidence/vendor-status.md).
