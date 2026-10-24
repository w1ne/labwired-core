# SAMD51 software-triggered byte-DMA checkpoint

[PR210](https://github.com/CrispStrobe/labwired-core/pull/210) merged on2026-10-12.
The selected model is opt-in `sam_dmac_byte`, not full `sam_dmac`. The production
PyBadge descriptor, application pins, captures, acknowledgements and performance
thresholds were not changed. The separately authored model is MIT; this is not
whole-runtime provenance, licence or store clearance.

## Exact source and checks

- Reviewed/tested source: `637f6cea54342ccf1eba88e5087aa82605d416f5`.
- Base: `e26d948e24cba22e7ccd514ee96a29c5c0019cd0`.
- Actual test checkout: `c5a65b4cc34c42d8188b9cc74efbe465d2fc8ddd`.
- Landed merge: `208ee0518bd8ff5204e6d4f0d19a1524b266f5da`.
- Reviewed, tested and landed tree: `a21ab1adce62ff73e6c98fa85b5ceeae03c09f1b`;
  tested/landed parents both match the declared base and source.
- [Rust run38310859190](https://github.com/CrispStrobe/labwired-core/actions/runs/38310859190),
  pull-request attempt1, passed. All22 enabled final-head checks succeeded;
  only `ci-runner-image`, `core-full`, `core-integrity`, `pr-gate-warm` skipped
  under their declared conditions. Normal exact-head merge, no gate bypass.

## Actual guest originals

| Profile | Original job | Cases | Entries/returns | Original bytes |
| --- | --- | --- | --- | --- |
| Feature-off | [115015972724](https://github.com/CrispStrobe/labwired-core/actions/runs/38310859190/job/115015972724) |90 |36/36 |656582 |
| Feature-on | [115015972512](https://github.com/CrispStrobe/labwired-core/actions/runs/38310859190/job/115015972512) |90 |36/36 |123133 |

Off original SHA256: `4b3d5fa6a9f1221d7589033ce657cb3bfc23f94a3b51c9009da15c0b9f3dcace`.
On original SHA256: `b776c813e218a26a87432a6c63817607f6a387cb6666e27385b6c57d9a81d57c`.
A separately frozen stdlib-only reader checked all cases, flags/counts/handler
witnesses, three named successful integration tests per profile, official
source/job/step bindings and original checkout/tree equality. No guest replay
was used to read the originals. These checks are not complete binary provenance.

The CPU creates descriptors/data, configures DMAC/NVIC and acknowledges actual
handlers. Six channels cover all five vector routes; fifteen modes include
masks, disabled/no-trigger controls, partial-progress reset, fixed/incrementing
addresses and actual read/write/writeback faults. Separate host controls cover
shared IRQ clearing and explicit unsupported descriptor/mapped-target refusal.

## Failures preserved

Earlier attempts are not silently promoted to final-source passes. Preserve
the [missing Debug failure](https://github.com/CrispStrobe/labwired-core/pull/210#issuecomment-6129813588),
[downcast ratchet failure](https://github.com/CrispStrobe/labwired-core/pull/210#issuecomment-6130568809),
[formatting failure](https://github.com/CrispStrobe/labwired-core/pull/210#issuecomment-6131279239),
[Clippy failure and preceding guest records](https://github.com/CrispStrobe/labwired-core/pull/210#issuecomment-6132116201),
and [generated C6 fingerprint failure](https://github.com/CrispStrobe/labwired-core/pull/210#issuecomment-6132947119).
The final repair changed one smoke-board generated hash, not a silicon record.

## Remaining boundary

Software BLOCK/BYTE, one unlinked descriptor, primary RAM and NOACT/INT only.
Unsupported modes refuse; unmapped targets exercise real bus faults. This is
bus-assisted service, not event/performance migration or physical arbitration.
Live SERCOM requests, production clock binding, RX/links/abort/suspend, final
SPI TXC and actual CODAL callbacks/events/fibers remain unqualified. No active
Arcade RTx, full P4, firmware admission or installed app/debugger result follows.
Continue the [bounded live-TX contract](../../engineering/samd-dmac-byte-foundation.md#next-bounded-lane--live-sercom4-tx-byte-requests).
