# MISRA C:2012 Deviations

The `iolinki-master` protocol core (`src/master_*.c`, `include/iolinki_master/master.h`)
targets MISRA C:2012. It is checked with `cppcheck --addon=misra` (run by
`check_quality.sh`, opt-in). This file records the deviations that are
**accepted** rather than fixed, with rationale. Rules not listed here are either
clean or fixed in code.

Cleared in code: Rule 17.7 (ignored `memcpy`/`memset` returns now `(void)`-cast),
Rule 13.4 (assignment-in-`while` replaced with an explicit read + `break`),
Rule 15.7 (all `if … else if` chains terminated with an `else`).

## Accepted deviations

| Rule | Type | Where | Rationale |
| --- | --- | --- | --- |
| **11.5** | Required | `master_internal.h` opaque-storage accessors (×4) | `void*` → private-state pointer. The public ABI is caller-owned, heap-free, and opaque; the `_storage_must_fit` static asserts guarantee size and the `iolink_master_*_t` union alignment members guarantee alignment. Annotated at the source. |
| **19.2** | Advisory | `master.h` `iolink_master_port_t` / `_controller_t` (×N) | The `union` keyword. Used only for the opaque caller-owned storage types, which need a fixed size and worst-case alignment. No other unions exist. |
| **10.4** | Required | status/error comparisons (×22) | The public API returns `int` for status/error codes (a deliberate, forward-compatible ABI choice) and compares against named `IOLINK_MASTER_*` enum constants. The comparisons are value-correct; unifying the essential type would change the public return type. |
| **15.5** | Advisory | throughout (×~200) | Multiple `return` statements (guard-clause early exits). The style is the established idiom in this stack and is clearer than deep nesting; single-exit restructuring would reduce readability. |
| **13.3** | Advisory | buffer index post-increments (×9) | `buf[i++]` within a larger expression. Local, idiomatic, and clear; no sequencing ambiguity. |
| **10.8** | Advisory | `master_parameters.c`, `master_port.c` (×2) | Composite expression cast to a narrower type / enum. Each operand is masked to range before the cast, so the conversion is value-safe. |
| **9.3** | Advisory | `{0U}` array initializers (×2) | A partial initializer that zero-initializes the whole array per C. Intentional. |
| **2.3 / 2.4 / 2.5** | Advisory | public header (×N) | Typedefs / tags / macros declared in the installed public header but unused by the single translation unit under analysis. They are part of the API surface, not dead code. |
| **8.7** | Advisory | public API functions (×3) | Flagged as "could be static", but these are the public API declared in `include/iolinki_master/master.h` and have external callers. False positive. |

## Re-running the check

```sh
IOLINKI_MASTER_MISRA_ENFORCE=1 ./check_quality.sh
```

The MISRA stage is skipped when the cppcheck MISRA addon is not installed, unless
`IOLINKI_MASTER_MISRA_ENFORCE=1` is set (then a missing addon fails the gate).
