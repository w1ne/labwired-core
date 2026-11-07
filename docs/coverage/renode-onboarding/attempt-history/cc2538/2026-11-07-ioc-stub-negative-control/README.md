# CC2538 IOC stub negative control (2026-11-07)

Same firmware (`firmware-cc2538dk-smoke`, with `ioc_selftest`) and the same
`examples/renode-onboarding/cc2538-smoke.yaml` assertions, run against the
pre-change `configs/chips/cc2538sf53.yaml`, where `ioc` was `type: stub`.

Result: **FAIL 2/3 checks** (`uart_ordered` fails). The stub answers every IOC
read with zero, so the read-back line is

    ioc pa1_sel=00 pa1_over=00 pa0_over=00 rx0=00 pd7_over=00 gpt3ocp2=00 restored=00

instead of the `cc2538_ioc` model's

    ioc pa1_sel=00 pa1_over=08 pa0_over=00 rx0=00 pd7_over=04 gpt3ocp2=1f restored=00

(`PA1_OVER` written 0x8, `PD7_OVER` at its Table 9-7 reset 0x4, an all-ones
`IOC_GPT3OCP2` write kept to `INPUT_SEL[4:0]`). `uart.log` and `result.json`
here are that run's output. This is a model-only CLI result, not a hardware
capture.
