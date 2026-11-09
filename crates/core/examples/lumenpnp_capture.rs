//! Executable LumenPnP verification. MCU -> driver edges -> CAD kinematics ->
//! contact / pneumatic state -> XGZP registers -> MCU pickup decisions.
//! Physical parameters are declared assumptions. Every trajectory follows MCU output.

#![recursion_limit = "256"]

#[path = "lumenpnp/vacuum.rs"]
mod vacuum;

use anyhow::{ensure, Context, Result};
use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::bus::SystemBus;
use labwired_core::cosim::CosimSession;
use labwired_core::logic_capture::LogicSource;
use labwired_core::peripherals::components::step_dir_motor::StepDirMotor;
use labwired_core::{AdvanceRequest, AdvanceStop, Bus, Cpu, DebugControl, Machine};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::fs::{self, File};
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Instant;
use vacuum::{VacuumParams, VacuumPlant};

const CPU_HZ: u64 = 168_000_000;
const BOARD_TRAVEL: i32 = 7;
const VACUUM_LOSS_RESPONSE_LIMIT_S: f64 = 0.100;
const TERMINAL_OBSERVATION_S: f64 = 0.300;
const AXIS_IDS: [&str; 6] = ["X", "Y1", "Y2", "Z", "A", "B"];
// Triplets STEP, DIR, active-low EN, in the verified PCB's physical pin map.
const PINS: [(&str, &str, u8); 25] = [
    ("X_STEP", "gpiob", 15),
    ("X_DIR", "gpiob", 14),
    ("X_EN", "gpiod", 9),
    ("Y1_STEP", "gpioe", 15),
    ("Y1_DIR", "gpioe", 14),
    ("Y1_EN", "gpiob", 13),
    ("Y2_STEP", "gpiod", 6),
    ("Y2_DIR", "gpiod", 7),
    ("Y2_EN", "gpioa", 3),
    ("Z_STEP", "gpioe", 7),
    ("Z_DIR", "gpiob", 1),
    ("Z_EN", "gpioe", 9),
    ("A_STEP", "gpioc", 4),
    ("A_DIR", "gpioa", 4),
    ("A_EN", "gpiob", 0),
    ("B_STEP", "gpioe", 11),
    ("B_DIR", "gpioe", 10),
    ("B_EN", "gpioe", 13),
    ("PUMP_A", "gpiob", 8),
    ("PUMP_B", "gpioe", 5),
    ("VALVE_A", "gpiob", 9),
    ("VALVE_B", "gpioe", 6),
    ("X_LIMIT", "gpioc", 6),
    ("Y_LIMIT", "gpiod", 15),
    ("Z_LIMIT", "gpiod", 14),
];

#[derive(Clone, Deserialize)]
struct Pocket {
    index: usize,
    center: [f64; 3],
    size: [f64; 3],
    populated: bool,
    mass_kg: f64,
    volume_mm3: f64,
}
#[derive(Clone, Deserialize)]
struct Target {
    #[serde(rename = "ref")]
    reference: String,
    center: [f64; 3],
    rotation_deg: f64,
    source_pocket: usize,
}
#[derive(Clone, Serialize)]
struct Part {
    pocket: usize,
    center: [f64; 3],
    size: [f64; 3],
    mass_kg: f64,
    rotation_deg: f64,
    attached_head: Option<usize>,
    placed: bool,
    dropped: bool,
    #[serde(skip)]
    grasp_offset: [f64; 3],
    #[serde(skip)]
    grasp_angle_deg: f64,
}

struct Geometry {
    board_surface_z: f64,
    board_bounds: [[f64; 3]; 2],
    head_separation_mm: f64,
    z_link_sum_mm: f64,
    xy_tolerance: f64,
    z_tolerance: f64,
    pockets: Vec<Pocket>,
    targets: Vec<Target>,
    parts: Vec<Part>,
    placements: Vec<Value>,
    drops: usize,
    contacts: [Option<usize>; 2],
}

impl Geometry {
    fn from_contract(contract: &Value) -> Result<Self> {
        let pockets: Vec<Pocket> = serde_json::from_value(contract["tray"]["pockets"].clone())?;
        let targets = serde_json::from_value(contract["targets"].clone())?;
        let density = contract["kinematics"]["package_density_kg_m3"]
            .as_f64()
            .context("declared package density")?;
        for pocket in &pockets {
            ensure!(
                (pocket.mass_kg - pocket.volume_mm3 * 1e-9 * density).abs() < 1e-10,
                "CAD mass and density disagree"
            );
        }
        let parts = pockets
            .iter()
            .filter(|p| p.populated)
            .map(|p| Part {
                pocket: p.index,
                center: p.center,
                size: p.size,
                mass_kg: p.mass_kg,
                rotation_deg: 0.0,
                attached_head: None,
                placed: false,
                dropped: false,
                grasp_offset: [0.0; 3],
                grasp_angle_deg: 0.0,
            })
            .collect();
        Ok(Self {
            board_surface_z: contract["board"]["surface_z"]
                .as_f64()
                .context("board surface")?,
            board_bounds: serde_json::from_value(contract["board"]["bounds"].clone())?,
            head_separation_mm: contract["kinematics"]["head_separation_mm"]
                .as_f64()
                .context("evaluated head separation")?,
            z_link_sum_mm: contract["kinematics"]["z_link_sum_mm"]
                .as_f64()
                .context("evaluated Z coupling")?,
            xy_tolerance: contract["contact"]["xy_tolerance"]
                .as_f64()
                .context("XY tolerance")?,
            z_tolerance: contract["contact"]["z_tolerance"]
                .as_f64()
                .context("Z tolerance")?,
            pockets,
            targets,
            parts,
            placements: vec![],
            drops: 0,
            contacts: [None; 2],
        })
    }

    fn tips(&self, axes: [f64; 5]) -> [[f64; 3]; 2] {
        [
            [axes[0], axes[1], self.board_surface_z + axes[2]],
            [
                axes[0] + self.head_separation_mm,
                axes[1],
                self.board_surface_z + self.z_link_sum_mm - axes[2],
            ],
        ]
    }

    fn attached(&self, head: usize) -> Option<usize> {
        self.parts
            .iter()
            .position(|p| p.attached_head == Some(head))
    }

    fn contact_at(&self, tip: [f64; 3]) -> Option<usize> {
        self.parts.iter().position(|p| {
            p.attached_head.is_none()
                && !p.placed
                && !p.dropped
                && (tip[0] - p.center[0]).abs() <= self.xy_tolerance
                && (tip[1] - p.center[1]).abs() <= self.xy_tolerance
                && (tip[2] - p.center[2] - p.size[2] * 0.5).abs() <= self.z_tolerance
        })
    }

    fn seals(
        &mut self,
        tips: [[f64; 3]; 2],
        time_s: f64,
        events: &mut impl Write,
    ) -> Result<[bool; 2]> {
        let mut seals = [false; 2];
        for head in 0..2 {
            let contact = self.attached(head).or_else(|| self.contact_at(tips[head]));
            if contact != self.contacts[head] {
                if let Some(index) = contact {
                    writeln!(
                        events,
                        "{}",
                        json!({"time_s":time_s,"event":"CONTACT","head":head,"pocket":self.parts[index].pocket,"position":tips[head]})
                    )?;
                }
                self.contacts[head] = contact;
            }
            seals[head] = contact.is_some();
        }
        Ok(seals)
    }

    fn update_parts(
        &mut self,
        tips: [[f64; 3]; 2],
        angles: [f64; 2],
        forces: [f64; 2],
        valves: [bool; 2],
        time_s: f64,
        events: &mut impl Write,
    ) -> Result<()> {
        for head in 0..2 {
            if let Some(index) = self.attached(head) {
                let part = &mut self.parts[index];
                let offset = rotate_z(part.grasp_offset, angles[head]);
                part.center = std::array::from_fn(|axis| tips[head][axis] + offset[axis]);
                part.rotation_deg = angles[head] + part.grasp_angle_deg;
                if forces[head] < part.mass_kg * 9.80665 {
                    part.attached_head = None;
                    let release_gap = part.center[2] - part.size[2] * 0.5 - self.board_surface_z;
                    let (sin, cos) = part.rotation_deg.to_radians().sin_cos();
                    let half_x = (cos.abs() * part.size[0] + sin.abs() * part.size[1]) * 0.5;
                    let half_y = (sin.abs() * part.size[0] + cos.abs() * part.size[1]) * 0.5;
                    let supported = release_gap.abs() <= self.z_tolerance
                        && part.center[0] - half_x >= self.board_bounds[0][0]
                        && part.center[0] + half_x <= self.board_bounds[1][0]
                        && part.center[1] - half_y >= self.board_bounds[0][1]
                        && part.center[1] + half_y <= self.board_bounds[1][1];
                    // Quasi-static support resolution within the declared contact tolerance.
                    // Free flight, impact, friction and solder contact remain unmodeled.
                    if supported {
                        part.center[2] = self.board_surface_z + part.size[2] * 0.5;
                    }
                    let target = self
                        .targets
                        .iter()
                        .find(|t| t.source_pocket == part.pocket)
                        .context("part target")?;
                    let errors = [
                        part.center[0] - target.center[0],
                        part.center[1] - target.center[1],
                        part.center[2] - target.center[2],
                    ];
                    let angle_error = ((part.rotation_deg - target.rotation_deg + 180.0)
                        .rem_euclid(360.0)
                        - 180.0)
                        .abs();
                    let placed = supported
                        && errors[0].abs() <= self.xy_tolerance
                        && errors[1].abs() <= self.xy_tolerance
                        && errors[2].abs() <= self.z_tolerance
                        && angle_error <= 0.5;
                    let event = json!({"time_s":time_s,"event":if placed {"PLACE"} else {"DROP"},"head":head,"pocket":part.pocket,"target":target.reference,"position":part.center,"rotation_deg":part.rotation_deg,"errors_mm":errors,"angle_error_deg":angle_error,"force_n":forces[head],"board_supported":supported,"release_gap_mm":release_gap,"contact_model":"quasi_static_board_support"});
                    writeln!(events, "{event}")?;
                    if placed {
                        part.placed = true;
                        self.placements.push(event);
                    } else {
                        part.dropped = true;
                        self.drops += 1;
                    }
                }
            } else if valves[head] {
                if let Some(index) = self.contact_at(tips[head]) {
                    let part = &mut self.parts[index];
                    if forces[head] > part.mass_kg * 9.80665 * 1.5 {
                        part.attached_head = Some(head);
                        part.grasp_offset = rotate_z(
                            std::array::from_fn(|axis| part.center[axis] - tips[head][axis]),
                            -angles[head],
                        );
                        part.grasp_angle_deg = part.rotation_deg - angles[head];
                        writeln!(
                            events,
                            "{}",
                            json!({"time_s":time_s,"event":"ATTACH","head":head,"pocket":part.pocket,"position":tips[head],"part_position":part.center,"rotation_deg":part.rotation_deg,"grasp_offset_local_mm":part.grasp_offset,"grasp_angle_deg":part.grasp_angle_deg,"force_n":forces[head],"weight_n":part.mass_kg*9.80665})
                        )?;
                    }
                }
            }
        }
        Ok(())
    }
}

fn rotate_z(point: [f64; 3], angle_deg: f64) -> [f64; 3] {
    let (sin, cos) = angle_deg.to_radians().sin_cos();
    [
        cos * point[0] - sin * point[1],
        sin * point[0] + cos * point[1],
        point[2],
    ]
}

fn argument(name: &str) -> Option<String> {
    std::env::args()
        .collect::<Vec<_>>()
        .windows(2)
        .find(|w| w[0] == name)
        .map(|w| w[1].clone())
}
fn digest(path: &Path) -> Result<String> {
    Ok(format!("{:x}", Sha256::digest(fs::read(path)?)))
}
fn gpio(bus: &mut SystemBus, base: u64, bit: u8) -> Result<bool> {
    Ok(bus.read_u32(base + 0x14)? & (1 << bit) != 0)
}

fn main() -> Result<()> {
    let base = argument("--base")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("examples/lumenpnp"));
    let geometry_path = argument("--geometry")
        .map(PathBuf::from)
        .unwrap_or_else(|| base.join("geometry-contract.json"));
    let scenario = argument("--scenario").unwrap_or_else(|| "healthy".into());
    ensure!(
        matches!(
            scenario.as_str(),
            "healthy" | "open-x-step" | "vacuum-leak" | "repair-x"
        ),
        "unknown scenario"
    );
    let max_seconds: f64 = argument("--seconds")
        .unwrap_or_else(|| "60".into())
        .parse()?;
    ensure!(
        max_seconds.is_finite() && max_seconds > 0.0,
        "invalid duration"
    );
    let coupling_us: u64 = argument("--coupling-us")
        .unwrap_or_else(|| "1000".into())
        .parse()?;
    ensure!(
        matches!(coupling_us, 250 | 500 | 1000),
        "coupling must be 250, 500 or 1000 us"
    );
    let coupling_cycles = CPU_HZ / 1_000_000 * coupling_us;
    let output = PathBuf::from(
        argument("--out").unwrap_or_else(|| base.join(&scenario).display().to_string()),
    );
    fs::create_dir_all(&output)?;
    let geometry_bytes = fs::read(&geometry_path)?;
    let geometry_hash = format!("{:x}", Sha256::digest(&geometry_bytes));
    let contract: Value = serde_json::from_slice(&geometry_bytes)?;
    let geometry_dir = geometry_path.parent().context("geometry directory")?;
    let source_path = geometry_dir.join(
        contract["source_file"]
            .as_str()
            .context("CAD source path")?,
    );
    let source_hash = digest(&source_path)?;
    ensure!(
        contract["source_sha256"] == source_hash,
        "CAD source hash mismatch"
    );
    let evidence_path = geometry_dir.join(
        contract["verification"]["evidence_file"]
            .as_str()
            .context("CAD evidence path")?,
    );
    let evidence_bytes = fs::read(&evidence_path)?;
    let evidence_hash = format!("{:x}", Sha256::digest(&evidence_bytes));
    ensure!(
        contract["verification"]["evidence_sha256"] == evidence_hash,
        "CAD evidence hash mismatch"
    );
    let evidence: Value = serde_json::from_slice(&evidence_bytes)?;
    let geometry_verified = contract["status"] == "kinematics_verified"
        && contract["verification"]["kinematics_pass"] == true
        && contract["verification"]["design_loop_functional"] == true
        && contract["verification"]["sampled_interference_count"] == 0
        && evidence["source_sha256"] == source_hash
        && evidence["coverage"]["captured_input_identity"] == "passed"
        && evidence["calibration_pose_count"].as_u64().unwrap_or(0) >= 7
        && evidence["maximum_affine_residual_mm"]
            .as_f64()
            .unwrap_or(f64::INFINITY)
            < 1e-9;
    ensure!(
        geometry_verified,
        "required captured geometry evidence failed preflight"
    );
    let mut geometry = Geometry::from_contract(&contract)?;
    let system_path = base.join("system.yaml");
    let firmware_path = base.join("firmware.elf");
    let mut manifest = SystemManifest::from_file(&system_path)?;
    fs::write(
        output.join("executed-system.yaml"),
        serde_yaml::to_string(&manifest)?,
    )?;
    let chip_path = base.join(&manifest.chip);
    let chip = ChipDescriptor::from_file(&chip_path)?;
    manifest.chip = chip_path.display().to_string();
    let mut bus = SystemBus::from_config(&chip, &manifest)?;
    let sink = Arc::new(Mutex::new(Vec::<u8>::new()));
    ensure!(
        bus.attach_uart_tx_sink_named("usart1", sink.clone(), false),
        "UART1 sink"
    );
    let (cpu, _) = labwired_core::system::cortex_m::configure_cortex_m(&mut bus);
    let mut machine = Machine::new(cpu, bus);
    // Obtain the driver only for the duration of each operation: observed_of
    // borrows the bus; keeping that reference across advance() would prevent
    // the same Machine from moving and resetting.
    if matches!(scenario.as_str(), "open-x-step" | "repair-x") {
        machine
            .bus
            .observed_of::<StepDirMotor>()
            .find(|motor| motor.id() == "X")
            .context("X driver observer")?
            .set_step_connected(false);
    }
    machine.config.idle_fast_forward_enabled = true;
    let wide = machine.max_safe_tick_interval();
    if wide > 1 && machine.bus.per_cycle_tick_is_trivial() {
        machine.config.peripheral_tick_interval = wide;
        machine.exact_batch_boundaries = true;
    }
    let firmware_bytes = fs::read(&firmware_path)?;
    let firmware_hash = format!("{:x}", Sha256::digest(&firmware_bytes));
    machine.load_firmware(&labwired_loader::load_elf_bytes(&firmware_bytes)?)?;
    let mut cosim = CosimSession::new(&manifest.cosim_models, &base, &machine.bus)?
        .context("analog session")?;
    ensure!(
        cosim.binding_errors().is_empty(),
        "binding errors {:?}",
        cosim.binding_errors()
    );
    machine.attach_analog_trace(cosim.analog_trace_registry());
    let peripherals = machine.get_peripherals();
    let channels: Vec<_> = PINS
        .iter()
        .map(|(_, port, pin)| {
            let peripheral = peripherals
                .iter()
                .position(|(name, _, _)| name == port)
                .expect("GPIO exists");
            Some(LogicSource::Pad {
                peripheral,
                pin: *pin,
            })
        })
        .collect();
    let mut levels: Vec<bool> = machine
        .logic_watch(&channels)
        .into_iter()
        .map(|v| v.unwrap_or(false))
        .collect();
    let mut logic_cursor = 0;
    let mut analog_cursor = 0;
    let mut logic_dropped = 0;
    let mut analog_dropped = 0;
    let mut analog_ring_evictions = 0;
    let mut edge_counts = [0i64; 6];
    let mut rising_counts = [0u64; 6];
    let mut last_high = [0u64; 6];
    let mut min_high_cycles = [u64::MAX; 6];
    let mut edge_model_mismatch_ticks = 0;
    let mut model_counts = [0i64; 6];
    let mut model_angles = [0.0f64; 6];
    let mut angle_mismatch_ticks = 0;
    let mut phantom_motion = false;
    let mut maximum_y_skew_mm = 0.0f64;
    // Sequential GPIO writes can temporarily separate Y counts by one microstep.
    let maximum_rigid_y_skew_mm = 0.025;
    let mut leak_injected_at = None;
    let linear_travel_per_revolution = contract["kinematics"]["linear_mm_per_revolution"]
        .as_f64()
        .context("CAD pulley travel")?;
    let z_degrees_per_mm = contract["kinematics"]["z_drive_degrees_per_mm"]
        .as_f64()
        .context("CAD Z drive ratio")?;
    let mut axis_values = [20.0, 20.0, 25.0, 0.0, 0.0];
    let mut plants = ["A", "B"].map(|head| {
        VacuumPlant::new(VacuumParams {
            nozzle_bore_radius_m: contract["nozzles"][head]["bore_radius"]
                .as_f64()
                .expect("nozzle bore radius")
                * 1e-3,
            ..VacuumParams::default()
        })
    });
    let mut pressures = [0.0; 2];
    let mut forces = [0.0; 2];
    let mut obs = [0i32; 19];
    let mut last_phase = -1;
    let mut last_uart = 0;
    let mut empty_pressure_min = 0.0f64;
    let mut empty_samples = 0usize;
    let mut empty_attachment = false;
    let mut maximum_firmware_time_error_ms = 0.0f64;
    let mut fuel = 0u64;
    let mut primary_steps = 0u64;
    let mut idle_cycles = 0u64;
    let mut repair_at_s: Option<f64> = None;
    let mut first_fault_at_s: Option<f64> = None;
    let mut x_steps_before_repair = 0i64;
    let mut x_model_steps_before_repair = 0i64;
    let mut recovery_gpio_mismatch_ticks = 0u64;
    let mut firmware_epoch_s = 0.0;
    let mut repaired = false;
    let mut fault_observed_safe = false;
    let mut fault_x_static = false;
    let mut final_mark = None;
    let mut terminal_state = None;
    let mut terminal_state_mismatch_samples = 0u64;
    let mut terminal_outputs_safe = true;
    let mut step_edges_after_terminal = 0u64;
    let mut step_edges_while_drivers_disabled = 0u64;
    let mut telemetry = BufWriter::new(File::create(output.join("telemetry.csv"))?);
    let mut edges = BufWriter::new(File::create(output.join("logic.csv"))?);
    let mut analog = BufWriter::new(File::create(output.join("analog.csv"))?);
    let mut events = BufWriter::new(File::create(output.join("events.jsonl"))?);
    let mut uart = BufWriter::new(File::create(output.join("uart.log"))?);
    writeln!(telemetry, "time_s,fw_ms,phase,job,head,attempt,x_mm,y_mm,z_mm,a_deg,b_deg,nozzle_a_x,nozzle_a_y,nozzle_a_z,nozzle_b_x,nozzle_b_y,nozzle_b_z,pressure_a_pa,pressure_b_pa,sensor_a_pa,sensor_b_pa,pump_a_voltage_v,pump_b_voltage_v,pump_a_current_a,pump_b_current_a,pump_a_rpm,pump_b_rpm,pump_a_gpio,pump_b_gpio,valve_a_gpio,valve_b_gpio,attached_a,attached_b,placed_count,fw_placed_count,fw_fault,retries")?;
    writeln!(edges, "cycle,time_s,signal,value")?;
    for (i, level) in levels.iter().enumerate() {
        writeln!(edges, "0,0,{},{}", PINS[i].0, u8::from(*level))?;
    }
    let start = Instant::now();
    let mut progress = Instant::now();
    let mut progress_time = 0.0;
    let mut analog_header = false;
    while (machine.total_cycles as f64 / CPU_HZ as f64) < max_seconds {
        // Endstops close from actual ideal-actuator position, with no firmware-state input.
        ensure!(
            machine
                .bus
                .drive_input_bit(0x40020800, 6, axis_values[0] <= 0.0),
            "X endstop route"
        );
        ensure!(
            machine
                .bus
                .drive_input_bit(0x40020c00, 15, axis_values[1] <= 0.0),
            "Y endstop route"
        );
        ensure!(
            machine
                .bus
                .drive_input_bit(0x40020c00, 14, axis_values[2] >= 50.0),
            "Z endstop route"
        );
        let before = machine.total_cycles;
        let run = cosim.advance_budget(
            &mut machine,
            AdvanceRequest::run(None).with_cycle_limit(coupling_cycles),
        )?;
        ensure!(
            run.new_routing_errors.is_empty(),
            "routing errors {:?}",
            run.new_routing_errors
        );
        ensure!(
            matches!(
                run.report.stop,
                AdvanceStop::CycleLimit | AdvanceStop::FuelLimit
            ),
            "CPU stopped {:?} PC={:08x}",
            run.report.stop,
            machine.cpu.get_pc()
        );
        fuel += run.report.fuel_consumed;
        primary_steps += run.report.primary_steps;
        idle_cycles += run.report.idle_cycles;
        let time_s = machine.total_cycles as f64 / CPU_HZ as f64;
        let dt_s = (machine.total_cycles - before) as f64 / CPU_HZ as f64;
        let batch = machine.logic_read_edges(logic_cursor);
        logic_cursor = batch.cursor;
        logic_dropped = batch.dropped;
        for edge in batch.edges {
            let channel = edge.ch as usize;
            if final_mark.is_some()
                && (((18..22).contains(&channel) && edge.value)
                    || (scenario != "healthy"
                        && scenario != "repair-x"
                        && channel < 18
                        && channel % 3 == 2
                        && !edge.value))
            {
                terminal_outputs_safe = false;
            }
            if channel < 18 && channel.is_multiple_of(3) {
                let axis = channel / 3;
                if edge.value {
                    if final_mark.is_some() {
                        step_edges_after_terminal += 1;
                    }
                    if (0..AXIS_IDS.len()).all(|motor| levels[motor * 3 + 2]) {
                        step_edges_while_drivers_disabled += 1;
                    }
                    last_high[axis] = edge.cycle;
                    if !levels[channel + 2] {
                        edge_counts[axis] += if levels[channel + 1] { 1 } else { -1 };
                        rising_counts[axis] += 1;
                    }
                } else if last_high[axis] != 0 {
                    min_high_cycles[axis] = min_high_cycles[axis].min(edge.cycle - last_high[axis]);
                }
            }
            levels[channel] = edge.value;
            writeln!(
                edges,
                "{},{:.9},{},{}",
                edge.cycle,
                edge.cycle as f64 / CPU_HZ as f64,
                PINS[channel].0,
                u8::from(edge.value)
            )?;
        }
        for model in machine.bus.observed_of::<StepDirMotor>() {
            let index = AXIS_IDS
                .iter()
                .position(|id| *id == model.id())
                .context("stepper ID")?;
            model_counts[index] = model.steps();
            model_angles[index] = model.angle_degrees_f64();
        }
        if model_counts != edge_counts {
            edge_model_mismatch_ticks += 1;
        }
        if repaired
            && model_counts[0] - x_model_steps_before_repair
                != edge_counts[0] - x_steps_before_repair
        {
            recovery_gpio_mismatch_ticks += 1;
        }
        if (0..6).any(|axis| (model_angles[axis] - edge_counts[axis] as f64 * 0.225).abs() > 1e-8) {
            angle_mismatch_ticks += 1;
        }
        phantom_motion |= (0..6).any(|axis| rising_counts[axis] == 0 && model_counts[axis] != 0);
        maximum_y_skew_mm = maximum_y_skew_mm
            .max((model_angles[1] - model_angles[2]).abs() / 360.0 * linear_travel_per_revolution);
        axis_values = [
            20.0 + model_angles[0] / 360.0 * linear_travel_per_revolution,
            20.0 + (model_angles[1] + model_angles[2]) / 720.0 * linear_travel_per_revolution,
            25.0 + model_angles[3] / z_degrees_per_mm,
            model_angles[4],
            model_angles[5],
        ];
        let tips = geometry.tips(axis_values);
        for (index, word) in obs.iter_mut().enumerate() {
            *word = machine.bus.read_u32(0x20010000 + index as u64 * 4)? as i32;
        }
        let mut seals = geometry.seals(tips, time_s, &mut events)?;
        if scenario == "vacuum-leak"
            && leak_injected_at.is_none()
            && obs[2] == BOARD_TRAVEL
            && geometry.attached(0).is_some()
        {
            leak_injected_at = Some(time_s);
            writeln!(
                events,
                "{}",
                json!({"time_s": time_s, "event": "FAULT_INJECTED", "fault": "nozzle_a_seal_leak"})
            )?;
        }
        if leak_injected_at.is_some() {
            seals[0] = false;
        }
        let valves = [
            gpio(&mut machine.bus, 0x40020400, 9)?,
            gpio(&mut machine.bus, 0x40021000, 6)?,
        ];
        let pumps = [
            gpio(&mut machine.bus, 0x40020400, 8)?,
            gpio(&mut machine.bus, 0x40021000, 5)?,
        ];
        let motors = machine.bus.motor_snapshots();
        let motor_a = motors.iter().find(|m| m.id == "pump_a").context("pump A")?;
        let motor_b = motors.iter().find(|m| m.id == "pump_b").context("pump B")?;
        let motor_refs = [motor_a, motor_b];
        for head in 0..2 {
            let p = plants[head].step(
                dt_s,
                motor_refs[head].speed_rpm * std::f64::consts::PI / 30.0,
                valves[head],
                seals[head],
            );
            pressures[head] = p.pressure_pa;
            forces[head] = p.suction_force_n;
            machine.bus.set_input(
                Some(if head == 0 { "vac1" } else { "vac2" }),
                "pressure_pa",
                p.pressure_pa,
            )?;
            machine
                .bus
                .set_motor_named_input(
                    if head == 0 { "pump_a" } else { "pump_b" },
                    "load-torque-nm",
                    p.pump_load_torque_nm,
                )
                .map_err(anyhow::Error::msg)?;
        }
        geometry.update_parts(
            tips,
            [axis_values[3], axis_values[4]],
            forces,
            valves,
            time_s,
            &mut events,
        )?;
        if obs[0] == 0x504e5035 && time_s - firmware_epoch_s > 0.1 {
            maximum_firmware_time_error_ms = maximum_firmware_time_error_ms
                .max(((time_s - firmware_epoch_s) * 1000.0 - obs[1] as f64).abs());
        }
        let empty = &geometry.pockets[1];
        if valves[1]
            && (tips[1][0] - empty.center[0]).abs() <= geometry.xy_tolerance
            && (tips[1][1] - empty.center[1]).abs() <= geometry.xy_tolerance
            && (tips[1][2] - empty.center[2] - empty.size[2] * 0.5).abs() <= geometry.z_tolerance
        {
            empty_pressure_min = empty_pressure_min.min(pressures[1]);
            empty_samples += 1;
            empty_attachment |= geometry.attached(1).is_some();
        }
        let attached: [i32; 2] = std::array::from_fn(|head| {
            geometry
                .attached(head)
                .map_or(-1, |i| geometry.parts[i].pocket as i32)
        });
        writeln!(telemetry, "{time_s:.9},{},{},{},{},{},{:.6},{:.6},{:.6},{:.4},{:.4},{:.6},{:.6},{:.6},{:.6},{:.6},{:.6},{:.6},{:.6},{},{},{:.6},{:.6},{:.9},{:.9},{:.6},{:.6},{},{},{},{},{},{},{},{},{},{}",
            obs[1],obs[2],obs[3],obs[4],obs[5],axis_values[0],axis_values[1],axis_values[2],axis_values[3],axis_values[4],
            tips[0][0],tips[0][1],tips[0][2],tips[1][0],tips[1][1],tips[1][2],pressures[0],pressures[1],obs[11],obs[12],
            motor_a.bus_voltage_v,motor_b.bus_voltage_v,motor_a.current_a.unwrap_or(f64::NAN),motor_b.current_a.unwrap_or(f64::NAN),motor_a.speed_rpm,motor_b.speed_rpm,
            u8::from(pumps[0]),u8::from(pumps[1]),u8::from(valves[0]),u8::from(valves[1]),attached[0],attached[1],geometry.placements.len(),obs[14],obs[15],obs[16])?;
        let samples = machine.analog_trace_snapshot(analog_cursor);
        analog_dropped += samples
            .next_cursor
            .saturating_sub(analog_cursor)
            .saturating_sub(samples.samples.len() as u64);
        analog_cursor = samples.next_cursor;
        analog_ring_evictions = samples.dropped;
        if !analog_header {
            write!(analog, "time_ns")?;
            for channel in &samples.channels {
                write!(analog, ",{}", channel.name)?;
            }
            writeln!(analog)?;
            analog_header = true;
        }
        for sample in samples.samples {
            write!(analog, "{}", sample.time_ns)?;
            for value in sample.values {
                write!(analog, ",{value}")?;
            }
            writeln!(analog)?;
        }
        {
            let bytes = sink.lock().unwrap();
            if bytes.len() > last_uart {
                uart.write_all(&bytes[last_uart..])?;
                last_uart = bytes.len();
            }
        }
        if obs[0] == 0x504e5035
            && (obs[2] != last_phase
                || time_s - progress_time >= 2.0
                || progress.elapsed().as_secs() >= 10)
        {
            eprintln!("{scenario} t={time_s:.3}s fw={} phase={} job={} attempt={} xyz={:.2},{:.2},{:.2} p={:.0},{:.0} placed={} fault={} PC={:08x}",obs[1],obs[2],obs[3],obs[5],axis_values[0],axis_values[1],axis_values[2],pressures[0],pressures[1],geometry.placements.len(),obs[15],machine.cpu.get_pc());
            last_phase = obs[2];
            progress = Instant::now();
            progress_time = time_s;
        }
        if let Some((phase, fault)) = terminal_state {
            if obs[0] != 0x504e5035 || obs[2] != phase || obs[15] != fault {
                terminal_state_mismatch_samples += 1;
            }
        } else if obs[0] == 0x504e5035 && (obs[2] == 11 || obs[2] == 12) {
            final_mark = Some(time_s);
            terminal_state = Some((obs[2], obs[15]));
            terminal_outputs_safe = levels[18..22].iter().all(|level| !level)
                && (matches!(scenario.as_str(), "healthy" | "repair-x")
                    || (0..AXIS_IDS.len()).all(|motor| levels[motor * 3 + 2]));
        }
        if final_mark.is_some_and(|terminal| time_s - terminal >= TERMINAL_OBSERVATION_S) {
            if scenario == "repair-x" && !repaired {
                ensure!(
                    obs[2] == 12 && obs[15] == 40,
                    "repair-x requires observed X-homing fault 40"
                );
                ensure!(
                    levels[18..22].iter().all(|v| !v) && (0..6).all(|i| levels[i * 3 + 2]),
                    "repair-x refuses repair while outputs remain enabled"
                );
                ensure!(
                    model_counts[0] == 0 && rising_counts[0] > 0 && geometry.placements.is_empty(),
                    "repair-x requires real missing X motion before repair"
                );
                fault_observed_safe = true;
                fault_x_static = true;
                first_fault_at_s = final_mark;
                // Repair the physical STEP path WITHOUT replacing the motor,
                // analog solver, machine, or geometry state. An operator then
                // resets the same MCU; the fault was latched by firmware.
                x_steps_before_repair = edge_counts[0];
                x_model_steps_before_repair = model_counts[0];
                machine
                    .bus
                    .observed_of::<StepDirMotor>()
                    .find(|motor| motor.id() == "X")
                    .context("X driver observer on repair")?
                    .set_step_connected(true);
                machine.reset()?;
                repaired = true;
                repair_at_s = Some(time_s);
                firmware_epoch_s = time_s;
                final_mark = None;
                terminal_state = None;
                obs.fill(0);
                writeln!(
                    events,
                    "{}",
                    json!({"time_s":time_s,
                    "event":"COPPER_REPAIRED_MCU_RESET",
                    "fault":40,"x_driver_steps_before":model_counts[0],
                    "mcucycles":machine.total_cycles,
                    "same_machine":true})
                )?;
                continue;
            }
            break;
        }
    }
    let serial = String::from_utf8_lossy(&sink.lock().unwrap()).into_owned();
    let motor_snapshots = machine.bus.motor_snapshots();
    let step_dir_reports: Vec<_> = motor_snapshots
        .iter()
        .filter_map(|m| {
            m.step_dir
                .as_ref()
                .map(|report| json!({"id":m.id,"report":report}))
        })
        .collect();
    let all_driver_timing_valid = step_dir_reports.len() == AXIS_IDS.len()
        && step_dir_reports.iter().all(|driver| {
            let timing = &driver["report"]["timing"];
            timing["status"] == "within_limits"
                && timing["checked_rising_edges"].as_u64().unwrap_or(0) > 0
                && timing["violations"]
                    .as_object()
                    .is_some_and(|v| v.is_empty())
        });
    let vacuum_loss_response_s = leak_injected_at
        .zip(final_mark)
        .map(|(injected, detected)| detected - injected);
    let duration_s = machine.total_cycles as f64 / CPU_HZ as f64;
    let terminal_observation_s = final_mark.map(|terminal| duration_s - terminal);
    let mut checks = if scenario == "healthy" || scenario == "repair-x" {
        json!({
            "kernelcad_geometry_verified":geometry_verified,
            "firmware_mission_complete":obs[2]==11 && obs[15]==0 && serial.contains("MISSION_COMPLETE"),
            "three_physical_placements":geometry.placements.len()==3 && geometry.drops==0,
            "all_objects_at_requested_target":geometry.parts.iter().all(|p|p.placed),
            "independent_gpio_counts_equal_models":edge_model_mismatch_ticks==0,
            "all_six_signed_angles_match_raw_edges":angle_mismatch_ticks==0,
            "dual_y_steps_equal":model_counts[1]==model_counts[2],
            "empty_pocket_attempted":empty_samples>100,
            "empty_pocket_never_attaches":!empty_attachment,
            "open_nozzle_does_not_cross_pick_threshold":empty_pressure_min > -25000.0,
            "exactly_one_failed_pick_and_retry":serial.matches("PICK_EMPTY").count()==1 && obs[16]==1,
            "backup_pocket_three_physically_placed":geometry.placements.iter().any(|p|p["pocket"]==3 && p["head"]==1),
            "no_logic_capture_overflow":logic_dropped==0,
            "all_driver_timing_limits_satisfied":all_driver_timing_valid,
            "both_sensor_channels_read_ambient":serial.contains("DUAL_SENSOR_AMBIENT_OK")
        })
    } else if scenario == "open-x-step" {
        json!({"firmware_detects_disconnected_x":obs[2]==12 && obs[15]==40,
            "no_physical_placement":geometry.placements.is_empty(),
            "no_false_mission_complete":!serial.contains("MISSION_COMPLETE"),
            "x_motor_static":model_counts[0]==0,
            "firmware_emitted_x_steps":rising_counts[0]>0,
            "independent_counter_detects_disconnection":edge_model_mismatch_ticks>0,
            "all_pumps_disabled_at_stop":!levels[18] && !levels[19],
            "no_logic_capture_overflow":logic_dropped==0})
    } else {
        json!({
            "seal_leak_injected_while_carrying":leak_injected_at.is_some(),
            "firmware_detects_vacuum_loss":obs[2]==12 && obs[15]==61,
            "vacuum_loss_response_within_limit":vacuum_loss_response_s.is_some_and(|elapsed| {
                (0.0..=VACUUM_LOSS_RESPONSE_LIMIT_S).contains(&elapsed)
            }),
            "no_physical_placement":geometry.placements.is_empty(),
            "no_false_mission_complete":!serial.contains("MISSION_COMPLETE"),
            "all_pumps_disabled_at_stop":!levels[18] && !levels[19],
            "independent_gpio_counts_equal_models":edge_model_mismatch_ticks==0,
            "no_logic_capture_overflow":logic_dropped==0
        })
    };
    if scenario == "repair-x" {
        // The disconnected interval intentionally diverges from GPIO counts.
        // After physical reconnection, only post-repair delta counts must match.
        checks["independent_gpio_counts_equal_models"] = json!(
            recovery_gpio_mismatch_ticks == 0
                && edge_counts[0] - x_steps_before_repair
                    == model_counts[0] - x_model_steps_before_repair
        );
        checks["all_six_signed_angles_match_raw_edges"] = json!(
            (model_counts[0] - x_model_steps_before_repair) as f64 * 0.225
                == (edge_counts[0] - x_steps_before_repair) as f64 * 0.225
        );
        checks["fault_safe_before_repair"] = json!(fault_observed_safe);
        checks["x_static_before_repair"] = json!(fault_x_static);
        checks["physical_step_trace_reconnected"] = json!(
            repaired
                && machine
                    .bus
                    .observed_of::<StepDirMotor>()
                    .any(|motor| motor.id() == "X" && motor.step_connected())
        );
        checks["same_machine_time_monotonic"] = json!(repair_at_s
            .zip(first_fault_at_s)
            .is_some_and(|(repair, fault)| repair > fault && duration_s > repair));
    }
    checks["terminal_state_reached"] = json!(final_mark.is_some());
    checks["terminal_observation_complete"] =
        json!(terminal_observation_s.is_some_and(|elapsed| elapsed >= TERMINAL_OBSERVATION_S));
    checks["terminal_state_stable"] = json!(terminal_state_mismatch_samples == 0);
    checks["terminal_outputs_remained_safe"] = json!(terminal_outputs_safe);
    checks["no_step_edges_after_terminal_state"] = json!(step_edges_after_terminal == 0);
    checks["no_step_edges_while_all_drivers_disabled"] =
        json!(step_edges_while_drivers_disabled == 0);
    checks["all_analog_samples_captured"] = json!(analog_dropped == 0);
    checks["no_motion_without_step_edges"] = json!(!phantom_motion);
    checks["firmware_clock_matches_simulation"] = json!(maximum_firmware_time_error_ms < 12.0);
    checks["all_vacuum_outputs_disabled"] = json!(levels[18..22].iter().all(|level| !level));
    checks["driver_timing_configured_without_violations"] = json!(
        step_dir_reports.len() == AXIS_IDS.len()
            && step_dir_reports.iter().all(|driver| {
                let timing = &driver["report"]["timing"];
                timing["simulation_clock_hz"]
                    .as_u64()
                    .is_some_and(|hz| hz > 0)
                    && matches!(
                        timing["status"].as_str(),
                        Some("within_limits" | "not_observed")
                    )
                    && timing["violations"]
                        .as_object()
                        .is_some_and(|v| v.is_empty())
            })
    );
    checks["dual_y_sampled_skew_within_limit"] =
        json!(maximum_y_skew_mm <= maximum_rigid_y_skew_mm + 1e-9);
    let passed = checks
        .as_object()
        .unwrap()
        .values()
        .all(|v| v == &Value::Bool(true));
    let report = json!({
        "schema":"labwired.lumenpnp.proof.v3","scenario":scenario,"passed":passed,"checks":checks,
        "source_commit":option_env!("LABWIRED_SOURCE_COMMIT"),
        "pcb_source_commit":"64cf68d5b7980f095ef11913065e7f45a1e3e447",
        "firmware_sha256":firmware_hash,"geometry_sha256":geometry_hash,
        "runner_executable_sha256":digest(&std::env::current_exe()?)?,
        "compiled_runner_source_sha256":format!("{:x}", Sha256::digest(include_bytes!("lumenpnp_capture.rs"))),
        "compiled_vacuum_source_sha256":format!("{:x}", Sha256::digest(include_bytes!("lumenpnp/vacuum.rs"))),
        "chip_descriptor_sha256":digest(&chip_path)?,
        "geometry_status":contract["status"],"cad_source_sha256":source_hash,"cad_evidence_sha256":evidence_hash,"kinematics":contract["kinematics"],"system_sha256":digest(&output.join("executed-system.yaml"))?,
        "cpu_hz":CPU_HZ,"maximum_firmware_time_error_ms":maximum_firmware_time_error_ms,
        "axis_calibration":{"xy_steps_per_mm":40,"z_steps_per_mm":40,"rotary_steps_per_revolution":1600,"driver_microsteps":8,"belt_travel_mm_per_revolution":40,"initial_xyzab":[20,20,25,0,0]},
        "coupling_step_s":coupling_us as f64*1e-6,"analog_step_s":0.0001,
        "maximum_sampled_y_skew_mm":maximum_y_skew_mm,"rigid_y_skew_limit_mm":maximum_rigid_y_skew_mm,
        "step_dir_models":step_dir_reports,"leak_injected_at_s":leak_injected_at,
        "fault_before_repair_at_s":first_fault_at_s,
        "repaired_in_same_machine_at_s":repair_at_s,
        "recovery_gpio_mismatch_ticks":recovery_gpio_mismatch_ticks,
        "repair_x_driver_original_step_count":x_model_steps_before_repair,
        "terminal_state_at_s":final_mark,"step_edges_after_terminal_state":step_edges_after_terminal,
        "terminal_observation_s":terminal_observation_s,"terminal_observation_required_s":TERMINAL_OBSERVATION_S,
        "terminal_state_mismatch_samples":terminal_state_mismatch_samples,
        "step_edges_while_all_drivers_disabled":step_edges_while_drivers_disabled,
        "vacuum_loss_response_s":vacuum_loss_response_s,"vacuum_loss_response_limit_s":VACUUM_LOSS_RESPONSE_LIMIT_S,
        "duration_s":duration_s,"wall_seconds":start.elapsed().as_secs_f64(),
        "fuel":fuel,"primary_steps":primary_steps,"idle_cycles":idle_cycles,
        "final_observation":obs,"final_axes":axis_values,"driver_counts":model_counts,
        "independent_gpio_counts":edge_counts,"rising_edges":rising_counts,
        "minimum_high_time_ns":min_high_cycles.map(|c| if c==u64::MAX {0.0} else {c as f64/CPU_HZ as f64*1e9}),
        "counter_mismatch_ticks":edge_model_mismatch_ticks,"angle_mismatch_ticks":angle_mismatch_ticks,"phantom_motion":phantom_motion,"driver_angles_deg":model_angles,"logic_dropped":logic_dropped,"analog_uncaptured_samples":analog_dropped,"analog_ring_evictions_after_capture":analog_ring_evictions,
        "placements":geometry.placements,"parts":geometry.parts,
        "empty_pocket_samples":empty_samples,"empty_pocket_min_pressure_pa":empty_pressure_min,
        "models":{"motion":"Ideal commanded STEP/DIR kinematics with per-driver timing diagnostics and sampled rigid dual-Y constraint", "pump":"Analog R/L/back-EMF armature and native rotor inertia/friction", "switch":"Ideal voltage-controlled low-side switch, assumed 50mOhm on resistance", "pneumatics":"Fixed-volume compressible chamber, flow proportional to pump speed, geometry-dependent seal/leak, pressure torque feedback", "contact":"Tolerance-based nozzle seal, preserved rigid grasp transform, gravity force threshold and quasi-static board support", "firmware":"Custom Rev05 diagnostic application"},
        "unverified":["physical parameter calibration", "load-aware stepper motion", "continuous payload collision", "free-flight and impact dynamics", "lead-to-pad and solder contact"]
    });
    telemetry.flush()?;
    edges.flush()?;
    analog.flush()?;
    events.flush()?;
    uart.flush()?;
    fs::write(
        output.join("verdict.json"),
        serde_json::to_vec_pretty(&report)?,
    )?;
    if let Some(series) = machine.analog_series() {
        fs::write(
            output.join("analog-series.json"),
            serde_json::to_vec(&series)?,
        )?;
    }
    println!(
        "{}",
        json!({"scenario":scenario,"passed":passed,"duration_s":report["duration_s"],"output":output})
    );
    ensure!(
        passed,
        "scenario verification failed or incomplete, see verdict.json"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> Geometry {
        Geometry::from_contract(
            &serde_json::from_slice(include_bytes!("lumenpnp/geometry-contract.json")).unwrap(),
        )
        .unwrap()
    }

    fn attach(geometry: &mut Geometry, tip: [f64; 3]) {
        geometry
            .update_parts(
                [tip, [0.0; 3]],
                [0.0; 2],
                [1.0, 0.0],
                [true, false],
                0.0,
                &mut Vec::new(),
            )
            .unwrap();
        assert_eq!(geometry.parts[0].attached_head, Some(0));
    }

    #[test]
    fn grasp_preserves_offset_and_initial_orientation_through_rotation() {
        let mut geometry = fixture();
        geometry.parts[0].rotation_deg = 10.0;
        attach(&mut geometry, [49.8, 60.0, 161.4]);
        geometry
            .update_parts(
                [[230.0, 225.0, 185.0], [0.0; 3]],
                [90.0, 0.0],
                [1.0, 0.0],
                [true, false],
                1.0,
                &mut Vec::new(),
            )
            .unwrap();
        let part = &geometry.parts[0];
        assert!((part.center[0] - 230.0).abs() < 1e-9);
        assert!((part.center[1] - 225.2).abs() < 1e-9);
        assert!((part.center[2] - 184.3).abs() < 1e-9);
        assert!((part.rotation_deg - 100.0).abs() < 1e-9);
    }

    #[test]
    fn release_resolves_board_support_and_records_original_gap() {
        let mut geometry = fixture();
        attach(&mut geometry, [50.0, 60.0, 161.3]);
        geometry
            .update_parts(
                [[230.0, 225.0, 161.4], [0.0; 3]],
                [0.0; 2],
                [0.0; 2],
                [false; 2],
                1.0,
                &mut Vec::new(),
            )
            .unwrap();
        assert_eq!(geometry.placements.len(), 1);
        assert!((geometry.parts[0].center[2] - 160.7).abs() < 1e-9);
        assert!((geometry.placements[0]["release_gap_mm"].as_f64().unwrap() - 0.1).abs() < 1e-9);
        assert_eq!(geometry.placements[0]["board_supported"], true);
    }

    #[test]
    fn package_overhanging_board_cannot_count_as_placed() {
        let mut geometry = fixture();
        attach(&mut geometry, [50.0, 60.0, 161.4]);
        geometry.targets[0].center[0] = 200.2;
        geometry
            .update_parts(
                [[200.2, 225.0, 161.4], [0.0; 3]],
                [0.0; 2],
                [0.0; 2],
                [false; 2],
                1.0,
                &mut Vec::new(),
            )
            .unwrap();
        assert!(geometry.placements.is_empty());
        assert_eq!(geometry.drops, 1);
    }

    #[test]
    fn release_above_board_cannot_snap_to_target() {
        let mut geometry = fixture();
        attach(&mut geometry, [50.0, 60.0, 161.4]);
        geometry
            .update_parts(
                [[230.0, 225.0, 170.0], [0.0; 3]],
                [0.0; 2],
                [0.0; 2],
                [false; 2],
                1.0,
                &mut Vec::new(),
            )
            .unwrap();
        assert!(geometry.placements.is_empty());
        assert_eq!(geometry.drops, 1);
        assert!((geometry.parts[0].center[2] - 169.3).abs() < 1e-9);
    }

    #[test]
    fn rotation_acceptance_wraps_at_full_turns() {
        let mut geometry = fixture();
        attach(&mut geometry, [50.0, 60.0, 161.4]);
        geometry
            .update_parts(
                [[230.0, 225.0, 161.4], [0.0; 3]],
                [360.0, 0.0],
                [0.0; 2],
                [false; 2],
                1.0,
                &mut Vec::new(),
            )
            .unwrap();
        assert_eq!(geometry.placements.len(), 1);
        assert_eq!(geometry.placements[0]["angle_error_deg"], 0.0);
    }
}
