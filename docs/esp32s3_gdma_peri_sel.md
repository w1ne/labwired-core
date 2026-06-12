# ESP32-S3 GDMA PERI_SEL Register Ground Truth

## Register offsets (channel-relative, within a 0xC0-byte channel block)

| Register     | Field name     | Channel-relative offset | Absolute (CH0, GDMA_BASE = 0x6080_0000) |
|--------------|----------------|------------------------|------------------------------------------|
| IN_PERI_SEL  | PERI_IN_SEL    | 0x48                   | GDMA_BASE + 0x048                        |
| OUT_PERI_SEL | PERI_OUT_SEL   | 0xA8                   | GDMA_BASE + 0x0A8                        |

Channel N offset formula: `GDMA_BASE + N * 0xC0 + <channel-relative offset>`.

Example: CH2 IN_PERI_SEL = `GDMA_BASE + 2*0xC0 + 0x48` = `GDMA_BASE + 0x1C8`. This matches
`GDMA_IN_PERI_SEL_CH2_REG = DR_REG_GDMA_BASE + 0x1C8` in gdma_reg.h.

## Field encoding

Both registers use the same 6-bit field at bits [5:0]. Reset value: `0x3F` (no peripheral selected).

```
bits [31:6]  reserved
bits  [5:0]  sel  — peripheral index
```

## Value → peripheral table

| sel value | Peripheral | Notes                          |
|-----------|------------|--------------------------------|
| 0         | SPI2       | GP-SPI2 master/slave           |
| 1         | SPI3       | GP-SPI3 master/slave           |
| 2         | UHCI0      | UHCI0 bridge → UART DMA path   |
| 3         | I2S0       | I2S0 TX/RX                     |
| 4         | I2S1       | I2S1 TX/RX                     |
| 5         | LCD_CAM    | LCD/camera controller          |
| 6         | AES        | AES accelerator                |
| 7         | SHA        | SHA accelerator                |
| 8         | ADC_DAC    | SAR ADC (DAC label vestigial)  |
| 9         | RMT        | RMT controller                 |
| 0x3F (63) | —          | Reset / unbound                |

## Collision check against gdma.rs (lines 99–115)

Existing modeled channel-relative offsets:

```
IN:   0x00 IN_CONF0, 0x04 IN_CONF1, 0x08 IN_INT_RAW, 0x0C IN_INT_ST,
      0x10 IN_INT_ENA, 0x14 IN_INT_CLR, 0x20 IN_LINK
OUT:  0x60 OUT_CONF0, 0x64 OUT_CONF1, 0x68 OUT_INT_RAW, 0x6C OUT_INT_ST,
      0x70 OUT_INT_ENA, 0x74 OUT_INT_CLR, 0x80 OUT_LINK
```

**0x48 and 0xA8 are unused by any existing constant — no collision.**

## Citations

1. **gdma_reg.h** (absolute address macros + field description comments)
   `components/soc/esp32s3/register/soc/gdma_reg.h`
   https://github.com/espressif/esp-idf/blob/master/components/soc/esp32s3/register/soc/gdma_reg.h
   Macros: `GDMA_IN_PERI_SEL_CH0_REG`, `GDMA_PERI_IN_SEL_CH0`, `GDMA_OUT_PERI_SEL_CH0_REG`, `GDMA_PERI_OUT_SEL_CH0`

2. **gdma_struct.h** (C struct layout; `peri_sel.sel` field at IN+0x48 / OUT+0xA8)
   `components/soc/esp32s3/register/soc/gdma_struct.h`
   https://github.com/espressif/esp-idf/blob/master/components/soc/esp32s3/register/soc/gdma_struct.h
   Struct: `gdma_dev_t` → `channel[n].in.peri_sel` / `channel[n].out.peri_sel`

3. **esp-pacs esp32s3 PAC** (Rust register access crate, SVD-derived, independent source)
   `esp32s3/src/dma/ch.rs` — offset annotations `0x48` / `0xa8`
   `esp32s3/src/dma/ch/in_peri_sel.rs` — `PERI_IN_SEL_W<REG, 6>` (6-bit field, bits 0:5), reset `0x3f`
   `esp32s3/src/dma/ch/out_peri_sel.rs` — `PERI_OUT_SEL_W<REG, 6>` (6-bit field, bits 0:5), reset `0x3f`
   https://github.com/esp-rs/esp-pacs/tree/main/esp32s3/src/dma/ch

All three sources agree on offsets and value encoding. No in-repo vendored headers found.
