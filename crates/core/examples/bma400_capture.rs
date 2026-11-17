// Copyright (C) 2026 LabWired. MIT.
// CowManager BMA400 browser example: a capture from the SAME generic SPI
// register model used by LabWired, with clearly labelled ideal byte-timed
// SPI lines. No synthetic chip ID or fabricated register responses.
use std::io::{self, Read};

use anyhow::{bail, Context, Result};
use labwired_core::analog::{parse_netlist, Integration, Solver};
use labwired_core::peripherals::components::declarative_spi::GenericSpiDevice;
use labwired_core::peripherals::device::SpiDevice;
use labwired_core::sim_input::SimInput;
use serde::Deserialize;
use serde_json::{json, Value};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Scenario {
    samples: Vec<Stimulus>,
    #[serde(default = "default_hz")]
    spi_hz: f64,
}
fn default_hz() -> f64 {
    1_000_000.0
}

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct Stimulus {
    time_us: f64,
    accel_x_g: f64,
    accel_y_g: f64,
    accel_z_g: f64,
    #[serde(default = "default_vdd")]
    vdd_v: f64,
    #[serde(default = "default_vdd")]
    vddio_v: f64,
}
fn default_vdd() -> f64 {
    3.3
}

#[derive(Default)]
struct Trace {
    cs: Vec<Value>,
    sck: Vec<Value>,
    mosi: Vec<Value>,
    miso: Vec<Value>,
    transactions: Vec<Value>,
}
impl Trace {
    fn push(signal: &mut Vec<Value>, t: f64, v: u8) {
        if signal
            .last()
            .and_then(|x| x.get("v"))
            .and_then(Value::as_u64)
            != Some(u64::from(v))
        {
            signal.push(json!({"t_us": round(t), "v": v}));
        }
    }
    fn spi(
        &mut self,
        dev: &mut GenericSpiDevice,
        title: &str,
        at: f64,
        tx: &[u8],
        hz: f64,
    ) -> (Vec<u8>, f64) {
        let bit_us = 1_000_000.0 / hz;
        let mut t = at;
        dev.cs_select();
        Self::push(&mut self.cs, t, 0);
        Self::push(&mut self.sck, t, 0);
        let mut rx = Vec::with_capacity(tx.len());
        for &byte in tx {
            let reply = dev.transfer(byte);
            rx.push(reply);
            for bit in (0..8).rev() {
                Self::push(&mut self.mosi, t, (byte >> bit) & 1);
                Self::push(&mut self.miso, t, (reply >> bit) & 1);
                t += bit_us / 2.0;
                Self::push(&mut self.sck, t, 1);
                t += bit_us / 2.0;
                Self::push(&mut self.sck, t, 0);
            }
        }
        dev.cs_release();
        Self::push(&mut self.cs, t, 1);
        let tx_hex: Vec<String> = tx.iter().map(|v| format!("{v:02X}")).collect();
        let rx_hex: Vec<String> = rx.iter().map(|v| format!("{v:02X}")).collect();
        self.transactions.push(json!({
            "name": title, "start_us": round(at), "end_us": round(t),
            "mosi": tx_hex, "miso": rx_hex,
        }));
        (rx, t)
    }
}

fn round(v: f64) -> f64 {
    (v * 1000.0).round() / 1000.0
}

fn capture(scenario: Scenario) -> Result<Value> {
    if scenario.samples.is_empty() || scenario.samples.len() > 400 {
        bail!("samples must contain 1–400 entries");
    }
    if !scenario.spi_hz.is_finite() || !(100_000.0..=8_000_000.0).contains(&scenario.spi_hz) {
        bail!("spi_hz must be 100000..8000000");
    }
    let mut last = -1.0;
    for (i, x) in scenario.samples.iter().enumerate() {
        if !x.time_us.is_finite()
            || x.time_us < 0.0
            || x.time_us <= last
            || x.time_us > 10_000_000.0
        {
            bail!("samples[{i}].time_us must strictly increase within 0..10000000");
        }
        last = x.time_us;
        for (name, n, lo, hi) in [
            ("accel_x_g", x.accel_x_g, -16.0, 16.0),
            ("accel_y_g", x.accel_y_g, -16.0, 16.0),
            ("accel_z_g", x.accel_z_g, -16.0, 16.0),
            ("vdd_v", x.vdd_v, 0.0, 4.0),
            ("vddio_v", x.vddio_v, 0.0, 4.0),
        ] {
            if !n.is_finite() || !(lo..=hi).contains(&n) {
                bail!("samples[{i}].{name} is outside {lo}..{hi}");
            }
        }
    }

    // Real in-core analog MNA solver, same Solver as LabWired's oscilloscope.
    // Generic RC supply fixture (not a CowManager PCB model). The CSV values
    // drive separate VDD and VDDIO voltage sources; these are measured at the
    // decoupled nodes below. 4.7 ohm + 22uF is a deliberately visible fixture
    // time constant, not a claim about the customer's hardware.
    let netlist = r#"
VVDDSRC vdd_source 0 DC 3.3
RVDDS vdd_source vdd 4.7
CVDDS vdd 0 22u
RVDDBLEED vdd 0 150k
VVIOSRC vio_source 0 DC 3.3
RVIOS vio_source vddio 4.7
CVIOS vddio 0 22u
RVIOBLEED vddio 0 150k
.end
"#;
    let circuit = parse_netlist(netlist).context("parse analog supply fixture")?;
    let mut solver = Solver::new(circuit, Integration::BackwardEuler)
        .context("initialize analog supply solver")?;
    let vdd_idx = solver
        .voltage_source_index("VVDDSRC")
        .context("missing VDD source")?;
    let vio_idx = solver
        .voltage_source_index("VVIOSRC")
        .context("missing VDDIO source")?;
    let vdd_node = solver.circuit().node("vdd").context("missing VDD probe")?;
    let vio_node = solver
        .circuit()
        .node("vddio")
        .context("missing VDDIO probe")?;
    let mut analog_out = Vec::with_capacity(scenario.samples.len());
    let mut solver_time_us = 0.0;

    let descriptor =
        labwired_config::embedded_device_yaml("bma400").context("BMA400 not registered")?;
    let mut device = GenericSpiDevice::from_yaml(descriptor, "CSB")?;
    let mut trace = Trace::default();
    Trace::push(&mut trace.cs, 0.0, 1);
    Trace::push(&mut trace.sck, 0.0, 0);
    Trace::push(&mut trace.mosi, 0.0, 0);
    Trace::push(&mut trace.miso, 0.0, 1);

    // BMA400 after reset selects SPI only after first CS rising edge.
    let (first, mut t) = trace.spi(
        &mut device,
        "Before SPI activation (expected invalid)",
        1.0,
        &[0x80, 0x00, 0x00],
        scenario.spi_hz,
    );
    let first_invalid = first == [0xFF, 0xFF, 0xFF];
    let (id, end) = trace.spi(
        &mut device,
        "Read CHIP_ID at 0x00 (command + dummy + data)",
        t + 5.0,
        &[0x80, 0x00, 0x00],
        scenario.spi_hz,
    );
    let id_ok = id.len() == 3 && id[1] == 0x00 && id[2] == 0x90;
    t = end;
    let (_, end) = trace.spi(
        &mut device,
        "ACC_CONFIG1: ±4 g, default 0x49",
        t + 5.0,
        &[0x1A, 0x49],
        scenario.spi_hz,
    );
    let (_, end) = trace.spi(
        &mut device,
        "ACC_CONFIG0: enable normal mode 0x02",
        end + 5.0,
        &[0x19, 0x02],
        scenario.spi_hz,
    );
    t = end;

    let mut samples_out = Vec::with_capacity(scenario.samples.len());
    let mut counts_ok = true;
    for (i, sample) in scenario.samples.iter().enumerate() {
        // Advance under the PREVIOUS source level up to this timestamp. Then
        // apply this CSV row's source step; a capacitor cannot jump instantly.
        let advance_us = sample.time_us - solver_time_us;
        if advance_us > 0.0 {
            solver.advance(advance_us * 1e-6)?;
        }
        solver.set_voltage_source(vdd_idx, sample.vdd_v);
        solver.set_voltage_source(vio_idx, sample.vddio_v);
        solver.advance(1e-9)?;
        solver_time_us = sample.time_us + 0.001;
        let vdd_rail = solver.node_voltage(vdd_node);
        let vio_rail = solver.node_voltage(vio_node);
        analog_out.push(json!({
            "t_us": sample.time_us,
            "accel_x_g": sample.accel_x_g, "accel_y_g": sample.accel_y_g,
            "accel_z_g": sample.accel_z_g,
            "vdd_v": round(vdd_rail), "vddio_v": round(vio_rail),
            "vdd_input_v": sample.vdd_v, "vddio_input_v": sample.vddio_v,
        }));
        for (ch, v) in [
            ("accel_x", sample.accel_x_g),
            ("accel_y", sample.accel_y_g),
            ("accel_z", sample.accel_z_g),
        ] {
            device.set_input(ch, v)?;
        }
        // Do not make a "sample" appear to happen before any earlier SPI
        // transaction. The analog values retain their supplied stimulus time.
        let start = (sample.time_us + 2.0).max(t + 4.0);
        let (rx, end) = trace.spi(
            &mut device,
            &format!("Sample {i}: ACC_X/Y/Z at 0x04"),
            start,
            &[0x84, 0x00, 0, 0, 0, 0, 0, 0],
            scenario.spi_hz,
        );
        t = end;
        let mut counts = Vec::new();
        for axis in 0..3 {
            let word = u16::from(rx[2 + axis * 2]) | u16::from(rx[3 + axis * 2]) << 8;
            let n = word & 0x0FFF;
            counts.push(if n & 0x0800 != 0 {
                (n as i16) - 4096
            } else {
                n as i16
            });
        }
        let expected: Vec<i16> = [sample.accel_x_g, sample.accel_y_g, sample.accel_z_g]
            .iter()
            .map(|x| (x * 512.0).round().clamp(-2048.0, 2047.0) as i16)
            .collect();
        counts_ok &= counts == expected;
        samples_out.push(json!({
            "stimulus_time_us": sample.time_us,
            "read_time_us": round(start),
            "accel_g": [sample.accel_x_g, sample.accel_y_g, sample.accel_z_g],
            "counts": counts,
            "expected_counts": expected,
            "pass": counts == expected
        }));
    }
    Ok(json!({
        "model": "Bosch BMA400 (LabWired declarative SPI engine)",
        "trace_kind": "simulated register transfers; SPI edges reconstructed at ideal clock rate",
        "analog_kind": "CSV acceleration stimulus plus real LabWired MNA RC supply transient solution; NOT MEMS analog circuit, customer PCB or measurement",
        "analog_fixture": "VDD/VDDIO: separate 4.7-ohm + 22-uF decoupling, 150-kohm bleed; ideal programmable sources; NOT CowManager hardware",

        "device_id": format!("0x{:02X}", id[2]),
        "test": { "initial_spi_blocked": first_invalid, "chip_id_0x90": id_ok,
                  "acceleration_registers": counts_ok, "pass": first_invalid && id_ok && counts_ok },
        "spi_hz": scenario.spi_hz,
        "samples": samples_out,
        "analog": analog_out,
        "digital": { "cs": trace.cs, "sck": trace.sck, "mosi": trace.mosi, "miso": trace.miso },
        "transactions": trace.transactions,
        "end_us": round(t),
    }))
}

fn main() {
    if let Err(e) = run() {
        eprintln!("{e:#}");
        std::process::exit(1);
    }
}
fn run() -> Result<()> {
    let mut input = String::new();
    io::stdin()
        .take(256 * 1024)
        .read_to_string(&mut input)
        .context("read scenario JSON from stdin")?;
    let scenario: Scenario = serde_json::from_str(&input).context("parse scenario JSON")?;
    let response = capture(scenario)?;
    println!("{}", serde_json::to_string(&response)?);
    Ok(())
}
