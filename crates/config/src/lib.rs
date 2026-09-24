// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! `labwired-config`: the crate that owns every YAML-schema type (chips,
//! system/environment manifests, part packs and register descriptors, motor
//! and co-simulation models, CI test scripts, faults and display assertions)
//! plus their parsing and validation.
//!
//! This file used to be a single ~5.7k-line module. It is now a thin
//! aggregator: each domain lives in its own submodule below and is
//! re-exported here unchanged, so every `labwired_config::X` path used by
//! other crates keeps resolving exactly as before. This split is a pure
//! move — no logic, names, signatures or return conventions changed.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};

mod chip;
mod cosim;
mod display;
mod embedded;
mod fault;
mod logic;
mod manifest;
mod motor;
mod peripherals;
mod size;
mod test_script;

pub use chip::*;
pub use cosim::*;
pub use display::*;
pub use embedded::*;
pub use fault::*;
pub use logic::*;
pub use manifest::*;
pub use motor::*;
pub use peripherals::*;
pub use size::*;
pub use test_script::*;

pub mod expr;
pub mod rules;
pub mod uart;

pub use rules::{
    compile_rules, validate_rule_names, Action, BitFieldSpec, CompiledAction, CompiledRule, Event,
    FifoField, FifoFill, FifoOverflow, FifoRegisterField, FifoSpec, FifoWatermark, FrameSpec,
    PinEdge, RegBits, Rule, RuleCompileError, RuleNames,
};
pub use uart::{
    validate_uart, Template, TemplateError, TemplateFormat, TemplateWrap, UartFrames, UartMatch,
    UartResponse, UartSpec, UartUnsolicited,
};

#[cfg(test)]
#[path = "lib_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "modulshop_49213_tests.rs"]
mod modulshop_49213_tests;
