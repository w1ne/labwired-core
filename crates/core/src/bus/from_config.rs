// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! `SystemBus::from_config`: build a bus + its peripherals from a chip
//! descriptor + system manifest. Split out of `bus/mod.rs`.

use super::*;
use crate::memory::LinearMemory;
use crate::peripherals::gpio::GpioRegisterLayout;
use crate::Peripheral;
use anyhow::Context;
use labwired_config::{parse_size, ChipDescriptor, SystemManifest};
use std::cell::Cell;
use std::path::{Path, PathBuf};

/// Default on-disk dumps when `image_env` is unset. Keeps copyrighted ROMs out
/// of the repo path contract (env still wins) while letting matrix/CLI find the
/// in-tree `crates/core/roms/esp32c3/*` copies used by e2e gates.
fn default_region_image_path(env: &str) -> Option<PathBuf> {
    let rel = match env {
        "LABWIRED_ESP32C3_ROM" => "roms/esp32c3/esp32c3_rom.bin",
        "LABWIRED_ESP32C3_ROM_DATA" => "roms/esp32c3/esp32c3_drom.bin",
        // In-tree minimal B0 bootrom so Arduino/Zephyr `rom_func_lookup` works
        // on plain `labwired test` without exporting the env. Bare-metal ELFs
        // that need the Cortex-M flash boot alias at 0 (PIO onboarding) can
        // set LABWIRED_RP2040_BOOTROM= (empty) to skip the image — from_config
        // then leaves the region out so flash alias wins.
        "LABWIRED_RP2040_BOOTROM" => "roms/rp2040/bootrom.bin",
        _ => return None,
    };
    // Walk: CWD, CWD/crates/core, crate-relative from this source tree layout.
    let mut candidates = Vec::new();
    if let Ok(cwd) = std::env::current_dir() {
        candidates.push(cwd.join(rel));
        candidates.push(cwd.join("crates/core").join(rel));
    }
    // `CARGO_MANIFEST_DIR` for labwired-core when tests run from the crate.
    if let Ok(manifest) = std::env::var("CARGO_MANIFEST_DIR") {
        candidates.push(Path::new(&manifest).join(rel));
        // crates/cli → ../../crates/core/roms/...
        candidates.push(Path::new(&manifest).join("../core").join(rel));
    }
    candidates.into_iter().find(|p| p.is_file())
}

impl SystemBus {
    /// Collect debugger-only register schemas from each peripheral's optional
    /// `config.debug_schema` path.
    ///
    /// This is for NATIVE peripherals — those modeled in hand-written Rust,
    /// which advertise no `describe_registers()` and therefore inspect as
    /// `registers: []`. The schema names what the model already holds; it never
    /// changes what the bus does. See [`SystemBus::debug_schemas`].
    ///
    /// A `debug_schema` on a `declarative` peripheral is redundant (it already
    /// describes itself from its own descriptor) and simply loses to
    /// `describe_registers()` at inspect time.
    ///
    /// Resolution mirrors the declarative descriptor path exactly: embedded
    /// first (wasm32 has no `std::fs`), filesystem second. A path that resolves
    /// to neither is skipped with a warning rather than failing the build —
    /// a missing debugger convenience must never stop a simulation from running.
    fn load_debug_schemas(
        chip: &ChipDescriptor,
        manifest: &SystemManifest,
    ) -> std::collections::HashMap<String, Vec<crate::inspect::RegisterSchema>> {
        let mut out = std::collections::HashMap::new();

        for p_cfg in &chip.peripherals {
            let Some(path) = p_cfg.config.get("debug_schema").and_then(|v| v.as_str()) else {
                continue;
            };

            let descriptor = if let Some(embedded) = super::embedded_descriptors::lookup(path) {
                labwired_config::PeripheralDescriptor::from_yaml(embedded).ok()
            } else {
                let resolved = Self::resolve_peripheral_path(manifest, path);
                labwired_config::PeripheralDescriptor::from_file(&resolved).ok()
            };

            match descriptor {
                Some(descriptor) => {
                    out.insert(
                        p_cfg.id.clone(),
                        crate::inspect::schema_from_descriptor(&descriptor),
                    );
                }
                None => {
                    tracing::warn!(
                        "debug_schema '{}' for peripheral '{}' could not be loaded; \
                         its registers will inspect unnamed",
                        path,
                        p_cfg.id
                    );
                }
            }
        }

        out
    }

    /// Attach a chip's debugger-only register schemas to a bus that was built
    /// programmatically instead of through [`Self::from_config`].
    ///
    /// The ESP32-classic and ESP32-S3 wiring (`configure_xtensa_esp32`,
    /// `configure_xtensa_esp32s3`) starts from [`SystemBus::new`] and registers
    /// its peripheral bank in Rust, deliberately bypassing the chip YAML's
    /// peripheral list. That also bypassed `load_debug_schemas`, so every
    /// `debug_schema:` an Xtensa chip declared was silently inert and its
    /// peripherals inspected as `registers: []` no matter what the YAML said.
    /// Runners on that path call this after wiring the bus.
    ///
    /// Schemas are keyed by the chip YAML's peripheral `id` and matched against
    /// the bus peripheral's `name`, so an id that no bus peripheral answers to
    /// is simply never consulted. Like `from_config`, this only names registers
    /// the model already holds — it never changes what the bus does.
    pub fn attach_debug_schemas(&mut self, chip: &ChipDescriptor, manifest: &SystemManifest) {
        self.debug_schemas = Self::load_debug_schemas(chip, manifest);
    }

    pub fn from_config(chip: &ChipDescriptor, manifest: &SystemManifest) -> anyhow::Result<Self> {
        Self::from_config_with_plugins(chip, manifest, &[])
    }

    /// [`Self::from_config`] with out-of-tree chip plugins. Each peripheral
    /// type is offered to `plugins` first (in order): the first `Some(Ok)`
    /// wins, `Some(Err)` aborts the build with that error, and `None` from
    /// every plugin falls through to the in-tree factories below.
    pub fn from_config_with_plugins(
        chip: &ChipDescriptor,
        manifest: &SystemManifest,
        plugins: &[&dyn crate::plugin::ChipPlugin],
    ) -> anyhow::Result<Self> {
        // Part-pack contract, enforced HERE rather than in the manifest loader:
        // `from_file` is the CLI's path, and the browser and hosted runners
        // parse with `from_yaml`. Validating at load time only would mean two
        // of our three runtimes silently accept documents the third rejects.
        manifest.validate_parts()?;
        let flash_size = parse_size(&chip.flash.size)?;
        let ram_size = parse_size(&chip.ram.size)?;

        let mut extra_mem = Vec::with_capacity(chip.memory_regions.len());
        for region in &chip.memory_regions {
            let size = parse_size(&region.size)?;
            let mut mem = LinearMemory::new(size as usize, region.base);
            // Optionally preload a raw binary image (e.g. a dumped mask ROM)
            // from a path given by an env var. Copyrighted vendor blobs are not
            // committed, so a missing image just leaves the region zero-filled.
            let mut loaded_image = false;
            if let Some(env) = &region.image_env {
                // Env pin first; else well-known in-tree dumps so Arduino-matrix
                // / plain `labwired test` can call C3 ROM helpers without
                // requiring the operator to export LABWIRED_ESP32C3_ROM*.
                // Explicit empty env → skip image (opt-out of in-tree default).
                let path_owned = match std::env::var(env) {
                    Ok(p) if p.is_empty() => None,
                    Ok(p) => Some(p),
                    Err(_) => default_region_image_path(env).map(|p| p.display().to_string()),
                };
                if let Some(path) = path_owned {
                    match std::fs::read(&path) {
                        Ok(bytes) => {
                            let n = bytes.len().min(mem.data.len());
                            mem.data[..n].copy_from_slice(&bytes[..n]);
                            loaded_image = n > 0;
                            tracing::info!(
                                "loaded {n} bytes into '{}' region @ {:#010x} from {path}",
                                region.name,
                                region.base
                            );
                        }
                        Err(e) => tracing::warn!(
                            "region '{}' image {path} (${env}) unreadable: {e}",
                            region.name
                        ),
                    }
                }
            }
            // Skip an empty image_env region *only* when it's based at address
            // 0: a zero-filled window there would shadow the Cortex-M flash boot
            // alias (breaks RP2040 bare-metal onboarding ELFs that rely on
            // VTOR=0 → flash). Nonzero-based ROM windows (e.g. the C3 IROM @
            // 0x40000000 / DROM @ 0x3FF00000) must stay installed as zeros even
            // with no on-disk image: on the wasm/browser path there is no
            // filesystem to preload from, and the ROM arrives later as blobs
            // that `inject_rom_regions` copies into these slots. Dropping them
            // here left no slot to fill, which is what surfaced to users as
            // "C3 flash fast-start: chip YAML declares no IROM region at
            // 0x40000000". Regions without image_env (plain RAM holes) are
            // always installed as zeros.
            if region.image_env.is_some() && !loaded_image && region.base == 0 {
                tracing::debug!(
                    "skipping empty image_env region '{}' @ {:#010x}",
                    region.name,
                    region.base
                );
                continue;
            }
            extra_mem.push(mem);
        }

        let mut bus = Self {
            flash_thunks: std::collections::HashMap::new(),
            flash: LinearMemory::new_erased(flash_size as usize, chip.flash.base),
            ram: LinearMemory::new(ram_size as usize, chip.ram.base),
            extra_mem,
            peripherals: Vec::new(),
            debug_schemas: Self::load_debug_schemas(chip, manifest),
            // Filled by `record_external_devices` below — the one home for it.
            external_device_decls: Vec::new(),
            nvic: None,
            observers: Vec::new(),
            config: crate::SimulationConfig::default(),
            bit_band_enabled: Self::chip_has_bit_band(chip),
            reset_vector_offset: chip.reset_vector_offset,
            atomic_register_aliases: chip.atomic_register_aliases,
            pending_cpu_irqs: [0; 2],
            dport_idx: None,
            rcc_idx: None,
            clock_gating_bypass: false,
            fault_unclocked: std::collections::HashMap::new(),
            peripheral_ranges: Vec::new(),
            legacy_tick_indices: Vec::new(),
            bus_tick_indices: Vec::new(),
            scheduler_driver_indices: Vec::new(),
            matrix_source_scratch: Vec::new(),
            peripheral_hint: Cell::new(None),
            last_route: Cell::new(None),
            last_gap: Cell::new(None),
            last_gpio_in: None,
            current_cycle: 0,
            cycle_clock: crate::CycleClock::default(),
            pending_schedule: Vec::new(),
            freerunning_timer_poll_mmio: Cell::new(0),
            side_effecting_mmio: Cell::new(0),
            legacy_walk_disabled: false,
            hcsr04: Vec::new(),
            gpio_devices: Vec::new(),
            ws2812: Vec::new(),
            servos: Vec::new(),
            step_dir_motors: Vec::new(),
            h_bridge_motors: Vec::new(),
            motors: Vec::new(),
            motor_cycle_anchor: 0,
            ili9341_parallel: Vec::new(),
            unipolar_steppers: Vec::new(),
            tm1637: Vec::new(),
            hx711: Vec::new(),
            seven_segment: Vec::new(),
            analog_inputs: Vec::new(),
            can_diagnostic_testers: Vec::new(),
            can_uds_testers: Vec::new(),
            can_log_players: Vec::new(),
            esp32c3_irq_routing: false,
            riscv_irq_lines: 0,
            esp32c3_system_idx: None,
            esp32c3_interrupt_core0_idx: None,
            esp32c3_irq_cache: None,
            esp32c3_asserted_sources: [0; 2],
            esp32c3_sched_asserted_sources: [0; 2],
            esp32c3_sensitive_idx: None,
            esp32c3_pms: None,
            pms_write_bypass: false,
            esp32c3_pms_armed: false,
            esp32s3_irq_routing: false,
            esp32s3_intmatrix_idx: None,
            esp32s3_asserted_sources: [0; 2],
            esp32s3_sched_asserted_sources: [0; 2],
            flash_models_ops: false,
            nordic_gpio_service: false,
            hcsr04_scheduling_disabled: false,
            flash_error_flags_idx: None,
            nrf52_nvmc_idx: None,
            bus_trace: bus_trace::new_log(),
            logic_tap: crate::logic_capture::LogicTap::new(),
            pin_map: std::collections::HashMap::new(),
        };
        bus.record_external_devices(manifest);

        // Authoritative pin map (silicon truth) — resolution prefers this over the
        // label-letter parse; see routing::resolve_pin_odr.
        for (label, loc) in &chip.pins {
            bus.pin_map
                .insert(label.to_ascii_uppercase(), (loc.gpio.clone(), loc.bit));
        }

        let mut merged_peripherals = chip.peripherals.clone();
        for m_p in &manifest.peripherals {
            if let Some(existing) = merged_peripherals.iter_mut().find(|p| p.id == m_p.id) {
                // Merge config map
                for (k, v) in &m_p.config {
                    existing.config.insert(k.clone(), v.clone());
                }
                // Also override other fields if provided
                if m_p.base_address != 0 {
                    existing.base_address = m_p.base_address;
                }
                if m_p.irq.is_some() {
                    existing.irq = m_p.irq;
                }
                if m_p.size.is_some() {
                    existing.size = m_p.size.clone();
                }
            } else {
                merged_peripherals.push(m_p.clone());
            }
        }

        // External-device ids already attached by a chip-specific I²C path
        // (the `i2c` / `esp32c3_i2c` arms below). The generic external-device
        // loop must NOT re-process these — otherwise a device that the bus
        // loader correctly attached as an I²C slave would also fall through to
        // the generic `_ =>` arm and emit a spurious "Unsupported external
        // device" WARN (it is supported — just by a path that ran first).
        let mut attached_i2c_ext_ids: std::collections::HashSet<&str> =
            std::collections::HashSet::new();

        // I²C bus-switch topology. `external_devices[].connection` may name
        // another external device's id instead of a controller — that is how a
        // slave is placed behind a TCA9548A, the only way several devices with
        // the same fixed address can share one bus. Validate the whole shape up
        // front (this runs for EVERY family, since every peripheral factory is
        // invoked from this loop) so a mis-wired switch is a loud error rather
        // than a device that quietly answers nothing.
        //
        // Every device behind a switch is marked attached here: it is wired as
        // part of its parent by `build_i2c_tree`, and letting it also reach the
        // generic attach loop below would put a second copy straight on the
        // controller — on the wrong bus segment, ahead of the switch.
        let mux_children = crate::peripherals::components::validate_i2c_mux_topology(manifest)?;
        attached_i2c_ext_ids.extend(mux_children.iter().copied());

        for p_cfg in &merged_peripherals {
            let canonical_type = Self::canonical_peripheral_type(&p_cfg.r#type);
            if canonical_type != p_cfg.r#type.to_ascii_lowercase() {
                tracing::debug!(
                    "Canonicalized peripheral type '{}' -> '{}' for id '{}'",
                    p_cfg.r#type,
                    canonical_type,
                    p_cfg.id
                );
            }

            // Out-of-tree chip plugins get first claim on every peripheral
            // type: `None` from all of them falls through to the in-tree
            // factories, `Some(Err)` aborts the build (a plugin that recognises
            // its own type but fails to build it must not be masked by the
            // unknown-type stub fallback below).
            let plugin_dev: Option<Box<dyn Peripheral>> = plugins
                .iter()
                .find_map(|plugin| {
                    plugin.try_build_peripheral(
                        &crate::plugin::PeripheralBuildCtx {
                            canonical_type: &canonical_type,
                            manifest,
                            bus_trace: &bus.bus_trace,
                            chip_map: crate::peripherals::chip_map::ChipMap::new(
                                &merged_peripherals,
                            ),
                        },
                        p_cfg,
                    )
                })
                .transpose()
                .with_context(|| {
                    format!(
                        "chip plugin failed to build peripheral '{}' (type '{}')",
                        p_cfg.id, p_cfg.r#type
                    )
                })?;

            // Per-family factories own their peripheral arms in their own modules,
            // so this central match stops growing (and shrinks as families migrate
            // out). Try them first; unmigrated families fall through to the match.
            let family_dev = plugin_dev
                .or_else(|| crate::peripherals::esp32s3::factory::try_build(&canonical_type, p_cfg))
                .or_else(|| crate::peripherals::esp32c3::factory::try_build(&canonical_type, p_cfg))
                // ESP32-classic was missing from this chain. Its factory has
                // always existed with all 14 `esp32_*` types, but only the
                // Xtensa builder called it, so a plain `from_config` bus --
                // the path a system manifest takes -- could not construct an
                // ESP32 peripheral at all. Declaring `uart1` in esp32.yaml
                // therefore failed the build outright with "no register
                // layout modelled yet", when the model was sitting right
                // there. That guard was doing its job: refusing to map an
                // ESP32 UART onto an STM32 layout is exactly right, and the
                // fix it asks for ("add a dedicated model") was to call the
                // model that already existed.
                .or_else(|| crate::peripherals::esp32::factory::try_build(&canonical_type, p_cfg))
                .or_else(|| {
                    crate::peripherals::nrf52::factory::try_build(
                        &canonical_type,
                        p_cfg,
                        manifest,
                        &bus.bus_trace,
                        crate::peripherals::chip_map::ChipMap::new(&merged_peripherals),
                    )
                })
                .or_else(|| {
                    crate::peripherals::nrf54l::factory::try_build(
                        &canonical_type,
                        p_cfg,
                        manifest,
                        &bus.bus_trace,
                    )
                });
            if let Some(dev) = family_dev {
                // The nRF52 serial-instance mux (SPIM0/TWIM0) attaches all
                // external devices connected to the shared MMIO window itself,
                // so mark them here so the kit registry pass below does not
                // try to attach them a second time (which would fail because
                // Nrf52SerialInstance is not an I2c/Esp32c3I2c).
                //
                // The standalone TWIM model does the same thing in its own
                // factory arm and needs the same bookkeeping. Without it a
                // device on a TWIM bus is attached twice when its type is in
                // the kit registry (mpu6050), and emits a bogus "Unsupported
                // external device" WARN when it is not (max30102, cap1188,
                // drv2605) — despite having been attached correctly. Found
                // while bringing up the nRF54L15 smart-ring system.
                if canonical_type == "nrf52_serial_instance"
                    || canonical_type == "nrf52840_twim"
                    || canonical_type == "nrf52_twim"
                    || canonical_type == "nrf54l_twim"
                {
                    for ext in &manifest.external_devices {
                        if ext.connection != p_cfg.id {
                            continue;
                        }
                        // Kits are attached by the universal pass after the
                        // controller is on the bus (nRF factories skip kit
                        // types in their own loop). Only mark factory-only
                        // residue as already attached.
                        if crate::peripherals::kit::registry::lookup(&ext.r#type).is_some() {
                            continue;
                        }
                        // Only suppress the kit pass when the family factory
                        // actually can build this type. Kit-only devices were
                        // previously marked attached even when the factory
                        // warned "unknown device type" and skipped them —
                        // leaving the bus empty (matrix L3 nRF ANACK on INA219).
                        let factory_handles =
                            crate::peripherals::components::build_external_i2c_device(
                                &ext.r#type,
                                &ext.id,
                                &ext.config,
                            )
                            .is_some();
                        if factory_handles {
                            attached_i2c_ext_ids.insert(ext.id.as_str());
                        }
                    }
                }
                bus.push_peripheral(p_cfg, dev)?;
                continue;
            }
            // Cross-vendor / generic peripherals (fallible: size + profile parsing).
            if let Some(dev) = crate::peripherals::generic_factory::try_build(
                &canonical_type,
                p_cfg,
                manifest,
                &bus.bus_trace,
            )? {
                bus.push_peripheral(p_cfg, dev)?;
                continue;
            }

            // I²C controllers that carry external slaves. Build the controller,
            // REGISTER it, then attach every wired slave through the single bus
            // choke point `attach_i2c_slave`, which wraps each device into the
            // shared bus trace. There is no per-controller `set_bus_trace` and no
            // inline wrapping — a family that reaches the bus this way cannot be
            // silently untraced (the ESP32-C3 blind-bus bug that motivated this).
            if matches!(
                canonical_type.as_str(),
                "i2c"
                    | "stm32f1_i2c"
                    | "stm32f2_i2c"
                    | "stm32f4_i2c"
                    | "stm32f7_i2c"
                    | "efm32ggi2ccontroller"
                    | "esp32c3_i2c"
            ) {
                let controller: Box<dyn Peripheral> = if canonical_type == "esp32c3_i2c" {
                    // ESP32-C3 behavioral I²C0 controller (command-list engine);
                    // the C3 (RISC-V) reaches it through this config loader rather
                    // than a hand-wired system builder.
                    Box::new(crate::peripherals::esp32c3::i2c::Esp32c3I2c::new())
                } else {
                    let layout: crate::peripherals::i2c::I2cRegisterLayout =
                        Self::parse_profile_or_default(p_cfg, "I2C")?;
                    let mut ctl = crate::peripherals::i2c::I2c::new_with_layout(layout);
                    // Optional ERROR-line vector. STM32 splits I2C into two NVIC
                    // lines: EVENT (the peripheral's `irq:`) and ERROR. AF/BERR/
                    // ARLO/OVR raise ERROR, and an interrupt-mode HAL only learns
                    // an address was NACKed via that handler, so a chip that
                    // declares only the EVENT vector makes every NACK look like a
                    // 100 ms timeout to real firmware.
                    if let Some(err_irq) = p_cfg.config.get("irq_error").and_then(|v| v.as_u64()) {
                        ctl.set_error_irq(err_irq as u32);
                    }
                    Box::new(ctl)
                };
                bus.push_peripheral(p_cfg, controller)?;
                for ext in &manifest.external_devices {
                    if ext.connection != p_cfg.id {
                        continue;
                    }
                    // Types the universal pass (parts → kit registry →
                    // declarative) claims attach THERE, not via the legacy
                    // factory — otherwise a type living in both (aht20,
                    // bme280, …) would resolve factory-first here and
                    // kit-first on the Xtensa path: same manifest, different
                    // model per chip family.
                    if crate::peripherals::kit::registry::lookup(&ext.r#type).is_some()
                        || super::declarative_device::lookup(&ext.r#type)?.is_some()
                        || super::part_pack::lookup(manifest, &ext.r#type)?.is_some()
                    {
                        continue;
                    }
                    // `build_i2c_tree`, not the bare factory: when `ext` is a
                    // bus switch this also builds every device wired behind it
                    // and buckets them onto its channels, so what reaches the
                    // attach choke point below is ONE assembled unit — the
                    // switch — exactly as on the board.
                    match crate::peripherals::components::build_i2c_tree(manifest, ext)? {
                        Some(device) => {
                            tracing::info!(
                                "i2c attach: '{}' (type={}) -> '{}'",
                                ext.id,
                                ext.r#type,
                                p_cfg.id
                            );
                            bus.attach_i2c_slave_with_route(&p_cfg.id, device, Some(&ext.route))?;
                            attached_i2c_ext_ids.insert(ext.id.as_str());
                        }
                        None => {
                            // Devices migrated to the PeripheralKit contract are
                            // attached by the kit pass below; their absence here
                            // is expected. Only warn for types no path handles.
                            if crate::peripherals::kit::registry::lookup(&ext.r#type).is_none() {
                                tracing::warn!(
                                    "i2c attach skipped: unknown device type '{}' for external id '{}' on bus '{}'",
                                    ext.r#type,
                                    ext.id,
                                    p_cfg.id
                                );
                            }
                        }
                    }
                }
                continue;
            }

            // Remaining: the YAML descriptor loaders (declarative / strict_ir) and
            // the unknown-type stub fallback.
            let dev: Box<dyn Peripheral> = match canonical_type.as_str() {
                "uart" | "stm32_uart" | "stm32f1_uart" | "stm32f2_uart" | "stm32f4_uart"
                | "stm32f7_usart" | "stm32h5_usart" | "efm32_uart" | "nxp_lpuart" | "ns16550"
                | "pl011" | "gaislerapbuart" => {
                    let layout = Self::uart_layout_for(p_cfg)?;
                    // CR3 writable mask is a per-part delta on the shared F1 map:
                    // F1 implements [10:0] (0x07FF), F4 adds bit 11 ONEBIT (0x0FFF).
                    // YAML: `config: { cr3_mask: 0xFFF }`; default F1.
                    let cr3_mask: u32 = p_cfg
                        .config
                        .get("cr3_mask")
                        .and_then(|v| v.as_u64())
                        .map(|n| n as u32)
                        .unwrap_or(0x0000_07FF);
                    Box::new(crate::peripherals::uart::Uart::new_with_layout_cr3(
                        layout, cr3_mask,
                    ))
                }
                "gpio" | "stm32_gpioport" | "stm32f4_gpio" | "efmgpioport" | "npcx_gpio"
                | "imxrt_gpio" => {
                    // Deterministic, type-driven layout resolution. The bare
                    // vendor-neutral `gpio` type MUST name a profile; it is never
                    // silently defaulted onto STM32F1 (which would move the ODR
                    // offset and blank a display's D/C line — the KW41Z "cow" bug).
                    let layout: GpioRegisterLayout = Self::gpio_layout_for(p_cfg)?;
                    // Optional `reg_offset`: the window's offset inside the
                    // family register map (the SVD `addressBlock.offset`).
                    // nRF53/nRF54 GPIO ports declare 0x500, because Nordic
                    // bases those at OUT rather than at the block start and
                    // two ports are only 0x300 apart — see
                    // `GpioPort::window_offset`.
                    let window_offset: u64 = p_cfg
                        .config
                        .get("reg_offset")
                        .and_then(|v| v.as_u64())
                        .unwrap_or(0);
                    // For nRF52 ports, an optional `num_pins` config key caps the
                    // valid-pin range (e.g. 16 for nRF52840 P1 which has P1.0–P1.15).
                    // Writes outside that range are discarded; reads return 0.
                    if layout == GpioRegisterLayout::Nrf52 {
                        let num_pins: u32 = p_cfg
                            .config
                            .get("num_pins")
                            .and_then(|v| v.as_u64())
                            .map(|n| n as u32)
                            .unwrap_or(32);
                        Box::new(
                            crate::peripherals::gpio::GpioPort::new_nrf52(num_pins)
                                .with_window_offset(window_offset),
                        )
                    } else if layout == GpioRegisterLayout::Stm32V2
                        && p_cfg.config.contains_key("reset_moder")
                    {
                        // Per-port silicon reset values (MODER/OSPEEDR/PUPDR)
                        // supplied by the chip yaml; missing keys default to 0.
                        let cfg_u32 = |key: &str| -> u32 {
                            p_cfg
                                .config
                                .get(key)
                                .and_then(|v| v.as_u64())
                                .map(|n| n as u32)
                                .unwrap_or(0)
                        };
                        Box::new(
                            crate::peripherals::gpio::GpioPort::new_stm32v2_with_resets(
                                cfg_u32("reset_moder"),
                                cfg_u32("reset_ospeedr"),
                                cfg_u32("reset_pupdr"),
                            )
                            .with_window_offset(window_offset),
                        )
                    } else {
                        Box::new(
                            crate::peripherals::gpio::GpioPort::new_with_layout(layout)
                                .with_window_offset(window_offset),
                        )
                    }
                }
                // ESP32-C3 behavioral GP-SPI2 controller (CPU/W-buffer
                // transaction engine). Same Espressif GP-SPI IP family as the
                // S3; the C3 chip yaml selects this type for `spi2`. The
                // descriptor `irq` overrides the default intr-matrix source
                // (GP-SPI2 = 19 on the C3).
                "esp32c3_spi" => {
                    let src = p_cfg
                        .irq
                        .unwrap_or(crate::peripherals::esp32c3::spi::SPI2_INTR_SOURCE_ID);
                    Box::new(crate::peripherals::esp32c3::spi::Esp32c3Spi::new(src))
                }
                // ESP32-C3 behavioral SAR ADC controller (one-shot conversion
                // engine). Drives a channel-dependent result + DONE handshake
                // for the IDF `adc_oneshot` flow; the C3 chip yaml selects this
                // type for `apb_saradc`.
                "esp32c3_apb_saradc" => {
                    let src = p_cfg.irq.unwrap_or(
                        crate::peripherals::esp32c3::apb_saradc::APB_SARADC_INTR_SOURCE_ID,
                    );
                    Box::new(crate::peripherals::esp32c3::apb_saradc::Esp32c3ApbSarAdc::new(src))
                }
                // ESP32-C3 behavioral LEDC (LED PWM) controller. Drives the
                // four low-speed timers as live up-counters that advance with
                // elapsed cycles and latch LSTIMERx_OVF on wrap; the C3 chip
                // yaml selects this type for `ledc`. The descriptor `irq`
                // overrides the default intr-matrix source (LEDC = 23).
                "esp32c3_ledc" => {
                    let src = p_cfg
                        .irq
                        .unwrap_or(crate::peripherals::esp32c3::ledc::LEDC_INTR_SOURCE_ID);
                    Box::new(crate::peripherals::esp32c3::ledc::Esp32c3Ledc::new(src))
                }
                // Nordic peripherals — register-surface models cross-validated
                // by hw-oracle::nrf52_onboarding_diff. See peripherals/nrf52/.
                // TWIM (I²C master with EasyDMA) — nRF52840 PS §6.31.
                // `nrf52840_i2c` is the canonical chip-YAML type; `nrf52840_twim`
                // and `nrf52_twim` are also accepted so firmware configs that
                // name it more precisely still resolve here.
                // ESP32-family Timer Group (TIMG0/TIMG1) — the same IP block is
                // used by the classic ESP32, S3, and C3.  All share the register
                // layout: T0CONFIG=0x00, T0LO=0x04, T0HI=0x08, T0UPDATE=0x0C.
                // Wiring via this type string gives C3 (RISC-V, from_config path)
                // the same live counter that the Xtensa chips get via their
                // hard-wired system builders.
                "declarative" => {
                    let descriptor_path = p_cfg
                        .config
                        .get("path")
                        .and_then(|v| v.as_str())
                        .ok_or_else(|| {
                            anyhow::anyhow!(
                                "Field 'path' is required in 'config' for declarative peripheral '{}'",
                                p_cfg.id
                            )
                        })?;

                    // Prefer the descriptor embedded in the binary (wasm32 has no
                    // std::fs); fall back to the filesystem for native builds and
                    // any path not embedded.
                    let desc = if let Some(embedded) =
                        super::embedded_descriptors::lookup(descriptor_path)
                    {
                        labwired_config::PeripheralDescriptor::from_yaml(embedded).with_context(
                            || {
                                format!(
                                    "Failed to parse embedded declarative descriptor for '{}' ('{}')",
                                    p_cfg.id, descriptor_path
                                )
                            },
                        )?
                    } else {
                        let resolved_path =
                            Self::resolve_peripheral_path(manifest, descriptor_path);
                        labwired_config::PeripheralDescriptor::from_file(&resolved_path).with_context(
                            || {
                                format!(
                                    "Failed to load declarative descriptor for '{}' from '{}' (resolved to '{}')",
                                    p_cfg.id,
                                    descriptor_path,
                                    resolved_path.display()
                                )
                            },
                        )?
                    };

                    Box::new(crate::peripherals::declarative::GenericPeripheral::new(
                        desc,
                    ))
                }
                "strict_ir" => {
                    let descriptor_path = p_cfg
                        .config
                        .get("path")
                        .and_then(|v| v.as_str())
                        .ok_or_else(|| {
                            anyhow::anyhow!(
                                "Field 'path' is required in 'config' for strict_ir peripheral '{}'",
                                p_cfg.id
                            )
                        })?;

                    let resolved_path = Self::resolve_peripheral_path(manifest, descriptor_path);
                    let content = std::fs::read_to_string(&resolved_path).with_context(|| {
                        format!(
                            "Failed to read IR file '{}' (resolved to '{}')",
                            descriptor_path,
                            resolved_path.display()
                        )
                    })?;
                    let ir_peripheral = match serde_json::from_str::<labwired_ir::IrPeripheral>(
                        &content,
                    ) {
                        Ok(peripheral) => peripheral,
                        Err(peripheral_err) => {
                            let device: labwired_ir::IrDevice = serde_json::from_str(&content)
                                .with_context(|| {
                                    format!(
                                        "Failed to parse Strict IR from {} as IrPeripheral ({}) or IrDevice",
                                        resolved_path.display(),
                                        peripheral_err
                                    )
                                })?;

                            if let Some(peripheral) = device.peripherals.get(&p_cfg.id) {
                                peripheral.clone()
                            } else if device.peripherals.len() == 1 {
                                device
                                    .peripherals
                                    .into_values()
                                    .next()
                                    .expect("len() checked above")
                            } else {
                                let available = device
                                    .peripherals
                                    .keys()
                                    .cloned()
                                    .collect::<Vec<_>>()
                                    .join(", ");
                                return Err(anyhow::anyhow!(
                                    "Strict IR '{}' contains multiple peripherals [{}]; no match for id '{}'",
                                    resolved_path.display(),
                                    available,
                                    p_cfg.id
                                ));
                            }
                        }
                    };

                    let desc: labwired_config::PeripheralDescriptor = ir_peripheral.into();

                    Box::new(crate::peripherals::declarative::GenericPeripheral::new(
                        desc,
                    ))
                }
                "strict_ir_internal" => {
                    let val = p_cfg.config.get("internal_ir_peripheral").ok_or_else(|| {
                        anyhow::anyhow!("Missing internal_ir_peripheral config for converted IR")
                    })?;
                    // Convert yaml Value (which was serde_yaml::to_value(p)) back to IrPeripheral
                    let ir_peripheral: labwired_ir::IrPeripheral =
                        serde_yaml::from_value(val.clone())?;
                    let desc: labwired_config::PeripheralDescriptor = ir_peripheral.into();

                    Box::new(crate::peripherals::declarative::GenericPeripheral::new(
                        desc,
                    ))
                }
                // No model, no descriptor, no plugin claimed it. This used to
                // install a zero-filled stub for ANY type, which is the one
                // outcome this product cannot ship: firmware talks to the
                // address, reads back zeros, and the run reports success while
                // having modelled nothing.
                //
                // So an unrecognised type now FAILS THE LOAD. The only
                // exceptions are the types measured as already reaching this
                // arm in the shipped configs, each with a written reason in
                // `known_stubs.rs` — see that file for the rules and the exit.
                other => {
                    // Census (measurement only; a no-op unless `silent-census`
                    // is compiled in) — recorded BEFORE the allowlist decides,
                    // so it keeps counting exactly what it counted when this
                    // arm stubbed unconditionally. The histogram is what makes
                    // the allowlist shrinkable: it says which entries are
                    // actually reached, not just which ones are declared.
                    crate::census::record_stub(&p_cfg.r#type);
                    match super::known_stubs::known_stub_reason(other) {
                        Some(reason) => {
                            tracing::debug!(
                                "peripheral '{}' (type '{}') resolves to a zero stub; \
                             allowlisted: {}",
                                p_cfg.id,
                                p_cfg.r#type,
                                reason
                            );
                            Box::new(crate::peripherals::stub::StubPeripheral::new(0x00))
                        }
                        None => {
                            return Err(anyhow::anyhow!(
                                "unknown peripheral type '{}' for peripheral '{}' in chip '{}' \
                             (chip file '{}'): this engine has no model for it, no plugin \
                             claimed it{}, and it is not on the known-stub allowlist. \
                             Refusing to answer it with a zero-filled stub — firmware would \
                             read zeros from silicon that was never modelled and the run \
                             would still report success. Fix it one of three ways: model the \
                             peripheral, describe it with `type: declarative` and a \
                             descriptor `path`, or — if answering zeros really is right — \
                             declare `type: stub` in the chip YAML (or add '{}' to \
                             KNOWN_STUBBED_PERIPHERAL_TYPES in \
                             crates/core/src/bus/known_stubs.rs with a written reason).",
                                p_cfg.r#type,
                                p_cfg.id,
                                chip.name,
                                manifest.chip,
                                if plugins.is_empty() {
                                    String::new()
                                } else {
                                    format!(" (of {} loaded plugin(s))", plugins.len())
                                },
                                other,
                            ));
                        }
                    }
                }
            };

            bus.push_peripheral(p_cfg, dev)?;
        }

        // Bus-trace wiring is no longer a per-peripheral property: the shared
        // trace is applied at the single attach choke point (`attach_i2c_slave`
        // / `attach_spi_device`), so there is nothing to wire here.
        for ext in &manifest.external_devices {
            // Already attached as an I²C slave by a chip-specific i2c path
            // (the `i2c` / `esp32c3_i2c` arms above). Don't let it fall through
            // to the generic arms — it is handled, so re-processing it here
            // would emit a spurious "Unsupported external device" WARN.
            if attached_i2c_ext_ids.contains(ext.id.as_str()) {
                continue;
            }
            // The canonical universal pass (parts → kit registry →
            // declarative descriptors) — one shared implementation, see
            // `bus::external_devices`. Anything it doesn't claim falls
            // through to the hand-written arms below.
            if matches!(
                super::external_devices::attach_external_device_universal(&mut bus, manifest, ext)?,
                super::external_devices::UniversalResolution::Attached
            ) {
                continue;
            }
            // Residual product attach arms are gone — kits / declarative / parts
            // claim every supported external above. History of migrations:
            // ili9341, sensors, neo6m, bg770a, iolink-master, SPI displays,
            // hc-sr04/dht22/rotary/keypad declarative, neopixel/servo/motors
            // kits, CAN testers (`CAN_*_KIT`). Nothing left here but fail-loud.
            // A green run with a silently missing device proves nothing.
            return Err(super::external_devices::unsupported_external_device_error(
                &format!("from_config (connection '{}')", ext.connection),
                ext,
            ));
        }

        bus.rebuild_peripheral_ranges();
        // Buttons/switches declared in `board_io` become bus-resident stimulus
        // devices. They are the one input family the canvas emits WITHOUT an
        // external_devices entry (a passive contact needs no device block), so
        // without this pass a button on the canvas is inert in a headless run:
        // it drives no pin and exposes no `pressed` channel, and every stimulus
        // naming it is rejected as an unknown channel. Ranges must already be
        // rebuilt — the IDR address is resolved through the registered GPIO.
        bus.attach_board_io_buttons(manifest);
        // ESP32-C3: share IO_MUX pad controls with GPIO so an Arduino
        // `INPUT_PULLUP` changes the floating input level. No-op for every
        // other chip.
        bus.wire_esp32c3_pad_controls();
        // ESP32-C3: share the I²C0 bit engine's live SDA/SCL line levels with
        // the C3 GPIO model so matrix-routed pads carry the real waveform.
        // No-op for every other chip.
        bus.wire_esp32c3_i2c_pads();
        // And GP-SPI2's SCK/MOSI/CS plus each UART's TX, so those buses are
        // measurable on the C3 too rather than reading as a flat line. MISO/RX
        // are deliberately unbound — nothing drives them.
        bus.wire_esp32c3_spi_pads();
        bus.wire_esp32c3_uart_pads();
        // Same for the S3's I²C0, whose pads reach GPIO through the S3 output
        // matrix rather than an AF nibble.
        bus.wire_esp32s3_i2c_pads();
        // …and the S3's GP-SPI2 / UART TX, whose matrix indices are 101/103/110
        // and 12/15/18 — neither the C3's nor the classic part's.
        bus.wire_esp32s3_spi_pads();
        bus.wire_esp32s3_uart_pads();
        // Same for classic ESP32 (LX6), whose matrix indices are 29/30 —
        // neither the C3's 53/54 nor the S3's 89/90.
        bus.wire_esp32_i2c_pads();
        // …and the classic part's VSPI (SPI3) and UART TX. VSPI's 63/65/68
        // happen to be the C3's FSPI numbers and are NOT SPI signals on the S3;
        // the UART indices 14/17/198 are the classic part's alone.
        bus.wire_esp32_spi_pads();
        bus.wire_esp32_uart_pads();
        // RP2040: bind I²C wires to the pads IO_BANK0's FUNCSEL can route them to.
        bus.wire_rp2040_i2c_pads();
        // Same for the RP2040 UARTs' TX/RX, so serial output is a waveform on
        // the routed pad and not just console text.
        bus.wire_rp2040_uart_pads();
        // And the RP2040 SPI controllers' SCK/MOSI/CSn, so a probe on an SPI pad
        // measures the shifted bytes rather than the SIO output latch. MISO is
        // deliberately unrouted — nothing drives it.
        bus.wire_rp2040_spi_pads();
        // STM32: share each classic/FIFO SPI bit engine's live SCK/MOSI/MISO
        // line levels with the STM32 GPIO ports so AF-routed pads carry the
        // real waveform. No-op for every other chip.
        bus.wire_stm32_spi_pads();
        // Same for each STM32 I²C controller's SCL/SDA, so that bus is
        // measurable too rather than reading as a flat line.
        bus.wire_stm32_i2c_pads();
        // And each USART's TX/RX, so serial output is a waveform on the routed
        // AF pad rather than the idle GPIO latch.
        bus.wire_stm32_uart_pads();
        // nRF52: bind every TWIM/SPIM/UARTE wire to every pad its PSEL can
        // name. Unlike the four above this is not a datasheet AF table — the
        // pad has no function register on this family, so the peripherals
        // publish which pin they claim and the port reads it. No-op for every
        // other chip.
        bus.wire_nrf52_pads();
        // Resolve declared per-peripheral RCC clock-gates now that every
        // peripheral (incl. the RCC, needed to map reg-name → offset) is on the
        // bus. Peripherals without a `clock:` field stay ungated.
        bus.resolve_clock_gates(&merged_peripherals)?;
        bus.install_motor_models(manifest)?;
        // Walk-deletion decision (only consulted under the `event-scheduler`
        // feature; the legacy build always walks, so this is inert there).
        //
        //   Some(true)  → force deleted (hand opt-in / escape hatch)
        //   Some(false) → pin the walk ON, overriding auto-derivation
        //   None        → auto-derive: delete iff EVERY peripheral is provably
        //                 walk-independent for all firmware states.
        //
        // The auto-derivation is deliberately conservative — see
        // `derive_walk_deletable`. It only fires when deleting the walk is
        // byte-identical for ANY reachable firmware state, so it can never
        // silently starve a peripheral of its per-cycle `tick()`. A hand
        // `walk_deleted: true` stays honored for configs whose byte-identity is
        // firmware-specific (the firmware never arms the timers/ADC/DMA the chip
        // descriptor instantiates) and thus not config-derivable.
        bus.legacy_walk_disabled = match manifest.walk_deleted {
            Some(explicit) => explicit,
            None => bus.derive_walk_deletable(),
        };

        // One bind API only: attach_lab_air. Single-board CLI mints a private
        // lab air here so MQTT/CSQ work without a browser. Multi-node World and
        // the playground rebind via attach_lab_air with a *shared* air (same
        // method — deliberate replace of the private fabric, not double-bind).
        if bus.has_cellular_modem() {
            let node = if manifest.name.is_empty() {
                "lab"
            } else {
                manifest.name.as_str()
            };
            bus.attach_private_lab_air(node);
        }

        Ok(bus)
    }

    /// Materialise every `board_io` button/switch binding as a bus-resident
    /// [`Button`](crate::peripherals::components::button::Button).
    ///
    /// `board_io` is the canvas compiler's existing output for passive contacts
    /// — it already carries the owning GPIO peripheral, the pin index, and the
    /// `active_high` polarity derived from which rail the other terminal is
    /// wired to. Reading it here rather than inventing a second declaration
    /// keeps ONE source of truth for "there is a button on this pin".
    ///
    /// Only `signal: input` bindings attach: a `kind: button` emitted as an
    /// output is not a contact the firmware samples. A binding naming a
    /// peripheral that is not registered is skipped with a warning rather than
    /// failing the build — the rest of the system still runs, the button simply
    /// stays undrivable.
    ///
    /// The button is anchored to its GPIO by that peripheral's BASE address, not
    /// by an input-register address: the level is applied through the owning
    /// peripheral's `set_gpio_input`, which every GPIO model implements, so this
    /// works for a per-port register model (STM32, Nordic, Kinetis) and a single
    /// GPIO-matrix model (ESP32/C3/S3) alike.
    fn attach_board_io_buttons(&mut self, manifest: &SystemManifest) {
        use labwired_config::{BoardIoKind, BoardIoSignal};

        for binding in &manifest.board_io {
            if binding.kind != BoardIoKind::Button || binding.signal != BoardIoSignal::Input {
                continue;
            }
            let Some(idx) = self.find_peripheral_index_by_name(&binding.peripheral) else {
                tracing::warn!(
                    "board_io button '{}' names unregistered peripheral '{}'; \
                     the button will not be drivable",
                    binding.id,
                    binding.peripheral
                );
                continue;
            };
            let anchor = self.peripherals[idx].base;
            let mut button = crate::peripherals::components::button::Button::with_channel(
                binding.id.clone(),
                (anchor, binding.pin),
                binding.active_high,
                binding.channel.as_deref(),
            );

            // Settle the released level NOW, before the firmware's first sample:
            // an active-low button whose pin is left at the input register's
            // reset value of 0 reads as a press that is never released, so a
            // sketch waiting on the button fires immediately at boot.
            //
            // Then PROVE the level landed. A GPIO model that does not honour an
            // externally driven level would leave a button that discovery
            // advertises, `set_input` accepts, and the firmware never sees move
            // — a stimulus that reports success and proves nothing. Where we
            // cannot demonstrate the drive, we do not claim the capability: the
            // button is dropped and the pin is left exactly as that chip had it.
            let (level, _) = button.service();
            let landed = self.drive_input_bit(anchor, binding.pin, level)
                && self.peripherals[idx].dev.read_gpio_input(binding.pin) == Some(level);
            if !landed {
                tracing::warn!(
                    "board_io button '{}' on '{}' pin {}: this GPIO model does not reflect an \
                     externally driven input level, so the button is not attached and cannot be \
                     driven by a stimulus",
                    binding.id,
                    binding.peripheral,
                    binding.pin
                );
                continue;
            }
            self.gpio_devices.push(Box::new(button));
        }
    }

    /// Install a GPIO edge observer on ESP32 / ESP32-S3 GPIO models when present.
    ///
    /// Public so kits (`Transport::GpioGroup`) and hand arms share one choke
    /// point — the same path `AttachCtx::install_gpio_observer` uses.
    pub fn install_gpio_observer<T>(bus: &mut SystemBus, observer: std::sync::Arc<T>)
    where
        T: crate::peripherals::esp32s3::gpio::GpioObserver
            + crate::peripherals::esp32::gpio::GpioObserver
            + 'static,
    {
        if let Some(idx) = bus.find_peripheral_index_by_name("gpio") {
            let any = bus.peripherals[idx].dev.as_any_mut();
            if let Some(gpio) =
                any.and_then(|a| a.downcast_mut::<crate::peripherals::esp32s3::gpio::Esp32s3Gpio>())
            {
                gpio.add_observer(observer);
                return;
            }
        }
        // Classic ESP32 GPIO (separate type).
        if let Some(idx) = bus.find_peripheral_index_by_name("gpio") {
            if let Some(gpio) = bus.peripherals[idx]
                .dev
                .as_any_mut()
                .and_then(|a| a.downcast_mut::<crate::peripherals::esp32::gpio::Esp32Gpio>())
            {
                gpio.add_observer(observer);
            }
        }
    }

    #[allow(dead_code)] // residual GPIO helpers kept for CAN/tester arms that may re-use them
    fn gpio_from_config(
        ext: &labwired_config::ExternalDevice,
        key: &str,
        alt_key: &str,
        default: &str,
    ) -> anyhow::Result<u8> {
        let label = ext
            .config
            .get(key)
            .or_else(|| ext.config.get(alt_key))
            .and_then(|v| v.as_str())
            .unwrap_or(default);
        Self::parse_esp32s3_gpio_pin(label)
            .or_else(|| Self::parse_esp32_gpio_pin(label))
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "{}: pin '{}' (config {}/{}) is not a parseable GPIO",
                    ext.id,
                    label,
                    key,
                    alt_key
                )
            })
    }

    #[allow(dead_code)]
    fn optional_gpio_from_config(
        ext: &labwired_config::ExternalDevice,
        key: &str,
        alt_key: &str,
    ) -> Option<u8> {
        let label = ext
            .config
            .get(key)
            .or_else(|| ext.config.get(alt_key))
            .and_then(|v| v.as_str())?;
        Self::parse_esp32s3_gpio_pin(label).or_else(|| Self::parse_esp32_gpio_pin(label))
    }
}

#[cfg(test)]
mod image_env_region_tests {
    use super::*;

    /// A nonzero-based `image_env` region with no loadable image must still be
    /// installed (zero-filled) so a later filler — `inject_rom_regions` on the
    /// wasm/browser fast-start path — has a slot to copy the ROM blobs into.
    /// Only a region based at address 0 (the RP2040 bootrom, which would shadow
    /// the Cortex-M flash boot alias) is dropped when empty.
    ///
    /// Regression for the browser "C3 flash fast-start: chip YAML declares no
    /// IROM region at 0x40000000" failure: with no filesystem to preload from,
    /// `from_config` used to drop the C3 IROM window entirely, leaving
    /// `inject_rom_regions` nothing to fill.
    #[test]
    fn empty_image_env_region_kept_unless_based_at_zero() {
        // `LABWIRED_TEST_MISSING_ROM` has no default image path and is never
        // set, so both regions below fail to load an image — exactly the
        // wasm/browser condition — without touching any real env var or file.
        let chip_yaml = r#"
name: "test-image-env"
arch: "riscv"
flash:
  base: 0x42000000
  size: "1KB"
ram:
  base: 0x3FC80000
  size: "1KB"
memory_regions:
  - name: "irom"
    base: 0x40000000
    size: "1KB"
    image_env: "LABWIRED_TEST_MISSING_ROM"
  - name: "bootrom_at_zero"
    base: 0x0
    size: "1KB"
    image_env: "LABWIRED_TEST_MISSING_ROM"
peripherals: []
"#;
        let manifest_yaml = r#"
name: "test-image-env-system"
chip: "test-image-env"
"#;
        let chip: ChipDescriptor = serde_yaml::from_str(chip_yaml).expect("parse chip");
        let manifest: SystemManifest = serde_yaml::from_str(manifest_yaml).expect("parse manifest");

        // Guard the hermeticity assumption: if some ambient env ever defines
        // this name, the test would silently stop exercising the empty path.
        assert!(
            std::env::var("LABWIRED_TEST_MISSING_ROM").is_err(),
            "test env var must be unset for this regression to be meaningful"
        );

        let bus = SystemBus::from_config(&chip, &manifest).expect("build bus");

        assert!(
            bus.extra_mem.iter().any(|m| m.base_addr == 0x4000_0000),
            "nonzero-based empty image_env region must be installed so \
             inject_rom_regions can fill it (was dropped → browser IROM error)"
        );
        assert!(
            !bus.extra_mem.iter().any(|m| m.base_addr == 0),
            "empty image_env region based at 0 must be dropped so it can't \
             shadow the flash boot alias"
        );
    }
}
