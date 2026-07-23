use labwired_config::{
    Arch, BoardIoBinding, BoardIoKind, BoardIoSignal, ChipDescriptor, SystemManifest,
};
use labwired_core::bus::SystemBus;

// #124 Phase 4: browser-side JIT prototype. Runs the dominant
// `0x400829cc` hot block through `js_sys::WebAssembly` instead of the
// interpreter when `jit_enabled()` has been toggled on from JS.
mod inputs;
mod inspect;
mod install;
mod jit_browser;
mod traces;
// CortexM and XtensaLx7 are used via Box<dyn Cpu>; the concrete types are
// only constructed inside the configure_* fns and immediately boxed.
use labwired_core::decoder::arm::{decode_thumb_16, decode_thumb_32};
use labwired_core::decoder::riscv::{decode_rv32, decode_rv32c};
use labwired_core::decoder::xtensa;
use labwired_core::decoder::xtensa_length;
use labwired_core::decoder::xtensa_narrow;
use labwired_core::memory::{LinearMemory, ProgramImage};
use labwired_core::peripherals::adc::Adc;
use labwired_core::system::cortex_m::configure_cortex_m;
use labwired_core::system::xtensa::configure_xtensa_esp32;
use labwired_core::Arch as CoreArch;
use labwired_core::Bus;
use labwired_core::{AdvanceRequest, Cpu, Machine};
use labwired_loader::load_elf_bytes;
use wasm_bindgen::prelude::*;

// GDB-over-WASM scaffolding (`WasmGdbConn`, `WasmGdbEventLoop`, etc.) was
// removed when `WasmSimulator` switched to `Machine<Box<dyn Cpu>>` — the
// `gdbstub::target::Target` impl in `labwired-gdbstub` is concrete per arch.
// Restore once a dyn-aware Target wrapper exists.
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

/// Per-instance state for the ESP32-classic cross-core IPI bridge that lets
/// dual-core ESP-IDF firmware run on our single-CPU sim. Real silicon routes
/// FROM_CPU_INTR0/1 through DPORT's intmatrix to a CPU internal interrupt
/// bit; we sample the mapping each step and synthesise the edge on PRO_CPU.
#[derive(Default)]
struct Esp32IpiBridge {
    from_cpu_bit0: Option<u8>,
    from_cpu_bit1: Option<u8>,
    last_from_cpu0_val: u32,
    last_from_cpu1_val: u32,
    /// Per-firmware dual-core handshake byte addresses, resolved from the
    /// firmware ELF's symbol table by `install_arduino_esp32_quirks`. The
    /// keep-alive in `step_with_esp32_aids` re-writes 0x01 to each of these
    /// every 10 000 cycles so the firmware's `.bss` zero-init can't wipe
    /// them between the install and the spin-wait check. Empty when the
    /// hardcoded reference-firmware keep-alive is in use (the old
    /// `install_esp32_arduino_quirks` path).
    handshake_bytes: Vec<u32>,
}

#[wasm_bindgen]
pub struct WasmSimulator {
    machine: Option<Machine<Box<dyn Cpu>>>,
    board_io: Vec<BoardIoBinding>,
    uart_sink: Arc<Mutex<Vec<u8>>>,
    uart_rx_bufs: Vec<Arc<Mutex<VecDeque<u8>>>>,
    #[allow(dead_code)]
    arch: Arch,
    /// Set by `install_esp32_arduino_quirks` / `enable_esp32_dual_core_emulation`.
    /// When `Some`, `step_with_esp32_aids` runs the IPI bridge + dual-core
    /// handshake keep-alives each cycle.
    esp32_ipi: Option<Esp32IpiBridge>,
    /// #124 Phase 4: browser-side JIT cache. Off by default — flip via
    /// `set_jit_enabled(true)` from JS. We deliberately don't auto-enable
    /// until benchmarks confirm a net win, so production playground
    /// behaviour is unchanged unless the operator opts in.
    jit_browser_enabled: bool,
    /// Lazy-init at first JIT-able step. Boxed so the typical "JIT off"
    /// path pays no per-instance allocation.
    jit_browser_cache: Option<Box<jit_browser::BrowserJitCache>>,
}

/// Public shape returned by `step_batch_profile`.
///
/// The six execution counters intentionally mirror `StepProfile` exactly.
/// `executed_cycles` is the batch boundary observable; workload-specific
/// markers such as ESP32-S3 OLED first-paint and completion remain outside
/// this generic API and are measured by the workload harness in the same
/// simulation pass.
#[derive(serde::Serialize)]
struct WasmStepBatchProfile {
    requested_cycles: u32,
    executed_cycles: u32,
    wall_ms: f64,
    cycles_per_second: f64,
    cpu_instructions: u64,
    cpu_batches: u64,
    peripheral_ticks: u64,
    peripheral_ticked_entries: u64,
    bus_tick_entries: u64,
    legacy_tick_entries: u64,
}

#[cfg(test)]
const ESP32C3_APP_IMAGE_OFFSET: usize = 0x1_0000;
const ESP_IMAGE_HEADER_LEN: usize = 24;
const ESP_IMAGE_MAGIC: u8 = 0xE9;
const ESP32C3_FLASH_FAST_START_BLOB: &str = "labwired_esp32c3_flash_fast_start";

fn esp32c3_program_image_from_flash_offset(
    flash: &[u8],
    offset: usize,
    label: &str,
) -> Result<ProgramImage, String> {
    let image = flash.get(offset..).ok_or_else(|| {
        format!("ESP32-C3 flash image is smaller than {label} offset {offset:#x}")
    })?;
    if image.len() < ESP_IMAGE_HEADER_LEN {
        return Err(format!("ESP32-C3 {label} image header is truncated"));
    }
    if image[0] != ESP_IMAGE_MAGIC {
        return Err(format!(
            "ESP32-C3 {label} image has bad magic 0x{:02x} at flash offset {offset:#x}",
            image[0],
        ));
    }

    let segment_count = image[1] as usize;
    let entry = u32::from_le_bytes(image[4..8].try_into().unwrap()) as u64;
    let mut program = ProgramImage::new(entry, CoreArch::RiscV);
    let mut cursor = ESP_IMAGE_HEADER_LEN;

    for index in 0..segment_count {
        let header = image
            .get(cursor..cursor + 8)
            .ok_or_else(|| format!("ESP32-C3 {label} segment {index} header is truncated"))?;
        let load_addr = u32::from_le_bytes(header[0..4].try_into().unwrap()) as u64;
        let len = u32::from_le_bytes(header[4..8].try_into().unwrap()) as usize;
        cursor += 8;
        let data = image
            .get(cursor..cursor + len)
            .ok_or_else(|| format!("ESP32-C3 {label} segment {index} data is truncated"))?;
        program.add_segment(load_addr, data.to_vec());
        cursor += len;
    }

    if program.segments.is_empty() {
        return Err(format!("ESP32-C3 {label} image has no loadable segments"));
    }

    Ok(program)
}

#[cfg(test)]
fn esp32c3_app_program_image_from_merged_flash(flash: &[u8]) -> Result<ProgramImage, String> {
    esp32c3_program_image_from_flash_offset(flash, ESP32C3_APP_IMAGE_OFFSET, "app")
}

fn esp32c3_bootloader_program_image_from_merged_flash(
    flash: &[u8],
) -> Result<ProgramImage, String> {
    esp32c3_program_image_from_flash_offset(flash, 0, "bootloader")
}

fn load_program_segments_without_reset(
    machine: &mut labwired_core::Machine<Box<dyn Cpu>>,
    program_image: &ProgramImage,
) -> Result<(), String> {
    for segment in &program_image.segments {
        if machine.bus.flash.load_from_segment(segment)
            || machine.bus.ram.load_from_segment(segment)
            || machine
                .bus
                .extra_mem
                .iter_mut()
                .any(|m| m.load_from_segment(segment))
        {
            continue;
        }

        for (i, byte) in segment.data.iter().enumerate() {
            let addr = segment.start_addr + i as u64;
            machine
                .bus
                .write_u8(addr, *byte)
                .map_err(|e| format!("load segment at {addr:#x}: {e}"))?;
        }
    }

    Ok(())
}

#[wasm_bindgen]
impl WasmSimulator {
    /// Legacy constructor: hardcoded STM32F107 Cortex-M3 with 128KB flash + 20KB RAM.
    /// Kept for backward compatibility with the existing landing page sandbox.
    #[wasm_bindgen(constructor)]
    pub fn new(firmware: &[u8]) -> Result<WasmSimulator, JsValue> {
        let mut bus = SystemBus::new();
        bus.flash = LinearMemory::new(128 * 1024, 0x0800_0000);
        bus.ram = LinearMemory::new(20 * 1024, 0x2000_0000);
        bus.refresh_peripheral_index();

        let uart_sink = Arc::new(Mutex::new(Vec::new()));
        bus.attach_uart_tx_sink(uart_sink.clone(), false);
        let uart_rx_bufs = bus.attach_uart_rx_source();

        let (cpu, _nvic) = configure_cortex_m(&mut bus);
        let boxed: Box<dyn Cpu> = Box::new(cpu);
        let mut machine = Machine::new(boxed, bus);

        let program_image = load_elf_bytes(firmware)
            .map_err(|e| JsValue::from_str(&format!("Loader Error: {}", e)))?;
        machine
            .load_firmware(&program_image)
            .map_err(|e| JsValue::from_str(&format!("Simulation Error: {}", e)))?;

        Ok(WasmSimulator {
            machine: Some(machine),
            board_io: Vec::new(),
            uart_sink,
            uart_rx_bufs,
            arch: Arch::Arm,
            esp32_ipi: None,
            jit_browser_enabled: false,
            jit_browser_cache: None,
        })
    }

    /// Config-driven constructor: initialize from system YAML, chip YAML, and firmware ELF.
    ///
    /// Dispatches on `chip.arch`:
    ///   * `Arm` → `SystemBus::from_config` + `configure_cortex_m` (existing path).
    ///   * `Xtensa` → `configure_xtensa_esp32` + inline external-device attach.
    ///     ESP32 chip YAMLs declare RAM banks (IRAM/DRAM/flash XIP/ROM) via
    ///     `peripherals: [{type: ram, ...}]`, which `from_config` doesn't
    ///     understand — it'd stub them out and break instruction fetch. So
    ///     ESP32 takes the dedicated path that explicitly registers those
    ///     banks before attaching SPI / I²C external devices.
    #[wasm_bindgen]
    pub fn new_from_config(
        system_yaml: &str,
        chip_yaml: &str,
        firmware: &[u8],
        blobs: JsValue,
    ) -> Result<WasmSimulator, JsValue> {
        let manifest: SystemManifest = serde_yaml::from_str(system_yaml)
            .map_err(|e| JsValue::from_str(&format!("System YAML error: {}", e)))?;
        let chip: ChipDescriptor = serde_yaml::from_str(chip_yaml)
            .map_err(|e| JsValue::from_str(&format!("Chip YAML error: {}", e)))?;

        match chip.arch {
            Arch::Arm | Arch::Unknown => Self::new_from_config_arm(&chip, &manifest, firmware),
            Arch::RiscV => {
                let blob_map = parse_named_blobs(&blobs);
                // A board opts into faithful ROM boot by supplying the merged
                // flash image (`bootloader@0x0 + partition-table@0x8000 +
                // app@0x10000`) as the `esp32c3_flash` blob — the same on-demand
                // named-blob idiom the ROM images already use. Its presence is
                // the trigger (no schema flag needed): with it, the browser boots
                // the real mask ROM from the reset vector exactly like the native
                // `--rom-boot` CLI; without it, the pre-existing fast-boot path
                // runs, treating `firmware` as a bare esp-hal ELF.
                if blob_map.contains_key("esp32c3_flash")
                    && blob_map.contains_key(ESP32C3_FLASH_FAST_START_BLOB)
                {
                    Self::new_from_config_riscv_flash_fastboot(&chip, &manifest, &blob_map)
                } else if blob_map.contains_key("esp32c3_flash") {
                    Self::new_from_config_riscv_romboot(&chip, &manifest, &blob_map)
                } else {
                    Self::new_from_config_riscv(&chip, &manifest, firmware, &blob_map)
                }
            }
            Arch::Xtensa if chip.name.starts_with("esp32s3") => {
                let blob_map = parse_named_blobs(&blobs);
                Self::new_from_config_xtensa_esp32s3(&manifest, firmware, &blob_map)
            }
            Arch::Xtensa => Self::new_from_config_xtensa_esp32(&manifest, firmware),
        }
    }

    fn new_from_config_arm(
        chip: &ChipDescriptor,
        manifest: &SystemManifest,
        firmware: &[u8],
    ) -> Result<WasmSimulator, JsValue> {
        let mut bus = SystemBus::from_config(chip, manifest)
            .map_err(|e| JsValue::from_str(&format!("Bus config error: {:#}", e)))?;

        let uart_sink = Arc::new(Mutex::new(Vec::new()));
        if let Some(debug_uart) = manifest.debug_uart.as_deref() {
            if !bus.attach_uart_tx_sink_named(debug_uart, uart_sink.clone(), false) {
                bus.attach_uart_tx_sink(uart_sink.clone(), false);
            }
        } else {
            bus.attach_uart_tx_sink(uart_sink.clone(), false);
        }
        let uart_rx_bufs = bus.attach_uart_rx_source();

        let (cpu, _nvic) = configure_cortex_m(&mut bus);
        let boxed: Box<dyn Cpu> = Box::new(cpu);
        let mut machine = Machine::new(boxed, bus);

        let program_image = load_elf_bytes(firmware)
            .map_err(|e| JsValue::from_str(&format!("Loader Error: {}", e)))?;
        machine
            .load_firmware(&program_image)
            .map_err(|e| JsValue::from_str(&format!("Simulation Error: {}", e)))?;

        let board_io = manifest.board_io.clone();

        Ok(WasmSimulator {
            machine: Some(machine),
            board_io,
            uart_sink,
            uart_rx_bufs,
            arch: Arch::Arm,
            esp32_ipi: None,
            jit_browser_enabled: false,
            jit_browser_cache: None,
        })
    }

    /// RISC-V (esp32c3) bus setup. Mirrors `new_from_config_arm` but builds a
    /// RISC-V core via `configure_riscv` and seeds the stack pointer at the top
    /// of DRAM — fast-boot skips the ROM/2nd-stage bootloader that would
    /// normally set SP, so the app's first prologue store would otherwise fault.
    ///
    /// The ESP32-C3 boot ROM is injected on demand via `blobs` under
    /// `esp32c3_irom`/`esp32c3_drom` — the RISC-V analogue of the S3 path's
    /// `Esp32s3Opts.rom_images`. The chip YAML declares zero-filled `rom` /
    /// `rom_data` regions (IROM 0x4000_0000, DROM 0x3FF0_0000) that native
    /// builds fill from env pins or the vendored images; on wasm the vendored
    /// images are excluded from the bundle, so the browser fetches the two ROM
    /// bins and passes them here. With the ROM present, esp-hal's ROM function
    /// calls during init resolve for real (zero thunks) instead of dispatching
    /// through zeros.
    fn new_from_config_riscv(
        chip: &ChipDescriptor,
        manifest: &SystemManifest,
        firmware: &[u8],
        blobs: &std::collections::HashMap<String, Vec<u8>>,
    ) -> Result<WasmSimulator, JsValue> {
        let program_image = load_elf_bytes(firmware)
            .map_err(|e| JsValue::from_str(&format!("Loader Error: {}", e)))?;
        Self::new_from_config_riscv_program_image(chip, manifest, &program_image, blobs)
    }

    fn new_from_config_riscv_flash_fastboot(
        chip: &ChipDescriptor,
        manifest: &SystemManifest,
        blobs: &std::collections::HashMap<String, Vec<u8>>,
    ) -> Result<WasmSimulator, JsValue> {
        use labwired_core::boot::esp32c3_rom::{
            build_rom_boot_machine, c3_rom_data_init_writes, inject_rom_regions, RomBootOpts,
        };
        use labwired_core::boot::esp32s3_rom::RomImages;

        let mut bus = SystemBus::from_config(chip, manifest)
            .map_err(|e| JsValue::from_str(&format!("Bus config error: {:#}", e)))?;

        let (Some(irom), Some(drom)) = (blobs.get("esp32c3_irom"), blobs.get("esp32c3_drom"))
        else {
            return Err(JsValue::from_str(
                "C3 flash fast-start needs ESP32-C3 ROM blobs: pass esp32c3_irom + esp32c3_drom",
            ));
        };
        let images = RomImages {
            irom: irom.clone(),
            drom: drom.clone(),
        };
        if !inject_rom_regions(&mut bus, &images) {
            return Err(JsValue::from_str(
                "C3 flash fast-start: chip YAML declares no IROM region at 0x40000000",
            ));
        }
        // The bootloader calls ROM helpers through DRAM tables initialized by
        // the mask ROM reset code. Because this path skips that reset code, copy
        // those ROM `.data` records before entering the second-stage bootloader.
        for (dst, bytes) in c3_rom_data_init_writes(irom) {
            for (i, b) in bytes.iter().enumerate() {
                let _ = bus.write_u8(dst as u64 + i as u64, *b);
            }
        }

        let flash = blobs
            .get("esp32c3_flash")
            .ok_or_else(|| JsValue::from_str("fast-start needs esp32c3_flash"))?;
        let bootloader_image = esp32c3_bootloader_program_image_from_merged_flash(flash)
            .map_err(|e| JsValue::from_str(&format!("ESP32-C3 flash fast-start: {e}")))?;

        let uart_sink = Arc::new(Mutex::new(Vec::new()));
        let capture_usb_serial = manifest
            .debug_uart
            .as_deref()
            .map(|debug_uart| {
                debug_uart.eq_ignore_ascii_case("usb_serial_jtag")
                    || debug_uart.eq_ignore_ascii_case("usb-serial-jtag")
            })
            .unwrap_or(false);
        if !capture_usb_serial {
            if let Some(debug_uart) = manifest.debug_uart.as_deref() {
                if !bus.attach_uart_tx_sink_named(debug_uart, uart_sink.clone(), false) {
                    bus.attach_uart_tx_sink(uart_sink.clone(), false);
                }
            } else {
                bus.attach_uart_tx_sink(uart_sink.clone(), false);
            }
        }
        let uart_rx_bufs = bus.attach_uart_rx_source();

        let mut machine = build_rom_boot_machine(
            bus,
            flash.clone(),
            RomBootOpts {
                efuse_mac: None,
                usb_serial_sink: capture_usb_serial.then(|| uart_sink.clone()),
            },
            |c| Box::new(c) as Box<dyn Cpu>,
        );
        load_program_segments_without_reset(&mut machine, &bootloader_image)
            .map_err(|e| JsValue::from_str(&format!("C3 flash fast-start load: {e}")))?;

        let sp_top =
            (chip.ram.base + labwired_config::parse_size(&chip.ram.size).unwrap_or(0)) as u32;
        machine.cpu.set_sp(sp_top & !0xF);
        machine.cpu.set_pc(bootloader_image.entry_point as u32);

        let board_io = manifest.board_io.clone();

        Ok(WasmSimulator {
            machine: Some(machine),
            board_io,
            uart_sink,
            uart_rx_bufs,
            arch: Arch::RiscV,
            esp32_ipi: None,
            jit_browser_enabled: false,
            jit_browser_cache: None,
        })
    }

    fn new_from_config_riscv_program_image(
        chip: &ChipDescriptor,
        manifest: &SystemManifest,
        program_image: &ProgramImage,
        blobs: &std::collections::HashMap<String, Vec<u8>>,
    ) -> Result<WasmSimulator, JsValue> {
        let mut bus = SystemBus::from_config(chip, manifest)
            .map_err(|e| JsValue::from_str(&format!("Bus config error: {:#}", e)))?;

        // Inject the on-demand ESP32-C3 boot ROM blobs into the chip's still
        // zero-filled `rom`/`rom_data` regions, matching how the native
        // `--rom-boot` path (`build_c3_rom_boot_machine`) provisions them.
        // Absent blobs (non-C3 RISC-V chips, or the browser not supplying them)
        // leave the regions zero, preserving the pre-existing fast-boot path.
        let faithful_c3_rom = {
            use labwired_core::boot::esp32c3_rom::{c3_rom_data_init_writes, DROM_BASE, IROM_BASE};
            let mut injected_irom: Option<Vec<u8>> = None;
            for mem in bus.extra_mem.iter_mut() {
                let src = if mem.base_addr == IROM_BASE as u64 {
                    blobs.get("esp32c3_irom")
                } else if mem.base_addr == DROM_BASE as u64 {
                    blobs.get("esp32c3_drom")
                } else {
                    None
                };
                if let Some(src) = src {
                    let n = src.len().min(mem.data.len());
                    mem.data[..n].copy_from_slice(&src[..n]);
                    if mem.base_addr == IROM_BASE as u64 {
                        injected_irom = Some(src.clone());
                    }
                }
            }
            // Fast-boot skips the ROM reset's own `.data` copy, so replicate it:
            // land the ROM's DRAM globals (ROM function tables esp-hal calls
            // dispatch through) exactly as silicon does — otherwise those calls
            // jump through a null/garbage pointer. Mirrors the S3 path's
            // `s3_rom_data_init_writes` in `configure_xtensa_esp32s3`.
            if let Some(irom) = injected_irom {
                for (dst, bytes) in c3_rom_data_init_writes(&irom) {
                    for (i, b) in bytes.iter().enumerate() {
                        let _ = bus.write_u8(dst as u64 + i as u64, *b);
                    }
                }
                // With the real ROM present, esp-hal's clock bring-up runs the
                // genuine `rom_i2c_*Reg` helpers, which drive the analog I²C
                // master / ANA_CONFIG block (0x6000_E000) for the PLL. That
                // block is not in the chip YAML (it's the same custom model the
                // native `--rom-boot` builder wires), so add it here on the
                // faithful path — otherwise the first ROM PLL transaction faults
                // on an unmapped access. Its FSM-status model lets the ROM's
                // transaction busy-poll complete.
                bus.add_peripheral(
                    "rtc_i2c_ana",
                    0x6000_E000,
                    0x400,
                    None,
                    Box::new(labwired_core::peripherals::esp32c3::ana_i2c::Esp32c3AnaI2c::new()),
                );
                bus.refresh_peripheral_index();
                true
            } else {
                false
            }
        };

        let uart_sink = Arc::new(Mutex::new(Vec::new()));
        // On the faithful C3 ROM path, esp-println's `jtag-serial` feature (used
        // by esp-hal apps) prints through USB_SERIAL_JTAG (0x6004_3000), not
        // UART0. The chip YAML only has a declarative register stub there, which
        // never drains bytes, so route the real behavioral model (same IP as the
        // S3, reused unchanged) into `uart_sink` — mirroring the S3 path — so the
        // widget's Serial tab shows the app's output. A narrower, later-registered
        // window overrides the declarative stub.
        if faithful_c3_rom {
            use labwired_core::peripherals::esp32s3::usb_serial_jtag::UsbSerialJtag;
            let mut usb_serial = UsbSerialJtag::new();
            usb_serial.set_sink(Some(uart_sink.clone()), false);
            bus.add_peripheral(
                "usb_serial_jtag",
                0x6004_3000,
                0x100,
                None,
                Box::new(usb_serial),
            );
            bus.refresh_peripheral_index();
        }
        if let Some(debug_uart) = manifest.debug_uart.as_deref() {
            if !bus.attach_uart_tx_sink_named(debug_uart, uart_sink.clone(), false) {
                bus.attach_uart_tx_sink(uart_sink.clone(), false);
            }
        } else {
            bus.attach_uart_tx_sink(uart_sink.clone(), false);
        }
        let uart_rx_bufs = bus.attach_uart_rx_source();

        let cpu = labwired_core::system::riscv::configure_riscv(&mut bus);
        let boxed: Box<dyn Cpu> = Box::new(cpu);
        let mut machine = Machine::new(boxed, bus);

        machine
            .load_firmware(program_image)
            .map_err(|e| JsValue::from_str(&format!("Simulation Error: {}", e)))?;

        let sp_top =
            (chip.ram.base + labwired_config::parse_size(&chip.ram.size).unwrap_or(0)) as u32;
        machine.cpu.set_sp(sp_top & !0xF);
        machine.cpu.set_pc(program_image.entry_point as u32);

        let board_io = manifest.board_io.clone();

        Ok(WasmSimulator {
            machine: Some(machine),
            board_io,
            uart_sink,
            uart_rx_bufs,
            arch: Arch::RiscV,
            esp32_ipi: None,
            jit_browser_enabled: false,
            jit_browser_cache: None,
        })
    }

    /// RISC-V (ESP32-C3) FAITHFUL ROM-boot path — the browser analogue of the
    /// native CLI `--rom-boot`. Unlike fast-boot (which jumps straight to an ELF
    /// app entry), this resets to the BROM vector `0x4000_0000` and runs the
    /// genuine mask ROM → 2nd-stage bootloader → `app_main()`, loading from a
    /// merged flash image. Arduino/ESP-IDF images are flash images that run from
    /// flash via cache/XIP, so they REQUIRE this sequence.
    ///
    /// Blobs (all fetched on demand, none baked into the wasm bundle):
    ///   * `esp32c3_irom` / `esp32c3_drom` — the boot ROM images, injected into
    ///     the chip's zero-filled `rom`/`rom_data` regions.
    ///   * `esp32c3_flash` — the merged flash image; this is the actual program.
    ///
    /// All peripheral wiring + reset-vector boot is the shared core builder
    /// [`labwired_core::boot::esp32c3_rom::build_rom_boot_machine`], byte-for-byte
    /// the same machine the native CLI assembles. Zero thunks.
    fn new_from_config_riscv_romboot(
        chip: &ChipDescriptor,
        manifest: &SystemManifest,
        blobs: &std::collections::HashMap<String, Vec<u8>>,
    ) -> Result<WasmSimulator, JsValue> {
        use labwired_core::boot::esp32c3_rom::{
            build_rom_boot_machine, inject_rom_regions, RomBootOpts,
        };
        use labwired_core::boot::esp32s3_rom::RomImages;

        let mut bus = SystemBus::from_config(chip, manifest)
            .map_err(|e| JsValue::from_str(&format!("Bus config error: {:#}", e)))?;

        // Provision the boot ROM into the chip's zero-filled rom/rom_data
        // regions (the native path fills them from env pins / vendored images;
        // on wasm the browser fetches and passes the two bins). ROM-boot cannot
        // proceed without the real ROM — the reset vector executes it directly.
        let (Some(irom), Some(drom)) = (blobs.get("esp32c3_irom"), blobs.get("esp32c3_drom"))
        else {
            return Err(JsValue::from_str(
                "rom-boot needs the ESP32-C3 boot ROM: pass esp32c3_irom + esp32c3_drom blobs",
            ));
        };
        let images = RomImages {
            irom: irom.clone(),
            drom: drom.clone(),
        };
        if !inject_rom_regions(&mut bus, &images) {
            return Err(JsValue::from_str(
                "rom-boot: chip YAML declares no IROM region at 0x40000000 to load the boot ROM",
            ));
        }

        let flash_bytes = blobs
            .get("esp32c3_flash")
            .expect("esp32c3_flash presence checked by caller")
            .clone();

        // Capture one console for the widget's Serial tab. The C3 boot ROM
        // prints the same banner to UART0 and USB_SERIAL_JTAG; wiring both into
        // one browser buffer renders every ROM character twice. Default to
        // UART0 (Arduino/IDF Serial in hosted labs). A manifest can explicitly
        // request the USB console with debug_uart: usb_serial_jtag.
        let uart_sink = Arc::new(Mutex::new(Vec::new()));
        let capture_usb_serial = manifest
            .debug_uart
            .as_deref()
            .map(|debug_uart| {
                debug_uart.eq_ignore_ascii_case("usb_serial_jtag")
                    || debug_uart.eq_ignore_ascii_case("usb-serial-jtag")
            })
            .unwrap_or(false);
        if !capture_usb_serial {
            if let Some(debug_uart) = manifest.debug_uart.as_deref() {
                if !bus.attach_uart_tx_sink_named(debug_uart, uart_sink.clone(), false) {
                    bus.attach_uart_tx_sink(uart_sink.clone(), false);
                }
            } else {
                bus.attach_uart_tx_sink(uart_sink.clone(), false);
            }
        }
        let uart_rx_bufs = bus.attach_uart_rx_source();

        let machine = build_rom_boot_machine(
            bus,
            flash_bytes,
            RomBootOpts {
                efuse_mac: None,
                usb_serial_sink: capture_usb_serial.then(|| uart_sink.clone()),
            },
            // WasmSimulator holds Machine<Box<dyn Cpu>>; box the concrete RiscV.
            |c| Box::new(c) as Box<dyn Cpu>,
        );

        let board_io = manifest.board_io.clone();

        Ok(WasmSimulator {
            machine: Some(machine),
            board_io,
            uart_sink,
            uart_rx_bufs,
            arch: Arch::RiscV,
            esp32_ipi: None,
            jit_browser_enabled: false,
            jit_browser_cache: None,
        })
    }

    /// ESP32-classic (Xtensa LX6) bus setup. `configure_xtensa_esp32` adds
    /// IRAM / DRAM / flash XIP / ROM / UART0; external device attach
    /// (SSD1680 e-paper etc) is handled by the core helper since this code
    /// path doesn't go through `SystemBus::from_config`.
    fn new_from_config_xtensa_esp32(
        manifest: &SystemManifest,
        firmware: &[u8],
    ) -> Result<WasmSimulator, JsValue> {
        let mut bus = SystemBus::new();
        let cpu = configure_xtensa_esp32(&mut bus);

        let uart_sink = Arc::new(Mutex::new(Vec::new()));
        bus.attach_uart_tx_sink(uart_sink.clone(), false);
        let uart_rx_bufs = bus.attach_uart_rx_source();

        labwired_core::system::xtensa::attach_esp32_external_devices(&mut bus, manifest)
            .map_err(|e| JsValue::from_str(&format!("ESP32 external_devices: {:#}", e)))?;
        bus.refresh_peripheral_index();

        let boxed: Box<dyn Cpu> = Box::new(cpu);
        // Real dual-core: attach a second LX6 as APP_CPU (PRID 0xABAB → core 1,
        // starts halted until PRO_CPU releases it via ets_set_appcpu_boot_addr).
        // This replaces the old single-core handshake-forging stub: loopTask
        // (pinned to CONFIG_ARDUINO_RUNNING_CORE=1) now runs on a genuine
        // second core, and the cross-core yield IPI is delivered by the core's
        // DPORT through Machine::step — see crates/core/tests/e2e_labwired_ereader.rs.
        let app_cpu: Box<dyn Cpu> = Box::new(labwired_core::cpu::XtensaLx7::new_app_cpu());
        let mut machine = Machine::new(boxed, bus).with_secondary_cpu(app_cpu);

        let program_image = load_elf_bytes(firmware)
            .map_err(|e| JsValue::from_str(&format!("Loader Error: {}", e)))?;
        machine
            .load_firmware(&program_image)
            .map_err(|e| JsValue::from_str(&format!("Simulation Error: {}", e)))?;
        // XtensaLx7::reset() defaults PC to 0x40000400 (BROM reset vector).
        // We skip BROM emulation and jump straight to the ELF's app entry,
        // matching where a 2nd-stage bootloader would land.
        machine.cpu.set_pc(program_image.entry_point as u32);
        // BROM seeds SP near top of DRAM before call_start_cpu0; we skip BROM,
        // so seed both cores' stacks (APP_CPU in a separate DRAM region below
        // PRO_CPU's), matching the native dual-core bring-up.
        machine.cpu.set_sp(0x3FFE_0000);
        if let Some(cpu1) = machine.cpu_secondary.as_mut() {
            cpu1.set_sp(0x3FFD_8000);
        }

        let board_io = manifest.board_io.clone();

        Ok(WasmSimulator {
            machine: Some(machine),
            board_io,
            uart_sink,
            uart_rx_bufs,
            arch: Arch::Xtensa,
            esp32_ipi: None,
            jit_browser_enabled: false,
            jit_browser_cache: None,
        })
    }

    /// ESP32-S3 (Xtensa LX7) bus setup — the FAITHFUL fast-boot path.
    ///
    /// `configure_xtensa_esp32s3` installs IRAM/DRAM/RTC/flash-XIP plus the
    /// real boot ROM (zero thunks; the ROM `.data` init lands
    /// `rom_cache_internal_table_ptr` so esp-hal's ROM cache calls run for
    /// real). The ROM is NOT baked into the wasm bundle — it is fetched on
    /// demand and passed in `blobs` under `esp32s3_irom`/`esp32s3_drom`, then
    /// injected via `Esp32s3Opts.rom_images`. `fast_boot` then loads the app
    /// ELF's segments (identity XIP) and synthesises post-bootloader CPU state.
    /// Serial output on the S3 esp-hal apps goes through USB_SERIAL_JTAG, so we
    /// route that peripheral's sink into the `uart_sink` the widget reads.
    fn new_from_config_xtensa_esp32s3(
        manifest: &SystemManifest,
        firmware: &[u8],
        blobs: &std::collections::HashMap<String, Vec<u8>>,
    ) -> Result<WasmSimulator, JsValue> {
        use labwired_core::boot::esp32s3::{fast_boot, BootOpts};
        use labwired_core::boot::esp32s3_rom::RomImages;
        use labwired_core::peripherals::esp32s3::usb_serial_jtag::UsbSerialJtag;
        use labwired_core::system::xtensa::{configure_xtensa_esp32s3, Esp32s3Opts};

        // Inject the on-demand ROM blobs (None → configure falls back to the
        // native provision chain, which is None on wasm → thunk harness).
        let rom_images = match (blobs.get("esp32s3_irom"), blobs.get("esp32s3_drom")) {
            (Some(irom), Some(drom)) => Some(RomImages {
                irom: irom.clone(),
                drom: drom.clone(),
            }),
            _ => None,
        };

        let mut bus = SystemBus::new();
        // Default XIP model (fast-boot identity; --rom-boot's MMU model is
        // native-CLI only) + the injected faithful ROM.
        let opts = Esp32s3Opts {
            rom_images,
            ..Esp32s3Opts::default()
        };
        let wiring = configure_xtensa_esp32s3(&mut bus, &opts);
        let mut cpu = wiring.cpu;

        // Route USB-serial-JTAG bytes into the widget's serial sink. esp-hal's
        // `esp_println`/`println!` on the S3 targets USB_SERIAL_JTAG, not UART0.
        let uart_sink = Arc::new(Mutex::new(Vec::new()));
        for p in bus.peripherals.iter_mut() {
            if p.name == "usb_serial_jtag" {
                if let Some(any_mut) = p.dev.as_any_mut() {
                    if let Some(jtag) = any_mut.downcast_mut::<UsbSerialJtag>() {
                        jtag.set_sink(Some(uart_sink.clone()), false);
                    }
                }
            }
        }
        // Also capture UART0 in case a sketch uses the classic UART path.
        bus.attach_uart_tx_sink(uart_sink.clone(), false);
        let uart_rx_bufs = bus.attach_uart_rx_source();

        // Wire any devices the manifest declares (e.g. an SH1107 OLED on i2c0) —
        // the same factory the classic-ESP32 and native builder paths use. Without
        // this, an S3 board's `external_devices` were silently dropped and the
        // panel never rendered. Connect the blocks the manifest says are wired.
        labwired_core::system::xtensa::attach_esp32_external_devices(&mut bus, manifest)
            .map_err(|e| JsValue::from_str(&format!("ESP32-S3 external_devices: {:#}", e)))?;
        bus.refresh_peripheral_index();

        fast_boot(
            firmware,
            &mut bus,
            &mut cpu,
            &BootOpts {
                stack_top_fallback: 0x3FCD_FFF0,
                icache_backing: Some(wiring.icache_backing),
                dcache_backing: Some(wiring.dcache_backing),
                factory_flash_base: None,
            },
        )
        .map_err(|e| JsValue::from_str(&format!("ESP32-S3 fast_boot: {e}")))?;

        let boxed: Box<dyn Cpu> = Box::new(cpu);
        let machine = Machine::new(boxed, bus);

        Ok(WasmSimulator {
            machine: Some(machine),
            board_io: manifest.board_io.clone(),
            uart_sink,
            uart_rx_bufs,
            arch: Arch::Xtensa,
            esp32_ipi: None,
            jit_browser_enabled: false,
            jit_browser_cache: None,
        })
    }

    fn machine(&mut self) -> &mut Machine<Box<dyn Cpu>> {
        self.machine.as_mut().unwrap()
    }

    /// Read the output state of a board_io binding using peripheral snapshot.
    fn read_board_io_state(
        &self,
        machine: &Machine<Box<dyn Cpu>>,
        binding: &BoardIoBinding,
    ) -> bool {
        let idx = match machine
            .bus
            .find_peripheral_index_by_name(&binding.peripheral)
        {
            Some(i) => i,
            None => return false,
        };

        let pin_high = match binding.kind {
            BoardIoKind::Led | BoardIoKind::PwmOutput => machine.bus.peripherals[idx]
                .dev
                .read_gpio_output(binding.pin)
                .unwrap_or(false),
            BoardIoKind::Button => machine.bus.peripherals[idx]
                .dev
                .read_gpio_input(binding.pin)
                .unwrap_or(false),
            // Analog/bus kinds are not boolean and are exposed through typed state accessors.
            BoardIoKind::AdcInput
            | BoardIoKind::I2cDevice
            | BoardIoKind::SpiDevice
            | BoardIoKind::UartDevice => {
                return false;
            }
        };

        if binding.active_high {
            pin_high
        } else {
            !pin_high
        }
    }

    /// Browser-side GDB stub entry point.
    ///
    /// Disabled in this build: the GdbStub `Target` impl in `labwired-gdbstub`
    /// is concrete on `LabwiredTarget<CortexM>` / `LabwiredTarget<RiscV>`,
    /// but `WasmSimulator` now holds `Machine<Box<dyn Cpu>>` so the bound
    /// isn't satisfied. The playground has no JS caller for this method,
    /// so we return an empty packet rather than refactor `labwired-gdbstub`
    /// to be dyn-aware. Track via the v0.6 plan.
    #[wasm_bindgen]
    pub fn gdb_process_packet(&mut self, _packet: &[u8]) -> Vec<u8> {
        Vec::new()
    }

    #[wasm_bindgen]
    pub fn step(&mut self, cycles: u32) -> Result<(), JsValue> {
        for _ in 0..cycles {
            self.machine()
                .advance(AdvanceRequest::single())
                .map_err(|e| JsValue::from_str(&format!("Step Error: {}", e)))?;
        }
        Ok(())
    }

    #[wasm_bindgen]
    pub fn step_single(&mut self) -> Result<(), JsValue> {
        self.machine()
            .advance(AdvanceRequest::single())
            .map(|_| ())
            .map_err(|e| JsValue::from_str(&format!("Step Error: {}", e)))
    }

    /// Connect this chip's UART (`uart_id`, e.g. "uart2") to a shared cross-link
    /// `bus`, so it exchanges bytes with the other chip on the same `link_id`.
    /// The two chips of a point-to-point IO-Link use opposite `side`s (0 and 1)
    /// of the SAME `WireBus`. Bytes flow through the bus with no per-byte host
    /// round-trip, so both chips can keep stepping in batches. Chips wired to
    /// different `WireBus` instances are fully isolated.
    #[wasm_bindgen]
    pub fn attach_uart_wire(
        &mut self,
        uart_id: &str,
        link_id: u32,
        side: u8,
        bus: &WireBus,
    ) -> Result<(), JsValue> {
        let endpoint = Box::new(bus.inner.endpoint(link_id, side));
        self.machine()
            .bus
            .attach_uart_stream_by_id(uart_id, endpoint)
            .map_err(|e| JsValue::from_str(&format!("attach_uart_wire: {e:#}")))?;
        // Keep the cross-link's raw protocol octets out of the human serial
        // monitor — they're decoded by the protocol analyzer (uart_trace), and
        // dumping them into the console floods both peers with identical-looking
        // binary. The debug UART (USART1) still feeds the console normally.
        self.machine()
            .bus
            .detach_uart_sink_by_id(uart_id)
            .map_err(|e| JsValue::from_str(&format!("attach_uart_wire(sink): {e:#}")))
    }

    #[wasm_bindgen]
    pub fn get_pc(&self) -> u32 {
        self.machine.as_ref().unwrap().cpu.get_pc()
    }

    #[wasm_bindgen]
    pub fn get_register(&self, id: u8) -> u32 {
        self.machine.as_ref().unwrap().cpu.get_register(id)
    }

    #[wasm_bindgen]
    pub fn get_register_names(&self) -> JsValue {
        let names = self.machine.as_ref().unwrap().cpu.get_register_names();
        serde_wasm_bindgen::to_value(&names).unwrap()
    }

    #[wasm_bindgen]
    pub fn read_memory(&self, addr: u32, len: u32) -> Vec<u8> {
        let machine = self.machine.as_ref().unwrap();
        (0..len)
            .map(|i| machine.bus.read_u8(addr as u64 + i as u64).unwrap_or(0))
            .collect()
    }

    #[wasm_bindgen]
    pub fn get_disassembly(&self) -> String {
        let machine = self.machine.as_ref().unwrap();
        let pc = machine.cpu.get_pc();
        match self.arch {
            // ESP32-C3 / generic RV32: use the RISC-V decoder. The previous path
            // always ran Thumb decode, so C3 Trace showed ARM-looking ops and
            // frequent `Unknown32` against real RISC-V encodings.
            Arch::RiscV => {
                let pc = pc & !1;
                match machine.bus.read_u16(pc as u64) {
                    Ok(lo) => {
                        // RV32C: least-significant two bits != 0b11 ⇒ 16-bit.
                        if lo & 0b11 != 0b11 {
                            format!("{:?}", decode_rv32c(lo))
                        } else {
                            match machine.bus.read_u16(pc as u64 + 2) {
                                Ok(hi) => {
                                    let word = (u32::from(hi) << 16) | u32::from(lo);
                                    format!("{:?}", decode_rv32(word))
                                }
                                Err(_) => "?? (Error reading RV hi half)".to_string(),
                            }
                        }
                    }
                    Err(_) => "?? (Error reading RV instruction)".to_string(),
                }
            }
            Arch::Xtensa => {
                // Match the LX7 fetch path: length from byte0, then narrow/wide.
                match machine.bus.read_u8(pc as u64) {
                    Ok(b0) => {
                        let len = xtensa_length::instruction_length(b0);
                        if len == 2 {
                            match machine.bus.read_u16(pc as u64) {
                                Ok(hw) => format!("{:?}", xtensa_narrow::decode_narrow(hw)),
                                Err(_) => "?? (Error reading Xtensa narrow)".to_string(),
                            }
                        } else {
                            match machine.bus.read_u32(pc as u64) {
                                Ok(w) => format!("{:?}", xtensa::decode(w)),
                                Err(_) => "?? (Error reading Xtensa wide)".to_string(),
                            }
                        }
                    }
                    Err(_) => "?? (Error reading Xtensa instruction)".to_string(),
                }
            }
            Arch::Arm | Arch::Unknown => {
                let pc = pc & !1;
                match machine.bus.read_u16(pc as u64) {
                    Ok(h1) => {
                        let is_32bit = (h1 & 0xE000) == 0xE000 && (h1 & 0x1800) != 0;
                        if is_32bit {
                            match machine.bus.read_u16(pc as u64 + 2) {
                                Ok(h2) => format!("{:?}", decode_thumb_32(h1, h2)),
                                Err(_) => "?? (Error reading h2)".to_string(),
                            }
                        } else {
                            format!("{:?}", decode_thumb_16(h1))
                        }
                    }
                    Err(_) => "?? (Error reading h1)".to_string(),
                }
            }
        }
    }

    /// Execute up to max_cycles steps, returning the number actually executed.
    #[wasm_bindgen]
    pub fn step_batch(&mut self, max_cycles: u32) -> Result<u32, JsValue> {
        let machine = self.machine();
        let before = machine.total_cycles;
        match machine.advance(AdvanceRequest::run(Some(u64::from(max_cycles)))) {
            Ok(report) => {
                let elapsed = machine.total_cycles.saturating_sub(before);
                debug_assert_eq!(elapsed, report.elapsed_cycles);
                Ok(elapsed.min(u64::from(u32::MAX)) as u32)
            }
            Err(e) => {
                let elapsed = machine.total_cycles.saturating_sub(before);
                let executed = elapsed.min(u64::from(u32::MAX)) as u32;
                if executed > 0 {
                    Ok(executed)
                } else {
                    Err(JsValue::from_str(&format!("Step Error: {}", e)))
                }
            }
        }
    }

    /// Execute one measured batch and return both wall-clock timing and core
    /// run-loop counters. Intended for worker/Playwright profiling; normal
    /// animation still calls `step_batch`.
    #[wasm_bindgen]
    pub fn step_batch_profile(&mut self, max_cycles: u32) -> Result<JsValue, JsValue> {
        let t0 = perf_now();
        let machine = self.machine();
        let before = machine.total_cycles;
        machine.reset_step_profile();
        let advance_result = machine.advance(AdvanceRequest::run(Some(u64::from(max_cycles))));
        let elapsed = machine.total_cycles.saturating_sub(before);
        let executed = match advance_result {
            Ok(report) => {
                debug_assert_eq!(elapsed, report.elapsed_cycles);
                report.elapsed_cycles.min(u64::from(u32::MAX)) as u32
            }
            Err(e) => {
                let partial = elapsed.min(u64::from(u32::MAX)) as u32;
                if partial == 0 {
                    return Err(JsValue::from_str(&format!("Step Error: {}", e)));
                }
                partial
            }
        };
        let profile = machine.step_profile();
        let t1 = perf_now();

        serde_wasm_bindgen::to_value(&WasmStepBatchProfile {
            requested_cycles: max_cycles,
            executed_cycles: executed,
            wall_ms: t1 - t0,
            cycles_per_second: if t1 > t0 {
                (executed as f64) * 1000.0 / (t1 - t0)
            } else {
                0.0
            },
            cpu_instructions: profile.cpu_instructions,
            cpu_batches: profile.cpu_batches,
            peripheral_ticks: profile.peripheral_ticks,
            peripheral_ticked_entries: profile.peripheral_ticked_entries,
            bus_tick_entries: profile.bus_tick_entries,
            legacy_tick_entries: profile.legacy_tick_entries,
        })
        .map_err(|e| JsValue::from_str(&format!("profile serialize: {e}")))
    }

    // ──────────────────────────────────────────────────────────────────────
    //  IO-Link DI demo: 74HC165 input toggling + IO-Link master readout.
    //  These find the device by iterating the bus (the shifter/master are
    //  `external_devices`, not `board_io` bindings), which suits the single
    //  shifter + single master of the IO-Link DI/DO demo.
    // ──────────────────────────────────────────────────────────────────────

    // DEPRECATED: renamed to install_esp32_arduino_quirks for clarity.
    // The concern is Arduino-ESP32 firmware bootstrap (heap-caps thunks,
    // dual-core handshake fakery, sendHello stub, WifiWsLink::loop stub,
    // esp_crc8 thunk, etc.), not a specific customer product. Kept as a
    // thin wrapper so the standalone /playground page (and any other
    // pre-rename caller) keeps working.
    #[wasm_bindgen]
    #[allow(deprecated)]
    #[deprecated(
        note = "Renamed to install_esp32_arduino_quirks — the bootstrap is generic Arduino-ESP32 glue, not firmware-specific."
    )]
    /// #124 Phase 4: enable/disable the browser-side JIT fast-path. When
    /// on, `step_with_esp32_aids` short-circuits any pre-fetch step
    /// whose PC matches the JIT'd hot block (`0x400829cc`) into a wasm
    /// call constructed via `js_sys::WebAssembly`. Off by default —
    /// callers opt in from JS once they've benchmarked.
    #[wasm_bindgen]
    pub fn set_jit_enabled(&mut self, enabled: bool) {
        self.jit_browser_enabled = enabled;
        if !enabled {
            // Cleanly drop the cached module + closures so the next
            // enable rebuilds from scratch.
            self.jit_browser_cache = None;
        }
    }

    /// Enable/disable scheduler-safe CPU idle fast-forwarding. Off by default;
    /// browser callers opt in explicitly after comparing accelerated and
    /// non-accelerated traces for the target firmware.
    #[wasm_bindgen]
    pub fn set_idle_fast_forward_enabled(&mut self, enabled: bool) {
        self.machine().config.idle_fast_forward_enabled = enabled;
    }

    /// Cumulative cycles advanced by idle fast-forward (WFI skip), not
    /// interpreted. Browser `?perf=1` uses this to prove FF is firing; stays
    /// 0 when FF is off or firmware never parks in a skippable idle.
    #[wasm_bindgen]
    pub fn idle_fast_forward_cycles_skipped(&self) -> u64 {
        self.machine
            .as_ref()
            .map(|m| m.idle_fast_forward_cycles_skipped)
            .unwrap_or(0)
    }

    /// Set the peripheral tick interval used by `Machine::run`.
    ///
    /// `1` is the exact default: tick orchestration runs after every executed
    /// instruction. Larger values are a bounded browser acceleration knob for
    /// firmware bring-up paths whose active peripherals are scheduler-driven or
    /// inactive.
    ///
    /// The machine and bus each hold a `SimulationConfig`; both are updated —
    /// the run loop paces ticks off the machine's copy while the legacy-walk
    /// quantum (`tick_elapsed(interval)`) and the HC-SR04 event-scheduling
    /// gate read the bus's, and they must agree or walked peripherals run
    /// `interval`× slow.
    #[wasm_bindgen]
    pub fn set_peripheral_tick_interval(&mut self, interval: u32) {
        let machine = self.machine();
        machine.config.peripheral_tick_interval = interval.max(1);
        machine.bus.config.peripheral_tick_interval = interval.max(1);
    }

    /// The largest `peripheral_tick_interval` this machine's bus can run at
    /// without losing fidelity (see `SystemBus::max_safe_tick_interval`): a
    /// batching interval when every peripheral is scheduler-driven, `1` when
    /// anything non-relaxable (IO-Link master, op-modeling FLASH, a live
    /// legacy walk) is present. The TS side calls this once at engine init
    /// and feeds the answer straight into `set_peripheral_tick_interval`.
    #[wasm_bindgen]
    pub fn recommended_tick_interval(&mut self) -> u32 {
        self.machine().bus.max_safe_tick_interval()
    }

    /// Total number of times the browser JIT has dispatched a
    /// compiled block. Useful for confirming the JIT path actually
    /// fired during a benchmark.
    #[wasm_bindgen]
    pub fn jit_hits(&self) -> u64 {
        self.jit_browser_cache
            .as_ref()
            .map(|c| c.total_hits())
            .unwrap_or(0)
    }

    /// Total number of JIT refusals (host bus errors, JS-side
    /// dispatch failures). Surfaced for the bench harness so it can
    /// distinguish "JIT was tried and rejected" from "JIT was never
    /// hit because PC never reached the block".
    #[wasm_bindgen]
    pub fn jit_refusals(&self) -> u64 {
        self.jit_browser_cache
            .as_ref()
            .map(|c| c.refusals)
            .unwrap_or(0)
    }

    /// Bench runner: execute `cycles` `step_with_esp32_aids` iterations
    /// and return elapsed milliseconds (measured via
    /// `performance.now()`). The caller drives this twice — once with
    /// `set_jit_enabled(false)`, once with `set_jit_enabled(true)` —
    /// and compares the two numbers to quantify JIT speedup.
    ///
    /// Returns a `Result<f64, JsValue>`: the `Err` path bubbles step
    /// errors so the bench harness can show a useful message.
    #[wasm_bindgen]
    pub fn bench_jit(&mut self, cycles: u32) -> Result<f64, JsValue> {
        let t0 = perf_now();
        self.step_with_esp32_aids(cycles)?;
        let t1 = perf_now();
        Ok(t1 - t0)
    }

    /// Step `cycles` cycles with the ESP32-classic IPI bridge active. Each
    /// cycle samples the DPORT FROM_CPU intmatrix mapping and trigger
    /// registers, raises the corresponding INTERRUPT bit, and clears the
    /// trigger so the next write re-edges. The dual-core handshake bytes
    /// are re-applied every 10k cycles (matching the e2e test cadence).
    /// Falls back to plain `step` if `install_esp32_arduino_quirks` hasn't
    /// been called yet.
    #[wasm_bindgen]
    pub fn step_with_esp32_aids(&mut self, cycles: u32) -> Result<(), JsValue> {
        // Real dual-core: a genuine APP_CPU is attached, so the handshake
        // keep-alive and the FROM_CPU IPI bridge below are unnecessary — the
        // firmware drives the rendezvous itself and Machine::step delivers the
        // cross-core IPI via the DPORT. Just step both cores.
        if self
            .machine
            .as_ref()
            .is_some_and(|m| m.cpu_secondary.is_some())
        {
            return self.step(cycles);
        }
        if self.esp32_ipi.is_none() {
            return self.step(cycles);
        }
        for i in 0..cycles {
            {
                let machine = self.machine.as_mut().unwrap();
                let bridge = self.esp32_ipi.as_mut().unwrap();
                if let Ok(v) = machine.bus.read_u32(0x3FF0_0164) {
                    let bit = (v & 0x1F) as u8;
                    if v != 0 && bit < 32 {
                        bridge.from_cpu_bit0 = Some(bit);
                    }
                }
                if let Ok(v) = machine.bus.read_u32(0x3FF0_0168) {
                    let bit = (v & 0x1F) as u8;
                    if v != 0 && bit < 32 {
                        bridge.from_cpu_bit1 = Some(bit);
                    }
                }
                if let Ok(v0) = machine.bus.read_u32(0x3FF0_00DC) {
                    if v0 != 0 && v0 != bridge.last_from_cpu0_val {
                        if let Some(bit) = bridge.from_cpu_bit0 {
                            machine.cpu.raise_interrupt_bits(1u32 << bit);
                        }
                        let _ = machine.bus.write_u32(0x3FF0_00DC, 0);
                    }
                    bridge.last_from_cpu0_val = 0;
                }
                if let Ok(v1) = machine.bus.read_u32(0x3FF0_00E0) {
                    if v1 != 0 && v1 != bridge.last_from_cpu1_val {
                        if let Some(bit) = bridge.from_cpu_bit1 {
                            machine.cpu.raise_interrupt_bits(1u32 << bit);
                        }
                        let _ = machine.bus.write_u32(0x3FF0_00E0, 0);
                    }
                    bridge.last_from_cpu1_val = 0;
                }
                // Dual-core handshake keep-alive. Re-asserts the handshake
                // bytes every 10k cycles so .bss zero-init can't wipe them
                // before the spin-wait check in call_start_cpu0. Uses the
                // per-firmware addresses resolved by autodiscover when
                // available; falls back to the hardcoded reference-firmware
                // addresses for the legacy install_esp32_arduino_quirks
                // path.
                if i % 10_000 == 0 {
                    if !bridge.handshake_bytes.is_empty() {
                        for &addr in &bridge.handshake_bytes {
                            let _ = machine.bus.write_u8(addr as u64, 0x01);
                        }
                    } else {
                        let _ = machine.bus.write_u8(0x3FFC_6F04, 0x01);
                        let _ = machine.bus.write_u8(0x3FFC_6F01, 0x01);
                        let _ = machine.bus.write_u8(0x3FFC_6F02, 0x01);
                        let _ = machine.bus.write_u8(0x3FFC_6FFD, 0x01);
                        let _ = machine.bus.write_u8(0x3FFC_6FFE, 0x01);
                        let _ = machine.bus.write_u8(0x3FFC_7190, 0x01);
                    }
                }
            }

            // #124 Phase 4: browser-side JIT fast-path. Runs BEFORE
            // `machine.step()` so a successful JIT call advances PC
            // past the hot block (0x400829cc -> 0x400829e4) and the
            // regular step picks up at the post-block callx8.
            // CCOUNT advance happens inside the JIT helper to keep
            // CCOMPARE0 edge detection honest.
            if self.jit_browser_enabled {
                let machine = self.machine.as_mut().unwrap();
                if self.jit_browser_cache.is_none() {
                    self.jit_browser_cache = Some(Box::new(jit_browser::BrowserJitCache::new()));
                }
                let cache = self.jit_browser_cache.as_mut().unwrap();
                if let Some(any) = machine.cpu.as_any_mut() {
                    if let Some(xt) = any.downcast_mut::<labwired_core::cpu::XtensaLx7>() {
                        jit_browser::try_browser_jit_step(xt, &mut machine.bus, cache);
                    }
                }
            }

            self.machine()
                .step()
                .map_err(|e| JsValue::from_str(&format!("Step Error: {e}")))?;
        }
        Ok(())
    }
}

// ── Browser-side performance.now() shim ────────────────────────────────
//
// `web-sys` would bring in a large generated binding tree just to call
// `performance.now()`. We use an explicit `wasm-bindgen` import instead.
// Same ABI; ~zero overhead. The matching console.warn shim lives in
// `jit_browser.rs` to keep its module self-contained.

#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(js_namespace = performance, js_name = now)]
    fn perf_now() -> f64;
}

/// A shared UART cross-link medium, owned by the host. Create one per multi-chip
/// lab-group and pass it to every chip's `attach_uart_wire`; chips sharing a bus
/// exchange bytes, chips on different buses are isolated. A fresh `WireBus` per
/// lab (re)load replaces the former module-global reset — a new bus starts empty,
/// so no stale link buffers can leak into the new station.
#[wasm_bindgen]
pub struct WireBus {
    inner: labwired_core::network::virtual_uart_wire::VirtualWireBus,
}

#[wasm_bindgen]
impl WireBus {
    #[wasm_bindgen(constructor)]
    #[allow(clippy::new_without_default)]
    pub fn new() -> WireBus {
        WireBus {
            inner: labwired_core::network::virtual_uart_wire::VirtualWireBus::new(),
        }
    }

    /// Drop every link's buffered bytes on this bus. Rarely needed — prefer a
    /// fresh `WireBus` per lab load — but exposed for in-place resets.
    #[wasm_bindgen]
    pub fn clear(&self) {
        self.inner.clear();
    }
}

/// Parse a JS `{ name: Uint8Array }` object into a `name → bytes` map. Values
/// that aren't `Uint8Array` are skipped; `null`/`undefined` → empty map.
///
/// This is the generic on-demand binary-blob channel: a board fetches only the
/// assets it needs (e.g. the ESP32-S3 boot ROM) and passes them through
/// `new_from_config`, so no per-board blob is baked into the shared wasm bundle.
fn parse_named_blobs(blobs: &JsValue) -> std::collections::HashMap<String, Vec<u8>> {
    use wasm_bindgen::JsCast;
    let mut map = std::collections::HashMap::new();
    if blobs.is_undefined() || blobs.is_null() {
        return map;
    }
    if let Ok(obj) = blobs.clone().dyn_into::<js_sys::Object>() {
        for entry in js_sys::Object::entries(&obj).iter() {
            if let Ok(pair) = entry.dyn_into::<js_sys::Array>() {
                if let (Some(key), Ok(arr)) = (
                    pair.get(0).as_string(),
                    pair.get(1).dyn_into::<js_sys::Uint8Array>(),
                ) {
                    map.insert(key, arr.to_vec());
                }
            }
        }
    }
    map
}

// WasmGdbEventLoop removed — see `gdb_process_packet` above for the rationale.
// Restoring this requires `LabwiredTarget` to be implemented for an arch-erased
// CPU type, which is the follow-up tracked alongside Phase 1.

#[cfg(all(test, not(target_arch = "wasm32")))]
mod machine_advance_tests {
    use super::*;
    use std::collections::BTreeSet;

    fn wrap_test_machine<C: Cpu + 'static>(
        cpu: C,
        mut bus: SystemBus,
        arch: Arch,
    ) -> WasmSimulator {
        let uart_sink = Arc::new(Mutex::new(Vec::new()));
        bus.attach_uart_tx_sink(uart_sink.clone(), false);
        let uart_rx_bufs = bus.attach_uart_rx_source();
        let cpu: Box<dyn Cpu> = Box::new(cpu);
        let mut machine = Machine::new(cpu, bus);
        machine.config.peripheral_tick_interval = 64;
        machine.bus.config.peripheral_tick_interval = 64;

        WasmSimulator {
            machine: Some(machine),
            board_io: Vec::new(),
            uart_sink,
            uart_rx_bufs,
            arch,
            esp32_ipi: None,
            jit_browser_enabled: false,
            jit_browser_cache: None,
        }
    }

    fn arm_simulator() -> WasmSimulator {
        let mut bus = SystemBus::new();
        let mut cpu = labwired_core::cpu::CortexM::new();
        for index in 0..64_u64 {
            bus.write_u16(index * 2, 0xBF00).unwrap();
        }
        cpu.set_pc(0);
        wrap_test_machine(cpu, bus, Arch::Arm)
    }

    fn configured_arm_simulator() -> WasmSimulator {
        let mut bus = SystemBus::new();
        let (mut cpu, _) = configure_cortex_m(&mut bus);
        for index in 0..64_u64 {
            bus.write_u16(index * 2, 0xBF00).unwrap();
        }
        cpu.set_pc(0);
        wrap_test_machine(cpu, bus, Arch::Arm)
    }

    fn riscv_simulator() -> WasmSimulator {
        let mut bus = SystemBus::new();
        let mut cpu = labwired_core::system::riscv::configure_riscv(&mut bus);
        for index in 0..64_u64 {
            bus.write_u32(index * 4, 0x0000_0013).unwrap();
        }
        cpu.set_pc(0);
        wrap_test_machine(cpu, bus, Arch::RiscV)
    }

    fn xtensa_simulator() -> WasmSimulator {
        let mut bus = SystemBus::new();
        let mut cpu = labwired_core::cpu::XtensaLx7::new();
        for index in 0..64_u64 {
            bus.write_u8(index * 2, 0x3d).unwrap();
            bus.write_u8(index * 2 + 1, 0xf0).unwrap();
        }
        cpu.set_pc(0);
        wrap_test_machine(cpu, bus, Arch::Xtensa)
    }

    fn assert_batch_matches_32_singles(
        build: impl Fn() -> WasmSimulator,
        expected_batch_count: u64,
        expect_peripherals: bool,
    ) {
        let mut singles = build();
        let mut batch = build();

        for _ in 0..32 {
            singles.step_single().expect("single step");
        }
        assert_eq!(batch.step_batch(32).expect("batch step"), 32);

        let singles = singles.machine.as_ref().unwrap();
        let batch = batch.machine.as_ref().unwrap();
        let singles_snapshot = singles.snapshot();
        let batch_snapshot = batch.snapshot();

        assert_eq!(
            serde_json::to_value(&singles_snapshot).unwrap(),
            serde_json::to_value(&batch_snapshot).unwrap()
        );
        assert_eq!(
            serde_json::to_value(singles.cpu.snapshot()).unwrap(),
            serde_json::to_value(batch.cpu.snapshot()).unwrap()
        );
        assert_eq!(singles_snapshot.peripherals, batch_snapshot.peripherals);
        if expect_peripherals {
            assert!(!singles_snapshot.peripherals.is_empty());
            assert!(!batch_snapshot.peripherals.is_empty());
        }
        assert_eq!(singles.total_cycles, batch.total_cycles);
        assert_eq!(singles.bus.current_cycle, batch.bus.current_cycle);
        assert_eq!(singles.cpu.get_pc(), batch.cpu.get_pc());
        assert_ne!(singles.cpu.get_pc(), 0);
        assert_eq!((singles.total_cycles, batch.total_cycles), (32, 32));

        let singles_profile = singles.step_profile();
        let batch_profile = batch.step_profile();
        assert_eq!(singles_profile.cpu_instructions, 32);
        assert_eq!(batch_profile.cpu_instructions, 32);
        assert_eq!(singles_profile.cpu_batches, 32);
        assert_eq!(batch_profile.cpu_batches, expected_batch_count);
        if expected_batch_count == 1 {
            assert!(batch_profile.cpu_batches < batch_profile.cpu_instructions);
        }

        // CPU batch count is intentionally execution-path dependent. Every
        // peripheral-work counter must remain identical across the two paths.
        assert_eq!(
            singles_profile.peripheral_ticks,
            batch_profile.peripheral_ticks
        );
        assert_eq!(
            singles_profile.peripheral_ticked_entries,
            batch_profile.peripheral_ticked_entries
        );
        assert_eq!(
            singles_profile.bus_tick_entries,
            batch_profile.bus_tick_entries
        );
        assert_eq!(
            singles_profile.legacy_tick_entries,
            batch_profile.legacy_tick_entries
        );
    }

    #[test]
    fn arm_batch_matches_32_single_boundaries() {
        assert_batch_matches_32_singles(arm_simulator, 1, false);
    }

    #[test]
    fn configured_arm_batch_matches_32_single_boundaries() {
        // A real Cortex-M topology contains an SCB, whose reset-fidelity rail
        // intentionally commits one instruction per CPU batch.
        assert_batch_matches_32_singles(configured_arm_simulator, 32, true);
    }

    #[test]
    fn riscv_batch_matches_32_single_boundaries() {
        assert_batch_matches_32_singles(riscv_simulator, 1, false);
    }

    #[test]
    fn xtensa_batch_matches_32_single_boundaries() {
        assert_batch_matches_32_singles(xtensa_simulator, 1, false);
    }

    #[test]
    fn step_batch_profile_schema_is_exact() {
        let value = serde_json::to_value(WasmStepBatchProfile {
            requested_cycles: 1,
            executed_cycles: 2,
            wall_ms: 3.0,
            cycles_per_second: 4.0,
            cpu_instructions: 5,
            cpu_batches: 6,
            peripheral_ticks: 7,
            peripheral_ticked_entries: 8,
            bus_tick_entries: 9,
            legacy_tick_entries: 10,
        })
        .unwrap();
        let actual = value
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect::<BTreeSet<_>>();
        let expected = [
            "bus_tick_entries",
            "cpu_batches",
            "cpu_instructions",
            "cycles_per_second",
            "executed_cycles",
            "legacy_tick_entries",
            "peripheral_ticked_entries",
            "peripheral_ticks",
            "requested_cycles",
            "wall_ms",
        ]
        .into_iter()
        .collect::<BTreeSet<_>>();

        assert_eq!(actual, expected);
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod romboot_tests {
    //! Regression guard for the ESP32-C3 wasm faithful ROM-boot path.
    //!
    //! Exercises the exact browser entry [`WasmSimulator::new_from_config_riscv_romboot`]
    //! on the native test target (a real headless browser isn't available): it
    //! provisions the boot ROM from the two ROM blobs, injects them into the
    //! chip's `rom`/`rom_data` regions, hands the merged flash image to the
    //! shared core builder, resets to `0x4000_0000`, and runs the genuine mask
    //! ROM → 2nd-stage bootloader → `app_main()`. Asserts it reaches the IDF
    //! `Calling app_main()` / "Hello world!" banner. Zero thunks.
    //!
    //! `#[ignore]` because the faithful path spends ~150M steps in the real ROM
    //! before `app_main`; run it in release:
    //!   `cargo test -p labwired-wasm --release romboot -- --ignored --nocapture`
    use super::*;
    use labwired_config::{ChipDescriptor, SystemManifest};
    use std::collections::HashMap;
    use std::path::PathBuf;

    fn root() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
    }

    #[test]
    fn parses_esp32c3_app_segments_from_merged_flash() {
        let mut flash = vec![0xff; ESP32C3_APP_IMAGE_OFFSET + 256];
        let app = ESP32C3_APP_IMAGE_OFFSET;
        flash[app] = ESP_IMAGE_MAGIC;
        flash[app + 1] = 2;
        flash[app + 4..app + 8].copy_from_slice(&0x4200_1234u32.to_le_bytes());

        let mut cursor = app + ESP_IMAGE_HEADER_LEN;
        flash[cursor..cursor + 4].copy_from_slice(&0x3FC8_0010u32.to_le_bytes());
        flash[cursor + 4..cursor + 8].copy_from_slice(&3u32.to_le_bytes());
        cursor += 8;
        flash[cursor..cursor + 3].copy_from_slice(&[1, 2, 3]);
        cursor += 3;

        flash[cursor..cursor + 4].copy_from_slice(&0x4200_2000u32.to_le_bytes());
        flash[cursor + 4..cursor + 8].copy_from_slice(&4u32.to_le_bytes());
        cursor += 8;
        flash[cursor..cursor + 4].copy_from_slice(&[4, 5, 6, 7]);

        let image = esp32c3_app_program_image_from_merged_flash(&flash).expect("parse app image");

        assert_eq!(image.entry_point, 0x4200_1234);
        assert_eq!(image.arch, CoreArch::RiscV);
        assert_eq!(image.segments.len(), 2);
        assert_eq!(image.segments[0].start_addr, 0x3FC8_0010);
        assert_eq!(image.segments[0].data, vec![1, 2, 3]);
        assert_eq!(image.segments[1].start_addr, 0x4200_2000);
        assert_eq!(image.segments[1].data, vec![4, 5, 6, 7]);
    }

    #[test]
    fn rejects_flash_without_esp_app_magic_at_app_offset() {
        let flash = vec![0xff; ESP32C3_APP_IMAGE_OFFSET + ESP_IMAGE_HEADER_LEN];

        let err = esp32c3_app_program_image_from_merged_flash(&flash).unwrap_err();

        assert!(err.contains("bad magic"), "{err}");
    }

    #[test]
    fn parses_esp32c3_bootloader_segments_from_merged_flash() {
        let mut flash = vec![0xff; 128];
        flash[0] = ESP_IMAGE_MAGIC;
        flash[1] = 1;
        flash[4..8].copy_from_slice(&0x4038_0100u32.to_le_bytes());

        let cursor = ESP_IMAGE_HEADER_LEN;
        flash[cursor..cursor + 4].copy_from_slice(&0x4038_0100u32.to_le_bytes());
        flash[cursor + 4..cursor + 8].copy_from_slice(&4u32.to_le_bytes());
        flash[cursor + 8..cursor + 12].copy_from_slice(&[0x13, 0x00, 0x00, 0x00]);

        let image =
            esp32c3_bootloader_program_image_from_merged_flash(&flash).expect("parse bootloader");

        assert_eq!(image.entry_point, 0x4038_0100);
        assert_eq!(image.segments.len(), 1);
        assert_eq!(image.segments[0].start_addr, 0x4038_0100);
        assert_eq!(image.segments[0].data, vec![0x13, 0x00, 0x00, 0x00]);
    }

    #[test]
    #[ignore = "boots the real C3 mask ROM (~150M steps); run with --release --ignored"]
    fn wasm_romboot_reaches_app_main() {
        let manifest_dir = root();
        let chip_yaml =
            std::fs::read_to_string(manifest_dir.join("../../configs/chips/esp32c3.yaml"))
                .expect("read esp32c3 chip yaml");
        let system_yaml =
            std::fs::read_to_string(manifest_dir.join("../../configs/systems/esp32c3-devkit.yaml"))
                .expect("read esp32c3-devkit system yaml");
        let chip: ChipDescriptor = serde_yaml::from_str(&chip_yaml).expect("parse chip yaml");
        let manifest: SystemManifest =
            serde_yaml::from_str(&system_yaml).expect("parse system yaml");

        // The browser fetches these on demand; here we read the vendored ROM
        // bins + the committed IDF hello_world flash image directly.
        let irom = std::fs::read(manifest_dir.join("../core/roms/esp32c3/esp32c3_rom.bin"))
            .expect("read vendored C3 IROM");
        let drom = std::fs::read(manifest_dir.join("../core/roms/esp32c3/esp32c3_drom.bin"))
            .expect("read vendored C3 DROM");
        let flash =
            std::fs::read(manifest_dir.join("tests/fixtures/esp32c3-hello-world-flash.bin"))
                .expect("read C3 hello_world flash image");

        let mut blobs: HashMap<String, Vec<u8>> = HashMap::new();
        blobs.insert("esp32c3_irom".into(), irom);
        blobs.insert("esp32c3_drom".into(), drom);
        blobs.insert("esp32c3_flash".into(), flash);

        let mut sim = WasmSimulator::new_from_config_riscv_romboot(&chip, &manifest, &blobs)
            .expect("construct C3 rom-boot WasmSimulator");

        // Step in batches; stop as soon as the app_main banner appears.
        const BATCH: u32 = 1_000_000;
        const MAX_STEPS: u64 = 300_000_000;
        let mut steps: u64 = 0;
        let mut reached = false;
        while steps < MAX_STEPS {
            sim.step(BATCH).expect("step");
            steps += BATCH as u64;
            let out = String::from_utf8_lossy(&sim.uart_sink.lock().unwrap()).into_owned();
            if out.contains("Hello world!") {
                reached = true;
                eprintln!("reached app_main at ~{steps} steps");
                break;
            }
        }
        let out = String::from_utf8_lossy(&sim.uart_sink.lock().unwrap()).into_owned();
        assert!(
            reached,
            "C3 wasm rom-boot did not reach app_main within {MAX_STEPS} steps.\n\
             --- captured serial ---\n{out}"
        );
        assert!(
            out.contains("Calling app_main()"),
            "expected IDF 'Calling app_main()' banner; got:\n{out}"
        );
    }

    /// Decisive proof the browser OLED lab paints: boot the curated
    /// `esp32c3-oled-demo` IDF flash image FAITHFULLY through the real mask ROM
    /// (the exact browser entry `new_from_config_riscv_romboot`), let the
    /// firmware's register-level SSD1306 driver run, then read the panel's
    /// GDDRAM back through the same `get_ssd1306_framebuffer` accessor the
    /// playground/embed uses and assert a non-trivial number of pixels are lit.
    ///
    /// Zero thunks: every lit pixel is a byte the firmware pushed via a genuine
    /// I²C transaction the simulated C3 command-list controller executed against
    /// the attached SSD1306 model. No hardcoded PCs, no faked framebuffer.
    ///
    /// `#[ignore]` for the same reason as the app_main guard (~150M ROM steps);
    /// run with:
    ///   `cargo test -p labwired-wasm --release romboot_oled -- --ignored --nocapture`
    #[test]
    #[ignore = "boots the real C3 mask ROM then paints the OLED; run with --release --ignored"]
    fn wasm_romboot_oled_paints() {
        let manifest_dir = root();
        let chip_yaml =
            std::fs::read_to_string(manifest_dir.join("../../configs/chips/esp32c3.yaml"))
                .expect("read esp32c3 chip yaml");
        let system_yaml = std::fs::read_to_string(
            manifest_dir.join("../../configs/systems/esp32c3-oled-demo.yaml"),
        )
        .expect("read esp32c3-oled-demo system yaml");
        let chip: ChipDescriptor = serde_yaml::from_str(&chip_yaml).expect("parse chip yaml");
        let manifest: SystemManifest =
            serde_yaml::from_str(&system_yaml).expect("parse system yaml");

        let irom = std::fs::read(manifest_dir.join("../core/roms/esp32c3/esp32c3_rom.bin"))
            .expect("read vendored C3 IROM");
        let drom = std::fs::read(manifest_dir.join("../core/roms/esp32c3/esp32c3_drom.bin"))
            .expect("read vendored C3 DROM");
        let flash = std::fs::read(manifest_dir.join("tests/fixtures/esp32c3-oled-demo-flash.bin"))
            .expect("read C3 OLED demo flash image");

        let mut blobs: HashMap<String, Vec<u8>> = HashMap::new();
        blobs.insert("esp32c3_irom".into(), irom);
        blobs.insert("esp32c3_drom".into(), drom);
        blobs.insert("esp32c3_flash".into(), flash);

        let mut sim = WasmSimulator::new_from_config_riscv_romboot(&chip, &manifest, &blobs)
            .expect("construct C3 rom-boot WasmSimulator");

        // Step until the OLED framebuffer holds a non-trivial picture. The
        // firmware paints once shortly after app_main; poll the same accessor
        // the playground uses.
        const BATCH: u32 = 1_000_000;
        const MAX_STEPS: u64 = 300_000_000;
        // "LabWired" + "OLED LAB C3" + frame + bar lights well over this many.
        const MIN_LIT: usize = 400;
        let mut steps: u64 = 0;
        let mut lit = 0usize;
        let mut painted = false;
        while steps < MAX_STEPS {
            sim.step(BATCH).expect("step");
            steps += BATCH as u64;
            if let Ok(fb) = sim.get_ssd1306_framebuffer("oled") {
                lit = fb.iter().map(|b| b.count_ones() as usize).sum();
                if lit >= MIN_LIT {
                    painted = true;
                    eprintln!("OLED painted: {lit} lit pixels at ~{steps} steps");
                    break;
                }
            }
        }
        let out = String::from_utf8_lossy(&sim.uart_sink.lock().unwrap()).into_owned();
        assert!(
            painted,
            "C3 OLED lab did not paint (>= {MIN_LIT} lit pixels) within {MAX_STEPS} steps; \
             last count = {lit}.\n--- captured serial ---\n{out}"
        );
    }

    /// Accelerated C3 flash shares must still run the real app image and paint
    /// attached devices. This skips the mask-ROM replay, but does not fake the
    /// OLED: pixels must come from firmware I2C writes into the SSD1306 model.
    ///
    /// Uses `step_batch` (browser worker path via `Machine::run`), not per-insn
    /// `step`, and applies the same tick + idle-FF policy the playground sets
    /// after `recommended_tick_interval()`.
    #[test]
    #[ignore = "browser-path C3 fast-start paint; run with --release --ignored --nocapture"]
    fn wasm_c3_flash_fast_start_oled_paints_quickly() {
        let (mut sim, rec_tick) = c3_browser_fast_start_sim();
        apply_browser_c3_policy(&mut sim, rec_tick);

        const BATCH: u32 = 2_000_000;
        const MAX_STEPS: u64 = 80_000_000;
        const MIN_LIT: usize = 400;
        let mut steps: u64 = 0;
        let mut lit = 0usize;
        let t0 = std::time::Instant::now();
        while steps < MAX_STEPS {
            let n = sim.step_batch(BATCH).expect("step_batch");
            assert!(n > 0, "step_batch returned 0 executed cycles (MCU stuck?)");
            steps += n as u64;
            if let Ok(fb) = sim.get_ssd1306_framebuffer("oled") {
                lit = fb.iter().map(|b| b.count_ones() as usize).sum();
                if lit >= MIN_LIT {
                    let out = String::from_utf8_lossy(&sim.uart_sink.lock().unwrap()).into_owned();
                    assert!(
                        out.contains("oled-lab") || out.contains("OLED painted"),
                        "C3 flash fast-start painted but did not capture app serial; \
                         captured serial:\n{out}"
                    );
                    eprintln!(
                        "browser-path OLED painted: lit={lit} device_cycles={steps} \
                         rec_tick={rec_tick} wall={:.2}s",
                        t0.elapsed().as_secs_f64()
                    );
                    return;
                }
            }
        }

        let out = String::from_utf8_lossy(&sim.uart_sink.lock().unwrap()).into_owned();
        panic!(
            "C3 flash fast-start did not paint OLED (>= {MIN_LIT} lit pixels) within \
             {MAX_STEPS} steps; last count = {lit}.\n--- captured serial ---\n{out}"
        );
    }

    /// Pre-deploy gate: browser C3 path must stay healthy for **several
    /// device-seconds** after paint (no hang, cycles advance, framebuffer
    /// stays lit, serial remains readable). Mirrors worker `step_batch` +
    /// tick-512 + idle FF — not the slow per-instruction `step` API.
    #[test]
    #[ignore = "multi-second browser-path smoke; run with --release --ignored --nocapture"]
    fn wasm_c3_browser_path_runs_few_device_seconds() {
        let (mut sim, rec_tick) = c3_browser_fast_start_sim();
        apply_browser_c3_policy(&mut sim, rec_tick);
        assert!(
            rec_tick >= 64,
            "walk-free C3 should recommend a batched tick interval, got {rec_tick}"
        );

        // 160 MHz silicon: 3 device-seconds ≈ 480e6 cycles. With idle FF the
        // host wall is much shorter; without it this is still a hard progress
        // + stability gate.
        const DEVICE_SECONDS: f64 = 3.0;
        const CPU_HZ: f64 = 160_000_000.0;
        const TARGET_CYCLES: u64 = (DEVICE_SECONDS * CPU_HZ) as u64;
        const BATCH: u32 = 4_000_000;
        const MIN_LIT: usize = 400;

        let t0 = std::time::Instant::now();
        let mut total: u64 = 0;
        let mut lit_at_paint = 0usize;
        let mut painted = false;

        while total < TARGET_CYCLES {
            let n = sim
                .step_batch(BATCH)
                .unwrap_or_else(|e| panic!("step_batch failed at cycle {total}: {e:?}"));
            assert!(
                n > 0,
                "step_batch executed 0 cycles at total={total} — MCU not advancing"
            );
            total += n as u64;

            if let Ok(fb) = sim.get_ssd1306_framebuffer("oled") {
                let lit = fb.iter().map(|b| b.count_ones() as usize).sum::<usize>();
                if !painted && lit >= MIN_LIT {
                    painted = true;
                    lit_at_paint = lit;
                    eprintln!(
                        "paint @ device_cycles={total} lit={lit} wall={:.2}s",
                        t0.elapsed().as_secs_f64()
                    );
                }
                // After paint, framebuffer must not collapse to empty.
                if painted {
                    assert!(
                        lit >= MIN_LIT / 4,
                        "framebuffer collapsed after paint: lit={lit} at cycle {total}"
                    );
                }
            }
        }

        let wall = t0.elapsed().as_secs_f64();
        let out = String::from_utf8_lossy(&sim.uart_sink.lock().unwrap()).into_owned();
        let skipped = sim.idle_fast_forward_cycles_skipped();
        let mips = total as f64 / wall / 1.0e6;
        eprintln!(
            "browser-path multi-second: device_cycles={total} (~{DEVICE_SECONDS}s @ 160MHz) \
             wall={wall:.2}s host_MIPS={mips:.1} rec_tick={rec_tick} idle_ff_skipped={skipped} \
             painted={painted} lit_at_paint={lit_at_paint}"
        );
        eprintln!(
            "serial tail (last 800 chars):\n{}",
            out.chars()
                .rev()
                .take(800)
                .collect::<String>()
                .chars()
                .rev()
                .collect::<String>()
        );

        assert!(
            painted,
            "must paint OLED within {DEVICE_SECONDS} device-seconds; serial:\n{out}"
        );
        assert!(
            total >= TARGET_CYCLES,
            "must advance at least {TARGET_CYCLES} device cycles"
        );
        // Sanity: host must make progress (not deadlocked / near-zero MIPS).
        assert!(
            mips > 1.0,
            "host throughput too low ({mips:.3} MIPS) — web MCU effectively not running"
        );
    }

    fn c3_browser_fast_start_sim() -> (WasmSimulator, u32) {
        let manifest_dir = root();
        let chip_yaml =
            std::fs::read_to_string(manifest_dir.join("../../configs/chips/esp32c3.yaml"))
                .expect("read esp32c3 chip yaml");
        let system_yaml = std::fs::read_to_string(
            manifest_dir.join("../../configs/systems/esp32c3-oled-demo.yaml"),
        )
        .expect("read esp32c3-oled-demo system yaml");
        let chip: ChipDescriptor = serde_yaml::from_str(&chip_yaml).expect("parse chip yaml");
        let manifest: SystemManifest =
            serde_yaml::from_str(&system_yaml).expect("parse system yaml");

        let irom = std::fs::read(manifest_dir.join("../core/roms/esp32c3/esp32c3_rom.bin"))
            .expect("read vendored C3 IROM");
        let drom = std::fs::read(manifest_dir.join("../core/roms/esp32c3/esp32c3_drom.bin"))
            .expect("read vendored C3 DROM");
        let flash = std::fs::read(manifest_dir.join("tests/fixtures/esp32c3-oled-demo-flash.bin"))
            .expect("read C3 OLED demo flash image");

        let mut blobs: HashMap<String, Vec<u8>> = HashMap::new();
        blobs.insert("esp32c3_irom".into(), irom);
        blobs.insert("esp32c3_drom".into(), drom);
        blobs.insert("esp32c3_flash".into(), flash);
        // Same marker blob playground injects for fast-start selection.
        blobs.insert(crate::ESP32C3_FLASH_FAST_START_BLOB.to_string(), Vec::new());

        let mut sim = WasmSimulator::new_from_config_riscv_flash_fastboot(&chip, &manifest, &blobs)
            .expect("construct C3 flash fast-start WasmSimulator (browser path)");
        let rec = sim.recommended_tick_interval();
        (sim, rec)
    }

    fn apply_browser_c3_policy(sim: &mut WasmSimulator, rec_tick: u32) {
        sim.set_peripheral_tick_interval(rec_tick);
        sim.set_idle_fast_forward_enabled(true);
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod disasm_arch_tests {
    use labwired_core::decoder::arm::{decode_thumb_16, decode_thumb_32};
    use labwired_core::decoder::riscv::{decode_rv32, decode_rv32c};

    /// ADDI x1, x0, 1 — must surface as RISC-V Addi, not Thumb Unknown32.
    #[test]
    fn rv32_addi_is_not_thumb_unknown32() {
        let word: u32 = 0x0010_0093;
        let rv = format!("{:?}", decode_rv32(word));
        assert!(rv.contains("Addi"), "expected Addi, got {rv}");
        let lo = word as u16;
        let hi = (word >> 16) as u16;
        let thumb = format!("{:?}", decode_thumb_32(lo, hi));
        // The old wasm path always used Thumb: that is the bug users saw as
        // Unknown32 / Lsl / BranchCond on C3 ROM+app addresses.
        assert!(
            thumb.contains("Unknown") || !rv.eq_ignore_ascii_case(&thumb),
            "thumb decode of RV word should not look like a real RV Addi: thumb={thumb} rv={rv}"
        );
    }

    #[test]
    fn rv32c_caddi_decodes() {
        // c.addi x8, 1 — common compressed form; just ensure decode path is live.
        let hw: u16 = 0x0505;
        let s = format!("{:?}", decode_rv32c(hw));
        assert!(!s.is_empty());
        let _ = decode_thumb_16(hw);
    }
}
