# LP-MSPM0G3507 smoke pin map

Pin assignments below are from TI's [Rev. E LaunchPad User's Guide](https://www.ti.com/lit/ug/slau873e/slau873e.pdf)
and the MSPM0G350x SDK header. The simulator currently captures UART register
traffic; it does not propagate signals through IOMUX or model board voltage.

| Function | MCU route | Board connection | Smoke coverage |
|---|---|---|---|
| UART0 TX | PA10, PINCM21, PF=UART0_TX (2) | J21 1:2 to XDS110 backchannel UART; J101 TXD path | TXDATA, CTL0.EN/TXE gate and TXFE status are modeled; mux and electrical path are not |
| UART0 RX | PA11, PINCM22, PF=UART0_RX (2) | J22 1:2 from XDS110 backchannel UART; J101 RXD path | Not modeled |
| Red LED1 | PA0 | J4 | Not modeled |
| RGB LED2 | PB22/PB26/PB27 | J5/J6/J7 | Not modeled |

PA10/PA11 correspond to PINCM21/22 in the G3507 SDK pin map. The package mux
encoding is 2; the PINCM register also needs its connected bit set. The listed
board routes depend on the jumpers being in the XDS110 positions documented by
TI. The firmware smoke intentionally does not write IOMUX because that block
is absent from the simulator profile.
