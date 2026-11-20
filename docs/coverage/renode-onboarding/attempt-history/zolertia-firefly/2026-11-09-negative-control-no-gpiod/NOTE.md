# zolertia-firefly — negative control: no-gpiod (2026-11-09)

Run: `firmware-zolertia-firefly-smoke` (this round) with `examples/renode-onboarding/zolertia-firefly-smoke.yaml` against this round's descriptor, with the `gpiod` block removed from the included cc2538sf53.yaml (cc2538sf53.yaml here). Result: FAIL 1/7 checks. HardFault from a precise BusFault at 0x400DC420 (GPIO_D AFSEL, the LED port set-up) at PC 0x00200526; no UART output, summary word 0. The LED checks depend on the modelled GPIO D port.
