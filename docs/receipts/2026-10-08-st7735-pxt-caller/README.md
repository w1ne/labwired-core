# PXT caller component qualification — 2026-10-08

[PR172](https://github.com/CrispStrobe/labwired-core/pull/172) source
`0c63615f38782fd96008cd337aab97c5f5255963` passed all fifteen enabled checks;
the four declared full/warm/image jobs skipped. Merge
`ef6e085b3163200f2e6b49356dc2cc8d5a602a6b` has tree
`91b1ca3633ca0ff75da8511d2e6b9a92579c9327`, identical to tested PR checkout
`0a80981bb91ac516973a3913155f4dd02a9b8c10` and the reviewed merge preview.

## Actual observations

- [Focused run 37728675697, job 113152589066](https://github.com/CrispStrobe/labwired-core/actions/runs/37728675697/job/113152589066)
  passed the original method capture, sixteen mocked admission controls,
  both caller builds and the corrupt-first-RAMWR-byte negative. Compiler:
  Ubuntu g++ 13.3.0-6ubuntu2~24.04.1, 32-bit host x86, not ARM.
- Each `USE_RGB444` enabled/absent build ran eleven cases: full frame, padded
  partial frame, aligned and padded main/status splits, predicate-off,
  missing-LCD and doubled fallbacks, invalid dimensions/bpp, synthetic capacity
  rejection and missing-display/reentry rejection.
- A separate read-only standard-library audit, without importing source helpers,
  checked all 93,360 emitted RAMWR bytes, row/column/padding sentinels, transfer
  lengths, copy lengths, branch/capacity verdicts and the exact MIT notice.
  Full 160×128 emits 30,720 bytes; partial 160×5 emits 1,200; aligned 120+8
  emits 28,800+1,920; padded 125+3 emits 30,000+720.
- [Core run 37728675497](https://github.com/CrispStrobe/labwired-core/actions/runs/37728675497)
  passed all nine existing SAM guest tests in each scheduler configuration
  (jobs 113152588677 and 113152588812), with zero failed, ignored or filtered.
  These are separate existing authored guests, not execution of PXT on ARM.

## Immutable artifact identity

The original [Actions artifact 11528831923](https://api.github.com/repos/CrispStrobe/labwired-core/actions/artifacts/11528831923/zip),
`st7735-pxt-direct-fragment-trace`, reports ZIP size 25,074 bytes. Preserve its
raw members without rewriting. Actions retention may expire; this compact
index is not a replacement for the full raw caller capture.

| Member | Bytes | SHA256 |
| --- | ---: | --- |
| `st7735-pxt-caller-trace.json` | 1,925,418 | `2bbaa4c66b083cc1eab7002b1211e474d1189d521d7c9bfe4f4efdc860e21dd8` |
| `st7735-pxt-direct-trace.json` | 44,948 | `82803444d5a6079388de7f924a451d5781b47abd1eb6ae4a95dc8952e2220a04` |
| `LICENSE-pxt-common-packages.txt` | 1,183 | `dea9265341829002e2c23a7372393eb2ed6e26085fb623f38a4ba0af833f30a6` |

The direct-method member is byte-identical to the earlier
[PR169 capture](../2026-10-07-st7735-pxt-direct/README.md).
Exact Microsoft source/fragment hashes and authored boundaries are enforced by
[`caller_run.py`](../../../scripts/st7735-pxt-direct-trace/caller_run.py)
and documented in [CALLER.md](../../../scripts/st7735-pxt-direct-trace/CALLER.md).

## Limits

Original ImageHeader/accessors/updateScreen/method fragments execute inside
authored objects. RefImage inheritance, allocation, GC, original constructors,
palette/configuration/SPI/window/wait/fallback implementations do not execute.
The synthetic 160×1 copy is rejected by an authored interceptor before mutation;
it is not an original guard or deployed overflow. Observed partial/status
padding is not proof that those dimensions are supported in production.

This does not establish the real runtime compiler/macros, loaded CF2, selected
constructor, module straps/aperture/LUT, glass colour, full ARM runtime, DMA/IRQ,
active PyBadge RTx, browser WASM or app adoption. P3/CP14 remain open.
Continue with the [production evidence lanes](../../engineering/st7735-production-next-lanes.md).
