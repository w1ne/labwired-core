# Pinned PXT direct RGB444 fragment capture — host check passed

This hosted-only source-component control passed its focused
[actual capture check](../../docs/receipts/2026-10-07-st7735-pxt-direct/README.md).
It is not a working PyBadge panel or full PXT runtime, and does not qualify
the separate retention guest or all PR gates.

`run.py` downloads and SHA256-checks `screen.cpp` and its Microsoft MIT license
at pxt-common-packages commit `31abf23d118f35010fb75122e60a1eb2b6dffe9f`.
It extracts the original `sendIndexedImage444` method and board-ID predicate
as exact byte fragments, recording their separate hashes. It compiles those
fragments in an authored host class/transport boundary, without source fixes or
substitutions. The original full class, constructor, palette setter, Image type,
`updateScreen` dispatch and runtime are **not** compiled or executed.

Before compilation the runner also checks the two exact fragment hashes.
`test_run.py` has ten mocked admission controls: byte preservation, ambiguous or
missing delimiters, wrong-range selection, exact/corrupt/oversized input, network
failure and immutable declared pins. These controls perform no network request,
compiler invocation or renderer execution; they do not qualify the capture.

The artificial boundary supplies literal palette nibbles, source buffers and
synchronous SPI/pins. Predicate cases cover all six legacy IDs, three VID
boundary/example IDs, four unknown/absent IDs and missing SPI. Those controls
test the actual extracted predicate, not generated compiler macros, real CF2
loading or original branch dispatch.

The proposed four capture cases check command `0x2C`, every data byte, CS/D/C
levels, CS release and exact transfer sizes. A named host-capture mutant flips
the first data byte and must fail with `RAMWR mismatch`.

- Small 4×2: eight distinct literal palette colours, one 12-byte transfer.
- Odd 3×2: the original loop emits eight pixel slots for six declared image pixels. Both
  padded nibbles remain in the 12-byte capture; they are not stripped or fixed.
- Width 160, height 5: expected transfers 480, 480, 240 bytes. The original
  lookahead `3*(W+1)/2` flushes after two rows despite the three-row buffer.
- Width 159, height 4: expected transfers 720, 240 bytes, with each row's padded
  pixel retained. This does not establish correct production Image layout.

These independently tabulated expectations matched the source-bound hosted
execution recorded above. They establish no panel RGB444 transfer function, valid pixels,
module GM/aperture, actual PXT Image storage, ARM/SAM/DMA/IRQ/fiber behavior,
physical timing, performance or consumer adoption. Do not seed an identity LUT
to make the existing ST7735R prototype render these bytes. Follow the
[panel contract](../../docs/boards/pybadge-native.md) for those separate gaps.

## Image-storage boundary — source review only

At the same PXT pin,
[RefImage accessors](https://github.com/microsoft/pxt-common-packages/blob/31abf23d118f35010fb75122e60a1eb2b6dffe9f/libs/base/pxtbase.h)
and [image allocation](https://github.com/microsoft/pxt-common-packages/blob/31abf23d118f35010fb75122e60a1eb2b6dffe9f/libs/screen/image.cpp)
use column-major storage with four-byte-aligned column strides for 4bpp:
`byteHeight = ((height*4 + 31) >> 5) << 2` and
`pix(x,y) = pix() + byteHeight*x + (y >> 1)`.
The header SHA256 is
`87c459c4d8fefa5bd851f862be8e83659185825c41448155caed34a22fe0bd17`;
the image source SHA256 is
`74b51956789d8cc6da461415ae80a5016e207b140ae190ebfc60fa022baac193`.
Both match the inspected pinned package bundle.

Thus a 4×2 RefImage has 16 pixel-storage bytes, while this direct method's
loops consume four bytes for those dimensions. The small capture's tightly
packed input is deliberately **not** a real RefImage. For 160×128, both extents
are 10,240 bytes, but matching extents alone do not prove coordinate/color
correctness or actual runtime dispatch. Height-dependent padding also matters
for status bars and partial images. Before claiming production compatibility,
execute the actual Image/caller boundary with distinct column/row sentinels and
padding, including even full frames and supported partial dimensions. Do not
silently transpose, repack or strip padding from the original source path.

The workflow is intended to produce only the capture JSON and exact downloaded
Microsoft license. It does not upload binaries or restricted firmware.
The prerequisite [authored orientation guest](../../docs/engineering/st7735-orientation-controls.md)
landed in PR168; it does not qualify this separate direct-renderer capture.
Require the actual capture/mutant and all enabled exact-head checks before landing.

## Referenced source and notice

- [Original screen.cpp](https://github.com/microsoft/pxt-common-packages/blob/31abf23d118f35010fb75122e60a1eb2b6dffe9f/libs/screen---st7735/screen.cpp)
- [Original license](https://github.com/microsoft/pxt-common-packages/blob/31abf23d118f35010fb75122e60a1eb2b6dffe9f/LICENSE)

MIT License

Copyright (c) Microsoft Corporation. All rights reserved.

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.
