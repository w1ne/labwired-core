// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Device physics for the nonlinear elements: the exponential diode, the
//! Ebers–Moll BJT and the level-1 MOSFET, plus the constants and the
//! voltage-limiting heuristics Newton–Raphson needs to reach them.
//!
//! This module is equations only. It owns no state, touches no matrix, and
//! knows nothing about nodes: [`super::mna`] calls it with the controlling
//! voltages of one device and gets back that device's currents and the partial
//! derivatives to stamp. Keeping it separate is what makes the DC operating
//! points testable against hand calculations without building a circuit.
//!
//! ## What is modelled, and what is not
//!
//! DC/large-signal behaviour only. There are **no device capacitances** in this
//! first version — no diode junction or diffusion charge, no BJT `Cje`/`Cjc`,
//! no MOSFET overlap or channel charge — and no temperature model: every
//! parameter is taken at [`TEMPERATURE_K`] as written. A `.model` line may
//! carry those parameters (real vendor models do) and they are accepted and
//! ignored; see [`super::netlist`]. The consequence is concrete: a circuit
//! whose behaviour comes from stored device charge — reverse-recovery, a
//! Miller-limited edge, a charge-pump — is not this engine's to solve, and
//! `adapter: external_process` with ngspice is.
//!
//! ## Determinism
//!
//! `exp` and `log` come from the `libm` crate, not from the platform's libm.
//! `f64::exp` is a call into the C library on a native build and into whatever
//! the toolchain links on `wasm32`, and those are not the same function: they
//! agree to within an ulp or two, which is exactly the size of drift that turns
//! a bit-exact browser/native differential into a flaky one. `libm` is pure
//! Rust with no dependencies of its own and produces identical bits on both.
//! `analog_devices.rs` pins the operating points of one diode, one BJT and one
//! MOSFET as raw `f64` bit patterns so any drift shows up as a failing test
//! rather than as a 0.1 % disagreement somebody argues about.

/// Temperature every device parameter is evaluated at, in kelvin.
///
/// The engine has no temperature model, so this is a constant rather than a
/// setting. ngspice's default is 300.15 K (27 °C); the 0.05 % difference in
/// [`THERMAL_VOLTAGE`] moves a silicon junction by about 13 µV per decade of
/// current, which is why the ngspice differential decks pin `temp`/`tnom` to
/// 26.85 °C — exactly 300.00 K — instead of arguing about it.
pub const TEMPERATURE_K: f64 = 300.0;

/// Boltzmann's constant, J/K (CODATA 2019, exact by definition).
const BOLTZMANN: f64 = 1.380_649e-23;

/// Elementary charge, C (CODATA 2019, exact by definition).
const ELEMENTARY_CHARGE: f64 = 1.602_176_634e-19;

/// `kT/q` at [`TEMPERATURE_K`]: about 25.852 mV.
pub const THERMAL_VOLTAGE: f64 = BOLTZMANN * TEMPERATURE_K / ELEMENTARY_CHARGE;

/// Conductance added across every pn junction, siemens.
///
/// SPICE's `GMIN`. It exists so a device that is fully off still ties its
/// terminals together: without it a reverse-biased diode leaves the node behind
/// it floating and the MNA matrix singular, which reports as "check for a
/// floating node" on a circuit that has none. 1 pA/V is below any current a
/// probe in this engine resolves.
pub const GMIN: f64 = 1e-12;

/// Largest argument handed to `exp`.
///
/// Voltage limiting keeps the junction voltages sane, but the *first* Newton
/// iteration of a step is stamped from a guess that limiting has not seen yet,
/// and `exp(800)` is `+inf` — after which every later iteration is `NaN` and
/// the solver reports nothing useful. Clamping the argument (and evaluating the
/// derivative at the clamp, so the linearisation stays a valid tangent) keeps
/// the iteration finite and lets limiting walk it back. `exp(200)` is ~7e86;
/// times a femtoamp saturation current that is still a finite, absurd number
/// the next iteration reduces.
const EXP_ARG_MAX: f64 = 200.0;

/// `exp`, clamped so a wild Newton guess cannot produce an infinity.
fn exp_clamped(arg: f64) -> f64 {
    libm::exp(if arg > EXP_ARG_MAX { EXP_ARG_MAX } else { arg })
}

/// Natural logarithm, from `libm` so it is the same function everywhere.
pub fn ln(value: f64) -> f64 {
    libm::log(value)
}

// ---------------------------------------------------------------------------
// Diode
// ---------------------------------------------------------------------------

/// One diode's linearisation: the current through the junction and `dI/dV`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DiodeOp {
    /// Junction current, anode → cathode, amps.
    pub id: f64,
    /// Junction conductance `dI/dV`, siemens, `GMIN` included.
    pub gd: f64,
}

/// Shockley diode: `I = Is·(exp(V / (N·Vt)) − 1) + GMIN·V`.
///
/// `v_junction` is the voltage across the junction itself — with `Rs` in the
/// model that is *not* the anode-to-cathode voltage, because the series
/// resistance drops the difference.
pub fn diode_op(v_junction: f64, is: f64, n: f64) -> DiodeOp {
    let vte = n * THERMAL_VOLTAGE;
    let exponential = exp_clamped(v_junction / vte);
    DiodeOp {
        id: is * (exponential - 1.0) + GMIN * v_junction,
        gd: is * exponential / vte + GMIN,
    }
}

/// The junction voltage above which [`pn_limit`] starts damping, per SPICE:
/// the point where `dI/dV` of the exponential passes `1/Vt`.
pub fn pn_critical_voltage(is: f64, n: f64) -> f64 {
    let vte = n * THERMAL_VOLTAGE;
    vte * ln(vte / (core::f64::consts::SQRT_2 * is))
}

/// SPICE's `DEVpnjlim`: damp a Newton step across a pn junction.
///
/// An undamped Newton step on an exponential overshoots by orders of magnitude
/// — one iteration asks for 5 V across a junction, the next sees `exp(193)` and
/// asks for −40 V, and it never settles. This replaces a large forward step
/// with the logarithmically equivalent one, which is what makes a diode circuit
/// converge in single-digit iterations.
///
/// It changes only the *path*, never the answer: the converged point satisfies
/// the same equations whatever the limiter did on the way, which is why the
/// operating-point tests can be written against hand calculations.
pub fn pn_limit(v_new: f64, v_old: f64, vte: f64, v_critical: f64) -> f64 {
    if v_new > v_critical && (v_new - v_old).abs() > 2.0 * vte {
        if v_old > 0.0 {
            let arg = 1.0 + (v_new - v_old) / vte;
            if arg > 0.0 {
                v_old + vte * ln(arg)
            } else {
                v_critical
            }
        } else {
            vte * ln(v_new / vte)
        }
    } else {
        v_new
    }
}

// ---------------------------------------------------------------------------
// Bipolar junction transistor
// ---------------------------------------------------------------------------

/// One BJT's linearisation, in device coordinates (NPN sense; a PNP's
/// terminal voltages and currents are negated by the caller).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BjtOp {
    /// Current into the collector, amps.
    pub ic: f64,
    /// Current into the base, amps.
    pub ib: f64,
    /// `∂Ict/∂Vbe` — the forward transconductance, siemens.
    pub gif: f64,
    /// `−∂Ict/∂Vbc` — the reverse transconductance, siemens.
    pub gir: f64,
    /// `∂Ibe/∂Vbe` — the base-emitter (`pi`) conductance, siemens.
    pub gpi: f64,
    /// `∂Ibc/∂Vbc` — the base-collector (`mu`) conductance, siemens.
    pub gmu: f64,
}

/// Ebers–Moll in transport form, which is Gummel–Poon with the base-width
/// modulation and high-injection terms switched off:
///
/// ```text
/// Ict = Is·(exp(Vbe/(Nf·Vt)) − exp(Vbc/(Nr·Vt)))
/// Ibe = (Is/Bf)·(exp(Vbe/(Nf·Vt)) − 1)
/// Ibc = (Is/Br)·(exp(Vbc/(Nr·Vt)) − 1)
/// Ic  = Ict − Ibc          Ib = Ibe + Ibc          Ie = −(Ic + Ib)
/// ```
///
/// That is exactly ngspice's BJT with `VAF`/`VAR` infinite, `IKF`/`IKR`
/// infinite, `RB` = `RC` = `RE` = 0 and no capacitances — the defaults for
/// every one of those but the resistances, which we simply do not model. The
/// missing Early effect is the one a user will notice: output conductance in
/// the active region is `GMIN` here, not `Ic/VAF`.
pub fn bjt_op(vbe: f64, vbc: f64, is: f64, bf: f64, br: f64, nf: f64, nr: f64) -> BjtOp {
    let vtf = nf * THERMAL_VOLTAGE;
    let vtr = nr * THERMAL_VOLTAGE;
    let forward = exp_clamped(vbe / vtf);
    let reverse = exp_clamped(vbc / vtr);

    let gif = is * forward / vtf;
    let gir = is * reverse / vtr;
    let gpi = gif / bf;
    let gmu = gir / br;

    let ict = is * (forward - reverse);
    let ibe = (is / bf) * (forward - 1.0);
    let ibc = (is / br) * (reverse - 1.0);

    BjtOp {
        ic: ict - ibc,
        ib: ibe + ibc,
        gif,
        gir,
        gpi,
        gmu,
    }
}

// ---------------------------------------------------------------------------
// MOSFET
// ---------------------------------------------------------------------------

/// One MOSFET's linearisation, in device coordinates (NMOS sense, drain and
/// source already put the way round that makes `Vds ≥ 0`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MosOp {
    /// Drain current, drain → source, amps. Never negative in these
    /// coordinates.
    pub id: f64,
    /// `∂Id/∂Vgs`, siemens.
    pub gm: f64,
    /// `∂Id/∂Vds`, siemens.
    pub gds: f64,
}

/// Shichman–Hodges (SPICE level 1), with `beta = Kp·W/L`:
///
/// ```text
/// cutoff      Vgs ≤ Vto            Id = 0
/// linear      Vds < Vgs − Vto      Id = beta·(1 + λ·Vds)·Vds·(Vgs − Vto − Vds/2)
/// saturation  Vds ≥ Vgs − Vto      Id = (beta/2)·(1 + λ·Vds)·(Vgs − Vto)²
/// ```
///
/// No body effect: `Vth` is `Vto`, which is ngspice's level 1 with its default
/// `GAMMA = 0`. The bulk terminal is required by the syntax and connected —
/// [`super::mna`] ties it to drain and source through `GMIN`, which is what a
/// reverse-biased bulk junction is — but it does not shift the threshold.
pub fn mos_op(vgs: f64, vds: f64, vto: f64, beta: f64, lambda: f64) -> MosOp {
    let vgst = vgs - vto;
    if vgst <= 0.0 {
        return MosOp {
            id: 0.0,
            gm: 0.0,
            gds: 0.0,
        };
    }
    let betap = beta * (1.0 + lambda * vds);
    if vds < vgst {
        MosOp {
            id: betap * vds * (vgst - 0.5 * vds),
            gm: betap * vds,
            gds: betap * (vgst - vds) + lambda * beta * vds * (vgst - 0.5 * vds),
        }
    } else {
        MosOp {
            id: 0.5 * betap * vgst * vgst,
            gm: betap * vgst,
            gds: 0.5 * lambda * beta * vgst * vgst,
        }
    }
}

/// SPICE's `DEVfetlim`: damp a Newton step on a MOSFET gate voltage.
///
/// Like [`pn_limit`], a path heuristic that does not move the converged answer.
pub fn fet_limit(v_new: f64, v_old: f64, vto: f64) -> f64 {
    let step_high = (2.0 * (v_old - vto)).abs() + 2.0;
    let step_low = step_high / 2.0 + 2.0;
    let v_ox = vto + 3.5;
    let delta = v_new - v_old;

    if v_old >= vto {
        if v_old >= v_ox {
            if delta <= 0.0 {
                // Turning off.
                if v_new >= v_ox {
                    if -delta > step_low {
                        return v_old - step_low;
                    }
                } else {
                    return v_new.max(vto + 2.0);
                }
            } else if delta >= step_high {
                // Staying on.
                return v_old + step_high;
            }
        } else if delta <= 0.0 {
            return v_new.max(vto - 0.5);
        } else {
            return v_new.min(vto + 4.0);
        }
    } else if delta <= 0.0 {
        if -delta > step_high {
            return v_old - step_high;
        }
    } else {
        let threshold = vto + 0.5;
        if v_new <= threshold {
            if delta > step_low {
                return v_old + step_low;
            }
        } else {
            return threshold;
        }
    }
    v_new
}

/// SPICE's `DEVlimvds`: damp a Newton step on a MOSFET drain-source voltage.
pub fn vds_limit(v_new: f64, v_old: f64) -> f64 {
    if v_old >= 3.5 {
        if v_new > v_old {
            v_new.min(3.0 * v_old + 2.0)
        } else if v_new < 3.5 {
            v_new.max(2.0)
        } else {
            v_new
        }
    } else if v_new > v_old {
        v_new.min(4.0)
    } else {
        v_new.max(-0.5)
    }
}
