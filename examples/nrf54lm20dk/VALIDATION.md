# Validation — nRF54LM20 DK

- Nordic's [nRF54LM20 DK page](https://www.nordicsemi.com/Products/Development-hardware/nRF54LM20-DK/Get-Started)
  identifies the physical nRF54LM20B SoC and explicitly supports the nRF54LM20A
  as an emulated target.
- Nordic's [DK hardware guide, Buttons and LEDs](https://docs.nordicsemi.com/r/bundle/ug_nrf54lm20_dk/page/ug/nrf54lm20_dk/hw_desription/buttons_leds.html)
  maps LED0 to P1.22 and states the LEDs are active-high.
- Nordic's [nRF Connect SDK target list](https://nrfconnectdocs.nordicsemi.com/ncs/latest/nrf/app_dev/device_guides/nrf54l/index.html)
  lists `nrf54lm20dk/nrf54lm20a/cpuapp` as the supported emulation target.
- The local smoke gates both the UARTE20 marker and at least 16 GPIO edges on
  `gpio1:22`; see the committed run log and simulation output beside this file.
- The hosted LabWired oracle returned `model_verified` for 17 transitions on
  `gpio1:22`; see [the verifier record](evidence/labwired-verification.md) for
  the power-rail observation gap.

This is model evidence for the scoped GPIO/UARTE20 path, not a hardware
measurement. It does not verify LED optical output, PMIC behavior, radio, RF,
NFC, debugger, or external flash operation.
