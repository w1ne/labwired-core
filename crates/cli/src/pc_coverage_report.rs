// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! Firmware statement-coverage report. The implementation moved to
//! [`labwired_loader::coverage`] so the wasm build and the Python module
//! produce the same report as the CLI; this path re-exports it.

pub use labwired_loader::coverage::*;
