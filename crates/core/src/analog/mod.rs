// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! In-core analog engine: a small deterministic MNA transient solver for
//! linear elements and ideal switches, plus the [`crate::cosim::CosimAdapter`]
//! that steps it in lockstep with firmware.
//!
//! It exists because the browser cannot spawn a process, so the ngspice
//! `external_process` adapter has no browser counterpart. This engine compiles
//! into the same WASM the playground ships, runs natively too, and adds one
//! dependency (`libm`, for bit-identical transcendentals) to either. What it
//! does not do — subcircuits, model libraries, AC/DC sweeps, and every device
//! capacitance — it refuses by name and points at
//! `tools/cosim/labwired_ngspice.py`, which does them all.
//!
//! Nonlinear devices — the Shockley diode, the Ebers–Moll BJT and the level-1
//! MOSFET — are solved by Newton–Raphson iteration inside each time step. A
//! circuit with none of them takes exactly the code path it took before they
//! existed, down to the floating-point operation order.
//!
//! * [`netlist`] — the accepted SPICE subset and its errors.
//! * [`device`] — the device equations, their limiters and their constants.
//! * [`mna`] — modified nodal analysis with companion models, Newton
//!   iteration, backward Euler or trapezoidal, dense LU.
//! * [`adapter`] — manifest config, routed inputs, probes, step semantics.
//! * [`trace`] — the bounded sample ring the oscilloscope reads.

pub mod adapter;
pub mod device;
pub mod mna;
pub mod netlist;
pub mod trace;

pub use adapter::{AnalogConfig, AnalogCosimAdapter, Probe};
pub use mna::{Integration, Solver, MAX_STEP_CUTS, MAX_UNKNOWNS};
pub use netlist::{
    parse_netlist, parse_spice_value, AnalogError, Bjt, BjtModel, Capacitor, Cccs, Ccvs, Circuit,
    ComparatorModel, ComparatorOutput, CurrentSource, Diode, DiodeModel, Inductor, ModelCard,
    MosModel, Mosfet, NodeRef, OpAmpModel, Polarity, PowerSink, RegulatorKind, RegulatorModel,
    Resistor, Switch, Vccs, Vcvs, VoltageSource, VoltageSwitchControl, Waveform, COMPARATOR_PP_GAIN, MACRO_CLAMP_DIODE,
    MACRO_GAIN_RESISTANCE, POWER_SINK_VMIN, REGULATOR_LIMIT_DIODE, REGULATOR_LIMIT_GAIN,
};
pub use trace::{
    AnalogChannel, AnalogSample, AnalogSeries, AnalogSeriesChannel, AnalogTrace, AnalogTraceBatch,
    AnalogTraceHandle, AnalogTraceRegistry, DEFAULT_SERIES_POINTS, DEFAULT_TRACE_SAMPLES,
    SERIES_DEADBAND_AMPS, SERIES_DEADBAND_REL, SERIES_DEADBAND_VOLTS,
};
