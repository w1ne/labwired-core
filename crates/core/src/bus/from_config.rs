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
    pub fn from_config(chip: &ChipDescriptor, manifest: &SystemManifest) -> anyhow::Result<Self> {
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
            flash: LinearMemory::new(flash_size as usize, chip.flash.base),
            ram: LinearMemory::new(ram_size as usize, chip.ram.base),
            extra_mem,
            peripherals: Vec::new(),
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
            last_gpio_in: [0; 2],
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
            esp32s3_irq_routing: false,
            esp32s3_intmatrix_idx: None,
            esp32s3_asserted_sources: [0; 2],
            esp32s3_sched_asserted_sources: [0; 2],
            flash_models_ops: false,
            iolink_master_attached: false,
            nordic_gpio_service: false,
            hcsr04_scheduling_disabled: false,
            flash_error_flags_idx: None,
            bus_trace: bus_trace::new_log(),
            logic_tap: crate::logic_capture::LogicTap::new(),
            pin_map: std::collections::HashMap::new(),
        };

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

            // Per-family factories own their peripheral arms in their own modules,
            // so this central match stops growing (and shrinks as families migrate
            // out). Try them first; unmigrated families fall through to the match.
            let family_dev =
                crate::peripherals::esp32s3::factory::try_build(&canonical_type, p_cfg)
                    .or_else(|| {
                        crate::peripherals::esp32c3::factory::try_build(&canonical_type, p_cfg)
                    })
                    .or_else(|| {
                        crate::peripherals::nrf52::factory::try_build(
                            &canonical_type,
                            p_cfg,
                            manifest,
                            &bus.bus_trace,
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
                    Box::new(crate::peripherals::i2c::I2c::new_with_layout(layout))
                };
                bus.push_peripheral(p_cfg, controller)?;
                for ext in &manifest.external_devices {
                    if ext.connection != p_cfg.id {
                        continue;
                    }
                    match crate::peripherals::components::build_external_i2c_device(
                        &ext.r#type,
                        &ext.id,
                        &ext.config,
                    ) {
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
                        Box::new(crate::peripherals::gpio::GpioPort::new_nrf52(num_pins))
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
                        Box::new(crate::peripherals::gpio::GpioPort::new_stm32v2_with_resets(
                            cfg_u32("reset_moder"),
                            cfg_u32("reset_ospeedr"),
                            cfg_u32("reset_pupdr"),
                        ))
                    } else {
                        Box::new(crate::peripherals::gpio::GpioPort::new_with_layout(layout))
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
                _other => {
                    tracing::debug!(
                        "Mapping unknown peripheral type '{}' to Stub for id '{}'",
                        p_cfg.r#type,
                        p_cfg.id
                    );
                    Box::new(crate::peripherals::stub::StubPeripheral::new(0x00))
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
            // First-pass: peripherals that have migrated to the unified
            // `PeripheralKit` contract are dispatched through the registry,
            // so each one ships its own `attach` next to its model instead
            // of a hand-written arm here.
            if let Some(kit) = crate::peripherals::kit::registry::lookup(&ext.r#type) {
                let mut ctx = crate::peripherals::kit::AttachCtx::new(&mut bus, ext);
                kit.attach(&mut ctx)?;
                continue;
            }
            // Second-pass: the GPIO / pin-timing family migrated to declarative
            // `configs/devices/*.yaml` descriptors. `attach_declarative_device`
            // resolves the descriptor's pin bindings and instantiates the named
            // primitive — no hand-written arm here.
            if let Some(desc) = super::declarative_device::lookup(&ext.r#type)? {
                bus.attach_declarative_device(ext, &desc)?;
                continue;
            }
            match ext.r#type.as_str() {
                // ili9341, adxl345/mpu6050/bme280/oled-ssd1306, neo6m-gps,
                // and bg770a-cellular dispatch through the PeripheralKit
                // registry above — see `peripherals::kit`.
                // iolink-master dispatches through the PeripheralKit registry above.
                // max31855, sn74hc165, ssd1680_tricolor_290, and pcd8544
                // dispatch through the PeripheralKit registry above.
                // hc-sr04 / hcsr04 now dispatches through the declarative device
                // path above (configs/devices/hc_sr04.yaml, `pulse_echo`
                // primitive) — see super::declarative_device.
                // dht22 / am2302 now dispatches through the declarative device
                // path above (configs/devices/dht22.yaml, `one_wire` primitive) —
                // see super::declarative_device.
                // rotary-encoder / rotary_encoder now dispatches through the
                // declarative device path above (configs/devices/rotary_encoder.yaml,
                // `quadrature` primitive) — see super::declarative_device.
                // keypad now dispatches through the declarative device path above
                // (configs/devices/keypad.yaml, `matrix` primitive) — see
                // super::declarative_device.
                "neopixel" | "ws2812" => {
                    // Addressable LED strip driven by a single-wire, self-clocked
                    // bit-stream on ONE GPIO. On the ESP32-S3 the RMT peripheral
                    // generates that waveform and the GPIO matrix (FUNC_OUT_SEL)
                    // routes it to the pad; this decoder attaches as a GPIO
                    // observer on the data pin and reconstructs pixels from the
                    // edge timing — purely edge-driven, no per-tick pass. On
                    // non-S3 boards there is no RMT→pad drive path yet, so the
                    // strip is stored for readback but simply never sees an edge.
                    let data = ext
                        .config
                        .get("data_pin")
                        .and_then(|v| v.as_str())
                        .unwrap_or("GPIO48")
                        .to_string();
                    let num_pixels = ext
                        .config
                        .get("num_pixels")
                        .and_then(|v| v.as_u64())
                        .unwrap_or(1) as usize;
                    let cpu_hz = ext
                        .config
                        .get("cpu_hz")
                        .and_then(|v| v.as_u64())
                        .unwrap_or(160_000_000);
                    let pin = Self::parse_esp32s3_gpio_pin(&data).ok_or_else(|| {
                        anyhow::anyhow!(
                            "neopixel '{}' data_pin '{}' could not be parsed to an ESP32-S3 GPIO (0..=48)",
                            ext.id,
                            data
                        )
                    })?;
                    let strip =
                        std::sync::Arc::new(crate::peripherals::components::ws2812::Ws2812::new(
                            pin, num_pixels, cpu_hz,
                        ));
                    // Install as a GPIO observer on the S3 GPIO peripheral, if one
                    // is registered (walk-free: filters by pin internally).
                    if let Some(idx) = bus.find_peripheral_index_by_name("gpio") {
                        if let Some(gpio) = bus.peripherals[idx].dev.as_any_mut().and_then(|a| {
                            a.downcast_mut::<crate::peripherals::esp32s3::gpio::Esp32s3Gpio>()
                        }) {
                            gpio.add_observer(strip.clone());
                        }
                    }
                    bus.ws2812.push(strip);
                }
                "servo" | "sg90" | "mg996r" => {
                    // Hobby PWM servo twin. Driven by GPIO edges and/or LEDC
                    // duty observers (ledcWrite path). Part id is stored so
                    // WASM `get_actuator_states` can key canvas animation.
                    let pin_label = ext
                        .config
                        .get("signal_pin")
                        .or_else(|| ext.config.get("control_pin"))
                        .or_else(|| ext.config.get("pwm_pin"))
                        .or_else(|| ext.config.get("pin"))
                        .and_then(|v| v.as_str())
                        .unwrap_or("GPIO18");
                    let pin = Self::parse_esp32s3_gpio_pin(pin_label)
                        .or_else(|| Self::parse_esp32_gpio_pin(pin_label))
                        .ok_or_else(|| {
                            anyhow::anyhow!(
                                "servo '{}' control/signal pin '{}' is not a parseable GPIO",
                                ext.id,
                                pin_label
                            )
                        })?;
                    // Prefer explicit config.model; fall back to the type alias
                    // (sg90/mg996r as top-level type) then standard cal.
                    let model = ext
                        .config
                        .get("model")
                        .and_then(|v| v.as_str())
                        .unwrap_or(ext.r#type.as_str());
                    let cal = match model {
                        "sg90" => crate::peripherals::components::servo::ServoCal::sg90(),
                        "mg996r" => crate::peripherals::components::servo::ServoCal::mg996r(),
                        _ => crate::peripherals::components::servo::ServoCal::standard(),
                    };
                    let servo =
                        std::sync::Arc::new(crate::peripherals::components::servo::Servo::with_id(
                            ext.id.clone(),
                            cal,
                            pin,
                        ));
                    Self::install_gpio_observer(&mut bus, servo.clone());
                    // LEDC duty path (classic ESP32). Optional `ledc_channel`
                    // binds one channel; otherwise fan out to all channels
                    // (fine for single-servo boards).
                    let ledc_channel = ext.config.get("ledc_channel").and_then(|v| v.as_u64());
                    for name in ["ledc", "LEDC"] {
                        if let Some(idx) = bus.find_peripheral_index_by_name(name) {
                            if let Some(ledc) =
                                bus.peripherals[idx].dev.as_any_mut().and_then(|a| {
                                    a.downcast_mut::<crate::peripherals::esp32::ledc::Ledc>()
                                })
                            {
                                if let Some(ch) = ledc_channel {
                                    ledc.add_duty_observer(std::sync::Arc::new(
                                        crate::peripherals::components::servo::LedcServoDriver::new(
                                            ch,
                                            servo.clone(),
                                        ),
                                    ));
                                } else {
                                    for ch in 0..16u64 {
                                        ledc.add_duty_observer(std::sync::Arc::new(
                                            crate::peripherals::components::servo::LedcServoDriver::new(
                                                ch,
                                                servo.clone(),
                                            ),
                                        ));
                                    }
                                }
                            }
                        }
                    }
                    bus.servos.push(servo);
                }
                "a4988" | "drv8825" | "tmc2209" => {
                    let step = Self::gpio_from_config(ext, "step_pin", "STEP", "GPIO16")?;
                    let dir = Self::gpio_from_config(ext, "dir_pin", "DIR", "GPIO17")?;
                    let en = Self::optional_gpio_from_config(ext, "en_pin", "EN");
                    let mut motor =
                        crate::peripherals::components::step_dir_motor::StepDirMotor::new(
                            &ext.id, step, dir, en,
                        );
                    if ext.r#type == "tmc2209" {
                        // SilentStepStick often 1/16 microstep default → treat as 1.8/16
                        motor = motor.with_config(
                            crate::peripherals::components::step_dir_motor::StepDirConfig {
                                degrees_per_step: 1.8 / 16.0,
                                enable_active_low: true,
                            },
                        );
                    }
                    let motor = std::sync::Arc::new(motor);
                    Self::install_gpio_observer(&mut bus, motor.clone());
                    bus.step_dir_motors.push(motor);
                }
                "l298n" | "tb6612" | "l293d" => {
                    // First motor channel (A)
                    let in1 = Self::gpio_from_config(ext, "in1_pin", "IN1", "GPIO16")
                        .or_else(|_| Self::gpio_from_config(ext, "ain1_pin", "AIN1", "GPIO16"))?;
                    let in2 = Self::gpio_from_config(ext, "in2_pin", "IN2", "GPIO17")
                        .or_else(|_| Self::gpio_from_config(ext, "ain2_pin", "AIN2", "GPIO17"))?;
                    let en = Self::optional_gpio_from_config(ext, "en_pin", "ENA")
                        .or_else(|| Self::optional_gpio_from_config(ext, "pwma_pin", "PWMA"));
                    let motor = std::sync::Arc::new(
                        crate::peripherals::components::h_bridge_motor::HBridgeMotor::new(
                            format!("{}-a", ext.id),
                            in1,
                            in2,
                            en,
                        ),
                    );
                    Self::install_gpio_observer(&mut bus, motor.clone());
                    bus.h_bridge_motors.push(motor);
                    // Optional B channel when IN3/IN4 (or BIN*) are configured.
                    let has_b = ext.config.contains_key("in3_pin")
                        || ext.config.contains_key("IN3")
                        || ext.config.contains_key("bin1_pin")
                        || ext.config.contains_key("BIN1");
                    if has_b {
                        if let (Ok(b1), Ok(b2)) = (
                            Self::gpio_from_config(ext, "in3_pin", "IN3", "GPIO18").or_else(|_| {
                                Self::gpio_from_config(ext, "bin1_pin", "BIN1", "GPIO18")
                            }),
                            Self::gpio_from_config(ext, "in4_pin", "IN4", "GPIO19").or_else(|_| {
                                Self::gpio_from_config(ext, "bin2_pin", "BIN2", "GPIO19")
                            }),
                        ) {
                            let enb = Self::optional_gpio_from_config(ext, "enb_pin", "ENB")
                                .or_else(|| {
                                    Self::optional_gpio_from_config(ext, "pwmb_pin", "PWMB")
                                });
                            let motor_b = std::sync::Arc::new(
                                crate::peripherals::components::h_bridge_motor::HBridgeMotor::new(
                                    format!("{}-b", ext.id),
                                    b1,
                                    b2,
                                    enb,
                                ),
                            );
                            Self::install_gpio_observer(&mut bus, motor_b.clone());
                            bus.h_bridge_motors.push(motor_b);
                        }
                    }
                }
                "uln2003" | "stepper-28byj48" => {
                    let p1 = Self::gpio_from_config(ext, "in1_pin", "IN1", "GPIO16")?;
                    let p2 = Self::gpio_from_config(ext, "in2_pin", "IN2", "GPIO17")?;
                    let p3 = Self::gpio_from_config(ext, "in3_pin", "IN3", "GPIO18")?;
                    let p4 = Self::gpio_from_config(ext, "in4_pin", "IN4", "GPIO19")?;
                    let motor = std::sync::Arc::new(
                        crate::peripherals::components::unipolar_stepper::UnipolarStepper::new_28byj48(
                            &ext.id,
                            [p1, p2, p3, p4],
                        ),
                    );
                    Self::install_gpio_observer(&mut bus, motor.clone());
                    bus.unipolar_steppers.push(motor);
                }
                "can-diagnostic-tester" | "uds-diagnostic-tester" => {
                    if bus.find_peripheral_index_by_name(&ext.connection).is_none() {
                        return Err(anyhow::anyhow!(
                            "CAN diagnostic tester '{}' connection '{}' was not found",
                            ext.id,
                            ext.connection
                        ));
                    }
                    let request_id = Self::yaml_u32(ext.config.get("request_id"), 0x7E0);
                    let request_data =
                        Self::yaml_bytes(ext.config.get("request_data"), &[0x03, 0x22, 0xF1, 0x90]);
                    bus.can_diagnostic_testers.push(CanDiagnosticTester {
                        id: ext.id.clone(),
                        connection: ext.connection.clone(),
                        request_id,
                        request_data,
                        sent: false,
                    });
                }
                "uds-tester" => {
                    // Stateful ISO-TP / UDS tester: a real second CAN node that
                    // drives a multi-frame SecurityAccess handshake against the
                    // named CAN peripheral (bxCAN or FDCAN) in normal mode.
                    if bus.find_peripheral_index_by_name(&ext.connection).is_none() {
                        return Err(anyhow::anyhow!(
                            "UDS tester '{}' connection '{}' was not found",
                            ext.id,
                            ext.connection
                        ));
                    }
                    let mut tester = CanUdsTester::new(ext.id.clone(), ext.connection.clone());
                    tester.request_id = Self::yaml_u32(
                        ext.config.get("request_id"),
                        CanUdsTester::DEFAULT_REQUEST_ID,
                    );
                    tester.reply_id =
                        Self::yaml_u32(ext.config.get("reply_id"), CanUdsTester::DEFAULT_REPLY_ID);
                    tester.first_frame = Self::yaml_bytes(
                        ext.config.get("first_frame"),
                        &CanUdsTester::DEFAULT_FIRST_FRAME,
                    );
                    tester.consecutive_frame = Self::yaml_bytes(
                        ext.config.get("consecutive_frame"),
                        &CanUdsTester::DEFAULT_CONSECUTIVE_FRAME,
                    );
                    tester.script = Self::parse_script(ext.config.get("script"));
                    // When no `script:` key is present, synthesize a single step
                    // from the legacy first_frame / consecutive_frame fields.
                    if !ext.config.contains_key("script") {
                        let ff = &tester.first_frame;
                        // FF: byte0 high nibble == 1; 12-bit length in (byte0 & 0x0F) << 8 | byte1
                        let pdu_len = if ff.len() >= 2 {
                            (((ff[0] & 0x0F) as usize) << 8) | (ff[1] as usize)
                        } else {
                            0
                        };
                        if ext.config.contains_key("first_frame") && (ff.len() < 2 || pdu_len == 0)
                        {
                            tracing::warn!(
                                "[uds-tester] '{}': first_frame is too short or decodes pdu_len=0 \
                                 — synthesized send will be empty",
                                ext.id
                            );
                        }
                        let ff_payload: &[u8] = if ff.len() >= 2 { &ff[2..] } else { &[] };
                        let cf_payload: &[u8] = if !tester.consecutive_frame.is_empty() {
                            &tester.consecutive_frame[1..]
                        } else {
                            &[]
                        };
                        let raw: Vec<u8> = ff_payload
                            .iter()
                            .chain(cf_payload.iter())
                            .copied()
                            .take(pdu_len)
                            .collect();
                        if raw.is_empty() && ext.config.contains_key("first_frame") {
                            tracing::warn!(
                                "[uds-tester] '{}': reassembled send payload is empty \
                                 — check first_frame / consecutive_frame config",
                                ext.id
                            );
                        }
                        tester.script = vec![UdsStep {
                            send: raw,
                            expect: vec![Some(0x06), Some(0x67)],
                            expect_nrc: None,
                        }];
                    }
                    bus.can_uds_testers.push(tester);
                }
                "can-player" => {
                    if bus.find_peripheral_index_by_name(&ext.connection).is_none() {
                        return Err(anyhow::anyhow!(
                            "can-player '{}' connection '{}' was not found",
                            ext.id,
                            ext.connection
                        ));
                    }
                    let Some(data) = ext.config.get("data").and_then(|v| v.as_str()) else {
                        return Err(anyhow::anyhow!(
                            "can-player '{}': set 'path' (a candump .log file) or inline 'data'",
                            ext.id
                        ));
                    };
                    let tps = Self::yaml_u32(ext.config.get("ticks_per_second"), 1_000_000) as u64;
                    let player = CanLogPlayer::from_candump(
                        ext.id.clone(),
                        ext.connection.clone(),
                        data,
                        tps,
                    )
                    .map_err(|e| anyhow::anyhow!(e))?;
                    bus.can_log_players.push(player);
                }
                // ntc-thermistor dispatches through the PeripheralKit registry above.
                _ => {
                    tracing::warn!(
                        "Unsupported external device '{}' type '{}' on connection '{}'; skipping",
                        ext.id,
                        ext.r#type,
                        ext.connection
                    );
                    continue;
                }
            }
        }

        bus.rebuild_peripheral_ranges();
        // ESP32-C3: share IO_MUX pad controls with GPIO so an Arduino
        // `INPUT_PULLUP` changes the floating input level. No-op for every
        // other chip.
        bus.wire_esp32c3_pad_controls();
        // ESP32-C3: share the I²C0 bit engine's live SDA/SCL line levels with
        // the C3 GPIO model so matrix-routed pads carry the real waveform.
        // No-op for every other chip.
        bus.wire_esp32c3_i2c_pads();
        // STM32: share each classic/FIFO SPI bit engine's live SCK/MOSI/MISO
        // line levels with the STM32 GPIO ports so AF-routed pads carry the
        // real waveform. No-op for every other chip.
        bus.wire_stm32_spi_pads();
        // Resolve declared per-peripheral RCC clock-gates now that every
        // peripheral (incl. the RCC, needed to map reg-name → offset) is on the
        // bus. Peripherals without a `clock:` field stay ungated.
        bus.resolve_clock_gates(&merged_peripherals)?;
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
        Ok(bus)
    }

    /// Install a GPIO edge observer on ESP32 / ESP32-S3 GPIO models when present.
    fn install_gpio_observer<T>(bus: &mut SystemBus, observer: std::sync::Arc<T>)
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
