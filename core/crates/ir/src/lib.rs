//! LabWired Intermediate Representation (IR)
//!
//! This crate defines the portable, serializable data structures used to model hardware peripherals
//! in the LabWired ecosystem. It serves as the common language between:
//!
//! 1. **Ingestion Tools**: Parsers for SVD, IP-XACT, or Datasheets (PDF/HTML).
//! 2. **Generator Tools**: Code generators that produce Rust/C++ simulation models.
//! 3. **The Simulator Core**: Which can load these dynamic models at runtime.

#![warn(missing_docs)]

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[cfg(feature = "svd")]
pub mod svd_transform;

/// The top-level root of a chip description.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IrDevice {
    /// The name of the device (e.g., "STM32F103").
    pub name: String,

    /// Optional description of the device.
    pub description: Option<String>,

    /// Map of peripherals, keyed by their instance name (e.g., "USART1").
    /// Note: This map flattens any hardware clusters or arrays.
    pub peripherals: HashMap<String, IrPeripheral>,

    /// Map of interrupt names to their vector number.
    pub interrupt_mapping: HashMap<String, u32>,
}

/// A distinct hardware block mapped to a memory address.
///
/// This structure removes SVD complexity (clusters, arrays) and presents a flat view.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IrPeripheral {
    /// The instance name of the peripheral (e.g., "USART1").
    pub name: String,

    /// The absolute base address of the peripheral in the memory map.
    pub base_address: u64,

    /// Optional description.
    pub description: Option<String>,

    /// Flat list of registers belonging to this peripheral.
    /// Arrays and clusters are unrolled into this list with absolute offsets.
    pub registers: Vec<IrRegister>,

    /// List of interrupts localized to this peripheral block (if any).
    #[serde(default)]
    pub interrupts: Vec<IrInterrupt>,
}

/// A 32-bit (or similar) storage unit within a peripheral.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IrRegister {
    /// The flattened name of the register (e.g., "GPIO_A_MODER").
    pub name: String,

    /// The absolute offset of the register relative to the peripheral base address.
    pub offset: u64,

    /// The size of the register in bits (usually 32).
    pub size: u32,

    /// The access permissions for the entire register.
    pub access: IrAccess,

    /// The value of the register after a system reset.
    pub reset_value: u64,

    /// The bit-fields contained in this register.
    pub fields: Vec<IrField>,

    /// Optional description.
    pub description: Option<String>,
}

/// A named bit-range within a register with specific behavior.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IrField {
    /// The name of the field (e.g., "TXE").
    pub name: String,

    /// The bit position of the least significant bit of the field.
    pub bit_offset: u32,

    /// The width of the field in bits.
    pub bit_width: u32,

    /// Specific access permissions for this field, if different from the register.
    pub access: Option<IrAccess>,

    /// Optional description.
    pub description: Option<String>,
}

/// Defines how software can interact with a register or field.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum IrAccess {
    /// Read-only. Writes are ignored or trigger faults.
    ReadOnly,
    /// Write-only. Reads return undefined values.
    WriteOnly,
    /// Read and Write allowed.
    ReadWrite,
    /// Writing 1 clears the bit (common for status flags).
    Write1ToClear,
    /// Reading the register clears the bit.
    ReadToClear,
    /// Write once (e.g., OTP or lock bits).
    WriteOnce,
    /// Read / Write Once.
    ReadWriteOnce,
    /// Access mode not specified or unknown.
    Unknown,
}

/// Represents an interrupt definition associated with a peripheral.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IrInterrupt {
    /// Name of the interrupt.
    pub name: String,

    /// Description.
    pub description: Option<String>,

    /// Vector index / IRQ number.
    pub value: u32,
}
