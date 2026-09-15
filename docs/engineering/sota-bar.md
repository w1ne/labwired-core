# SOTA bar (code we write, crates we reuse)

**Date:** 2026-09-15  
Every change asks: is this the current best practice, or a leftover?

## Do not cargo-cult

Renode / QEMU / Simics are sources, not specs. LabWired’s clock is **Hz + 1 cycle/insn**, not MIPS. Browser JIT must share **wasm bytes** with native wasmtime — that is why we do **not** switch the guest JIT to `cranelift-jit` (docs: experimental) or copy-and-patch (no maintained Cortex-M embed).

## Dependencies (researched 2026-09-15)

| Crate | We use | SOTA | Action |
|---|---|---|---|
| `serde_yaml` 0.9 | Value DOM + `from_str` everywhere | Archived Mar 2024. 2026 options: **noyalib** (pure Rust, Value + serde, YAML 1.2) or **serde-saphyr** (typed only, **no Value**). `serde_yml` is also deprecated. | **Do not migrate in this change.** Include merge needs `Value`. Budget a dedicated PR to noyalib once MSRV and `Value` parity are proven on the chip catalog. |
| `wasmtime` **45** | RV32 JIT | **48.0.2** (2026-09-10). LTS is ×12 (36…). 45 is off LTS and 3 months behind. | Bump only with RISC-V JIT lockstep + C3 OLED gates green. Separate PR. |
| `bincode` 1.3 | snapshots | 2.x exists; 1.3 is frozen but fine for an internal blob. | Leave until snapshot format versioning is designed. |
| `cranelift-jit` | unused | Marked **experimental**. Wasmtime is production Cranelift. | Do not replace wasmtime. |

## Code we wrote that was average

| Practice | Average | SOTA here |
|---|---|---|
| Chip `include` merge | `to_string` → `from_str` so `size: 1024` parses | Coerce numeric `size` on the `Value`, then `from_value` — no text round-trip |
| Optional fields | Serialize `irq_controller: null` | `skip_serializing_if = "Option::is_none"` |
| Thumb JIT | Call it DBT | All-bail walker is a **gate**, not a translator. Emit only after SysTick/NVIC/MMIO contract (see `riscv-jit-side-exit-tick-contract.md`) |
| YAML sugar | Copy `.repl` | Keep YAML; sugar is DX, not TCG |

## Before writing new engine code

1. Read the RISC-V JIT contract.
2. Name the industry analogue (QEMU TB, Simics realtime, …) and the **difference**.
3. Prefer the in-tree lockstep/tick gates over a Renode anecdote.
