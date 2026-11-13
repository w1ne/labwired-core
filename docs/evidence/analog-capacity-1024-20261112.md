# Explicit analog capacity up to 1024 unknowns

The optional `config.max_unknowns` upper limit is extended from 512 to 1024.
The default is still 128, including `Solver::new`. No circuit is automatically
raised to the new maximum. A caller can choose the exact required size, such
as 667. A circuit above its selected bound is rejected before dense allocation;
a configured bound above 1024 is rejected even for a small circuit.

## Source-bound need

The exact combined native FC and four-ESC electrical export has **667 unknowns**.
Its deck SHA-256 is
`67422deca6e3f0820174c404f3ce7149f966cdb6c6ed0c0267eb8f6e213a6fce`.
The standalone FC has 215 unknowns and remains within its explicit 512 limit.
The [count receipt](analog-capacity-1024-20261112/native-count-receipt.json)
records the frozen production runner and separate `max_unknowns: 1` diagnostic
which refused both decks before operating-point allocation or solve. Neither
original 512 export was changed by that count diagnostic.

The combined runtime will choose an explicit 667 limit in a separately hashed
export. This change establishes bounded solver admission only. It does not
qualify the pending regulator/controller model, firmware startup, motor motion
or a flight. The historical 379-unknown native capacity receipt and its trace
are unchanged in [the original evidence](analog-capacity-20261112.md).

## Allocation budget

`Solver::new_with_max_unknowns` owns two dense `Vec<f64>` buffers: `matrix`
and `factors`. Each has `N*N` elements. LU modifies the matrix in place; it
allocates no third dense matrix. Their combined size is `16*N*N` bytes:

| Unknowns | Two dense buffers | Fixed linear work buffers, 64-bit host |
| ---: | ---: | ---: |
| 128 | 262,144 bytes (256 KiB) | 5,120 bytes |
| 512 | 4,194,304 bytes (4 MiB) | 20,480 bytes |
| 667 | 7,118,224 bytes (6.7885 MiB) | 26,680 bytes |
| 1024 | 16,777,216 bytes (16 MiB) | 40,960 bytes |

The fixed linear count includes three `N`-element `f64` vectors (`rhs`,
`solution`, `previous`), `N` pivot indices, and the node-voltage plus
branch-current vectors, whose lengths sum to `N`. Wasm32 has narrower pivot
indices. Parsed elements, per-element reactive/nonlinear histories, trace
storage, allocator overhead and host process memory are additional. The limit
therefore bounds the quadratic solver buffers, not all host memory.

[Machine-readable accounting](analog-capacity-1024-20261112/allocation-budget.json)
preserves this calculation. LU remains cubic in the worst case. The source
changes neither stamping, integration, factorization nor step budgets.

## Verification contract

The focused regressions preserve default-129 refusal, the prior explicit
379/512 analytic ladders and explicit-512 refusal of 513 unknowns. They add
complete analytic solutions at explicit 667 and 1024, refusal of 1025 before
allocation, and malformed/out-of-range configuration cases. Small RC
transients must remain bit-identical with the default, explicit 512 and
explicit 1024 in both integration modes.

The manifest validator, shared World, production CLI and Wasm constructor must
all use the same selected bound. The CLI executes two bounded steps for the
667-unknown analytic circuit. The exact native combined circuit's controller
and power-up tests belong to the separate integrated runtime receipt.

## Focused results

All **47 focused tests passed**: 45 core solver/adapter/capacity/golden/copper
checks, one production CLI case matrix, and one Wasm constructor case matrix.
The CLI test executed a **synthetic analytic 667-unknown resistor ladder** for
two steps at an explicit limit of 667. It did not execute the native deck
`67422deca6e3f0820174c404f3ce7149f966cdb6c6ed0c0267eb8f6e213a6fce`.

The [command receipt](analog-capacity-1024-20261112/focused-gates.json) binds the
source and logs by SHA-256. The checks also prove exact node values through
1024 unknowns, unchanged RC bit patterns in both integration modes, and the
same refusal bounds through standalone, World, CLI and Wasm construction.
Normal repository commit and push hooks remain required for publication.
