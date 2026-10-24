# SAM SPI bus byte-acceptance prerequisite

[PR214](https://github.com/CrispStrobe/labwired-core/pull/214) merged on2026-10-13.
This qualifies the bus acceptance seam and its host controls with CPU time
advancement. It does not qualify CPU-programmed peripheral DMA, native Arcade,
physical timing, whole-image provenance or licence/store clearance.

## Exact source and landing

- Reviewed source: `d67c6354add425c49b1f9e7f36a460f28c38c1a1`.
- Base: `ab5651b37ca88102ea7769d85f973306d56a1b15`.
- Actual tested checkout: `19be0a4e7b92a6bf4b6630dae66ab20199eea445`.
- Landed merge: `46d91e099dca062145ae2b8b68dd549c20da7763`.
- Reviewed/tested/landed tree: `3dbe227b92c41e9a82f7570f0ed0dee34bc40090`;
  tested and landed parents match the declared base/source.
- [Rust38356282707](https://github.com/CrispStrobe/labwired-core/actions/runs/38356282707),
  pull-request attempt1, succeeded. All22 enabled exact-head checks passed;
  `ci-runner-image`, `core-full`, `core-integrity`, `pr-gate-warm` alone skipped
  under their declared conditions. Normal matched-head merge, no gate bypass.

## Actual originals and boundaries

| Profile | Original job | Bus drives | Preserved SPI split cases / entries / returns | Preserved byte cases / entries / returns |
| --- | --- | --- | --- | --- |
| Off | [115160382435](https://github.com/CrispStrobe/labwired-core/actions/runs/38356282707/job/115160382435) |2 |24 /8 /8 |90 /36 /36 |
| On | [115160382543](https://github.com/CrispStrobe/labwired-core/actions/runs/38356282707/job/115160382543) |4 |48 /16 /16 |90 /36 /36 |

Off original:657639 bytes, SHA256
`5bf14e0d7f4856e5d96abaf92c2957d4300b5e02548a05626737af3ca97be237`.
On original:123901 bytes, SHA256
`4ca8629fec4bd14277db9bdce8834f3e447cd2a28acae2cef8aba1ba4284b813`.
A stdlib-only checker frozen before original acquisition verified the exact
profile cases, named tests, official source/job/step bindings and actual
checkout/tree. Final admission checked all enabled checks and landed parents/
tree using retained originals, without guest replay or a second acquisition.

Each bus drive accepts two bytes, observes one byte at the owned slave, and
discards the second accepted but unshifted byte on reset. Six blocked attempts
and eleven unsupported/alias/storage refusals report no accepted write.
The test checks accepted-only observer notifications, non-consuming RX sampling,
ordinary CPU successful/dropped-store notifications, both paused SPI clocks,
IRQ reconciliation and pump/event retirement. Forced walk and event profiles
use the applicable batch1/64/1024 controls. The preserved actual CPU guests
exercise existing SPI and software byte-DMA, not a new peripheral-DMA operation.

## Remaining work

Use the [live TX contract](../../engineering/samd-dmac-byte-foundation.md#next-bounded-lane--live-sercom4-tx-byte-requests)
for an explicit hardware-request profile and CPU-programmed descriptor guests.
No generic MMIO `Ok` may establish an accepted DMA beat. Keep last DATA
acceptance, final count/writeback, DMA completion, last wire byte and SPI TXC
distinct; qualify CS/callback ordering before runtime integration.

The production descriptor, app pins, captures, drift acknowledgements and
thresholds did not move. No new RTx measurement or whole-runtime admission
was obtained. The earlier superseded-source checks are not final-source evidence.
