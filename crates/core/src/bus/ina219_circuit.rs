//! Circuit-coupled INA219: netlist shunt current + bus voltage → SimInput,
//! averaged over the conversion time so fast PWM reads as its mean.
//!
//! Cosim routes `board.ina219.<id>.current_a` and `board.ina219.<id>.bus_v`
//! from the analog island's shunt probe. Each co-sim step feeds one sample;
//! after [`INA219_CONVERSION_NS`] (12-bit continuous, 532 µs) the averages are
//! latched into the declarative device's `current` / `bus_voltage` channels so
//! the existing register encodings (CAL 4096, Adafruit 32V/2A) stay unchanged.

use super::*;

/// 12-bit ADC conversion time in continuous shunt+bus mode (datasheet
/// SBOS448G Table 4: 532 µs). Matches the Adafruit default CONFIG and the
/// firmware comment that one conversion averages many 31.4 kHz PWM periods.
pub(crate) const INA219_CONVERSION_NS: u64 = 532_000;

/// Why `board.ina219.<id>.…` could not bind.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CircuitIna219Lookup {
    /// No external device with that id and type `ina219`.
    Unknown,
}

/// Running average for one circuit-wired INA219 breakout.
#[derive(Debug)]
pub(crate) struct Ina219CircuitSense {
    id: String,
    sum_current_a: f64,
    sum_bus_v: f64,
    samples: u64,
    window_start_ns: u64,
    pending_current_a: Option<f64>,
    pending_bus_v: Option<f64>,
    dirty: bool,
}

impl Ina219CircuitSense {
    fn new(id: String) -> Self {
        Self {
            id,
            sum_current_a: 0.0,
            sum_bus_v: 0.0,
            samples: 0,
            window_start_ns: 0,
            pending_current_a: None,
            pending_bus_v: None,
            dirty: false,
        }
    }
}

impl SystemBus {
    /// Resolve `board.ina219.<id>.…` against declared external devices.
    pub(crate) fn circuit_ina219_id<'a>(
        &self,
        id: &'a str,
    ) -> Result<&'a str, CircuitIna219Lookup> {
        let ok = self
            .external_device_decls
            .iter()
            .any(|d| d.id == id && d.device_type == "ina219");
        if ok {
            Ok(id)
        } else {
            Err(CircuitIna219Lookup::Unknown)
        }
    }

    fn ina219_sense_mut(&mut self, id: &str) -> Option<&mut Ina219CircuitSense> {
        if let Some(i) = self.ina219_senses.iter().position(|s| s.id == id) {
            return self.ina219_senses.get_mut(i);
        }
        if self.circuit_ina219_id(id).is_err() {
            return None;
        }
        self.ina219_senses
            .push(Ina219CircuitSense::new(id.to_string()));
        self.ina219_senses.last_mut()
    }

    /// Instantaneous shunt current from the netlist (amperes).
    pub(crate) fn feed_ina219_current(&mut self, id: &str, current_a: f64) -> bool {
        if !current_a.is_finite() {
            return false;
        }
        let Some(sense) = self.ina219_sense_mut(id) else {
            return false;
        };
        sense.pending_current_a = Some(current_a);
        sense.dirty = true;
        true
    }

    /// Instantaneous bus voltage from the netlist (volts at VIN−).
    pub(crate) fn feed_ina219_bus_v(&mut self, id: &str, bus_v: f64) -> bool {
        if !bus_v.is_finite() {
            return false;
        }
        let Some(sense) = self.ina219_sense_mut(id) else {
            return false;
        };
        sense.pending_bus_v = Some(bus_v);
        sense.dirty = true;
        true
    }

    /// Take one sample per dirty sense and latch averages when the conversion
    /// window completes. Called once per co-sim apply with the step's `time_ns`.
    pub(crate) fn flush_ina219_circuit(&mut self, time_ns: u64) {
        let mut latches: Vec<(String, f64, f64)> = Vec::new();
        for sense in &mut self.ina219_senses {
            if !sense.dirty {
                continue;
            }
            sense.dirty = false;
            let i = sense.pending_current_a.unwrap_or(0.0);
            let v = sense.pending_bus_v.unwrap_or(0.0);
            if sense.samples == 0 {
                sense.window_start_ns = time_ns;
            }
            sense.sum_current_a += i;
            sense.sum_bus_v += v;
            sense.samples += 1;
            let elapsed = time_ns.saturating_sub(sense.window_start_ns);
            if elapsed >= INA219_CONVERSION_NS && sense.samples > 0 {
                let n = sense.samples as f64;
                let avg_i = sense.sum_current_a / n;
                let avg_v = sense.sum_bus_v / n;
                sense.sum_current_a = 0.0;
                sense.sum_bus_v = 0.0;
                sense.samples = 0;
                sense.window_start_ns = time_ns;
                latches.push((sense.id.clone(), avg_i, avg_v));
            }
        }
        for (id, avg_i, avg_v) in latches {
            let _ = self.set_inputs(&[
                (Some(id.as_str()), "current", avg_i),
                (Some(id.as_str()), "bus_voltage", avg_v),
            ]);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::INA219_CONVERSION_NS;

    #[test]
    fn conversion_window_is_twelve_bit_datasheet_value() {
        assert_eq!(INA219_CONVERSION_NS, 532_000);
    }
}
