# Live vendor status — BBC micro:bit v2

Checked 2026-11-04 UTC.

- The Micro:bit Educational Foundation's [official products page](https://microbit.org/buy/?location=US&version=microbitV2)
  lists the BBC micro:bit V2 as a current board-only product sold through
  channel partners. Adafruit's [live listing, product 4781](https://www.adafruit.com/product/4781),
  showed the board at **$17.95, 21 in stock**, with an Add to Cart control when
  checked. Adafruit notes that stock can be either V2.0 or V2.2; the interface
  MCU differs between those revisions.
- Nordic's [nRF52833 product page](https://www.nordicsemi.com/Products/nRF52833)
  lists the SoC and current distributor inventory. DigiKey's listing for
  [NRF52833-QIAA-B-R7](https://www.digikey.com/en/products/detail/nordic-semiconductor-asa/NRF52833-QIAA-B-R7/15998610)
  reported **Active** and **745 in stock**. The board's application MCU is the
  nRF52833; that QIAA order code is a buyable family variant, not a claim about
  the package suffix fitted to every board revision. The onboarding smoke
  exercises the application MCU's UARTE0 transmitter.
- The board's ST motion sensor is documented by the Foundation as LSM303AGR.
  ST's [live LSM303AGR product page](https://www.st.com/en/mems-and-sensors/lsm303agr.html)
  lists `LSM303AGRTR` as **Active — product is in volume production**. ST's
  [live eStore listing](https://estore.st.com/en/products/mems-and-sensors/e-compasses/lsm303agr.html)
  showed `LSM303AGRTR` **In Stock**, 440 units, with an Add to Cart control.
  This smoke does not exercise the motion sensor.
- Adafruit notes that V2.0 or V2.2 stock may ship, with a different interface
  MCU between revisions. The Foundation's [revision notes](https://tech.microbit.org/hardware/2-2-revision/)
  describe the interface-MCU variants. DigiKey lists the KL27 option
  [MKL27Z64VFM4](https://www.digikey.com/en/products/detail/nxp-usa-inc/MKL27Z64VFM4/5361440)
  as **Active** with **1,730 in stock**, and the nRF52820 option
  [NRF52820-QDAA-D-R7](https://www.digikey.com/en/products/detail/nordic-semiconductor-asa/NRF52820-QDAA-D-R7/15998603)
  as **Active** with **2,010 in stock**. The interface MCU and physical USB CDC
  path are outside this smoke's claim.

The committed simulation profile `configs/systems/microbit-v2-uart-smoke.yaml`
is scoped to the nRF52833 UARTE0 path and its GPIO pad banks. The test asserts
UARTE0 serial output captured by the simulator; that console connector abstracts
the interface-MCU/USB bridge and is not evidence for its physical behavior.
The LED matrix, buttons, motion sensor, microphone, speaker, and USB programming
path are not exercised by this onboarding smoke.
