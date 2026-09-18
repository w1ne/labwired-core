// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.
//
// `step_inner`'s decode chain, split by instruction class. Each submodule
// holds a verbatim move of one or more contiguous opcode checks out of
// `cpu/avr.rs`; the fetch, `next`-PC setup and the final decode-error
// fallback stay in `step_inner` itself.
//
// Unlike Cortex-M/RISC-V's exhaustive `match` over a decoded enum, AVR's
// original decoder is a sequential chain of mutually exclusive `if (op &
// MASK) == PATTERN` checks, and at least one pair of checks overlaps by
// construction (the generic `LDD`/`STD` mask with q=0 always intercepts the
// opcodes the later bare `LD Rd,Y`/`LD Rd,Z` checks target, making those
// checks dead code — intentionally reproduced, not fixed, since this is a
// pure move). Reordering opcode checks relative to each other is therefore
// NOT safe in general. Every exec_* function below preserves the exact
// original relative order of the checks it contains, and `step_inner` calls
// every function in the exact sequence the checks appeared in the original
// file, so the overall check order is unchanged end to end.
//
// Each exec_* function returns `SimResult<Option<()>>`: `Ok(Some(()))` when
// the opcode matched and was fully handled (mirrors the original arm's
// `return Ok(());`), `Ok(None)` when nothing in this function matched (so
// `step_inner` tries the next function), and `Err(..)` propagates exactly
// like the original `?`/`return Err(..)` did.

pub(in crate::cpu::avr) mod arith;
pub(in crate::cpu::avr) mod bitops;
pub(in crate::cpu::avr) mod branch;
pub(in crate::cpu::avr) mod load_store;
pub(in crate::cpu::avr) mod mul;
pub(in crate::cpu::avr) mod system;
