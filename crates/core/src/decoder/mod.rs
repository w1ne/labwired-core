// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

pub mod arm;
pub mod riscv;
pub mod xtensa;

pub use arm::decode_thumb_16;
pub use arm::decode_thumb_32;
pub use arm::Instruction as ArmInstruction;
pub use xtensa::decode_xtensa;
pub use xtensa::Instruction as XtensaInstruction;
