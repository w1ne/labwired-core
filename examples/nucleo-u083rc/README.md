# NUCLEO-U083RC

This narrow board profile boots firmware for the STM32U083RCT6 and toggles
the board's green user LED LD4 on PA5 (Arduino D13). It declares the U0 RCC
GPIOA clock gate and the GPIOA output path used by that smoke. USART2/VCP,
USB, ST-LINK, the buttons, and other MCU peripherals are not modeled here.

Run from the repository root:

```sh
CARGO_BUILD_RUSTC_WRAPPER= scripts/onboarding_smoke.sh \
  --target-id nucleo-u083rc \
  --crate firmware-nucleo-u083rc-smoke \
  --target thumbv6m-none-eabi \
  --script examples/nucleo-u083rc/onboarding-smoke.yaml \
  --system configs/systems/nucleo-u083rc.yaml \
  --out-dir examples/nucleo-u083rc/evidence
```

The committed logs and simulator output are under [`evidence/`](evidence/).
This is local engine evidence, not a physical-board capture or a hosted
`labwired_verify` verdict. The profile has no routed power net, so the engine
assumes an ideal VDD rail and does not simulate power-on or brown-out reset.

## Modeled path

| Board signal | MCU pin | Modeled path |
| --- | --- | --- |
| Green user LED LD4 / Arduino D13 | PA5 | U0 RCC IOPENR GPIOA gate → GPIOA output |

The profile deliberately omits USART2 and the ST-LINK virtual COM connection,
USB, ST-LINK, buttons, and remaining STM32U083 peripherals. It uses no
zero-answering stubs.

## Sources

- ST's [NUCLEO-U083RC product page](https://www.st.com/en/evaluation-tools/nucleo-u083rc.html)
  lists the board as Active and in volume production.
- ST's [STM32U083RC product page](https://www.st.com/en/microcontrollers-microprocessors/stm32u083rc.html)
  lists STM32U083RCT6 as Active and in volume production.
- ST [UM3256 Rev 2](https://www.st.com/resource/en/user_manual/dm01021128.pdf)
  maps LD4 to PA5 and documents the default solder-bridge connection.
- The [Zephyr NUCLEO-U083RC board DTS](https://github.com/zephyrproject-rtos/zephyr/blob/main/boards/st/nucleo_u083rc/nucleo_u083rc.dts)
  identifies LD4 as active-high on PA5.
- ST [RM0503 Rev 4](https://www.st.com/resource/en/reference_manual/DM00908031.pdf)
  documents the U0 memory map, GPIOA register base, and RCC_IOPENR at offset
  0x4C.
- Live availability check: [`evidence/vendor-status.md`](evidence/vendor-status.md).
