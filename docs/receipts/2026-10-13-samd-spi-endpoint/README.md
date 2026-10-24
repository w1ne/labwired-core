# SAM SPI controller-local TX endpoint prerequisite

[PR212](https://github.com/CrispStrobe/labwired-core/pull/212) merged on2026-10-13.
This qualifies controller-local readiness/accepted-byte hooks, not a connected
Bus/DMAC path or CPU-driven hardware DMA. The
[software byte foundation](../2026-10-12-samd-dmac-byte/README.md) remains separate.

## Exact source and landing

- Reviewed source: `970ad59d2db8bca9924e9a4041b9610b8ae897d1`.
- Base: `c9dbe2b37eb85775bd213aa2b97d2ccd2d2a9e90`.
- Actual tested checkout: `28f5c45c90e91b385aaf2a8e9f4d44a8ef83856c`.
- Landed merge: `5d815bda2a9a7d9f5b74bca13d61dc98b38abe61`.
- Reviewed/tested/landed tree: `988c2cfdc81f0494b6abc996991af2768f2c5946`;
  both tested and landed parents match the declared base/source.
- [Rust38342351934](https://github.com/CrispStrobe/labwired-core/actions/runs/38342351934),
  pull-request attempt1, succeeded. All22 enabled final-head checks passed;
  only `ci-runner-image`, `core-full`, `core-integrity`, `pr-gate-warm` skipped
  under their declared conditions. Normal exact-head merge, no gate bypass.

## Actual originals and test roles

| Profile | Original job | SPI split cases / entries / returns | Byte cases / entries / returns |
| --- | --- | --- | --- |
| Off | [115115948057](https://github.com/CrispStrobe/labwired-core/actions/runs/38342351934/job/115115948057) |24 /8 /8 |90 /36 /36 |
| On | [115115948130](https://github.com/CrispStrobe/labwired-core/actions/runs/38342351934/job/115115948130) |48 /16 /16 |90 /36 /36 |

Off original:657086 bytes, SHA256
`4aef798ed26b83268542a156a4bb9286268173116c8c3a46e41f33ce58913f02`.
On original:123238 bytes, SHA256
`d71f59824697e3624d2cb1d78ce4748df06694b93a015a5cb4f5b895cf5946e4`.
A stdlib-only reader frozen before original acquisition checked exact profile
rows, named endpoint/SPI/byte tests, official source/job/affected-step bindings
and actual checkout/tree equality. No guest replay was used to read the logs.

The new **host** control checks default refusal, disabled/full endpoints, the
exact low DATA lane, INTEN independence, and actual slave observation of an
accepted byte despite a later ordinary store returning success while dropping
data. It also checks reset discard of an accepted but not yet shifted byte.
The direct call explicitly refreshes the test pump; this is not production bus
or DMA wiring. Existing authored **CPU guests** prove SPI/byte-DMA regressions
still pass; they do not exercise a new CPU-driven peripheral DMA operation.

## Remaining admission

The next bus seam must preserve live clocks, routing/storage precedence,
observer notifications, scheduler wakes and IRQ/pump-index reconciliation.
Do not bypass those responsibilities with a raw peripheral call. Then qualify
CPU-created hardware-triggered descriptors, writeback/error behavior and final
SPI TXC separately from DMA completion under the
[live TX contract](../../engineering/samd-dmac-byte-foundation.md#next-bounded-lane--live-sercom4-tx-byte-requests).
No production descriptor, app pin, capture, acknowledgement or threshold moved.
No physical timing, full P4/CODAL/native Arcade, active RTx, whole-firmware
provenance or licence/store clearance follows from this prerequisite.
