;; LabWired - Firmware Simulation Platform
;; Copyright (C) 2026 Andrii Shylenko
;; SPDX-License-Identifier: MIT
;;
;; Xtensa LX7 JIT — hot basic block at PC 0x400829cc (#124 Phase 4).
;;
;; This file is the SINGLE source of truth for the JIT'd block's wasm body.
;; Consumed two ways:
;;   * `crates/core/build.rs` reads it via fs::read_to_string and
;;     wat::parse_str's it into wasm bytes baked at `OUT_DIR/xtensa_jit_hot_bb.wasm`.
;;   * `crates/core/src/cpu/xtensa_jit/wasm_bytes.rs` `include_bytes!`s
;;     the baked artifact; both the native (wasmtime) and browser
;;     (`js_sys::WebAssembly`) backends consume those bytes verbatim.
;;
;; Block disassembly + exit-code semantics live in `bb_multi.rs`'s module
;; docs. Side-exit code 0 = clean fall-through to PC 0x400829e4; code 5 =
;; host bus error reported by the `host.read_u8` import.
(module
  (import "host" "read_u8" (func $read_u8 (param i32) (result i32)))
  (func (export "run")
        (param $a3 i32) (param $a5 i32) (param $l32r i32)
        (result i32 i32 i32 i32 i32)
    (local $a2 i32)
    (local $a6 i32)
    (local $a8 i32)
    (local $a10 i32)
    (local $tmp i32)

    ;; 1. or a10, a5, a5  -> a10 = a5
    (local.set $a10 (local.get $a5))

    ;; 2. memw — barrier, semantic no-op in sim

    ;; 3. l8ui a6, a3, 0  -> a6 = read_u8(a3 + 0)
    (local.set $tmp (call $read_u8 (local.get $a3)))
    (if (i32.lt_s (local.get $tmp) (i32.const 0))
      (then
        (return (i32.const 5) (i32.const 0) (i32.const 0) (i32.const 0) (i32.const 0))))
    (local.set $a6 (i32.and (local.get $tmp) (i32.const 0xFF)))

    ;; 4. memw — barrier

    ;; 5. l8ui a2, a3, 1  -> a2 = read_u8(a3 + 1)
    (local.set $tmp (call $read_u8 (i32.add (local.get $a3) (i32.const 1))))
    (if (i32.lt_s (local.get $tmp) (i32.const 0))
      (then
        (return (i32.const 5) (i32.const 0) (i32.const 0) (i32.const 0) (i32.const 0))))
    (local.set $a2 (i32.and (local.get $tmp) (i32.const 0xFF)))

    ;; 6. extui a2, a2, 0, 8  -> a2 = (a2 >> 0) & ((1<<8) - 1) = a2 & 0xFF
    (local.set $a2 (i32.and (local.get $a2) (i32.const 0xFF)))

    ;; 7. and a2, a2, a6  -> a2 &= a6
    (local.set $a2 (i32.and (local.get $a2) (local.get $a6)))

    ;; 8. l32r a8, 0x40080534  -> a8 = pre-resolved literal
    (local.set $a8 (local.get $l32r))

    (i32.const 0)
    (local.get $a2)
    (local.get $a6)
    (local.get $a8)
    (local.get $a10)
  )
)
