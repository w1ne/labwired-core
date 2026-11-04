// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Device physics for the nonlinear elements: the exponential diode, the
//! Gummel–Poon BJT with Early effect, high-injection rolloff, base
//! recombination and bias-dependent base resistance, the level-1 JFET with its
//! Parker–Skellern fit and gate-junction depletion charge, and
//! the level-1 MOSFET, plus the constants and voltage-limiting heuristics
//! Newton–Raphson needs to reach them.
//!
//! This module is equations only. It owns no state, touches no matrix, and
//! knows nothing about nodes: [`super::mna`] calls it with the controlling
//! voltages of one device and gets back that device's currents and the partial
//! derivatives to stamp. Keeping it separate is what makes the DC operating
//! points testable against hand calculations without building a circuit.
//!
//! ## What is modelled, and what is not
//!
//! DC/large-signal behaviour for the nonlinear current equations. Diode
//! forward/reverse high-injection current knees (`IK`/`IKF`, `IKR`), depletion
//! charge (`CJO`, `VJ`, `M`, `FC`), recombination current (`ISR`, `NR`),
//! quasi-static diffusion charge (`TT`) or soft reverse-recovery charge
//! (`TT`/`VP`), BJT
//! base-junction depletion charge (`CJE`/`VJE`/`MJE`, `CJC`/`VJC`/`MJC`,
//! `FC`), and JFET gate-junction depletion charge (`CGS`, `CGD`, `PB`, `FC`,
//! with level-1 `M=0.5`) are integrated in transient runs. The level-1 JFET
//! Parker–Skellern channel fit from `B` (default 1) is solved at each operating
//! point. MOSFET gate overlap
//! charge (`CGSO`, `CGDO`, `CGBO`) is integrated as linear capacitance using
//! instance `W` and `L`; BJT transit-time charge (`TF`/`TR`) is integrated
//! using ideal transit times with ngspice's forward `XTF`/`VTF`/`ITF` bias
//! adjustment. MOSFET intrinsic channel charge from `TOX` uses the level-1
//! Meyer model by default; opt-in Ward–Dutton mode integrates strong-inversion
//! terminal charge with an analytic Jacobian. MOSFET bulk depletion charge integrates
//! bottom-area capacitance from `CBD`/`CBS` or `CJ` times instance `AD`/`AS`,
//! and sidewall capacitance from `CJSW` times `PD`/`PS`, with separate `MJ`
//! and `MJSW` grading coefficients.
//! There is no temperature model: every parameter is taken at
//! [`TEMPERATURE_K`] as written. A `.model` line may
//! carry unmodelled parameters (real vendor models do) and they are accepted
//! and ignored; see [`super::netlist`]. Diode soft recovery covers the
//! first-order `VP` charge state. Other recovery models and stored charge in
//! unsupported junctions still require `adapter: external_process` with ngspice.
//!
//! ## Determinism
//!
//! `exp` and `log` come from the `libm` crate, not from the platform's libm.
//! `f64::exp` is a call into the C library on a native build and into whatever
//! the toolchain links on `wasm32`, and those are not the same function: they
//! agree to within an ulp or two, which is exactly the size of drift that turns
//! a bit-exact browser/native differential into a flaky one. `libm` is pure
//! Rust with no dependencies of its own and produces identical bits on both.
//! `analog_devices.rs` pins representative diode, BJT and MOSFET operating
//! points as raw `f64` bit patterns and checks JFET regions against hand
//! calculations, so drift or an incorrect equation fails a test.

use super::netlist::BjtModel;
use super::netlist::{MosChannelChargeModel, Mosfet};

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

/// SiO₂ permittivity used by ngspice's level-1 MOSFET `TOX` model, F/m.
pub const OXIDE_PERMITTIVITY_F_PER_M: f64 = 3.453_133e-11;

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

/// Ngspice's optional diode recombination-current branch for `ISR` and `NR`.
///
/// In forward operation its current is multiplied by the junction-generation
/// factor `( (1 − V/VJ)² + 0.005 )^(M/2)`. In reverse operation ngspice holds
/// the recombination contribution at its value at `−3·N·Vt`, including in
/// breakdown. An omitted `ISR` is represented by zero and contributes nothing.
pub fn diode_recombination_op(
    v_junction: f64,
    isr: f64,
    nr: f64,
    n: f64,
    vj: f64,
    m: f64,
) -> DiodeOp {
    if isr == 0.0 {
        return DiodeOp { id: 0.0, gd: 0.0 };
    }

    let forward_boundary = -3.0 * n * THERMAL_VOLTAGE;
    let voltage = v_junction.max(forward_boundary);
    let vte = nr * THERMAL_VOLTAGE;
    let exponential = exp_clamped(voltage / vte);
    let base_current = isr * (exponential - 1.0);
    let x = 1.0 - voltage / vj;
    let shape = x * x + 0.005;
    let factor = exp_clamped(0.5 * m * ln(shape));
    let current = base_current * factor;

    let conductance = if v_junction < forward_boundary {
        0.0
    } else {
        let d_factor_dv = if m == 0.0 {
            0.0
        } else {
            -m * x / vj * exp_clamped((0.5 * m - 1.0) * ln(shape))
        };
        isr * exponential / vte * factor + base_current * d_factor_dv
    };

    DiodeOp {
        id: current,
        gd: conductance,
    }
}

/// PN-junction depletion charge and differential capacitance for SPICE's
/// zero-bias capacitance, junction potential, grading coefficient, and
/// forward coefficient parameters.
///
/// Below the forward transition (`FC·VJ`), this integrates
/// `C(v) = C0·(1 − v/VJ)⁻ᴹ`. Above it, the capacitance is continued linearly
/// from its value at the transition so both charge and capacitance remain
/// continuous. `M = 1` uses the logarithmic charge limit.
pub fn junction_depletion_charge(voltage: f64, c0: f64, vj: f64, m: f64, fc: f64) -> (f64, f64) {
    if c0 == 0.0 {
        return (0.0, 0.0);
    }

    let vfc = fc * vj;
    let xfc = 1.0 - fc;
    let pow = |base: f64, exponent: f64| libm::exp(exponent * ln(base));
    if voltage < vfc {
        let x = 1.0 - voltage / vj;
        let charge = if m == 1.0 {
            -c0 * vj * libm::log1p(-voltage / vj)
        } else {
            -c0 * vj / (1.0 - m) * libm::expm1((1.0 - m) * ln(x))
        };
        let capacitance = c0 * pow(x, -m);
        (charge, capacitance)
    } else {
        let dv = voltage - vfc;
        let cfc = c0 * pow(xfc, -m);
        let slope = m * c0 * pow(xfc, -1.0 - m) / vj;
        let qfc = if m == 1.0 {
            -c0 * vj * libm::log1p(-fc)
        } else {
            -c0 * vj / (1.0 - m) * libm::expm1((1.0 - m) * ln(xfc))
        };
        (qfc + cfc * dv + 0.5 * slope * dv * dv, cfc + slope * dv)
    }
}

/// Diode-specific spelling retained for callers that probe the diode model.
pub fn diode_depletion_charge(v_junction: f64, cjo: f64, vj: f64, m: f64, fc: f64) -> (f64, f64) {
    junction_depletion_charge(v_junction, cjo, vj, m, fc)
}

/// Ideal diode diffusion charge and its differential capacitance for `TT`.
///
/// The quasi-static SPICE model is `Qd = TT·Id`, so `dQd/dV = TT·dId/dV`.
/// `op` may come from either the ordinary or breakdown diode equation. Its
/// `GMIN` contribution is removed because the numerical shunt does not carry
/// minority-carrier storage.
pub fn diode_diffusion_charge(voltage: f64, op: DiodeOp, tt: f64) -> (f64, f64) {
    (tt * (op.id - GMIN * voltage), tt * (op.gd - GMIN))
}

/// Relative tolerance of the breakdown-knee fit in [`breakdown_voltage`].
///
/// ngspice uses its `RELTOL` here; the differential decks set that to 1e-9,
/// and so does this engine's Newton ([`super::mna::NEWTON_RELTOL`]), so the two
/// engines stop the fit at the same iteration.
pub const BREAKDOWN_FIT_RELTOL: f64 = 1e-9;

/// ngspice's effective breakdown voltage `tBrkdwnV` (`DIOtemp`, ngspice-47).
///
/// The reverse exponential is `I = −Is·exp(−(V + xbv)/(NBV·Vt))`, and `xbv`
/// is `BV` moved so that the current at `V = −BV` is `IBV` once the reverse
/// saturation region is accounted for. ngspice finds it by a fixed-point
/// iteration of at most 25 steps; this is that iteration, operation for
/// operation, so a zener's knee sits where ngspice puts it rather than
/// `NBV·Vt·ln(IBV/Is)` away from it. When `IBV` is too small to reach from the
/// saturation current (`IBV < Is·BV/Vt`) ngspice gives up and uses `BV` as
/// written; so does this.
///
/// Note that ngspice divides by the bare thermal voltage here, not by `N·Vt` —
/// kept as written, since agreeing with ngspice is the point.
pub fn breakdown_voltage(bv: f64, ibv: f64, is: f64, nbv: f64) -> f64 {
    let vt = THERMAL_VOLTAGE;
    let cbv = ibv;
    if cbv < is * bv / vt {
        return bv;
    }
    let tol = BREAKDOWN_FIT_RELTOL * cbv;
    let mut xbv = bv - nbv * vt * ln(1.0 + cbv / is);
    for _ in 0..25 {
        xbv = bv - nbv * vt * ln(cbv / is + 1.0 - xbv / vt);
        let xcbv = is * (libm::exp((bv - xbv) / (nbv * vt)) - 1.0 + xbv / vt);
        if (xcbv - cbv).abs() <= tol {
            break;
        }
    }
    xbv
}

/// A diode with reverse breakdown: ngspice's three-region level-1 equation.
///
/// ```text
/// V ≥ −3·N·Vt        I = Is·(exp(V/(N·Vt)) − 1)                (forward)
/// −xbv ≤ V < −3·N·Vt  I = −Is·(1 + (3·N·Vt/(e·V))³)             (reverse)
/// V < −xbv           I = −Is·exp(−(xbv + V)/(NBV·Vt))          (breakdown)
/// ```
///
/// plus `GMIN·V` everywhere. The forward region is [`diode_op`]'s exactly.
/// The middle region is SPICE's cubic reverse-saturation approximation, which
/// meets the exponential at `−3·N·Vt` in value; a diode **without** `BV` does
/// not use it — it keeps the plain exponential it always had, so that nothing
/// already simulated moves by a bit.
pub fn diode_op_breakdown(v_junction: f64, is: f64, n: f64, xbv: f64, nbv: f64) -> DiodeOp {
    let vte = n * THERMAL_VOLTAGE;
    if v_junction >= -3.0 * vte {
        return diode_op(v_junction, is, n);
    }
    if v_junction >= -xbv {
        let arg = 3.0 * vte / (v_junction * core::f64::consts::E);
        let arg = arg * arg * arg;
        return DiodeOp {
            id: -is * (1.0 + arg) + GMIN * v_junction,
            gd: is * 3.0 * arg / v_junction + GMIN,
        };
    }
    let vtebrk = nbv * THERMAL_VOLTAGE;
    let evrev = exp_clamped(-(xbv + v_junction) / vtebrk);
    DiodeOp {
        id: -is * evrev + GMIN * v_junction,
        gd: is * evrev / vtebrk + GMIN,
    }
}

/// ngspice's breakdown-side limiter: when the new junction voltage is past
/// `min(0, −xbv + 10·NBV·Vt)`, [`pn_limit`] is applied to the mirrored
/// voltage `−(V + xbv)` — the breakdown exponential's own argument — so a
/// Newton step into breakdown is damped exactly like a forward one. Otherwise
/// the ordinary forward limiter applies. Returns `v_new` itself, bit for bit,
/// when nothing was limited.
pub fn breakdown_limit(
    v_new: f64,
    v_old: f64,
    vte: f64,
    v_critical: f64,
    xbv: f64,
    nbv: f64,
) -> f64 {
    let vtebrk = nbv * THERMAL_VOLTAGE;
    let threshold = (-xbv + 10.0 * vtebrk).min(0.0);
    if v_new < threshold {
        let mirrored = -(v_new + xbv);
        let limited = pn_limit(mirrored, -(v_old + xbv), vtebrk, v_critical);
        if limited == mirrored {
            v_new
        } else {
            -(limited + xbv)
        }
    } else {
        pn_limit(v_new, v_old, vte, v_critical)
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

/// Gummel–Poon transport form. Optional `ISE`/`ISC` terms add the B-E and B-C
/// base-recombination currents. The base-width modulation uses the SPICE Early
/// voltages, and `IKF`/`IKR` optionally add forward and reverse high-injection
/// terms:
///
/// ```text
/// q1  = 1 / (1 − Vbc/VAF − Vbe/VAR)
/// q2  = Is·(exp(Vbe/(Nf·Vt)) − 1)/IKF
///     + Is·(exp(Vbc/(Nr·Vt)) − 1)/IKR
/// qb  = q1·(1 + (1 + 4·q2)^NKF)/2, when `NKF` is supplied
///     = q1·(1 + sqrt(1 + 4·q2))/2, otherwise (`NKF=0.5`)
/// Ict = Is·(exp(Vbe/(Nf·Vt)) − exp(Vbc/(Nr·Vt))) / qb
/// Ibe = (Is/Bf)·(exp(Vbe/(Nf·Vt)) − 1) + ISE·(exp(Vbe/(NE·Vt)) − 1)
/// Ibc = (Is/Br)·(exp(Vbc/(Nr·Vt)) − 1) + ISC·(exp(Vbc/(NC·Vt)) − 1)
/// Ic  = Ict − Ibc          Ib = Ibe + Ibc          Ie = −(Ic + Ib)
/// ```
///
/// Infinite `IKF` or `IKR` disables that direction's rolloff term; with both
/// infinite, the legacy arithmetic is retained exactly. With `RB` = `RC` =
/// `RE` = 0 and no capacitances, this is the portion of ngspice's model
/// represented in-core. A zero or infinite Early voltage disables its
/// corresponding base-width term.
pub fn bjt_op(
    vbe: f64,
    vbc: f64,
    is: f64,
    bf: f64,
    br: f64,
    nf: f64,
    nr: f64,
    vaf: f64,
    var: f64,
    ikf: f64,
    ikr: f64,
    ise: f64,
    ne: f64,
    isc: f64,
    nc: f64,
) -> BjtOp {
    bjt_op_with_nkf(
        vbe, vbc, is, bf, br, nf, nr, vaf, var, ikf, ikr, ise, ne, isc, nc, None,
    )
}

/// Gummel–Poon operating point with an optional high-current beta exponent.
/// `None` keeps the historical square-root path bit-for-bit; `Some(nkf)` uses
/// the explicit ngspice `NKF` rolloff exponent.
pub fn bjt_op_with_nkf(
    vbe: f64,
    vbc: f64,
    is: f64,
    bf: f64,
    br: f64,
    nf: f64,
    nr: f64,
    vaf: f64,
    var: f64,
    ikf: f64,
    ikr: f64,
    ise: f64,
    ne: f64,
    isc: f64,
    nc: f64,
    nkf: Option<f64>,
) -> BjtOp {
    let vtf = nf * THERMAL_VOLTAGE;
    let vtr = nr * THERMAL_VOLTAGE;
    let vte = ne * THERMAL_VOLTAGE;
    let vtc = nc * THERMAL_VOLTAGE;
    let forward = exp_clamped(vbe / vtf);
    let reverse = exp_clamped(vbc / vtr);
    let inv_vaf = if vaf > 0.0 && vaf.is_finite() {
        1.0 / vaf
    } else {
        0.0
    };
    let inv_var = if var > 0.0 && var.is_finite() {
        1.0 / var
    } else {
        0.0
    };
    let base_width = 1.0 - vbc * inv_vaf - vbe * inv_var;
    let transport = is * (forward - reverse);

    // Leave the common infinite-knee path in its original form. Besides being
    // cheaper, preserving the operations keeps existing model cards bit stable.
    let (ict, gif, gir) = if ikf.is_infinite() && ikr.is_infinite() {
        (
            transport * base_width,
            is * forward / vtf * base_width - transport * inv_var,
            is * reverse / vtr * base_width + transport * inv_vaf,
        )
    } else {
        let q1 = 1.0 / base_width;
        let inv_ikf = if ikf > 0.0 && ikf.is_finite() {
            1.0 / ikf
        } else {
            0.0
        };
        let inv_ikr = if ikr > 0.0 && ikr.is_finite() {
            1.0 / ikr
        } else {
            0.0
        };
        let ibe_injection = is * (forward - 1.0);
        let ibc_injection = is * (reverse - 1.0);
        let q2 = inv_ikf * ibe_injection + inv_ikr * ibc_injection;
        let q2_argument = (1.0 + 4.0 * q2).max(0.0);
        let (qb, d_qb_d_vbe, d_qb_d_vbc) = if let Some(nkf) = nkf {
            // The exponent was introduced as an optional model-card parameter.
            // Preserve the legacy square-root arithmetic below when omitted.
            // The clamped zero branch is flat, so its high-injection derivative
            // is zero rather than dividing by a zero argument.
            let rolloff = if q2_argument > 0.0 {
                libm::pow(q2_argument, nkf)
            } else {
                1.0
            };
            let qb = q1 * (1.0 + rolloff) / 2.0;
            let d_rolloff_d_q2 = if q2_argument > 0.0 {
                2.0 * rolloff * nkf / q2_argument
            } else {
                0.0
            };
            (
                qb,
                q1 * (qb * inv_var + inv_ikf * is * forward / vtf * d_rolloff_d_q2),
                q1 * (qb * inv_vaf + inv_ikr * is * reverse / vtr * d_rolloff_d_q2),
            )
        } else {
            // ngspice uses 1 when the clamped argument is zero, keeping both
            // qb and its tangent finite at the limit.
            let sqrt_q2 = if q2_argument > 0.0 {
                libm::sqrt(q2_argument)
            } else {
                1.0
            };
            let qb = q1 * (1.0 + sqrt_q2) / 2.0;
            let d_qb_d_vbe = q1 * (qb * inv_var + inv_ikf * is * forward / vtf / sqrt_q2);
            let d_qb_d_vbc = q1 * (qb * inv_vaf + inv_ikr * is * reverse / vtr / sqrt_q2);
            (qb, d_qb_d_vbe, d_qb_d_vbc)
        };
        let d_transport_d_vbe = is * forward / vtf;
        let d_transport_d_vbc = -is * reverse / vtr;
        let qb_squared = qb * qb;
        (
            transport / qb,
            d_transport_d_vbe / qb - transport * d_qb_d_vbe / qb_squared,
            -(d_transport_d_vbc / qb - transport * d_qb_d_vbc / qb_squared),
        )
    };
    // Early effect and high injection change transport current. Recombination
    // is a separate exponential current in each base junction.
    let (ibe, gpi) = if ise > 0.0 {
        let exp_be = exp_clamped(vbe / vte);
        (
            (is / bf) * (forward - 1.0) + ise * (exp_be - 1.0),
            is * forward / vtf / bf + ise * exp_be / vte,
        )
    } else {
        ((is / bf) * (forward - 1.0), is * forward / vtf / bf)
    };
    let (ibc, gmu) = if isc > 0.0 {
        let exp_bc = exp_clamped(vbc / vtc);
        (
            (is / br) * (reverse - 1.0) + isc * (exp_bc - 1.0),
            is * reverse / vtr / br + isc * exp_bc / vtc,
        )
    } else {
        ((is / br) * (reverse - 1.0), is * reverse / vtr / br)
    };

    BjtOp {
        ic: ict - ibc,
        ib: ibe + ibc,
        gif,
        gir,
        gpi,
        gmu,
    }
}

/// Ngspice's level-1 Gummel–Poon base-spreading resistance for the current
/// flowing into the base. `RBM` defaults to `RB` and `IRB` defaults to
/// infinity, so an omitted transition leaves the old fixed resistor exactly
/// unchanged.
///
/// The interpolation is the SPICE3/ngspice form:
///
/// ```text
/// x = max(Ib / IRB, 1e-9)
/// z = (sqrt(1 + 14.59025·x) − 1) / (2.4317·sqrt(x))
/// RB(Ib) = RBM + 3·(RB − RBM)·(tan(z) − z)/(z·tan²(z))
/// ```
pub fn bjt_base_resistance(model: BjtModel, base_current: f64) -> f64 {
    let rb = model.rb;
    let rbm = model.rbm.unwrap_or(rb);
    if rb == 0.0 || rbm == rb || model.irb.is_infinite() {
        return rb;
    }

    let ratio = (base_current / model.irb).max(1e-9);
    if ratio.is_infinite() {
        return rbm;
    }
    let sqrt_ratio = libm::sqrt(ratio);
    let z = (libm::sqrt(1.0 + 14.59025 * ratio) - 1.0) / (2.4317 * sqrt_ratio);
    let tan_z = libm::tan(z);
    let transition = 3.0 * (tan_z - z) / (z * tan_z * tan_z);
    rbm + (rb - rbm) * transition
}

/// Whether `RBM`/`IRB` make the BJT base resistor depend on bias.
pub fn bjt_has_bias_dependent_base_resistance(model: BjtModel) -> bool {
    model.rb > 0.0 && model.irb.is_finite() && model.rbm.is_some_and(|rbm| rbm < model.rb)
}

/// Transit charge and its derivatives for the forward and reverse BJT
/// transport junctions. Charge is stored in normalized NPN coordinates; the
/// MNA caller applies device polarity when it stamps terminal currents.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct BjtTransitCharge {
    /// Forward base-emitter transport charge, coulombs.
    pub qbe: f64,
    /// `∂Qbe/∂Vbe`, farads.
    pub dqbe_dvbe: f64,
    /// `∂Qbe/∂Vbc`, farads.
    pub dqbe_dvbc: f64,
    /// Reverse base-collector transport charge, coulombs.
    pub qbc: f64,
    /// `∂Qbc/∂Vbe`, farads.
    pub dqbc_dvbe: f64,
    /// `∂Qbc/∂Vbc`, farads.
    pub dqbc_dvbc: f64,
}

/// Gummel–Poon transit charge for `TF` and `TR`:
///
/// ```text
/// Qbe = TF · If · (1 + XTF · exp(Vbc/VTF) · (If/(If+ITF))²) / qb
/// Qbe = TF · If                                         (for Vbe <= 0)
/// Qbc = TR · Is · (exp(Vbc/(NR·Vt)) − 1)
/// ```
///
/// For `Vbe <= 0`, forward transit charge uses the unscaled junction current,
/// as ngspice's level-1 BJT model does. `qb` is the same Early-effect and
/// high-injection base-charge factor used by [`bjt_op`]. `VTF = 0` disables
/// only the exponential voltage dependence; the default infinity makes its
/// exponential equal one. `ITF = 0` disables current suppression.
pub fn bjt_transit_charge(vbe: f64, vbc: f64, model: BjtModel) -> BjtTransitCharge {
    if model.tf == 0.0 && model.tr == 0.0 {
        return BjtTransitCharge::default();
    }

    let vtf = model.nf * THERMAL_VOLTAGE;
    let vtr = model.nr * THERMAL_VOLTAGE;
    let forward = exp_clamped(vbe / vtf);
    let reverse = exp_clamped(vbc / vtr);
    let cbe = model.is * (forward - 1.0);
    let gbe = model.is * forward / vtf;
    let cbc = model.is * (reverse - 1.0);
    let gbc = model.is * reverse / vtr;

    let inv_vaf = if model.vaf > 0.0 && model.vaf.is_finite() {
        1.0 / model.vaf
    } else {
        0.0
    };
    let inv_var = if model.var > 0.0 && model.var.is_finite() {
        1.0 / model.var
    } else {
        0.0
    };
    let base_width = 1.0 - vbc * inv_vaf - vbe * inv_var;

    let (qb, dqb_dvbe, dqb_dvbc) = if model.ikf.is_infinite() && model.ikr.is_infinite() {
        let qb = 1.0 / base_width;
        (qb, qb * qb * inv_var, qb * qb * inv_vaf)
    } else {
        let q1 = 1.0 / base_width;
        let inv_ikf = if model.ikf > 0.0 && model.ikf.is_finite() {
            1.0 / model.ikf
        } else {
            0.0
        };
        let inv_ikr = if model.ikr > 0.0 && model.ikr.is_finite() {
            1.0 / model.ikr
        } else {
            0.0
        };
        let q2 = inv_ikf * cbe + inv_ikr * cbc;
        let q2_argument = (1.0 + 4.0 * q2).max(0.0);
        if let Some(nkf) = model.nkf {
            let rolloff = if q2_argument > 0.0 {
                libm::pow(q2_argument, nkf)
            } else {
                1.0
            };
            let qb = q1 * (1.0 + rolloff) / 2.0;
            let d_rolloff_d_q2 = if q2_argument > 0.0 {
                2.0 * rolloff * nkf / q2_argument
            } else {
                0.0
            };
            (
                qb,
                q1 * (qb * inv_var + inv_ikf * gbe * d_rolloff_d_q2),
                q1 * (qb * inv_vaf + inv_ikr * gbc * d_rolloff_d_q2),
            )
        } else {
            let sqrt_q2 = if q2_argument > 0.0 {
                libm::sqrt(q2_argument)
            } else {
                1.0
            };
            let qb = q1 * (1.0 + sqrt_q2) / 2.0;
            (
                qb,
                q1 * (qb * inv_var + inv_ikf * gbe / sqrt_q2),
                q1 * (qb * inv_vaf + inv_ikr * gbc / sqrt_q2),
            )
        }
    };

    let (qbe, dqbe_dvbe, dqbe_dvbc) = if model.tf == 0.0 {
        (0.0, 0.0, 0.0)
    } else if vbe > 0.0 {
        let qb_squared = qb * qb;
        if model.xtf == 0.0 {
            // Preserve the legacy expression and operation order for cards
            // that do not request the bias-dependent extension.
            (
                model.tf * cbe / qb,
                model.tf * (gbe / qb - cbe * dqb_dvbe / qb_squared),
                -model.tf * cbe * dqb_dvbc / qb_squared,
            )
        } else {
            let voltage_factor = if model.vtf == 0.0 {
                1.0
            } else {
                exp_clamped(vbc / model.vtf)
            };
            let current_ratio = if model.itf == 0.0 {
                1.0
            } else {
                cbe / (cbe + model.itf)
            };
            let boost = model.xtf * voltage_factor * current_ratio * current_ratio;
            let dboost_dvbe = if model.itf == 0.0 {
                0.0
            } else {
                let current_denominator = cbe + model.itf;
                model.xtf * voltage_factor * 2.0 * current_ratio * model.itf * gbe
                    / (current_denominator * current_denominator)
            };
            let dboost_dvbc = if model.vtf == 0.0 || model.vtf.is_infinite() {
                0.0
            } else {
                boost / model.vtf
            };
            let adjusted_current = cbe * (1.0 + boost);
            (
                model.tf * adjusted_current / qb,
                model.tf
                    * (gbe * (1.0 + boost) + cbe * dboost_dvbe - adjusted_current * dqb_dvbe / qb)
                    / qb,
                model.tf * (cbe * dboost_dvbc - adjusted_current * dqb_dvbc / qb) / qb,
            )
        }
    } else {
        (model.tf * cbe, model.tf * gbe, 0.0)
    };

    BjtTransitCharge {
        qbe,
        dqbe_dvbe,
        dqbe_dvbc,
        qbc: model.tr * cbc,
        dqbc_dvbe: 0.0,
        dqbc_dvbc: model.tr * gbc,
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
    /// `∂Id/∂Vbs`, siemens.
    pub gmb: f64,
}

/// Shichman–Hodges (SPICE level 1), with `beta = Kp·W/L`:
///
/// ```text
/// cutoff      Vgs ≤ Vto            Id = 0
/// linear      Vds < Vgs − Vto      Id = beta·(1 + λ·Vds)·Vds·(Vgs − Vto − Vds/2)
/// saturation  Vds ≥ Vgs − Vto      Id = (beta/2)·(1 + λ·Vds)·(Vgs − Vto)²
/// ```
///
/// The body-effect threshold shift follows SPICE level 1:
/// `Vth = VTO + GAMMA·(sqrt(PHI − Vbs) − sqrt(PHI))`, where `Vbs` is the
/// normalised bulk-to-effective-source voltage. For forward body bias, the
/// square-root term uses ngspice's linear continuation, floored at zero.
/// `gmb` is the matching `∂Id/∂Vbs`; the MNA solver stamps it with the gate and
/// drain derivatives. The bulk terminal's drain and source pn junctions are
/// stamped separately using the model's `IS`.
pub fn mos_op(
    vgs: f64,
    vds: f64,
    vto: f64,
    beta: f64,
    lambda: f64,
    gamma: f64,
    phi: f64,
    vbs: f64,
) -> MosOp {
    let (threshold, gmb_factor) = if gamma == 0.0 {
        (vto, 0.0)
    } else {
        let sqrt_phi = libm::sqrt(phi);
        let (body_root, gmb_factor) = if vbs <= 0.0 {
            let body_root = libm::sqrt(phi - vbs);
            (body_root, gamma / (2.0 * body_root))
        } else {
            let body_root = (sqrt_phi - vbs / (2.0 * sqrt_phi)).max(0.0);
            let gmb_factor = if body_root > 0.0 {
                gamma / (2.0 * sqrt_phi)
            } else {
                0.0
            };
            (body_root, gmb_factor)
        };
        (vto + gamma * (body_root - sqrt_phi), gmb_factor)
    };
    let vgst = vgs - threshold;
    if vgst <= 0.0 {
        return MosOp {
            id: 0.0,
            gm: 0.0,
            gds: 0.0,
            gmb: 0.0,
        };
    }
    let betap = beta * (1.0 + lambda * vds);
    if vds < vgst {
        MosOp {
            id: betap * vds * (vgst - 0.5 * vds),
            gm: betap * vds,
            gds: betap * (vgst - vds) + lambda * beta * vds * (vgst - 0.5 * vds),
            gmb: betap * vds * gmb_factor,
        }
    } else {
        MosOp {
            id: 0.5 * betap * vgst * vgst,
            gm: betap * vgst,
            gds: 0.5 * lambda * beta * vgst * vgst,
            gmb: betap * vgst * gmb_factor,
        }
    }
}

/// ngspice level-1 Meyer gate-capacitance halves in effective-source
/// coordinates, ordered G-S, G-D, G-B.
///
/// `cox` is the total oxide capacitance for this instance (`εox·W·L/TOX`).
/// The returned values are halves because SPICE averages the previous and
/// current Meyer capacitances when it integrates the charge over a step. The
/// model is enabled only when the card supplies `TOX`.
pub fn mos_meyer_capacitance_halves(
    vgs: f64,
    vds: f64,
    vbs: f64,
    vto: f64,
    gamma: f64,
    phi: f64,
    cox: f64,
) -> [f64; 3] {
    if cox == 0.0 {
        return [0.0; 3];
    }

    let threshold = if gamma == 0.0 {
        vto
    } else {
        let sqrt_phi = libm::sqrt(phi);
        let body_root = if vbs <= 0.0 {
            libm::sqrt(phi - vbs)
        } else {
            (sqrt_phi - vbs / (2.0 * sqrt_phi)).max(0.0)
        };
        vto + gamma * (body_root - sqrt_phi)
    };
    let vgst = vgs - threshold;
    let vdsat = vgst.max(0.0).max(0.025);

    if vgst <= -phi {
        [0.0, 0.0, cox / 2.0]
    } else if vgst <= -phi / 2.0 {
        [0.0, 0.0, -vgst * cox / (2.0 * phi)]
    } else if vgst <= 0.0 {
        let mut cgs = vgst * cox / (1.5 * phi) + cox / 3.0;
        let mut cgd = 0.0;
        let cgb = -vgst * cox / (2.0 * phi);
        if vds < vdsat {
            let vddif = 2.0 * vdsat - vds;
            let vddif2 = vddif * vddif;
            cgd = cgs * (1.0 - vdsat * vdsat / vddif2);
            cgs *= 1.0 - (vdsat - vds) * (vdsat - vds) / vddif2;
        }
        [cgs, cgd, cgb]
    } else if vdsat <= vds {
        [cox / 3.0, 0.0, 0.0]
    } else {
        let vddif = 2.0 * vdsat - vds;
        let vddif2 = vddif * vddif;
        [
            cox * (1.0 - (vdsat - vds) * (vdsat - vds) / vddif2) / 3.0,
            cox * (1.0 - vdsat * vdsat / vddif2) / 3.0,
            0.0,
        ]
    }
}

/// Terminal charges and their derivatives for one intrinsic MOSFET channel.
///
/// `q` is ordered gate, physical drain, physical source, bulk. `dq_dv` columns
/// are derivatives with respect to physical `VGS`, `VDS`, and `VBS`; rows use
/// the same order as `q`. For the Ward–Dutton model the four terminal charges
/// sum to zero at every bias, and the Jacobian rows do too.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MosChannelChargeOp {
    /// Intrinsic terminal charges, coulombs.
    pub q: [f64; 4],
    /// `∂Qterminal/∂(VGS,VDS,VBS)`, farads.
    pub dq_dv: [[f64; 3]; 4],
}

/// Forward-mode dual number for the three independent MOSFET terminal
/// voltages. It keeps the charge Jacobian analytic across the smooth portions
/// of the piecewise long-channel model without a finite-difference step size.
#[derive(Debug, Clone, Copy)]
struct Dual3 {
    value: f64,
    derivative: [f64; 3],
}

impl Dual3 {
    fn constant(value: f64) -> Self {
        Self {
            value,
            derivative: [0.0; 3],
        }
    }

    fn variable(value: f64, index: usize) -> Self {
        let mut derivative = [0.0; 3];
        derivative[index] = 1.0;
        Self { value, derivative }
    }

    fn sqrt(self) -> Self {
        let root = libm::sqrt(self.value);
        Self {
            value: root,
            derivative: self.derivative.map(|partial| partial / (2.0 * root)),
        }
    }
}

impl core::ops::Add for Dual3 {
    type Output = Self;

    fn add(self, rhs: Self) -> Self::Output {
        Self {
            value: self.value + rhs.value,
            derivative: core::array::from_fn(|i| self.derivative[i] + rhs.derivative[i]),
        }
    }
}

impl core::ops::Sub for Dual3 {
    type Output = Self;

    fn sub(self, rhs: Self) -> Self::Output {
        Self {
            value: self.value - rhs.value,
            derivative: core::array::from_fn(|i| self.derivative[i] - rhs.derivative[i]),
        }
    }
}

impl core::ops::Mul for Dual3 {
    type Output = Self;

    #[allow(clippy::suspicious_arithmetic_impl)] // product rule for forward derivatives
    fn mul(self, rhs: Self) -> Self::Output {
        Self {
            value: self.value * rhs.value,
            derivative: core::array::from_fn(|i| {
                self.derivative[i] * rhs.value + self.value * rhs.derivative[i]
            }),
        }
    }
}

impl core::ops::Div for Dual3 {
    type Output = Self;

    fn div(self, rhs: Self) -> Self::Output {
        let denominator = rhs.value * rhs.value;
        Self {
            value: self.value / rhs.value,
            derivative: core::array::from_fn(|i| {
                (self.derivative[i] * rhs.value - self.value * rhs.derivative[i]) / denominator
            }),
        }
    }
}

impl core::ops::Neg for Dual3 {
    type Output = Self;

    fn neg(self) -> Self::Output {
        Self {
            value: -self.value,
            derivative: self.derivative.map(|partial| -partial),
        }
    }
}

/// Ward–Dutton partition of the level-1 long-channel inversion charge.
///
/// The channel charge density follows the same square-law gradual-channel
/// relation as [`mos_op`]. In strong inversion, the channel's total charge and
/// drain share are integrated along the channel; the source share is the
/// remainder. This conserves intrinsic terminal charge and gives a consistent
/// analytic Jacobian for the MNA transient stamp. It deliberately models only
/// inversion charge: accumulation/depletion gate charge remains available via
/// the default Meyer formulation, selected by `CHARGEMOD=MEYER`.
pub fn mos_ward_dutton_channel_charge(
    mosfet: &Mosfet,
    vgs: f64,
    vds: f64,
    vbd: f64,
    vbs: f64,
) -> MosChannelChargeOp {
    if mosfet.model.channel_charge_model != MosChannelChargeModel::WardDutton {
        return MosChannelChargeOp {
            q: [0.0; 4],
            dq_dv: [[0.0; 3]; 4],
        };
    }
    let Some(tox) = mosfet.model.tox else {
        return MosChannelChargeOp {
            q: [0.0; 4],
            dq_dv: [[0.0; 3]; 4],
        };
    };

    let vgs = Dual3::variable(vgs, 0);
    let vds = Dual3::variable(vds, 1);
    let vbs = Dual3::variable(vbs, 2);
    let sign = mosfet.model.polarity.sign();
    let reversed = vds.value < 0.0;
    let (vgs_eff, vds_eff, vbs_eff) = if reversed {
        (
            vgs - vds,
            -vds,
            // `vbd` is the normalized bulk-to-physical-drain voltage; in
            // reverse operation the physical drain is the effective source.
            Dual3 {
                value: vbd,
                derivative: [0.0, -1.0, 1.0],
            },
        )
    } else {
        (vgs, vds, vbs)
    };

    let model = mosfet.model;
    let threshold = if model.gamma == 0.0 {
        Dual3::constant(sign * model.vto)
    } else {
        let sqrt_phi = libm::sqrt(model.phi);
        let body_root = if vbs_eff.value <= 0.0 {
            (Dual3::constant(model.phi) - vbs_eff).sqrt()
        } else {
            let linear = Dual3::constant(sqrt_phi) - vbs_eff / Dual3::constant(2.0 * sqrt_phi);
            if linear.value < 0.0 {
                Dual3::constant(0.0)
            } else {
                linear
            }
        };
        Dual3::constant(sign * model.vto)
            + Dual3::constant(model.gamma) * (body_root - Dual3::constant(sqrt_phi))
    };
    let overdrive = vgs_eff - threshold;
    if overdrive.value <= 0.0 {
        return MosChannelChargeOp {
            q: [0.0; 4],
            dq_dv: [[0.0; 3]; 4],
        };
    }

    let channel_drop = if vds_eff.value >= overdrive.value {
        overdrive
    } else {
        vds_eff
    };
    let r = channel_drop / overdrive;
    let r2 = r * r;
    let cox = Dual3::constant(OXIDE_PERMITTIVITY_F_PER_M * mosfet.w * mosfet.l / tox);
    let total_magnitude = cox * overdrive * (Dual3::constant(1.0) - r + r2 / Dual3::constant(3.0))
        / (Dual3::constant(1.0) - r / Dual3::constant(2.0));
    let drain_share = (Dual3::constant(0.5) - Dual3::constant(2.5) * r / Dual3::constant(3.0)
        + Dual3::constant(0.5) * r2
        - r2 * r / Dual3::constant(10.0))
        / ((Dual3::constant(1.0) - r / Dual3::constant(2.0))
            * (Dual3::constant(1.0) - r / Dual3::constant(2.0)));
    let drain_magnitude = cox * overdrive * drain_share;
    let qg = total_magnitude;
    let qd_eff = -drain_magnitude;
    // Form source charge as the exact remainder, so the charge vector closes
    // by construction rather than relying on two separately rounded formulas.
    let qs_eff = -(qg + qd_eff);
    let qb = Dual3::constant(0.0);
    let mut terminal = if reversed {
        [qg, qs_eff, qd_eff, qb]
    } else {
        [qg, qd_eff, qs_eff, qb]
    };
    for q in &mut terminal {
        q.value *= sign;
    }

    MosChannelChargeOp {
        q: terminal.map(|q| q.value),
        // The physical charge and physical voltage polarity both change sign
        // for PMOS, so their signs cancel in the capacitance Jacobian.
        dq_dv: terminal.map(|q| q.derivative),
    }
}

// ---------------------------------------------------------------------------
// JFET
// ---------------------------------------------------------------------------

/// One level-1 JFET's channel linearisation, in normalised N-channel
/// coordinates with the effective drain and source already selected.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct JfetOp {
    /// Channel current, effective drain → source, amps.
    pub id: f64,
    /// `∂Id/∂Vgs`, siemens.
    pub gm: f64,
    /// `∂Id/∂Vds`, siemens.
    pub gds: f64,
}

/// SPICE level-1 JFET with the default Parker–Skellern fit `B=1`.
///
/// This five-argument wrapper is retained for callers of the original equation.
pub fn jfet_op(vgs: f64, vds: f64, vto: f64, beta: f64, lambda: f64) -> JfetOp {
    jfet_op_with_b(vgs, vds, vto, beta, lambda, 1.0, 1.0)
}

/// SPICE JFET level 1 with the Parker–Skellern fitting parameter `B`:
///
/// ```text
/// vgst = Vgs − VTO
/// βp   = BETA·(1 + LAMBDA·Vds)
///
/// bfac = (1 − B) / (PB − VTO)
/// cutoff      vgst ≤ 0       Id = 0
/// linear      vgst ≥ Vds     Id = βp·Vds·[Vds·(bfac·Vds − B)
///                                               + vgst·(2B + 3bfac·(vgst−Vds))]
/// saturation  vgst < Vds     Id = βp·vgst²·(B + vgst·bfac)
/// ```
///
/// Gate-junction current and the channel's GMIN path are stamped separately by
/// the MNA solver. `vto` is negative for an N-channel JFET; the caller mirrors
/// voltages and currents for a P-channel device. `pb` is the gate-junction
/// potential used by the Parker–Skellern fit.
pub fn jfet_op_with_b(
    vgs: f64,
    vds: f64,
    vto: f64,
    beta: f64,
    lambda: f64,
    b: f64,
    pb: f64,
) -> JfetOp {
    let vgst = vgs - vto;
    if vgst <= 0.0 {
        return JfetOp {
            id: 0.0,
            gm: 0.0,
            gds: 0.0,
        };
    }

    let beta_p = beta * (1.0 + lambda * vds);
    let bfac = if b == 1.0 {
        0.0
    } else {
        (1.0 - b) / (pb - vto)
    };
    if vgst >= vds {
        let apart = 2.0 * b + 3.0 * bfac * (vgst - vds);
        let cpart = vds * (vds * (bfac * vds - b) + vgst * apart);
        JfetOp {
            id: beta_p * cpart,
            gm: beta_p * vds * (apart + 3.0 * bfac * vgst),
            gds: beta_p * (vgst - vds) * apart + beta * lambda * cpart,
        }
    } else {
        let bfac_vgst = bfac * vgst;
        let cpart = vgst * vgst * (b + bfac_vgst);
        JfetOp {
            id: beta_p * cpart,
            gm: beta_p * vgst * (2.0 * b + 3.0 * bfac_vgst),
            gds: beta * lambda * cpart,
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
