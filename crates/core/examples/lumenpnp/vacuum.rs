//! Lumped pneumatic plant for the LumenPnP demonstration.
//!
//! These parameters are declared engineering assumptions. The geometry supplies
//! nozzle contact and aperture, and the native brushed DC motor supplies shaft
//! speed. The pressure state integrates mass balance for a fixed-volume chamber.

#[derive(Clone, Copy, Debug)]
pub struct VacuumParams {
    pub ambient_pressure_pa: f64,
    pub ultimate_pressure_pa: f64,
    pub volume_m3: f64,
    pub nominal_pump_flow_m3_s: f64,
    pub nominal_pump_speed_rad_s: f64,
    pub open_nozzle_conductance_m3_s: f64,
    pub sealed_leak_conductance_m3_s: f64,
    pub vent_conductance_m3_s: f64,
    pub nozzle_bore_radius_m: f64,
    pub max_pump_load_torque_nm: f64,
}

impl Default for VacuumParams {
    fn default() -> Self {
        Self {
            ambient_pressure_pa: 101_325.0,
            ultimate_pressure_pa: 31_325.0,
            volume_m3: 10.0e-6,
            nominal_pump_flow_m3_s: 60.0e-6,
            nominal_pump_speed_rad_s: 700.0,
            open_nozzle_conductance_m3_s: 420.0e-6,
            sealed_leak_conductance_m3_s: 0.3e-6,
            vent_conductance_m3_s: 400.0e-6,
            nozzle_bore_radius_m: 0.25e-3,
            max_pump_load_torque_nm: 0.005,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct VacuumSnapshot {
    pub pressure_pa: f64,
    pub suction_force_n: f64,
    pub pump_load_torque_nm: f64,
}

pub struct VacuumPlant {
    params: VacuumParams,
    absolute_pressure_pa: f64,
}

impl VacuumPlant {
    pub fn new(params: VacuumParams) -> Self {
        assert!(params.ambient_pressure_pa > params.ultimate_pressure_pa);
        assert!(params.ultimate_pressure_pa > 0.0);
        assert!(params.volume_m3 > 0.0);
        assert!(params.nominal_pump_speed_rad_s > 0.0);
        assert!(params.nominal_pump_flow_m3_s >= 0.0);
        assert!(params.open_nozzle_conductance_m3_s >= 0.0);
        assert!(params.sealed_leak_conductance_m3_s >= 0.0);
        assert!(params.vent_conductance_m3_s >= 0.0);
        assert!(params.nozzle_bore_radius_m > 0.0);
        assert!(params.max_pump_load_torque_nm >= 0.0);
        Self {
            absolute_pressure_pa: params.ambient_pressure_pa,
            params,
        }
    }

    /// Evolve pressure with constant inputs over this interval. `valve_vacuum`
    /// means the energized valve connects the nozzle to the pump. Deenergizing
    /// it vents the nozzle. An open nozzle supplies the high-conductance leak.
    pub fn step(
        &mut self,
        dt_s: f64,
        pump_speed_rad_s: f64,
        valve_vacuum: bool,
        seal_closed: bool,
    ) -> VacuumSnapshot {
        assert!(dt_s.is_finite() && dt_s >= 0.0);
        assert!(pump_speed_rad_s.is_finite());
        let p = self.params;
        let pump_flow = if valve_vacuum {
            p.nominal_pump_flow_m3_s * (pump_speed_rad_s.max(0.0) / p.nominal_pump_speed_rad_s)
        } else {
            0.0
        };
        let leak_flow = if !valve_vacuum {
            p.vent_conductance_m3_s
        } else if seal_closed {
            p.sealed_leak_conductance_m3_s
        } else {
            p.open_nozzle_conductance_m3_s
        };
        let total_flow = pump_flow + leak_flow;
        if total_flow > 0.0 {
            let equilibrium = (pump_flow * p.ultimate_pressure_pa
                + leak_flow * p.ambient_pressure_pa)
                / total_flow;
            self.absolute_pressure_pa = equilibrium
                + (self.absolute_pressure_pa - equilibrium)
                    * libm::exp(-total_flow * dt_s / p.volume_m3);
        }
        self.snapshot(valve_vacuum)
    }

    pub fn snapshot(&self, valve_vacuum: bool) -> VacuumSnapshot {
        let p = self.params;
        let differential = (p.ambient_pressure_pa - self.absolute_pressure_pa).max(0.0);
        let load_fraction = differential / (p.ambient_pressure_pa - p.ultimate_pressure_pa);
        VacuumSnapshot {
            pressure_pa: self.absolute_pressure_pa - p.ambient_pressure_pa,
            suction_force_n: differential * std::f64::consts::PI * p.nozzle_bore_radius_m.powi(2),
            pump_load_torque_nm: if valve_vacuum {
                p.max_pump_load_torque_nm * load_fraction
            } else {
                0.0
            },
        }
    }
}

impl Default for VacuumPlant {
    fn default() -> Self {
        Self::new(VacuumParams::default())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stopped_pump_leaves_ambient_and_sealed_pump_builds_vacuum() {
        let mut plant = VacuumPlant::default();
        assert_eq!(plant.step(1.0, 0.0, true, true).pressure_pa, 0.0);
        let sealed = plant.step(0.5, 700.0, true, true);
        assert!(sealed.pressure_pa < -60_000.0);
        assert!(sealed.suction_force_n > 0.010);
        assert!(sealed.pump_load_torque_nm > 0.004);
    }

    #[test]
    fn open_nozzle_cannot_reach_pickup_threshold_and_vent_releases() {
        let mut plant = VacuumPlant::default();
        let open = plant.step(1.0, 700.0, true, false);
        assert!(open.pressure_pa > -10_000.0);
        assert!(open.pressure_pa < -8_000.0);
        plant.step(1.0, 700.0, true, true);
        let vented = plant.step(0.3, 700.0, false, true);
        assert!(vented.pressure_pa.abs() < 1.0);
        assert_eq!(vented.pump_load_torque_nm, 0.0);
    }

    #[test]
    fn constant_input_solution_is_independent_of_time_partition() {
        let mut one = VacuumPlant::default();
        let mut many = VacuumPlant::default();
        let expected = one.step(0.5, 700.0, true, true);
        for _ in 0..500 {
            many.step(0.001, 700.0, true, true);
        }
        assert!((expected.pressure_pa - many.snapshot(true).pressure_pa).abs() < 1.0e-8);
    }
}
