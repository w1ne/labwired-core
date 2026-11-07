# CH32V203C8T6 / WeAct BluePill-Plus-CH32 live availability check

Checked live on 2026-11-06 UTC.

- WeAct's [BluePill-Plus-CH32 repository](https://github.com/WeActStudio/WeActStudio.BluePill-Plus-CH32)
  lists CH32V203C8T6 ("144Mhz Max, 20KB RAM, 64KB ROM, RISC-V") among the
  board's fitted MCU options and documents "Blue LED `PB2` Active high" and
  "User KEY `PA0`". Its CH32V203 `01-template` example prints over the WCH
  SDK's default `DEBUG_UART1` console and toggles PB2.
- Robosap's live [WeAct Studio CH32V203C8T6 RISC-V Low Power Core Demo Board](https://robosap.in/product/weact-studio-ch32v203c8t6-risc-v-low-power-core-demo-board/)
  listing showed ₹311.52 incl. GST, "18 in stock", and an "Add to cart"
  control. Other resellers in the same search (Indian Hobby Center, ElectroPi)
  showed the board sold out; WeAct's own AliExpress store was not checked.
- The CH32V203C8T6 is also sold on a second current board: MuseLab's
  [nanoCH32V203 on Tindie](https://www.tindie.com/products/johnnywu/nanoch32v203-development-board)
  showed $3.00 with 89 base boards in stock. That board is not the one modeled
  here.
- Upstream Zephyr ships a `bluepillplus_ch32v203` board definition whose DTS
  places `led0` on `gpiob 2 GPIO_ACTIVE_HIGH`, matching WeAct's README.
- WCH's [CH32V203 product page](https://www.wch-ic.com/products/CH32V203.html)
  renders only with JavaScript and returned no readable status to an HTTP
  client. WCH does not publish an "Active"/"NRND" label for its parts (see the
  CH32V003 receipt), so this receipt claims reseller stock, not a formal WCH
  lifecycle classification.

## Re-check 2026-11-07 (ch32fun profile round)

- Robosap's WeAct CH32V203C8T6 board listing: "₹311.52 incl. GST", "Status:18
  in stock", "Add to cart" present.
- MuseLab nanoCH32V203 on Tindie (also CH32V203C8T6): $3.00, "nanoCH32V203
  Development Board [89 in stock]".
- LCSC's search page did not render results for an HTTP client, so it is not
  cited.
