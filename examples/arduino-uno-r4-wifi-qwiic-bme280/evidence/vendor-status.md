# Live availability — Arduino UNO R4 WiFi + BME280

Checked 2026-11-10 UTC:

- Arduino's official [UNO R4 WiFi store page](https://store.arduino.cc/products/uno-r4-wifi)
  lists SKU **ABX00087** at **€22.88**, with **Add to cart** available. The
  product is currently orderable and names the Renesas RA4M1 MCU.
- Renesas's [RA4M1 product selector](https://www.renesas.com/en/products/ra4m1?tab=documentation&type%5B0%5D=Quality%2C+Reliability%2C+%26+Packaging)
  lists the board's exact MCU, `R7FA4M1AB3CFM#AA0`, as **Active**, **Available**,
  and **In Stock**.
- Arduino's [official full pinout](https://docs.arduino.cc/resources/pinouts/ABX00087-full-pinout.pdf)
  maps the Qwiic connector to IIC0: SDA=P401, SCL=P400, +3.3 V, and GND. The
  connector is 3.3 V only.
- Adafruit's [BME280 STEMMA QT breakout, PID 2652](https://www.adafruit.com/product/2652)
  lists **In stock**, **Add to Cart**, and **US$14.95**. Adafruit states that
  STEMMA QT is compatible with SparkFun Qwiic connectors.

The checked board and sensor are currently orderable. The simulator profile
models the RA4M1 IIC0 transaction path and the BME280 register behavior; it
does not simulate Qwiic cable, voltage, or analog behavior.
