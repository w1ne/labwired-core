//! The flag tracks actual clipping in effective channel coordinates.
use super::*;
fn fixture(kind: &str, sign: f64) -> Solver {
    Solver::new(super::super::parse_netlist(&format!(".model channel {kind}(VTO={} KP=1 IS=1e-15)\nVg gate 0 0\nVd drain 0 0\nM1 drain gate 0 0 channel\n",sign)).unwrap(),Integration::BackwardEuler).unwrap()
}
fn sample(s: &mut Solver, sign: f64, gs: f64, ds: f64) {
    let gate = s.circuit.node("gate").unwrap().unwrap();
    let drain = s.circuit.node("drain").unwrap().unwrap();
    s.solution[gate] = sign * gs;
    s.solution[drain] = sign * ds;
    s.mos_vgs[0] = 0.0;
    s.mos_vds[0] = 0.0;
    // Keep the body junctions at their present Newton voltages, so these
    // witnesses isolate the channel limiter rather than a pn limiter.
    s.mos_vbd[0] = -ds;
    s.mos_vbs[0] = 0.0;
    s.relinearise();
}
#[test]
fn near_gate_limit_keeps_both_unclipped_and_clipped_paths_in_both_orientations() {
    for (kind, sign) in [("NMOS", 1.0), ("PMOS", -1.0)] {
        for orientation in [-1.0, 1.0] {
            for (effective, clipped) in [(1.5 - 1e-6, false), (1.5 + 1e-6, true)] {
                let mut s = fixture(kind, sign);
                let ds = orientation * 0.2;
                let gs = if ds < 0.0 { effective + ds } else { effective };
                sample(&mut s, sign, gs, ds);
                assert_eq!(s.limiter_clamped, clipped);
                if clipped {
                    let actual = if ds < 0.0 {
                        s.mos_vgs[0] - s.mos_vds[0]
                    } else {
                        s.mos_vgs[0]
                    };
                    assert!((actual - 1.5).abs() < 1e-14);
                } else {
                    assert_eq!(s.mos_vgs[0], gs);
                    assert_eq!(s.mos_vds[0], ds);
                }
            }
        }
    }
}
#[test]
fn near_drain_limit_keeps_both_unclipped_and_clipped_paths_in_both_orientations() {
    for (kind, sign) in [("NMOS", 1.0), ("PMOS", -1.0)] {
        for orientation in [-1.0, 1.0] {
            for (magnitude, clipped) in [(4.0 - 1e-6, false), (4.0 + 1e-6, true)] {
                let mut s = fixture(kind, sign);
                let ds = orientation * magnitude;
                let gs = if ds < 0.0 { 0.1 + ds } else { 0.1 };
                sample(&mut s, sign, gs, ds);
                assert_eq!(s.limiter_clamped, clipped);
                if clipped {
                    assert_eq!(s.mos_vds[0], orientation * 4.0);
                } else {
                    assert_eq!(s.mos_vgs[0], gs);
                    assert_eq!(s.mos_vds[0], ds);
                }
            }
        }
    }
}
