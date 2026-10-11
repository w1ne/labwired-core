# Luckfox Pico Mini A vendor assets

Retrieved **2026-10-11**, directly from official Luckfox hosts or the official repository linked from the [vendor downloads page](https://wiki.luckfox.com/Luckfox-Pico-RV1103/Downloads/). The [source manifest](sources.json) records original URLs, byte counts and SHA-256 hashes. These are original vendor files, not generated illustrations or new simulator artifacts.

| File | Contents / inspection |
|---|---|
| [product.webp](product.webp) | Exact Mini A photograph; inspected visually, NAND footprint unpopulated |
| [pinout.jpg](pinout.jpg) | Shared Mini A/B layout; inspected visually, all 22 numbered side pads transcribed |
| [dimensions.jpg](dimensions.jpg) | Shared Mini A/B mechanical reference; inspected visually, 21.00 × 28.14 mm, 17.78 mm row spacing, 2.54 mm pitch |
| [schematic.pdf](schematic.pdf) | Official shared Mini schematic; one sheet, RV1103G1 identified; PDF text and rendered page inspected. No revision/date printed in the inspected sheet; pin source by hash |
| [rv1103-datasheet.pdf](rv1103-datasheet.pdf) | Official Rockchip RV1103 datasheet Rev 1.1, revision history dated 2022-04-27; title/revision text checked |
| [vendor-cad.zip](vendor-cad.zip) | Official multi-board CAD archive; member listing inspected and includes `Luckfox-Pico-CAD/Luckfox Pico Mini.DWG`. DWG geometry has not been rendered or dimensionally validated |

The pin/dimension illustrations depict a NAND-populated Mini variant. They are used as shared interface/mechanical references and must not be presented as the Mini A component population. The separate `product.webp` shows the correct Mini A population. The schematic includes a flash circuit shared with other population options; Mini A does not fit that NAND part.

The official documentation source was pinned at **b28b3929a5b5f78308b2c26d0cad8629f20fcead**. A vendor [STEP archive](https://github.com/LuckfoxTECH/Luckfox-Pico-docs/blob/b28b3929a5b5f78308b2c26d0cad8629f20fcead/Hardware/3D%20Models/Luckfox-Pico-Step.zip) is also available; it was not downloaded or inspected here, and exact Mini A contents are unverified.

Copyright and ownership remain with Luckfox/Rockchip and other original rights holders. No explicit permissive asset-redistribution license was established; the repository's code license must not be assumed to relicense these vendor images, documents or CAD. Preserve attribution and verify rights for any subsequent redistribution. No community CAD has been imported.
