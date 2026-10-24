# Actual Timer caller adversaries — prospective builder

This separate lane preserves the successful positive caller run
[38169003939](https://github.com/CrispStrobe/labwired-core/actions/runs/38169003939).
It changes only authored adapter sources at the immutable CODAL source
`d55214588780d737640536b0a58707cd6d69cfce`; complete upstream `Timer.cpp`
remains unchanged. Generated derivatives are explicitly mutants, not original
source or clean-room implementations.

| Mutant | Authored change | Expected guest rejection, not yet observed |
| --- | --- | --- |
| Unconditional restore | Enable IRQs after every actual paired leave | Constructor guard `0xb02`, thread mode |
| Early restore | Enable IRQs before applying actual paired leave | Open-pair handler guard `0xa14`, exception16 |
| Unsupported event | Enter excluded event constructor explicitly | Adapter rejection `0xa07`, thread mode |

`build.py` first requires the clean selected source and matching positive
ELF/map/source manifest. It refuses missing or ambiguous mutation sites, retains
the unchanged Timer/startup objects, replaces only the named adapter unit,
and records original/generated source, object and linked-image identities.
Compilation/link failure is not successful guest rejection. The manifest's
`expected_failure` is a hypothesis, not execution evidence.

Still owed: hosted build of all three mutants, complete compiler/header/source
binding and producer controls, then bounded actual guests in both scheduler
profiles. Require exact rejection/fault/mask/context, no success status, and
64 consecutive terminal observations; timeout, missing witness, wrong failure,
or an unrelated compiler/engine failure must refuse. Preserve raw logs and
original source-bound results, separately audit, and pass every final-head gate
before merging the Timer caller lane. This preparatory builder alone qualifies
neither the mutants nor the real runtime, licensing, hardware or performance.
