# External components and provenance

Only Linux and static BusyBox are needed for the first boot. Kernel development headers, modules, external-driver compilation and smartbasket software are separate later work.

| Component | Official pinned source | APK SHA256 | Extracted SHA256 |
| --- | --- | --- | --- |
| Alpine ARMv7 Linux LTS 6.18.56-r0 | https://dl-cdn.alpinelinux.org/alpine/v3.23/main/armv7/linux-lts-6.18.56-r0.apk | `00059546e63df3dbadb80ff08f577dce1ef871b8cd800665d6404936a956eade` | zImage `eac9582162e912bdb577bba6ea224d98a29c997e52ee891383ea836aa704680a` |
| Alpine ARMv7 BusyBox static 1.37.0-r30 | https://dl-cdn.alpinelinux.org/alpine/v3.23/main/armv7/busybox-static-1.37.0-r30.apk | `fb023d6a1e2dce9069b11b302e674c273b9e17d04d17b03ee16526475b0971e6` | BusyBox `1c0ed2db00c0bbdd796e4bdb4de58c564a3df8e61afc716652004b3ce0ee50f2` |

Linux APK: 53,474,254 bytes; zImage: 8,524,288 bytes; config: 269,493 bytes, SHA256 `4e64812402b287e13fd7513aa830432782a1ae94a5461bfe579999d1e18432fb`. BusyBox APK: 628,018 bytes; static binary: 886,344 bytes. There is no version fallback.

The Linux aports recipe is pinned at [b599422f9f640e35932a995ceedf52ec76b66963](https://gitlab.alpinelinux.org/alpine/aports/-/blob/b599422f9f640e35932a995ceedf52ec76b66963/main/linux-lts/APKBUILD); Linux is GPL-2.0-only. BusyBox's recipe is pinned at [1e823a60eb85606954b3a5af5f8e5bbd1ea680cf](https://gitlab.alpinelinux.org/alpine/aports/-/blob/1e823a60eb85606954b3a5af5f8e5bbd1ea680cf/main/busybox/APKBUILD); BusyBox is GPL-2.0-only. Preserve applicable licenses and provide corresponding sources when distributing these binaries.

The embedded ARM backend uses upstream tlib revision `167decf9129758829762e582939128c0694e90d6`, source archive SHA256 `8db4d1fe0a316eec5dcae83bd4d2757b5172332493d6fdecfd9d4b81cb7a77a3`. The source archive hash is lineage, not the native binary identity. The CLI records SHA256 of the actual compiled embedded DLL separately. Vendored sources remain unchanged; documented corrections are generated in the native build directory. Upstream contains LGPL and per-file notices; the new bridge is MIT. See `third_party/tlib-PROVENANCE.md`.

Downloads and generated zImage/DTB/cpio live outside the repository. The committed artifact manifest documents source pins and deterministic derived identities; local preparation and boot produce separate receipts.

APK preparation bounds the full gzip expansion before parsing member metadata. A physical 512-byte TAR scan checks header checksums, record counts, declared payload sizes, extension metadata sizes and truncation before `tarfile` can consume PAX or GNU extension records. Expanded-byte limits include every header, alignment byte and zero trailer; unsupported GNU sparse continuation records are refused. Normal member counts and payload totals retain their original limits.

| APK | Observed visible / physical records | Expanded TAR bytes | Extension bytes / largest record |
| --- | --- | --- | --- |
| Linux | 3,717 / 7,434 | 87,126,528 | 300,872 / 204 |
| BusyBox | 4 / 8 | 895,488 | 156 / 90 |

Each pinned APK uses one short PAX timestamp/path record per visible member. The physical count ceilings allow twice the existing visible ceilings: 8,000 for Linux and 32 for BusyBox. Each extension is limited to 512 bytes; cumulative extension ceilings are 512,000 and 8,192 bytes. Linux's 100,000,000-byte expansion ceiling covers the existing 90,000,000-byte payload budget plus at most 1,024 header/padding bytes per physical record, the separate metadata budget and three 10-KiB TAR trailers. BusyBox's 2,000,000-byte ceiling covers the corresponding 1,000,000-byte payload budget and overhead. These enforced ceilings and measured physical statistics appear in the preparation manifest; artifact contents and their SHA256 identities are unchanged.
