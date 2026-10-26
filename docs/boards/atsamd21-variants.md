# ATSAMD21 A-variant orderable geometries

Eleven exact A-variant SAM D21 orderable parts are described in
`configs/chips/` alongside the existing ATSAMD21G18A engine. They differ only
in memory, package/pin count, GPIO count and (on the E) the number of SERCOM
instances; they do **not** add peripheral or clock models.

Source of truth: Microchip **SAM D21/DA1 Family Data Sheet DS40001882H**,
section 2 (Configuration Summary), section 3 (Ordering Information) and
section 5 (Pinout):
[official PDF](https://ww1.microchip.com/downloads/aemDocuments/documents/MCU32/ProductDocuments/DataSheets/SAM-D21-DA1-Family-Data-Sheet-DS40001882H.pdf).

Peripheral bases and IRQs come from Microchip's **Apache-2.0**
`ATSAMD21G18A.svd`, vendored at
`tests/fixtures/real_world/atsamd21g18a.svd`. The SAM D21 family shares one
linear address map; the E simply does not have SERCOM4/SERCOM5.

| Orderable ID | Flash | SRAM | Package pins | GPIO | SERCOM |
|--------------|-------|------|--------------|------|--------|
| `atsamd21e15a` | 32 KB | 4 KB | 32 (TQFP/QFN) | 26 | 4 |
| `atsamd21e16a` | 64 KB | 8 KB | 32 (TQFP/QFN) | 26 | 4 |
| `atsamd21e17a` | 128 KB | 16 KB | 32 (TQFP/QFN) | 26 | 4 |
| `atsamd21e18a` | 256 KB | 32 KB | 32 (TQFP/QFN) | 26 | 4 |
| `atsamd21g15a` | 32 KB | 4 KB | 48 (TQFP/QFN) | 38 | 6 |
| `atsamd21g16a` | 64 KB | 8 KB | 48 (TQFP/QFN) | 38 | 6 |
| `atsamd21g17a` | 128 KB | 16 KB | 48 (TQFP/QFN) | 38 | 6 |
| `atsamd21j15a` | 32 KB | 4 KB | 64 (TQFP/QFN) | 52 | 6 |
| `atsamd21j16a` | 64 KB | 8 KB | 64 (TQFP/QFN) | 52 | 6 |
| `atsamd21j17a` | 128 KB | 16 KB | 64 (TQFP/QFN) | 52 | 6 |
| `atsamd21j18a` | 256 KB | 32 KB | 64 (TQFP/QFN) | 52 | 6 |

Each descriptor records the A-variant orderable suffixes in TQFP/QFN (`AU`,
`AUT`, `AF`, `AFT`, `MU`, `MUT`, `MF`, `MFT`) as data. B/C/D/L variants are
different silicon/pinout options and are not aliased to these files. The G17A
and G18A are also offered in a 45-ball WLCSP; those descriptors describe the
48-pin TQFP/QFN geometry.

`crates/core/tests/atsamd21_variant_geometry.rs` gates the table above,
including the four-vs-six SERCOM split and the shared bootable SAM D21 estate.
`svd_conformance` checks every declared base/IRQ against the vendored SVD.

## What is reused — and what is not claimed

These are **geometry variants on the public [ATSAMD21G18A](atsamd21g18a.md)
engine**. They reuse its PM, SYSCTRL, GCLK, WDT, NVMCTRL, PORT and SERCOM
models by path. Everything that page says is *not* proven remains not proven
here: no silicon diff, no clock gating, `SYNCBUSY` reads zero, SERCOM is USART
mode only, and USB/EIC/DMAC/EVSYS/RTC/TCC/TC/ADC/AC/DAC/I2S/PTC are absent.
The `pins:` maps are family-wide label→(port, bit) maps, not package bond-out
claims; the package pin counts in the table are the ordering datapoint.
