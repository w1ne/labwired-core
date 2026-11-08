# Replay: the ZG222 GPIO-less negative control, after the ARMv6-M fault fix

Same firmware (`firmware-efm32zg222-smoke`) and smoke assertions as
`2026-11-08-negative-control-no-gpio-timer`, against a temporary copy of the
current `configs/chips/onboarding/efm32zg222.yaml` with only USART1 kept
(`efm32zg222-negctl.yaml` here; flash now at 0x0). The firmware still faults on
its first GPIO access (0x40006050) and the smoke still FAILS 2/5.

What changed is the report. Before, on this Cortex-M0+:

    FAULT  HardFault escalated from a precise BusFault: data access at 0x4000_6050 (BFAR valid), ...
           HFSR=0x40000000 CFSR=0x00008200 [FORCED PRECISERR] BFAR=0x40006050

After:

    FAULT  HardFault: data access at 0x4000_6050, at PC 0x0000_01CC in `core::ptr::read_volatile` (mod.rs:2091), called from `firmware_efm32zg222_smoke::__cortex_m_rt_main`.
           ARMv6-M (no HFSR/CFSR); engine-recorded cause [PRECISERR] address=0x40006050

ARMv6-M has one fault exception and no CFSR/HFSR/MMFAR/BFAR (ARMv6-M ARM
B3.2.2), so nothing "escalates" and there is no BFAR to name. The engine still
records the cause; the report now says that is what it is. Firmware on an M0/M0+
now also reads zero from those reserved offsets, and SHCSR keeps only
SVCALLPENDED.

Run with the debug CLI from `examples/renode-onboarding`
(`labwired test --script efm32zg222-negctl-smoke.yaml`, system file pointing at
the chip copy above), 2026-11-08.
