# NUCLEO-H753ZI evidence before M7 FPU smoke

This directory preserves the committed passing NUCLEO-H753ZI USART3 smoke
artifacts that the 2026-11-12 M7 FPU run replaced in `metrics/nucleo_h753zi/`.
The prior run used `thumbv7em-none-eabi`, declared four peripheral windows,
and did not exercise the Cortex-M7 FPU. Its original metric, logs, JUnit
report, simulator result, and snapshot are preserved here.
