//! Embedded privileged ARMv7 Cortex-A7 with physical-bus callbacks.
//!
//! The opt-in `native` backend requires x86_64 Linux GNU and panic unwinding.
//! Production builds use `cargo build --profile native-system --features native`.
//! Instances own distinct shared-library inodes; each library's native globals
//! are private. An idle instance may move between threads; mutable methods
//! prevent concurrent execution of one CPU.
#[cfg(all(feature = "native", panic = "abort"))]
compile_error!(
    "native ARM callback containment requires panic=unwind; build with --profile native-system"
);

/// Physical addresses after the guest MMU. Width is exactly 1, 2, 4 or 8 bytes.
/// State is the native transaction state, passed without reinterpretation.
pub trait PhysicalBus {
    /// Stable allocation-unique identity; a replacement bus must get a new ID.
    /// Host code edits within one bus require `ArmCpu::invalidate_code`.
    fn code_identity(&self) -> u64;
    fn read(&mut self, address: u64, width: u32, state: u64) -> Result<u64, String>;
    fn write(&mut self, address: u64, width: u32, value: u64, state: u64) -> Result<(), String>;
    /// External CP15 accesses only. Architecturally internal registers stay native.
    /// Canonical register_key: 32-bit `(opc1 << 21) | (CRn << 16) |
    /// (opc2 << 5) | CRm`; 64-bit `(opc1 << 4) | CRm`.
    /// Keys omit opcode, condition, Rt and read/write direction.
    fn cp15_read(&mut self, register_key: u32, wide: bool) -> Result<u64, String>;
    fn cp15_write(&mut self, register_key: u32, wide: bool, value: u64) -> Result<(), String>;
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Execution {
    pub retired: u64,
    pub waiting: bool,
}

/// Encoding captured from the instruction's native translated physical pages.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Instruction {
    pub pc: u32,
    pub opcode: u32,
}

/// Diagnostic JIT allocation state; this is not restorable CPU state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TranslationCacheStatus {
    pub size_bytes: u64,
    pub allocations: u64,
}

#[cfg(feature = "native")]
mod native;
#[cfg(feature = "native")]
pub use native::ArmCpu;

#[cfg(not(feature = "native"))]
pub struct ArmCpu;
#[cfg(not(feature = "native"))]
impl ArmCpu {
    pub fn new() -> Result<Self, String> {
        Err("ARM system backend requires the native feature".into())
    }
    pub fn register(&mut self, _: u8) -> Result<u32, String> {
        Self::disabled()
    }
    pub fn set_register(&mut self, _: u8, _: u32) -> Result<(), String> {
        Self::disabled()
    }
    pub fn executable_range(&mut self, _: u64, _: u64) -> Result<(), String> {
        Self::disabled()
    }
    pub fn interrupt_levels(&mut self, _: bool, _: bool) -> Result<(), String> {
        Self::disabled()
    }
    pub fn execute(&mut self, _: &mut dyn PhysicalBus, _: u32) -> Result<Execution, String> {
        Self::disabled()
    }
    pub fn execute_traced(
        &mut self,
        _: &mut dyn PhysicalBus,
    ) -> Result<(Execution, Option<Instruction>), String> {
        Self::disabled()
    }
    pub fn invalidate_code(&mut self) -> Result<(), String> {
        Self::disabled()
    }
    pub fn translation_cache_status(&mut self) -> Result<TranslationCacheStatus, String> {
        Self::disabled()
    }
    pub fn reset(&mut self) -> Result<(), String> {
        Self::disabled()
    }
    fn disabled<T>() -> Result<T, String> {
        Err("ARM system backend requires the native feature".into())
    }
}
#[cfg(all(test, not(feature = "native")))]
mod tests {
    #[test]
    fn feature_off_is_explicit() {
        assert!(super::ArmCpu::new()
            .err()
            .unwrap()
            .contains("native feature"));
    }
}

/// Pinned upstream source lineage is separate from compiled binary identity.
pub const SOURCE_REVISION: &str = "167decf9129758829762e582939128c0694e90d6";
/// SHA256 of the actual embedded native shared-library bytes; feature-off
/// builds have no compiled backend and return None.
pub fn compiled_backend_sha256() -> Option<String> {
    #[cfg(feature = "native")]
    {
        use sha2::{Digest, Sha256};
        Some(format!(
            "{:x}",
            Sha256::digest(include_bytes!(env!("LABWIRED_ARM_SYSTEM_LIBRARY")))
        ))
    }
    #[cfg(not(feature = "native"))]
    {
        None
    }
}
#[cfg(test)]
mod identity_tests {
    #[test]
    fn embedded_binary_identity_is_feature_scoped() {
        let identity = super::compiled_backend_sha256();
        #[cfg(feature = "native")]
        {
            let hash = identity.expect("native build must identify its embedded binary");
            assert_eq!(hash.len(), 64);
            assert!(hash.bytes().all(|c| c.is_ascii_hexdigit()));
        }
        #[cfg(not(feature = "native"))]
        assert!(identity.is_none());
    }
}
