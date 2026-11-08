# Live vendor status — nRF54LM20 DK

Checked 2026-11-04 UTC.

- Nordic's [current nRF54LM20 DK page](https://www.nordicsemi.com/Products/Development-hardware/nRF54LM20-DK/Get-Started)
  describes a DK featuring the nRF54LM20B, supports the nRF54LM20A as an
  emulated target, and lists four user-programmable LEDs and buttons. Nordic's
  page notes project-based sampling for product updates; it does not list the
  kit as NRND or obsolete.
- DigiKey's [live listing](https://www.digikey.com/en/products/filter/evaluation-boards/embedded-mcu-dsp-evaluation-boards/nordic-semiconductor-asa/786)
  showed the `NRF54LM20-DK` as **Active**, **363 In Stock**, with a one-unit
  price of **$45.00** when checked. Its [product detail page](https://www.digikey.com/en/products/detail/nordic-semiconductor-asa/NRF54LM20-DK/27685498)
  offers quantity entry and **Add to Cart**.
- Nordic's [DK hardware guide](https://docs.nordicsemi.com/r/bundle/ug_nrf54lm20_dk/page/ug/nrf54lm20_dk/hw_desription/buttons_leds.html)
  maps LED0 to P1.22 and confirms the LEDs are active-high.

The firmware and chip profile target the vendor-supported nRF54LM20A emulation
mode. The actual DK is fitted with the nRF54LM20B; this onboarding does not
claim that the B target is modeled.
