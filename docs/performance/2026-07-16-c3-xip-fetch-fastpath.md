# C3 model speed: XIP instruction-fetch fast path

**Date:** 2026-07-16  
**Host:** Apple M4, release `event-scheduler`  
**Workload:** `esp32c3-oled-demo` fast-start, 50M instruction budget, idle FF off

## Why

Native samples of the OLED lab showed **~60% of time in instruction fetch**
through `FlashXipPeripheral`: every `RiscV::step` did `bus.read_u32(pc)`, and
the default `Peripheral::read_u32` splits into **four** `read()` calls. Each
byte lock-contended on:

1. the shared MMU table mutex (`translate`)
2. the flash backing mutex

So one instruction fetch paid **~8 mutex lock/unlock pairs**.

This is **model** cost, not firmware — same bytes, same MMU semantics.

## Changes (fidelity-preserving)

1. **`FlashXipPeripheral::read_u32` / `read_u16`** — one translate + one
   backing lock for an in-page span (the common fetch case).
2. **MMU page-translation cache** — last `(generation, entry_id → phys_page)`.
   `SharedMmu.generation` bumps on every MMU register write / snapshot restore
   so remaps cannot leave a stale phys_page.
3. **JIT min block length (16)** — measured: default RV JIT on this lab was a
   **~20× regression** (tiny blocks + wasmtime reg-sync). Refuse to install
   blocks shorter than 16 guest instructions so enabling JIT cannot tank real
   FreeRTOS/driver code; synthetic ALU benches still clear 16 and stay ~30×.

No firmware changes, no fake timers, no skipped I²C.

## Results (cumulative on this branch)

| Tick interval | main (before) | word+xlat cache | +page mirror | +RV fetch window |
|---------------|---------------|-----------------|--------------|------------------|
| 64 | **14.6 MIPS** | 22.3 (~1.5×) | 24.0 | **~27.4 MIPS (~1.9×)** |
| 1 | ~5.2 MIPS | 6.0 | 6.1 | ~6.0 |

Additional on this branch after the first landing:

4. **Lock-free physical-page mirror** — one 64 KiB host page filled under the
   backing mutex; steady-state fetches copy from the mirror (SPI does not
   mutate the flash Vec today; `invalidate_page_mirror` is ready if that changes).
5. **RISC-V 256-byte instruction-fetch window** — bulk XIP/`read_span` into the
   CPU; sequential execution skips per-instruction `find_peripheral_index`
   (the post-XIP profile hotspot). Only side-effect-free code memory; MMIO
   data paths unchanged.

Probe:

```bash
LABWIRED_MIPS_INTERVAL=64 cargo test -p labwired-core --release \
  --features event-scheduler --test esp32c3_walk_differential \
  oled_lab_native_mips_probe -- --ignored --nocapture
```

## What did **not** work for this lab

| Approach | Result |
|----------|--------|
| Enable production RV JIT as-is | 0.66 MIPS (tiny blocks) |
| JIT min-16 only | 12.2 MIPS (still slower than pure interp — dispatch tax) |
| Firmware `delay()` yield | **Rejected** — app hack, not model speed |

## Next model levers (if still needed)

1. Ship this into browser wasm (same XIP path).
2. Further fetch: IRAM-style `fetch_slice` for XIP once SPI-write invalidate
   is wired (avoid stale page mirrors).
3. RV JIT only after profitable-block coverage is proven on C3 OLED
   differential + positive MIPS.
