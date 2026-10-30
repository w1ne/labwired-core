# MIMXRT595S

The **NXP i.MX RT595S** is the i.MX RT500 crossover MCU on the **NXP
MIMXRT595-EVK**. Only its Cortex-M33 is modelled; the HiFi4 DSP, the 2D GPU
and the PowerQuad are not. See the board page for the pin map, support matrix
and validation evidence:

→ **[MIMXRT595-EVK](mimxrt595-evk.md)**

This part-level page exists so the model-rating table links to the promoted
board package. The earlier auto-generated `imxrt500` onboarding report
exercised a staged descriptor with a generic UART fixture; that is not the
promoted FLEXCOMM USART+LED L1 package, which also boots stock Zephyr
v3.7.0 on the secure address map.
