# PXT direct RGB444 fragments — actual host capture

Tested source: `c1d03878589b173c51c8d72fb1e7e33719dd6774`,
submitted in [PR169](https://github.com/CrispStrobe/labwired-core/pull/169).
The [actual capture run37647759023](https://github.com/CrispStrobe/labwired-core/actions/runs/37647759023)
and [job112882868540](https://github.com/CrispStrobe/labwired-core/actions/runs/37647759023/job/112882868540)
passed. This records the focused host check, not completion of all PR gates or
an already landed merge.

The original [artifact11494779371](https://api.github.com/repos/CrispStrobe/labwired-core/actions/artifacts/11494779371/zip)
reported a 2,447-byte ZIP. Its byte-preserved
[JSON member](fragment-trace.json) has SHA256
`82803444d5a6079388de7f924a451d5781b47abd1eb6ae4a95dc8952e2220a04`.
The companion exact Microsoft MIT license has SHA256
`dea9265341829002e2c23a7372393eb2ed6e26085fb623f38a4ba0af833f30a6`;
the notice and immutable original link remain in the
[harness guide](../../../scripts/st7735-pxt-direct-trace/README.md).

Exact SHA-bound original method and board-ID predicate fragments compiled with
Ubuntu g++13.3.0 for **32-bit host x86**, inside the documented authored
class/palette/buffer/transport boundary. Ten admission controls passed.
Thirteen board-ID cases (nine accepted, four rejected) and the missing-SPI
negative passed. All emitted bytes, CS release and transfer sizes matched:

| Authored input dimensions | RAMWR data bytes | Transfer sizes |
| --- | ---: | --- |
| 4×2 | 12 | 12 |
| 3×2 | 12 | 12 |
| 160×5 | 1,200 | 480,480,240 |
| 159×4 | 960 | 720,240 |

The odd 3×2 case emits eight slots for six declared pixels. Odd-width padding
and original batching are preserved, not normalized. A named corrupt-first-data
capture was rejected with `RAMWR mismatch`. A separate read-only artifact audit
checked all four literal byte sequences, source buffers, dimensions, transfer
extents, predicate results and license hash without importing source helpers.

This is not execution of full WDisplay, actual Image storage, its constructor,
updateScreen branch dispatch, ARM/SAM/DMA/IRQ, deployed macros/CF2, module
GM/aperture/LUT, physical timing, RTx, browser-WASM or an installed app.
The real Image four-byte-aligned column stride remains a separate compatibility
gate. This capture does not justify inventing an RGB444 identity LUT or closing
P3/CP14.
