// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! [`GuestBuf`] — the backing byte buffer of a [`LinearMemory`](super::LinearMemory).
//!
//! This is a `Vec<u8>` replacement with one extra property: the allocation is
//! shaped so that the **RV32IMC wasm-JIT can import it directly as a wasm
//! linear memory**, making guest RAM and the JIT's wasm memory *the same
//! bytes*. Without that, every compiled-block run had to copy the whole guest
//! RAM window into the wasm memory and back out again (~800 KB per run on a
//! 400 KB-RAM ESP32-C3) — which made short blocks catastrophically
//! unprofitable and is why the JIT was a net regression on real firmware.
//!
//! ## Layout
//!
//! ```text
//!   0                    JIT_PREFIX_BYTES        JIT_PREFIX_BYTES+len   capacity
//!   ├────────────────────┼───────────────────────┼──────────────────────┤
//!   │  JIT control area  │      guest bytes      │   wasm-page padding  │
//!   │ (regs, PC/fault    │  (what `data` derefs  │  (never guest-       │
//!   │  slots — see       │   to; the guest sees  │   visible; exists so
//!   │  `jit_framework::  │   exactly `len`)      │   the whole thing is
//!   │  riscv::emit`)     │                       │   a whole wasm-page
//!   │                    │                       │   count)
//! ```
//!
//! The prefix is *not* guest-visible: [`Deref`] hands out only the `len`
//! guest bytes, so every existing `LinearMemory` accessor keeps indexing from
//! guest offset 0 exactly as it did over the old `Vec<u8>`.
//!
//! ## Why the buffer is shared, and why that is sound
//!
//! The allocation lives behind an [`Arc`] so the JIT can hold a second handle
//! to it (see `jit_framework::riscv::exec::SharedGuestMemory`) and hand its
//! raw pointer to `wasmtime` via the `MemoryCreator` trait. Both the
//! interpreter (through `LinearMemory`) and compiled wasm blocks then address
//! one buffer with **no copy in either direction**.
//!
//! Aliasing is sound because access is strictly *interleaved*, never
//! concurrent:
//!
//!   * Rust code (the interpreter) touches the bytes only through `&`/`&mut`
//!     borrows derived on demand from the raw pointer; no such borrow is ever
//!     held across a call into wasm.
//!   * wasm touches the bytes only inside `TypedFunc::call`, at which point
//!     the caller holds no Rust reference into the buffer (`CompiledBlock::run`
//!     takes `&mut self` and the register sync completes before the call).
//!   * The simulator is single-threaded per `Machine`; `GuestBuf` is `Send`
//!     but the JIT never shares one buffer across threads.
//!
//! The allocation never moves or resizes in place: [`GuestBuf::resize`]
//! builds a *fresh* allocation, and the JIT re-binds (dropping its block
//! cache) when it observes the buffer identity change — see
//! `RiscvJitEngine::try_compile_from_bus`.

use std::alloc::{alloc_zeroed, dealloc, Layout};
use std::ops::{Deref, DerefMut};
use std::ptr::NonNull;
use std::sync::Arc;

/// Bytes reserved at the front of every [`GuestBuf`] allocation for the wasm
/// JIT's control area (guest register file, next-PC / fault / reservation
/// slots). Guest byte 0 lives at this offset in the underlying allocation.
///
/// Must equal `jit_framework::riscv::emit::RAM_WINDOW_OFF` — asserted by
/// `jit_prefix_matches_ram_window_off` in that module.
pub const JIT_PREFIX_BYTES: usize = 256;

/// Size of a WebAssembly page. The allocation is rounded up to a whole
/// number of these so it can back a wasm linear memory verbatim.
pub const WASM_PAGE_BYTES: usize = 65536;

/// The raw, page-rounded allocation behind a [`GuestBuf`].
///
/// # Safety
///
/// Holds a unique owning pointer to `layout`-sized zeroed memory. `Send`/`Sync`
/// are asserted manually because the pointer is the *only* non-`Send` thing
/// about it: the buffer is plain bytes with no interior invariants, and the
/// interleaving discipline described in the module docs — not the type system —
/// is what rules out data races.
struct GuestAlloc {
    ptr: NonNull<u8>,
    layout: Layout,
    /// Guest-visible byte count (excludes the prefix and the page padding).
    len: usize,
}

// SAFETY: `GuestAlloc` is an owning box of plain bytes. See the type's docs and
// the module-level aliasing argument.
unsafe impl Send for GuestAlloc {}
unsafe impl Sync for GuestAlloc {}

impl GuestAlloc {
    fn new(len: usize) -> Self {
        // Prefix + guest bytes, rounded up to a whole number of wasm pages so
        // the same allocation can be handed to wasmtime as a linear memory.
        let total = (JIT_PREFIX_BYTES + len)
            .next_multiple_of(WASM_PAGE_BYTES)
            .max(WASM_PAGE_BYTES);
        // Page-align the base: `wasmtime::LinearMemory` requires it.
        let layout = Layout::from_size_align(total, WASM_PAGE_BYTES)
            .expect("guest allocation layout is always valid");
        // SAFETY: `total >= WASM_PAGE_BYTES > 0`, so the layout is non-zero-sized.
        let ptr = unsafe { alloc_zeroed(layout) };
        let ptr = NonNull::new(ptr).unwrap_or_else(|| std::alloc::handle_alloc_error(layout));
        Self { ptr, layout, len }
    }

    /// Pointer to guest byte 0 (i.e. past the JIT control prefix).
    fn guest_ptr(&self) -> *mut u8 {
        // SAFETY: `JIT_PREFIX_BYTES + len <= layout.size()` by construction.
        unsafe { self.ptr.as_ptr().add(JIT_PREFIX_BYTES) }
    }
}

impl Drop for GuestAlloc {
    fn drop(&mut self) {
        // SAFETY: `ptr` came from `alloc_zeroed` with exactly `layout`.
        unsafe { dealloc(self.ptr.as_ptr(), self.layout) }
    }
}

/// The backing buffer of a [`LinearMemory`](super::LinearMemory): behaves like
/// a `Vec<u8>` of the guest bytes (via [`Deref`]/[`DerefMut`]) while keeping
/// the allocation shaped for direct import as a wasm linear memory.
///
/// Cloning copies the bytes into a fresh allocation (like `Vec`), so a clone
/// is *not* aliased with the original.
pub struct GuestBuf {
    alloc: Arc<GuestAlloc>,
}

impl GuestBuf {
    /// A zeroed buffer of `len` guest bytes.
    pub fn new(len: usize) -> Self {
        Self {
            alloc: Arc::new(GuestAlloc::new(len)),
        }
    }

    /// Replace the contents with a `len`-byte zeroed buffer, in a **fresh**
    /// allocation (any JIT binding to the old one is thereby orphaned; the JIT
    /// detects this by identity and re-binds).
    pub fn resize(&mut self, len: usize) {
        *self = Self::new(len);
    }

    /// Handle to the whole page-rounded allocation, for the JIT to import as a
    /// wasm linear memory. Returns the base pointer (start of the control
    /// prefix, i.e. wasm offset 0), the total byte size, and an [`Arc`] that
    /// keeps the allocation alive for as long as wasmtime holds it.
    ///
    /// # Safety
    ///
    /// The caller must only touch the returned region while no `&`/`&mut`
    /// borrow of this `GuestBuf` is live — see the module docs.
    #[allow(dead_code)] // live only on the JIT path; exercised by unit tests
    pub(crate) unsafe fn raw_shared(&self) -> (*mut u8, usize, Arc<dyn Send + Sync>) {
        let alloc = Arc::clone(&self.alloc);
        let ptr = alloc.ptr.as_ptr();
        let size = alloc.layout.size();
        (ptr, size, alloc as Arc<dyn Send + Sync>)
    }

    /// Stable identity of the underlying allocation. The JIT compares this
    /// across compiles to notice a buffer swap (e.g. a test replacing
    /// `bus.ram.data`) and re-bind instead of addressing a stale allocation.
    #[allow(dead_code)] // live only on the JIT path; exercised by unit tests
    pub(crate) fn alloc_id(&self) -> usize {
        Arc::as_ptr(&self.alloc) as usize
    }
}

impl Deref for GuestBuf {
    type Target = [u8];

    fn deref(&self) -> &[u8] {
        // SAFETY: `guest_ptr()` is valid for `len` initialised bytes, and no
        // wasm call is in flight while this borrow lives (module docs).
        unsafe { std::slice::from_raw_parts(self.alloc.guest_ptr(), self.alloc.len) }
    }
}

impl DerefMut for GuestBuf {
    fn deref_mut(&mut self) -> &mut [u8] {
        // SAFETY: as `deref`, plus: `&mut self` proves no other Rust borrow of
        // this buffer is live. The JIT's second `Arc` handle is only ever
        // dereferenced from inside a wasm call, which cannot be in flight here.
        unsafe { std::slice::from_raw_parts_mut(self.alloc.guest_ptr(), self.alloc.len) }
    }
}

impl Clone for GuestBuf {
    fn clone(&self) -> Self {
        let mut out = Self::new(self.alloc.len);
        out.copy_from_slice(self);
        out
    }
}

impl From<Vec<u8>> for GuestBuf {
    fn from(v: Vec<u8>) -> Self {
        let mut out = Self::new(v.len());
        out.copy_from_slice(&v);
        out
    }
}

impl From<&[u8]> for GuestBuf {
    fn from(v: &[u8]) -> Self {
        let mut out = Self::new(v.len());
        out.copy_from_slice(v);
        out
    }
}

impl std::fmt::Debug for GuestBuf {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GuestBuf")
            .field("len", &self.alloc.len)
            .finish_non_exhaustive()
    }
}

impl PartialEq for GuestBuf {
    fn eq(&self, other: &Self) -> bool {
        **self == **other
    }
}

impl Eq for GuestBuf {}

impl serde::Serialize for GuestBuf {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        serde::Serialize::serialize(&**self, s)
    }
}

impl<'de> serde::Deserialize<'de> for GuestBuf {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let v = Vec::<u8>::deserialize(d)?;
        Ok(Self::from(v))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn guest_bytes_are_zeroed_and_addressable() {
        let mut b = GuestBuf::new(400 * 1024);
        assert_eq!(b.len(), 400 * 1024);
        assert!(b.iter().all(|&x| x == 0));
        b[0] = 0xAB;
        b[400 * 1024 - 1] = 0xCD;
        assert_eq!(b[0], 0xAB);
        assert_eq!(b[400 * 1024 - 1], 0xCD);
    }

    #[test]
    fn allocation_is_page_rounded_and_prefix_backed() {
        // 400 KB of guest RAM + a 256-byte prefix must round up to 7 wasm pages.
        let b = GuestBuf::new(400 * 1024);
        // SAFETY: test only inspects the reported geometry.
        let (ptr, size, _keep) = unsafe { b.raw_shared() };
        assert_eq!(size, 7 * WASM_PAGE_BYTES);
        assert_eq!(size % WASM_PAGE_BYTES, 0);
        assert_eq!(ptr as usize % WASM_PAGE_BYTES, 0, "page-aligned base");
        // Guest byte 0 sits exactly `JIT_PREFIX_BYTES` into the allocation.
        assert_eq!(b.as_ptr() as usize - ptr as usize, JIT_PREFIX_BYTES);
    }

    #[test]
    fn clone_is_a_deep_copy_not_an_alias() {
        let mut a = GuestBuf::new(64);
        a[0] = 1;
        let mut c = a.clone();
        c[0] = 2;
        assert_eq!(a[0], 1, "clone must not alias the original");
        assert_ne!(a.alloc_id(), c.alloc_id());
    }

    #[test]
    fn resize_reallocates_and_changes_identity() {
        let mut b = GuestBuf::new(16);
        let before = b.alloc_id();
        b.resize(32);
        assert_eq!(b.len(), 32);
        assert_ne!(b.alloc_id(), before, "JIT must be able to notice the swap");
    }

    #[test]
    fn zero_length_buffer_is_valid() {
        let b = GuestBuf::new(0);
        assert_eq!(b.len(), 0);
        // SAFETY: test only inspects geometry.
        let (_p, size, _k) = unsafe { b.raw_shared() };
        assert_eq!(size, WASM_PAGE_BYTES, "never a zero-sized allocation");
    }
}
