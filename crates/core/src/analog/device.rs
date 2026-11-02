// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Device physics for the nonlinear elements: the exponential diode, the
//! Ebers–Moll BJT, the level-1 JFET and MOSFET, plus the constants and the
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
//! no JFET `Cgs`/`Cgd`, no MOSFET overlap or channel charge — and no temperature
//! model: every parameter is taken at [`TEMPERATURE_K`] as written. A `.model` line may
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
//! `analog_devices.rs` pins representative diode, BJT and MOSFET operating
//! points as raw `f64` bit patterns and checks JFET regions against hand
//! calculations, so drift or an incorrect equation fails a test.

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

/// SPICE JFET level 1 with the Parker–Skellern fitting parameter `B = 1`:
///
/// ```text
/// vgst = Vgs − VTO
/// βp   = BETA·(1 + LAMBDA·Vds)
///
/// cutoff      vgst ≤ 0       Id = 0
/// linear      Vds < vgst     Id = βp·Vds·(2·vgst − Vds)
/// saturation  Vds ≥ vgst     Id = βp·vgst²
/// ```
///
/// Gate-junction current and the channel's GMIN path are stamped separately by
/// the MNA solver. `vto` is negative for an N-channel JFET; the caller mirrors
/// voltages and currents for a P-channel device.
pub fn jfet_op(vgs: f64, vds: f64, vto: f64, beta: f64, lambda: f64) -> JfetOp {
    let vgst = vgs - vto;
    if vgst <= 0.0 {
        return JfetOp {
            id: 0.0,
            gm: 0.0,
            gds: 0.0,
        };
    }

    let beta_p = beta * (1.0 + lambda * vds);
    if vds < vgst {
        JfetOp {
            id: beta_p * vds * (2.0 * vgst - vds),
            gm: 2.0 * beta_p * vds,
            gds: beta
                * (lambda * vds * (2.0 * vgst - vds)
                    + (1.0 + lambda * vds) * (2.0 * vgst - 2.0 * vds)),
        }
    } else {
        JfetOp {
            id: beta_p * vgst * vgst,
            gm: 2.0 * beta_p * vgst,
            gds: beta * lambda * vgst * vgst,
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
