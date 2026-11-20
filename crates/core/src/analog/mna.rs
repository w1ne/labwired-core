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
//! ## Small-signal AC
//!
//! [`Solver::ac_analysis`] linearises the circuit about its present solution
//! and solves `(G + jωC)·x = b` at each frequency. `G` is the Newton Jacobian
//! of the operating-point stamp. `C` is not written per device: it is read
//! back from the backward-Euler transient stamp, whose matrix is exactly
//! `G + C/h` for every element whose reactive part is a charge (capacitors,
//! diode, BJT, JFET and MOSFET charge) or a flux (inductors, mutual
//! inductance). So every device that integrates in a transient gets its
//! small-signal admittance from the same code, and a device model added later
//! gets it without an AC stamp of its own. Two step lengths are stamped and
//! compared; a transient term that is not proportional to `1/h` (diode soft
//! recovery, op-amp slew or current limits) is refused by name rather than
//! mis-modelled. A delay is not a capacitance: a transmission line's far-port
//! coupling and a BJT's excess-phase transconductance (`PTF`) are taken out
//! of the stamps and put back per frequency as the exact phasor `e^{-jωTD}`,
//! so neither is approximated by lumped sections or by Weil's filter. `b`
//! holds the `AC <mag> [<phase>]` values of the independent sources; every
//! other source is a small-signal zero.
//!
//! ## Determinism
//!
//! `f64` throughout, `Vec` indices in the hot path, `BTreeMap` only for name
//! lookup at build time, no threads and no wall clock. The same netlist and the
//! same input sequence produce bit-identical voltages on every host.

use super::behavioural::Control as BehaviouralControl;
use super::device;
use super::netlist::{
    AnalogError, Bjt, BjtModel, Circuit, DcSweep, DiodeModel, MosChannelChargeModel, Mosfet,
    NodeRef, SwitchSense, TransferOutput, MACRO_GAIN_RESISTANCE, POWER_SINK_VMIN,
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

/// A complex small-signal value: volts at a node, amps in a branch.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Phasor {
    /// Real part.
    pub re: f64,
    /// Imaginary part.
    pub im: f64,
}

impl Phasor {
    /// `|x|`.
    pub fn magnitude(&self) -> f64 {
        libm::hypot(self.re, self.im)
    }

    /// `arg(x)` in degrees, in `(−180, 180]`.
    pub fn phase_degrees(&self) -> f64 {
        libm::atan2(self.im, self.re).to_degrees()
    }

    /// `20·log10|x|`.
    pub fn db(&self) -> f64 {
        20.0 * libm::log10(self.magnitude())
    }
}

/// The result of [`Solver::ac_analysis`]: one phasor per unknown per
/// frequency.
#[derive(Debug, Clone, PartialEq)]
pub struct AcResponse {
    frequencies: Vec<f64>,
    nodes: usize,
    dim: usize,
    values: Vec<Phasor>,
}

impl AcResponse {
    /// The analysed frequencies, hertz, in the order given.
    pub fn frequencies(&self) -> &[f64] {
        &self.frequencies
    }

    /// Node voltage phasor at frequency index `point`; ground is zero.
    pub fn node_voltage(&self, point: usize, node: NodeRef) -> Phasor {
        node.map_or(Phasor::default(), |node| {
            self.values[point * self.dim + node]
        })
    }

    /// Branch current phasor `index` at frequency index `point`, in the order
    /// [`Solver::branch_current`] uses.
    pub fn branch_current(&self, point: usize, index: usize) -> Phasor {
        self.values[point * self.dim + self.nodes + index]
    }
}

/// What makes a [`NoiseSource`]'s current.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NoiseMechanism {
    /// Johnson noise `4kT/R` of a resistor, a switch or a device's terminal
    /// resistance.
    Thermal,
    /// Shot noise `2q|I|` of a junction current.
    Shot,
    /// FET channel thermal noise `(8/3)kT·|gm|`.
    Channel,
    /// Flicker noise `KF·|I|^AF/f` of a device current: the diode current, the
    /// BJT base current, the JFET drain current, or the MOSFET drain current
    /// over `W·Leff·C'ox²`.
    Flicker,
}

/// One noise current generator between two nodes: white, or `1/f` for
/// [`NoiseMechanism::Flicker`].
#[derive(Debug, Clone, PartialEq)]
pub struct NoiseSource {
    /// ngspice's name for the term: `r1` for a resistor or switch, else
    /// `<term>_<element>`, e.g. `rs_d1`, `id_d1`, `rb_q1`, `ic_q1`, `ib_q1`,
    /// `rd_m1`, `id_j1`, `1overf_m1`.
    pub name: String,
    /// What makes the current.
    pub mechanism: NoiseMechanism,
    /// Current spectral density, A²/Hz; for [`NoiseMechanism::Flicker`] the
    /// density at 1 Hz. See [`Self::density_at`].
    pub density: f64,
    a: NodeRef,
    b: NodeRef,
}

impl NoiseSource {
    /// Current spectral density at `hertz`, A²/Hz.
    pub fn density_at(&self, hertz: f64) -> f64 {
        if self.mechanism == NoiseMechanism::Flicker {
            self.density / hertz
        } else {
            self.density
        }
    }
}

/// The result of [`Solver::transfer_analysis`], ngspice's `.tf` output.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TransferResponse {
    /// Output per unit input: V/V, A/V, V/A or A/A for a voltage or current
    /// output and a `V` or `I` input source. ngspice's `transfer_function`.
    pub transfer: f64,
    /// Small-signal resistance the input source sees, ohms. ngspice's
    /// `<src>#input_impedance`.
    pub input_impedance: f64,
    /// Small-signal resistance seen at the output with the input source at
    /// rest (a `V` shorted, an `I` open), ohms: across the output nodes for
    /// `v(...)`, or in series with the element for `i(...)`. ngspice's
    /// `output_impedance_at_v(...)` and `<element>#output_impedance`.
    pub output_impedance: f64,
}

/// The result of [`Solver::noise_analysis`].
///
/// Densities are power spectral densities: V²/Hz at the output, and V²/Hz
/// or A²/Hz at the input for a `V` or an `I` input source. Take the square
/// root for ngspice's `onoise_spectrum`/`inoise_spectrum`.
#[derive(Debug, Clone, PartialEq)]
pub struct NoiseResponse {
    frequencies: Vec<f64>,
    sources: Vec<NoiseSource>,
    /// Output-referred density of each source, `point * sources + k`.
    contributions: Vec<f64>,
    /// `1/|H|²` from the input source to the output, per point.
    gain_square_inverse: Vec<f64>,
}

impl NoiseResponse {
    /// The analysed frequencies, hertz, in the order given.
    pub fn frequencies(&self) -> &[f64] {
        &self.frequencies
    }

    /// Every noise generator, in the order [`Self::contribution`] uses.
    pub fn sources(&self) -> &[NoiseSource] {
        &self.sources
    }

    /// Output density from source `source` at frequency index `point`, V²/Hz.
    pub fn contribution(&self, point: usize, source: usize) -> f64 {
        self.contributions[point * self.sources.len() + source]
    }

    /// Total output density at frequency index `point`, V²/Hz.
    pub fn output_density(&self, point: usize) -> f64 {
        (0..self.sources.len())
            .map(|source| self.contribution(point, source))
            .sum()
    }

    /// Input-referred density at frequency index `point`: the output density
    /// over the squared gain from the input source.
    pub fn input_density(&self, point: usize) -> f64 {
        self.output_density(point) * self.gain_square_inverse[point]
    }

    /// RMS output noise over the sweep, volts: ngspice's `onoise_total`.
    pub fn output_total(&self) -> f64 {
        libm::sqrt(self.integrate(false))
    }

    /// RMS input-referred noise over the sweep: ngspice's `inoise_total`.
    pub fn input_total(&self) -> f64 {
        libm::sqrt(self.integrate(true))
    }

    /// ngspice's `Nintegrate`: each source's output density is taken as a
    /// power of frequency between two points and integrated exactly; the sum
    /// is over sources and intervals. For the input-referred total ngspice
    /// divides each interval by the squared gain at its upper end only.
    fn integrate(&self, input: bool) -> f64 {
        // ngspice's `N_MINLOG`.
        const FLOOR: f64 = 1e-38;
        let mut total = 0.0;
        for point in 1..self.frequencies.len() {
            let (f0, f1) = (self.frequencies[point - 1], self.frequencies[point]);
            let (ln_f0, ln_f1) = (libm::log(f0), libm::log(f1));
            if ln_f1 == ln_f0 {
                continue;
            }
            let mut interval = 0.0;
            for source in 0..self.sources.len() {
                let n1 = self.contribution(point, source);
                let ln_n0 = libm::log(self.contribution(point - 1, source).max(FLOOR));
                let ln_n1 = libm::log(n1.max(FLOOR));
                let exponent = (ln_n1 - ln_n0) / (ln_f1 - ln_f0);
                interval += if exponent.abs() < FLOOR {
                    n1 * (f1 - f0)
                } else {
                    // `a·(f1^e − f0^e)/e` with `e = exponent + 1`, written
                    // as `a·f0^e·expm1(e·ln(f1/f0))/e`. A `1/f` term has
                    // `e` within rounding of zero, where the difference form
                    // cancels to noise; this one tends to `a·ln(f1/f0)`.
                    let a = libm::exp(ln_n1 - exponent * ln_f1);
                    let exponent = exponent + 1.0;
                    let span = ln_f1 - ln_f0;
                    if exponent.abs() < FLOOR {
                        a * span
                    } else {
                        a * libm::exp(exponent * ln_f0) * libm::expm1(exponent * span) / exponent
                    }
                };
            }
            total += if input {
                interval * self.gain_square_inverse[point]
            } else {
                interval
            };
        }
        total
    }
}

/// The result of [`Solver::dc_analysis`]: the full solution at every point
/// of a `.dc` sweep. Points run inner-fastest: point `k` has inner index
/// `k % inner_len()` and outer index `k / inner_len()`.
#[derive(Debug, Clone, PartialEq)]
pub struct DcResponse {
    inner: Vec<f64>,
    outer: Option<Vec<f64>>,
    nodes: usize,
    dim: usize,
    values: Vec<f64>,
}

impl DcResponse {
    /// Number of solved points.
    pub fn len(&self) -> usize {
        self.values.len() / self.dim.max(1)
    }

    /// True when the sweep has no points.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Number of inner sweep values.
    pub fn inner_len(&self) -> usize {
        self.inner.len()
    }

    /// Inner swept value at `point`: volts, amps, ohms or °C.
    pub fn inner_value(&self, point: usize) -> f64 {
        self.inner[point % self.inner.len()]
    }

    /// Outer swept value at `point`, if the sweep has an outer source.
    pub fn outer_value(&self, point: usize) -> Option<f64> {
        self.outer
            .as_ref()
            .map(|outer| outer[point / self.inner.len()])
    }

    /// Node voltage at `point`; ground is zero.
    pub fn node_voltage(&self, point: usize, node: NodeRef) -> f64 {
        node.map_or(0.0, |node| self.values[point * self.dim + node])
    }

    /// Branch current `index` at `point`, in the order
    /// [`Solver::branch_current`] uses.
    pub fn branch_current(&self, point: usize, index: usize) -> f64 {
        self.values[point * self.dim + self.nodes + index]
    }
}

/// Gaussian elimination with partial pivoting, in place: `rhs` becomes the
/// solution. A pivot below `1e-30` returns its column.
fn solve_dense(matrix: &mut [f64], rhs: &mut [f64], size: usize) -> Result<(), usize> {
    for column in 0..size {
        let pivot_row = (column..size)
            .max_by(|a, b| {
                matrix[a * size + column]
                    .abs()
                    .total_cmp(&matrix[b * size + column].abs())
            })
            .expect("non-empty range");
        if matrix[pivot_row * size + column].abs() < 1e-30 {
            return Err(column);
        }
        if pivot_row != column {
            for index in 0..size {
                matrix.swap(pivot_row * size + index, column * size + index);
            }
            rhs.swap(pivot_row, column);
        }
        let pivot = matrix[column * size + column];
        for row in (column + 1)..size {
            let factor = matrix[row * size + column] / pivot;
            if factor == 0.0 {
                continue;
            }
            for index in column..size {
                matrix[row * size + index] -= factor * matrix[column * size + index];
            }
            rhs[row] -= factor * rhs[column];
        }
    }
    for row in (0..size).rev() {
        let tail: f64 = ((row + 1)..size)
            .map(|index| matrix[row * size + index] * rhs[index])
            .sum();
        rhs[row] = (rhs[row] - tail) / matrix[row * size + row];
    }
    Ok(())
}

/// The realified complex system `[G −ωC; ωC G]` at `hertz`, with each
/// delayed term multiplied by `e^{-jωTD}`, into `system` (`2·dim` square).
fn small_signal_system(
    system: &mut [f64],
    dim: usize,
    g: &[f64],
    c: &[f64],
    couplings: &[DelayedCoupling],
    hertz: f64,
) {
    let size = 2 * dim;
    let omega = 2.0 * std::f64::consts::PI * hertz;
    for row in 0..dim {
        for column in 0..dim {
            let conductance = g[row * dim + column];
            let susceptance = omega * c[row * dim + column];
            system[row * size + column] = conductance;
            system[row * size + dim + column] = -susceptance;
            system[(dim + row) * size + column] = susceptance;
            system[(dim + row) * size + dim + column] = conductance;
        }
    }
    // Each port of a line sees the other's wave `TD` later, and a BJT's
    // transport current lags its B-E voltage by `td`: `e^{-jωTD}`.
    for &DelayedCoupling {
        row,
        column,
        coefficient,
        delay,
        ..
    } in couplings
    {
        let re = coefficient * libm::cos(omega * delay);
        let im = -coefficient * libm::sin(omega * delay);
        system[row * size + column] += re;
        system[row * size + dim + column] -= im;
        system[(dim + row) * size + column] += im;
        system[(dim + row) * size + dim + column] += re;
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Stamp {
    /// t = 0: capacitors open, inductors short.
    OperatingPoint,
    /// One transient step of length `h` seconds under the given rule.
    Transient(f64, Integration),
}

/// Linearised BJT charge: B-E and internal B-C branches can depend on both
/// junction voltages through the Gummel–Poon base-charge factor. The remaining
/// B-C depletion charge follows the external base when `XCJC < 1`. With `RCO`
/// and `QCO`, Kull's epitaxial charge adds to the internal B-C branch and to a
/// fourth branch from the internal base to the collector contact.
struct BjtChargeOp {
    /// B-E charge, internal B-C charge, external-base B-C depletion charge,
    /// and base-to-collector-contact epitaxial charge.
    q: [f64; 4],
    /// `[charge branch][Vbe, internal Vbc, external Vbc, contact Vbcx]`, in
    /// farads.
    dq: [[f64; 4]; 4],
}

fn bjt_charge_op(
    vbe: f64,
    vbc: f64,
    vbc_external: f64,
    vbcx: f64,
    model: BjtModel,
    thermal_voltage: f64,
) -> BjtChargeOp {
    let epi = |voltage: f64| match model.rco {
        Some(_) => device::bjt_epi_charge(voltage, model.qco, model.gamma, thermal_voltage),
        None => (0.0, 0.0),
    };
    let (qbci, cbci) = epi(vbc);
    let (qbcx, cbcx) = epi(vbcx);
    let transit = device::bjt_transit_charge_at_thermal_voltage(vbe, vbc, model, thermal_voltage);
    let (qbe, cbe) =
        device::junction_depletion_charge(vbe, model.cje, model.vje, model.mje, model.fc);
    let (qbc, cbc) = device::junction_depletion_charge(
        vbc,
        model.cjc * model.xcjc,
        model.vjc,
        model.mjc,
        model.fc,
    );
    let (qbc_external, cbc_external) = device::junction_depletion_charge(
        vbc_external,
        model.cjc * (1.0 - model.xcjc),
        model.vjc,
        model.mjc,
        model.fc,
    );
    BjtChargeOp {
        q: [
            qbe + transit.qbe,
            qbc + transit.qbc + qbci,
            qbc_external,
            qbcx,
        ],
        dq: [
            [cbe + transit.dqbe_dvbe, transit.dqbe_dvbc, 0.0, 0.0],
            [transit.dqbc_dvbe, cbc + transit.dqbc_dvbc + cbci, 0.0, 0.0],
            [0.0, 0.0, cbc_external, 0.0],
            [0.0, 0.0, 0.0, cbcx],
        ],
    }
}

/// Weil's second-order excess-phase filter over one step of `h` seconds, as
/// ngspice's `bjtload.c` discretises `td²/3·y'' + td·y' + y = cbe/qb` with
/// group delay `td`. Returns the history term `cc` and the weight `arg3` of
/// the present `cbe`, so the delayed transport current is `cc + arg3·cbe/qb`.
/// `previous` is the last step length; with none yet the two history points
/// are equal and the ratio cancels.
fn excess_phase_step(delay: f64, h: f64, previous: f64, history: [f64; 2]) -> (f64, f64) {
    let arg1 = h / delay;
    let arg2 = 3.0 * arg1;
    let arg1 = arg2 * arg1;
    let denom = 1.0 + arg1 + arg2;
    let ratio = if previous > 0.0 { h / previous } else { 1.0 };
    let cc = (history[0] * (1.0 + ratio + arg2) - history[1] * ratio) / denom;
    (cc, arg1 / denom)
}

/// One BJT's DC linearisation at junction voltages `vbe`, `vbc` in device
/// coordinates, before any excess phase.
fn bjt_op(bjt: &Bjt, vbe: f64, vbc: f64) -> device::BjtOp {
    let model = bjt.operating_model;
    device::bjt_op_split_at_thermal_voltage(
        vbe,
        vbc,
        model.junction_saturation_currents(),
        model.bf,
        model.br,
        model.nf,
        model.nr,
        model.vaf,
        model.var,
        model.ikf,
        model.ikr,
        model.ise,
        model.ne,
        model.isc,
        model.nc,
        model.nkf,
        bjt.thermal_voltage,
    )
}

fn bjt_charge_active(model: BjtModel, branch: usize) -> bool {
    let epi = model.rco.is_some() && model.qco != 0.0;
    match branch {
        0 => model.cje != 0.0 || model.tf != 0.0,
        1 => model.cjc * model.xcjc != 0.0 || model.tr != 0.0 || epi,
        2 => model.cjc * (1.0 - model.xcjc) != 0.0,
        _ => epi,
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
    if mosfet.operating_model.channel_charge_model != MosChannelChargeModel::Meyer {
        return [0.0; 3];
    }
    let Some(tox) = mosfet.operating_model.tox else {
        return [0.0; 3];
    };
    let sign = mosfet.operating_model.polarity.sign();
    let reversed = vds < 0.0;
    let (vgs_eff, vds_eff, vbs_eff) = if reversed {
        (vgs - vds, -vds, vbd)
    } else {
        (vgs, vds, vbs)
    };
    let cox = device::OXIDE_PERMITTIVITY_F_PER_M * mosfet.w * mosfet.effective_length() / tox;
    let mut caps = device::mos_meyer_capacitance_halves(
        vgs_eff,
        vds_eff,
        vbs_eff,
        sign * mosfet.operating_model.vto,
        mosfet.operating_model.gamma,
        mosfet.operating_model.phi,
        cox,
    );
    if reversed {
        caps.swap(0, 1);
    }
    caps
}

fn diode_temperature_parameters(
    model: DiodeModel,
    temperature_celsius: Option<f64>,
) -> (f64, f64, f64) {
    let Some(temperature_celsius) = temperature_celsius else {
        return (model.is, model.isr, device::THERMAL_VOLTAGE);
    };
    let temperature_kelvin = temperature_celsius + 273.15;
    let nominal_kelvin = model.tnom_celsius + 273.15;
    let is = device::diode_saturation_current_at_temperature(
        model.is,
        model.n,
        model.eg,
        model.xti,
        nominal_kelvin,
        temperature_kelvin,
    );
    let isr = device::diode_saturation_current_at_temperature(
        model.isr,
        model.nr,
        model.eg,
        model.xti,
        nominal_kelvin,
        temperature_kelvin,
    );
    (
        is,
        isr,
        device::thermal_voltage_at_temperature(temperature_kelvin),
    )
}

/// Total sidewall saturation current `JSW·PJ` at the instance temperature.
/// ngspice-42 scales it with the `NS` law, and `NS` defaults to 1 there even
/// when the current itself shares the bottom junction's `N`.
fn diode_sidewall_saturation_current(model: DiodeModel, temperature_celsius: Option<f64>) -> f64 {
    let Some(temperature_celsius) = temperature_celsius else {
        return model.jsw;
    };
    if model.jsw == 0.0 {
        return 0.0;
    }
    device::diode_saturation_current_at_temperature(
        model.jsw,
        model.ns.unwrap_or(1.0),
        model.eg,
        model.xti,
        model.tnom_celsius + 273.15,
        temperature_celsius + 273.15,
    )
}

/// Trap-assisted tunnel current from `JTUN·AREA + JTUNSW·PJ`, or `None`
/// when the card has none. ngspice-42 adds it to the junction current, so it
/// passes through the `IK`/`IKR` knee and the `TT` diffusion charge. Its
/// saturation current follows the `IS` temperature law with `NTUN`, `XTITUN`
/// and `KEG·EG`.
fn diode_tunnel_op(
    voltage: f64,
    model: DiodeModel,
    temperature_celsius: Option<f64>,
) -> Option<device::DiodeOp> {
    let saturation = model.jtun + model.jtunsw;
    if saturation <= 0.0 {
        return None;
    }
    let Some(temperature_celsius) = temperature_celsius else {
        return Some(device::diode_tunnel_op(
            voltage,
            saturation,
            model.ntun,
            device::THERMAL_VOLTAGE,
        ));
    };
    let temperature_kelvin = temperature_celsius + 273.15;
    let saturation = device::diode_saturation_current_at_temperature(
        saturation,
        model.ntun,
        model.keg * model.eg,
        model.xtitun,
        model.tnom_celsius + 273.15,
        temperature_kelvin,
    );
    Some(device::diode_tunnel_op(
        voltage,
        saturation,
        model.ntun,
        device::thermal_voltage_at_temperature(temperature_kelvin),
    ))
}

fn diode_model_op(
    voltage: f64,
    model: DiodeModel,
    breakdown: Option<f64>,
    temperature_celsius: Option<f64>,
) -> device::DiodeOp {
    let (is, isr, thermal_voltage) = diode_temperature_parameters(model, temperature_celsius);
    let isw = diode_sidewall_saturation_current(model, temperature_celsius);
    // ngspice-42 merges a sidewall current with no `NS` of its own into the
    // bottom saturation current, so it shares `N`, the reverse and breakdown
    // regions and the IK/IKR knee. With `NS` it has its own characteristic.
    let (bottom_is, sidewall) = match model.ns {
        None => (is + isw, None),
        Some(ns) if isw > 0.0 => (is, Some((isw, ns))),
        Some(_) => (is, None),
    };
    let junction_op = |is: f64, n: f64| match (breakdown, temperature_celsius) {
        (None, None) => device::pn_junction_op(voltage, is, n),
        (Some(xbv), None) => {
            device::diode_op_breakdown(voltage, is, n, xbv, model.nbv.unwrap_or(model.n))
        }
        (None, Some(_)) => {
            device::pn_junction_op_at_thermal_voltage(voltage, is, n, thermal_voltage)
        }
        (Some(xbv), Some(_)) => device::diode_op_breakdown_at_thermal_voltage(
            voltage,
            is,
            n,
            xbv,
            model.nbv.unwrap_or(model.n),
            thermal_voltage,
        ),
    };
    let mut op = junction_op(bottom_is, model.n);
    if let Some((isw, ns)) = sidewall {
        // The sidewall op carries its own GMIN term; the bottom op already
        // has the one GMIN the junction gets.
        let side = junction_op(isw, ns);
        op.id += side.id - device::GMIN * voltage;
        op.gd += side.gd - device::GMIN;
    }
    if let Some(tunnel) = diode_tunnel_op(voltage, model, temperature_celsius) {
        op.id += tunnel.id;
        op.gd += tunnel.gd;
    }
    let recombination = match temperature_celsius {
        None => {
            device::diode_recombination_op(voltage, model.isr, model.nr, model.n, model.vj, model.m)
        }
        Some(_) => device::diode_recombination_op_at_thermal_voltage(
            voltage,
            isr,
            model.nr,
            model.n,
            model.vj,
            model.m,
            thermal_voltage,
        ),
    };

    // ngspice's IK/IKR high-injection correction applies to the junction
    // current (including recombination) but leaves GMIN outside the clamp.
    // Preserve the old arithmetic exactly when the selected knee is omitted,
    // or when the current is too small for the correction to apply.
    let forward = voltage >= -3.0 * model.n * thermal_voltage;
    let knee = if forward { model.ik } else { model.ikr };
    let junction_current = op.id + recombination.id - device::GMIN * voltage;
    let active_current = if forward {
        junction_current > 1e-18
    } else {
        junction_current < -1e-18
    };
    if knee > 0.0 && active_current {
        let root = (junction_current.abs() / knee).sqrt();
        let denominator = 1.0 + root;
        let junction_conductance = op.gd + recombination.gd - device::GMIN;
        let conductance_scale = (1.0 + root / 2.0) / (denominator * denominator);
        return device::DiodeOp {
            id: junction_current / denominator + device::GMIN * voltage,
            gd: junction_conductance * conductance_scale + device::GMIN,
        };
    }

    op.id += recombination.id;
    op.gd += recombination.gd;
    op
}

fn diode_charge_op(voltage: f64, model: DiodeModel, op: device::DiodeOp) -> (f64, f64) {
    let (q_bottom, c_bottom) =
        device::diode_depletion_charge(voltage, model.cjo, model.vj, model.m, model.fc);
    let (q_sidewall, c_sidewall) = diode_sidewall_depletion_charge(voltage, model);
    let (q_depletion, c_depletion) = (q_bottom + q_sidewall, c_bottom + c_sidewall);
    let (q_diffusion, c_diffusion) = if model.vp > 0.0 && model.tt > 0.0 {
        (0.0, 0.0)
    } else {
        device::diode_diffusion_charge(voltage, op, model.tt)
    };
    (q_depletion + q_diffusion, c_depletion + c_diffusion)
}

/// Sidewall depletion charge of `CJSW·PJ` with `PHP`, `MJSW` and `FCS`.
///
/// Below `FCS·PHP` this is the ordinary graded-junction integral. Above it,
/// ngspice-42 `dioload.c` starts the linearised region from `CJSW·F1`, where
/// `F1` is the *bottom* junction's transition charge per farad (from `VJ`,
/// `M`, `FC`), not the sidewall's own. The capacitance stays continuous but
/// the charge steps at `FCS·PHP` unless both junctions share their
/// parameters. This keeps that offset so transient currents match ngspice.
fn diode_sidewall_depletion_charge(voltage: f64, model: DiodeModel) -> (f64, f64) {
    if model.cjsw == 0.0 {
        return (0.0, 0.0);
    }
    let (charge, capacitance) =
        device::junction_depletion_charge(voltage, model.cjsw, model.php, model.mjsw, model.fcs);
    if voltage < model.fcs * model.php {
        return (charge, capacitance);
    }
    let own_transition = device::junction_depletion_charge(
        model.fcs * model.php,
        model.cjsw,
        model.php,
        model.mjsw,
        model.fcs,
    )
    .0;
    let bottom_transition = model.cjsw
        * device::junction_depletion_charge(model.fc * model.vj, 1.0, model.vj, model.m, model.fc)
            .0;
    (charge - own_transition + bottom_transition, capacitance)
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
    model.cjo != 0.0 || model.cjsw != 0.0 || model.tt != 0.0
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
    /// Each BJT's voltage from the internal base to the collector contact in
    /// device coordinates. It differs from `bjt_vbc` only with `RCO`.
    bjt_vbcx: Vec<f64>,
    /// Depletion plus transit charge and current history for each BJT, ordered
    /// B-E, internal B-C, external-base B-C depletion, and base-to-contact
    /// epitaxial charge in device coordinates.
    bjt_q: Vec<[f64; 4]>,
    bjt_i: Vec<[f64; 4]>,
    /// Weil excess-phase history of each BJT with `PTF`: the delayed
    /// transport current `cc + cex/qb` (ngspice's `cexbc`) at the last two
    /// accepted steps, newest first.
    bjt_excess: Vec<[f64; 2]>,
    /// Length of the last accepted transient step, seconds; zero right after
    /// the operating point (ngspice's `CKTdeltaOld[1]`).
    previous_step: f64,
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
    /// Drive-to-output voltage for each op-amp output current limiter.
    output_limit_point: Vec<f64>,
    /// Input-referred error for each slew-limited op-amp gain stage.
    slew_limit_point: Vec<f64>,
    /// The quantities each behavioural source reads, at its linearisation
    /// point: voltage-form sources first, then current-form, each in
    /// [`super::Expression::controls`] order.
    behavioural_point: Vec<Vec<f64>>,
    /// Simulated time the behavioural expressions are evaluated at: the end
    /// of the step being solved, 0 at the operating point.
    behavioural_time: f64,
    /// The first expression that left its domain during the last stamp.
    behavioural_error: Option<AnalogError>,
    /// Set only while `.ac` reads `C` back from the transient stamp: the
    /// BJT excess-phase filter is then left out, and `.ac` applies its
    /// exact phasor instead.
    excess_phase_off: bool,
    /// What each `T` line's ports sent into it, `[time, w1, w2]` with
    /// `w = v + Z0·i` for the current entering the port, oldest first. A run
    /// of equal waves is kept as its two end points, so a line at rest holds
    /// at most two entries however long it rests.
    line_history: Vec<Vec<[f64; 3]>>,
    /// The linearisation points at the start of the step in progress, for a
    /// retry with a cut step; see [`Solver::advance`].
    saved_diode_vj: Vec<f64>,
    saved_bjt_vbe: Vec<f64>,
    saved_bjt_vbc: Vec<f64>,
    saved_bjt_vbcx: Vec<f64>,
    saved_mos_vgs: Vec<f64>,
    saved_mos_vds: Vec<f64>,
    saved_mos_vbd: Vec<f64>,
    saved_mos_vbs: Vec<f64>,
    saved_jfet_vgs: Vec<f64>,
    saved_jfet_vgd: Vec<f64>,
    saved_jfet_vds: Vec<f64>,
    saved_sink_point: Vec<[f64; 3]>,
    saved_output_limit_point: Vec<f64>,
    saved_slew_limit_point: Vec<f64>,
    saved_behavioural_point: Vec<Vec<f64>>,
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

    /// The operating point is in its first phase, where every `off` device is
    /// held cut off (ngspice's `MODEINITFIX`).
    holding_off: bool,
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
        // A SPICE switch starts from its `on`/`off` flag; every other switch
        // starts open.
        let switch_closed = circuit
            .switches
            .iter()
            .map(|switch| {
                switch
                    .threshold
                    .as_ref()
                    .is_some_and(|control| control.initially_closed)
            })
            .collect();
        let capacitors = circuit.capacitors.len();
        let inductors = circuit.inductors.len();
        let nonlinear = circuit.is_nonlinear();
        let has_waveforms = circuit.has_waveforms();
        let diodes = circuit.diodes.len();
        let bjts = circuit.bjts.len();
        let mosfets = circuit.mosfets.len();
        let jfets = circuit.jfets.len();
        let sinks = circuit.power_sinks.len();
        let output_limits = circuit.output_current_limits.len();
        let slew_limits = circuit.slew_limited_vccs.len();
        let lines = circuit.transmission_lines.len();
        let behavioural_point: Vec<Vec<f64>> = circuit
            .behavioural_voltages
            .iter()
            .chain(&circuit.behavioural_currents)
            .map(|source| vec![0.0; source.controls.len()])
            .collect();

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
            bjt_vbcx: vec![0.0; bjts],
            bjt_q: vec![[0.0; 4]; bjts],
            bjt_i: vec![[0.0; 4]; bjts],
            bjt_excess: vec![[0.0; 2]; bjts],
            previous_step: 0.0,
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
            output_limit_point: vec![0.0; output_limits],
            slew_limit_point: vec![0.0; slew_limits],
            saved_behavioural_point: behavioural_point.clone(),
            behavioural_point,
            behavioural_time: 0.0,
            behavioural_error: None,
            excess_phase_off: false,
            line_history: vec![Vec::new(); lines],
            saved_diode_vj: vec![0.0; diodes],
            saved_bjt_vbe: vec![0.0; bjts],
            saved_bjt_vbc: vec![0.0; bjts],
            saved_bjt_vbcx: vec![0.0; bjts],
            saved_mos_vgs: vec![0.0; mosfets],
            saved_mos_vds: vec![0.0; mosfets],
            saved_mos_vbd: vec![0.0; mosfets],
            saved_mos_vbs: vec![0.0; mosfets],
            saved_jfet_vgs: vec![0.0; jfets],
            saved_jfet_vgd: vec![0.0; jfets],
            saved_jfet_vds: vec![0.0; jfets],
            saved_sink_point: vec![[0.0; 3]; sinks],
            saved_output_limit_point: vec![0.0; output_limits],
            saved_slew_limit_point: vec![0.0; slew_limits],
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
            holding_off: false,
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

    /// Index of the named resistor, with or without a temperature law.
    pub fn resistor_index(&self, name: &str) -> Option<usize> {
        let key = name.trim().to_ascii_lowercase();
        self.circuit
            .resistors
            .iter()
            .position(|resistor| resistor.name.to_ascii_lowercase() == key)
    }

    /// Set a resistor's nominal resistance, as a `.dc R...` sweep does. An
    /// NTC or `tc1`/`tc2` law still applies to the new value, as in ngspice.
    /// Returns false when the resulting resistance is zero or not finite.
    pub fn set_resistor(&mut self, index: usize, ohms: f64) -> bool {
        let Some(resistor) = self.circuit.resistors.get_mut(index) else {
            return false;
        };
        let mut changed = resistor.clone();
        changed.ohms = ohms;
        let resistance = changed.resistance_ohms();
        if !resistance.is_finite() || resistance == 0.0 {
            return false;
        }
        if resistor.ohms != ohms {
            resistor.ohms = ohms;
            self.cached_stamp = None;
            self.restart = true;
            self.settled = false;
        }
        true
    }

    /// Lower every temperature-dependent element again at `celsius`, as a
    /// `.dc TEMP` point: the `D`, `Q`, `J` and `M` devices are replaced and
    /// each resistor takes the fresh NTC or `tc1`/`tc2` temperature while
    /// keeping its present nominal value. Sources and switches are untouched.
    fn set_circuit_temperature(&mut self, celsius: f64) -> Result<(), AnalogError> {
        let lowered = self.circuit.at_temperature(celsius)?;
        if lowered.resistors.len() != self.circuit.resistors.len()
            || lowered.diodes.len() != self.circuit.diodes.len()
            || lowered.bjts.len() != self.circuit.bjts.len()
            || lowered.mosfets.len() != self.circuit.mosfets.len()
            || lowered.jfets.len() != self.circuit.jfets.len()
        {
            return Err(AnalogError::Config(
                "`.dc TEMP` lowered a circuit that no longer matches the solver's".to_string(),
            ));
        }
        for (resistor, fresh) in self.circuit.resistors.iter_mut().zip(lowered.resistors) {
            resistor.ntc = fresh.ntc;
            resistor.temperature_coefficients = fresh.temperature_coefficients;
        }
        self.circuit.diodes = lowered.diodes;
        self.circuit.bjts = lowered.bjts;
        self.circuit.mosfets = lowered.mosfets;
        self.circuit.jfets = lowered.jfets;
        self.circuit.temperature_celsius = lowered.temperature_celsius;
        self.cached_stamp = None;
        self.restart = true;
        self.settled = false;
        Ok(())
    }

    /// Index of a named resistor that has an NTC Beta model.
    pub fn thermistor_index(&self, name: &str) -> Option<usize> {
        let key = name.trim().to_ascii_lowercase();
        self.circuit.resistors.iter().position(|resistor| {
            resistor.ntc.is_some() && resistor.name.to_ascii_lowercase() == key
        })
    }

    /// Index of a named resistor whose resistance depends on external temperature.
    pub fn temperature_dependent_resistor_index(&self, name: &str) -> Option<usize> {
        let key = name.trim().to_ascii_lowercase();
        self.circuit.resistors.iter().position(|resistor| {
            (resistor.ntc.is_some() || resistor.temperature_coefficients.is_some())
                && resistor.name.to_ascii_lowercase() == key
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

    /// Set an NTC or temperature-coefficient resistor's external temperature.
    /// A changed value invalidates the linear stamp so its resistance is
    /// rebuilt on the next step. Returns false for a fixed resistor, an invalid
    /// temperature, or a model whose resulting resistance is not finite and
    /// positive.
    pub fn set_resistor_temperature(&mut self, index: usize, celsius: f64) -> bool {
        if !celsius.is_finite() || celsius <= -273.15 {
            return false;
        }
        let Some(resistor) = self.circuit.resistors.get_mut(index) else {
            return false;
        };
        let (ntc, temperature_coefficients) =
            match (resistor.ntc, resistor.temperature_coefficients) {
                (Some(mut ntc), None) => {
                    ntc.temperature_celsius = celsius;
                    (Some(ntc), None)
                }
                (None, Some(mut model)) => {
                    model.temperature_celsius = celsius;
                    (None, Some(model))
                }
                // Parsing rejects a resistor that has both temperature laws.
                // Treat a fixed resistor (or an invalid hand-built circuit) as
                // non-routable.
                _ => return false,
            };
        let resistance = ntc
            .map(|model| model.resistance_ohms(resistor.ohms))
            .or_else(|| temperature_coefficients.map(|model| model.resistance_ohms(resistor.ohms)))
            .unwrap_or(f64::NAN);
        if !resistance.is_finite() || resistance <= 0.0 {
            return false;
        }
        if resistor.ntc != ntc || resistor.temperature_coefficients != temperature_coefficients {
            resistor.ntc = ntc;
            resistor.temperature_coefficients = temperature_coefficients;
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
            .filter(|(_, s)| s.is_routed() && s.ctrl.to_ascii_lowercase() == key)
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
    /// Voltage- and current-controlled switches ignore this: their state is
    /// refreshed from the sensed quantity at the start of every step.
    pub fn set_switch(&mut self, index: usize, closed: bool) {
        if !self.circuit.switches[index].is_routed() {
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
            if let Some(control) = &self.circuit.switches[index].threshold {
                // SPICE `SW`/`CSW`: signed, with the band centred on the
                // threshold, and the state held inside the band.
                let sensed = match control.sense {
                    SwitchSense::Voltage { pos, neg } => node_diff(&self.node_v, pos, neg),
                    SwitchSense::Current { branch, .. } => self.branch_i[branch],
                };
                let closed_now = self.switch_closed[index];
                let desired = if sensed > control.threshold + control.hysteresis {
                    true
                } else if sensed < control.threshold - control.hysteresis {
                    false
                } else {
                    closed_now
                };
                if desired != closed_now {
                    self.switch_closed[index] = desired;
                    changed = true;
                }
                continue;
            }
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
    ///
    /// Under `.tran ... uic` there is no operating point: like SPICE's
    /// `MODEUIC`, every node and branch starts at zero and only the `.ic` /
    /// `ic=` overrides seed the reactive state.
    pub fn solve_operating_point(&mut self) -> Result<(), AnalogError> {
        let passes = if self.circuit.uic {
            self.node_v.fill(0.0);
            self.branch_i.fill(0.0);
            0
        } else {
            // Voltage-controlled contacts depend on the solved node voltages,
            // so the OP may need a few passes until the contacts stop flipping.
            8
        };
        for _ in 0..passes {
            self.cached_stamp = None;
            self.settled = false;
            if self.nonlinear {
                self.save_linearisation();
                if let Err(error) = self.operating_point_newton() {
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
        // A line at its operating point has carried the same two waves for
        // ever.
        for index in 0..self.circuit.transmission_lines.len() {
            let [w1, w2] = self.line_waves(index, &self.node_v, &self.branch_i);
            self.line_history[index] = vec![[0.0, w1, w2]];
        }
        self.restart = true;
        Ok(())
    }

    /// `v + Z0·i` at each port of `T` line `index`: what the port sends into
    /// the line, and what the other port sees `TD` later.
    fn line_waves(&self, index: usize, node_v: &[f64], branch_i: &[f64]) -> [f64; 2] {
        let line = &self.circuit.transmission_lines[index];
        let branch = self.circuit.transmission_line_branch(index);
        [
            node_diff(node_v, line.p1, line.n1) + line.z0 * branch_i[branch],
            node_diff(node_v, line.p2, line.n2) + line.z0 * branch_i[branch + 1],
        ]
    }

    /// The history terms of `T` line `index` for a step of `h` to `time`:
    /// `[alpha, e1, e2]`, where port 1 obeys `v1 − Z0·i1 − alpha·w2 = e1`
    /// and port 2 the mirror image.
    ///
    /// The wave wanted is the one sent at `time − TD`. With `TD >= h` that
    /// instant is at or before the last accepted point, so `alpha` is 0 and
    /// `e` is read from the history by linear interpolation. With `TD < h`
    /// it falls inside the step: the wave is interpolated between the last
    /// accepted point and the step's own unknown end point, which puts the
    /// weight `alpha = 1 − TD/h` on the unknown. At the operating point
    /// `alpha` is 1 and `e` is 0: the two ports are wired together.
    fn line_terms(&self, index: usize, stamp: Stamp, time: f64) -> [f64; 3] {
        let delay = self.circuit.transmission_lines[index].delay;
        let alpha = line_alpha(delay, stamp);
        if stamp == Stamp::OperatingPoint {
            return [alpha, 0.0, 0.0];
        }
        let history = &self.line_history[index];
        let newest = history[history.len() - 1];
        if alpha > 0.0 {
            return [alpha, (1.0 - alpha) * newest[2], (1.0 - alpha) * newest[1]];
        }
        let wanted = time - delay;
        // The newest entry holds from its own time on: a settled line stops
        // recording, and a wave wanted from that stretch is the settled one.
        let after = history.partition_point(|entry| entry[0] < wanted);
        if after == history.len() {
            return [0.0, newest[2], newest[1]];
        }
        if after == 0 {
            return [0.0, history[0][2], history[0][1]];
        }
        let (old, new) = (history[after - 1], history[after]);
        let weight = (wanted - old[0]) / (new[0] - old[0]);
        [
            0.0,
            old[2] + weight * (new[2] - old[2]),
            old[1] + weight * (new[1] - old[1]),
        ]
    }

    /// The matrix rows and columns of every `T` line, which depend only on
    /// the stamp.
    fn stamp_lines(&mut self, stamp: Stamp) {
        let dim = self.dim();
        for (index, line) in self.circuit.transmission_lines.iter().enumerate() {
            let alpha = line_alpha(line.delay, stamp);
            let row1 = self.nodes + self.circuit.transmission_line_branch(index);
            let row2 = row1 + 1;
            let ports = [
                (row1, line.p1, line.n1, row2, line.p2, line.n2),
                (row2, line.p2, line.n2, row1, line.p1, line.n1),
            ];
            for (row, p, n, far_row, far_p, far_n) in ports {
                let matrix = &mut self.matrix;
                // The branch current enters the port at `+` and leaves at `-`.
                add(matrix, dim, p, Some(row), 1.0);
                add(matrix, dim, n, Some(row), -1.0);
                // v − Z0·i − alpha·(v_far + Z0·i_far) = e.
                add(matrix, dim, Some(row), p, 1.0);
                add(matrix, dim, Some(row), n, -1.0);
                matrix[row * dim + row] -= line.z0;
                add(matrix, dim, Some(row), far_p, -alpha);
                add(matrix, dim, Some(row), far_n, alpha);
                matrix[row * dim + far_row] -= alpha * line.z0;
            }
        }
    }

    /// The right-hand side of every `T` line's two rows: the delayed waves.
    fn stamp_line_history(&mut self, stamp: Stamp) {
        let time = match stamp {
            Stamp::OperatingPoint => 0.0,
            Stamp::Transient(h, _) => self.time + h,
        };
        for index in 0..self.circuit.transmission_lines.len() {
            let [_, e1, e2] = self.line_terms(index, stamp, time);
            let row = self.nodes + self.circuit.transmission_line_branch(index);
            self.rhs[row] = e1;
            self.rhs[row + 1] = e2;
        }
    }

    /// Record what each `T` line's ports sent at the end of the step just
    /// solved, and drop what no later step can ask for. Returns true when
    /// every line was at rest before the step and still is.
    fn record_line_waves(&mut self, time: f64) -> bool {
        let previous_time = self.time;
        let mut at_rest = true;
        for index in 0..self.circuit.transmission_lines.len() {
            let nodes = self.nodes;
            let [w1, w2] = self.line_waves(index, &self.solution[..nodes], &self.solution[nodes..]);
            let delay = self.circuit.transmission_lines[index].delay;
            let history = &mut self.line_history[index];
            let same = |entry: &[f64; 3], w1: f64, w2: f64| {
                entry[1].to_bits() == w1.to_bits() && entry[2].to_bits() == w2.to_bits()
            };
            let newest = history[history.len() - 1];
            at_rest &= history.iter().all(|entry| same(entry, w1, w2));
            // A settled line skipped its records: the newest wave held up to
            // the start of this step.
            let extend = |history: &mut Vec<[f64; 3]>, time: f64, w1: f64, w2: f64| {
                let count = history.len();
                if count >= 2
                    && same(&history[count - 2], w1, w2)
                    && same(&history[count - 1], w1, w2)
                {
                    history[count - 1][0] = time;
                } else {
                    history.push([time, w1, w2]);
                }
            };
            if newest[0] < previous_time {
                extend(history, previous_time, newest[1], newest[2]);
            }
            extend(history, time, w1, w2);
            // Keep the newest entry at or before the oldest instant a step
            // can still ask for, and everything after it.
            let oldest = time - delay;
            let keep_from = history
                .partition_point(|entry| entry[0] <= oldest)
                .saturating_sub(1);
            history.drain(..keep_from);
        }
        at_rest
    }

    /// `.ic V(node)=v` and element `ic=` override the operating point.
    ///
    /// The operating point is solved first and then overridden, rather than
    /// solved subject to the constraints: the two agree for every netlist where
    /// `.ic` is used for what it is for (seeding a reactive element), and the
    /// override order — node `.ic` first, then per-element `ic=` — is stated
    /// here so a netlist that sets both is not ambiguous.
    ///
    /// Under `uic` a semiconductor instance `ic=` replaces the node voltages
    /// its charge history is evaluated at, as ngspice's `MODEINITTRAN` with
    /// `MODEUIC` does: the stored charge is `q(ic)`, so the first step draws
    /// `(q(v) - q(ic)) / h` through the junction.
    fn apply_initial_conditions(&mut self) {
        let uic = self.circuit.uic;
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
            let charge_model = diode.operating_model;
            if diode_charge_active(charge_model) {
                self.diode_vj[index] = match diode.ic {
                    Some(vd) if uic => vd,
                    _ => node_diff(&self.node_v, diode.junction_anode, diode.cathode),
                };
                let op = diode_model_op(
                    self.diode_vj[index],
                    diode.model,
                    diode.breakdown,
                    diode.temperature_celsius,
                );
                self.diode_q[index] = diode_charge_op(self.diode_vj[index], charge_model, op).0;
                self.diode_i[index] = 0.0;
                if charge_model.vp > 0.0 && charge_model.tt > 0.0 {
                    self.diode_recovery_state[index] = -charge_model.tt * op.id;
                    self.diode_recovery_rate[index] = 0.0;
                }
            }
        }
        for index in 0..self.circuit.bjts.len() {
            let bjt = &self.circuit.bjts[index];
            let sign = bjt.model.polarity.sign();
            let (vbe, vbc, vbc_external, vbcx) = match bjt.ic {
                // bjtload.c: vbe = type·icVBE, vbc = vbx = vbe − type·icVCE.
                Some([ic_vbe, ic_vce]) if uic => {
                    let vbe = sign * ic_vbe;
                    let vbc = vbe - sign * ic_vce;
                    (vbe, vbc, vbc, vbc)
                }
                _ => (
                    sign * node_diff(&self.node_v, bjt.junction_b, bjt.junction_e),
                    sign * node_diff(&self.node_v, bjt.junction_b, bjt.junction_c),
                    sign * node_diff(&self.node_v, bjt.b, bjt.junction_c),
                    sign * node_diff(&self.node_v, bjt.junction_b, bjt.junction_cx),
                ),
            };
            self.bjt_q[index] = bjt_charge_op(
                vbe,
                vbc,
                vbc_external,
                vbcx,
                bjt.operating_model,
                bjt.thermal_voltage,
            )
            .q;
            self.bjt_i[index] = [0.0; 4];
            // ngspice MODEINITTRAN: both history points hold the DC cbe/qb.
            let transport =
                device::bjt_forward_transport(vbe, vbc, bjt.operating_model, bjt.thermal_voltage);
            let delayed = transport.cbe / transport.qb;
            self.bjt_excess[index] = [delayed, delayed];
        }
        self.previous_step = 0.0;
        for index in 0..self.circuit.mosfets.len() {
            let mosfet = &self.circuit.mosfets[index];
            let sign = mosfet.operating_model.polarity.sign();
            let node_v = &self.node_v;
            // Terminal volts the charges start from: the intrinsic nodes, or
            // under uic the instance `ic=` (mos1load.c MODEINITJCT reads
            // `type·icVDS` etc.; MOS1getic fills an omitted one from the
            // external nodes). Under uic ngspice does NOT replace an all-zero
            // ic with vgs = VTO, vbs = −1: that needs MODETRAN, MODEDCOP or
            // no MODEUIC, and the uic start runs in MODETRANOP | MODEUIC.
            let [vgs, vds, vbs, vgd, vgb, vbd] = if uic {
                let ic = mosfet.ic;
                let vds = ic[0].unwrap_or_else(|| node_diff(node_v, mosfet.d, mosfet.s));
                let vgs = ic[1].unwrap_or_else(|| node_diff(node_v, mosfet.g, mosfet.s));
                let vbs = ic[2].unwrap_or_else(|| node_diff(node_v, mosfet.bulk, mosfet.s));
                [vgs, vds, vbs, vgs - vds, vgs - vbs, vbs - vds]
            } else {
                let (g, d, s, b) = (mosfet.g, mosfet.junction_d, mosfet.junction_s, mosfet.bulk);
                [
                    node_diff(node_v, g, s),
                    node_diff(node_v, d, s),
                    node_diff(node_v, b, s),
                    node_diff(node_v, g, d),
                    node_diff(node_v, g, b),
                    node_diff(node_v, b, d),
                ]
            };
            let voltages = [sign * vbd, sign * vbs];
            self.mos_bulk_q[index] = core::array::from_fn(|junction| {
                mosfet
                    .bulk_junction_charge(voltages[junction], junction == 0)
                    .0
            });
            self.mos_bulk_i[index] = [0.0; 2];

            let voltages = [vgs, vgd, vgb];
            let capacitances = mosfet.overlap_capacitances();
            self.mos_overlap_q[index] =
                core::array::from_fn(|junction| capacitances[junction] * voltages[junction]);
            self.mos_overlap_i[index] = [0.0; 3];

            let channel_caps = mos_channel_capacitance_halves(
                mosfet,
                sign * vgs,
                sign * vds,
                sign * vbd,
                sign * vbs,
            );
            self.mos_channel_v[index] = voltages;
            self.mos_channel_q[index] =
                core::array::from_fn(|junction| 2.0 * channel_caps[junction] * voltages[junction]);
            self.mos_channel_i[index] = [0.0; 3];
            self.mos_channel_cap_half[index] = channel_caps;

            if mosfet.operating_model.channel_charge_model == MosChannelChargeModel::WardDutton {
                self.mos_conserving_q[index] = device::mos_ward_dutton_channel_charge(
                    mosfet,
                    sign * vgs,
                    sign * vds,
                    sign * vbd,
                    sign * vbs,
                )
                .q;
                self.mos_conserving_i[index] = [0.0; 4];
            }
        }
        for index in 0..self.circuit.jfets.len() {
            let jfet = &self.circuit.jfets[index];
            let sign = jfet.model.polarity.sign();
            let model = jfet.operating_model;
            let (vgs, vgd) = match jfet.ic {
                // jfetload.c: vgs = type·icVGS, vgd = vgs − type·icVDS.
                Some([ic_vds, ic_vgs]) if uic => (sign * ic_vgs, sign * (ic_vgs - ic_vds)),
                _ => (
                    sign * node_diff(&self.node_v, jfet.g, jfet.junction_s),
                    sign * node_diff(&self.node_v, jfet.g, jfet.junction_d),
                ),
            };
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
        self.saved_bjt_vbcx.copy_from_slice(&self.bjt_vbcx);
        self.saved_mos_vgs.copy_from_slice(&self.mos_vgs);
        self.saved_mos_vds.copy_from_slice(&self.mos_vds);
        self.saved_mos_vbd.copy_from_slice(&self.mos_vbd);
        self.saved_mos_vbs.copy_from_slice(&self.mos_vbs);
        self.saved_jfet_vgs.copy_from_slice(&self.jfet_vgs);
        self.saved_jfet_vgd.copy_from_slice(&self.jfet_vgd);
        self.saved_jfet_vds.copy_from_slice(&self.jfet_vds);
        self.saved_sink_point.copy_from_slice(&self.sink_point);
        self.saved_output_limit_point
            .copy_from_slice(&self.output_limit_point);
        self.saved_slew_limit_point
            .copy_from_slice(&self.slew_limit_point);
        self.saved_behavioural_point
            .clone_from(&self.behavioural_point);
    }

    fn restore_linearisation(&mut self) {
        self.diode_vj.copy_from_slice(&self.saved_diode_vj);
        self.bjt_vbe.copy_from_slice(&self.saved_bjt_vbe);
        self.bjt_vbc.copy_from_slice(&self.saved_bjt_vbc);
        self.bjt_vbcx.copy_from_slice(&self.saved_bjt_vbcx);
        self.mos_vgs.copy_from_slice(&self.saved_mos_vgs);
        self.mos_vds.copy_from_slice(&self.saved_mos_vds);
        self.mos_vbd.copy_from_slice(&self.saved_mos_vbd);
        self.mos_vbs.copy_from_slice(&self.saved_mos_vbs);
        self.jfet_vgs.copy_from_slice(&self.saved_jfet_vgs);
        self.jfet_vgd.copy_from_slice(&self.saved_jfet_vgd);
        self.jfet_vds.copy_from_slice(&self.saved_jfet_vds);
        self.sink_point.copy_from_slice(&self.saved_sink_point);
        self.output_limit_point
            .copy_from_slice(&self.saved_output_limit_point);
        self.slew_limit_point
            .copy_from_slice(&self.saved_slew_limit_point);
        self.behavioural_point
            .clone_from(&self.saved_behavioural_point);
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
            let charge_model = diode.operating_model;
            if !diode_charge_active(charge_model) {
                continue;
            }
            let v_new = node_diff(
                &self.solution[..self.nodes],
                diode.junction_anode,
                diode.cathode,
            );
            let op = diode_model_op(
                v_new,
                diode.model,
                diode.breakdown,
                diode.temperature_celsius,
            );
            let (q_new, _) = diode_charge_op(v_new, charge_model, op);
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
            let charge_model = diode.operating_model;
            if charge_model.vp <= 0.0 || charge_model.tt <= 0.0 {
                continue;
            }
            let v_new = node_diff(
                &self.solution[..self.nodes],
                diode.junction_anode,
                diode.cathode,
            );
            let op = diode_model_op(
                v_new,
                diode.model,
                diode.breakdown,
                diode.temperature_celsius,
            );
            let recovery = diode_recovery_step(
                charge_model,
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
            let model = bjt.operating_model;
            let nodes = &self.solution[..self.nodes];
            let voltages = [
                sign * node_diff(nodes, bjt.junction_b, bjt.junction_e),
                sign * node_diff(nodes, bjt.junction_b, bjt.junction_c),
                sign * node_diff(nodes, bjt.b, bjt.junction_c),
                sign * node_diff(nodes, bjt.junction_b, bjt.junction_cx),
            ];
            let charge = bjt_charge_op(
                voltages[0],
                voltages[1],
                voltages[2],
                voltages[3],
                bjt.operating_model,
                bjt.thermal_voltage,
            );
            for junction in 0..4 {
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
            let delay = device::bjt_excess_phase_delay(bjt.model);
            if delay > 0.0 {
                let transport = device::bjt_forward_transport(
                    voltages[0],
                    voltages[1],
                    model,
                    bjt.thermal_voltage,
                );
                let (history, arg3) =
                    excess_phase_step(delay, h, self.previous_step, self.bjt_excess[index]);
                let delayed = history + transport.cbe * arg3 / transport.qb;
                let [newest, older] = self.bjt_excess[index];
                self.settled &= newest.to_bits() == delayed.to_bits()
                    && older.to_bits() == newest.to_bits()
                    && self.previous_step.to_bits() == h.to_bits();
                self.bjt_excess[index] = [delayed, newest];
            }
        }
        self.previous_step = h;
        for index in 0..self.circuit.jfets.len() {
            let jfet = &self.circuit.jfets[index];
            let sign = jfet.model.polarity.sign();
            let model = jfet.operating_model;
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
            for (junction, voltage) in voltages.iter().copied().enumerate() {
                let drain = junction == 0;
                if mosfet.bulk_junction_capacitances(drain) == [0.0, 0.0] {
                    continue;
                }
                let q_new = mosfet.bulk_junction_charge(voltage, drain).0;
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
            for (junction, (voltage, capacitance)) in voltages.iter().zip(capacitances).enumerate()
            {
                let q_new = capacitance * voltage;
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

            if mosfet.operating_model.channel_charge_model == MosChannelChargeModel::WardDutton {
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

        self.settled &= self.record_line_waves(time);

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

    /// Run the netlist's own `.dc` sweep; see [`Solver::dc_analysis`].
    pub fn dc_sweep(&self) -> Result<DcResponse, AnalogError> {
        let sweep = self.circuit.dc_sweep.as_ref().ok_or_else(|| {
            AnalogError::Config(
                "netlist has no `.dc <src> <start> <stop> <incr>` sweep".to_string(),
            )
        })?;
        self.dc_analysis(sweep)
    }

    /// DC transfer curve: the operating point at every value of `sweep`, as
    /// ngspice's `.dc` computes it. A swept `V` or `I` takes the value as its
    /// DC level; a swept `R` takes it as its nominal resistance, so an NTC or
    /// `tc1`/`tc2` law still scales it. A swept `TEMP` (°C) lowers every
    /// temperature-dependent device and resistor again at that temperature,
    /// as a `.temp` card would, unless the element sets its own `temp=`; this
    /// needs the circuit to come from a parsed deck. Each point starts Newton
    /// from the one before, so a sweep through a diode knee converges like a
    /// source ramp.
    /// Every other source keeps its present value; `.tran ... uic` does not
    /// apply, because a DC sweep has no initial transient.
    ///
    /// The sweep runs on a copy of the solver: `self` and its transient state
    /// are not touched.
    pub fn dc_analysis(&self, sweep: &DcSweep) -> Result<DcResponse, AnalogError> {
        #[derive(Clone, Copy)]
        enum Swept {
            Voltage(usize),
            Current(usize),
            Resistor(usize),
            Temperature,
        }
        let resolve = |name: &str| {
            if name.eq_ignore_ascii_case("temp") {
                return Ok(Swept::Temperature);
            }
            self.voltage_source_index(name)
                .map(Swept::Voltage)
                .or_else(|| self.current_source_index(name).map(Swept::Current))
                .or_else(|| self.resistor_index(name).map(Swept::Resistor))
                .ok_or_else(|| {
                    AnalogError::Config(format!(
                        "`.dc` sweeps `{name}`, but the netlist has no such `V`, `I` or `R` element"
                    ))
                })
        };
        let inner_source = resolve(&sweep.inner.source)?;
        let outer_source = sweep
            .outer
            .as_ref()
            .map(|outer| resolve(&outer.source))
            .transpose()?;
        let inner = sweep.inner.values();
        let outer = sweep.outer.as_ref().map(|outer| outer.values());

        let mut work = self.clone();
        work.circuit.uic = false;
        let set = |work: &mut Self, swept: Swept, value: f64| match swept {
            Swept::Voltage(index) => {
                work.set_voltage_source(index, value);
                Ok(())
            }
            Swept::Current(index) => {
                work.set_current_source(index, value);
                Ok(())
            }
            Swept::Resistor(index) => {
                if work.set_resistor(index, value) {
                    Ok(())
                } else {
                    Err(AnalogError::Config(format!(
                        "`.dc` sets resistor `{}` to {value} ohm, which gives a zero or non-finite resistance",
                        work.circuit.resistors[index].name
                    )))
                }
            }
            Swept::Temperature => work.set_circuit_temperature(value),
        };
        let dim = self.dim();
        let outer_values = outer.clone().unwrap_or_else(|| vec![0.0]);
        let mut values = Vec::with_capacity(inner.len() * outer_values.len() * dim);
        for &outer_value in &outer_values {
            if let Some(swept) = outer_source {
                set(&mut work, swept, outer_value)?;
            }
            for &inner_value in &inner {
                set(&mut work, inner_source, inner_value)?;
                work.solve_operating_point()?;
                values.extend_from_slice(&work.node_v);
                values.extend_from_slice(&work.branch_i);
            }
        }
        Ok(DcResponse {
            inner,
            outer,
            nodes: self.nodes,
            dim,
            values,
        })
    }

    /// Run the netlist's own `.ac` sweep; see [`Solver::ac_analysis`].
    pub fn ac_sweep(&mut self) -> Result<AcResponse, AnalogError> {
        let sweep = self.circuit.ac_sweep.ok_or_else(|| {
            AnalogError::Config("netlist has no `.ac dec|oct|lin ...` sweep".to_string())
        })?;
        self.ac_analysis(&sweep.frequencies())
    }

    /// Small-signal AC analysis at each of `frequencies` (hertz), linearised
    /// about the solver's present solution — the DC operating point on a
    /// fresh solver. The stimulus is every source's `AC <mag> [<phase>]`.
    ///
    /// The transient state is left exactly as it was: the next
    /// [`Solver::advance`] produces the same bits it would have without
    /// this call.
    pub fn ac_analysis(&mut self, frequencies: &[f64]) -> Result<AcResponse, AnalogError> {
        if let Some(bad) = frequencies.iter().find(|f| !(f.is_finite() && **f >= 0.0)) {
            return Err(AnalogError::Config(format!(
                "`.ac` frequency {bad} Hz must be finite and not negative"
            )));
        }
        let dim = self.dim();
        let (g, c, couplings, excitation) = self.linearised(|solver| {
            let couplings = solver.delayed_couplings();
            let (g, c) = solver.ac_matrices(&couplings)?;
            Ok((g, c, couplings, solver.ac_excitation()))
        })?;

        let mut values = Vec::with_capacity(frequencies.len() * dim);
        let size = 2 * dim;
        let mut system = vec![0.0; size * size];
        for &hertz in frequencies {
            small_signal_system(&mut system, dim, &g, &c, &couplings, hertz);
            let mut rhs: Vec<f64> = excitation
                .iter()
                .map(|p| p.re)
                .chain(excitation.iter().map(|p| p.im))
                .collect();
            solve_dense(&mut system, &mut rhs, size)
                .map_err(|row| AnalogError::Singular { row: row % dim })?;
            values.extend((0..dim).map(|k| Phasor {
                re: rhs[k],
                im: rhs[dim + k],
            }));
        }
        Ok(AcResponse {
            frequencies: frequencies.to_vec(),
            nodes: self.nodes,
            dim,
            values,
        })
    }

    /// Run the netlist's own `.noise` analysis; see
    /// [`Solver::noise_analysis`].
    pub fn noise_sweep(&mut self) -> Result<NoiseResponse, AnalogError> {
        let noise = self.circuit.noise_sweep.clone().ok_or_else(|| {
            AnalogError::Config("netlist has no `.noise v(<out>) <src> ...` analysis".to_string())
        })?;
        self.noise_analysis(
            &noise.output,
            noise.reference.as_deref(),
            &noise.input,
            &noise.sweep.frequencies(),
        )
    }

    /// Small-signal noise analysis, as ngspice's `.noise`: every noise
    /// generator of the circuit linearised about the solver's present
    /// solution (the DC operating point on a fresh solver), carried to
    /// `v(output) − v(reference)` at each of `frequencies` (hertz). The
    /// generators are uncorrelated, so their output powers add.
    ///
    /// The input-referred density divides by the squared gain from `input`,
    /// an independent `V` or `I` source, driven at unit magnitude whatever
    /// its `AC` value, as ngspice does. Thermal noise uses each element's own
    /// temperature: a device's or resistor's `temp=`, else `.temp`, else the
    /// engine's fixed 300 K.
    ///
    /// The transient state is left exactly as it was, as for
    /// [`Solver::ac_analysis`].
    pub fn noise_analysis(
        &mut self,
        output: &str,
        reference: Option<&str>,
        input: &str,
        frequencies: &[f64],
    ) -> Result<NoiseResponse, AnalogError> {
        if let Some(bad) = frequencies.iter().find(|f| !(f.is_finite() && **f > 0.0)) {
            return Err(AnalogError::Config(format!(
                "`.noise` frequency {bad} Hz must be finite and positive"
            )));
        }
        let node = |name: &str| {
            self.circuit.node(name).ok_or_else(|| {
                AnalogError::Config(format!(
                    "`.noise` output node `{name}` is not in the netlist"
                ))
            })
        };
        let output = node(output)?;
        let reference = reference.map(node).transpose()?.flatten();
        if output == reference {
            return Err(AnalogError::Config(
                "`.noise` output and reference are the same node".to_string(),
            ));
        }
        let dim = self.dim();
        let mut stimulus = vec![0.0; dim];
        if let Some(index) = self.voltage_source_index(input) {
            stimulus[self.nodes + index] = 1.0;
        } else if let Some(index) = self.current_source_index(input) {
            let source = &self.circuit.current_sources[index];
            inject(&mut stimulus, source.p, -1.0);
            inject(&mut stimulus, source.n, 1.0);
        } else {
            return Err(AnalogError::Config(format!(
                "`.noise` input `{input}` is not an independent `V` or `I` source"
            )));
        }
        let (g, c, couplings, sources) = self.linearised(|solver| {
            let couplings = solver.delayed_couplings();
            let (g, c) = solver.ac_matrices(&couplings)?;
            Ok((g, c, couplings, solver.noise_sources()))
        })?;

        // ngspice's `N_MINGAIN`.
        const MIN_GAIN: f64 = 1e-20;
        let size = 2 * dim;
        let mut system = vec![0.0; size * size];
        let mut transpose = vec![0.0; size * size];
        let mut contributions = Vec::with_capacity(frequencies.len() * sources.len());
        let mut gain_square_inverse = Vec::with_capacity(frequencies.len());
        for &hertz in frequencies {
            small_signal_system(&mut system, dim, &g, &c, &couplings, hertz);
            for row in 0..size {
                for column in 0..size {
                    transpose[column * size + row] = system[row * size + column];
                }
            }
            // The realified transpose is the conjugate transpose `Yᴴ`, so the
            // solution is `w = conj(Y⁻ᵀ·e)`: one solve gives the transfer
            // from a current injected at any node pair to the output.
            let mut adjoint = vec![0.0; size];
            for (node, sign) in [(output, 1.0), (reference, -1.0)] {
                if let Some(node) = node {
                    adjoint[node] = sign;
                }
            }
            solve_dense(&mut transpose, &mut adjoint, size)
                .map_err(|row| AnalogError::Singular { row: row % dim })?;
            let at = |node: NodeRef| node.map_or((0.0, 0.0), |n| (adjoint[n], adjoint[dim + n]));
            let (mut gain_re, mut gain_im) = (0.0, 0.0);
            for (k, &value) in stimulus.iter().enumerate() {
                gain_re += adjoint[k] * value;
                gain_im += adjoint[dim + k] * value;
            }
            gain_square_inverse.push(1.0 / (gain_re * gain_re + gain_im * gain_im).max(MIN_GAIN));
            for source in &sources {
                let (a_re, a_im) = at(source.a);
                let (b_re, b_im) = at(source.b);
                let (re, im) = (a_re - b_re, a_im - b_im);
                contributions.push((re * re + im * im) * source.density_at(hertz));
            }
        }
        Ok(NoiseResponse {
            frequencies: frequencies.to_vec(),
            sources,
            contributions,
            gain_square_inverse,
        })
    }

    /// Run the netlist's own `.tf` card; see [`Solver::transfer_analysis`].
    pub fn transfer_function(&mut self) -> Result<TransferResponse, AnalogError> {
        let card = self.circuit.transfer_function.clone().ok_or_else(|| {
            AnalogError::Config("netlist has no `.tf v(<out>)|i(<element>) <src>` card".to_string())
        })?;
        self.transfer_analysis(&card.output, &card.input)
    }

    /// DC small-signal transfer function, as ngspice's `.tf`: the circuit
    /// linearised about the solver's present solution (the DC operating
    /// point on a fresh solver) and driven by a unit change of `input`, an
    /// independent `V` or `I` source. One solve gives the gain and the input
    /// resistance; a second, driven at the output with the input at rest,
    /// gives the output resistance.
    ///
    /// Two differences from ngspice-42 are deliberate. An `i(<element>)`
    /// output's resistance is `−1/I` for the branch current `I` a unit
    /// voltage in series with the element drives, the formula ngspice uses
    /// for a `V` input; ngspice's `tfanal.c` takes `1/max(1e-20, I)` and so
    /// prints 1e20 whenever that current is negative. A resistance with no
    /// current to define it is `f64::INFINITY`, where ngspice prints 1e20.
    ///
    /// The transient state is left exactly as it was, as for
    /// [`Solver::ac_analysis`].
    pub fn transfer_analysis(
        &mut self,
        output: &TransferOutput,
        input: &str,
    ) -> Result<TransferResponse, AnalogError> {
        let input_port = if let Some(index) = self.voltage_source_index(input) {
            Port::Branch(self.nodes + index)
        } else if let Some(index) = self.current_source_index(input) {
            let source = &self.circuit.current_sources[index];
            Port::Nodes(source.p, source.n)
        } else {
            return Err(AnalogError::Config(format!(
                "`.tf` input `{input}` is not an independent `V` or `I` source"
            )));
        };
        let output_port = match output {
            TransferOutput::Voltage { node, reference } => {
                let resolve = |name: &str| {
                    self.circuit.node(name).ok_or_else(|| {
                        AnalogError::Config(format!(
                            "`.tf` output node `{name}` is not in the netlist"
                        ))
                    })
                };
                let node = resolve(node)?;
                let reference = reference.as_deref().map(resolve).transpose()?.flatten();
                if node == reference {
                    return Err(AnalogError::Config(
                        "`.tf` output and reference are the same node".to_string(),
                    ));
                }
                // A unit current drawn from the reference into the output.
                Port::Nodes(reference, node)
            }
            TransferOutput::Current(element) => {
                let index = self.circuit.branch_index(element).ok_or_else(|| {
                    AnalogError::Config(format!(
                        "`.tf` output `i({element})` names no `V`, `L`, `E`, `H` or \
                         behavioural voltage source"
                    ))
                })?;
                Port::Branch(self.nodes + index)
            }
        };
        let dim = self.dim();
        let mut g = self.linearised(|solver| {
            let couplings = solver.delayed_couplings();
            let (mut g, _) = solver.ac_matrices(&couplings)?;
            // At DC every delayed term is at its full value.
            for coupling in &couplings {
                g[coupling.row * dim + coupling.column] += coupling.coefficient;
            }
            Ok(g)
        })?;
        let solve = |g: &mut [f64], port: Port| {
            let mut rhs = vec![0.0; dim];
            port.excite(&mut rhs);
            solve_dense(g, &mut rhs, dim).map_err(|row| AnalogError::Singular { row })?;
            Ok::<_, AnalogError>(rhs)
        };
        let forward = solve(&mut g.clone(), input_port)?;
        let input_impedance = input_port.impedance(&forward);
        let output_impedance = if output_port == input_port {
            input_impedance
        } else {
            output_port.impedance(&solve(&mut g, output_port)?)
        };
        Ok(TransferResponse {
            transfer: output_port.response(&forward),
            input_impedance,
            output_impedance,
        })
    }

    /// Run `body` with every nonlinear device linearised at the present
    /// solution, then put the linearisation back so the next transient step
    /// is unchanged.
    fn linearised<T>(
        &mut self,
        body: impl FnOnce(&mut Self) -> Result<T, AnalogError>,
    ) -> Result<T, AnalogError> {
        if self.nonlinear {
            self.save_linearisation();
            self.relinearise();
        }
        let result = body(self);
        if self.nonlinear {
            self.restore_linearisation();
        }
        self.cached_stamp = None;
        self.settled = false;
        result
    }

    /// Every noise current generator at the present linearisation, as
    /// ngspice's `resnoise.c`, `swnoise.c`, `dionoise.c`, `bjtnoise.c`,
    /// `mos1noi.c` and `jfetnoise.c` give them. A flicker generator is added
    /// only for a card with a nonzero `KF`. A part whose name holds `#` is one
    /// this engine added for a macromodel or a BJT substrate junction and is
    /// noiseless.
    fn noise_sources(&self) -> Vec<NoiseSource> {
        let kelvin = |celsius: Option<f64>| celsius.map_or(device::TEMPERATURE_K, |t| t + 273.15);
        let circuit_kelvin = kelvin(self.circuit.temperature_celsius);
        let thermal = |kelvin: f64, siemens: f64| 4.0 * device::BOLTZMANN * kelvin * siemens;
        let shot = |amps: f64| 2.0 * device::ELEMENTARY_CHARGE * amps.abs();
        // ngspice's `KF·exp(AF·ln(max(|I|, N_MINLOG)))`, the density at 1 Hz.
        let flicker = |kf: f64, af: f64, amps: f64| kf * libm::pow(amps.abs().max(1e-38), af);
        let user = |name: &str| !name.contains('#');
        let mut sources = Vec::new();
        let mut push = |term: &str, element: &str, mechanism, density: f64, a, b| {
            if density > 0.0 && a != b {
                let element = element.to_ascii_lowercase();
                sources.push(NoiseSource {
                    name: if term.is_empty() {
                        element
                    } else {
                        format!("{term}_{element}")
                    },
                    mechanism,
                    density,
                    a,
                    b,
                });
            }
        };
        use NoiseMechanism::{Channel, Flicker, Shot, Thermal};

        for resistor in self.circuit.resistors.iter().filter(|r| user(&r.name)) {
            let celsius = resistor.ntc.map(|ntc| ntc.temperature_celsius).or(resistor
                .temperature_coefficients
                .map(|tc| tc.temperature_celsius));
            let kelvin = celsius.map_or(circuit_kelvin, |t| t + 273.15);
            let density = thermal(kelvin, 1.0 / resistor.resistance_ohms());
            push("", &resistor.name, Thermal, density, resistor.a, resistor.b);
        }
        for (index, switch) in self.circuit.switches.iter().enumerate() {
            if !user(&switch.name) {
                continue;
            }
            let ohms = if self.switch_closed[index] {
                switch.ron
            } else {
                switch.roff
            };
            let density = thermal(circuit_kelvin, 1.0 / ohms);
            push("", &switch.name, Thermal, density, switch.a, switch.b);
        }
        for (index, diode) in self.circuit.diodes.iter().enumerate() {
            if !user(&diode.name) {
                continue;
            }
            let m = diode.multiplier;
            let kelvin = kelvin(diode.temperature_celsius);
            if diode.series_resistance_ohms > 0.0 {
                let density = thermal(kelvin, m / diode.series_resistance_ohms);
                push(
                    "rs",
                    &diode.name,
                    Thermal,
                    density,
                    diode.anode,
                    diode.junction_anode,
                );
            }
            let op = diode_model_op(
                self.diode_vj[index],
                diode.model,
                diode.breakdown,
                diode.temperature_celsius,
            );
            let density = shot(m * op.id);
            let (a, k) = (diode.junction_anode, diode.cathode);
            push("id", &diode.name, Shot, density, a, k);
            let density = m * flicker(diode.model.kf, diode.model.af, op.id);
            push("1overf", &diode.name, Flicker, density, a, k);
        }
        for (index, bjt) in self.circuit.bjts.iter().enumerate() {
            if !user(&bjt.name) {
                continue;
            }
            let m = bjt.multiplier;
            let model = bjt.operating_model;
            let kelvin = kelvin(bjt.temperature_celsius);
            let op = bjt_op(bjt, self.bjt_vbe[index], self.bjt_vbc[index]);
            let rb = device::bjt_base_resistance(model, op.ib);
            for (term, ohms, a, b) in [
                ("rc", model.rc, bjt.c, bjt.junction_cx),
                ("rb", rb, bjt.b, bjt.junction_b),
                ("re", model.re, bjt.e, bjt.junction_e),
            ] {
                if ohms > 0.0 {
                    push(term, &bjt.name, Thermal, thermal(kelvin, m / ohms), a, b);
                }
            }
            let (c, b, e) = (bjt.junction_c, bjt.junction_b, bjt.junction_e);
            push("ic", &bjt.name, Shot, shot(m * op.ic), c, e);
            push("ib", &bjt.name, Shot, shot(m * op.ib), b, e);
            let density = m * flicker(model.kf, model.af, op.ib);
            push("1overf", &bjt.name, Flicker, density, b, e);
        }
        for (index, mosfet) in self.circuit.mosfets.iter().enumerate() {
            if !user(&mosfet.name) {
                continue;
            }
            let m = mosfet.multiplier;
            let model = mosfet.operating_model;
            let kelvin = kelvin(mosfet.temperature_celsius);
            for (term, ohms, a, b) in [
                (
                    "rd",
                    mosfet.drain_resistance_ohms,
                    mosfet.d,
                    mosfet.junction_d,
                ),
                (
                    "rs",
                    mosfet.source_resistance_ohms,
                    mosfet.s,
                    mosfet.junction_s,
                ),
            ] {
                if ohms > 0.0 {
                    push(term, &mosfet.name, Thermal, thermal(kelvin, m / ohms), a, b);
                }
            }
            let (vgs, vds) = (self.mos_vgs[index], self.mos_vds[index]);
            let (vgs, vds, vbs) = if vds < 0.0 {
                (vgs - vds, -vds, self.mos_vbd[index])
            } else {
                (vgs, vds, self.mos_vbs[index])
            };
            let sign = model.polarity.sign();
            let op = device::mos_op(
                vgs,
                vds,
                sign * model.vto,
                mosfet.beta,
                model.lambda,
                model.gamma,
                model.phi,
                vbs,
            );
            let density = thermal(kelvin, m * 2.0 / 3.0 * op.gm.abs());
            let (d, s) = (mosfet.junction_d, mosfet.junction_s);
            push("id", &mosfet.name, Channel, density, d, s);
            // `mos1noi.c` takes `C'ox` as 3.9·ε0/TOX, TOX defaulting to
            // 100 nm, not the `EPSOX` the level-1 charge uses.
            let oxide = 3.9 * device::VACUUM_PERMITTIVITY_F_PER_M / model.tox.unwrap_or(1e-7);
            let area = mosfet.w * mosfet.effective_length() * oxide * oxide;
            let density = m * flicker(model.kf, model.af, op.id) / area;
            push("1overf", &mosfet.name, Flicker, density, d, s);
        }
        for (index, jfet) in self.circuit.jfets.iter().enumerate() {
            if !user(&jfet.name) {
                continue;
            }
            let m = jfet.multiplier;
            let kelvin = kelvin(jfet.temperature_celsius);
            for (term, ohms, a, b) in [
                ("rd", jfet.model.rd, jfet.d, jfet.junction_d),
                ("rs", jfet.model.rs, jfet.s, jfet.junction_s),
            ] {
                if ohms > 0.0 {
                    push(term, &jfet.name, Thermal, thermal(kelvin, m / ohms), a, b);
                }
            }
            let vgs = if self.jfet_vds[index] < 0.0 {
                self.jfet_vgd[index]
            } else {
                self.jfet_vgs[index]
            };
            let op = device::jfet_op_with_b(
                vgs,
                self.jfet_vds[index].abs(),
                jfet.model.polarity.sign() * jfet.operating_model.vto,
                jfet.operating_model.beta,
                jfet.model.lambda,
                jfet.model.b,
                jfet.operating_model.pb,
            );
            let density = thermal(kelvin, m * 2.0 / 3.0 * op.gm.abs());
            let (d, s) = (jfet.junction_d, jfet.junction_s);
            push("id", &jfet.name, Channel, density, d, s);
            let density = m * flicker(jfet.model.kf, jfet.model.af, op.id);
            push("1overf", &jfet.name, Flicker, density, d, s);
        }
        sources
    }

    /// `G` and `C` of the small-signal system, row-major `dim × dim`.
    fn ac_matrices(
        &mut self,
        couplings: &[DelayedCoupling],
    ) -> Result<(Vec<f64>, Vec<f64>), AnalogError> {
        // A step far below any time constant a netlist carries keeps `C/h`
        // well above `G`, so the subtraction loses little; the second step
        // checks that the difference really is `C/h`.
        const STEP: f64 = 1e-15;
        let dim = self.dim();
        let raw = |solver: &mut Self, stamp: Stamp| {
            solver.build(stamp);
            match solver.behavioural_error.take() {
                Some(error) => Err(error),
                None => Ok(solver.matrix[..dim * dim].to_vec()),
            }
        };
        // Each delayed term is a fixed pattern times a factor set by the
        // stamp; take it out of each matrix so that what is left is the
        // frequency-independent part. The BJT excess-phase filter is turned
        // off for the transient stamps, so its term is the plain DC one.
        let stamp = |solver: &mut Self, kind: Stamp| {
            let mut matrix = raw(solver, kind)?;
            for coupling in couplings {
                matrix[coupling.row * dim + coupling.column] -=
                    coupling.present_factor(kind) * coupling.coefficient;
            }
            Ok::<_, AnalogError>(matrix)
        };
        let g = stamp(self, Stamp::OperatingPoint);
        self.excess_phase_off = true;
        let transient = g.and_then(|g| {
            let short = stamp(self, Stamp::Transient(STEP, Integration::BackwardEuler))?;
            let long = stamp(
                self,
                Stamp::Transient(2.0 * STEP, Integration::BackwardEuler),
            )?;
            Ok((g, short, long))
        });
        self.excess_phase_off = false;
        let (g, short, long) = transient?;
        let mut c = vec![0.0; dim * dim];
        for cell in 0..dim * dim {
            let first = short[cell] - g[cell];
            let second = long[cell] - g[cell];
            let tolerance = 1e-9 * (first.abs() + g[cell].abs()) + f64::MIN_POSITIVE;
            if (first - 2.0 * second).abs() > tolerance {
                return Err(AnalogError::Config(format!(
                    "`.ac` cannot linearise row `{}`, column `{}`: an element there has a \
                     transient term that is not a capacitance or inductance (diode soft \
                     recovery `VP`, op-amp `SR`/`IOLIM`)",
                    self.unknown_name(cell / dim),
                    self.unknown_name(cell % dim),
                )));
            }
            c[cell] = first * STEP;
        }
        Ok((g, c))
    }

    /// Every matrix term that `.ac` multiplies by `e^{-jωTD}`.
    ///
    /// The far-port terms of each `T` line, per unit of the coupling factor:
    /// [`Solver::stamp_lines`] writes `alpha · coefficient` at each.
    ///
    /// The forward transconductance `∂Ict/∂Vbe` of each BJT with `PTF`, at
    /// the present linearisation. ngspice's `bjtacld.c` delays exactly this
    /// term by Weil's `td = PTF·π/180·TF`; the reverse term `∂Ict/∂Vbc` and
    /// the base current are not delayed. With `e^{-jωtd} = 1` the system is
    /// the one without excess phase.
    fn delayed_couplings(&self) -> Vec<DelayedCoupling> {
        let mut couplings = Vec::new();
        for (index, bjt) in self.circuit.bjts.iter().enumerate() {
            let delay = device::bjt_excess_phase_delay(bjt.model);
            if delay <= 0.0 {
                continue;
            }
            let gif = bjt.multiplier * bjt_op(bjt, self.bjt_vbe[index], self.bjt_vbc[index]).gif;
            for (row, sign) in [(bjt.junction_c, 1.0), (bjt.junction_e, -1.0)] {
                for (column, polarity) in [(bjt.junction_b, 1.0), (bjt.junction_e, -1.0)] {
                    if let (Some(row), Some(column)) = (row, column) {
                        couplings.push(DelayedCoupling {
                            row,
                            column,
                            coefficient: sign * polarity * gif,
                            delay,
                            line: false,
                        });
                    }
                }
            }
        }
        for (index, line) in self.circuit.transmission_lines.iter().enumerate() {
            let row1 = self.nodes + self.circuit.transmission_line_branch(index);
            let row2 = row1 + 1;
            for (row, far_p, far_n, far_row) in [
                (row1, line.p2, line.n2, row2),
                (row2, line.p1, line.n1, row1),
            ] {
                if let Some(column) = far_p {
                    couplings.push(DelayedCoupling {
                        row,
                        column,
                        coefficient: -1.0,
                        delay: line.delay,
                        line: true,
                    });
                }
                if let Some(column) = far_n {
                    couplings.push(DelayedCoupling {
                        row,
                        column,
                        coefficient: 1.0,
                        delay: line.delay,
                        line: true,
                    });
                }
                couplings.push(DelayedCoupling {
                    row,
                    column: far_row,
                    coefficient: -line.z0,
                    delay: line.delay,
                    line: true,
                });
            }
        }
        couplings
    }

    /// The `AC` phasor of every independent source, on the MNA right-hand
    /// side.
    fn ac_excitation(&self) -> Vec<Phasor> {
        let mut b = vec![Phasor::default(); self.dim()];
        let phasor = |ac: Option<super::netlist::AcStimulus>| {
            ac.map_or(Phasor::default(), |ac| {
                let radians = ac.phase_degrees.to_radians();
                Phasor {
                    re: ac.magnitude * libm::cos(radians),
                    im: ac.magnitude * libm::sin(radians),
                }
            })
        };
        for (index, source) in self.circuit.voltage_sources.iter().enumerate() {
            b[self.nodes + index] = phasor(source.ac);
        }
        for source in &self.circuit.current_sources {
            let amps = phasor(source.ac);
            for (node, sign) in [(source.p, -1.0), (source.n, 1.0)] {
                if let Some(node) = node {
                    b[node].re += sign * amps.re;
                    b[node].im += sign * amps.im;
                }
            }
        }
        b
    }

    /// A node name, or the element whose branch current an unknown is.
    fn unknown_name(&self, index: usize) -> String {
        if index < self.nodes {
            format!("v({})", self.circuit.node_name(index))
        } else {
            format!("i({})", self.circuit.branch_name(index - self.nodes))
        }
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
            let source = &self.circuit.voltage_sources[index].wave;
            if source.is_constant() {
                continue;
            }
            let value = source.at(time);
            self.set_voltage_source(index, value);
        }
        for index in 0..self.circuit.current_sources.len() {
            let source = &self.circuit.current_sources[index].wave;
            if source.is_constant() {
                continue;
            }
            let value = source.at(time);
            self.set_current_source(index, value);
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
            let model = bjt.operating_model;
            if model.rb > 0.0 && !device::bjt_has_bias_dependent_base_resistance(model) {
                conductance(
                    &mut self.matrix,
                    dim,
                    bjt.b,
                    bjt.junction_b,
                    bjt.multiplier / model.rb,
                );
            }
            if model.rc > 0.0 {
                conductance(
                    &mut self.matrix,
                    dim,
                    bjt.c,
                    bjt.junction_cx,
                    bjt.multiplier / model.rc,
                );
            }
            if model.re > 0.0 {
                conductance(
                    &mut self.matrix,
                    dim,
                    bjt.e,
                    bjt.junction_e,
                    bjt.multiplier / model.re,
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
                    mosfet.multiplier / mosfet.drain_resistance_ohms,
                );
            }
            if mosfet.source_resistance_ohms > 0.0 {
                conductance(
                    &mut self.matrix,
                    dim,
                    mosfet.junction_s,
                    mosfet.s,
                    mosfet.multiplier / mosfet.source_resistance_ohms,
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
                    jfet.multiplier / jfet.model.rd,
                );
            }
            if jfet.model.rs > 0.0 {
                conductance(
                    &mut self.matrix,
                    dim,
                    jfet.junction_s,
                    jfet.s,
                    jfet.multiplier / jfet.model.rs,
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
        self.stamp_lines(stamp);
        self.stamp_line_history(stamp);
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
        let off = self.circuit.has_off_devices();
        if off {
            self.hold_off_devices();
        }
        let mut shunt = 1.0;
        while shunt >= 1e-12 {
            self.node_shunt = shunt;
            self.holding_off = off;
            let stage = self.newton(Stamp::OperatingPoint, None, self.time);
            self.holding_off = false;
            if stage.is_err() {
                self.node_shunt = 0.0;
                return stage;
            }
            shunt /= 10.0;
        }
        self.node_shunt = 0.0;
        self.newton(Stamp::OperatingPoint, None, self.time)
    }

    /// The operating-point Newton solve. With an `off` device it runs twice,
    /// as ngspice's `MODEINITFIX` then `MODEINITFLOAT`: first with every
    /// `off` device held cut off until the rest of the circuit converges,
    /// then with it released, starting from that solution. A circuit with one
    /// operating point reaches it either way; a bistable one keeps the state
    /// the held device chose. Without an `off` device this is one plain solve,
    /// so every other netlist's trajectory is unchanged.
    fn operating_point_newton(&mut self) -> Result<(), AnalogError> {
        if self.circuit.has_off_devices() {
            self.hold_off_devices();
            self.holding_off = true;
            let held = self.newton(Stamp::OperatingPoint, None, self.time);
            self.holding_off = false;
            held?;
        }
        self.newton(Stamp::OperatingPoint, None, self.time)
    }

    /// Put every `off` device's linearisation point at 0 V, where ngspice's
    /// `MODEINITJCT` puts it.
    fn hold_off_devices(&mut self) {
        for (index, diode) in self.circuit.diodes.iter().enumerate() {
            if diode.off {
                self.diode_vj[index] = 0.0;
            }
        }
        for (index, bjt) in self.circuit.bjts.iter().enumerate() {
            if bjt.off {
                self.bjt_vbe[index] = 0.0;
                self.bjt_vbc[index] = 0.0;
                self.bjt_vbcx[index] = 0.0;
            }
        }
        for (index, mosfet) in self.circuit.mosfets.iter().enumerate() {
            if mosfet.off {
                self.mos_vgs[index] = 0.0;
                self.mos_vds[index] = 0.0;
                self.mos_vbd[index] = 0.0;
                self.mos_vbs[index] = 0.0;
            }
        }
        for (index, jfet) in self.circuit.jfets.iter().enumerate() {
            if jfet.off {
                self.jfet_vgs[index] = 0.0;
                self.jfet_vgd[index] = 0.0;
                self.jfet_vds[index] = 0.0;
            }
        }
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
        self.stamp_line_history(stamp);
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
        self.behavioural_time = time;

        for _ in 1..=MAX_NEWTON_ITERATIONS {
            self.build(stamp);
            if let Some(error) = self.behavioural_error.take() {
                return Err(error);
            }
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
            if self.holding_off && diode.off {
                continue;
            }
            let raw = node_diff(
                &self.solution[..self.nodes],
                diode.junction_anode,
                diode.cathode,
            );
            let (is, _, thermal_voltage) =
                diode_temperature_parameters(diode.model, diode.temperature_celsius);
            let vte = diode.model.n * thermal_voltage;
            let critical = match diode.temperature_celsius {
                None => device::pn_critical_voltage(diode.model.is, diode.model.n),
                Some(_) => device::pn_critical_voltage_at_thermal_voltage(
                    is,
                    diode.model.n,
                    thermal_voltage,
                ),
            };
            let limited = match (diode.breakdown, diode.temperature_celsius) {
                (None, _) => device::pn_limit(raw, self.diode_vj[index], vte, critical),
                (Some(xbv), None) => device::breakdown_limit(
                    raw,
                    self.diode_vj[index],
                    vte,
                    critical,
                    xbv,
                    diode.model.nbv.unwrap_or(diode.model.n),
                ),
                (Some(xbv), Some(_)) => device::breakdown_limit_at_thermal_voltage(
                    raw,
                    self.diode_vj[index],
                    vte,
                    critical,
                    xbv,
                    diode.model.nbv.unwrap_or(diode.model.n),
                    thermal_voltage,
                ),
            };
            self.limiter_clamped |= limited != raw;
            self.diode_vj[index] = limited;
        }

        for index in 0..self.circuit.bjts.len() {
            let bjt = &self.circuit.bjts[index];
            if self.holding_off && bjt.off {
                continue;
            }
            let sign = bjt.model.polarity.sign();
            let nodes = &self.solution[..self.nodes];
            let raw_be = sign * node_diff(nodes, bjt.junction_b, bjt.junction_e);
            let raw_bc = sign * node_diff(nodes, bjt.junction_b, bjt.junction_c);
            let model = bjt.operating_model;
            let vtf = model.nf * bjt.thermal_voltage;
            let vtr = model.nr * bjt.thermal_voltage;
            let crit_f = device::pn_critical_voltage_at_thermal_voltage(
                model.is,
                model.nf,
                bjt.thermal_voltage,
            );
            let crit_r = device::pn_critical_voltage_at_thermal_voltage(
                model.is,
                model.nr,
                bjt.thermal_voltage,
            );
            let be = device::pn_limit(raw_be, self.bjt_vbe[index], vtf, crit_f);
            let bc = device::pn_limit(raw_bc, self.bjt_vbc[index], vtr, crit_r);
            self.limiter_clamped |= be != raw_be || bc != raw_bc;
            self.bjt_vbe[index] = be;
            self.bjt_vbc[index] = bc;
            // The epitaxial current is exponential in the base-to-contact
            // voltage too, so that voltage gets the collector junction's
            // limiter. ngspice leaves it unlimited.
            if model.rco.is_some() {
                let raw_bcx = sign * node_diff(nodes, bjt.junction_b, bjt.junction_cx);
                let bcx = device::pn_limit(raw_bcx, self.bjt_vbcx[index], vtr, crit_r);
                self.limiter_clamped |= bcx != raw_bcx;
                self.bjt_vbcx[index] = bcx;
            }
        }

        for index in 0..self.circuit.mosfets.len() {
            let mosfet = &self.circuit.mosfets[index];
            if self.holding_off && mosfet.off {
                continue;
            }
            let sign = mosfet.operating_model.polarity.sign();
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
            let eff_gs =
                device::fet_limit(eff_gs_new, eff_gs_old, sign * mosfet.operating_model.vto);
            let (gs, ds) = if reversed_now {
                (eff_gs - eff_ds, -eff_ds)
            } else {
                (eff_gs, eff_ds)
            };
            self.limiter_clamped |= gs != raw_gs || ds != raw_ds;
            self.mos_vgs[index] = gs;
            self.mos_vds[index] = ds;

            let vte = mosfet.thermal_voltage();
            let (drain_is, source_is) = mosfet.junction_saturation_currents();
            let drain_critical = device::pn_critical_voltage_at_thermal_voltage(drain_is, 1.0, vte);
            let source_critical =
                device::pn_critical_voltage_at_thermal_voltage(source_is, 1.0, vte);
            let bd = device::pn_limit(raw_bd, self.mos_vbd[index], vte, drain_critical);
            let bs = device::pn_limit(raw_bs, self.mos_vbs[index], vte, source_critical);
            self.limiter_clamped |= bd != raw_bd || bs != raw_bs;
            self.mos_vbd[index] = bd;
            self.mos_vbs[index] = bs;
        }

        for index in 0..self.circuit.jfets.len() {
            let jfet = &self.circuit.jfets[index];
            if self.holding_off && jfet.off {
                continue;
            }
            let sign = jfet.model.polarity.sign();
            let nodes = &self.solution[..self.nodes];
            let raw_gs = sign * node_diff(nodes, jfet.g, jfet.junction_s);
            let raw_gd = sign * node_diff(nodes, jfet.g, jfet.junction_d);
            let raw_ds = sign * node_diff(nodes, jfet.junction_d, jfet.junction_s);
            // ngspice limits the gate junctions at N = 1 whatever the card's
            // `N`; the limiter only paces Newton, the solution is unchanged.
            let vt = jfet.thermal_voltage();
            let critical =
                device::pn_critical_voltage_at_thermal_voltage(jfet.operating_model.is, 1.0, vt);
            let gs = device::pn_limit(raw_gs, self.jfet_vgs[index], vt, critical);
            let gd = device::pn_limit(raw_gd, self.jfet_vgd[index], vt, critical);

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

        for (index, limit) in self.circuit.output_current_limits.iter().enumerate() {
            self.output_limit_point[index] =
                node_diff(&self.solution[..self.nodes], limit.drive, limit.output);
        }
        for (index, source) in self.circuit.slew_limited_vccs.iter().enumerate() {
            let nodes = &self.solution[..self.nodes];
            let common_mode = 0.5
                * (node_diff(nodes, source.input_plus, None)
                    + node_diff(nodes, source.input_minus, None)
                    - node_diff(nodes, source.vcc, None)
                    - node_diff(nodes, source.vee, None));
            self.slew_limit_point[index] = node_diff(nodes, source.input_plus, source.input_minus)
                + source.cm_gain * common_mode
                + source.vos;
        }

        // A behavioural expression has no limiter in ngspice either: it takes
        // the plain Newton point, unless it holds a `TABLE`, whose flat ends
        // make plain Newton cycle (see `Expression::step_limit`).
        let sources = self
            .circuit
            .behavioural_voltages
            .iter()
            .chain(&self.circuit.behavioural_currents);
        for (point, source) in self.behavioural_point.iter_mut().zip(sources) {
            let limit = source.expression.step_limit();
            for (value, control) in point.iter_mut().zip(&source.controls) {
                let new = match *control {
                    BehaviouralControl::Voltage(p, n) => {
                        node_diff(&self.solution[..self.nodes], p, n)
                    }
                    BehaviouralControl::Branch(branch) => self.solution[self.nodes + branch],
                };
                *value = match limit {
                    Some(limit) if (new - *value).abs() > limit => {
                        self.limiter_clamped = true;
                        *value + limit.copysign(new - *value)
                    }
                    _ => new,
                };
            }
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
            let m = diode.multiplier;
            let charge_model = diode.operating_model;
            if diode.series_resistance_ohms > 0.0 {
                conductance(
                    matrix,
                    dim,
                    diode.anode,
                    diode.junction_anode,
                    m / diode.series_resistance_ohms,
                );
            }
            let vj = self.diode_vj[index];
            let op = diode_model_op(vj, diode.model, diode.breakdown, diode.temperature_celsius);
            let control = [(op.gd, diode.junction_anode, diode.cathode, vj)];
            stamp_linearised(matrix, rhs, dim, m, diode.junction_anode, op.id, &control);
            stamp_linearised(
                matrix,
                rhs,
                dim,
                m,
                diode.cathode,
                -op.id,
                &negate(&control),
            );

            if diode_charge_active(charge_model) {
                if let Stamp::Transient(h, rule) = stamp {
                    let (charge, capacitance) = diode_charge_op(vj, charge_model, op);
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
                        m,
                        diode.junction_anode,
                        current,
                        &charge_control,
                    );
                    stamp_linearised(
                        matrix,
                        rhs,
                        dim,
                        m,
                        diode.cathode,
                        -current,
                        &negate(&charge_control),
                    );
                }
            }

            if charge_model.vp > 0.0 && charge_model.tt > 0.0 {
                if let Stamp::Transient(h, rule) = stamp {
                    let recovery = diode_recovery_step(
                        charge_model,
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
                        m,
                        diode.junction_anode,
                        recovery.current,
                        &recovery_control,
                    );
                    stamp_linearised(
                        matrix,
                        rhs,
                        dim,
                        m,
                        diode.cathode,
                        -recovery.current,
                        &negate(&recovery_control),
                    );
                }
            }
        }

        for (index, bjt) in self.circuit.bjts.iter().enumerate() {
            let m = bjt.multiplier;
            let sign = bjt.model.polarity.sign();
            let model = bjt.operating_model;
            let operating_model = bjt.operating_model;
            let (vbe, vbc) = (self.bjt_vbe[index], self.bjt_vbc[index]);
            let mut op = bjt_op(bjt, vbe, vbc);
            if let Stamp::Transient(h, _) = stamp {
                let delay = device::bjt_excess_phase_delay(bjt.model);
                if delay > 0.0 && !self.excess_phase_off {
                    // ngspice replaces cbe by cex = arg3·cbe in the transport
                    // current and adds the history term cc; the base current
                    // keeps the undelayed cbe.
                    let transport = device::bjt_forward_transport(
                        vbe,
                        vbc,
                        operating_model,
                        bjt.thermal_voltage,
                    );
                    let (history, arg3) =
                        excess_phase_step(delay, h, self.previous_step, self.bjt_excess[index]);
                    let shift = transport.cbe * (arg3 - 1.0);
                    let qb_squared = transport.qb * transport.qb;
                    op.ic += history + shift / transport.qb;
                    op.gif += transport.gbe * (arg3 - 1.0) / transport.qb
                        - shift * transport.dqb_dvbe / qb_squared;
                    op.gir += shift * transport.dqb_dvbc / qb_squared;
                }
            }

            // The Gummel–Poon base resistance is updated from the transistor's
            // base current on each Newton pass, following ngspice's RBM/IRB
            // conductance stamp. Fixed RB remains in the linear build path.
            if device::bjt_has_bias_dependent_base_resistance(model) {
                let resistance = device::bjt_base_resistance(model, op.ib);
                conductance(matrix, dim, bjt.b, bjt.junction_b, m / resistance);
            }

            // GMIN across both junctions. Without it a transistor that is fully
            // off leaves its base and collector tied to the rest of the circuit
            // by nothing at all, and the matrix is singular on a netlist the
            // user can see is connected.
            conductance(
                matrix,
                dim,
                bjt.junction_b,
                bjt.junction_e,
                m * device::GMIN,
            );
            conductance(
                matrix,
                dim,
                bjt.junction_b,
                bjt.junction_c,
                m * device::GMIN,
            );

            // Controlling voltages in circuit coordinates: u1 = v(b_internal)
            // − v(e), u2 = v(b_internal) − v(c), and u3 = v(b_external) − v(c).
            // The device sees sign·u1, sign·u2, and sign·u3.
            let u1 = sign * vbe;
            let u2 = sign * vbc;
            let u3 = node_diff(&self.solution[..self.nodes], bjt.b, bjt.junction_c);
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
            stamp_linearised(
                matrix,
                rhs,
                dim,
                m,
                bjt.junction_c,
                sign * op.ic,
                &collector,
            );
            stamp_linearised(matrix, rhs, dim, m, bjt.junction_b, sign * op.ib, &base);
            stamp_linearised(
                matrix,
                rhs,
                dim,
                m,
                bjt.junction_e,
                -sign * (op.ic + op.ib),
                &emitter,
            );

            // Kull's quasi-saturation region from the collector contact to
            // the intrinsic collector, controlled by u2 and by
            // u4 = v(b_internal) − v(c_contact).
            let vbcx = self.bjt_vbcx[index];
            let u4 = sign * vbcx;
            if let Some(rco) = model.rco {
                let epi =
                    device::bjt_epi_op(vbc, vbcx, rco, model.vo, model.gamma, bjt.thermal_voltage);
                let control = [
                    (epi.g_vbc, bjt.junction_b, bjt.junction_c, u2),
                    (epi.g_vbcx, bjt.junction_b, bjt.junction_cx, u4),
                ];
                stamp_linearised(
                    matrix,
                    rhs,
                    dim,
                    m,
                    bjt.junction_cx,
                    sign * epi.current,
                    &control,
                );
                stamp_linearised(
                    matrix,
                    rhs,
                    dim,
                    m,
                    bjt.junction_c,
                    -sign * epi.current,
                    &negate(&control),
                );
            }

            if let Stamp::Transient(h, rule) = stamp {
                let charge = bjt_charge_op(
                    vbe,
                    vbc,
                    sign * u3,
                    vbcx,
                    operating_model,
                    bjt.thermal_voltage,
                );
                for junction in 0..4 {
                    if !bjt_charge_active(model, junction) {
                        continue;
                    }
                    let (positive, negative) = match junction {
                        0 => (bjt.junction_b, bjt.junction_e),
                        1 => (bjt.junction_b, bjt.junction_c),
                        2 => (bjt.b, bjt.junction_c),
                        _ => (bjt.junction_b, bjt.junction_cx),
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
                        (charge.dq[junction][2] * factor, bjt.b, bjt.junction_c, u3),
                        (
                            charge.dq[junction][3] * factor,
                            bjt.junction_b,
                            bjt.junction_cx,
                            u4,
                        ),
                    ];
                    stamp_linearised(matrix, rhs, dim, m, positive, sign * current, &control);
                    stamp_linearised(
                        matrix,
                        rhs,
                        dim,
                        m,
                        negative,
                        -sign * current,
                        &negate(&control),
                    );
                }
            }
        }

        for (index, mosfet) in self.circuit.mosfets.iter().enumerate() {
            let m = mosfet.multiplier;
            let sign = mosfet.operating_model.polarity.sign();
            let model = mosfet.operating_model;
            let (vgs, vds) = (self.mos_vgs[index], self.mos_vds[index]);

            // Below Vto the channel is off, so GMIN connects drain to source.
            // The two bulk pn junctions include their own GMIN through
            // diode_op, and use the model's bulk-junction saturation current.
            conductance(
                matrix,
                dim,
                mosfet.junction_d,
                mosfet.junction_s,
                m * device::GMIN,
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
            stamp_linearised(matrix, rhs, dim, m, drain, sign * op.id, &drain_terms);
            stamp_linearised(
                matrix,
                rhs,
                dim,
                m,
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
                    stamp_linearised(matrix, rhs, dim, m, positive, current, &control);
                    stamp_linearised(matrix, rhs, dim, m, negative, -current, &negate(&control));
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
                        stamp_linearised(matrix, rhs, dim, m, node, current, &control);
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
                        stamp_linearised(matrix, rhs, dim, m, positive, current, &control);
                        stamp_linearised(
                            matrix,
                            rhs,
                            dim,
                            m,
                            negative,
                            -current,
                            &negate(&control),
                        );
                    }
                }
            }

            // Level-1 MOSFETs have two bulk pn junctions. In NMOS coordinates
            // the bulk is the anode and drain/source are the cathodes; the
            // polarity sign reverses that orientation for PMOS.
            let (drain_is, source_is) = mosfet.junction_saturation_currents();
            let bd = device::mos_bulk_junction_op(
                self.mos_vbd[index],
                drain_is,
                mosfet.thermal_voltage(),
            );
            let bd_terms = [(
                bd.gd,
                mosfet.bulk,
                mosfet.junction_d,
                sign * self.mos_vbd[index],
            )];
            stamp_linearised(matrix, rhs, dim, m, mosfet.bulk, sign * bd.id, &bd_terms);
            stamp_linearised(
                matrix,
                rhs,
                dim,
                m,
                mosfet.junction_d,
                -sign * bd.id,
                &negate(&bd_terms),
            );

            let bs = device::mos_bulk_junction_op(
                self.mos_vbs[index],
                source_is,
                mosfet.thermal_voltage(),
            );
            let bs_terms = [(
                bs.gd,
                mosfet.bulk,
                mosfet.junction_s,
                sign * self.mos_vbs[index],
            )];
            stamp_linearised(matrix, rhs, dim, m, mosfet.bulk, sign * bs.id, &bs_terms);
            stamp_linearised(
                matrix,
                rhs,
                dim,
                m,
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
                    stamp_linearised(
                        matrix,
                        rhs,
                        dim,
                        m,
                        mosfet.bulk,
                        sign * current,
                        &charge_terms,
                    );
                    stamp_linearised(
                        matrix,
                        rhs,
                        dim,
                        m,
                        junction,
                        -sign * current,
                        &negate(&charge_terms),
                    );
                }
            }
            // The gate carries no DC current in this model.
        }

        for (index, jfet) in self.circuit.jfets.iter().enumerate() {
            let m = jfet.multiplier;
            let sign = jfet.model.polarity.sign();
            let reversed = self.jfet_vds[index] < 0.0;
            let (drain, source, vgs) = if reversed {
                (jfet.junction_s, jfet.junction_d, self.jfet_vgd[index])
            } else {
                (jfet.junction_d, jfet.junction_s, self.jfet_vgs[index])
            };
            let vds = self.jfet_vds[index].abs();
            let op = device::jfet_op_with_b(
                vgs,
                vds,
                sign * jfet.operating_model.vto,
                jfet.operating_model.beta,
                jfet.model.lambda,
                jfet.model.b,
                jfet.operating_model.pb,
            );

            // The channel's GMIN keeps a cut-off device connected. The gate
            // junctions below each carry their own SPICE GMIN as part of the
            // diode equation.
            conductance(
                matrix,
                dim,
                jfet.junction_d,
                jfet.junction_s,
                m * device::GMIN,
            );

            let channel_terms = [
                (op.gm, jfet.g, source, sign * vgs),
                (op.gds, drain, source, sign * vds),
            ];
            stamp_linearised(matrix, rhs, dim, m, drain, sign * op.id, &channel_terms);
            stamp_linearised(
                matrix,
                rhs,
                dim,
                m,
                source,
                -sign * op.id,
                &negate(&channel_terms),
            );

            let gate_model = jfet.operating_model;
            let gate_vt = jfet.thermal_voltage();
            for (junction, (cathode, voltage, c0)) in [
                (jfet.junction_s, self.jfet_vgs[index], gate_model.cgs),
                (jfet.junction_d, self.jfet_vgd[index], gate_model.cgd),
            ]
            .into_iter()
            .enumerate()
            {
                let gate = device::pn_junction_op_at_thermal_voltage(
                    voltage,
                    gate_model.is,
                    gate_model.n,
                    gate_vt,
                );
                let gate_voltage = sign * voltage;
                let gate_terms = [(gate.gd, jfet.g, cathode, gate_voltage)];
                stamp_linearised(matrix, rhs, dim, m, jfet.g, sign * gate.id, &gate_terms);
                stamp_linearised(
                    matrix,
                    rhs,
                    dim,
                    m,
                    cathode,
                    -sign * gate.id,
                    &negate(&gate_terms),
                );

                if c0 > 0.0 {
                    if let Stamp::Transient(h, rule) = stamp {
                        let (charge, capacitance) = device::junction_depletion_charge(
                            voltage,
                            c0,
                            gate_model.pb,
                            0.5,
                            gate_model.fc,
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
                        stamp_linearised(
                            matrix,
                            rhs,
                            dim,
                            m,
                            jfet.g,
                            sign * current,
                            &charge_terms,
                        );
                        stamp_linearised(
                            matrix,
                            rhs,
                            dim,
                            m,
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

        // Op-amp output stages use a Norton current whose small-signal slope is
        // 1/ROUT. IOLIM clips that current symmetrically; on the flat part its
        // derivative is zero, so the output settles to the load's voltage at
        // the limited current instead of asking the gain node to supply more.
        for (index, limit) in self.circuit.output_current_limits.iter().enumerate() {
            let delta = self.output_limit_point[index];
            let unconstrained = delta / limit.rout;
            let current = unconstrained.clamp(-limit.ilim, limit.ilim);
            let conductance = if unconstrained.abs() < limit.ilim {
                1.0 / limit.rout
            } else {
                0.0
            };
            let control = [(-conductance, limit.drive, limit.output, delta)];
            stamp_linearised(matrix, rhs, dim, 1.0, limit.output, -current, &control);
        }

        // A finite SR limits only the gain-node capacitor current. The Rg
        // current is excluded so the settled open-loop gain remains AOL. The
        // same input error carries offset and CMRR through the limiter.
        for (index, source) in self.circuit.slew_limited_vccs.iter().enumerate() {
            let error = self.slew_limit_point[index];
            let nodes = &self.solution[..self.nodes];
            let gain_voltage = node_diff(nodes, source.n, source.p);
            let capacitor_current = source.gm * error - gain_voltage / MACRO_GAIN_RESISTANCE;
            // Slew rate constrains dV/dt, not the operating point. At t=0 the
            // capacitor is open, so preserve the unconstrained AOL equation.
            let saturated =
                !matches!(stamp, Stamp::OperatingPoint) && capacitor_current.abs() >= source.ilim;
            let current = if matches!(stamp, Stamp::OperatingPoint) {
                source.gm * error
            } else {
                gain_voltage / MACRO_GAIN_RESISTANCE
                    + capacitor_current.clamp(-source.ilim, source.ilim)
            };
            let gm = if saturated { 0.0 } else { source.gm };
            let cm_half = 0.5 * source.cm_gain;
            let gain_node_slope = if saturated {
                -1.0 / MACRO_GAIN_RESISTANCE
            } else {
                0.0
            };
            let control = [
                (
                    gm * (1.0 + cm_half),
                    source.input_plus,
                    None,
                    node_diff(nodes, source.input_plus, None),
                ),
                (
                    gm * (-1.0 + cm_half),
                    source.input_minus,
                    None,
                    node_diff(nodes, source.input_minus, None),
                ),
                (
                    -gm * cm_half,
                    source.vcc,
                    None,
                    node_diff(nodes, source.vcc, None),
                ),
                (
                    -gm * cm_half,
                    source.vee,
                    None,
                    node_diff(nodes, source.vee, None),
                ),
                (
                    gain_node_slope,
                    source.p,
                    source.n,
                    node_diff(nodes, source.p, source.n),
                ),
            ];
            stamp_linearised(matrix, rhs, dim, 1.0, source.p, current, &control);
            stamp_linearised(matrix, rhs, dim, 1.0, source.n, -current, &negate(&control));
        }

        // Behavioural sources: f(u) ≈ f0 + Σ gₖ·(uₖ − uₖ0), where each uₖ is a
        // node-voltage difference or a branch current. A voltage form owns a
        // branch row `v(n+) − v(n−) − Σ gₖ·uₖ = f0 − Σ gₖ·uₖ0` whose current
        // flows n+ → n− through the source, like an `E`; a current form
        // draws f out of n+ and returns it at n−, like a `G`.
        let circuit = &self.circuit;
        let behavioural_base = self.nodes
            + circuit.voltage_sources.len()
            + circuit.inductors.len()
            + circuit.vcvs.len()
            + circuit.ccvs.len();
        let voltages = circuit.behavioural_voltages.len();
        let sources = circuit
            .behavioural_voltages
            .iter()
            .chain(&circuit.behavioural_currents);
        for (index, source) in sources.enumerate() {
            let point = &self.behavioural_point[index];
            let (value, slopes) = match source.expression.evaluate(point, self.behavioural_time) {
                Ok(result) => result,
                Err(message) => {
                    if self.behavioural_error.is_none() {
                        self.behavioural_error = Some(AnalogError::Evaluation {
                            element: source.name.clone(),
                            time: self.behavioural_time,
                            message,
                        });
                    }
                    continue;
                }
            };
            let equivalent = value
                - slopes
                    .iter()
                    .zip(point)
                    .map(|(slope, at)| slope * at)
                    .sum::<f64>();
            // Each slope enters a row as `scale·g` on the columns of its
            // control.
            let nodes = self.nodes;
            let stamp_row = |matrix: &mut [f64], row: usize, scale: f64| {
                for (slope, control) in slopes.iter().zip(&source.controls) {
                    match *control {
                        BehaviouralControl::Voltage(p, n) => {
                            add(matrix, dim, Some(row), p, scale * slope);
                            add(matrix, dim, Some(row), n, -scale * slope);
                        }
                        BehaviouralControl::Branch(branch) => {
                            add(matrix, dim, Some(row), Some(nodes + branch), scale * slope);
                        }
                    }
                }
            };
            if index < voltages {
                let row = behavioural_base + index;
                add(matrix, dim, Some(row), source.p, 1.0);
                add(matrix, dim, Some(row), source.n, -1.0);
                add(matrix, dim, source.p, Some(row), 1.0);
                add(matrix, dim, source.n, Some(row), -1.0);
                stamp_row(matrix, row, -1.0);
                rhs[row] += equivalent;
            } else {
                for (terminal, sign) in [(source.p, 1.0), (source.n, -1.0)] {
                    let Some(row) = terminal else { continue };
                    stamp_row(matrix, row, sign);
                    rhs[row] -= sign * equivalent;
                }
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
/// Weight a `T` line's delayed wave puts on the far port's unknown end-of-step
/// value: 1 at the operating point, `1 − TD/h` when the step is longer than
/// the delay, otherwise 0. See [`Solver::line_terms`].
/// One matrix term that `.ac` multiplies by `e^{-jωTD}`; see
/// [`Solver::delayed_couplings`].
#[derive(Debug, Clone, Copy)]
struct DelayedCoupling {
    row: usize,
    column: usize,
    coefficient: f64,
    delay: f64,
    /// A `T` line's far-port term, which each stamp scales by
    /// [`line_alpha`]; otherwise a BJT excess-phase term, which every stamp
    /// `.ac` builds holds at its full DC value.
    line: bool,
}

impl DelayedCoupling {
    fn present_factor(&self, stamp: Stamp) -> f64 {
        if self.line {
            line_alpha(self.delay, stamp)
        } else {
            1.0
        }
    }
}

fn line_alpha(delay: f64, stamp: Stamp) -> f64 {
    match stamp {
        Stamp::OperatingPoint => 1.0,
        Stamp::Transient(h, _) if delay < h => 1.0 - delay / h,
        Stamp::Transient(..) => 0.0,
    }
}

fn stamp_linearised(
    matrix: &mut [f64],
    rhs: &mut [f64],
    dim: usize,
    multiplier: f64,
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
        add(matrix, dim, Some(row), *p, multiplier * g);
        add(matrix, dim, Some(row), *n, -multiplier * g);
        equivalent -= g * u;
    }
    rhs[row] -= multiplier * equivalent;
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

/// One port of a `.tf` analysis.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Port {
    /// A branch row, driven by a unit voltage in series with its element; the
    /// response is the branch current.
    Branch(usize),
    /// A node pair `(p, n)`, driven by 1 A taken from `p` and delivered to
    /// `n`, as an `I` source drives; the response is `v(n) − v(p)`.
    Nodes(NodeRef, NodeRef),
}

impl Port {
    fn excite(self, rhs: &mut [f64]) {
        match self {
            Port::Branch(row) => rhs[row] = 1.0,
            Port::Nodes(p, n) => {
                inject(rhs, p, -1.0);
                inject(rhs, n, 1.0);
            }
        }
    }

    fn response(self, solution: &[f64]) -> f64 {
        let at = |node: NodeRef| node.map_or(0.0, |n| solution[n]);
        match self {
            Port::Branch(row) => solution[row],
            Port::Nodes(p, n) => at(n) - at(p),
        }
    }

    /// The resistance the port's own drive sees.
    fn impedance(self, solution: &[f64]) -> f64 {
        match self {
            // ngspice's `tfanal.c` threshold for no current.
            Port::Branch(row) if solution[row].abs() < 1e-20 => f64::INFINITY,
            Port::Branch(row) => -1.0 / solution[row],
            Port::Nodes(..) => self.response(solution),
        }
    }
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
