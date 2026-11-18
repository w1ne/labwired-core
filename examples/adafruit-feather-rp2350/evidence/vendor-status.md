# Vendor status receipt — Adafruit Feather RP2350 with HSTX

Checked live on **2026-11-13** for the **no-PSRAM version**, Adafruit Product
ID **6000**.

- Official product page: <https://www.adafruit.com/product/6000>
- The page listed the Feather RP2350 with HSTX, the no-PSRAM option at
  **$12.50**, and **2 in stock**, with “Order now to ship today.”
- The product description states 8 MB flash and RP2350; the revision history
  says the board now ships with the A4 RP2350 as of March 20, 2026.
- The board's live pinout guide maps D13 to the red LED on GPIO7 and UART0 to
  TX=GPIO0 / RX=GPIO1: <https://learn.adafruit.com/adafruit-feather-rp2350/pinouts>
- The Adafruit PCB schematic grounds the LED cathode and routes GPIO7 through
  its series resistor to the anode, supporting the active-high LED mapping:
  <https://raw.githubusercontent.com/adafruit/Adafruit-Feather-RP2350-PCB/main/Adafruit%20Feather%20RP2350.sch>

The page showed a purchasable board; it did not provide a separate lifecycle
label. This receipt records availability for the exact no-PSRAM SKU, not a
guarantee of continuing stock.
