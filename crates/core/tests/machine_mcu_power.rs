//! Generic reset lifecycle and peripheral capability tests. These fixtures do
//! not claim an exact device's reset timing or hardware equivalence.
mod mcu_power_support;
use labwired_config::{ChipDescriptor, PowerSystemResetScope, SystemManifest};
use labwired_core::{
    bus::SystemBus,
    cpu::CortexM,
    memory::ProgramImage,
    power_domains::{PowerDomainHandle, PowerResetCause, PowerSupervisorControl},
    system::cortex_m::configure_cortex_m,
    AdvanceRequest, Arch, Bus, Cpu, Machine, Peripheral, SimResult,
};
use std::{
    collections::BTreeMap,
    path::Path,
    sync::{Arc, Mutex},
};

#[derive(Debug, Default)]
struct Observation {
    node_resets: usize,
    system_resets: usize,
    causes: Vec<PowerResetCause>,
    last_cause: Option<PowerResetCause>,
    pairs: Vec<(Option<f64>, Option<f64>)>,
}
#[derive(Debug)]
struct Witness {
    observations: Arc<Mutex<Observation>>,
    roles: BTreeMap<String, PowerDomainHandle>,
    control: Option<PowerSupervisorControl>,
    byte: u8,
}
impl Peripheral for Witness {
    fn read(&self, _: u64) -> SimResult<u8> {
        Ok(self.byte)
    }
    fn write(&mut self, _: u64, value: u8) -> SimResult<()> {
        self.byte = value;
        Ok(())
    }
    fn attach_power_domains(
        &mut self,
        roles: BTreeMap<String, PowerDomainHandle>,
    ) -> Result<(), String> {
        if roles.keys().map(String::as_str).collect::<Vec<_>>() != ["first", "second"] {
            return Err("test witness requires two exact roles".into());
        }
        self.roles = roles;
        Ok(())
    }
    fn attach_power_supervisor(&mut self, control: PowerSupervisorControl) -> Result<(), String> {
        self.control = Some(control);
        Ok(())
    }
    fn sync_power_domains(&mut self) {
        self.observations.lock().unwrap().pairs.push((
            self.roles["first"].observation().voltage_v,
            self.roles["second"].observation().voltage_v,
        ));
    }
    fn on_node_reset(&mut self) {
        let mut observed = self.observations.lock().unwrap();
        observed.node_resets += 1;
        observed.last_cause = None;
        self.byte = 0;
    }
    fn on_system_reset(&mut self) {
        self.observations.lock().unwrap().system_resets += 1;
    }
    fn on_power_reset(&mut self, cause: PowerResetCause) {
        let mut observed = self.observations.lock().unwrap();
        observed.causes.push(cause);
        observed.last_cause = Some(cause);
    }
}
fn descriptor() -> ChipDescriptor {
    let mut chip = ChipDescriptor::from_file(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../configs/chips/stm32f401.yaml"),
    )
    .unwrap();
    chip.supply_monitor = None;
    chip.power_domains = None;
    chip
}
fn machine(scope: PowerSystemResetScope) -> (Machine<CortexM>, Arc<Mutex<Observation>>) {
    let mut chip = descriptor();
    let system: SystemManifest =
        serde_yaml::from_str("name: power-fixture\nchip: stm32f401\n").unwrap();
    let mut bus = SystemBus::from_config(&chip, &system).unwrap();
    let observations = Arc::new(Mutex::new(Observation::default()));
    bus.add_peripheral(
        "power_witness",
        0x6000_0000,
        4,
        None,
        Box::new(Witness {
            observations: observations.clone(),
            roles: BTreeMap::new(),
            control: None,
            byte: 0,
        }),
    );
    let mut config = mcu_power_support::config();
    config.system_reset_scope = scope;
    config.peripherals.insert(
        "power_witness".into(),
        BTreeMap::from([
            ("first".into(), "io".into()),
            ("second".into(), "flash".into()),
        ]),
    );
    config.brown_out.as_mut().unwrap().control_peripheral = Some("power_witness".into());
    chip.power_domains = Some(config);
    bus.attach_mcu_power_domains(&chip).unwrap();
    let (cpu, _) = configure_cortex_m(&mut bus);
    let mut machine = Machine::new(cpu, bus);
    let mut bytes = Vec::new();
    bytes.extend(0x2000_1000u32.to_le_bytes());
    bytes.extend(0x0800_0009u32.to_le_bytes());
    bytes.extend(0xe7feu16.to_le_bytes());
    let mut image = ProgramImage::new(0x0800_0009, Arch::Arm);
    image.add_segment(0x0800_0000, bytes);
    machine.load_firmware(&image).unwrap();
    machine
        .bus
        .mcu_power
        .attach_sampled_routes(
            &[
                "core".into(),
                "io".into(),
                "detector".into(),
                "flash".into(),
            ],
            1000,
        )
        .unwrap();
    (machine, observations)
}
fn sample(machine: &mut Machine<CortexM>, time: u64, core: f64) {
    machine
        .bus
        .apply_mcu_power_voltages(
            &[
                ("core".into(), core),
                ("io".into(), 3.3),
                ("detector".into(), 3.3),
                ("flash".into(), 3.3),
            ],
            time,
        )
        .unwrap();
}
fn boundary(machine: &mut Machine<CortexM>) {
    machine.advance(AdvanceRequest::run(Some(0))).unwrap();
}
fn boot(machine: &mut Machine<CortexM>) {
    sample(machine, 0, 1.1);
    boundary(machine);
    sample(machine, 3000, 1.1);
    boundary(machine);
    assert!(!machine.power_domain_status().unwrap().restart_pending);
}

#[test]
fn real_core_only_aircr_resets_systick_but_preserves_detector_and_chip_state() {
    let (mut machine, observed) = machine(PowerSystemResetScope::CoreOnly);
    boot(&mut machine);
    let control = machine.bus.mcu_power.supervisor_control().unwrap();
    control.configure_brown_out(false, 0.946, 0.95546).unwrap();
    sample(&mut machine, 4000, 1.1);
    machine.bus.write_u8(0x6000_0000, 0x5a).unwrap();
    machine.bus.write_u32(0xe000_e014, 100).unwrap();
    machine.bus.write_u32(0xe000_e010, 1).unwrap();
    machine.bus.write_u32(0xe000_ed0c, 0x05fa_0004).unwrap();
    machine.step().unwrap();
    assert!(!control.brown_out_configuration().enabled);
    assert_eq!(machine.bus.read_u8(0x6000_0000).unwrap(), 0x5a);
    for address in [0xe000_e010, 0xe000_e014, 0xe000_e018] {
        assert_eq!(
            machine.bus.read_u32(address).unwrap(),
            0,
            "SysTick {address:#x}"
        );
    }
    let observed = observed.lock().unwrap();
    assert_eq!(observed.node_resets, 1);
    assert_eq!(observed.causes, [PowerResetCause::PowerOn]);
    assert_eq!(observed.system_resets, 1);
    assert_eq!(machine.power_domain_status().unwrap().reset_events, 1);
}

#[test]
fn chip_scoped_aircr_restores_detector_defaults_and_delivers_one_system_cause() {
    let (mut machine, observed) = machine(PowerSystemResetScope::Chip);
    boot(&mut machine);
    let control = machine.bus.mcu_power.supervisor_control().unwrap();
    control.configure_brown_out(false, 0.946, 0.95546).unwrap();
    sample(&mut machine, 4000, 1.1);
    machine.bus.write_u8(0x6000_0000, 0x5a).unwrap();
    machine.bus.write_u32(0xe000_ed0c, 0x05fa_0004).unwrap();
    machine.step().unwrap();
    assert!(control.brown_out_configuration().enabled);
    assert_eq!(machine.bus.read_u8(0x6000_0000).unwrap(), 0);
    let observed = observed.lock().unwrap();
    assert_eq!(observed.node_resets, 2);
    assert_eq!(
        observed.causes,
        [PowerResetCause::PowerOn, PowerResetCause::System]
    );
    assert_eq!(observed.last_cause, Some(PowerResetCause::System));
}

#[test]
fn brown_out_resets_peripherals_on_assertion_only_and_preserves_cause_on_restart() {
    let (mut machine, observed) = machine(PowerSystemResetScope::CoreOnly);
    boot(&mut machine);
    sample(&mut machine, 100_000, 0.90);
    sample(&mut machine, 103_000, 0.90);
    boundary(&mut machine);
    assert_eq!(observed.lock().unwrap().node_resets, 2);
    assert!(machine.power_domain_status().unwrap().restart_pending);
    let before = machine.cpu.get_pc();
    let report = machine.advance(AdvanceRequest::run(Some(100))).unwrap();
    assert_eq!(report.primary_steps, 0);
    assert_eq!(machine.cpu.get_pc(), before);
    sample(&mut machine, 104_000, 1.1);
    sample(&mut machine, 159_000, 1.1);
    boundary(&mut machine);
    assert!(!machine.power_domain_status().unwrap().restart_pending);
    let observed = observed.lock().unwrap();
    assert_eq!(
        observed.node_resets, 2,
        "release must not reset peripherals twice"
    );
    assert_eq!(
        observed.causes,
        [PowerResetCause::PowerOn, PowerResetCause::BrownOut]
    );
    assert_eq!(observed.last_cause, Some(PowerResetCause::BrownOut));
}

#[test]
fn role_observers_see_one_complete_batch_and_no_partial_invalid_update() {
    let (mut machine, observed) = machine(PowerSystemResetScope::CoreOnly);
    sample(&mut machine, 0, 1.1);
    observed.lock().unwrap().pairs.clear();
    machine
        .bus
        .apply_mcu_power_voltages(&[("io".into(), 2.0), ("flash".into(), 3.4)], 1000)
        .unwrap();
    assert_eq!(observed.lock().unwrap().pairs, [(Some(2.0), Some(3.4))]);
    assert!(machine
        .bus
        .apply_mcu_power_voltages(&[("io".into(), 3.3), ("flash".into(), f64::NAN)], 2000)
        .is_err());
    assert_eq!(observed.lock().unwrap().pairs, [(Some(2.0), Some(3.4))]);
    assert_eq!(
        machine
            .bus
            .mcu_power
            .handle("io")
            .unwrap()
            .observation()
            .voltage_v,
        Some(2.0)
    );
}

#[test]
fn declared_roles_and_supervision_require_explicit_peripheral_capabilities() {
    let system: SystemManifest =
        serde_yaml::from_str("name: power-fixture\nchip: stm32f401\n").unwrap();
    for kind in ["role", "control", "missing"] {
        let mut chip = descriptor();
        let mut config = mcu_power_support::config();
        if kind == "control" {
            config.brown_out.as_mut().unwrap().control_peripheral = Some("gpioa".into());
        } else {
            config.peripherals.insert(
                if kind == "missing" { "absent" } else { "gpioa" }.into(),
                BTreeMap::from([("io".into(), "io".into())]),
            );
        }
        chip.power_domains = Some(config);
        let error = match SystemBus::from_config(&chip, &system) {
            Ok(_) => panic!("accepted unsupported {kind}"),
            Err(error) => error.to_string(),
        };
        assert!(
            error.contains("power") || error.contains("brown-out"),
            "{error}"
        );
    }
}

#[test]
fn power_restart_refuses_a_missing_recorded_vector_mapping() {
    let (mut machine, _) = machine(PowerSystemResetScope::CoreOnly);
    machine.bus.flash_boot_alias = false;
    sample(&mut machine, 0, 1.1);
    sample(&mut machine, 3000, 1.1);
    let error = machine
        .advance(AdvanceRequest::run(Some(0)))
        .unwrap_err()
        .to_string();
    assert!(
        error.contains("reset vector") && error.contains("linear memory"),
        "{error}"
    );
    assert!(machine.power_domain_status().unwrap().restart_pending);
}
