// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! One-MCU synchronous mechanical coupling. No wall clock, UI frame or
//! requested instruction fuel is treated as simulated time.

use crate::{cosim::AdvanceFailure, lab_tools, WasmSimulator};
use labwired_core::bus::{ExternalMechanicalFrame, ExternalMechanicalInterval};
use labwired_core::{AdvanceRequest, AdvanceStop};
use serde::Serialize;
use wasm_bindgen::prelude::*;

const MAX_SAFE_CYCLE: u64 = (1_u64 << 53) - 1;

fn safe_cycle(value: f64) -> Result<u64, String> {
    if !value.is_finite() || value < 0.0 || value.fract() != 0.0 || value > MAX_SAFE_CYCLE as f64 {
        return Err("mechanical target_cycle must be a non-negative JS-safe integer".to_owned());
    }
    Ok(value as u64)
}

fn advance_error(error: AdvanceFailure) -> String {
    match error {
        AdvanceFailure::Machine(error) => format!("mechanical native step failed: {error}"),
        AdvanceFailure::Cosim(error) => error,
    }
}

fn interval_to_js(interval: &ExternalMechanicalInterval) -> Result<JsValue, JsValue> {
    interval
        .serialize(&serde_wasm_bindgen::Serializer::new().serialize_missing_as_null(true))
        .map_err(|error| JsValue::from_str(&error.to_string()))
}

#[wasm_bindgen]
impl WasmSimulator {
    /// Electrical torque integrated over every service substep since the last
    /// complete frame. Repeated reads do not consume the interval.
    pub fn get_external_mechanical_interval(&mut self) -> Result<JsValue, JsValue> {
        let interval = self
            .external_mechanical_interval_s()
            .map_err(|error| JsValue::from_str(&error))?;
        self.record(lab_tools::Op::ObserveExternalMechanical);
        interval_to_js(&interval)
    }

    /// Prime at cycle zero, then commit every owned shaft and the same complete
    /// sensor-input set at the current cycle. Invalid frames mutate nothing.
    pub fn apply_external_mechanical_frame(&mut self, frame: JsValue) -> Result<(), JsValue> {
        let frame: ExternalMechanicalFrame = serde_wasm_bindgen::from_value(frame)
            .map_err(|error| JsValue::from_str(&error.to_string()))?;
        self.apply_external_mechanical_frame_s(&frame)
            .map_err(|error| JsValue::from_str(&error))
    }

    /// Spend at most max_fuel while stopping no later than target_cycle.
    /// Partial calls retain one accumulated interval; the caller steps its
    /// mechanical solver only when its desired frontier has been reached.
    pub fn step_external_mechanical_to(
        &mut self,
        target_cycle: f64,
        max_fuel: u32,
    ) -> Result<JsValue, JsValue> {
        let target = safe_cycle(target_cycle).map_err(|error| JsValue::from_str(&error))?;
        let interval = self
            .step_external_mechanical_to_s(target, max_fuel)
            .map_err(|error| JsValue::from_str(&error))?;
        interval_to_js(&interval)
    }
}

impl WasmSimulator {
    fn external_mechanical_interval_s(&mut self) -> Result<ExternalMechanicalInterval, String> {
        self.machine
            .as_mut()
            .ok_or("simulator is not initialized")?
            .bus
            .get_external_mechanical_interval()
    }

    pub(crate) fn apply_external_mechanical_frame_s(
        &mut self,
        frame: &ExternalMechanicalFrame,
    ) -> Result<(), String> {
        self.machine
            .as_mut()
            .ok_or("simulator is not initialized")?
            .bus
            .apply_external_mechanical_frame(frame)?;
        self.record(lab_tools::Op::ApplyExternalMechanical(frame.clone()));
        Ok(())
    }

    /// Capability refusal happens before the first instruction, not after
    /// observing an overrun. Ordinary Machine cycle limits deliberately allow
    /// an atomic instruction/tick-cost boundary to cross their limit.
    fn validate_external_mechanical_step(&self, target: u64, max_fuel: u32) -> Result<(), String> {
        let machine = self
            .machine
            .as_ref()
            .ok_or("simulator is not initialized")?;
        if !machine.bus.has_external_mechanical_shafts()
            || !machine.bus.external_mechanical_is_primed()
        {
            return Err(
                "external mechanical shafts require a complete priming frame at cycle 0".to_owned(),
            );
        }
        if target > MAX_SAFE_CYCLE || target < machine.total_cycles || max_fuel == 0 {
            return Err("mechanical target must be at or after the current cycle and max_fuel must be positive".to_owned());
        }
        if self.cosim.is_some() {
            return Err("external mechanical frontiers cannot yet compose with another co-simulation session".to_owned());
        }
        if self.jit_browser_enabled
            || machine.config.riscv_jit_enabled
            || machine.config.cortex_m_jit_enabled
        {
            return Err("external mechanical frontiers require the bounded interpreter".to_owned());
        }
        if (machine.cpu.instruction_cycles_are_time() && machine.cpu.max_step_cycles() > 1)
            || machine
                .cpu_secondary
                .as_ref()
                .is_some_and(|cpu| cpu.instruction_cycles_are_time() && cpu.max_step_cycles() > 1)
        {
            return Err(
                "this CPU cannot guarantee an exact one-cycle mechanical frontier".to_owned(),
            );
        }
        if let Some(peripheral) = machine
            .bus
            .peripherals
            .iter()
            .find(|peripheral| !peripheral.dev.has_zero_tick_cycle_cost())
        {
            return Err(format!(
                "peripheral '{}' has no zero-tick-cycle guarantee for an exact mechanical frontier",
                peripheral.name
            ));
        }
        self.ensure_cosim_healthy().map_err(advance_error)
    }

    fn step_external_mechanical_to_s(
        &mut self,
        target: u64,
        max_fuel: u32,
    ) -> Result<ExternalMechanicalInterval, String> {
        self.validate_external_mechanical_step(target, max_fuel)?;
        self.record(lab_tools::Op::StepExternalMechanical(target, max_fuel));
        let legacy_ipi = self.esp32_ipi.is_some()
            && self
                .machine
                .as_ref()
                .is_some_and(|machine| machine.cpu_secondary.is_none());
        if legacy_ipi {
            // Preserve the same IPI/boot handshake service as the existing
            // Arduino aids path, without its optional unbounded browser JIT.
            for index in 0..max_fuel {
                let current = self.machine.as_ref().unwrap().total_cycles;
                if current >= target {
                    break;
                }
                self.service_esp32_singlecore_ipi(index % 10_000 == 0);
                let report = self
                    .advance_machine_at_mechanical_frontier(
                        AdvanceRequest::single().with_cycle_limit(target - current),
                    )
                    .map_err(advance_error)?;
                if !matches!(
                    report.stop,
                    AdvanceStop::FuelLimit | AdvanceStop::CycleLimit
                ) {
                    return Err(format!(
                        "native machine stopped before mechanical feedback: {:?}",
                        report.stop
                    ));
                }
            }
        } else {
            let current = self.machine.as_ref().unwrap().total_cycles;
            if current < target {
                let report = self
                    .advance_machine_at_mechanical_frontier(
                        AdvanceRequest::run(Some(u64::from(max_fuel)))
                            .with_cycle_limit(target - current),
                    )
                    .map_err(advance_error)?;
                if !matches!(
                    report.stop,
                    AdvanceStop::FuelLimit | AdvanceStop::CycleLimit
                ) {
                    return Err(format!(
                        "native machine stopped before mechanical feedback: {:?}",
                        report.stop
                    ));
                }
            }
        }
        let actual = self.machine.as_ref().unwrap().total_cycles;
        // This is an invariant check, not the bound mechanism: every
        // overshoot-capable CPU/peripheral was refused before execution.
        if actual > target {
            return Err(format!(
                "mechanical frontier invariant failed: cycle {actual} exceeds {target}"
            ));
        }
        self.external_mechanical_interval_s()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cycle_arguments_reject_fractional_nonfinite_and_unsafe_numbers() {
        for value in [-1.0, 0.5, f64::NAN, f64::INFINITY, (1_u64 << 53) as f64] {
            assert!(safe_cycle(value).is_err());
        }
        assert_eq!(safe_cycle(0.0).unwrap(), 0);
        assert_eq!(safe_cycle(MAX_SAFE_CYCLE as f64).unwrap(), MAX_SAFE_CYCLE);
    }
    use crate::Esp32IpiBridge;
    use labwired_config::{ChipDescriptor, SystemManifest};
    use labwired_core::bus::{Bus, ExternalShaftState, SystemBus};
    use labwired_core::console::{ConsoleCapture, HostConsole};
    use labwired_core::cpu::{CortexM, Cpu};
    use labwired_core::system::arch_policy::MachineFamily;
    use labwired_core::{Machine, Peripheral, PeripheralTickResult, SimResult};
    use std::sync::{Arc, Mutex};

    fn manifest() -> SystemManifest {
        serde_yaml::from_str(
            r#"
name: mechanical-frontier-test
chip: unused
motor_models:
  - kind: dc
    id: shaft
    shaft_owner: external
    resistance_ohm: 2.0
    inductance_h: 0.001
    torque_constant_nm_per_a: 0.02
    back_emf_constant_v_per_rad_s: 0.02
    rotor_inertia_kg_m2: 0.0
    viscous_friction_nm_per_rad_s: 0.0
    supply_voltage_v: 3.7
    load_torque_nm: 0.0
    encoder_cpr: 4
    simulation_clock_hz: 1000000
    pwm_pin: 3V3
    direction_pin: 3V3
"#,
        )
        .unwrap()
    }

    fn frame(cycle: u64) -> ExternalMechanicalFrame {
        ExternalMechanicalFrame {
            at_cycle: cycle,
            shafts: vec![ExternalShaftState {
                id: "shaft".to_owned(),
                position_rad: 0.0,
                angular_velocity_rad_s: 0.0,
            }],
            inputs: Vec::new(),
        }
    }

    fn wrap(cpu: Box<dyn Cpu>, bus: SystemBus, arch: MachineFamily) -> WasmSimulator {
        WasmSimulator {
            machine: Some(Machine::new(cpu, bus)),
            board_io: Vec::new(),
            uart_sink: Arc::new(Mutex::new(Vec::new())),
            console: ConsoleCapture::new(HostConsole::Undeclared, HostConsole::Undeclared),
            uart_rx_bufs: Vec::new(),
            arch,
            esp32_ipi: None,
            jit_browser_enabled: false,
            jit_browser_cache: None,
            cosim: None,
            tools: Default::default(),
        }
    }

    fn fixture() -> WasmSimulator {
        let chip: ChipDescriptor = serde_yaml::from_str(
            r#"
name: mechanical-frontier-test
arch: arm
core: cortex-m4
cpu_hz: 1000000
flash: { base: 0x08000000, size: "64KB" }
ram: { base: 0x20000000, size: "32KB" }
peripherals: []
"#,
        )
        .unwrap();
        let mut bus = SystemBus::from_config(&chip, &manifest()).unwrap();
        bus.flash.data[..2].copy_from_slice(&0xE7FE_u16.to_le_bytes()); // Thumb B .
        let mut cpu = CortexM::new();
        cpu.set_pc(0x08000000);
        wrap(Box::new(cpu), bus, MachineFamily::CortexM)
    }

    #[test]
    fn bounded_step_handles_partial_fuel_and_never_crosses_the_target() {
        let mut sim = fixture();
        assert!(sim.step_external_mechanical_to_s(100, 10).is_err());
        assert_eq!(sim.machine.as_ref().unwrap().total_cycles, 0);
        sim.apply_external_mechanical_frame_s(&frame(0)).unwrap();
        assert!(sim.advance_machine(AdvanceRequest::run(Some(10))).is_err());
        let first = sim.step_external_mechanical_to_s(100, 7).unwrap();
        assert_eq!(first.at_cycle, 7);
        assert_eq!(first.from_cycle, 0);
        let last = sim.step_external_mechanical_to_s(100, 10000).unwrap();
        assert_eq!(last.at_cycle, 100);
        assert_eq!(last.elapsed_s, 0.0001);
        assert!(last.shafts[0].torque_impulse_nm_s > first.shafts[0].torque_impulse_nm_s);
        assert_eq!(last.shafts[0].position_rad, 0.0);
        sim.apply_external_mechanical_frame_s(&frame(100)).unwrap();
        assert_eq!(
            sim.step_external_mechanical_to_s(101, 10000)
                .unwrap()
                .at_cycle,
            101
        );
    }

    #[test]
    fn esp32_real_constructors_seed_declared_clock_before_external_devices() {
        use labwired_core::system::builder::{
            build_esp32_system_from_manifest, build_machine, BootMode, BuildOptions, BuildRequest,
            FirmwareSource,
        };
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let chip_yaml = include_str!("../../../configs/chips/esp32.yaml");
        let chip: ChipDescriptor = serde_yaml::from_str(chip_yaml).unwrap();
        let firmware =
            include_bytes!("../../core/tests/fixtures/esp32-classic-console/console.elf");
        // Exercise the actual programmatic constructors, not from_config's
        // already-correct bus. A declared operating clock has the same
        // precedence as on every descriptor-built bus.
        for operating_clock in [None, Some(120_000_000)] {
            let mut system = manifest();
            system.chip = root
                .join("configs/chips/esp32.yaml")
                .to_string_lossy()
                .into_owned();
            system.cpu_hz = operating_clock;
            let clock_hz = operating_clock.unwrap_or(chip.cpu_hz);
            let labwired_config::MotorModelConfig::Dc(motor) = &mut system.motor_models[0] else {
                unreachable!()
            };
            motor.simulation_clock_hz = clock_hz;
            let inputs = std::rc::Rc::new(lab_tools::CtorInputs {
                system_yaml: serde_yaml::to_string(&system).unwrap(),
                chip_yaml: chip_yaml.to_owned(),
                firmware: firmware.to_vec(),
                blobs: Default::default(),
            });
            let mut sim = WasmSimulator::new_from_config_parts(inputs)
                .unwrap_or_else(|_| panic!("real ESP32 constructor rejected declared clock"));
            assert_eq!(sim.machine.as_ref().unwrap().bus.cpu_hz, clock_hz);
            assert!(sim.machine.as_ref().unwrap().cpu_secondary.is_some());
            let zero = sim.external_mechanical_interval_s().unwrap();
            assert_eq!(zero.clock_hz, clock_hz);
            assert_eq!(zero.at_cycle, 0);
            assert_eq!(zero.shafts[0].torque_average_nm, None);
            sim.apply_external_mechanical_frame_s(&frame(0)).unwrap();
            let stepped = sim.step_external_mechanical_to_s(3, 100).unwrap();
            assert_eq!(stepped.at_cycle, 3);
            assert_eq!(stepped.elapsed_s, 3.0 / clock_hz as f64);

            let (bus, _, _) = build_esp32_system_from_manifest(&system, &root.join("system.yaml"))
                .expect("native manifest constructor must attach the same external plant");
            assert_eq!(bus.cpu_hz, clock_hz);
            let blobs = Default::default();
            let mut built = build_machine(BuildRequest {
                chip: &chip,
                system: &system,
                firmware: FirmwareSource::Elf(firmware),
                boot: BootMode::FastBoot,
                blobs: &blobs,
                options: BuildOptions::default(),
            })
            .expect("native session constructor must attach the same external plant");
            assert_eq!(built.machine.cycles(), 0);
            assert!(built.machine.advance(AdvanceRequest::single()).is_err());
            assert_eq!(built.machine.cycles(), 0);
        }
    }

    #[derive(Debug)]
    struct ChargedPeripheral;
    impl Peripheral for ChargedPeripheral {
        fn read(&self, _offset: u64) -> SimResult<u8> {
            Ok(0)
        }
        fn write(&mut self, _offset: u64, _value: u8) -> SimResult<()> {
            Ok(())
        }
        fn tick(&mut self) -> PeripheralTickResult {
            PeripheralTickResult {
                cycles: 100,
                ..Default::default()
            }
        }
    }

    #[test]
    fn unknown_or_charged_tick_capability_is_refused_before_any_instruction() {
        let mut sim = fixture();
        sim.machine.as_mut().unwrap().bus.add_peripheral(
            "charged",
            0x40000000,
            0x1000,
            None,
            Box::new(ChargedPeripheral),
        );
        sim.apply_external_mechanical_frame_s(&frame(0)).unwrap();
        let pc = sim.machine.as_ref().unwrap().cpu.get_pc();
        let error = sim.step_external_mechanical_to_s(1, 1000).unwrap_err();
        assert!(
            error.contains("charged") && error.contains("zero-tick"),
            "{error}"
        );
        assert_eq!(sim.machine.as_ref().unwrap().total_cycles, 0);
        assert_eq!(sim.machine.as_ref().unwrap().cpu.get_pc(), pc);
    }

    #[test]
    fn jit_is_refused_before_moving_an_external_frontier() {
        let mut sim = fixture();
        sim.apply_external_mechanical_frame_s(&frame(0)).unwrap();
        sim.jit_browser_enabled = true;
        assert!(sim
            .step_external_mechanical_to_s(10, 100)
            .unwrap_err()
            .contains("interpreter"));
        assert_eq!(sim.machine.as_ref().unwrap().total_cycles, 0);
    }

    #[test]
    fn esp32_configured_peripherals_prove_zero_tick_cost_and_legacy_ipi_is_preserved() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let chip = ChipDescriptor::from_file(root.join("configs/chips/esp32.yaml")).unwrap();
        let mut system = manifest();
        system.chip = root
            .join("configs/chips/esp32.yaml")
            .to_string_lossy()
            .into_owned();
        let labwired_config::MotorModelConfig::Dc(motor) = &mut system.motor_models[0] else {
            unreachable!()
        };
        motor.simulation_clock_hz = chip.cpu_hz;
        let mut bus = SystemBus::from_config(&chip, &system).unwrap();
        let mut cpu = labwired_core::system::xtensa::configure_xtensa_esp32(&mut bus);
        // Canonical two-byte RET.N-free NOP stream used by existing advance tests.
        for index in 0..64_u64 {
            bus.write_u8(0x40080000 + index * 2, 0x3d).unwrap();
            bus.write_u8(0x40080001 + index * 2, 0xf0).unwrap();
        }
        cpu.set_pc(0x40080000);
        let mut sim = wrap(Box::new(cpu), bus, MachineFamily::Xtensa);
        sim.esp32_ipi = Some(Esp32IpiBridge {
            from_cpu_bit0: None,
            from_cpu_bit1: None,
            last_from_cpu0_val: 0,
            last_from_cpu1_val: 0,
            handshake_bytes: vec![0x3ffb0000],
        });
        sim.apply_external_mechanical_frame_s(&frame(0)).unwrap();
        let unsupported: Vec<_> = sim
            .machine
            .as_ref()
            .unwrap()
            .bus
            .peripherals
            .iter()
            .filter(|entry| !entry.dev.has_zero_tick_cycle_cost())
            .map(|entry| entry.name.clone())
            .collect();
        assert!(unsupported.is_empty(), "{unsupported:?}");
        assert_eq!(
            sim.step_external_mechanical_to_s(3, 100).unwrap().at_cycle,
            3
        );
        assert_eq!(
            sim.machine
                .as_ref()
                .unwrap()
                .bus
                .read_u8(0x3ffb0000)
                .unwrap(),
            1
        );
    }
    #[test]
    fn multi_cycle_instruction_timing_is_refused_before_any_instruction() {
        let mut sim = fixture();
        sim.machine.as_mut().unwrap().cpu = Box::new(labwired_core::cpu::Avr::new());
        sim.apply_external_mechanical_frame_s(&frame(0)).unwrap();
        let error = sim.step_external_mechanical_to_s(1, 100).unwrap_err();
        assert!(
            error.contains("CPU") && error.contains("frontier"),
            "{error}"
        );
        assert_eq!(sim.machine.as_ref().unwrap().total_cycles, 0);
    }
}
