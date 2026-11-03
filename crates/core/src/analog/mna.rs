// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Modified nodal analysis with companion models — the numerical core.
//!
//! One `Solver` owns a parsed [`Circuit`] plus the state that carries across
//! steps (node voltages, branch currents, capacitor voltages and currents,
//! inductor currents and voltages). Each call to [`Solver::advance`] stamps one
//! linear system for one internal step `h` and solves it with dense LU.
//!
//! ## Unknowns
//!
//! `N` node voltages (ground is not an unknown) followed by `M` branch
//! currents: one per voltage source in netlist order, then one per inductor in
//! netlist order. `N + M` is capped at [`MAX_UNKNOWNS`]; the matrix is dense
//! and the browser is the target, so a circuit that wants more wants ngspice.
//!
//! ## Companion models
//!
//! Backward Euler (default):
//!
//! * capacitor → conductance `C/h` in parallel with a current source
//!   `(C/h)·v_prev`;
//! * inductor → an extra row `v_a − v_b − (L/h)·i = −(L/h)·i_prev`;
//!   coupled inductors add the corresponding mutual terms `−(M/h)·i_other`.
//!
//! Trapezoidal (`integration: trap`):
//!
//! * capacitor → conductance `2C/h` with source `(2C/h)·v_prev + i_prev`;
//! * inductor → `v_a − v_b − (2L/h)·i = −(2L/h)·i_prev − v_prev`;
//!   coupled inductors add `−(2M/h)·i_other_prev` and the mutual matrix term.
//!
//! Trapezoidal is second-order accurate and roughly two orders of magnitude
//! closer to the closed form at the same step; backward Euler is the default
//! because it cannot ring on a step input.
//!
//! ## Restart after a discontinuity
//!
//! Trapezoidal integrates with the element's derivative at the previous point
//! (the capacitor current, the inductor voltage). When a source or switch
//! changes, that stored derivative belongs to the circuit as it was BEFORE the
//! change, and trusting it costs a half-step lag that never goes away — on the
//! 1 ms RC fixture that is 0.29 % at one tau, no better than backward Euler.
//! So the first internal step after any change (and after the operating point)
//! is taken with backward Euler, which needs no stored derivative, and
//! trapezoidal resumes from the consistent state it leaves. This is what SPICE
//! does at a breakpoint, and it brings the same fixture to 0.002 %.
//!
//! ## Determinism
//!
//! `f64` throughout, `Vec` indices in the hot path, `BTreeMap` only for name
//! lookup at build time, no threads and no wall clock. The same netlist and the
//! same input sequence produce bit-identical voltages on every host.

use super::device;
use super::netlist::{
    AnalogError, BjtModel, Circuit, DiodeModel, MosChannelChargeModel, Mosfet, NodeRef,
    POWER_SINK_VMIN,
};

/// Largest MNA system the in-core engine solves: `N` nodes + `M` branch
/// currents. Dense LU is `O(n³)`; at 64 unknowns one step is a few microseconds
/// even in the browser, and a bigger circuit is a sign the user wants ngspice.
pub const MAX_UNKNOWNS: usize = 64;

/// Integration rule for the reactive companion models.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Integration {
    /// Backward Euler — first order, unconditionally stable, never rings.
    #[default]
    BackwardEuler,
    /// Trapezoidal — second order, far more accurate per step, can ring on a
    /// hard step input.
    Trapezoidal,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Stamp {
    /// t = 0: capacitors open, inductors short.
    OperatingPoint,
    /// One transient step of length `h` seconds under the given rule.
    Transient(f64, Integration),
}

/// Linearised total BJT charge for the B-E and B-C branches. Each branch may
/// depend on both junction voltages through the Gummel–Poon base-charge factor.
struct BjtChargeOp {
    q: [f64; 2],
    /// `[charge branch][Vbe or Vbc]`, in farads.
    dq: [[f64; 2]; 2],
}

fn bjt_charge_op(vbe: f64, vbc: f64, model: BjtModel) -> BjtChargeOp {
    let transit = device::bjt_transit_charge(vbe, vbc, model);
    let (qbe, cbe) =
        device::junction_depletion_charge(vbe, model.cje, model.vje, model.mje, model.fc);
    let (qbc, cbc) =
        device::junction_depletion_charge(vbc, model.cjc, model.vjc, model.mjc, model.fc);
    BjtChargeOp {
        q: [qbe + transit.qbe, qbc + transit.qbc],
        dq: [
            [cbe + transit.dqbe_dvbe, transit.dqbe_dvbc],
            [transit.dqbc_dvbe, cbc + transit.dqbc_dvbc],
        ],
    }
}

fn bjt_charge_active(model: BjtModel, branch: usize) -> bool {
    if branch == 0 {
        model.cje != 0.0 || model.tf != 0.0
    } else {
        model.cjc != 0.0 || model.tr != 0.0
    }
}

/// Level-1 Meyer gate-capacitance halves in netlist G-S, G-D, G-B order.
/// The model's effective source/drain order follows the channel direction, but
/// the returned history remains attached to the physical netlist terminals.
fn mos_channel_capacitance_halves(
    mosfet: &Mosfet,
    vgs: f64,
    vds: f64,
    vbd: f64,
    vbs: f64,
) -> [f64; 3] {
    if mosfet.model.channel_charge_model != MosChannelChargeModel::Meyer {
        return [0.0; 3];
    }
    let Some(tox) = mosfet.model.tox else {
        return [0.0; 3];
    };
    let sign = mosfet.model.polarity.sign();
    let reversed = vds < 0.0;
    let (vgs_eff, vds_eff, vbs_eff) = if reversed {
        (vgs - vds, -vds, vbd)
    } else {
        (vgs, vds, vbs)
    };
    let cox = device::OXIDE_PERMITTIVITY_F_PER_M * mosfet.w * mosfet.l / tox;
    let mut caps = device::mos_meyer_capacitance_halves(
        vgs_eff,
        vds_eff,
        vbs_eff,
        sign * mosfet.model.vto,
        mosfet.model.gamma,
        mosfet.model.phi,
        cox,
    );
    if reversed {
        caps.swap(0, 1);
    }
    caps
}

fn diode_model_op(voltage: f64, model: DiodeModel, breakdown: Option<f64>) -> device::DiodeOp {
    let mut op = match breakdown {
        None => device::diode_op(voltage, model.is, model.n),
        Some(xbv) => device::diode_op_breakdown(
            voltage,
            model.is,
            model.n,
            xbv,
            model.nbv.unwrap_or(model.n),
        ),
    };
    let recombination =
        device::diode_recombination_op(voltage, model.isr, model.nr, model.n, model.vj, model.m);
    op.id += recombination.id;
    op.gd += recombination.gd;
    op
}

fn diode_charge_op(voltage: f64, model: DiodeModel, op: device::DiodeOp) -> (f64, f64) {
    let (q_depletion, c_depletion) =
        device::diode_depletion_charge(voltage, model.cjo, model.vj, model.m, model.fc);
    let (q_diffusion, c_diffusion) = if model.vp > 0.0 && model.tt > 0.0 {
        (0.0, 0.0)
    } else {
        device::diode_diffusion_charge(voltage, op, model.tt)
    };
    (q_depletion + q_diffusion, c_depletion + c_diffusion)
}

#[derive(Debug, Clone, Copy)]
struct DiodeRecoveryStep {
    state: f64,
    rate: f64,
    current: f64,
    conductance: f64,
}

/// One implicit step of ngspice's `TT`/`VP` soft reverse-recovery equation.
/// `state` is the normalized charge variable `q` in `q + TT·VP·dq/dt =
/// -TT·Id`; the diode current receives `-(1−VP)·dq/dt`. At `VP=0` this
/// reduces to the ordinary `TT·dId/dt` diffusion current, so only positive
/// `VP` uses this path.
fn diode_recovery_step(
    model: DiodeModel,
    op: device::DiodeOp,
    previous_state: f64,
    previous_rate: f64,
    h: f64,
    rule: Integration,
) -> DiodeRecoveryStep {
    let tau = model.tt * model.vp;
    let (state, rate, drate_dv) = match rule {
        Integration::BackwardEuler => {
            let ratio = tau / h;
            let denominator = 1.0 + ratio;
            let state = (-model.tt * op.id + ratio * previous_state) / denominator;
            let dstate_dv = -model.tt * op.gd / denominator;
            (state, (state - previous_state) / h, dstate_dv / h)
        }
        Integration::Trapezoidal => {
            let ratio = tau / h;
            let denominator = 1.0 + 2.0 * ratio;
            let state = (-model.tt * op.id + 2.0 * ratio * previous_state + tau * previous_rate)
                / denominator;
            let dstate_dv = -model.tt * op.gd / denominator;
            (
                state,
                2.0 * (state - previous_state) / h - previous_rate,
                2.0 * dstate_dv / h,
            )
        }
    };
    let current = -(1.0 - model.vp) * rate;
    let conductance = -(1.0 - model.vp) * drate_dv;
    DiodeRecoveryStep {
        state,
        rate,
        current,
        conductance,
    }
}

fn diode_charge_active(model: DiodeModel) -> bool {
    model.cjo != 0.0 || model.tt != 0.0
}

/// One circuit and the state that carries between steps.
#[derive(Debug, Clone)]
pub struct Solver {
    circuit: Circuit,
    integration: Integration,
    nodes: usize,
    branches: usize,

    voltage_source_values: Vec<f64>,
    current_source_values: Vec<f64>,
    switch_closed: Vec<bool>,

    node_v: Vec<f64>,
    branch_i: Vec<f64>,
    cap_v: Vec<f64>,
    cap_i: Vec<f64>,
    ind_i: Vec<f64>,
    ind_v: Vec<f64>,

    /// Junction voltage of each diode, the point its stamp is linearised
    /// about. Carried across steps: the previous step's answer is the best
    /// first guess there is.
    diode_vj: Vec<f64>,
    /// Depletion charge and current history for each diode, oriented anode to
    /// cathode. Charge is sufficient for BE; trap also uses the current.
    diode_q: Vec<f64>,
    diode_i: Vec<f64>,
    /// Ngspice's normalized soft-recovery charge state and its derivative.
    /// These are active only when `VP > 0` and `TT > 0`.
    diode_recovery_state: Vec<f64>,
    diode_recovery_rate: Vec<f64>,
    /// Each BJT's base-emitter voltage in device coordinates.
    bjt_vbe: Vec<f64>,
    /// Each BJT's base-collector voltage in device coordinates.
    bjt_vbc: Vec<f64>,
    /// Total depletion plus transit charge and current history for each BJT,
    /// ordered B-E then B-C in normalized device coordinates.
    bjt_q: Vec<[f64; 2]>,
    bjt_i: Vec<[f64; 2]>,
    /// Each MOSFET's gate-source voltage in device coordinates, as the netlist
    /// names the terminals — not swapped for reverse mode.
    mos_vgs: Vec<f64>,
    /// Each MOSFET's drain-source voltage in device coordinates, as the
    /// netlist names the terminals. Negative means the device is reversed.
    mos_vds: Vec<f64>,
    /// Each MOSFET's normalised bulk-drain junction voltage.
    mos_vbd: Vec<f64>,
    /// Each MOSFET's normalised bulk-source junction voltage.
    mos_vbs: Vec<f64>,
    /// Depletion charge and current history for each MOSFET bulk junction,
    /// ordered bulk-drain then bulk-source in normalized device coordinates.
    mos_bulk_q: Vec<[f64; 2]>,
    mos_bulk_i: Vec<[f64; 2]>,
    /// Linear overlap charge and current history for each MOSFET, ordered
    /// gate-source, gate-drain, then gate-bulk in circuit coordinates.
    mos_overlap_q: Vec<[f64; 3]>,
    mos_overlap_i: Vec<[f64; 3]>,
    /// Previous physical G-S, G-D and G-B voltages for intrinsic channel
    /// charge, in circuit coordinates.
    mos_channel_v: Vec<[f64; 3]>,
    /// Integrated intrinsic channel charge and current, in the same order.
    mos_channel_q: Vec<[f64; 3]>,
    mos_channel_i: Vec<[f64; 3]>,
    /// Previous endpoint Meyer capacitance halves, in the same order.
    mos_channel_cap_half: Vec<[f64; 3]>,
    /// Intrinsic Ward–Dutton terminal charges and currents, G-D-S-B order.
    mos_conserving_q: Vec<[f64; 4]>,
    mos_conserving_i: Vec<[f64; 4]>,
    /// Each JFET's normalised gate-source and gate-drain junction voltages.
    jfet_vgs: Vec<f64>,
    jfet_vgd: Vec<f64>,
    /// Each JFET's normalised drain-source voltage. Negative means reversed.
    jfet_vds: Vec<f64>,
    /// Depletion charge and current history for each JFET, ordered G-S then
    /// G-D and oriented in normalized device coordinates.
    jfet_q: Vec<[f64; 2]>,
    jfet_i: Vec<[f64; 2]>,
    /// Each power sink's linearisation point: `(v(p) − v(n), v(op) − v(on),
    /// i(control))`.
    sink_point: Vec<[f64; 3]>,
    /// The linearisation points at the start of the step in progress, for a
    /// retry with a cut step; see [`Solver::advance`].
    saved_diode_vj: Vec<f64>,
    saved_bjt_vbe: Vec<f64>,
    saved_bjt_vbc: Vec<f64>,
    saved_mos_vgs: Vec<f64>,
    saved_mos_vds: Vec<f64>,
    saved_mos_vbd: Vec<f64>,
    saved_mos_vbs: Vec<f64>,
    saved_jfet_vgs: Vec<f64>,
    saved_jfet_vgd: Vec<f64>,
    saved_jfet_vds: Vec<f64>,
    saved_sink_point: Vec<[f64; 3]>,
    /// Conductance from every node to ground, siemens; nonzero only inside
    /// [`Solver::gmin_stepping`].
    node_shunt: f64,
    /// True when the last [`Solver::relinearise`] damped a step, so the next
    /// solution is not a Newton step and must not be accepted as converged.
    limiter_clamped: bool,
    /// True when the circuit holds a device whose stamp depends on the
    /// solution. False takes the pre-Newton code path, untouched.
    nonlinear: bool,
    /// True when a source carries a transient function, so its value has to be
    /// re-evaluated against the clock each step.
    has_waveforms: bool,
    /// Simulated time at the end of the last completed step, seconds.
    time: f64,
    /// Internal steps completed since the solver was built.
    step_index: u64,

    matrix: Vec<f64>,
    rhs: Vec<f64>,
    solution: Vec<f64>,
    /// The previous Newton iterate, for the convergence test.
    previous: Vec<f64>,
    // Elimination multipliers are kept in the order they were applied, not
    // permuted as a conventional L matrix. Replaying them preserves the old
    // solver's RHS floating-point operation order exactly.
    factors: Vec<f64>,
    pivots: Vec<usize>,
    cached_stamp: Option<Stamp>,
    settled: bool,

    /// The next internal step must not trust the stored derivatives: a source
    /// or switch changed, or the state was just set by the operating point.
    restart: bool,
}

impl Solver {
    /// Build a solver over `circuit` and solve its operating point.
    ///
    /// The operating point uses the netlist's own DC values with capacitors
    /// open and inductors shorted, then applies `.ic` / `ic=` overrides —
    /// exactly what the ngspice wrapper gets from a transient that pauses just
    /// after t = 0 before any routed input is applied. Switches start open
    /// (`roff`), because their control is a routed input and no input has
    /// arrived yet.
    pub fn new(circuit: Circuit, integration: Integration) -> Result<Self, AnalogError> {
        let unknowns = circuit.unknowns();
        if unknowns > MAX_UNKNOWNS {
            return Err(AnalogError::TooLarge {
                unknowns,
                max: MAX_UNKNOWNS,
            });
        }
        if unknowns == 0 {
            return Err(AnalogError::Config(
                "netlist declares no nodes to solve for".to_string(),
            ));
        }

        let nodes = circuit.node_count();
        let branches = circuit.branch_count();
        let voltage_source_values = circuit.voltage_sources.iter().map(|s| s.dc).collect();
        let current_source_values = circuit.current_sources.iter().map(|s| s.dc).collect();
        let switch_closed = vec![false; circuit.switches.len()];
        let capacitors = circuit.capacitors.len();
        let inductors = circuit.inductors.len();
        let nonlinear = circuit.is_nonlinear();
        let has_waveforms = circuit.has_waveforms();
        let diodes = circuit.diodes.len();
        let bjts = circuit.bjts.len();
        let mosfets = circuit.mosfets.len();
        let jfets = circuit.jfets.len();
        let sinks = circuit.power_sinks.len();

        let mut solver = Self {
            circuit,
            integration,
            nodes,
            branches,
            voltage_source_values,
            current_source_values,
            switch_closed,
            node_v: vec![0.0; nodes],
            branch_i: vec![0.0; branches],
            cap_v: vec![0.0; capacitors],
            cap_i: vec![0.0; capacitors],
            ind_i: vec![0.0; inductors],
            ind_v: vec![0.0; inductors],
            // Every junction starts at zero volts, which is the state of a
            // circuit that has not been powered: the operating-point Newton
            // walks up from there.
            diode_vj: vec![0.0; diodes],
            diode_q: vec![0.0; diodes],
            diode_i: vec![0.0; diodes],
            diode_recovery_state: vec![0.0; diodes],
            diode_recovery_rate: vec![0.0; diodes],
            bjt_vbe: vec![0.0; bjts],
            bjt_vbc: vec![0.0; bjts],
            bjt_q: vec![[0.0; 2]; bjts],
            bjt_i: vec![[0.0; 2]; bjts],
            mos_vgs: vec![0.0; mosfets],
            mos_vds: vec![0.0; mosfets],
            mos_vbd: vec![0.0; mosfets],
            mos_vbs: vec![0.0; mosfets],
            mos_bulk_q: vec![[0.0; 2]; mosfets],
            mos_bulk_i: vec![[0.0; 2]; mosfets],
            mos_overlap_q: vec![[0.0; 3]; mosfets],
            mos_overlap_i: vec![[0.0; 3]; mosfets],
            mos_channel_v: vec![[0.0; 3]; mosfets],
            mos_channel_q: vec![[0.0; 3]; mosfets],
            mos_channel_i: vec![[0.0; 3]; mosfets],
            mos_channel_cap_half: vec![[0.0; 3]; mosfets],
            mos_conserving_q: vec![[0.0; 4]; mosfets],
            mos_conserving_i: vec![[0.0; 4]; mosfets],
            jfet_vgs: vec![0.0; jfets],
            jfet_vgd: vec![0.0; jfets],
            jfet_vds: vec![0.0; jfets],
            jfet_q: vec![[0.0; 2]; jfets],
            jfet_i: vec![[0.0; 2]; jfets],
            sink_point: vec![[0.0; 3]; sinks],
            saved_diode_vj: vec![0.0; diodes],
            saved_bjt_vbe: vec![0.0; bjts],
            saved_bjt_vbc: vec![0.0; bjts],
            saved_mos_vgs: vec![0.0; mosfets],
            saved_mos_vds: vec![0.0; mosfets],
            saved_mos_vbd: vec![0.0; mosfets],
            saved_mos_vbs: vec![0.0; mosfets],
            saved_jfet_vgs: vec![0.0; jfets],
            saved_jfet_vgd: vec![0.0; jfets],
            saved_jfet_vds: vec![0.0; jfets],
            saved_sink_point: vec![[0.0; 3]; sinks],
            node_shunt: 0.0,
            limiter_clamped: false,
            nonlinear,
            has_waveforms,
            time: 0.0,
            step_index: 0,
            matrix: vec![0.0; unknowns * unknowns],
            rhs: vec![0.0; unknowns],
            solution: vec![0.0; unknowns],
            previous: vec![0.0; unknowns],
            factors: vec![0.0; unknowns * unknowns],
            pivots: vec![0; unknowns],
            cached_stamp: None,
            settled: false,
            restart: true,
        };
        solver.solve_operating_point()?;
        Ok(solver)
    }

    /// The parsed circuit this solver integrates.
    pub fn circuit(&self) -> &Circuit {
        &self.circuit
    }

    /// Integration rule in force.
    pub fn integration(&self) -> Integration {
        self.integration
    }

    /// Index of the named voltage source, for the hot path.
    pub fn voltage_source_index(&self, name: &str) -> Option<usize> {
        let key = name.trim().to_ascii_lowercase();
        self.circuit
            .voltage_sources
            .iter()
            .position(|s| s.name.to_ascii_lowercase() == key)
    }

    /// Index of the named current source, for the hot path.
    pub fn current_source_index(&self, name: &str) -> Option<usize> {
        let key = name.trim().to_ascii_lowercase();
        self.circuit
            .current_sources
            .iter()
            .position(|s| s.name.to_ascii_lowercase() == key)
    }

    /// Index of a named resistor that has an NTC Beta model.
    pub fn thermistor_index(&self, name: &str) -> Option<usize> {
        let key = name.trim().to_ascii_lowercase();
        self.circuit.resistors.iter().position(|resistor| {
            resistor.ntc.is_some() && resistor.name.to_ascii_lowercase() == key
        })
    }

    /// Set an NTC's external temperature input in Celsius. A changed value
    /// invalidates the linear stamp so its resistance is rebuilt next step.
    /// Returns false for a non-NTC element, an invalid temperature or a model
    /// whose resulting resistance is not finite and positive.
    pub fn set_thermistor_temperature(&mut self, index: usize, celsius: f64) -> bool {
        if !celsius.is_finite() || celsius <= -273.15 {
            return false;
        }
        let Some(resistor) = self.circuit.resistors.get_mut(index) else {
            return false;
        };
        let Some(mut ntc) = resistor.ntc else {
            return false;
        };
        ntc.temperature_celsius = celsius;
        let resistance = ntc.resistance_ohms(resistor.ohms);
        if !resistance.is_finite() || resistance <= 0.0 {
            return false;
        }
        if resistor.ntc != Some(ntc) {
            resistor.ntc = Some(ntc);
            self.cached_stamp = None;
            self.restart = true;
            self.settled = false;
        }
        true
    }

    /// Indices of every switch driven by the routed boolean input `ctrl`.
    /// Several switches may share one control (a break-before-make pair).
    pub fn switches_controlled_by(&self, ctrl: &str) -> Vec<usize> {
        let key = ctrl.trim().to_ascii_lowercase();
        if key.is_empty() {
            return Vec::new();
        }
        self.circuit
            .switches
            .iter()
            .enumerate()
            .filter(|(_, s)| s.voltage.is_none() && s.ctrl.to_ascii_lowercase() == key)
            .map(|(index, _)| index)
            .collect()
    }

    /// Index of the switch named `name` (the `S...` element, not its control).
    pub fn switch_index(&self, name: &str) -> Option<usize> {
        let key = name.trim().to_ascii_lowercase();
        self.circuit
            .switches
            .iter()
            .position(|s| s.name.to_ascii_lowercase() == key)
    }

    /// Drive a voltage source. Takes effect on the next [`Self::advance`].
    ///
    /// Setting the value it already has is not a change: a GPIO held high is
    /// re-sent every co-simulation step, and treating that as an edge would
    /// restart trapezoidal integration every step.
    pub fn set_voltage_source(&mut self, index: usize, volts: f64) {
        if self.voltage_source_values[index] != volts {
            self.voltage_source_values[index] = volts;
            self.restart = true;
            self.settled = false;
        }
    }

    /// Drive a current source. Takes effect on the next [`Self::advance`].
    pub fn set_current_source(&mut self, index: usize, amps: f64) {
        if self.current_source_values[index] != amps {
            self.current_source_values[index] = amps;
            self.restart = true;
            self.settled = false;
        }
    }

    /// Open or close a switch. Takes effect on the next [`Self::advance`].
    ///
    /// Voltage-controlled switches ignore this: their state is refreshed from
    /// the sensed nodes at the start of every step.
    pub fn set_switch(&mut self, index: usize, closed: bool) {
        if self.circuit.switches[index].voltage.is_some() {
            return;
        }
        if self.switch_closed[index] != closed {
            self.switch_closed[index] = closed;
            self.cached_stamp = None;
            self.restart = true;
            self.settled = false;
        }
    }

    /// Refresh every voltage-controlled switch from the latest node voltages.
    ///
    /// All voltage-controlled contacts are evaluated against the same `|V|` in
    /// one pass, so an NO/NC pair transfers atomically: the NC opens and the
    /// NO closes in the same update (no break-before-make interval). Real Form
    /// C relays are break-before-make; modelling a short open-transfer delay is
    /// deferred.
    ///
    /// Returns true when any contact changed, so the caller can clear the
    /// linear stamp cache / restart integration.
    fn update_voltage_switches(&mut self) -> bool {
        let mut changed = false;
        for index in 0..self.circuit.switches.len() {
            let Some(control) = self.circuit.switches[index].voltage.clone() else {
                continue;
            };
            let v = node_diff(&self.node_v, control.pos, control.neg).abs();
            let on = control.vt;
            let off = (control.vt - control.vh).max(0.0);
            let closed_now = self.switch_closed[index];
            let desired = if control.invert {
                // NC: stays closed while |V| < vt; re-closes from open only at
                // |V| <= vt − vh.
                if closed_now {
                    v < on
                } else {
                    v <= off
                }
            } else if closed_now {
                // NO: stay closed until |V| falls through dropout.
                v > off
            } else {
                v >= on
            };
            if desired != closed_now {
                self.switch_closed[index] = desired;
                changed = true;
            }
        }
        if changed {
            self.cached_stamp = None;
            self.restart = true;
            self.settled = false;
        }
        changed
    }

    /// Voltage at a node (`None` is ground, which is always 0 V).
    pub fn node_voltage(&self, node: NodeRef) -> f64 {
        match node {
            Some(index) => self.node_v[index],
            None => 0.0,
        }
    }

    /// Current through branch `index` (voltage sources first, then inductors),
    /// flowing from the element's first terminal to its second.
    pub fn branch_current(&self, index: usize) -> f64 {
        self.branch_i[index]
    }

    /// Voltage across capacitor `index`, as carried into the next step.
    pub fn capacitor_voltage(&self, index: usize) -> f64 {
        self.cap_v[index]
    }

    /// Current through inductor `index`, as carried into the next step.
    pub fn inductor_current(&self, index: usize) -> f64 {
        self.ind_i[index]
    }

    fn dim(&self) -> usize {
        self.nodes + self.branches
    }

    /// Solve the DC operating point and reset the reactive state to it.
    pub fn solve_operating_point(&mut self) -> Result<(), AnalogError> {
        // Voltage-controlled contacts depend on the solved node voltages, so
        // the OP may need a few passes until the contacts stop flipping.
        for _ in 0..8 {
            self.cached_stamp = None;
            self.settled = false;
            if self.nonlinear {
                self.save_linearisation();
                if let Err(error) = self.newton(Stamp::OperatingPoint, None, self.time) {
                    self.restore_linearisation();
                    self.gmin_stepping().map_err(|_| error)?;
                }
            } else {
                self.build(Stamp::OperatingPoint);
                self.factorize()?;
                self.solve();
            }
            self.node_v.copy_from_slice(&self.solution[..self.nodes]);
            self.branch_i.copy_from_slice(&self.solution[self.nodes..]);
            if !self.update_voltage_switches() {
                break;
            }
        }
        for index in 0..self.circuit.capacitors.len() {
            let capacitor = &self.circuit.capacitors[index];
            self.cap_v[index] = node_diff(&self.node_v, capacitor.a, capacitor.b);
            self.cap_i[index] = 0.0;
        }
        for index in 0..self.circuit.inductors.len() {
            self.ind_i[index] = self.branch_i[self.circuit.voltage_sources.len() + index];
            self.ind_v[index] = 0.0;
        }
        self.apply_initial_conditions();
        self.restart = true;
        Ok(())
    }

    /// `.ic V(node)=v` and element `ic=` override the operating point.
    ///
    /// The operating point is solved first and then overridden, rather than
    /// solved subject to the constraints: the two agree for every netlist where
    /// `.ic` is used for what it is for (seeding a reactive element), and the
    /// override order — node `.ic` first, then per-element `ic=` — is stated
    /// here so a netlist that sets both is not ambiguous.
    fn apply_initial_conditions(&mut self) {
        for (node, value) in &self.circuit.node_ic {
            self.node_v[*node] = *value;
        }
        if !self.circuit.node_ic.is_empty() {
            for index in 0..self.circuit.capacitors.len() {
                let capacitor = &self.circuit.capacitors[index];
                self.cap_v[index] = node_diff(&self.node_v, capacitor.a, capacitor.b);
            }
        }
        for index in 0..self.circuit.capacitors.len() {
            if let Some(ic) = self.circuit.capacitors[index].ic {
                self.cap_v[index] = ic;
            }
        }
        for index in 0..self.circuit.inductors.len() {
            if let Some(ic) = self.circuit.inductors[index].ic {
                self.ind_i[index] = ic;
                self.branch_i[self.circuit.voltage_sources.len() + index] = ic;
            }
        }
        for index in 0..self.circuit.diodes.len() {
            let diode = &self.circuit.diodes[index];
            if diode_charge_active(diode.model) {
                self.diode_vj[index] = node_diff(&self.node_v, diode.junction_anode, diode.cathode);
                let op = diode_model_op(self.diode_vj[index], diode.model, diode.breakdown);
                self.diode_q[index] = diode_charge_op(self.diode_vj[index], diode.model, op).0;
                self.diode_i[index] = 0.0;
                if diode.model.vp > 0.0 && diode.model.tt > 0.0 {
                    self.diode_recovery_state[index] = -diode.model.tt * op.id;
                    self.diode_recovery_rate[index] = 0.0;
                }
            }
        }
        for index in 0..self.circuit.bjts.len() {
            let bjt = &self.circuit.bjts[index];
            let sign = bjt.model.polarity.sign();
            let vbe = sign * node_diff(&self.node_v, bjt.junction_b, bjt.junction_e);
            let vbc = sign * node_diff(&self.node_v, bjt.junction_b, bjt.junction_c);
            let model = bjt.model;
            self.bjt_q[index] = bjt_charge_op(vbe, vbc, model).q;
            self.bjt_i[index] = [0.0; 2];
        }
        for index in 0..self.circuit.mosfets.len() {
            let mosfet = &self.circuit.mosfets[index];
            let sign = mosfet.model.polarity.sign();
            let voltages = [
                sign * node_diff(&self.node_v, mosfet.bulk, mosfet.junction_d),
                sign * node_diff(&self.node_v, mosfet.bulk, mosfet.junction_s),
            ];
            self.mos_bulk_q[index] = core::array::from_fn(|junction| {
                mosfet
                    .bulk_junction_charge(voltages[junction], junction == 0)
                    .0
            });
            self.mos_bulk_i[index] = [0.0; 2];

            let voltages = [
                node_diff(&self.node_v, mosfet.g, mosfet.junction_s),
                node_diff(&self.node_v, mosfet.g, mosfet.junction_d),
                node_diff(&self.node_v, mosfet.g, mosfet.bulk),
            ];
            let capacitances = mosfet.overlap_capacitances();
            self.mos_overlap_q[index] =
                core::array::from_fn(|junction| capacitances[junction] * voltages[junction]);
            self.mos_overlap_i[index] = [0.0; 3];

            let sign = mosfet.model.polarity.sign();
            let channel_caps = mos_channel_capacitance_halves(
                mosfet,
                sign * voltages[0],
                sign * node_diff(&self.node_v, mosfet.junction_d, mosfet.junction_s),
                sign * node_diff(&self.node_v, mosfet.bulk, mosfet.junction_d),
                sign * node_diff(&self.node_v, mosfet.bulk, mosfet.junction_s),
            );
            self.mos_channel_v[index] = voltages;
            self.mos_channel_q[index] =
                core::array::from_fn(|junction| 2.0 * channel_caps[junction] * voltages[junction]);
            self.mos_channel_i[index] = [0.0; 3];
            self.mos_channel_cap_half[index] = channel_caps;

            if mosfet.model.channel_charge_model == MosChannelChargeModel::WardDutton {
                let vgs = sign * node_diff(&self.node_v, mosfet.g, mosfet.junction_s);
                let vds = sign * node_diff(&self.node_v, mosfet.junction_d, mosfet.junction_s);
                let vbd = sign * node_diff(&self.node_v, mosfet.bulk, mosfet.junction_d);
                let vbs = sign * node_diff(&self.node_v, mosfet.bulk, mosfet.junction_s);
                self.mos_conserving_q[index] =
                    device::mos_ward_dutton_channel_charge(mosfet, vgs, vds, vbd, vbs).q;
                self.mos_conserving_i[index] = [0.0; 4];
            }
        }
        for index in 0..self.circuit.jfets.len() {
            let jfet = &self.circuit.jfets[index];
            let sign = jfet.model.polarity.sign();
            let model = jfet.model;
            let vgs = sign * node_diff(&self.node_v, jfet.g, jfet.junction_s);
            let vgd = sign * node_diff(&self.node_v, jfet.g, jfet.junction_d);
            self.jfet_q[index] = [
                device::junction_depletion_charge(vgs, model.cgs, model.pb, 0.5, model.fc).0,
                device::junction_depletion_charge(vgd, model.cgd, model.pb, 0.5, model.fc).0,
            ];
            self.jfet_i[index] = [0.0; 2];
        }
    }

    /// Integrate one internal step of `h` seconds.
    ///
    /// A nonlinear step whose Newton iteration does not converge is retried as
    /// two half steps, recursively, down to `h / 2^`[`MAX_STEP_CUTS`] — what
    /// SPICE's timestep control does at a hard corner, and what a regenerative
    /// edge (a comparator with positive feedback, a Schmitt trigger) needs from
    /// a fixed-step engine. A step that converges first time takes exactly the
    /// path it took before this existed; only a step that used to be reported
    /// as [`AnalogError::NoConvergence`] is subdivided. The cut steps count as
    /// ONE step in [`Self::step_index`], and the error, if even the smallest
    /// cut fails, names the step the caller asked for.
    pub fn advance(&mut self, h: f64) -> Result<(), AnalogError> {
        if h <= 0.0 || !h.is_finite() {
            return Err(AnalogError::Config(format!(
                "internal step must be a positive finite number of seconds, got {h}"
            )));
        }
        if !self.nonlinear {
            return self.step(h);
        }
        let outer_index = self.step_index;
        self.save_linearisation();
        match self.step(h) {
            Err(AnalogError::NoConvergence { .. }) => {
                self.restore_linearisation();
                let result = self.step_cut(h, 1);
                self.step_index = outer_index + u64::from(result.is_ok());
                result.map_err(|error| match error {
                    AnalogError::NoConvergence {
                        iterations,
                        unknown,
                        delta,
                        ..
                    } => AnalogError::NoConvergence {
                        step: Some(outer_index),
                        time: self.time + h,
                        iterations,
                        unknown,
                        delta,
                    },
                    other => other,
                })
            }
            other => other,
        }
    }

    /// Two half steps of `h`, each cut again on failure, `depth` cuts deep.
    fn step_cut(&mut self, h: f64, depth: u32) -> Result<(), AnalogError> {
        let half = 0.5 * h;
        for _ in 0..2 {
            self.save_linearisation();
            match self.step(half) {
                Err(AnalogError::NoConvergence { .. }) if depth < MAX_STEP_CUTS => {
                    self.restore_linearisation();
                    self.step_cut(half, depth + 1)?;
                }
                other => other?,
            }
        }
        Ok(())
    }

    /// Keep each device's linearisation point, so a step that fails to
    /// converge can be retried from where it started rather than from wherever
    /// the failed iteration left it.
    fn save_linearisation(&mut self) {
        self.saved_diode_vj.copy_from_slice(&self.diode_vj);
        self.saved_bjt_vbe.copy_from_slice(&self.bjt_vbe);
        self.saved_bjt_vbc.copy_from_slice(&self.bjt_vbc);
        self.saved_mos_vgs.copy_from_slice(&self.mos_vgs);
        self.saved_mos_vds.copy_from_slice(&self.mos_vds);
        self.saved_mos_vbd.copy_from_slice(&self.mos_vbd);
        self.saved_mos_vbs.copy_from_slice(&self.mos_vbs);
        self.saved_jfet_vgs.copy_from_slice(&self.jfet_vgs);
        self.saved_jfet_vgd.copy_from_slice(&self.jfet_vgd);
        self.saved_jfet_vds.copy_from_slice(&self.jfet_vds);
        self.saved_sink_point.copy_from_slice(&self.sink_point);
    }

    fn restore_linearisation(&mut self) {
        self.diode_vj.copy_from_slice(&self.saved_diode_vj);
        self.bjt_vbe.copy_from_slice(&self.saved_bjt_vbe);
        self.bjt_vbc.copy_from_slice(&self.saved_bjt_vbc);
        self.mos_vgs.copy_from_slice(&self.saved_mos_vgs);
        self.mos_vds.copy_from_slice(&self.saved_mos_vds);
        self.mos_vbd.copy_from_slice(&self.saved_mos_vbd);
        self.mos_vbs.copy_from_slice(&self.saved_mos_vbs);
        self.jfet_vgs.copy_from_slice(&self.saved_jfet_vgs);
        self.jfet_vgd.copy_from_slice(&self.saved_jfet_vgd);
        self.jfet_vds.copy_from_slice(&self.saved_jfet_vds);
        self.sink_point.copy_from_slice(&self.saved_sink_point);
    }

    /// One step of exactly `h`, no retry.
    fn step(&mut self, h: f64) -> Result<(), AnalogError> {
        let time = self.time + h;
        self.update_voltage_switches();
        self.apply_waveforms(time);
        // Backward Euler for the first step after a discontinuity; see the
        // module docs. A no-op distinction when the rule already is BE.
        let rule = if self.restart {
            Integration::BackwardEuler
        } else {
            self.integration
        };
        let stamp = Stamp::Transient(h, rule);
        if self.nonlinear {
            // Every device stamp moves with the solution, so there is nothing
            // to cache between iterations, let alone between steps.
            self.newton(stamp, Some(self.step_index), time)?;
        } else if self.cached_stamp == Some(stamp) {
            if self.settled && !self.restart {
                self.time = time;
                self.step_index += 1;
                return Ok(());
            }
            self.build_rhs(stamp);
            self.solve();
        } else {
            self.cached_stamp = None;
            self.build(stamp);
            self.factorize()?;
            self.cached_stamp = Some(stamp);
            self.solve();
        }
        self.restart = false;
        self.settled = true;

        for index in 0..self.circuit.capacitors.len() {
            let capacitor = &self.circuit.capacitors[index];
            let v_new = node_diff(&self.solution[..self.nodes], capacitor.a, capacitor.b);
            let geq = match rule {
                Integration::BackwardEuler => capacitor.farads / h,
                Integration::Trapezoidal => 2.0 * capacitor.farads / h,
            };
            let i_new = match rule {
                Integration::BackwardEuler => geq * (v_new - self.cap_v[index]),
                Integration::Trapezoidal => geq * (v_new - self.cap_v[index]) - self.cap_i[index],
            };
            self.settled &= self.cap_v[index].to_bits() == v_new.to_bits()
                && self.cap_i[index].to_bits() == i_new.to_bits();
            self.cap_v[index] = v_new;
            self.cap_i[index] = i_new;
        }
        for index in 0..self.circuit.diodes.len() {
            let diode = &self.circuit.diodes[index];
            if !diode_charge_active(diode.model) {
                continue;
            }
            let v_new = node_diff(
                &self.solution[..self.nodes],
                diode.junction_anode,
                diode.cathode,
            );
            let op = diode_model_op(v_new, diode.model, diode.breakdown);
            let (q_new, _) = diode_charge_op(v_new, diode.model, op);
            let i_new = match rule {
                Integration::BackwardEuler => (q_new - self.diode_q[index]) / h,
                Integration::Trapezoidal => {
                    2.0 * (q_new - self.diode_q[index]) / h - self.diode_i[index]
                }
            };
            self.settled &= self.diode_q[index].to_bits() == q_new.to_bits()
                && self.diode_i[index].to_bits() == i_new.to_bits();
            self.diode_q[index] = q_new;
            self.diode_i[index] = i_new;
        }
        for index in 0..self.circuit.diodes.len() {
            let diode = &self.circuit.diodes[index];
            if diode.model.vp <= 0.0 || diode.model.tt <= 0.0 {
                continue;
            }
            let v_new = node_diff(
                &self.solution[..self.nodes],
                diode.junction_anode,
                diode.cathode,
            );
            let op = diode_model_op(v_new, diode.model, diode.breakdown);
            let recovery = diode_recovery_step(
                diode.model,
                op,
                self.diode_recovery_state[index],
                self.diode_recovery_rate[index],
                h,
                rule,
            );
            self.settled &= self.diode_recovery_state[index].to_bits() == recovery.state.to_bits()
                && self.diode_recovery_rate[index].to_bits() == recovery.rate.to_bits();
            self.diode_recovery_state[index] = recovery.state;
            self.diode_recovery_rate[index] = recovery.rate;
        }
        for index in 0..self.circuit.bjts.len() {
            let bjt = &self.circuit.bjts[index];
            let sign = bjt.model.polarity.sign();
            let model = bjt.model;
            let nodes = &self.solution[..self.nodes];
            let voltages = [
                sign * node_diff(nodes, bjt.junction_b, bjt.junction_e),
                sign * node_diff(nodes, bjt.junction_b, bjt.junction_c),
            ];
            let charge = bjt_charge_op(voltages[0], voltages[1], model);
            for junction in 0..2 {
                if !bjt_charge_active(model, junction) {
                    continue;
                }
                let q_new = charge.q[junction];
                let i_new = match rule {
                    Integration::BackwardEuler => (q_new - self.bjt_q[index][junction]) / h,
                    Integration::Trapezoidal => {
                        2.0 * (q_new - self.bjt_q[index][junction]) / h
                            - self.bjt_i[index][junction]
                    }
                };
                self.settled &= self.bjt_q[index][junction].to_bits() == q_new.to_bits()
                    && self.bjt_i[index][junction].to_bits() == i_new.to_bits();
                self.bjt_q[index][junction] = q_new;
                self.bjt_i[index][junction] = i_new;
            }
        }
        for index in 0..self.circuit.jfets.len() {
            let jfet = &self.circuit.jfets[index];
            let sign = jfet.model.polarity.sign();
            let model = jfet.model;
            let nodes = &self.solution[..self.nodes];
            let voltages = [
                sign * node_diff(nodes, jfet.g, jfet.junction_s),
                sign * node_diff(nodes, jfet.g, jfet.junction_d),
            ];
            let capacitances = [model.cgs, model.cgd];
            for junction in 0..2 {
                let c0 = capacitances[junction];
                if c0 == 0.0 {
                    continue;
                }
                let q_new = device::junction_depletion_charge(
                    voltages[junction],
                    c0,
                    model.pb,
                    0.5,
                    model.fc,
                )
                .0;
                let i_new = match rule {
                    Integration::BackwardEuler => (q_new - self.jfet_q[index][junction]) / h,
                    Integration::Trapezoidal => {
                        2.0 * (q_new - self.jfet_q[index][junction]) / h
                            - self.jfet_i[index][junction]
                    }
                };
                self.settled &= self.jfet_q[index][junction].to_bits() == q_new.to_bits()
                    && self.jfet_i[index][junction].to_bits() == i_new.to_bits();
                self.jfet_q[index][junction] = q_new;
                self.jfet_i[index][junction] = i_new;
            }
        }
        for index in 0..self.circuit.mosfets.len() {
            let mosfet = &self.circuit.mosfets[index];
            let nodes = &self.solution[..self.nodes];

            let voltages = [self.mos_vbd[index], self.mos_vbs[index]];
            for junction in 0..2 {
                let drain = junction == 0;
                if mosfet.bulk_junction_capacitances(drain) == [0.0, 0.0] {
                    continue;
                }
                let q_new = mosfet.bulk_junction_charge(voltages[junction], drain).0;
                let i_new = match rule {
                    Integration::BackwardEuler => (q_new - self.mos_bulk_q[index][junction]) / h,
                    Integration::Trapezoidal => {
                        2.0 * (q_new - self.mos_bulk_q[index][junction]) / h
                            - self.mos_bulk_i[index][junction]
                    }
                };
                self.settled &= self.mos_bulk_q[index][junction].to_bits() == q_new.to_bits()
                    && self.mos_bulk_i[index][junction].to_bits() == i_new.to_bits();
                self.mos_bulk_q[index][junction] = q_new;
                self.mos_bulk_i[index][junction] = i_new;
            }

            let voltages = [
                node_diff(nodes, mosfet.g, mosfet.junction_s),
                node_diff(nodes, mosfet.g, mosfet.junction_d),
                node_diff(nodes, mosfet.g, mosfet.bulk),
            ];
            let capacitances = mosfet.overlap_capacitances();
            for junction in 0..3 {
                let q_new = capacitances[junction] * voltages[junction];
                let i_new = match rule {
                    Integration::BackwardEuler => (q_new - self.mos_overlap_q[index][junction]) / h,
                    Integration::Trapezoidal => {
                        2.0 * (q_new - self.mos_overlap_q[index][junction]) / h
                            - self.mos_overlap_i[index][junction]
                    }
                };
                self.settled &= self.mos_overlap_q[index][junction].to_bits() == q_new.to_bits()
                    && self.mos_overlap_i[index][junction].to_bits() == i_new.to_bits();
                self.mos_overlap_q[index][junction] = q_new;
                self.mos_overlap_i[index][junction] = i_new;
            }

            let channel_caps = mos_channel_capacitance_halves(
                mosfet,
                self.mos_vgs[index],
                self.mos_vds[index],
                self.mos_vbd[index],
                self.mos_vbs[index],
            );
            let terminals = [mosfet.junction_s, mosfet.junction_d, mosfet.bulk];
            for junction in 0..3 {
                let voltage = node_diff(nodes, mosfet.g, terminals[junction]);
                let capacitance =
                    self.mos_channel_cap_half[index][junction] + channel_caps[junction];
                let charge_delta = (voltage - self.mos_channel_v[index][junction]) * capacitance;
                let q_new = self.mos_channel_q[index][junction] + charge_delta;
                let i_new = match rule {
                    Integration::BackwardEuler => charge_delta / h,
                    Integration::Trapezoidal => {
                        2.0 * charge_delta / h - self.mos_channel_i[index][junction]
                    }
                };
                self.settled &= self.mos_channel_cap_half[index][junction].to_bits()
                    == channel_caps[junction].to_bits()
                    && self.mos_channel_v[index][junction].to_bits() == voltage.to_bits()
                    && self.mos_channel_q[index][junction].to_bits() == q_new.to_bits()
                    && self.mos_channel_i[index][junction].to_bits() == i_new.to_bits();
                self.mos_channel_v[index][junction] = voltage;
                self.mos_channel_q[index][junction] = q_new;
                self.mos_channel_i[index][junction] = i_new;
                self.mos_channel_cap_half[index][junction] = channel_caps[junction];
            }

            if mosfet.model.channel_charge_model == MosChannelChargeModel::WardDutton {
                let charge = device::mos_ward_dutton_channel_charge(
                    mosfet,
                    self.mos_vgs[index],
                    self.mos_vds[index],
                    self.mos_vbd[index],
                    self.mos_vbs[index],
                );
                for terminal in 0..4 {
                    let q_new = charge.q[terminal];
                    let i_new = match rule {
                        Integration::BackwardEuler => {
                            (q_new - self.mos_conserving_q[index][terminal]) / h
                        }
                        Integration::Trapezoidal => {
                            2.0 * (q_new - self.mos_conserving_q[index][terminal]) / h
                                - self.mos_conserving_i[index][terminal]
                        }
                    };
                    self.settled &= self.mos_conserving_q[index][terminal].to_bits()
                        == q_new.to_bits()
                        && self.mos_conserving_i[index][terminal].to_bits() == i_new.to_bits();
                    self.mos_conserving_q[index][terminal] = q_new;
                    self.mos_conserving_i[index][terminal] = i_new;
                }
            }
        }
        for index in 0..self.circuit.inductors.len() {
            let inductor = &self.circuit.inductors[index];
            let branch = self.nodes + self.circuit.voltage_sources.len() + index;
            self.settled &= self.ind_i[index].to_bits() == self.solution[branch].to_bits()
                && self.ind_v[index].to_bits()
                    == node_diff(&self.solution[..self.nodes], inductor.a, inductor.b).to_bits();
            self.ind_i[index] = self.solution[branch];
            self.ind_v[index] = node_diff(&self.solution[..self.nodes], inductor.a, inductor.b);
        }

        self.node_v.copy_from_slice(&self.solution[..self.nodes]);
        self.branch_i.copy_from_slice(&self.solution[self.nodes..]);
        self.time = time;
        self.step_index += 1;
        Ok(())
    }

    /// Simulated time at the end of the last completed step, seconds.
    pub fn time(&self) -> f64 {
        self.time
    }

    /// Internal steps completed since the solver was built.
    pub fn step_index(&self) -> u64 {
        self.step_index
    }

    /// True when the circuit needs Newton iteration.
    pub fn is_nonlinear(&self) -> bool {
        self.nonlinear
    }

    /// Re-evaluate every source that carries a transient function at `time`.
    ///
    /// Sources are evaluated at the END of the step, which is the point the
    /// backward-Euler and trapezoidal companion models are written about, and
    /// is what SPICE does. A value that actually moved counts as a
    /// discontinuity for the trapezoidal restart rule, exactly as a routed
    /// input does.
    fn apply_waveforms(&mut self, time: f64) {
        if !self.has_waveforms {
            return;
        }
        for index in 0..self.circuit.voltage_sources.len() {
            let wave = self.circuit.voltage_sources[index].wave;
            if wave.is_constant() {
                continue;
            }
            self.set_voltage_source(index, wave.at(time));
        }
        for index in 0..self.circuit.current_sources.len() {
            let wave = self.circuit.current_sources[index].wave;
            if wave.is_constant() {
                continue;
            }
            self.set_current_source(index, wave.at(time));
        }
    }

    fn build(&mut self, stamp: Stamp) {
        let dim = self.dim();
        self.matrix.iter_mut().for_each(|cell| *cell = 0.0);
        self.rhs.iter_mut().for_each(|cell| *cell = 0.0);

        for resistor in &self.circuit.resistors {
            conductance(
                &mut self.matrix,
                dim,
                resistor.a,
                resistor.b,
                1.0 / resistor.resistance_ohms(),
            );
        }
        for bjt in &self.circuit.bjts {
            if bjt.model.rb > 0.0 {
                conductance(
                    &mut self.matrix,
                    dim,
                    bjt.b,
                    bjt.junction_b,
                    1.0 / bjt.model.rb,
                );
            }
            if bjt.model.rc > 0.0 {
                conductance(
                    &mut self.matrix,
                    dim,
                    bjt.c,
                    bjt.junction_c,
                    1.0 / bjt.model.rc,
                );
            }
            if bjt.model.re > 0.0 {
                conductance(
                    &mut self.matrix,
                    dim,
                    bjt.e,
                    bjt.junction_e,
                    1.0 / bjt.model.re,
                );
            }
        }
        for mosfet in &self.circuit.mosfets {
            if mosfet.drain_resistance_ohms > 0.0 {
                conductance(
                    &mut self.matrix,
                    dim,
                    mosfet.d,
                    mosfet.junction_d,
                    1.0 / mosfet.drain_resistance_ohms,
                );
            }
            if mosfet.source_resistance_ohms > 0.0 {
                conductance(
                    &mut self.matrix,
                    dim,
                    mosfet.junction_s,
                    mosfet.s,
                    1.0 / mosfet.source_resistance_ohms,
                );
            }
        }
        for jfet in &self.circuit.jfets {
            if jfet.model.rd > 0.0 {
                conductance(
                    &mut self.matrix,
                    dim,
                    jfet.d,
                    jfet.junction_d,
                    1.0 / jfet.model.rd,
                );
            }
            if jfet.model.rs > 0.0 {
                conductance(
                    &mut self.matrix,
                    dim,
                    jfet.junction_s,
                    jfet.s,
                    1.0 / jfet.model.rs,
                );
            }
        }
        for (index, switch) in self.circuit.switches.iter().enumerate() {
            let ohms = if self.switch_closed[index] {
                switch.ron
            } else {
                switch.roff
            };
            conductance(&mut self.matrix, dim, switch.a, switch.b, 1.0 / ohms);
        }
        for (index, source) in self.circuit.current_sources.iter().enumerate() {
            // SPICE: positive current flows from n+ through the source to n-,
            // so n+ is a sink and n- a supply.
            let amps = self.current_source_values[index];
            inject(&mut self.rhs, source.p, -amps);
            inject(&mut self.rhs, source.n, amps);
        }
        for (index, source) in self.circuit.voltage_sources.iter().enumerate() {
            let row = self.nodes + index;
            add(&mut self.matrix, dim, Some(row), source.p, 1.0);
            add(&mut self.matrix, dim, Some(row), source.n, -1.0);
            add(&mut self.matrix, dim, source.p, Some(row), 1.0);
            add(&mut self.matrix, dim, source.n, Some(row), -1.0);
            self.rhs[row] = self.voltage_source_values[index];
        }
        for (index, inductor) in self.circuit.inductors.iter().enumerate() {
            let row = self.nodes + self.circuit.voltage_sources.len() + index;
            add(&mut self.matrix, dim, Some(row), inductor.a, 1.0);
            add(&mut self.matrix, dim, Some(row), inductor.b, -1.0);
            add(&mut self.matrix, dim, inductor.a, Some(row), 1.0);
            add(&mut self.matrix, dim, inductor.b, Some(row), -1.0);
            match stamp {
                Stamp::OperatingPoint => {
                    // A shorted inductor: v_a - v_b = 0.
                    self.rhs[row] = 0.0;
                }
                Stamp::Transient(h, rule) => match rule {
                    Integration::BackwardEuler => {
                        let req = inductor.henries / h;
                        self.matrix[row * dim + row] -= req;
                        self.rhs[row] = -req * self.ind_i[index];
                    }
                    Integration::Trapezoidal => {
                        let req = 2.0 * inductor.henries / h;
                        self.matrix[row * dim + row] -= req;
                        self.rhs[row] = -req * self.ind_i[index] - self.ind_v[index];
                    }
                },
            }
        }
        if let Stamp::Transient(h, rule) = stamp {
            let scale = match rule {
                Integration::BackwardEuler => 1.0 / h,
                Integration::Trapezoidal => 2.0 / h,
            };
            let branch_base = self.nodes + self.circuit.voltage_sources.len();
            for mutual in &self.circuit.mutual_inductors {
                let first = branch_base + mutual.first;
                let second = branch_base + mutual.second;
                let coefficient = -scale * mutual.mutual_henries;
                self.matrix[first * dim + second] += coefficient;
                self.matrix[second * dim + first] += coefficient;
                self.rhs[first] -= scale * mutual.mutual_henries * self.ind_i[mutual.second];
                self.rhs[second] -= scale * mutual.mutual_henries * self.ind_i[mutual.first];
            }
        }
        if let Stamp::Transient(h, rule) = stamp {
            for (index, capacitor) in self.circuit.capacitors.iter().enumerate() {
                let (geq, ieq) = match rule {
                    Integration::BackwardEuler => {
                        let geq = capacitor.farads / h;
                        (geq, geq * self.cap_v[index])
                    }
                    Integration::Trapezoidal => {
                        let geq = 2.0 * capacitor.farads / h;
                        (geq, geq * self.cap_v[index] + self.cap_i[index])
                    }
                };
                conductance(&mut self.matrix, dim, capacitor.a, capacitor.b, geq);
                inject(&mut self.rhs, capacitor.a, ieq);
                inject(&mut self.rhs, capacitor.b, -ieq);
            }
        }
        self.stamp_dependent_sources();
        self.stamp_devices(stamp);
        if self.node_shunt != 0.0 {
            for node in 0..self.nodes {
                self.matrix[node * dim + node] += self.node_shunt;
            }
        }
    }

    /// SPICE's gmin stepping, the operating point's fallback when plain Newton
    /// does not converge: shunt every node to ground with a conductance large
    /// enough to make the problem easy, solve, and walk the shunt down a decade
    /// at a time — each solve starting from the last — to zero.
    ///
    /// What needs it is positive feedback: a comparator with hysteresis is
    /// bistable, and Newton from an all-zero guess can cycle between the two
    /// states forever. A shunt larger than the feedback's negative conductance
    /// makes the solution unique, and the walk down lets it settle into one of
    /// the states. Only reached after plain Newton failed, so an operating
    /// point that converged before converges exactly as before.
    fn gmin_stepping(&mut self) -> Result<(), AnalogError> {
        let mut shunt = 1.0;
        while shunt >= 1e-12 {
            self.node_shunt = shunt;
            let stage = self.newton(Stamp::OperatingPoint, None, self.time);
            if stage.is_err() {
                self.node_shunt = 0.0;
                return stage;
            }
            shunt /= 10.0;
        }
        self.node_shunt = 0.0;
        self.newton(Stamp::OperatingPoint, None, self.time)
    }

    /// Stamp the four linear dependent sources. All of them are constant
    /// matrix entries with a zero right-hand side, so they cost the cached
    /// factorisation nothing and leave [`Self::build_rhs`] untouched: a
    /// dependent source's row is `… = 0`, and `build_rhs` zeroes every row it
    /// does not own.
    ///
    /// Nothing here runs for a netlist without one — four empty loops — so the
    /// floating-point order of every existing circuit is unchanged.
    fn stamp_dependent_sources(&mut self) {
        let dim = self.dim();
        let matrix = &mut self.matrix;
        let circuit = &self.circuit;
        let vcvs_base = self.nodes + circuit.voltage_sources.len() + circuit.inductors.len();
        let ccvs_base = vcvs_base + circuit.vcvs.len();

        // E: v(n+) − v(n−) − gain·(v(nc+) − v(nc−)) = 0, with the branch
        // current flowing n+ → n− through the source like a V element's.
        for (index, source) in circuit.vcvs.iter().enumerate() {
            let row = Some(vcvs_base + index);
            add(matrix, dim, row, source.p, 1.0);
            add(matrix, dim, row, source.n, -1.0);
            add(matrix, dim, row, source.cp, -source.gain);
            add(matrix, dim, row, source.cn, source.gain);
            add(matrix, dim, source.p, row, 1.0);
            add(matrix, dim, source.n, row, -1.0);
        }
        // G: gm·(v(nc+) − v(nc−)) leaves n+ and enters n−.
        for source in &circuit.vccs {
            add(matrix, dim, source.p, source.cp, source.gm);
            add(matrix, dim, source.p, source.cn, -source.gm);
            add(matrix, dim, source.n, source.cp, -source.gm);
            add(matrix, dim, source.n, source.cn, source.gm);
        }
        // F: gain·i(control) leaves n+ and enters n−.
        for source in &circuit.cccs {
            let control = Some(self.nodes + source.control_branch);
            add(matrix, dim, source.p, control, source.gain);
            add(matrix, dim, source.n, control, -source.gain);
        }
        // H: v(n+) − v(n−) − r·i(control) = 0.
        for (index, source) in circuit.ccvs.iter().enumerate() {
            let row = Some(ccvs_base + index);
            let control = Some(self.nodes + source.control_branch);
            add(matrix, dim, row, source.p, 1.0);
            add(matrix, dim, row, source.n, -1.0);
            add(matrix, dim, row, control, -source.ohms);
            add(matrix, dim, source.p, row, 1.0);
            add(matrix, dim, source.n, row, -1.0);
        }
    }

    /// Refresh only history and source terms when the conductances are unchanged.
    fn build_rhs(&mut self, stamp: Stamp) {
        self.rhs.fill(0.0);
        for (index, source) in self.circuit.current_sources.iter().enumerate() {
            let amps = self.current_source_values[index];
            inject(&mut self.rhs, source.p, -amps);
            inject(&mut self.rhs, source.n, amps);
        }
        for (index, value) in self.voltage_source_values.iter().enumerate() {
            self.rhs[self.nodes + index] = *value;
        }
        if let Stamp::Transient(h, rule) = stamp {
            for (index, inductor) in self.circuit.inductors.iter().enumerate() {
                let row = self.nodes + self.circuit.voltage_sources.len() + index;
                self.rhs[row] = match rule {
                    Integration::BackwardEuler => -(inductor.henries / h) * self.ind_i[index],
                    Integration::Trapezoidal => {
                        -(2.0 * inductor.henries / h) * self.ind_i[index] - self.ind_v[index]
                    }
                };
            }
            let scale = match rule {
                Integration::BackwardEuler => 1.0 / h,
                Integration::Trapezoidal => 2.0 / h,
            };
            let branch_base = self.nodes + self.circuit.voltage_sources.len();
            for mutual in &self.circuit.mutual_inductors {
                let first = branch_base + mutual.first;
                let second = branch_base + mutual.second;
                self.rhs[first] -= scale * mutual.mutual_henries * self.ind_i[mutual.second];
                self.rhs[second] -= scale * mutual.mutual_henries * self.ind_i[mutual.first];
            }
            for (index, capacitor) in self.circuit.capacitors.iter().enumerate() {
                let ieq = match rule {
                    Integration::BackwardEuler => (capacitor.farads / h) * self.cap_v[index],
                    Integration::Trapezoidal => {
                        (2.0 * capacitor.farads / h) * self.cap_v[index] + self.cap_i[index]
                    }
                };
                inject(&mut self.rhs, capacitor.a, ieq);
                inject(&mut self.rhs, capacitor.b, -ieq);
            }
        }
    }

    /// Dense LU with partial pivoting, in place over the scratch matrix. Own
    /// implementation on purpose: the systems are at most 64×64, and a linear
    /// algebra dependency would have to compile to WASM and stay deterministic
    /// across hosts to buy nothing at this size.
    fn factorize(&mut self) -> Result<(), AnalogError> {
        let dim = self.dim();
        let matrix = &mut self.matrix;
        self.factors.fill(0.0);

        for column in 0..dim {
            let mut pivot_row = column;
            let mut pivot_magnitude = matrix[column * dim + column].abs();
            for row in (column + 1)..dim {
                let magnitude = matrix[row * dim + column].abs();
                if magnitude > pivot_magnitude {
                    pivot_magnitude = magnitude;
                    pivot_row = row;
                }
            }
            if pivot_magnitude < 1e-30 {
                return Err(AnalogError::Singular { row: column });
            }
            self.pivots[column] = pivot_row;
            if pivot_row != column {
                for index in 0..dim {
                    matrix.swap(pivot_row * dim + index, column * dim + index);
                }
            }
            let pivot = matrix[column * dim + column];
            for row in (column + 1)..dim {
                let factor = matrix[row * dim + column] / pivot;
                if factor == 0.0 {
                    continue;
                }
                self.factors[column * dim + row] = factor;
                matrix[row * dim + column] = 0.0;
                for index in (column + 1)..dim {
                    matrix[row * dim + index] -= factor * matrix[column * dim + index];
                }
            }
        }

        Ok(())
    }

    fn solve(&mut self) {
        let dim = self.dim();
        let matrix = &self.matrix;
        let rhs = &mut self.rhs;
        for column in 0..dim {
            rhs.swap(self.pivots[column], column);
            for row in (column + 1)..dim {
                let factor = self.factors[column * dim + row];
                if factor != 0.0 {
                    rhs[row] -= factor * rhs[column];
                }
            }
        }
        for row in (0..dim).rev() {
            let mut accumulator = rhs[row];
            for index in (row + 1)..dim {
                accumulator -= matrix[row * dim + index] * self.solution[index];
            }
            self.solution[row] = accumulator / matrix[row * dim + row];
        }
    }
}

// ---------------------------------------------------------------------------
// Newton–Raphson for the nonlinear devices
// ---------------------------------------------------------------------------

/// Iterations one step is allowed before it is reported as a failure.
///
/// SPICE's default is 100 for a transient step. Reaching anything near it means
/// the circuit is hard, not that the budget is tight: with voltage limiting a
/// diode or transistor stage converges in single digits, and a step that wants
/// 50 iterations is usually one that wants a smaller `substeps` interval.
pub const MAX_NEWTON_ITERATIONS: u32 = 100;

/// How many times [`Solver::advance`] may halve a step whose Newton iteration
/// does not converge: down to `h / 1024`.
pub const MAX_STEP_CUTS: u32 = 10;

/// Relative tolerance on every unknown between two Newton iterations.
///
/// Tighter than SPICE's `RELTOL` (1e-3) on purpose. The engine's job here is to
/// be reproducible rather than fast: at 1e-3 the converged answer depends on
/// the path the limiter took, which makes a golden worth nothing, and the extra
/// iterations cost microseconds on a 64-unknown dense solve.
pub const NEWTON_RELTOL: f64 = 1e-9;

/// Absolute floor for a node voltage, volts. SPICE's `VNTOL` is 1e-6.
pub const NEWTON_VNTOL: f64 = 1e-12;

/// Absolute floor for a branch current, amps. SPICE's `ABSTOL` is 1e-12.
pub const NEWTON_ABSTOL: f64 = 1e-15;

impl Solver {
    /// Iterate the device stamps until the solution stops moving.
    ///
    /// Each pass re-linearises every nonlinear device about the voltages the
    /// last pass produced, re-stamps, re-factorises and solves. Convergence is
    /// declared when every unknown moved less than its tolerance **and** the
    /// voltage limiter did nothing on the previous pass — a limited step is a
    /// damped one, not a Newton step, so accepting one would report an answer
    /// the equations do not satisfy.
    fn newton(&mut self, stamp: Stamp, step: Option<u64>, time: f64) -> Result<(), AnalogError> {
        let dim = self.dim();
        self.cached_stamp = None;
        self.settled = false;
        // NaN so the first pass can never compare as converged, whatever the
        // previous step left in the buffer.
        self.previous[..dim].fill(f64::NAN);
        self.limiter_clamped = true;

        for _ in 1..=MAX_NEWTON_ITERATIONS {
            self.build(stamp);
            self.factorize()?;
            self.solve();
            if self.newton_converged() {
                return Ok(());
            }
            self.previous[..dim].copy_from_slice(&self.solution[..dim]);
            self.relinearise();
        }

        let (unknown, delta) = self.worst_residual();
        Err(AnalogError::NoConvergence {
            step,
            time,
            iterations: MAX_NEWTON_ITERATIONS,
            unknown,
            delta,
        })
    }

    /// Per-unknown tolerance: volts for a node row, amps for a branch row.
    fn newton_tolerance(&self, index: usize, new: f64, old: f64) -> f64 {
        let floor = if index < self.nodes {
            NEWTON_VNTOL
        } else {
            NEWTON_ABSTOL
        };
        floor + NEWTON_RELTOL * new.abs().max(old.abs())
    }

    fn newton_converged(&self) -> bool {
        if self.limiter_clamped {
            return false;
        }
        for index in 0..self.dim() {
            let new = self.solution[index];
            let old = self.previous[index];
            let delta = (new - old).abs();
            // The NaN test is not decoration. Every comparison against a NaN is
            // false, so a bare `delta > tol` reads a NaN solution as CONVERGED
            // and writes it into the trace as an answer. Naming it here is what
            // turns that into the step's coded error instead.
            if delta.is_nan() || delta > self.newton_tolerance(index, new, old) {
                return false;
            }
        }
        true
    }

    /// The unknown that was still moving most, for the failure message.
    fn worst_residual(&self) -> (String, f64) {
        let mut worst = 0usize;
        let mut worst_ratio = f64::NEG_INFINITY;
        for index in 0..self.dim() {
            let new = self.solution[index];
            let old = self.previous[index];
            let delta = (new - old).abs();
            let ratio = delta / self.newton_tolerance(index, new, old);
            // `>` with a NaN ratio is false, so a NaN never wins the report and
            // hides a real residual behind it; the NaN is named only when
            // nothing else moved at all.
            if ratio > worst_ratio || (worst_ratio.is_nan() && !ratio.is_nan()) {
                worst_ratio = ratio;
                worst = index;
            }
        }
        let name = if worst < self.nodes {
            format!("v({})", self.circuit.node_name(worst))
        } else {
            format!("i({})", self.circuit.branch_name(worst - self.nodes))
        };
        (name, (self.solution[worst] - self.previous[worst]).abs())
    }

    /// Move every device's linearisation point to the solution just found,
    /// damped by the SPICE limiters.
    fn relinearise(&mut self) {
        self.limiter_clamped = false;

        for index in 0..self.circuit.diodes.len() {
            let diode = &self.circuit.diodes[index];
            let raw = node_diff(
                &self.solution[..self.nodes],
                diode.junction_anode,
                diode.cathode,
            );
            let vte = diode.model.n * device::THERMAL_VOLTAGE;
            let critical = device::pn_critical_voltage(diode.model.is, diode.model.n);
            let limited = match diode.breakdown {
                None => device::pn_limit(raw, self.diode_vj[index], vte, critical),
                Some(xbv) => device::breakdown_limit(
                    raw,
                    self.diode_vj[index],
                    vte,
                    critical,
                    xbv,
                    diode.model.nbv.unwrap_or(diode.model.n),
                ),
            };
            self.limiter_clamped |= limited != raw;
            self.diode_vj[index] = limited;
        }

        for index in 0..self.circuit.bjts.len() {
            let bjt = &self.circuit.bjts[index];
            let sign = bjt.model.polarity.sign();
            let nodes = &self.solution[..self.nodes];
            let raw_be = sign * node_diff(nodes, bjt.junction_b, bjt.junction_e);
            let raw_bc = sign * node_diff(nodes, bjt.junction_b, bjt.junction_c);
            let vtf = bjt.model.nf * device::THERMAL_VOLTAGE;
            let vtr = bjt.model.nr * device::THERMAL_VOLTAGE;
            let crit_f = device::pn_critical_voltage(bjt.model.is, bjt.model.nf);
            let crit_r = device::pn_critical_voltage(bjt.model.is, bjt.model.nr);
            let be = device::pn_limit(raw_be, self.bjt_vbe[index], vtf, crit_f);
            let bc = device::pn_limit(raw_bc, self.bjt_vbc[index], vtr, crit_r);
            self.limiter_clamped |= be != raw_be || bc != raw_bc;
            self.bjt_vbe[index] = be;
            self.bjt_vbc[index] = bc;
        }

        for index in 0..self.circuit.mosfets.len() {
            let mosfet = &self.circuit.mosfets[index];
            let sign = mosfet.model.polarity.sign();
            let nodes = &self.solution[..self.nodes];
            let raw_gs = sign * node_diff(nodes, mosfet.g, mosfet.junction_s);
            let raw_ds = sign * node_diff(nodes, mosfet.junction_d, mosfet.junction_s);
            let raw_bd = sign * node_diff(nodes, mosfet.bulk, mosfet.junction_d);
            let raw_bs = sign * node_diff(nodes, mosfet.bulk, mosfet.junction_s);

            // The limiters are SPICE's and assume the channel's own source, so
            // they are applied to the EFFECTIVE voltages — the ones a reversed
            // device measures against the terminal that is actually its source.
            // When the device crosses over between two iterations its previous
            // effective Vds in the new orientation is zero, which is exactly
            // where it crossed.
            let old_gs = self.mos_vgs[index];
            let old_ds = self.mos_vds[index];
            let reversed_now = raw_ds < 0.0;
            let reversed_before = old_ds < 0.0;
            let effective = |gs: f64, ds: f64, reversed: bool| {
                if reversed {
                    (gs - ds, -ds)
                } else {
                    (gs, ds)
                }
            };
            let (eff_gs_new, eff_ds_new) = effective(raw_gs, raw_ds, reversed_now);
            let (eff_gs_old, eff_ds_old) = if reversed_before == reversed_now {
                effective(old_gs, old_ds, reversed_before)
            } else {
                (effective(old_gs, old_ds, reversed_before).0, 0.0)
            };

            let eff_ds = device::vds_limit(eff_ds_new, eff_ds_old);
            let eff_gs = device::fet_limit(eff_gs_new, eff_gs_old, sign * mosfet.model.vto);
            let (gs, ds) = if reversed_now {
                (eff_gs - eff_ds, -eff_ds)
            } else {
                (eff_gs, eff_ds)
            };
            self.limiter_clamped |= gs != raw_gs || ds != raw_ds;
            self.mos_vgs[index] = gs;
            self.mos_vds[index] = ds;

            let vte = device::THERMAL_VOLTAGE;
            let critical = device::pn_critical_voltage(mosfet.model.is, 1.0);
            let bd = device::pn_limit(raw_bd, self.mos_vbd[index], vte, critical);
            let bs = device::pn_limit(raw_bs, self.mos_vbs[index], vte, critical);
            self.limiter_clamped |= bd != raw_bd || bs != raw_bs;
            self.mos_vbd[index] = bd;
            self.mos_vbs[index] = bs;
        }

        for index in 0..self.circuit.jfets.len() {
            let jfet = &self.circuit.jfets[index];
            let sign = jfet.model.polarity.sign();
            let nodes = &self.solution[..self.nodes];
            let raw_gs = sign * node_diff(nodes, jfet.g, jfet.junction_s);
            let raw_gd = sign * node_diff(nodes, jfet.g, jfet.junction_d);
            let raw_ds = sign * node_diff(nodes, jfet.junction_d, jfet.junction_s);
            let critical = device::pn_critical_voltage(jfet.model.is, 1.0);
            let gs = device::pn_limit(
                raw_gs,
                self.jfet_vgs[index],
                device::THERMAL_VOLTAGE,
                critical,
            );
            let gd = device::pn_limit(
                raw_gd,
                self.jfet_vgd[index],
                device::THERMAL_VOLTAGE,
                critical,
            );

            // The lower-potential channel terminal is the effective source.
            // Gate junctions keep their physical terminal identities; only
            // the channel control voltage changes when current reverses.
            let reversed_now = raw_ds < 0.0;
            let reversed_before = self.jfet_vds[index] < 0.0;
            let old_effective_ds = if reversed_before == reversed_now {
                self.jfet_vds[index].abs()
            } else {
                0.0
            };
            let effective_ds = device::vds_limit(raw_ds.abs(), old_effective_ds);
            let ds = if reversed_now {
                -effective_ds
            } else {
                effective_ds
            };

            self.limiter_clamped |= gs != raw_gs || gd != raw_gd || ds != raw_ds;
            self.jfet_vgs[index] = gs;
            self.jfet_vgd[index] = gd;
            self.jfet_vds[index] = ds;
        }

        // A power sink is smooth (a product over a floored quotient), so it
        // takes the plain Newton point with no limiting.
        for index in 0..self.circuit.power_sinks.len() {
            let sink = &self.circuit.power_sinks[index];
            let nodes = &self.solution[..self.nodes];
            self.sink_point[index] = [
                node_diff(nodes, sink.p, sink.n),
                node_diff(nodes, sink.op, sink.on),
                self.solution[self.nodes + sink.control_branch],
            ];
        }
    }

    /// Stamp every nonlinear device's companion model at its current
    /// linearisation point.
    ///
    /// Each terminal contributes one linearised current `I ≈ I0 + Σ gᵢ·(uᵢ −
    /// uᵢ0)`, where `uᵢ` are the device's controlling voltages in **circuit**
    /// coordinates. [`stamp_linearised`] turns that into the matrix and RHS
    /// entries; the polarity of a PNP or a PMOS shows up only in the sign of
    /// the currents, never in the conductances, because negating both the
    /// controlling voltage and the current leaves `∂I/∂u` alone.
    ///
    /// Nothing here runs for a circuit with no such device: all four vectors
    /// are empty, so a linear netlist takes the same floating-point operations
    /// in the same order it took before this existed.
    fn stamp_devices(&mut self, stamp: Stamp) {
        let dim = self.dim();
        let matrix = &mut self.matrix;
        let rhs = &mut self.rhs;

        for (index, diode) in self.circuit.diodes.iter().enumerate() {
            if diode.model.rs > 0.0 {
                conductance(
                    matrix,
                    dim,
                    diode.anode,
                    diode.junction_anode,
                    1.0 / diode.model.rs,
                );
            }
            let vj = self.diode_vj[index];
            let op = diode_model_op(vj, diode.model, diode.breakdown);
            let control = [(op.gd, diode.junction_anode, diode.cathode, vj)];
            stamp_linearised(matrix, rhs, dim, diode.junction_anode, op.id, &control);
            stamp_linearised(matrix, rhs, dim, diode.cathode, -op.id, &negate(&control));

            if diode_charge_active(diode.model) {
                if let Stamp::Transient(h, rule) = stamp {
                    let (charge, capacitance) = diode_charge_op(vj, diode.model, op);
                    let (geq, current) = match rule {
                        Integration::BackwardEuler => {
                            (capacitance / h, (charge - self.diode_q[index]) / h)
                        }
                        Integration::Trapezoidal => (
                            2.0 * capacitance / h,
                            2.0 * (charge - self.diode_q[index]) / h - self.diode_i[index],
                        ),
                    };
                    let charge_control = [(geq, diode.junction_anode, diode.cathode, vj)];
                    stamp_linearised(
                        matrix,
                        rhs,
                        dim,
                        diode.junction_anode,
                        current,
                        &charge_control,
                    );
                    stamp_linearised(
                        matrix,
                        rhs,
                        dim,
                        diode.cathode,
                        -current,
                        &negate(&charge_control),
                    );
                }
            }

            if diode.model.vp > 0.0 && diode.model.tt > 0.0 {
                if let Stamp::Transient(h, rule) = stamp {
                    let recovery = diode_recovery_step(
                        diode.model,
                        op,
                        self.diode_recovery_state[index],
                        self.diode_recovery_rate[index],
                        h,
                        rule,
                    );
                    let recovery_control = [(
                        recovery.conductance,
                        diode.junction_anode,
                        diode.cathode,
                        vj,
                    )];
                    stamp_linearised(
                        matrix,
                        rhs,
                        dim,
                        diode.junction_anode,
                        recovery.current,
                        &recovery_control,
                    );
                    stamp_linearised(
                        matrix,
                        rhs,
                        dim,
                        diode.cathode,
                        -recovery.current,
                        &negate(&recovery_control),
                    );
                }
            }
        }

        for (index, bjt) in self.circuit.bjts.iter().enumerate() {
            let sign = bjt.model.polarity.sign();
            let model = bjt.model;
            let (vbe, vbc) = (self.bjt_vbe[index], self.bjt_vbc[index]);
            let op = device::bjt_op(
                vbe, vbc, model.is, model.bf, model.br, model.nf, model.nr, model.vaf, model.var,
                model.ikf, model.ikr, model.ise, model.ne, model.isc, model.nc,
            );

            // GMIN across both junctions. Without it a transistor that is fully
            // off leaves its base and collector tied to the rest of the circuit
            // by nothing at all, and the matrix is singular on a netlist the
            // user can see is connected.
            conductance(matrix, dim, bjt.junction_b, bjt.junction_e, device::GMIN);
            conductance(matrix, dim, bjt.junction_b, bjt.junction_c, device::GMIN);

            // Controlling voltages in circuit coordinates: u1 = v(b) − v(e),
            // u2 = v(b) − v(c). The device sees sign·u1 and sign·u2.
            let u1 = sign * vbe;
            let u2 = sign * vbc;
            let collector = [
                (op.gif, bjt.junction_b, bjt.junction_e, u1),
                (-op.gir - op.gmu, bjt.junction_b, bjt.junction_c, u2),
            ];
            let base = [
                (op.gpi, bjt.junction_b, bjt.junction_e, u1),
                (op.gmu, bjt.junction_b, bjt.junction_c, u2),
            ];
            let emitter = [
                (-op.gif - op.gpi, bjt.junction_b, bjt.junction_e, u1),
                (op.gir, bjt.junction_b, bjt.junction_c, u2),
            ];
            stamp_linearised(matrix, rhs, dim, bjt.junction_c, sign * op.ic, &collector);
            stamp_linearised(matrix, rhs, dim, bjt.junction_b, sign * op.ib, &base);
            stamp_linearised(
                matrix,
                rhs,
                dim,
                bjt.junction_e,
                -sign * (op.ic + op.ib),
                &emitter,
            );

            if let Stamp::Transient(h, rule) = stamp {
                let charge = bjt_charge_op(vbe, vbc, model);
                for junction in 0..2 {
                    if !bjt_charge_active(model, junction) {
                        continue;
                    }
                    let (positive, negative) = if junction == 0 {
                        (bjt.junction_b, bjt.junction_e)
                    } else {
                        (bjt.junction_b, bjt.junction_c)
                    };
                    let factor = match rule {
                        Integration::BackwardEuler => 1.0 / h,
                        Integration::Trapezoidal => 2.0 / h,
                    };
                    let current = match rule {
                        Integration::BackwardEuler => {
                            (charge.q[junction] - self.bjt_q[index][junction]) / h
                        }
                        Integration::Trapezoidal => {
                            2.0 * (charge.q[junction] - self.bjt_q[index][junction]) / h
                                - self.bjt_i[index][junction]
                        }
                    };
                    let control = [
                        (
                            charge.dq[junction][0] * factor,
                            bjt.junction_b,
                            bjt.junction_e,
                            u1,
                        ),
                        (
                            charge.dq[junction][1] * factor,
                            bjt.junction_b,
                            bjt.junction_c,
                            u2,
                        ),
                    ];
                    stamp_linearised(matrix, rhs, dim, positive, sign * current, &control);
                    stamp_linearised(
                        matrix,
                        rhs,
                        dim,
                        negative,
                        -sign * current,
                        &negate(&control),
                    );
                }
            }
        }

        for (index, mosfet) in self.circuit.mosfets.iter().enumerate() {
            let sign = mosfet.model.polarity.sign();
            let model = mosfet.model;
            let (vgs, vds) = (self.mos_vgs[index], self.mos_vds[index]);

            // Below Vto the channel is off, so GMIN connects drain to source.
            // The two bulk pn junctions include their own GMIN through
            // diode_op, and use the model's bulk-junction saturation current.
            conductance(
                matrix,
                dim,
                mosfet.junction_d,
                mosfet.junction_s,
                device::GMIN,
            );

            // A MOSFET is symmetric: when Vds goes negative the terminal the
            // netlist calls the drain IS the source, and the level-1 equations
            // are written from the source out. Swapping the two nodes is the
            // whole of "reverse mode".
            let reversed = vds < 0.0;
            let (drain, source) = if reversed {
                (mosfet.junction_s, mosfet.junction_d)
            } else {
                (mosfet.junction_d, mosfet.junction_s)
            };
            let (vgs_eff, vds_eff) = if reversed {
                (vgs - vds, -vds)
            } else {
                (vgs, vds)
            };
            let vbs_eff = if reversed {
                self.mos_vbd[index]
            } else {
                self.mos_vbs[index]
            };
            let op = device::mos_op(
                vgs_eff,
                vds_eff,
                sign * model.vto,
                mosfet.beta,
                model.lambda,
                model.gamma,
                model.phi,
                vbs_eff,
            );

            let u1 = sign * vgs_eff;
            let u2 = sign * vds_eff;
            let u3 = sign * vbs_eff;
            let drain_terms = [
                (op.gm, mosfet.g, source, u1),
                (op.gds, drain, source, u2),
                (op.gmb, mosfet.bulk, source, u3),
            ];
            stamp_linearised(matrix, rhs, dim, drain, sign * op.id, &drain_terms);
            stamp_linearised(
                matrix,
                rhs,
                dim,
                source,
                -sign * op.id,
                &negate(&drain_terms),
            );

            if let Stamp::Transient(h, rule) = stamp {
                let capacitances = mosfet.overlap_capacitances();
                let terminals = [
                    (mosfet.g, mosfet.junction_s),
                    (mosfet.g, mosfet.junction_d),
                    (mosfet.g, mosfet.bulk),
                ];
                for junction in 0..3 {
                    let capacitance = capacitances[junction];
                    if capacitance == 0.0 {
                        continue;
                    }
                    let (positive, negative) = terminals[junction];
                    let voltage = node_diff(&self.solution[..self.nodes], positive, negative);
                    let charge = capacitance * voltage;
                    let (geq, current) = match rule {
                        Integration::BackwardEuler => (
                            capacitance / h,
                            (charge - self.mos_overlap_q[index][junction]) / h,
                        ),
                        Integration::Trapezoidal => (
                            2.0 * capacitance / h,
                            2.0 * (charge - self.mos_overlap_q[index][junction]) / h
                                - self.mos_overlap_i[index][junction],
                        ),
                    };
                    let control = [(geq, positive, negative, voltage)];
                    stamp_linearised(matrix, rhs, dim, positive, current, &control);
                    stamp_linearised(matrix, rhs, dim, negative, -current, &negate(&control));
                }

                if model.channel_charge_model == MosChannelChargeModel::WardDutton {
                    let charge = device::mos_ward_dutton_channel_charge(
                        mosfet,
                        vgs,
                        vds,
                        self.mos_vbd[index],
                        self.mos_vbs[index],
                    );
                    let terminals = [mosfet.g, mosfet.junction_d, mosfet.junction_s, mosfet.bulk];
                    let controls = [
                        (mosfet.g, mosfet.junction_s),
                        (mosfet.junction_d, mosfet.junction_s),
                        (mosfet.bulk, mosfet.junction_s),
                    ];
                    let voltages = controls.map(|(positive, negative)| {
                        node_diff(&self.solution[..self.nodes], positive, negative)
                    });
                    let scale = match rule {
                        Integration::BackwardEuler => 1.0 / h,
                        Integration::Trapezoidal => 2.0 / h,
                    };
                    for (terminal, &node) in terminals.iter().enumerate() {
                        let charge_delta =
                            charge.q[terminal] - self.mos_conserving_q[index][terminal];
                        let current = match rule {
                            Integration::BackwardEuler => charge_delta / h,
                            Integration::Trapezoidal => {
                                2.0 * charge_delta / h - self.mos_conserving_i[index][terminal]
                            }
                        };
                        let control: [(f64, NodeRef, NodeRef, f64); 3] =
                            core::array::from_fn(|control| {
                                (
                                    scale * charge.dq_dv[terminal][control],
                                    controls[control].0,
                                    controls[control].1,
                                    voltages[control],
                                )
                            });
                        stamp_linearised(matrix, rhs, dim, node, current, &control);
                    }
                } else {
                    let channel_caps = mos_channel_capacitance_halves(
                        mosfet,
                        vgs,
                        vds,
                        self.mos_vbd[index],
                        self.mos_vbs[index],
                    );
                    let terminals = [mosfet.junction_s, mosfet.junction_d, mosfet.bulk];
                    for junction in 0..3 {
                        let capacitance =
                            self.mos_channel_cap_half[index][junction] + channel_caps[junction];
                        if capacitance == 0.0 {
                            continue;
                        }
                        let positive = mosfet.g;
                        let negative = terminals[junction];
                        let voltage = node_diff(&self.solution[..self.nodes], positive, negative);
                        let charge_delta =
                            (voltage - self.mos_channel_v[index][junction]) * capacitance;
                        let factor = match rule {
                            Integration::BackwardEuler => 1.0 / h,
                            Integration::Trapezoidal => 2.0 / h,
                        };
                        let geq = factor * capacitance;
                        let current = match rule {
                            Integration::BackwardEuler => charge_delta / h,
                            Integration::Trapezoidal => {
                                2.0 * charge_delta / h - self.mos_channel_i[index][junction]
                            }
                        };
                        let control = [(geq, positive, negative, voltage)];
                        stamp_linearised(matrix, rhs, dim, positive, current, &control);
                        stamp_linearised(matrix, rhs, dim, negative, -current, &negate(&control));
                    }
                }
            }

            // Level-1 MOSFETs have two bulk pn junctions. In NMOS coordinates
            // the bulk is the anode and drain/source are the cathodes; the
            // polarity sign reverses that orientation for PMOS.
            let bd = device::diode_op(self.mos_vbd[index], model.is, 1.0);
            let bd_terms = [(
                bd.gd,
                mosfet.bulk,
                mosfet.junction_d,
                sign * self.mos_vbd[index],
            )];
            stamp_linearised(matrix, rhs, dim, mosfet.bulk, sign * bd.id, &bd_terms);
            stamp_linearised(
                matrix,
                rhs,
                dim,
                mosfet.junction_d,
                -sign * bd.id,
                &negate(&bd_terms),
            );

            let bs = device::diode_op(self.mos_vbs[index], model.is, 1.0);
            let bs_terms = [(
                bs.gd,
                mosfet.bulk,
                mosfet.junction_s,
                sign * self.mos_vbs[index],
            )];
            stamp_linearised(matrix, rhs, dim, mosfet.bulk, sign * bs.id, &bs_terms);
            stamp_linearised(
                matrix,
                rhs,
                dim,
                mosfet.junction_s,
                -sign * bs.id,
                &negate(&bs_terms),
            );

            for (history, (junction, voltage)) in [
                (mosfet.junction_d, self.mos_vbd[index]),
                (mosfet.junction_s, self.mos_vbs[index]),
            ]
            .into_iter()
            .enumerate()
            {
                let drain = history == 0;
                if mosfet.bulk_junction_capacitances(drain) == [0.0, 0.0] {
                    continue;
                }
                if let Stamp::Transient(h, rule) = stamp {
                    let (charge, capacitance) = mosfet.bulk_junction_charge(voltage, drain);
                    let (geq, current) = match rule {
                        Integration::BackwardEuler => (
                            capacitance / h,
                            (charge - self.mos_bulk_q[index][history]) / h,
                        ),
                        Integration::Trapezoidal => (
                            2.0 * capacitance / h,
                            2.0 * (charge - self.mos_bulk_q[index][history]) / h
                                - self.mos_bulk_i[index][history],
                        ),
                    };
                    let charge_terms = [(geq, mosfet.bulk, junction, sign * voltage)];
                    stamp_linearised(matrix, rhs, dim, mosfet.bulk, sign * current, &charge_terms);
                    stamp_linearised(
                        matrix,
                        rhs,
                        dim,
                        junction,
                        -sign * current,
                        &negate(&charge_terms),
                    );
                }
            }
            // The gate carries no DC current in this model.
        }

        for (index, jfet) in self.circuit.jfets.iter().enumerate() {
            let sign = jfet.model.polarity.sign();
            let reversed = self.jfet_vds[index] < 0.0;
            let (drain, source, vgs) = if reversed {
                (jfet.junction_s, jfet.junction_d, self.jfet_vgd[index])
            } else {
                (jfet.junction_d, jfet.junction_s, self.jfet_vgs[index])
            };
            let vds = self.jfet_vds[index].abs();
            let op = device::jfet_op(
                vgs,
                vds,
                sign * jfet.model.vto,
                jfet.model.beta,
                jfet.model.lambda,
            );

            // The channel's GMIN keeps a cut-off device connected. The gate
            // junctions below each carry their own SPICE GMIN as part of the
            // diode equation.
            conductance(matrix, dim, jfet.junction_d, jfet.junction_s, device::GMIN);

            let channel_terms = [
                (op.gm, jfet.g, source, sign * vgs),
                (op.gds, drain, source, sign * vds),
            ];
            stamp_linearised(matrix, rhs, dim, drain, sign * op.id, &channel_terms);
            stamp_linearised(
                matrix,
                rhs,
                dim,
                source,
                -sign * op.id,
                &negate(&channel_terms),
            );

            for (junction, (cathode, voltage, c0)) in [
                (jfet.junction_s, self.jfet_vgs[index], jfet.model.cgs),
                (jfet.junction_d, self.jfet_vgd[index], jfet.model.cgd),
            ]
            .into_iter()
            .enumerate()
            {
                let gate = device::diode_op(voltage, jfet.model.is, 1.0);
                let gate_voltage = sign * voltage;
                let gate_terms = [(gate.gd, jfet.g, cathode, gate_voltage)];
                stamp_linearised(matrix, rhs, dim, jfet.g, sign * gate.id, &gate_terms);
                stamp_linearised(
                    matrix,
                    rhs,
                    dim,
                    cathode,
                    -sign * gate.id,
                    &negate(&gate_terms),
                );

                if c0 > 0.0 {
                    if let Stamp::Transient(h, rule) = stamp {
                        let (charge, capacitance) = device::junction_depletion_charge(
                            voltage,
                            c0,
                            jfet.model.pb,
                            0.5,
                            jfet.model.fc,
                        );
                        let (geq, current) = match rule {
                            Integration::BackwardEuler => {
                                (capacitance / h, (charge - self.jfet_q[index][junction]) / h)
                            }
                            Integration::Trapezoidal => (
                                2.0 * capacitance / h,
                                2.0 * (charge - self.jfet_q[index][junction]) / h
                                    - self.jfet_i[index][junction],
                            ),
                        };
                        let charge_terms = [(geq, jfet.g, cathode, sign * voltage)];
                        stamp_linearised(matrix, rhs, dim, jfet.g, sign * current, &charge_terms);
                        stamp_linearised(
                            matrix,
                            rhs,
                            dim,
                            cathode,
                            -sign * current,
                            &negate(&charge_terms),
                        );
                    }
                }
            }
        }

        // Power sinks: I = −vo·ib / (eff·d), d = max(vin, POWER_SINK_VMIN),
        // flowing p → n through the element. Linearised in (vin, vo, ib):
        // the branch current is an unknown like a node voltage, so its term
        // goes into the branch column directly.
        for (index, sink) in self.circuit.power_sinks.iter().enumerate() {
            let [vin, vo, ib] = self.sink_point[index];
            let floored = vin < POWER_SINK_VMIN;
            let d = if floored { POWER_SINK_VMIN } else { vin };
            let k = 1.0 / (sink.eff * d);
            let current = -vo * ib * k;
            let g_vo = -ib * k;
            let g_ib = -vo * k;
            let g_vin = if floored { 0.0 } else { vo * ib * k / d };
            let column = Some(self.nodes + sink.control_branch);
            for (terminal, sign) in [(sink.p, 1.0), (sink.n, -1.0)] {
                let Some(row) = terminal else { continue };
                add(matrix, dim, Some(row), sink.p, sign * g_vin);
                add(matrix, dim, Some(row), sink.n, -sign * g_vin);
                add(matrix, dim, Some(row), sink.op, sign * g_vo);
                add(matrix, dim, Some(row), sink.on, -sign * g_vo);
                add(matrix, dim, Some(row), column, sign * g_ib);
                let equivalent = current - g_vin * vin - g_vo * vo - g_ib * ib;
                rhs[row] -= sign * equivalent;
            }
        }
    }
}

/// One terminal's linearised current, as a matrix row and an RHS entry.
///
/// `current` is the current flowing **out of** `terminal` into the device at
/// the linearisation point, and each entry of `control` is `(∂current/∂u, p, n,
/// u)` for a controlling voltage `u = v(p) − v(n)`. The equivalent current
/// source is `current − Σ g·u`, which is the part of the tangent that does not
/// depend on the unknowns.
fn stamp_linearised(
    matrix: &mut [f64],
    rhs: &mut [f64],
    dim: usize,
    terminal: NodeRef,
    current: f64,
    control: &[(f64, NodeRef, NodeRef, f64)],
) {
    let Some(row) = terminal else {
        // Ground is not an unknown: its KCL row is the one the MNA drops.
        return;
    };
    let mut equivalent = current;
    for (g, p, n, u) in control {
        add(matrix, dim, Some(row), *p, *g);
        add(matrix, dim, Some(row), *n, -*g);
        equivalent -= g * u;
    }
    rhs[row] -= equivalent;
}

/// The same controlling terms with every conductance negated — the other
/// terminal of a two-terminal current path.
fn negate<const N: usize>(
    control: &[(f64, NodeRef, NodeRef, f64); N],
) -> [(f64, NodeRef, NodeRef, f64); N] {
    control.map(|(g, p, n, u)| (-g, p, n, u))
}

fn node_diff(node_v: &[f64], a: NodeRef, b: NodeRef) -> f64 {
    let va = a.map(|index| node_v[index]).unwrap_or(0.0);
    let vb = b.map(|index| node_v[index]).unwrap_or(0.0);
    va - vb
}

fn add(matrix: &mut [f64], dim: usize, row: NodeRef, column: NodeRef, value: f64) {
    if let (Some(row), Some(column)) = (row, column) {
        matrix[row * dim + column] += value;
    }
}

fn conductance(matrix: &mut [f64], dim: usize, a: NodeRef, b: NodeRef, g: f64) {
    add(matrix, dim, a, a, g);
    add(matrix, dim, b, b, g);
    add(matrix, dim, a, b, -g);
    add(matrix, dim, b, a, -g);
}

fn inject(rhs: &mut [f64], node: NodeRef, amps: f64) {
    if let Some(index) = node {
        rhs[index] += amps;
    }
}

#[cfg(test)]
mod cache_tests {
    use super::*;
    use crate::analog::parse_netlist;

    // Independent pre-cache Gaussian elimination oracle: RHS row operations
    // are interleaved with matrix elimination, including multiple row pivots.
    fn original_solution(solver: &mut Solver, h: f64) -> Vec<u64> {
        let rule = if solver.restart {
            Integration::BackwardEuler
        } else {
            solver.integration
        };
        solver.build(Stamp::Transient(h, rule));
        let dim = solver.dim();
        let matrix = &mut solver.matrix;
        let rhs = &mut solver.rhs;
        for column in 0..dim {
            let mut pivot_row = column;
            let mut magnitude = matrix[column * dim + column].abs();
            for row in column + 1..dim {
                let candidate = matrix[row * dim + column].abs();
                if candidate > magnitude {
                    magnitude = candidate;
                    pivot_row = row;
                }
            }
            assert!(magnitude >= 1e-30);
            if pivot_row != column {
                for index in 0..dim {
                    matrix.swap(pivot_row * dim + index, column * dim + index);
                }
                rhs.swap(pivot_row, column);
            }
            let pivot = matrix[column * dim + column];
            for row in column + 1..dim {
                let factor = matrix[row * dim + column] / pivot;
                if factor == 0.0 {
                    continue;
                }
                matrix[row * dim + column] = 0.0;
                for index in column + 1..dim {
                    matrix[row * dim + index] -= factor * matrix[column * dim + index];
                }
                rhs[row] -= factor * rhs[column];
            }
        }
        let mut solution = vec![0.0; dim];
        for row in (0..dim).rev() {
            let mut accumulator = rhs[row];
            for index in row + 1..dim {
                accumulator -= matrix[row * dim + index] * solution[index];
            }
            solution[row] = accumulator / matrix[row * dim + row];
        }
        bits(&solution)
    }

    fn bits(values: &[f64]) -> Vec<u64> {
        values.iter().map(|value| value.to_bits()).collect()
    }

    #[test]
    fn cached_steps_match_refactorization_across_discontinuities() {
        for rule in [Integration::BackwardEuler, Integration::Trapezoidal] {
            let circuit = parse_netlist("V1 in 0 DC 5\nR1 in out 1000\nC1 out 0 1u ic=0\nL1 out tail 1m\nR2 tail 0 100\nI1 out 0 DC 0\nS1 out 0 touch ron=10 roff=1e9\n").unwrap();
            let mut cached = Solver::new(circuit, rule).unwrap();
            let mut reference = cached.clone();
            for step in 0..400 {
                let h = if step % 70 < 35 { 1e-6 } else { 2e-6 };
                for solver in [&mut cached, &mut reference] {
                    solver.set_voltage_source(0, if step % 90 < 45 { 5.0 } else { 0.0 });
                    solver.set_current_source(0, if step % 60 < 30 { 0.001 } else { 0.0 });
                    solver.set_switch(0, step % 100 >= 50);
                    if step == 250 {
                        solver.solve_operating_point().unwrap();
                    }
                }
                let expected = original_solution(&mut reference, h);
                reference.cached_stamp = None;
                reference.settled = false;
                cached.advance(h).unwrap();
                reference.advance(h).unwrap();
                assert_eq!(
                    bits(&cached.solution),
                    expected,
                    "original solve, step {step} {rule:?}"
                );
                assert_eq!(
                    bits(&cached.node_v),
                    bits(&reference.node_v),
                    "step {step} {rule:?}"
                );
                assert_eq!(bits(&cached.branch_i), bits(&reference.branch_i));
                assert_eq!(bits(&cached.cap_i), bits(&reference.cap_i));
                assert_eq!(bits(&cached.ind_v), bits(&reference.ind_v));
            }
        }
    }

    #[test]
    fn settled_circuit_resumes_after_input_or_step_change() {
        for rule in [Integration::BackwardEuler, Integration::Trapezoidal] {
            for netlist in [
                "V1 in 0 DC 5\nR1 in out 1000\nC1 out 0 1u\n",
                "V1 in 0 DC 5\nR1 in out 1000\nR2 out 0 1000\n",
            ] {
                let circuit = parse_netlist(netlist).unwrap();
                let mut solver = Solver::new(circuit, rule).unwrap();
                for _ in 0..10000 {
                    solver.advance(1e-3).unwrap();
                }
                assert!(solver.settled);
                let mut reference = solver.clone();
                for step in 0..50 {
                    if step == 10 {
                        solver.set_voltage_source(0, 0.0);
                        reference.set_voltage_source(0, 0.0);
                    }
                    reference.cached_stamp = None;
                    reference.settled = false;
                    let h = if step < 5 { 1e-3 } else { 1e-4 };
                    solver.advance(h).unwrap();
                    reference.advance(h).unwrap();
                    assert_eq!(bits(&solver.node_v), bits(&reference.node_v));
                    assert_eq!(bits(&solver.cap_i), bits(&reference.cap_i));
                }
            }
        }
    }
}
