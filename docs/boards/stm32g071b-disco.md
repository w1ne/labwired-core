# STM32G071B-DISCO

The STM32G071B-DISCO is a USB-C and Power Delivery Discovery kit built around
the STM32G071RB. The new narrow LabWired board profile covers boot, GPIOB pad
configuration, and USART3 output through the ST-LINK virtual COM path.

| Item | Path |
|---|---|
| System | [`configs/systems/stm32g071b-disco.yaml`](../../configs/systems/stm32g071b-disco.yaml) |
| Narrow chip profile | [`configs/chips/stm32g071rb-disco-smoke.yaml`](../../configs/chips/stm32g071rb-disco-smoke.yaml) |
| Firmware | [`crates/firmware-stm32g071b-disco-smoke/`](../../crates/firmware-stm32g071b-disco-smoke/) |
| Smoke and evidence | [`examples/stm32g071b-disco/`](../../examples/stm32g071b-disco/) |

The smoke asserts `STM32G071B_DISCO_OK` over USART3 at 115200 baud. The profile
does not claim the OLED, USB-C/PD controller path, current monitors, joystick,
reed sensor, or ST-LINK implementation. The selected RCC, GPIO, and UART
peripherals are modeled directly; none is a zero-answering stub.

ST lists the board as Active and in volume production. Its eStore listing was
In Stock with Add to Cart when checked on 2026-11-03; the receipt is committed
at [`examples/stm32g071b-disco/evidence/vendor-status.md`](../../examples/stm32g071b-disco/evidence/vendor-status.md).

Board wiring follows ST [UM2401](https://www.st.com/resource/en/user_manual/dm00496511-stm32g0-discovery-kit-for-usb-type-c-and-power-delivery-stmicroelectronics.pdf), §§9.14 and 10.1. The smoke is simulator evidence, not a physical-board capture.
