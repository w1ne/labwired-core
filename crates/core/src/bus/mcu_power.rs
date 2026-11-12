//! Bind descriptor-owned MCU rail roles to explicit peripheral capabilities.
use super::SystemBus;
use labwired_config::{ChipDescriptor, GpioInputThresholds};
use std::collections::BTreeMap;

impl SystemBus {
    pub fn attach_internal_regulators(&mut self, chip: &ChipDescriptor) -> anyhow::Result<()> {
        labwired_config::validate_internal_regulators(
            &chip.internal_regulators,
            chip.power_domains.as_ref(),
        )
        .map_err(anyhow::Error::msg)?;
        self.regulators = crate::regulator::RegulatorRegistry::new(&chip.internal_regulators);
        for config in &chip.internal_regulators {
            let index = self
                .find_peripheral_index_by_name(&config.control_peripheral)
                .ok_or_else(|| {
                    anyhow::anyhow!(
                        "regulator control peripheral '{}' is absent",
                        config.control_peripheral
                    )
                })?;
            let control = self
                .regulators
                .get(&config.id)
                .expect("declared regulator")
                .control
                .clone();
            self.peripherals[index]
                .dev
                .attach_regulator_control(control)
                .map_err(|reason| {
                    anyhow::anyhow!("internal regulator '{}': {reason}", config.id)
                })?;
        }
        Ok(())
    }
    pub fn attach_mcu_power_domains(&mut self, chip: &ChipDescriptor) -> anyhow::Result<()> {
        if chip.power_domains.is_some() && chip.supply_monitor.is_some() {
            anyhow::bail!("power_domains and legacy supply_monitor are mutually exclusive");
        }
        if let Some(GpioInputThresholds::Absolute {
            nominal_io_voltage_v,
            ..
        }) = chip.gpio_input_thresholds
        {
            if chip.io_voltage_v != Some(nominal_io_voltage_v) {
                anyhow::bail!(
                    "absolute GPIO thresholds require the same explicit nominal io_voltage_v mode"
                );
            }
        }
        let Some(config) = &chip.power_domains else {
            if chip
                .reset_input
                .as_ref()
                .is_some_and(|reset| reset.io_domain.is_some())
            {
                anyhow::bail!("reset_input.io_domain requires power_domains");
            }
            return Ok(());
        };
        config.validate().map_err(anyhow::Error::msg)?;
        let nominal = config.rails[&config.io_domain].nominal_v;
        if chip.io_voltage_v != Some(nominal) {
            anyhow::bail!(
                "power_domains.io_domain nominal_v must match the declared io_voltage_v mode"
            );
        }
        if let Some(reset) = &chip.reset_input {
            if let Some(domain) = &reset.io_domain {
                let rail = config.rails.get(domain).ok_or_else(|| {
                    anyhow::anyhow!("reset_input.io_domain '{domain}' is undeclared")
                })?;
                if reset
                    .nominal_io_voltage_v
                    .is_some_and(|expected| expected != rail.nominal_v)
                {
                    anyhow::bail!(
                        "reset input nominal calibration differs from its declared domain mode"
                    );
                }
            }
        }
        self.mcu_power = crate::power_domains::McuPowerDomains::new(Some(config.clone()))
            .map_err(anyhow::Error::msg)?;
        for (id, roles) in &config.peripherals {
            let index = self
                .find_peripheral_index_by_name(id)
                .ok_or_else(|| anyhow::anyhow!("power_domains peripheral '{id}' is absent"))?;
            let handles: BTreeMap<_, _> = roles
                .iter()
                .map(|(role, domain)| {
                    (
                        role.clone(),
                        self.mcu_power
                            .handle(domain)
                            .expect("validated power domain"),
                    )
                })
                .collect();
            self.peripherals[index]
                .dev
                .attach_power_domains(handles)
                .map_err(|reason| anyhow::anyhow!("power_domains peripheral '{id}': {reason}"))?;
        }
        if let Some(id) = config
            .brown_out
            .as_ref()
            .and_then(|bod| bod.control_peripheral.as_deref())
        {
            let index = self
                .find_peripheral_index_by_name(id)
                .ok_or_else(|| anyhow::anyhow!("brown-out control peripheral '{id}' is absent"))?;
            self.peripherals[index]
                .dev
                .attach_power_supervisor(
                    self.mcu_power
                        .supervisor_control()
                        .expect("validated brown-out controller"),
                )
                .map_err(|reason| {
                    anyhow::anyhow!("brown-out control peripheral '{id}': {reason}")
                })?;
        }
        Ok(())
    }
    pub fn apply_mcu_power_voltages(
        &mut self,
        updates: &[(String, f64)],
        time_ns: u64,
    ) -> Result<(), String> {
        self.mcu_power.apply_voltages(updates, time_ns)?;
        self.sync_mcu_power_domain_receivers();
        Ok(())
    }

    /// Prepare both receiver families before either changes. Device guards
    /// stay alive through MCU publication. Notifications run after all locks
    /// drop and therefore observe the same complete boundary.
    pub(crate) fn apply_power_rail_observations(
        &mut self,
        device_updates: &[(String, String, f64)],
        mcu_sample: Option<(&[(String, f64)], u64)>,
    ) -> Result<(), crate::power::RailBatchError> {
        use crate::power::RailBatchError;
        let mut devices = self
            .supply_domains
            .prepare_updates(device_updates)
            .map_err(|error| RailBatchError::Device(error.to_string()))?;
        let notify_devices = !devices.is_empty();
        if let Some((updates, time_ns)) = mcu_sample {
            self.mcu_power
                .apply_voltages_with_publication(updates, time_ns, || devices.publish())
                .map_err(RailBatchError::Mcu)?;
        } else {
            devices.publish();
        }
        drop(devices);
        if notify_devices {
            self.sync_device_supply_states();
        }
        if mcu_sample.is_some() {
            self.sync_mcu_power_domain_receivers();
        }
        Ok(())
    }

    fn sync_mcu_power_domain_receivers(&mut self) {
        if self.mcu_power.is_measured() {
            for peripheral in &mut self.peripherals {
                peripheral.dev.sync_power_domains();
            }
        }
    }
    pub(crate) fn gpio_threshold_mode_qualified(&self) -> bool {
        let io = self.mcu_power.io_handle();
        if io.as_ref().is_some_and(|rail| !rail.is_available()) {
            return false;
        }
        if let Some(GpioInputThresholds::Absolute {
            minimum_io_voltage_v: Some(minimum),
            ..
        }) = self.gpio_input_thresholds
        {
            let actual = io
                .and_then(|rail| rail.effective_voltage_v())
                .or(self.io_voltage_v);
            return actual.is_some_and(|volts| volts >= minimum);
        }
        true
    }

    /// Side-effect-free readiness of an actual linear reset-vector mapping.
    /// A gated flash span returns false; an unmapped/MMIO table is refused.
    /// Flash is not a universal CPU execution rail: RAM/ROM vectors are checked
    /// independently, using the same ROM-before-flash-alias precedence.
    pub(crate) fn reset_vector_mapping_ready(&self, address: u64) -> Result<bool, String> {
        let covers = |memory: &crate::memory::LinearMemory, at: u64| {
            at.checked_sub(memory.base_addr)
                .and_then(|offset| offset.checked_add(8))
                .is_some_and(|end| end <= memory.data.len() as u64)
        };
        let readable = |memory: &crate::memory::LinearMemory, at: u64| {
            memory.read_u32(at).is_some() && memory.read_u32(at + 4).is_some()
        };
        if covers(&self.ram, address) {
            return Ok(readable(&self.ram, address));
        }
        for memory in &self.extra_mem {
            if covers(memory, address) {
                return Ok(readable(memory, address));
            }
        }
        if covers(&self.flash, address) {
            if !self.config.optimized_bus_access && self.find_peripheral_index(address).is_some() {
                return Err(
                    "power-domain reset cannot qualify a peripheral-overridden vector mapping"
                        .into(),
                );
            }
            return Ok(readable(&self.flash, address));
        }
        if self.flash_boot_alias
            && self.flash.base_addr != 0
            && address
                .checked_add(8)
                .is_some_and(|end| end <= self.flash.data.len() as u64)
        {
            return Ok(readable(&self.flash, self.flash.base_addr + address));
        }
        Err(format!(
            "reset vector at {address:#x} is not an explicitly mapped linear memory span"
        ))
    }
}
