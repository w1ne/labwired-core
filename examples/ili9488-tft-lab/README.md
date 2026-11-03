# ILI9488 TFT Lab

STM32F103 SPI1 drives an Ilitek ILI9488 at COLMOD `0x66` (18-bit / three
bytes per pixel). The smoke script asserts UART banners plus a cyan
`display_region` colour count against the rgb888 artifact.

Pins: PA4 CS, PA5 SCK, PA7 MOSI, PB0 D/C, PB1 RESET (active-low).
