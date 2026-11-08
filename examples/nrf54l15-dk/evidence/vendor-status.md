# Live vendor status: Nordic nRF54L15 DK

Checked 2026-11-04 UTC.

- Nordic's [nRF54L15 DK product page](https://www.nordicsemi.com/Products/Development-hardware/nRF54L15-DK) identifies the DK as the evaluation kit for the nRF54L15 and lists four user-programmable LEDs, four buttons, two virtual UARTs, and 8 MB external flash.
- Nordic's [DK hardware guide](https://docs.nordicsemi.com/r/bundle/ug_nrf54l15_dk/page/ug/nrf54l15_dk/hw_desription/buttons_leds.html?contentId=vBV74pvpsehCkmEdh83LQ) maps LED0 to P2.09.
- Nordic's [nRF54L Series availability update](https://devzone.nordicsemi.com/nordic/nordic-blog/b/blog/posts/nrf54l-series-fourth-package-now-in-production-with-more-coming) states that the nRF54L15 QFN and WLCSP package options are in full-volume production.
- DigiKey's [NRF54L15-DK listing](https://www.digikey.com/en/products/detail/nordic-semiconductor-asa/NRF54L15-DK/25601516) reported **Part Status: Active**, positive stock, and **Add to Cart** when checked.

The board profile models the nRF54L15, its LED/button GPIO, and UARTE20 to the
virtual console. The radio, J-Link interface MCU, and external flash are not
attached to the simulation profile. No archive snapshot was used.
