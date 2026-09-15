// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Co-simulation models in the browser engine.
//!
//! A manifest's `cosim_models:` get the same [`CosimSession`] `labwired test`
//! builds, and every `WasmSimulator` method that advances the machine goes
//! through [`WasmSimulator::advance_machine`], which hands the machine to the
//! session's lockstep advance. A manifest without models builds no session,
//! and the machine is advanced exactly as it was before this module existed.
//!
//! The browser has no process to spawn and no file to open, so the two model
//! shapes that need either are refused when the simulator is built rather than
//! skipped: a lab whose circuit silently never ran would still draw a flat
//! oscilloscope trace and look like a result.

use crate::WasmSimulator;
use labwired_config::{CosimAdapter, CosimModelConfig, SystemManifest};
use labwired_core::cosim::{CosimAdvanceError, CosimSession};
use labwired_core::{AdvanceReport, AdvanceRequest, SimulationError};
use std::path::Path;
use wasm_bindgen::prelude::*;

/// Why a browser build refuses `adapter: external_process`.
pub(crate) const EXTERNAL_PROCESS_IN_BROWSER: &str =
    "external_process co-simulation needs a native build; use adapter: analog in the browser";

/// Why a browser build refuses an analog model whose netlist is a file path.
pub(crate) const NETLIST_FILE_IN_BROWSER: &str =
    "the browser has no filesystem; put the netlist inline as netlist_text";

/// Refuse the model shapes a browser cannot run, naming the model.
///
/// Checked before any adapter is built: building an `external_process` adapter
/// spawns the process, which is exactly what must not be attempted here.
fn check_browser_models(models: &[CosimModelConfig]) -> Result<(), String> {
    for model in models {
        let refusal = match model.adapter {
            CosimAdapter::ExternalProcess => Some(EXTERNAL_PROCESS_IN_BROWSER),
            // `netlist_text` wins over `netlist` when both are given (see
            // `labwired_core::analog::adapter::netlist_source`), so only a
            // model that would actually read a file is refused.
            CosimAdapter::Analog
                if model.config.contains_key("netlist")
                    && !model.config.contains_key("netlist_text") =>
            {
                Some(NETLIST_FILE_IN_BROWSER)
            }
            CosimAdapter::Analog | CosimAdapter::Mock | CosimAdapter::Fmi => None,
        };
        if let Some(refusal) = refusal {
            return Err(format!("co-sim model '{}': {refusal}", model.id));
        }
    }
    Ok(())
}

/// Report a co-simulation problem that does not stop the run.
fn warn(message: &str) {
    #[cfg(target_arch = "wasm32")]
    crate::jit_browser::web_sys_console_warn(message);
    #[cfg(not(target_arch = "wasm32"))]
    tracing::warn!("{message}");
}

/// Why [`WasmSimulator::advance_machine`] failed.
pub(crate) enum AdvanceFailure {
    /// The machine failed. As with `Machine::advance`, it may have retired
    /// part of its batch first.
    Machine(SimulationError),
    /// A co-simulation model failed at a boundary the machine reached.
    Cosim(String),
}

impl AdvanceFailure {
    /// The message every stepping method throws to JS.
    pub(crate) fn into_js(self) -> JsValue {
        match self {
            Self::Machine(error) => JsValue::from_str(&format!("Step Error: {error}")),
            Self::Cosim(message) => JsValue::from_str(&message),
        }
    }
}

#[wasm_bindgen]
impl WasmSimulator {
    /// Set a co-simulation signal from outside the engine: a canvas touch pad
    /// sends `('ui.<partId>.pressed', 1)` on press and `0` on release.
    ///
    /// For an input its model calls a logic level (an analog switch control),
    /// 0 is false and anything else true; any other input takes the number as
    /// given. Every model reading `path` sees the value from its next step.
    ///
    /// Throws when the lab has no `cosim_models`, when no model input reads
    /// `path` (a typo would otherwise do nothing and report success), when
    /// `path` is a board path the machine owns, and for NaN or an infinity.
    #[wasm_bindgen]
    pub fn set_cosim_signal(&mut self, path: &str, value: f64) -> Result<(), JsValue> {
        self.set_cosim_signal_number(path, value)
            .map_err(|message| JsValue::from_str(&message))
    }
}

impl WasmSimulator {
    /// [`Self::set_cosim_signal`] without the JS error type, so a native test
    /// can read the refusal (a `JsValue` cannot be built off wasm).
    pub(crate) fn set_cosim_signal_number(&mut self, path: &str, value: f64) -> Result<(), String> {
        let session = self.cosim.as_mut().ok_or_else(|| {
            format!(
                "co-sim signal '{path}': this lab declares no cosim_models, so nothing reads it"
            )
        })?;
        session
            .set_signal_number(path, value)
            .map_err(|error| error.to_string())
    }

    /// Build the co-simulation session for `manifest` and publish its analog
    /// trace on the machine, exactly as `labwired test` does. A manifest with
    /// no `cosim_models:` leaves the simulator untouched.
    ///
    /// Every failure is an error, never a skip: a model the browser cannot
    /// run, a model that fails to build, and a routed path that does not
    /// resolve on this chip.
    pub(crate) fn attach_cosim(&mut self, manifest: &SystemManifest) -> Result<(), String> {
        if manifest.cosim_models.is_empty() {
            return Ok(());
        }
        check_browser_models(&manifest.cosim_models)?;
        let machine = self
            .machine
            .as_mut()
            .ok_or_else(|| "simulator has no machine".to_string())?;
        // No base directory: the checks above leave no model that reads a path.
        let Some(session) = CosimSession::new(&manifest.cosim_models, Path::new("."), &machine.bus)
            .map_err(|e| format!("co-sim: failed to start the declared models: {e}"))?
        else {
            return Ok(());
        };
        if !session.binding_errors().is_empty() {
            let errors: Vec<String> = session
                .binding_errors()
                .iter()
                .map(ToString::to_string)
                .collect();
            return Err(errors.join("; "));
        }
        if session.uses_fallback_clock() {
            warn(&format!(
                "co-sim: this bus reports no core clock; assuming {} Hz for the model time base",
                session.cpu_hz()
            ));
        }
        machine.attach_analog_trace(session.analog_trace_registry());
        self.cosim = Some(session);
        Ok(())
    }

    /// The one path every stepping method advances the machine through.
    ///
    /// Without a session this is `Machine::advance(request)`, unchanged. With
    /// one, the whole request budget is spent in lockstep with the models
    /// (`CosimSession::advance_budget`), so a `step_batch(n)` still runs `n`
    /// while every model boundary inside it is stepped. A routed path that
    /// fails at apply time is reported once and the run continues, as in
    /// `labwired test`; a model that fails ends the call with an error.
    pub(crate) fn advance_machine(
        &mut self,
        request: AdvanceRequest,
    ) -> Result<AdvanceReport, AdvanceFailure> {
        let machine = self
            .machine
            .as_mut()
            .expect("a constructed simulator always has a machine");
        let Some(session) = self.cosim.as_mut() else {
            return machine.advance(request).map_err(AdvanceFailure::Machine);
        };
        match session.advance_budget(machine, request) {
            Ok(advance) => {
                for error in &advance.new_routing_errors {
                    warn(&error.to_string());
                }
                Ok(advance.report)
            }
            Err(CosimAdvanceError::Machine(error)) => Err(AdvanceFailure::Machine(error)),
            Err(CosimAdvanceError::Model { error, .. }) => Err(AdvanceFailure::Cosim(format!(
                "Co-sim Error: model step failed at cycle {}: {error}",
                machine.total_cycles
            ))),
        }
    }
}
